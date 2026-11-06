use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MAX_ARGS: usize = 64 * 1024;
pub(crate) const MAX_BATCH: usize = 128;
pub(crate) const MAX_COMMAND: usize = 1024 * 1024;
pub(crate) const MAX_PAGE: usize = 128;
pub(crate) const HISTORY_LIMIT: usize = 32;
pub(crate) const MAX_TIME: u64 = i64::MAX as u64;
const MAX_DELAY: u64 = 1_000_000;

#[derive(Debug)]
pub(crate) enum Error {
    Invalid(String),
    Storage(String),
    IncompatibleStore,
    LogConflict,
    AdmissionFull,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub(crate) struct $name(String);
        impl $name {
            pub(crate) fn new(value: impl Into<String>) -> Result<Self, Error> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > 96
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                {
                    return Err(Error::Invalid(stringify!($name).into()));
                }
                Ok(Self(value))
            }
            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(value: String) -> Result<Self, Error> {
                Self::new(value)
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }
    };
}
identifier!(DeploymentId);
identifier!(JobId);
identifier!(QueueId);
identifier!(ClientId);
identifier!(RequestId);
identifier!(WorkerName);
identifier!(SchemaId);
identifier!(WorkerSession);
identifier!(UniquenessKey);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub(crate) struct EncodedArgs(Vec<u8>);
impl EncodedArgs {
    pub(crate) fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() > MAX_ARGS {
            return Err(Error::Invalid("arguments too large".into()));
        }
        Ok(Self(bytes))
    }
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl TryFrom<Vec<u8>> for EncodedArgs {
    type Error = Error;
    fn try_from(value: Vec<u8>) -> Result<Self, Error> {
        Self::new(value)
    }
}
impl From<EncodedArgs> for Vec<u8> {
    fn from(value: EncodedArgs) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkerKey {
    name: WorkerName,
    version: std::num::NonZeroU32,
    schema: SchemaId,
}
impl WorkerKey {
    pub(crate) fn name(&self) -> &WorkerName {
        &self.name
    }
    pub(crate) fn version(&self) -> u32 {
        self.version.get()
    }
    pub(crate) fn schema(&self) -> &SchemaId {
        &self.schema
    }
    pub(crate) fn new(name: &str, version: u32, schema: &str) -> Result<Self, Error> {
        Ok(Self {
            name: WorkerName::new(name)?,
            version: std::num::NonZeroU32::new(version)
                .ok_or_else(|| Error::Invalid("worker version".into()))?,
            schema: SchemaId::new(schema)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "(u32, u64)", into = "(u32, u64)")]
pub(crate) struct QueuePolicy {
    pub(super) concurrency: u32,
    pub(super) lease_ticks: u64,
}
impl QueuePolicy {
    pub(crate) fn concurrency(&self) -> u32 {
        self.concurrency
    }
    pub(crate) fn lease_ticks(&self) -> u64 {
        self.lease_ticks
    }
    pub(crate) fn new(concurrency: u32, lease_ticks: u64) -> Result<Self, Error> {
        if !(1..=1024).contains(&concurrency) || !(1..=MAX_DELAY).contains(&lease_ticks) {
            return Err(Error::Invalid("queue policy".into()));
        }
        Ok(Self {
            concurrency,
            lease_ticks,
        })
    }
}
impl TryFrom<(u32, u64)> for QueuePolicy {
    type Error = Error;
    fn try_from(v: (u32, u64)) -> Result<Self, Error> {
        Self::new(v.0, v.1)
    }
}
impl From<QueuePolicy> for (u32, u64) {
    fn from(v: QueuePolicy) -> Self {
        (v.concurrency, v.lease_ticks)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "(u16, u64, u64)", into = "(u16, u64, u64)")]
pub(crate) struct RetryPolicy {
    max_attempts: u16,
    base_ticks: u64,
    max_ticks: u64,
}
impl RetryPolicy {
    pub(crate) fn max_attempts(&self) -> u16 {
        self.max_attempts
    }
    pub(crate) fn base_ticks(&self) -> u64 {
        self.base_ticks
    }
    pub(crate) fn max_ticks(&self) -> u64 {
        self.max_ticks
    }
    pub(crate) fn new(max_attempts: u16, base_ticks: u64, max_ticks: u64) -> Result<Self, Error> {
        if !(1..=100).contains(&max_attempts)
            || base_ticks == 0
            || base_ticks > max_ticks
            || max_ticks > MAX_DELAY
        {
            return Err(Error::Invalid("retry policy".into()));
        }
        Ok(Self {
            max_attempts,
            base_ticks,
            max_ticks,
        })
    }
    fn delay(&self, attempts: u16) -> u64 {
        self.base_ticks
            .saturating_mul(1u64 << u32::from(attempts.saturating_sub(1)).min(63))
            .min(self.max_ticks)
    }
}
impl TryFrom<(u16, u64, u64)> for RetryPolicy {
    type Error = Error;
    fn try_from(v: (u16, u64, u64)) -> Result<Self, Error> {
        Self::new(v.0, v.1, v.2)
    }
}
impl From<RetryPolicy> for (u16, u64, u64) {
    fn from(v: RetryPolicy) -> Self {
        (v.max_attempts, v.base_ticks, v.max_ticks)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Revision(pub(crate) u64);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunGeneration(pub(crate) u64);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct NewJob {
    pub(crate) id: JobId,
    pub(crate) queue: QueueId,
    pub(crate) worker: WorkerKey,
    pub(crate) args: EncodedArgs,
    pub(crate) scheduled_at_ms: u64,
    pub(crate) retry: RetryPolicy,
    pub(crate) unique: Option<UniquenessKey>,
}
impl NewJob {
    pub(crate) fn new(
        id: JobId,
        queue: QueueId,
        worker: WorkerKey,
        args: EncodedArgs,
        scheduled_at_ms: u64,
        retry: RetryPolicy,
        unique: Option<UniquenessKey>,
    ) -> Self {
        Self {
            id,
            queue,
            worker,
            args,
            scheduled_at_ms,
            retry,
            unique,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LeaseToken {
    pub(super) job: JobId,
    pub(super) run: RunGeneration,
    pub(super) claim_index: u64,
    pub(super) holder: WorkerSession,
}
impl LeaseToken {
    pub(crate) fn job(&self) -> &JobId {
        &self.job
    }
    pub(crate) fn run(&self) -> RunGeneration {
        self.run
    }
    pub(crate) fn claim_index(&self) -> u64 {
        self.claim_index
    }
    pub(crate) fn holder(&self) -> &WorkerSession {
        &self.holder
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Lease {
    pub(crate) token: LeaseToken,
    pub(crate) expires_at_tick: u64,
    pub(crate) cancel_requested: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum BlockReason {
    MissingWorker,
    IncompatibleWorker,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum TerminalOutcome {
    Completed,
    Cancelled,
    Discarded,
    Exhausted,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum JobState {
    Pending,
    Blocked(BlockReason),
    Leased(Lease),
    Terminal(TerminalOutcome),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct HistoryEntry {
    pub(crate) revision: Revision,
    pub(crate) tick: u64,
    pub(crate) run: RunGeneration,
    pub(crate) attempts: u16,
    pub(crate) state: JobState,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Job {
    pub(crate) spec: NewJob,
    pub(crate) revision: Revision,
    pub(crate) run: RunGeneration,
    pub(crate) attempts: u16,
    pub(crate) ready_at_tick: u64,
    pub(crate) state: JobState,
    pub(super) history: Vec<HistoryEntry>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Queue {
    pub(crate) policy: QueuePolicy,
    pub(crate) paused: bool,
    pub(crate) active_leases: u32,
    pub(crate) retained_jobs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Finish {
    Success,
    Failure,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Operation {
    ConfigureQueue {
        queue: QueueId,
        policy: QueuePolicy,
    },
    SetPaused {
        queue: QueueId,
        paused: bool,
    },
    Enqueue {
        jobs: Vec<NewJob>,
    },
    Claim {
        id: JobId,
        revision: Revision,
        holder: WorkerSession,
        worker: Option<WorkerKey>,
    },
    Renew {
        token: LeaseToken,
    },
    Complete {
        token: LeaseToken,
        outcome: Finish,
    },
    Cancel {
        id: JobId,
        revision: Revision,
    },
    Retry {
        id: JobId,
        revision: Revision,
    },
    Discard {
        id: JobId,
        revision: Revision,
    },
    Tick {
        tick: u64,
        utc_ms: u64,
        expire_limit: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Command {
    pub(super) client: ClientId,
    pub(super) request: RequestId,
    pub(super) operation: Operation,
}
impl Command {
    pub(crate) fn new(
        client: ClientId,
        request: RequestId,
        operation: Operation,
    ) -> Result<Self, Error> {
        let result = Self {
            client,
            request,
            operation,
        };
        result.validate()?;
        Ok(result)
    }
    pub(super) fn validate(&self) -> Result<(), Error> {
        match &self.operation {
            Operation::Enqueue { jobs } => {
                if jobs.is_empty()
                    || jobs.len() > MAX_BATCH
                    || jobs.iter().any(|j| j.scheduled_at_ms > MAX_TIME)
                {
                    return Err(Error::Invalid("batch bounds".into()));
                }
            }
            Operation::Tick {
                tick,
                utc_ms,
                expire_limit,
            } => {
                if *tick > MAX_TIME || *utc_ms > MAX_TIME || !(1..=MAX_PAGE).contains(expire_limit)
                {
                    return Err(Error::Invalid("tick bounds".into()));
                }
            }
            _ => {}
        }
        if serde_json::to_vec(self).map_err(storage_error)?.len() > MAX_COMMAND {
            return Err(Error::Invalid("command too large".into()));
        }
        Ok(())
    }
    pub(super) fn hash(&self) -> Result<[u8; 32], Error> {
        Ok(Sha256::digest(serde_json::to_vec(self).map_err(storage_error)?).into())
    }
    pub(super) fn key(&self) -> String {
        format!("{}/{}", self.client.as_str(), self.request.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Rejection {
    RequestConflict,
    UnknownQueue,
    UnknownJob,
    DuplicateJob,
    DuplicateUnique,
    JobLimit,
    QueueLimit,
    StaleRevision,
    StaleLease,
    NotReady,
    Paused,
    ConcurrencyLimit,
    InvalidState,
    ClockRegression,
    Arithmetic,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Effect {
    QueueConfigured,
    Enqueued(Vec<JobId>),
    Changed {
        id: JobId,
        revision: Revision,
        state: JobState,
    },
    Ticked {
        expired: Vec<JobId>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub(crate) index: u64,
    pub(crate) result: Result<Effect, Rejection>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoreStatus {
    pub(crate) applied_index: u64,
    pub(crate) tick: u64,
    pub(crate) utc_ms: u64,
    pub(crate) remaining_commands: u64,
    pub(crate) remaining_jobs: u64,
    pub(crate) remaining_queues: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoreLimits {
    pub(crate) max_jobs: u64,
    pub(crate) max_commands: u64,
    pub(crate) max_queues: u64,
}
impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_jobs: 100_000,
            max_commands: 1_000_000,
            max_queues: 1024,
        }
    }
}
impl StoreLimits {
    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.max_jobs == 0
            || self.max_commands == 0
            || self.max_queues == 0
            || self.max_jobs > 1_000_000
            || self.max_commands > 10_000_000
            || self.max_queues > 4096
        {
            return Err(Error::Invalid("store limits".into()));
        }
        Ok(())
    }
}

pub(super) fn storage_error(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

pub(super) struct Context<'a> {
    pub index: u64,
    pub tick: u64,
    pub utc_ms: u64,
    pub queue: &'a Queue,
}
impl Job {
    pub(super) fn admitted(spec: NewJob, index: u64, tick: u64) -> Self {
        let mut job = Self {
            spec,
            revision: Revision(index),
            run: RunGeneration(1),
            attempts: 0,
            ready_at_tick: tick,
            state: JobState::Pending,
            history: Vec::new(),
        };
        job.record(index, tick);
        job
    }
    pub(super) fn record(&mut self, index: u64, tick: u64) {
        self.revision = Revision(index);
        if self.history.len() == HISTORY_LIMIT {
            self.history.remove(0);
        }
        self.history.push(HistoryEntry {
            revision: self.revision,
            tick,
            run: self.run,
            attempts: self.attempts,
            state: self.state.clone(),
        });
    }
    pub(super) fn active(&self) -> bool {
        matches!(self.state, JobState::Leased(_))
    }
    fn check_revision(&self, revision: Revision) -> Result<(), Rejection> {
        if self.revision != revision {
            return Err(Rejection::StaleRevision);
        }
        Ok(())
    }
    fn lease(&self, token: &LeaseToken, tick: u64) -> Result<Lease, Rejection> {
        match &self.state {
            JobState::Leased(lease) if &lease.token == token && tick < lease.expires_at_tick => {
                Ok(lease.clone())
            }
            _ => Err(Rejection::StaleLease),
        }
    }
    fn failed(&mut self, tick: u64) -> Result<(), Rejection> {
        if self.attempts >= self.spec.retry.max_attempts {
            self.state = JobState::Terminal(TerminalOutcome::Exhausted);
        } else {
            self.ready_at_tick = add_time(tick, self.spec.retry.delay(self.attempts))?;
            self.state = JobState::Pending;
        }
        Ok(())
    }
    pub(super) fn transition(
        &self,
        operation: &Operation,
        ctx: &Context<'_>,
    ) -> Result<Self, Rejection> {
        let mut next = self.clone();
        match operation {
            Operation::Claim {
                revision,
                holder,
                worker,
                ..
            } => {
                self.check_revision(*revision)?;
                if !matches!(self.state, JobState::Pending | JobState::Blocked(_)) {
                    return Err(Rejection::InvalidState);
                }
                if ctx.queue.paused {
                    return Err(Rejection::Paused);
                }
                if ctx.queue.active_leases >= ctx.queue.policy.concurrency {
                    return Err(Rejection::ConcurrencyLimit);
                }
                if ctx.tick < self.ready_at_tick || ctx.utc_ms < self.spec.scheduled_at_ms {
                    return Err(Rejection::NotReady);
                }
                next.state = match worker {
                    Some(worker) if worker == &self.spec.worker => {
                        let expires_at_tick = add_time(ctx.tick, ctx.queue.policy.lease_ticks)?;
                        next.attempts = self
                            .attempts
                            .checked_add(1)
                            .filter(|n| *n <= self.spec.retry.max_attempts)
                            .ok_or(Rejection::InvalidState)?;
                        JobState::Leased(Lease {
                            token: LeaseToken {
                                job: self.spec.id.clone(),
                                run: self.run,
                                claim_index: ctx.index,
                                holder: holder.clone(),
                            },
                            expires_at_tick,
                            cancel_requested: false,
                        })
                    }
                    Some(worker) if worker.name == self.spec.worker.name => {
                        JobState::Blocked(BlockReason::IncompatibleWorker)
                    }
                    _ => JobState::Blocked(BlockReason::MissingWorker),
                };
            }
            Operation::Renew { token } => {
                let mut lease = self.lease(token, ctx.tick)?;
                if lease.cancel_requested {
                    return Err(Rejection::InvalidState);
                }
                lease.expires_at_tick = lease
                    .expires_at_tick
                    .max(add_time(ctx.tick, ctx.queue.policy.lease_ticks)?);
                next.state = JobState::Leased(lease);
            }
            Operation::Complete { token, outcome } => {
                let lease = self.lease(token, ctx.tick)?;
                if lease.cancel_requested {
                    next.state = JobState::Terminal(TerminalOutcome::Cancelled);
                } else {
                    match outcome {
                        Finish::Success => {
                            next.state = JobState::Terminal(TerminalOutcome::Completed)
                        }
                        Finish::Failure => next.failed(ctx.tick)?,
                        Finish::Cancelled => return Err(Rejection::InvalidState),
                    }
                }
            }
            Operation::Cancel { revision, .. } => {
                self.check_revision(*revision)?;
                match &mut next.state {
                    JobState::Leased(lease) => lease.cancel_requested = true,
                    JobState::Pending | JobState::Blocked(_) => {
                        next.state = JobState::Terminal(TerminalOutcome::Cancelled)
                    }
                    JobState::Terminal(_) => return Err(Rejection::InvalidState),
                }
            }
            Operation::Retry { revision, .. } => {
                self.check_revision(*revision)?;
                if !matches!(self.state, JobState::Terminal(_)) {
                    return Err(Rejection::InvalidState);
                }
                next.run = RunGeneration(self.run.0.checked_add(1).ok_or(Rejection::Arithmetic)?);
                next.attempts = 0;
                next.ready_at_tick = ctx.tick;
                next.state = JobState::Pending;
            }
            Operation::Discard { revision, .. } => {
                self.check_revision(*revision)?;
                if self.active() {
                    return Err(Rejection::InvalidState);
                }
                next.state = JobState::Terminal(TerminalOutcome::Discarded);
            }
            Operation::Tick { .. } => {
                let JobState::Leased(lease) = &self.state else {
                    return Err(Rejection::StaleLease);
                };
                if lease.expires_at_tick > ctx.tick {
                    return Err(Rejection::NotReady);
                }
                if lease.cancel_requested {
                    next.state = JobState::Terminal(TerminalOutcome::Cancelled);
                } else {
                    next.failed(ctx.tick)?;
                }
            }
            _ => return Err(Rejection::InvalidState),
        }
        next.record(ctx.index, ctx.tick);
        Ok(next)
    }
}
fn add_time(time: u64, delta: u64) -> Result<u64, Rejection> {
    time.checked_add(delta)
        .filter(|n| *n <= MAX_TIME)
        .ok_or(Rejection::Arithmetic)
}
