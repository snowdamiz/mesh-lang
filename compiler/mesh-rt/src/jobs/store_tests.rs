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
    Command::internal(Operation::ConfigureQueue {
        queue: QueueId::new("mail").unwrap(),
        policy: QueuePolicy::new(1, 10).unwrap(),
    })
    .unwrap()
}
fn submit() -> Command {
    Command::enqueue(
        ClientStream::new(1, 1).unwrap(),
        1,
        vec![NewJob::new(
            JobId::new("a").unwrap(),
            QueueId::new("mail").unwrap(),
            WorkerKey::new("worker", 1, "schema").unwrap(),
            EncodedArgs::new(vec![12; 1024]).unwrap(),
            0,
            RetryPolicy::new(2, 1, 2).unwrap(),
            Some(UniquenessKey::new("unique").unwrap()),
        )],
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
        store
            .apply(
                2,
                &Command::Register {
                    stream: ClientStream::new(1, 1).unwrap(),
                },
            )
            .unwrap()
            .result
            .unwrap();
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
        assert!(matches!(store.apply(3, &submit()), Err(Error::Storage(_))));
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
            matches!(
                reopened.receipt(&submit()).unwrap(),
                ReceiptLookup::Recorded(_)
            ),
            job.is_some()
        );
        assert_eq!(
            reopened.applied_index().unwrap(),
            if job.is_some() { 3 } else { 2 }
        );
        reopened.apply(3, &submit()).unwrap().result.unwrap();
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
    store
        .apply(
            2,
            &Command::Register {
                stream: ClientStream::new(1, 1).unwrap(),
            },
        )
        .unwrap()
        .result
        .unwrap();
    store.apply(3, &submit()).unwrap();
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
    let cancel = Command::internal(Operation::Cancel {
        id: JobId::new("a").unwrap(),
        revision: Revision(3),
    })
    .unwrap();
    assert!(matches!(store.apply(4, &cancel), Err(Error::Storage(_))));
    assert_eq!(store.applied_index().unwrap(), 3);
    assert!(store.receipt(&cancel).is_err());
}

#[test]
fn jobs_schema_mismatch_and_missing_index_refuse_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let store = JobStore::open(&path, deployment(), StoreLimits::default()).unwrap();
    let write = store.database.begin_write().unwrap();
    let mut meta: Metadata = read(&write, META, "state").unwrap().unwrap();
    meta.schema = SCHEMA + 1;
    put(&write, META, "state", &meta).unwrap();
    write.commit().unwrap();
    drop(store);
    assert!(matches!(
        JobStore::open(&path, deployment(), StoreLimits::default()),
        Err(Error::IncompatibleStore)
    ));
    let database = Database::create(&path).unwrap();
    let write = database.begin_write().unwrap();
    meta.schema = SCHEMA;
    put(&write, META, "state", &meta).unwrap();
    write.delete_table(DUE).unwrap();
    write.commit().unwrap();
    drop(database);
    assert!(matches!(
        JobStore::open(&path, deployment(), StoreLimits::default()),
        Err(Error::Storage(_))
    ));
}

