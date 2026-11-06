use super::*;
use redb::StorageBackend;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[derive(Debug)]
struct FaultBackend {
    file: redb::backends::FileBackend,
    fault: Arc<AtomicU8>,
}
impl FaultBackend {
    fn check(&self, kind: u8) -> std::io::Result<()> {
        if self
            .fault
            .compare_exchange(kind, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Err(std::io::Error::other("jobs injected file I/O failure"));
        }
        Ok(())
    }
}
impl StorageBackend for FaultBackend {
    fn len(&self) -> std::io::Result<u64> {
        self.file.len()
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> std::io::Result<()> {
        self.check(1)?;
        self.file.read(offset, out)
    }
    fn set_len(&self, len: u64) -> std::io::Result<()> {
        self.file.set_len(len)
    }
    fn sync_data(&self) -> std::io::Result<()> {
        self.check(3)?;
        self.file.sync_data()
    }
    fn write(&self, offset: u64, data: &[u8]) -> std::io::Result<()> {
        self.check(2)?;
        self.file.write(offset, data)
    }
    fn close(&self) -> std::io::Result<()> {
        self.file.close()
    }
}
fn deployment() -> DeploymentId {
    DeploymentId::new("fault-test").unwrap()
}
fn configure() -> Command {
    Command::new(
        ClientId::new("client").unwrap(),
        RequestId::new("configure").unwrap(),
        Operation::ConfigureQueue {
            queue: QueueId::new("mail").unwrap(),
            policy: QueuePolicy::new(1, 10).unwrap(),
        },
    )
    .unwrap()
}
fn submit() -> Command {
    Command::new(
        ClientId::new("client").unwrap(),
        RequestId::new("submit").unwrap(),
        Operation::Enqueue {
            jobs: vec![NewJob::new(
                JobId::new("a").unwrap(),
                QueueId::new("mail").unwrap(),
                WorkerKey::new("worker", 1, "schema").unwrap(),
                EncodedArgs::new(vec![12; 1024]).unwrap(),
                0,
                RetryPolicy::new(2, 1, 2).unwrap(),
                Some(UniquenessKey::new("unique").unwrap()),
            )],
        },
    )
    .unwrap()
}

#[test]
fn jobs_real_redb_write_and_sync_errors_never_return_acceptance() {
    for fault_kind in [2, 3] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("jobs.redb");
        let store = JobStore::open(&path, deployment(), StoreLimits::default()).unwrap();
        store.apply(1, &configure()).unwrap().result.unwrap();
        drop(store);
        let fault = Arc::new(AtomicU8::new(0));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let database = Database::builder()
            .create_with_backend(FaultBackend {
                file: redb::backends::FileBackend::new(file).unwrap(),
                fault: fault.clone(),
            })
            .unwrap();
        let store = JobStore { database };
        fault.store(fault_kind, Ordering::SeqCst);
        assert!(matches!(store.apply(2, &submit()), Err(Error::Storage(_))));
        assert_eq!(
            fault.load(Ordering::SeqCst),
            0,
            "fault must hit actual redb I/O"
        );
        drop(store);
        let reopened = JobStore::open(&path, deployment(), StoreLimits::default()).unwrap();
        let job = reopened.job(&JobId::new("a").unwrap()).unwrap();
        let queue = reopened
            .queue(&QueueId::new("mail").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(queue.retained_jobs, u64::from(job.is_some()));
        assert_eq!(
            reopened
                .inspect(&QueueId::new("mail").unwrap(), None, 10)
                .unwrap()
                .len(),
            usize::from(job.is_some())
        );
        assert_eq!(
            reopened
                .due(&QueueId::new("mail").unwrap(), None, 10)
                .unwrap()
                .jobs
                .len(),
            usize::from(job.is_some())
        );
        assert_eq!(
            reopened.receipt(&submit()).unwrap().is_some(),
            job.is_some()
        );
        assert_eq!(
            reopened.applied_index().unwrap(),
            if job.is_some() { 2 } else { 1 }
        );
        reopened.apply(2, &submit()).unwrap().result.unwrap();
        assert_eq!(
            reopened
                .queue(&QueueId::new("mail").unwrap())
                .unwrap()
                .unwrap()
                .retained_jobs,
            1
        );
    }
}

#[test]
fn jobs_read_failure_and_corrupt_rows_propagate_without_advancing_apply() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let store = JobStore::open(&path, deployment(), StoreLimits::default()).unwrap();
    store.apply(1, &configure()).unwrap();
    store.apply(2, &submit()).unwrap();
    drop(store);
    let fault = Arc::new(AtomicU8::new(1));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    assert!(Database::builder()
        .create_with_backend(FaultBackend {
            file: redb::backends::FileBackend::new(file).unwrap(),
            fault: fault.clone()
        })
        .is_err());
    assert_eq!(fault.load(Ordering::SeqCst), 0);
    let store = JobStore::open(&path, deployment(), StoreLimits::default()).unwrap();
    let write = store.database.begin_write().unwrap();
    write
        .open_table(JOBS)
        .unwrap()
        .insert("a", b"corrupt".as_slice())
        .unwrap();
    write.commit().unwrap();
    assert!(matches!(
        store.job(&JobId::new("a").unwrap()),
        Err(Error::Storage(_))
    ));
    let cancel = Command::new(
        ClientId::new("client").unwrap(),
        RequestId::new("cancel").unwrap(),
        Operation::Cancel {
            id: JobId::new("a").unwrap(),
            revision: Revision(2),
        },
    )
    .unwrap();
    assert!(matches!(store.apply(3, &cancel), Err(Error::Storage(_))));
    assert_eq!(store.applied_index().unwrap(), 2);
    assert!(store.receipt(&cancel).unwrap().is_none());
}

#[test]
fn jobs_schema_mismatch_and_missing_index_refuse_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let store = JobStore::open(&path, deployment(), StoreLimits::default()).unwrap();
    let write = store.database.begin_write().unwrap();
    let mut meta: Metadata = read(&write, META, "state").unwrap().unwrap();
    meta.schema = 2;
    put(&write, META, "state", &meta).unwrap();
    write.commit().unwrap();
    drop(store);
    assert!(matches!(
        JobStore::open(&path, deployment(), StoreLimits::default()),
        Err(Error::IncompatibleStore)
    ));
    let database = Database::create(&path).unwrap();
    let write = database.begin_write().unwrap();
    meta.schema = 1;
    put(&write, META, "state", &meta).unwrap();
    write.delete_table(DUE).unwrap();
    write.commit().unwrap();
    drop(database);
    assert!(matches!(
        JobStore::open(&path, deployment(), StoreLimits::default()),
        Err(Error::Storage(_))
    ));
}
