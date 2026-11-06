use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::Path;

use redb::{
    Database, Durability, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition,
    WriteTransaction,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::model::*;

const META: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_meta_v1");
const JOBS: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_rows_v1");
const QUEUES: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_queues_v1");
const REQUESTS: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_requests_v1");
const UNIQUE: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_unique_v1");
const DUE: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_due_v1");
const LEASES: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_leases_v1");
const BY_QUEUE: TableDefinition<&str, &[u8]> = TableDefinition::new("jobs_by_queue_v1");
const APPLIED: TableDefinition<u64, &[u8]> = TableDefinition::new("jobs_applied_v1");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Metadata {
    schema: u32,
    deployment: DeploymentId,
    limits: StoreLimits,
    last_index: u64,
    commands: u64,
    retained_jobs: u64,
    queues: u64,
    tick: u64,
    utc_ms: u64,
}
#[derive(Serialize, Deserialize)]
struct Applied {
    hash: [u8; 32],
    receipt: Receipt,
}

pub(crate) struct JobStore {
    database: Database,
}

#[derive(Clone, Debug)]
pub(crate) struct DueCursor(String);
#[derive(Debug)]
pub(crate) struct DuePage {
    pub(crate) jobs: Vec<Job>,
    pub(crate) next: Option<DueCursor>,
}

struct Changes {
    jobs: Vec<(Option<Job>, Job)>,
    queues: BTreeMap<QueueId, Queue>,
    clock: Option<(u64, u64)>,
}
impl Changes {
    fn new() -> Self {
        Self {
            jobs: Vec::new(),
            queues: BTreeMap::new(),
            clock: None,
        }
    }
    fn replace(&mut self, previous: Job, next: Job, mut queue: Queue) -> Result<(), Rejection> {
        if previous.active() != next.active() {
            queue.active_leases = if next.active() {
                queue.active_leases.checked_add(1)
            } else {
                queue.active_leases.checked_sub(1)
            }
            .ok_or(Rejection::Arithmetic)?;
        }
        self.queues.insert(next.spec.queue.clone(), queue);
        self.jobs.push((Some(previous), next));
        Ok(())
    }
}

impl JobStore {
    pub(crate) fn open(
        path: &Path,
        deployment: DeploymentId,
        limits: StoreLimits,
    ) -> Result<Self, Error> {
        limits.validate()?;
        if path.as_os_str().is_empty() {
            return Err(Error::Invalid("store path".into()));
        }
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::Invalid("store symlink".into()))
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(storage_error(e)),
            _ => {}
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path).map_err(storage_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(storage_error)?;
        }
        let database = Database::builder()
            .create_file(file)
            .map_err(storage_error)?;
        let mut write = database.begin_write().map_err(storage_error)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(storage_error)?;
        let existing: Option<Metadata> = read(&write, META, "state")?;
        if let Some(meta) = existing {
            if meta.schema != 1 || meta.deployment != deployment || meta.limits != limits {
                return Err(Error::IncompatibleStore);
            }
            // Opening existing tables must not silently repair a missing authoritative index.
            let names = write
                .list_tables()
                .map_err(storage_error)?
                .map(|t| {
                    use redb::TableHandle;
                    t.name().to_owned()
                })
                .collect::<BTreeSet<_>>();
            for name in [
                "jobs_rows_v1",
                "jobs_queues_v1",
                "jobs_requests_v1",
                "jobs_unique_v1",
                "jobs_due_v1",
                "jobs_leases_v1",
                "jobs_by_queue_v1",
                "jobs_applied_v1",
            ] {
                if !names.contains(name) {
                    return Err(Error::Storage(format!("missing table {name}")));
                }
            }
        } else {
            for table in [JOBS, QUEUES, REQUESTS, UNIQUE, DUE, LEASES, BY_QUEUE] {
                if write
                    .open_table(table)
                    .map_err(storage_error)?
                    .len()
                    .map_err(storage_error)?
                    != 0
                {
                    return Err(Error::Storage("missing store metadata".into()));
                }
            }
            if write
                .open_table(APPLIED)
                .map_err(storage_error)?
                .len()
                .map_err(storage_error)?
                != 0
            {
                return Err(Error::Storage("missing store metadata".into()));
            }
            put(
                &write,
                META,
                "state",
                &Metadata {
                    schema: 1,
                    deployment,
                    limits,
                    last_index: 0,
                    commands: 0,
                    retained_jobs: 0,
                    queues: 0,
                    tick: 0,
                    utc_ms: 0,
                },
            )?;
        }
        write.commit().map_err(storage_error)?;
        #[cfg(unix)]
        std::fs::File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .and_then(|directory| directory.sync_all())
        .map_err(storage_error)?;
        Ok(Self { database })
    }

    pub(crate) fn apply(&self, index: u64, command: &Command) -> Result<Receipt, Error> {
        command.validate()?;
        let hash = command.hash()?;
        let mut write = self.database.begin_write().map_err(storage_error)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(storage_error)?;
        let mut meta: Metadata = read(&write, META, "state")?
            .ok_or_else(|| Error::Storage("missing metadata".into()))?;
        if index <= meta.last_index {
            let table = write.open_table(APPLIED).map_err(storage_error)?;
            let record = table
                .get(index)
                .map_err(storage_error)?
                .ok_or(Error::LogConflict)?;
            let applied: Applied = decode(record.value())?;
            return if applied.hash == hash {
                Ok(applied.receipt)
            } else {
                Err(Error::LogConflict)
            };
        }
        if meta.commands >= meta.limits.max_commands {
            return Err(Error::AdmissionFull);
        }
        let request_key = command.key();
        let prior: Option<Applied> = read(&write, REQUESTS, &request_key)?;
        let receipt = if let Some(prior) = &prior {
            if prior.hash == hash {
                prior.receipt.clone()
            } else {
                Receipt {
                    index,
                    result: Err(Rejection::RequestConflict),
                }
            }
        } else {
            let mut changes = Changes::new();
            let result = self.plan(&write, &meta, index, &command.operation, &mut changes)?;
            if result.is_ok() {
                persist(&write, &mut meta, changes)?;
            }
            Receipt { index, result }
        };
        let applied = Applied {
            hash,
            receipt: receipt.clone(),
        };
        if prior.is_none() {
            put(&write, REQUESTS, &request_key, &applied)?;
        }
        let encoded = encode(&applied)?;
        write
            .open_table(APPLIED)
            .map_err(storage_error)?
            .insert(index, encoded.as_slice())
            .map_err(storage_error)?;
        meta.last_index = index;
        meta.commands += 1;
        put(&write, META, "state", &meta)?;
        write.commit().map_err(storage_error)?;
        Ok(receipt)
    }

    fn plan(
        &self,
        write: &WriteTransaction,
        meta: &Metadata,
        index: u64,
        operation: &Operation,
        changes: &mut Changes,
    ) -> Result<Result<Effect, Rejection>, Error> {
        match operation {
            Operation::ConfigureQueue { queue, policy } => {
                let existing: Option<Queue> = read(write, QUEUES, queue.as_str())?;
                if existing.is_none() && meta.queues >= meta.limits.max_queues {
                    return Ok(Err(Rejection::QueueLimit));
                }
                let mut row = existing.unwrap_or(Queue {
                    policy: policy.clone(),
                    paused: false,
                    active_leases: 0,
                    retained_jobs: 0,
                });
                row.policy = policy.clone();
                changes.queues.insert(queue.clone(), row);
                Ok(Ok(Effect::QueueConfigured))
            }
            Operation::SetPaused { queue, paused } => {
                let Some(mut row) = read::<Queue>(write, QUEUES, queue.as_str())? else {
                    return Ok(Err(Rejection::UnknownQueue));
                };
                row.paused = *paused;
                changes.queues.insert(queue.clone(), row);
                Ok(Ok(Effect::QueueConfigured))
            }
            Operation::Enqueue { jobs } => {
                if meta.retained_jobs + jobs.len() as u64 > meta.limits.max_jobs {
                    return Ok(Err(Rejection::JobLimit));
                }
                let mut ids = BTreeSet::new();
                let mut unique = BTreeSet::new();
                for spec in jobs {
                    if !ids.insert(spec.id.clone())
                        || read::<Job>(write, JOBS, spec.id.as_str())?.is_some()
                    {
                        return Ok(Err(Rejection::DuplicateJob));
                    }
                    if let Some(key) = unique_key(spec) {
                        if !unique.insert(key.clone())
                            || read::<JobId>(write, UNIQUE, &key)?.is_some()
                        {
                            return Ok(Err(Rejection::DuplicateUnique));
                        }
                    }
                    let queue = match changes.queues.get(&spec.queue) {
                        Some(queue) => queue.clone(),
                        None => {
                            let Some(queue) = read::<Queue>(write, QUEUES, spec.queue.as_str())?
                            else {
                                return Ok(Err(Rejection::UnknownQueue));
                            };
                            queue
                        }
                    };
                    changes.queues.insert(
                        spec.queue.clone(),
                        Queue {
                            retained_jobs: queue.retained_jobs + 1,
                            ..queue
                        },
                    );
                    changes
                        .jobs
                        .push((None, Job::admitted(spec.clone(), index, meta.tick)));
                }
                Ok(Ok(Effect::Enqueued(
                    jobs.iter().map(|j| j.id.clone()).collect(),
                )))
            }
            Operation::Tick {
                tick,
                utc_ms,
                expire_limit,
            } => {
                if *tick < meta.tick {
                    return Ok(Err(Rejection::ClockRegression));
                }
                let utc_ms = meta.utc_ms.max(*utc_ms);
                let mut expired = Vec::new();
                let upper = format!("{tick:020}/~");
                let ids = index_ids(write, LEASES, "", &upper, *expire_limit)?;
                for id in ids {
                    let previous = read::<Job>(write, JOBS, id.as_str())?
                        .ok_or_else(|| Error::Storage("dangling lease index".into()))?;
                    let queue = match changes.queues.get(&previous.spec.queue) {
                        Some(q) => q.clone(),
                        None => required_queue(write, &previous.spec.queue)?,
                    };
                    let ctx = Context {
                        index,
                        tick: *tick,
                        utc_ms,
                        queue: &queue,
                    };
                    let next = match previous.transition(operation, &ctx) {
                        Ok(next) => next,
                        Err(error) => return Ok(Err(error)),
                    };
                    expired.push(id);
                    if let Err(error) = changes.replace(previous, next, queue) {
                        return Ok(Err(error));
                    }
                }
                changes.clock = Some((*tick, utc_ms));
                Ok(Ok(Effect::Ticked { expired }))
            }
            _ => {
                let id = match operation {
                    Operation::Claim { id, .. }
                    | Operation::Cancel { id, .. }
                    | Operation::Retry { id, .. }
                    | Operation::Discard { id, .. } => id,
                    Operation::Renew { token } | Operation::Complete { token, .. } => &token.job,
                    _ => unreachable!(),
                };
                let Some(previous) = read::<Job>(write, JOBS, id.as_str())? else {
                    return Ok(Err(Rejection::UnknownJob));
                };
                let queue = required_queue(write, &previous.spec.queue)?;
                let ctx = Context {
                    index,
                    tick: meta.tick,
                    utc_ms: meta.utc_ms,
                    queue: &queue,
                };
                let next = match previous.transition(operation, &ctx) {
                    Ok(next) => next,
                    Err(error) => return Ok(Err(error)),
                };
                let effect = Effect::Changed {
                    id: id.clone(),
                    revision: next.revision,
                    state: next.state.clone(),
                };
                if let Err(error) = changes.replace(previous, next, queue) {
                    return Ok(Err(error));
                }
                Ok(Ok(effect))
            }
        }
    }

    pub(crate) fn job(&self, id: &JobId) -> Result<Option<Job>, Error> {
        self.read_row(JOBS, id.as_str())
    }
    pub(crate) fn queue(&self, id: &QueueId) -> Result<Option<Queue>, Error> {
        self.read_row(QUEUES, id.as_str())
    }
    pub(crate) fn status(&self) -> Result<StoreStatus, Error> {
        let meta: Metadata = self
            .read_row(META, "state")?
            .ok_or_else(|| Error::Storage("missing metadata".into()))?;
        Ok(StoreStatus {
            applied_index: meta.last_index,
            tick: meta.tick,
            utc_ms: meta.utc_ms,
            remaining_commands: meta
                .limits
                .max_commands
                .checked_sub(meta.commands)
                .ok_or_else(|| Error::Storage("command count exceeds bound".into()))?,
            remaining_jobs: meta
                .limits
                .max_jobs
                .checked_sub(meta.retained_jobs)
                .ok_or_else(|| Error::Storage("job count exceeds bound".into()))?,
            remaining_queues: meta
                .limits
                .max_queues
                .checked_sub(meta.queues)
                .ok_or_else(|| Error::Storage("queue count exceeds bound".into()))?,
        })
    }

    pub(crate) fn applied_index(&self) -> Result<u64, Error> {
        Ok(self
            .read_row::<Metadata>(META, "state")?
            .ok_or_else(|| Error::Storage("missing metadata".into()))?
            .last_index)
    }
    pub(crate) fn clock(&self) -> Result<(u64, u64), Error> {
        let meta: Metadata = self
            .read_row(META, "state")?
            .ok_or_else(|| Error::Storage("missing metadata".into()))?;
        Ok((meta.tick, meta.utc_ms))
    }
    pub(crate) fn receipt(&self, command: &Command) -> Result<Option<Receipt>, Error> {
        command.validate()?;
        let record: Option<Applied> = self.read_row(REQUESTS, &command.key())?;
        record
            .map(|record| {
                if record.hash != command.hash()? {
                    return Err(Error::Invalid("request body conflicts".into()));
                }
                Ok(record.receipt)
            })
            .transpose()
    }
    fn read_row<T: DeserializeOwned>(
        &self,
        definition: TableDefinition<&str, &[u8]>,
        key: &str,
    ) -> Result<Option<T>, Error> {
        let read = self.database.begin_read().map_err(storage_error)?;
        let table = read.open_table(definition).map_err(storage_error)?;
        let value = table.get(key).map_err(storage_error)?;
        value.map(|value| decode(value.value())).transpose()
    }
    pub(crate) fn inspect(
        &self,
        queue: &QueueId,
        after: Option<&JobId>,
        limit: usize,
    ) -> Result<Vec<Job>, Error> {
        page_limit(limit)?;
        let read = self.database.begin_read().map_err(storage_error)?;
        let table = read.open_table(BY_QUEUE).map_err(storage_error)?;
        let rows = read.open_table(JOBS).map_err(storage_error)?;
        let prefix = format!("{}/", queue.as_str());
        let lower = after.map(|id| format!("{prefix}{}", id.as_str()));
        let start = lower
            .as_deref()
            .map(Bound::Excluded)
            .unwrap_or(Bound::Included(prefix.as_str()));
        let upper = format!("{prefix}~");
        let mut jobs = Vec::new();
        for entry in table
            .range::<&str>((start, Bound::Excluded(upper.as_str())))
            .map_err(storage_error)?
            .take(limit)
        {
            let (_, id) = entry.map_err(storage_error)?;
            let id: JobId = decode(id.value())?;
            let row = rows
                .get(id.as_str())
                .map_err(storage_error)?
                .ok_or_else(|| Error::Storage("dangling queue index".into()))?;
            jobs.push(decode(row.value())?);
        }
        Ok(jobs)
    }
    pub(crate) fn due(
        &self,
        queue: &QueueId,
        after: Option<&DueCursor>,
        limit: usize,
    ) -> Result<DuePage, Error> {
        page_limit(limit)?;
        let read = self.database.begin_read().map_err(storage_error)?;
        let meta_table = read.open_table(META).map_err(storage_error)?;
        let meta: Metadata = decode(
            meta_table
                .get("state")
                .map_err(storage_error)?
                .ok_or_else(|| Error::Storage("missing metadata".into()))?
                .value(),
        )?;
        let table = read.open_table(DUE).map_err(storage_error)?;
        let rows = read.open_table(JOBS).map_err(storage_error)?;
        let prefix = format!("{}/", queue.as_str());
        if after.is_some_and(|cursor| !cursor.0.starts_with(&prefix)) {
            return Err(Error::Invalid("cursor queue".into()));
        }
        let lower = after
            .map(|c| Bound::Excluded(c.0.as_str()))
            .unwrap_or(Bound::Included(prefix.as_str()));
        let upper = format!("{}{utc:020}/~", prefix, utc = meta.utc_ms);
        let mut jobs = Vec::new();
        let mut last = None;
        let mut scanned = 0;
        for entry in table
            .range::<&str>((lower, Bound::Excluded(upper.as_str())))
            .map_err(storage_error)?
            .take(limit)
        {
            let (key, value) = entry.map_err(storage_error)?;
            last = Some(DueCursor(key.value().to_owned()));
            scanned += 1;
            let id: JobId = decode(value.value())?;
            let row = rows
                .get(id.as_str())
                .map_err(storage_error)?
                .ok_or_else(|| Error::Storage("dangling due index".into()))?;
            let job: Job = decode(row.value())?;
            if job.ready_at_tick <= meta.tick {
                jobs.push(job);
            }
        }
        Ok(DuePage {
            jobs,
            next: if scanned == limit { last } else { None },
        })
    }
    pub(crate) fn history(
        &self,
        id: &JobId,
        after: Revision,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>, Error> {
        page_limit(limit)?;
        Ok(self
            .job(id)?
            .map(|j| {
                j.history
                    .into_iter()
                    .filter(|h| h.revision.0 > after.0)
                    .take(limit)
                    .collect()
            })
            .unwrap_or_default())
    }
}