fn missing_metadata_cannot_reinitialize(delete_table: bool) {
    use redb::TableHandle;
    fn inventory(path: &std::path::Path) -> (Vec<String>, Option<Vec<u8>>) {
        let database = Database::open(path).unwrap();
        let tx = database.begin_write().unwrap();
        let names = tx
            .list_tables()
            .unwrap()
            .map(|t| t.name().to_owned())
            .collect::<Vec<_>>();
        let state = if names.iter().any(|name| name == META.name()) {
            tx.open_table(META)
                .unwrap()
                .get("state")
                .unwrap()
                .map(|value| value.value().to_vec())
        } else {
            None
        };
        (names, state)
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let store = JobStore::open(&path, deployment(), StoreLimits::default()).unwrap();
    let write = store.database.begin_write().unwrap();
    if delete_table {
        write.delete_table(META).unwrap();
    } else {
        write.open_table(META).unwrap().remove("state").unwrap();
    }
    write.commit().unwrap();
    drop(store);
    let before = inventory(&path);
    let result = JobStore::open(
        &path,
        DeploymentId::new("wrong-deployment").unwrap(),
        StoreLimits {
            max_jobs: 7,
            ..StoreLimits::default()
        },
    );
    let rejected = matches!(result, Err(Error::Storage(_)));
    drop(result);
    let unchanged = before == inventory(&path);
    assert_eq!(
        (rejected, unchanged),
        (true, true),
        "missing metadata must reject wrong deployment and limits without schema mutation"
    );
}
#[test]
fn jobs_missing_metadata_row_in_unused_store_cannot_reinitialize() {
    missing_metadata_cannot_reinitialize(false);
}
#[test]
fn jobs_missing_metadata_table_in_unused_store_cannot_reinitialize() {
    missing_metadata_cannot_reinitialize(true);
}

fn replay_rows(store: &JobStore, limit: u64) {
    use redb::TableHandle;
    let read = store.database.begin_read().unwrap();
    let receipts = read.open_table(REQUESTS).unwrap().len().unwrap();
    assert!(receipts <= limit);
    assert_eq!(receipts, store.status().unwrap().retained_receipts);
    assert_eq!(read.open_table(META).unwrap().len().unwrap(), 1);
    assert!(read.open_table(STREAMS).unwrap().len().unwrap() <= 1);
    assert!(!read
        .list_tables()
        .unwrap()
        .any(|t| t.name() == "jobs_applied_v1"));
}
fn commit_next(store: &JobStore, command: &Command) -> Receipt {
    let index = store.applied_index().unwrap() + 1;
    let receipt = store.apply(index, command).unwrap();
    assert_eq!(store.applied_index().unwrap(), index);
    receipt
}

#[test]
fn jobs_many_commands_keep_receipt_and_checkpoint_rows_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let limits = StoreLimits {
        max_streams: 1,
        receipts_per_stream: 2,
        max_jobs: 1,
        ..StoreLimits::default()
    };
    let mut store = JobStore::open(&path, deployment(), limits.clone()).unwrap();
    commit_next(&store, &configure()).result.unwrap();
    for generation in 1..=64 {
        let stream = ClientStream::new(1, generation).unwrap();
        commit_next(&store, &Command::Register { stream })
            .result
            .unwrap();
        let Command::Request { operation, .. } = submit() else {
            panic!("request")
        };
        let request = Command::Request {
            stream,
            sequence: 1,
            operation: operation.clone(),
        };
        let receipt = commit_next(&store, &request);
        if generation == 1 {
            receipt.result.as_ref().unwrap();
        } else {
            assert_eq!(receipt.result, Err(Rejection::JobLimit));
        }
        for _ in 0..16 {
            assert_eq!(commit_next(&store, &request), receipt);
            replay_rows(&store, 2);
        }
        let second = Command::Request {
            stream,
            sequence: 2,
            operation: operation.clone(),
        };
        let negative = commit_next(&store, &second);
        assert_eq!(negative.result, Err(Rejection::JobLimit));
        assert_eq!(commit_next(&store, &second), negative);
        let third = Command::Request {
            stream,
            sequence: 3,
            operation,
        };
        assert_eq!(
            commit_next(&store, &third).result,
            Err(Rejection::NotAdmitted(AdmissionRejection::WindowFull))
        );
        replay_rows(&store, 2);
        commit_next(&store, &Command::Retire { stream, through: 1 })
            .result
            .unwrap();
        assert_eq!(commit_next(&store, &third).result, Err(Rejection::JobLimit));
        commit_next(&store, &Command::Retire { stream, through: 3 })
            .result
            .unwrap();
        replay_rows(&store, 0);
        commit_next(&store, &Command::Close { stream })
            .result
            .unwrap();
        assert_eq!(
            commit_next(&store, &request).result,
            Err(Rejection::NotAdmitted(AdmissionRejection::RetiredStream))
        );
        if generation % 8 == 0 {
            drop(store);
            store = JobStore::open(&path, deployment(), limits.clone()).unwrap();
            replay_rows(&store, 0);
        }
    }
    assert_eq!(
        store
            .queue(&QueueId::new("mail").unwrap())
            .unwrap()
            .unwrap()
            .retained_jobs,
        1
    );
    assert!(store.applied_index().unwrap() > 1600);
}