fn persist(write: &WriteTransaction, meta: &mut Metadata, changes: Changes) -> Result<(), Error> {
    for (id, queue) in changes.queues {
        if read::<Queue>(write, QUEUES, id.as_str())?.is_none() {
            meta.queues += 1;
        }
        put(write, QUEUES, id.as_str(), &queue)?;
    }
    for (previous, job) in changes.jobs {
        if let Some(previous) = previous {
            if let Some(key) = due_key(&previous) {
                remove(write, DUE, &key)?;
            }
            if let Some(key) = lease_key(&previous) {
                remove(write, LEASES, &key)?;
            }
        } else {
            meta.retained_jobs += 1;
            put(
                write,
                BY_QUEUE,
                &format!("{}/{}", job.spec.queue.as_str(), job.spec.id.as_str()),
                &job.spec.id,
            )?;
            if let Some(key) = unique_key(&job.spec) {
                put(write, UNIQUE, &key, &job.spec.id)?;
            }
        }
        if let Some(key) = due_key(&job) {
            put(write, DUE, &key, &job.spec.id)?;
        }
        if let Some(key) = lease_key(&job) {
            put(write, LEASES, &key, &job.spec.id)?;
        }
        put(write, JOBS, job.spec.id.as_str(), &job)?;
    }
    if let Some((tick, utc_ms)) = changes.clock {
        meta.tick = tick;
        meta.utc_ms = utc_ms;
    }
    Ok(())
}
fn due_key(job: &Job) -> Option<String> {
    matches!(job.state, JobState::Pending | JobState::Blocked(_)).then(|| {
        format!(
            "{}/{:020}/{}",
            job.spec.queue.as_str(),
            job.spec.scheduled_at_ms,
            job.spec.id.as_str()
        )
    })
}
fn lease_key(job: &Job) -> Option<String> {
    match &job.state {
        JobState::Leased(lease) => Some(format!(
            "{:020}/{}",
            lease.expires_at_tick,
            job.spec.id.as_str()
        )),
        _ => None,
    }
}
fn unique_key(job: &NewJob) -> Option<String> {
    job.unique
        .as_ref()
        .map(|key| format!("{}/{}", job.queue.as_str(), key.as_str()))
}
fn required_queue(write: &WriteTransaction, id: &QueueId) -> Result<Queue, Error> {
    read(write, QUEUES, id.as_str())?
        .ok_or_else(|| Error::Storage("job references missing queue".into()))
}
fn page_limit(limit: usize) -> Result<(), Error> {
    if !(1..=MAX_PAGE).contains(&limit) {
        return Err(Error::Invalid("page limit".into()));
    }
    Ok(())
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(storage_error)
}
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(storage_error)
}
fn read<T: DeserializeOwned>(
    write: &WriteTransaction,
    definition: TableDefinition<&str, &[u8]>,
    key: &str,
) -> Result<Option<T>, Error> {
    let table = write.open_table(definition).map_err(storage_error)?;
    let value = table.get(key).map_err(storage_error)?;
    value.map(|value| decode(value.value())).transpose()
}
fn put(
    write: &WriteTransaction,
    definition: TableDefinition<&str, &[u8]>,
    key: &str,
    value: &impl Serialize,
) -> Result<(), Error> {
    let bytes = encode(value)?;
    write
        .open_table(definition)
        .map_err(storage_error)?
        .insert(key, bytes.as_slice())
        .map_err(storage_error)?;
    Ok(())
}
fn remove(
    write: &WriteTransaction,
    definition: TableDefinition<&str, &[u8]>,
    key: &str,
) -> Result<(), Error> {
    write
        .open_table(definition)
        .map_err(storage_error)?
        .remove(key)
        .map_err(storage_error)?;
    Ok(())
}
fn index_ids(
    write: &WriteTransaction,
    definition: TableDefinition<&str, &[u8]>,
    lower: &str,
    upper: &str,
    limit: usize,
) -> Result<Vec<JobId>, Error> {
    let table = write.open_table(definition).map_err(storage_error)?;
    table
        .range(lower..upper)
        .map_err(storage_error)?
        .take(limit)
        .map(|entry| {
            let (_, value) = entry.map_err(storage_error)?;
            decode(value.value())
        })
        .collect()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