#[test]
fn jobs_retirement_commit_errors_preserve_receipt_or_watermark_atomically() {
    for fault_kind in [2, 3] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("jobs.redb");
        let limits = StoreLimits {
            max_streams: 1,
            receipts_per_stream: 1,
            ..StoreLimits::default()
        };
        let stream = ClientStream::new(1, 1).unwrap();
        let store = JobStore::open(&path, deployment(), limits.clone()).unwrap();
        store.apply(1, &configure()).unwrap().result.unwrap();
        store
            .apply(2, &Command::Register { stream })
            .unwrap()
            .result
            .unwrap();
        store.apply(3, &submit()).unwrap().result.unwrap();
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
        let retire = Command::Retire { stream, through: 1 };
        fault.store(fault_kind, Ordering::SeqCst);
        assert!(matches!(store.apply(4, &retire), Err(Error::Storage(_))));
        assert_eq!(fault.load(Ordering::SeqCst), 0);
        drop(store);
        let store = JobStore::open(&path, deployment(), limits).unwrap();
        match store.receipt(&submit()).unwrap() {
            ReceiptLookup::Recorded(_) => {
                assert_eq!(store.applied_index().unwrap(), 3);
                assert_eq!(store.status().unwrap().retained_receipts, 1);
            }
            ReceiptLookup::Retired => {
                assert_eq!(store.applied_index().unwrap(), 4);
                assert_eq!(store.status().unwrap().retained_receipts, 0);
            }
            result => panic!("partial retirement {result:?}"),
        }
        replay_rows(&store, 1);
        store.apply(4, &retire).unwrap().result.unwrap();
        assert_eq!(
            store.apply(5, &submit()).unwrap().result,
            Err(Rejection::NotAdmitted(AdmissionRejection::RetiredRequest))
        );
        replay_rows(&store, 0);
        assert!(store.job(&JobId::new("a").unwrap()).unwrap().is_some());
    }
}

#[test]
fn jobs_old_schema_rejection_does_not_create_new_tables() {
    use redb::TableHandle;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let db = Database::create(&path).unwrap();
    let write = db.begin_write().unwrap();
    write
        .open_table(META)
        .unwrap()
        .insert("state", br#"{"schema":1}"#.as_slice())
        .unwrap();
    write.commit().unwrap();
    drop(db);
    assert!(matches!(
        JobStore::open(&path, deployment(), StoreLimits::default()),
        Err(Error::IncompatibleStore)
    ));
    let db = Database::open(&path).unwrap();
    assert_eq!(
        db.begin_read()
            .unwrap()
            .list_tables()
            .unwrap()
            .map(|t| t.name().to_owned())
            .collect::<Vec<_>>(),
        vec![META.name().to_owned()]
    );
}

#[test]
fn jobs_sequence_and_generation_exhaustion_never_wrap() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let limits = StoreLimits {
        max_streams: 1,
        receipts_per_stream: 1,
        ..StoreLimits::default()
    };
    let store = JobStore::open(&path, deployment(), limits).unwrap();
    commit_next(&store, &configure()).result.unwrap();
    let stream = ClientStream::new(1, u64::MAX).unwrap();
    let write = store.database.begin_write().unwrap();
    put(
        &write,
        STREAMS,
        "1",
        &StreamStatus {
            generation: u64::MAX,
            phase: StreamPhase::Open {
                retired_through: u64::MAX - 1,
                last_sequence: u64::MAX - 1,
            },
        },
    )
    .unwrap();
    write.commit().unwrap();
    let Command::Request { operation, .. } = submit() else {
        panic!("request")
    };
    let request = Command::Request {
        stream,
        sequence: u64::MAX,
        operation,
    };
    let receipt = commit_next(&store, &request);
    receipt.result.as_ref().unwrap();
    assert_eq!(commit_next(&store, &request), receipt);
    commit_next(
        &store,
        &Command::Retire {
            stream,
            through: u64::MAX,
        },
    )
    .result
    .unwrap();
    commit_next(&store, &Command::Close { stream })
        .result
        .unwrap();
    assert_eq!(
        commit_next(&store, &Command::Register { stream }).result,
        Err(Rejection::NotAdmitted(AdmissionRejection::RetiredStream))
    );
    assert_eq!(
        commit_next(
            &store,
            &Command::Register {
                stream: ClientStream::new(1, 1).unwrap()
            }
        )
        .result,
        Err(Rejection::NotAdmitted(AdmissionRejection::RetiredStream))
    );
    commit_next(
        &store,
        &Command::internal(Operation::Tick {
            tick: 1,
            utc_ms: 1,
            expire_limit: 1,
        })
        .unwrap(),
    )
    .result
    .unwrap();
    replay_rows(&store, 0);
}
