use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::actor;
use crate::db::jobs_connection::{
    wake_key, JobListener, JobSession, ListenEvent, ListenerStop, PgJobDb,
};
use crate::db::pg::JobsAckError;

use super::cron;
use super::drain_store;
use super::events::{
    self, Acknowledgement, Events, FailureKind, Identity, Kind, LeaseLoss, Operation,
};
use super::store::{
    claim_batch_with_timing, claim_with_timing, drain_work, finish, next_due_ms, prune, recover,
    renew, Claim, ClaimBatch, LeaseStatus, Outcome, OversizedClaim, Settlement, MAX_CLAIM_BATCH,
};
use super::with_pool;
use super::worker::{check_executable, handler_name, host_deadline, WorkerPoll, WorkerProcess};

const TICK: Duration = Duration::from_millis(10);
const MAX_GRACE_MS: u64 = 300_000;
const MAX_RUNNERS: usize = 256;
const MAX_IN_FLIGHT: usize = 1024;
const MAX_DELAY_MS: u64 = 365 * 24 * 60 * 60 * 1000;
const TIMEOUT: &str = "job handler timed out";
const STOPPED: &str = "job runner shutdown deadline elapsed";
const QUEUE_STOPPED: &str = "job queue shutdown deadline elapsed";
const CANCELLED: &str = "job cancelled";
const LEASE_LOST: &str = "job lease lost";

trait JobProcess: Send {
    fn reply_pending(&self) -> bool;
    fn poll(&mut self, now: Instant) -> Result<WorkerPoll, String>;
    fn renew(&mut self, lease_deadline: u64) -> Result<(), String>;
    fn kill(&mut self) -> Result<(), String>;
}

impl JobProcess for WorkerProcess {
    fn reply_pending(&self) -> bool {
        self.reply_pending()
    }

    fn poll(&mut self, now: Instant) -> Result<WorkerPoll, String> {
        self.poll(now)
    }

    fn renew(&mut self, lease_deadline: u64) -> Result<(), String> {
        self.renew(lease_deadline)
    }

    fn kill(&mut self) -> Result<(), String> {
        self.kill()
    }
}

type StartWorker =
    dyn Fn(String, Instant, u64, Sender<()>) -> Result<Box<dyn JobProcess>, String> + Send;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    namespace: String,
    queues: BTreeMap<String, usize>,
    workers: Vec<Worker>,
    lease_ms: u64,
    poll_ms: u64,
    shutdown_grace_ms: u64,
    recovery_interval_ms: Option<u64>,
    recovery_page_size: u32,
    cron_interval_ms: u64,
    cron_page_size: u32,
    cron_schedule_page_size: u32,
    prune_max_age_ms: Option<u64>,
    prune_interval_ms: u64,
    prune_page_size: u32,
    drain: bool,
    claim_batch: bool,
    event_buffer: usize,
    event_log: bool,
    listen: bool,
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
struct Worker {
    worker: String,
    version: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            namespace: "default".into(),
            queues: BTreeMap::new(),
            workers: Vec::new(),
            lease_ms: 30_000,
            poll_ms: 100,
            shutdown_grace_ms: 15_000,
            recovery_interval_ms: None,
            recovery_page_size: 64,
            cron_interval_ms: 1_000,
            cron_page_size: 64,
            cron_schedule_page_size: cron::MAX_TICK_JOBS,
            prune_max_age_ms: None,
            prune_interval_ms: 60_000,
            prune_page_size: 1000,
            drain: false,
            claim_batch: false,
            event_buffer: 0,
            event_log: false,
            listen: false,
        }
    }
}

impl Config {
    fn parse(json: &str) -> Result<Self, String> {
        if json.len() > 128 * 1024 {
            return Err("runner configuration exceeds 128 KiB".into());
        }
        let config: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        if !identifier(&config.namespace) {
            return Err("invalid runner namespace".into());
        }
        if config.queues.is_empty()
            || config.queues.len() > 64
            || config
                .queues
                .iter()
                .any(|(name, limit)| !identifier(name) || !(1..=MAX_IN_FLIGHT).contains(limit))
            || config.queues.values().sum::<usize>() > MAX_IN_FLIGHT
        {
            return Err("runner requires 1 to 64 queues and 1 to 1024 total slots".into());
        }
        let mut workers = HashSet::new();
        if config.workers.is_empty()
            || config.workers.len() > MAX_IN_FLIGHT
            || config.workers.iter().any(|worker| {
                !identifier(&worker.worker) || worker.version == 0 || !workers.insert(worker)
            })
        {
            return Err("runner requires 1 to 1024 distinct worker versions".into());
        }
        if !(50..=MAX_GRACE_MS).contains(&config.lease_ms) {
            return Err("runner lease_ms must be between 50 and 300000".into());
        }
        if !(1..=60_000).contains(&config.poll_ms) {
            return Err("runner poll_ms must be between 1 and 60000".into());
        }
        grace(config.shutdown_grace_ms)?;
        if config
            .recovery_interval_ms
            .is_some_and(|ms| !(1..=60_000).contains(&ms))
            || config.cron_interval_ms > 30_000
            || !(1..=86_400_000).contains(&config.prune_interval_ms)
        {
            return Err("Jobs intervals require recovery 1..=60000 ms, cron 0..=30000 ms, and prune 1..=86400000 ms".into());
        }
        if !(1..=1000).contains(&config.recovery_page_size)
            || !(1..=1000).contains(&config.prune_page_size)
            || !(1..=cron::MAX_TICK_JOBS).contains(&config.cron_page_size)
            || !(1..=cron::MAX_TICK_JOBS).contains(&config.cron_schedule_page_size)
        {
            return Err(
                "Jobs page sizes require recovery and prune 1..=1000 and cron 1..=128".into(),
            );
        }
        if config
            .prune_max_age_ms
            .is_some_and(|age| age > MAX_DELAY_MS)
        {
            return Err("runner prune_max_age_ms must be at most 365 days".into());
        }
        if config.event_buffer > events::MAX_BUFFER {
            return Err(format!(
                "runner event_buffer must be at most {}",
                events::MAX_BUFFER
            ));
        }
        Ok(config)
    }
}

fn identifier(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 255 && !value.contains('\0')
}

fn grace(ms: u64) -> Result<Duration, String> {
    if ms <= MAX_GRACE_MS {
        Ok(Duration::from_millis(ms))
    } else {
        Err("runner shutdown grace must be between 0 and 300000 milliseconds".into())
    }
}

enum Phase {
    Running,
    Stopping { deadline: Instant },
    Stopped,
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopping { .. } => "stopping",
            Self::Stopped => "stopped",
        }
    }

    fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped)
    }

    fn stop_wait_complete(&self) -> bool {
        match self {
            Self::Running => false,
            Self::Stopping { deadline } => Instant::now() >= *deadline,
            Self::Stopped => true,
        }
    }
}

struct ActiveJob {
    id: String,
    queue: String,
    execution: Execution,
    identity: Option<Arc<Identity>>,
    reported_lease_loss: bool,
}

impl ActiveJob {
    fn observe_local_lease_loss(&mut self, runner: &Runner) {
        let reply = match &self.execution {
            Execution::Running(worker) => worker.stop.as_ref(),
            Execution::Acknowledging { reply, .. } => Some(reply),
        };
        if matches!(
            reply,
            Some(
                Reply::Failed(FailureKind::LeaseLost, _) | Reply::Unstarted(FailureKind::LeaseLost)
            )
        ) {
            self.lease_lost(runner, LeaseLoss::LocalDeadline);
        }
    }

    fn lease_lost(&mut self, runner: &Runner, source: LeaseLoss) {
        if let Some(identity) = self.identity.as_ref().filter(|_| !self.reported_lease_loss) {
            runner.publish(Kind::LeaseLost { source }, Some(identity));
            self.reported_lease_loss = true;
        }
    }
}

struct LocalQueue {
    limit: usize,
    phase: QueuePhase,
    next_claim: Instant,
    source: ClaimWake,
}

#[derive(Clone, Copy)]
enum QueuePhase {
    Running,
    Paused,
    Stopping(Instant),
    Stopped,
}

impl QueuePhase {
    fn name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Stopping(_) => "stopping",
            Self::Stopped => "stopped",
        }
    }
}

impl LocalQueue {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            phase: QueuePhase::Running,
            next_claim: Instant::now(),
            source: ClaimWake::Ready,
        }
    }
}

struct Reservation {
    queue: String,
    slots: usize,
}

#[derive(Default)]
struct Counts {
    claimed: u64,
    peak_queued_context_bytes: usize,
    outcomes: OutcomeCounts,
    stale_acks: u64,
    recovered: u64,
    scheduled: u64,
    pruned: u64,
}

impl Counts {
    fn record_settlement(&mut self, settlement: Settlement) {
        let count = match settlement {
            Settlement::Completed => &mut self.outcomes.completed,
            Settlement::Cancelled => &mut self.outcomes.cancelled,
            Settlement::Discarded => &mut self.outcomes.discarded,
            Settlement::Retryable => &mut self.outcomes.retryable,
            Settlement::Scheduled => &mut self.outcomes.scheduled,
            Settlement::ReleasedUnstarted => &mut self.outcomes.released_unstarted,
            Settlement::Stale => &mut self.stale_acks,
        };
        *count += 1;
    }

    fn observe_queued_context_bytes(&mut self, bytes: usize) {
        self.peak_queued_context_bytes = self.peak_queued_context_bytes.max(bytes);
    }
}

#[derive(Default, Serialize)]
struct OutcomeCounts {
    completed: u64,
    cancelled: u64,
    discarded: u64,
    retryable: u64,
    scheduled: u64,
    released_unstarted: u64,
}

impl OutcomeCounts {
    fn total(&self) -> u64 {
        self.completed
            + self.cancelled
            + self.discarded
            + self.retryable
            + self.scheduled
            + self.released_unstarted
    }
}

struct State {
    phase: Phase,
    queues: BTreeMap<String, LocalQueue>,
    jobs: BTreeMap<String, ActiveJob>,
    pending: Option<Reservation>,
    counts: Counts,
    last_error: Option<String>,
}

impl State {
    fn new(config: &Config) -> Self {
        Self {
            phase: Phase::Running,
            queues: config
                .queues
                .iter()
                .map(|(name, &limit)| (name.clone(), LocalQueue::new(limit)))
                .collect(),
            jobs: BTreeMap::new(),
            pending: None,
            counts: Counts::default(),
            last_error: None,
        }
    }

    fn pending_for(&self, queue: &str) -> usize {
        self.pending
            .as_ref()
            .filter(|pending| pending.queue == queue)
            .map_or(0, |pending| pending.slots)
    }

    fn refresh_stopping_queues(&mut self) {
        for (name, queue) in &mut self.queues {
            if matches!(queue.phase, QueuePhase::Stopping(_))
                && !self.jobs.values().any(|job| &job.queue == name)
                && self
                    .pending
                    .as_ref()
                    .is_none_or(|pending| &pending.queue != name)
            {
                queue.phase = QueuePhase::Stopped;
            }
        }
    }

    fn fail_launch(&mut self, error: String, grace_ms: u64) {
        self.last_error = Some(error.chars().take(1024).collect());
        if matches!(self.phase, Phase::Running) {
            self.phase = Phase::Stopping {
                deadline: Instant::now() + Duration::from_millis(grace_ms),
            };
        }
    }
}

struct Runner {
    id: u64,
    config: Config,
    state: Mutex<State>,
    check_worker: fn() -> Result<(), String>,
    dispatcher: OnceLock<std::thread::Thread>,
    reply_wake: Sender<()>,
    events: Option<Events>,
    wakeups: Option<Arc<Wakeups>>,
}

impl Runner {
    fn wake_dispatcher(&self) {
        if let Some(dispatcher) = self.dispatcher.get() {
            dispatcher.unpark();
        }
    }

    fn request_stop(&self, grace: Duration) {
        let mut state = self.state.lock();
        let deadline = Instant::now() + grace;
        match &mut state.phase {
            Phase::Running => state.phase = Phase::Stopping { deadline },
            Phase::Stopping { deadline: current } => *current = (*current).min(deadline),
            Phase::Stopped => {}
        }
        drop(state);
        if let Some(wakeups) = &self.wakeups {
            wakeups.close();
        }
        self.wake_dispatcher();
    }

    fn supervise(&self, poll_all: bool) -> bool {
        if crate::process_signal::shutdown_requested() {
            self.request_stop(Duration::from_millis(self.config.shutdown_grace_ms));
        }
        let mut state = self.state.lock();
        let now = Instant::now();
        let shutting_down = match state.phase {
            Phase::Running => false,
            Phase::Stopping { deadline } => now >= deadline,
            Phase::Stopped => return false,
        };
        let mut reply_ready = false;
        let State {
            queues,
            jobs,
            last_error,
            ..
        } = &mut *state;
        for job in jobs.values_mut() {
            if let Execution::Running(worker) = &mut job.execution {
                if !poll_all && !worker.process.reply_pending() {
                    continue;
                }
                let queue_stopping = matches!(queues[&job.queue].phase, QueuePhase::Stopping(deadline) if now >= deadline);
                let stop = if shutting_down {
                    Some(STOPPED)
                } else if queue_stopping {
                    Some(QUEUE_STOPPED)
                } else {
                    None
                };
                if let Some(reply) = worker.poll(now, stop, last_error) {
                    job.execution = worker.acknowledging(reply, now);
                    reply_ready = true;
                }
                job.observe_local_lease_loss(self);
            }
        }
        drop(state);
        if reply_ready {
            self.wake_dispatcher();
        }
        true
    }

    fn report(&self, state: &State) -> String {
        let pending_claims = state.pending.as_ref().map_or(0, |pending| pending.slots);
        let undrained = state.jobs.len() + pending_claims;
        let queues: BTreeMap<_, _> = state.queues
            .iter()
            .map(|(name, queue)| {
                let in_flight = state.jobs.values().filter(|job| &job.queue == name).count();
                let pending = state.pending_for(name);
                (
                    name,
                    serde_json::json!({
                        "limit": queue.limit,
                        "in_flight": in_flight,
                        "pending_claims": pending,
                        "state": if state.phase.is_stopped() { "stopped" } else { queue.phase.name() },
                        "drained": in_flight == 0 && pending == 0,
                    }),
                )
            })
            .collect();
        let mut jobs: Vec<_> = state.jobs.values().map(|job| &job.id).collect();
        jobs.sort_unstable();
        let mut report = serde_json::json!({
            "runner_id": self.id,
            "namespace": self.config.namespace,
            "state": state.phase.name(),
            "drained": undrained == 0,
            "undrained": undrained,
            "undrained_jobs": jobs,
            "pending_claims": pending_claims,
            "queues": queues,
            "claimed": state.counts.claimed,
            "peak_queued_context_bytes": state.counts.peak_queued_context_bytes,
            "finished": state.counts.outcomes.total(),
            "outcomes": state.counts.outcomes,
            "stale_acks": state.counts.stale_acks,
            "recovered": state.counts.recovered,
            "scheduled": state.counts.scheduled,
            "pruned": state.counts.pruned,
            "last_error": state.last_error,
        });
        if let Some(events) = &self.events {
            report["events_dropped"] = events.dropped().into();
            report["event_logs_dropped"] = events.logs_dropped().into();
            report["event_buffer_bytes"] = events.buffer_bytes();
        }
        if let Some(wakeups) = &self.wakeups {
            report["wakeups"] = wakeups.report();
        }
        report.to_string()
    }

    fn wait_until(&self, ready: fn(&Phase) -> bool) -> String {
        loop {
            let state = self.state.lock();
            if ready(&state.phase) {
                return self.report(&state);
            }
            drop(state);
            actor::mesh_timer_sleep(TICK.as_millis() as i64);
        }
    }

    fn record_error(&self, error: String) {
        self.state.lock().last_error = Some(error.chars().take(1024).collect());
    }

    fn record_oversized(&self, row: &OversizedClaim) {
        self.record_error(format!(
            "mesh jobs oversized claim context: job {} requires {} bytes",
            row.id, row.context_bytes,
        ));
    }

    fn defer_claim(&self, queue: &str, next_due: Option<Instant>) {
        self.state
            .lock()
            .queues
            .get_mut(queue)
            .expect("known queue")
            .backoff(self, next_due);
    }

    fn publish(&self, kind: Kind, identity: Option<&Arc<Identity>>) {
        if let Some(events) = &self.events {
            events.publish(self.id, kind, identity);
        }
    }

    fn storage_failure(
        &self,
        operation: Operation,
        error: String,
        identity: Option<&Arc<Identity>>,
    ) {
        self.record_error(error);
        self.publish(Kind::StorageFailure { operation }, identity);
    }

    fn record_maintenance(&self, operation: Operation, count: u64) {
        if count == 0 {
            return;
        }
        let mut state = self.state.lock();
        match operation {
            Operation::Recover => state.counts.recovered += count,
            Operation::Cron => state.counts.scheduled += count,
            Operation::Prune => state.counts.pruned += count,
            _ => unreachable!("expected maintenance operation"),
        }
        self.publish(Kind::Maintenance { operation, count }, None);
    }
}

struct Registry {
    next_id: u64,
    accepting: bool,
    runners: BTreeMap<u64, Arc<Runner>>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| {
    Mutex::new(Registry {
        next_id: 1,
        accepting: true,
        runners: BTreeMap::new(),
    })
});

pub(super) fn start(
    pool: u64,
    config_json: &str,
    fn_ptr: *const u8,
    env_ptr: *const u8,
) -> Result<u64, String> {
    if pool == 0 {
        return Err("runner requires a PostgreSQL pool".into());
    }
    let handler = handler_name(fn_ptr, env_ptr)?;
    start_runner(
        pool,
        config_json,
        check_executable,
        worker_launcher(handler),
    )
}

fn worker_launcher(handler: String) -> Box<StartWorker> {
    Box::new(move |context, deadline, lease_deadline, reply_wake| {
        Ok(Box::new(WorkerProcess::start(
            &handler,
            context,
            deadline,
            lease_deadline,
            reply_wake,
        )?))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DrainOptions {
    max_jobs: u64,
    cutoff_ms: Option<u64>,
    recursive: bool,
    on_error: ErrorPolicy,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ErrorPolicy {
    Continue,
    Stop,
}

type DrainResult = Result<String, String>;

pub(super) fn drain(
    pool: u64,
    config_json: &str,
    fn_ptr: *const u8,
    env_ptr: *const u8,
    options_json: &str,
) -> DrainResult {
    if pool == 0 {
        return Err("Jobs.drain requires a PostgreSQL pool".into());
    }
    let mut config = Config::parse(config_json)?;
    config.claim_batch = false;
    let wire: serde_json::Value =
        serde_json::from_str(config_json).map_err(|error| error.to_string())?;
    if wire.get("namespace").is_none() || config.drain {
        return Err(
            "Jobs.drain requires an explicit namespace and uses separate drain options".into(),
        );
    }
    let options: DrainOptions = super::store::decode(options_json)?;
    if !(1..=1000).contains(&options.max_jobs) {
        return Err("Jobs.drain max_jobs must be between 1 and 1000".into());
    }
    if options
        .cutoff_ms
        .is_some_and(|cutoff| cutoff > super::store::MAX_TIMESTAMP_MS)
    {
        return Err("Jobs.drain cutoff_ms must be a UTC timestamp through year 9999".into());
    }
    let handler = handler_name(fn_ptr, env_ptr)?;
    let (publish, result) = crossbeam_channel::bounded(1);
    let runner = start_configured(
        pool,
        config,
        check_executable,
        worker_launcher(handler),
        Some((options, publish)),
    )?;
    runner.wait_until(Phase::is_stopped);
    result
        .try_recv()
        .map_err(|_| "Jobs.drain runner exited without a report")?
}

fn start_runner(
    pool: u64,
    config_json: &str,
    check_worker: fn() -> Result<(), String>,
    start_worker: Box<StartWorker>,
) -> Result<u64, String> {
    start_configured(
        pool,
        Config::parse(config_json)?,
        check_worker,
        start_worker,
        None,
    )
    .map(|runner| runner.id)
}

fn start_configured(
    pool: u64,
    config: Config,
    check_worker: fn() -> Result<(), String>,
    start_worker: Box<StartWorker>,
    bounded: Option<(DrainOptions, crossbeam_channel::Sender<DrainResult>)>,
) -> Result<Arc<Runner>, String> {
    let workers = serde_json::to_string(&config.workers).map_err(|error| error.to_string())?;
    let scheduler = actor::GLOBAL_SCHEDULER
        .get()
        .ok_or("actor scheduler is unavailable")?;
    let default_claim = !config.claim_batch && bounded.is_none();
    with_pool(pool, |db| {
        let rows = db.query(
            "WITH functions AS (
                 SELECT to_regprocedure('mesh_jobs.try_renew(text,uuid,uuid,bigint)') AS renew,
                        to_regprocedure('mesh_jobs.settle_state(text,uuid,uuid,jsonb,boolean)') AS settle,
                        to_regprocedure('mesh_jobs.claim_batch_v1(text,text,jsonb,bigint,integer,bigint)') AS batch,
                        to_regprocedure('mesh_jobs.claim_drain_v1(text,text,jsonb,bigint,bigint,jsonb)') AS single_claim
             ) SELECT CASE
                 WHEN version <> 1 OR to_regclass('mesh_jobs.cron_schedules') IS NULL
                     OR renew IS NULL OR settle IS NULL THEN 'migrate'
                 WHEN NOT (has_function_privilege(renew, 'EXECUTE')
                     AND has_function_privilege(settle, 'EXECUTE')) THEN 'permissions'
                 WHEN $1::boolean AND batch IS NULL THEN 'migrate'
                 WHEN $1::boolean AND NOT has_function_privilege(batch, 'EXECUTE') THEN 'batch_permissions'
                 WHEN $2::boolean AND single_claim IS NULL THEN 'migrate'
                 WHEN $2::boolean AND NOT has_function_privilege(single_claim, 'EXECUTE') THEN 'single_permissions'
                 ELSE 'ready'
             END FROM mesh_jobs.schema_version CROSS JOIN functions WHERE singleton",
            &[&config.claim_batch.to_string(), &default_claim.to_string()],
        )?;
        match rows
            .first()
            .and_then(|row| row.first())
            .map(|(_, status)| status.as_str())
        {
            Some("ready") => Ok(()),
            Some("permissions") => Err(
                "Jobs runner needs EXECUTE on mesh_jobs.try_renew and mesh_jobs.settle_state"
                    .into(),
            ),
            Some("batch_permissions") => {
                Err("Jobs batch runner needs EXECUTE on mesh_jobs.claim_batch_v1".into())
            }
            Some("single_permissions") => {
                Err("Jobs runner needs EXECUTE on mesh_jobs.claim_drain_v1".into())
            }
            _ => Err(
                "Jobs requires the current schema. Run Jobs.migrate before starting workers".into(),
            ),
        }
    })?;
    let sessions = if config.claim_batch {
        let claims = unsafe { JobSession::from_pool(pool)? };
        let mut lifecycle = unsafe { JobSession::from_pool(pool)? };
        lifecycle
            .with(|_| Ok(()))
            .map_err(|error| format!("could not reserve Jobs lifecycle session: {error}"))?;
        Some((claims, lifecycle))
    } else {
        None
    };
    let listener = (config.listen && !config.drain && bounded.is_none())
        .then(|| unsafe { JobListener::from_pool(pool, &config.namespace) })
        .transpose()?;
    let wakeups = listener.as_ref().map(|_| Arc::new(Wakeups::new(&config)));

    let mut registry = REGISTRY.lock();
    if !registry.accepting || scheduler.is_shutdown() || crate::process_signal::shutdown_requested()
    {
        return Err("runtime shutdown already requested".into());
    }
    if registry.runners.len() == MAX_RUNNERS {
        let stopped = registry
            .runners
            .iter()
            .find_map(|(id, runner)| runner.state.lock().phase.is_stopped().then_some(*id));
        registry
            .runners
            .remove(&stopped.ok_or("runner limit reached")?);
    }
    let id = registry.next_id;
    registry.next_id = id
        .checked_add(1)
        .filter(|&next| next <= i64::MAX as u64)
        .ok_or("runner identifiers exhausted")?;
    let events = Events::new(config.event_buffer, config.event_log)?;
    let state = Mutex::new(State::new(&config));
    let (reply_wake, reply_wakes) = crossbeam_channel::bounded(1);
    let runner = Arc::new(Runner {
        id,
        config,
        check_worker,
        dispatcher: OnceLock::new(),
        reply_wake,
        events,
        wakeups,
        state,
    });
    crate::process_signal::mesh_process_install_shutdown_signals();
    let supervised = Arc::clone(&runner);
    let supervisor = std::thread::Builder::new()
        .name(format!("mesh-jobs-watch-{id}"))
        .spawn(move || {
            let mut reap_at = Instant::now();
            loop {
                let timed = Instant::now() >= reap_at;
                if !supervised.supervise(timed) {
                    break;
                }
                if timed {
                    reap_at = Instant::now() + TICK;
                }
                // An early reply can precede exit. Keep the next timed reap in place.
                let _ = reply_wakes.recv_deadline(reap_at);
            }
        })
        .map_err(|error| format!("could not start job runner supervisor: {error}"))?;
    let dispatcher = Arc::clone(&runner);
    let thread = std::thread::Builder::new()
        .name(format!("mesh-jobs-{id}"))
        .spawn(move || {
            let _ = dispatcher.dispatcher.set(std::thread::current());
            dispatcher.publish(Kind::RunnerStarted, None);
            if let Some((options, publish)) = bounded {
                let _ = publish.send(dispatch_bounded(
                    &dispatcher,
                    pool,
                    &workers,
                    &*start_worker,
                    options,
                ));
            } else if dispatcher.config.drain {
                dispatch(&dispatcher, pool, &workers, &*start_worker, sessions);
            } else {
                let maintenance_runner = Arc::clone(&dispatcher);
                match std::thread::Builder::new()
                    .name(format!("mesh-jobs-maintenance-{id}"))
                    .spawn(move || maintain(&maintenance_runner, pool))
                {
                    Ok(maintenance) => {
                        if let Some(listener) = listener {
                            dispatcher.start_listener(listener);
                        }
                        dispatch(&dispatcher, pool, &workers, &*start_worker, sessions);
                        if let Some(wakeups) = &dispatcher.wakeups {
                            wakeups.close();
                        }
                        maintenance.thread().unpark();
                        let _ = maintenance.join();
                    }
                    Err(error) => {
                        dispatcher
                            .record_error(format!("could not start job maintenance: {error}"));
                    }
                }
            }
            {
                let mut state = dispatcher.state.lock();
                dispatcher.publish(Kind::RunnerStopped, None);
                if let Some(events) = &dispatcher.events {
                    events.close();
                }
                state.phase = Phase::Stopped;
            }
            let _ = supervisor.join();
        });
    if let Err(error) = thread {
        runner.state.lock().phase = Phase::Stopped;
        return Err(format!("could not start job runner: {error}"));
    }
    registry.runners.insert(id, Arc::clone(&runner));
    Ok(runner)
}

fn lookup(id: u64) -> Result<Arc<Runner>, String> {
    REGISTRY
        .lock()
        .runners
        .get(&id)
        .cloned()
        .ok_or_else(|| "unknown job runner".into())
}

pub(super) fn status(id: u64) -> Result<String, String> {
    let runner = lookup(id)?;
    let state = runner.state.lock();
    Ok(runner.report(&state))
}

pub(super) fn read_events(id: u64, limit: i64) -> Result<String, String> {
    if !(1..=events::MAX_READ as i64).contains(&limit) {
        return Err(format!(
            "Jobs event read limit must be between 1 and {}",
            events::MAX_READ
        ));
    }
    lookup(id)?
        .events
        .as_ref()
        .ok_or("Jobs event reader is disabled")?
        .read(limit as usize)
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum LocalAction {
    Start { limit: Option<usize> },
    Pause,
    Resume,
    Scale { limit: usize },
    Stop { grace_ms: Option<u64> },
}

#[derive(Deserialize)]
struct LocalControl {
    queue: String,
    #[serde(flatten)]
    action: LocalAction,
}

pub(super) fn control(id: u64, request: &str) -> Result<String, String> {
    if request.len() > 4096 {
        return Err("local Jobs control exceeds 4 KiB".into());
    }
    let LocalControl { queue, action } =
        serde_json::from_str(request).map_err(|error| error.to_string())?;
    if !identifier(&queue) {
        return Err("invalid local queue name".into());
    }
    let runner = lookup(id)?;
    let mut state = runner.state.lock();
    update_queue(&mut state, &runner.config, queue, action)?;
    let report = runner.report(&state);
    drop(state);
    runner.wake_dispatcher();
    Ok(report)
}

fn update_queue(
    state: &mut State,
    config: &Config,
    name: String,
    action: LocalAction,
) -> Result<(), String> {
    if config.drain || !matches!(state.phase, Phase::Running) {
        return Err("local queue control requires a running, non-drain runner".into());
    }
    state.refresh_stopping_queues();
    let current = state.queues.get(&name);
    let phase = current.map(|queue| queue.phase);
    let limit = match action {
        LocalAction::Start { limit } => {
            if matches!(phase, Some(QueuePhase::Stopping(_))) {
                return Err("local queue is still stopping".into());
            }
            Some(
                limit
                    .or_else(|| current.map(|queue| queue.limit))
                    .unwrap_or(1),
            )
        }
        LocalAction::Scale { limit } => {
            if !matches!(phase, Some(QueuePhase::Running | QueuePhase::Paused)) {
                return Err("local queue must be running or paused to scale".into());
            }
            Some(limit)
        }
        _ => None,
    };
    if let Some(limit) = limit {
        let other_slots: usize = state
            .queues
            .iter()
            .filter(|(key, queue)| **key != name && !matches!(queue.phase, QueuePhase::Stopped))
            .map(|(_, queue)| queue.limit)
            .sum();
        if !(1..=MAX_IN_FLIGHT).contains(&limit) || other_slots + limit > MAX_IN_FLIGHT {
            return Err("runner requires 1 to 1024 total local slots".into());
        }
    }

    if matches!(action, LocalAction::Start { .. }) {
        if current.is_none() && state.queues.len() >= 64 {
            return Err("runner supports at most 64 local queue names".into());
        }
        state
            .queues
            .insert(name, LocalQueue::new(limit.expect("validated start limit")));
        return Ok(());
    }

    let queue = state.queues.get_mut(&name).ok_or("unknown local queue")?;
    match action {
        LocalAction::Start { .. } => unreachable!(),
        LocalAction::Pause if matches!(queue.phase, QueuePhase::Running | QueuePhase::Paused) => {
            queue.phase = QueuePhase::Paused;
        }
        LocalAction::Resume if matches!(queue.phase, QueuePhase::Paused | QueuePhase::Running) => {
            queue.phase = QueuePhase::Running;
            queue.ready(ClaimWake::Ready);
        }
        LocalAction::Scale { limit } => {
            queue.limit = limit;
            queue.ready(ClaimWake::Ready);
        }
        LocalAction::Stop { grace_ms } => {
            let deadline = Instant::now() + grace(grace_ms.unwrap_or(config.shutdown_grace_ms))?;
            queue.phase = match queue.phase {
                QueuePhase::Stopped => QueuePhase::Stopped,
                QueuePhase::Stopping(current) => QueuePhase::Stopping(current.min(deadline)),
                _ => QueuePhase::Stopping(deadline),
            };
            state.refresh_stopping_queues();
        }
        _ => return Err("local queue must be running or paused".into()),
    }
    Ok(())
}

pub(super) fn stop(id: u64, grace_ms: u64) -> Result<String, String> {
    let runner = lookup(id)?;
    runner.request_stop(grace(grace_ms)?);
    Ok(runner.wait_until(Phase::stop_wait_complete))
}

pub(super) fn join(id: u64) -> Result<String, String> {
    Ok(lookup(id)?.wait_until(Phase::is_stopped))
}

pub(super) fn shutdown_all() {
    let runners: Vec<_> = {
        let mut registry = REGISTRY.lock();
        registry.accepting = false;
        registry.runners.values().cloned().collect()
    };
    for runner in &runners {
        runner.request_stop(Duration::from_millis(runner.config.shutdown_grace_ms));
    }
    for runner in runners {
        runner.wait_until(Phase::stop_wait_complete);
        while runner
            .state
            .lock()
            .jobs
            .values()
            .any(|job| matches!(job.execution, Execution::Running(_)))
        {
            actor::mesh_timer_sleep(TICK.as_millis() as i64);
        }
    }
}

#[derive(Clone, Copy)]
struct Lease {
    renew_at: Option<Instant>,
    expires_at: Instant,
    worker_expires_at: u64,
}

impl Lease {
    fn begin(duration: Duration) -> Result<Self, String> {
        // PostgreSQL starts the lease during the request, so its start time
        // cannot precede either of the local deadlines sampled before the query.
        let from = Instant::now();
        Ok(Self {
            renew_at: Some(from + duration / 3),
            expires_at: from + duration,
            worker_expires_at: host_deadline(from + duration)?,
        })
    }
}

struct Running {
    process: Box<dyn JobProcess>,
    deadline: Instant,
    lease_deadline: Instant,
    reaped: bool,
    stop: Option<Reply>,
    started_at: Option<Instant>,
}

impl Running {
    fn acknowledging(&self, reply: Reply, now: Instant) -> Execution {
        let duration_ms = self
            .started_at
            .map(|started| now.saturating_duration_since(started).as_millis() as u64);
        Execution::acknowledging(reply, duration_ms)
    }

    fn poll(
        &mut self,
        now: Instant,
        shutdown_reason: Option<&str>,
        last_error: &mut Option<String>,
    ) -> Option<Reply> {
        let poll_error = match self.process.poll(now) {
            Ok(WorkerPoll::Exited(result)) => {
                return Some(
                    self.stop
                        .take()
                        .unwrap_or_else(|| Reply::from_worker(result)),
                );
            }
            Ok(WorkerPoll::TimedOut) => {
                return Some(self.stop.take().unwrap_or_else(|| Reply::timed_out()));
            }
            Ok(WorkerPoll::LeaseExpired) => {
                return Some(self.stop.take().unwrap_or_else(|| Reply::lease_lost()));
            }
            Ok(WorkerPoll::AwaitingReply) => {
                self.reaped = true;
                return self.stop.take();
            }
            Ok(WorkerPoll::Running) => None,
            Err(error) => {
                *last_error = Some(error.chars().take(1024).collect());
                Some(error)
            }
        };
        if self.stop.is_none() {
            self.stop = if now >= self.lease_deadline && self.lease_deadline <= self.deadline {
                Some(Reply::lease_lost())
            } else if now >= self.deadline {
                Some(Reply::timed_out())
            } else if let Some(reason) = shutdown_reason {
                Some(Reply::Failed(FailureKind::Shutdown, reason.into()))
            } else {
                poll_error.map(|error| Reply::Failed(FailureKind::WorkerFailure, error))
            };
        }
        if self.stop.is_some() {
            self.kill(last_error);
        }
        None
    }

    fn stop(&mut self, reason: Reply, last_error: &mut Option<String>) {
        self.stop.get_or_insert(reason);
        self.kill(last_error);
    }

    fn kill(&mut self, last_error: &mut Option<String>) {
        if let Err(error) = self.process.kill() {
            *last_error = Some(error.chars().take(1024).collect());
        }
    }
}

pub(super) enum Reply {
    Returned(String),
    Failed(FailureKind, String),
    Cancelled,
    Unstarted(FailureKind),
}

impl Reply {
    pub(super) fn from_worker(result: Result<String, String>) -> Self {
        match result {
            Ok(output) => Self::Returned(output),
            Err(error) => Self::Failed(FailureKind::WorkerFailure, error),
        }
    }

    fn lease_lost() -> Self {
        Self::Failed(FailureKind::LeaseLost, LEASE_LOST.into())
    }

    fn timed_out() -> Self {
        Self::Failed(FailureKind::Timeout, TIMEOUT.into())
    }

    pub(super) fn outcome(&self) -> (Outcome, Option<FailureKind>) {
        match self {
            Self::Returned(json) => match serde_json::from_str(json) {
                Ok(Outcome::RetryAfter { delay_ms, .. } | Outcome::Snooze { delay_ms })
                    if delay_ms > MAX_DELAY_MS =>
                {
                    (
                        Outcome::Fail {
                            reason: "job handler outcome delay exceeds 365 days".into(),
                        },
                        Some(FailureKind::InvalidOutcome),
                    )
                }
                Ok(outcome) => {
                    let failure = matches!(
                        outcome,
                        Outcome::Fail { .. } | Outcome::RetryAfter { .. } | Outcome::Discard { .. }
                    )
                    .then_some(FailureKind::HandlerFailure);
                    (outcome, failure)
                }
                Err(_) => (
                    Outcome::Fail {
                        reason: "invalid job handler outcome JSON".into(),
                    },
                    Some(FailureKind::InvalidOutcome),
                ),
            },
            Self::Failed(kind, reason) => (
                Outcome::Fail {
                    reason: reason.chars().take(1024).collect(),
                },
                Some(*kind),
            ),
            Self::Cancelled => (
                Outcome::Cancel {
                    reason: CANCELLED.into(),
                },
                None,
            ),
            Self::Unstarted(kind) => (Outcome::ReleaseUnstarted, Some(*kind)),
        }
    }
}

enum Execution {
    Running(Running),
    Acknowledging {
        reply: Reply,
        retry_at: Option<Instant>,
        handler_duration_ms: Option<u64>,
    },
}

impl Execution {
    fn acknowledging(reply: Reply, handler_duration_ms: Option<u64>) -> Self {
        Self::Acknowledging {
            reply,
            retry_at: None,
            handler_duration_ms,
        }
    }

    fn apply_renewal(&mut self, renewed: &Lease, last_error: &mut Option<String>) -> bool {
        let Self::Running(worker) = self else {
            return true;
        };
        let now = Instant::now();
        // The worker may have exited while PostgreSQL was processing the renewal.
        if now >= worker.lease_deadline && !worker.reaped {
            if let Some(reply) = worker.poll(now, None, last_error) {
                *self = worker.acknowledging(reply, now);
                return true;
            }
            if !worker.reaped {
                return false;
            }
        }
        if !worker.reaped {
            if let Err(error) = worker.process.renew(renewed.worker_expires_at) {
                *last_error = Some(error.chars().take(1024).collect());
                worker.stop(Reply::Failed(FailureKind::WorkerFailure, error), last_error);
                return false;
            }
        }
        worker.lease_deadline = renewed.expires_at;
        true
    }
}

struct Attempt {
    id: String,
    token: String,
    queue: String,
    lease: Option<Lease>,
    submissions: u32,
}

struct LifecycleIo {
    pool: u64,
    session: Option<JobSession>,
}

impl LifecycleIo {
    fn with<T>(
        &mut self,
        run: impl FnOnce(&mut PgJobDb<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        match &mut self.session {
            Some(session) => session.with(run),
            None => with_pool(self.pool, run),
        }
    }
}

impl Attempt {
    fn poll(&mut self, runner: &Runner, io: &mut LifecycleIo) -> bool {
        self.poll_with_ack(runner, io, |io, id, token, outcome| {
            io.with(|db| Ok(finish(db, &runner.config.namespace, id, token, outcome)))
                .map_err(JobsAckError::Rejected)?
        })
    }

    fn poll_with_ack(
        &mut self,
        runner: &Runner,
        io: &mut LifecycleIo,
        acknowledge: impl FnOnce(
            &mut LifecycleIo,
            &str,
            &str,
            Outcome,
        ) -> Result<Settlement, JobsAckError>,
    ) -> bool {
        let outcome = {
            let mut state = runner.state.lock();
            let job = state.jobs.get_mut(&self.token).expect("tracked attempt");
            match &job.execution {
                Execution::Running(_) => None,
                Execution::Acknowledging {
                    reply,
                    retry_at,
                    handler_duration_ms,
                } => {
                    let Some(lease) = &self.lease else {
                        return false;
                    };
                    // The local deadline fences execution. PostgreSQL decides
                    // whether the token can still settle the reaped attempt.
                    if let Some(retry) = retry_at {
                        let now = Instant::now();
                        if now >= lease.expires_at {
                            job.lease_lost(runner, LeaseLoss::LocalDeadline);
                            return false;
                        }
                        if now < *retry {
                            return true;
                        }
                    }
                    Some((reply.outcome(), *handler_duration_ms, job.identity.clone()))
                }
            }
        };
        let Some(((outcome, failure_kind), handler_duration_ms, identity)) = outcome else {
            self.renew(runner, io);
            return true;
        };
        self.submissions = self.submissions.saturating_add(1);
        let acknowledgement = Acknowledgement {
            submission: self.submissions,
            handler_duration_ms,
            failure_kind,
        };
        match acknowledge(io, &self.id, &self.token, outcome) {
            Ok(settlement) => {
                runner.state.lock().counts.record_settlement(settlement);
                let event = match settlement {
                    Settlement::Stale => Kind::AckStale { acknowledgement },
                    settlement => Kind::SettlementCommitted {
                        settlement,
                        acknowledgement,
                    },
                };
                runner.publish(event, identity.as_ref());
                false
            }
            Err(error) => {
                match error {
                    JobsAckError::Rejected(error) => {
                        runner.storage_failure(Operation::Acknowledge, error, identity.as_ref());
                    }
                    JobsAckError::Unconfirmed(error) => {
                        runner.record_error(error);
                        runner.publish(Kind::AckUncertain { acknowledgement }, identity.as_ref());
                    }
                }
                let mut state = runner.state.lock();
                let job = state.jobs.get_mut(&self.token).expect("tracked attempt");
                if let Execution::Acknowledging { retry_at, .. } = &mut job.execution {
                    *retry_at = Some(Instant::now() + retry_interval(&runner.config));
                }
                true
            }
        }
    }

    fn renew(&mut self, runner: &Runner, io: &mut LifecycleIo) {
        let Some(lease) = &mut self.lease else {
            return;
        };
        let now = Instant::now();
        if now >= lease.expires_at || lease.renew_at.is_none_or(|due| now < due) {
            return;
        }
        let config = &runner.config;
        let renewal = io.with(|db| {
            if Instant::now() >= lease.expires_at {
                return Ok(None);
            }
            let renewed = Lease::begin(Duration::from_millis(config.lease_ms))?;
            renew(
                db,
                &config.namespace,
                &self.id,
                &self.token,
                config.lease_ms,
            )
            .map(|status| Some((renewed, status)))
        });
        let (renewed, status) = match renewal {
            Ok(Some(result)) => result,
            Ok(None) => return,
            Err(error) => {
                let identity = runner.state.lock().jobs[&self.token].identity.clone();
                runner.storage_failure(Operation::Renew, error, identity.as_ref());
                lease.renew_at = Some(Instant::now() + retry_interval(config));
                return;
            }
        };
        let mut state = runner.state.lock();
        let State {
            jobs, last_error, ..
        } = &mut *state;
        let job = jobs.get_mut(&self.token).expect("tracked attempt");
        match status {
            LeaseStatus::Active => {
                if job.execution.apply_renewal(&renewed, last_error) {
                    *lease = renewed;
                }
                job.observe_local_lease_loss(runner);
            }
            LeaseStatus::Cancelled => {
                lease.renew_at = None;
                if let Execution::Running(worker) = &mut job.execution {
                    worker.stop(Reply::Cancelled, last_error);
                }
            }
            LeaseStatus::Lost => {
                self.lease = None;
                job.lease_lost(runner, LeaseLoss::Database);
                if let Execution::Running(worker) = &mut job.execution {
                    worker.stop(Reply::lease_lost(), last_error);
                }
            }
        }
    }

    fn blocks_claims(&self) -> bool {
        self.lease
            .as_ref()
            .is_none_or(|lease| lease.renew_at.is_none() || Instant::now() >= lease.expires_at)
    }
}

fn retry_interval(config: &Config) -> Duration {
    Duration::from_millis(config.poll_ms.min(100).min(config.lease_ms / 6))
}

#[derive(Clone, Copy)]
enum ClaimWake {
    Ready,
    Subscription,
    Notification,
    Due,
    Fallback,
}

impl LocalQueue {
    fn ready(&mut self, source: ClaimWake) {
        self.next_claim = Instant::now();
        self.source = source;
    }

    fn backoff(&mut self, runner: &Runner, next_due: Option<Instant>) {
        let poll_ms = runner.config.poll_ms;
        let jitter = if runner.wakeups.is_some() {
            rand::random_range(0..=poll_ms / 5)
        } else {
            0
        };
        let fallback = Instant::now() + Duration::from_millis(poll_ms + jitter);
        if let Some(due) = next_due.filter(|&due| due < fallback) {
            self.next_claim = due;
            self.source = ClaimWake::Due;
        } else {
            self.next_claim = fallback;
            self.source = ClaimWake::Fallback;
        }
    }
}

struct Wakeups {
    stop: ListenerStop,
    hint_bits: BTreeMap<String, u64>,
    state: Mutex<WakeState>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WakeHints {
    configured: u64,
    rescan: bool,
    unknown: bool,
}

impl WakeHints {
    fn is_empty(&self) -> bool {
        self.configured == 0 && !self.rescan && !self.unknown
    }
}

#[derive(Default)]
struct WakeState {
    hints: WakeHints,
    closed: bool,
    subscribed: bool,
    subscriptions: u64,
    subscription_scans: u64,
    notification_scans: u64,
    due_scans: u64,
    fallback_scans: u64,
}

impl Wakeups {
    fn new(config: &Config) -> Self {
        let mut hint_bits = BTreeMap::new();
        for (index, name) in config.queues.keys().enumerate() {
            *hint_bits.entry(wake_key(name)).or_insert(0) |= 1 << index;
        }
        Self {
            stop: ListenerStop::default(),
            hint_bits,
            state: Mutex::default(),
        }
    }

    fn event(&self, event: ListenEvent<'_>) -> bool {
        let mut state = self.state.lock();
        if state.closed {
            return false;
        }
        let was_empty = state.hints.is_empty();
        match event {
            ListenEvent::Subscribed => {
                state.subscribed = true;
                state.subscriptions += 1;
                state.hints.rescan = true;
            }
            ListenEvent::Disconnected => state.subscribed = false,
            ListenEvent::Hint(key) if state.subscribed => match self.hint_bits.get(key) {
                Some(bits) => state.hints.configured |= bits,
                None => state.hints.unknown = true,
            },
            ListenEvent::Hint(_) => {}
        }
        was_empty && !state.hints.is_empty()
    }

    fn close(&self) {
        let mut state = self.state.lock();
        if state.closed {
            return;
        }
        state.closed = true;
        state.subscribed = false;
        self.stop.stop();
    }

    fn take(&self) -> WakeHints {
        std::mem::take(&mut self.state.lock().hints)
    }

    fn apply(&self, queues: &mut BTreeMap<String, LocalQueue>, freed: &mut HashSet<String>) {
        let hints = self.take();
        if hints.is_empty() && freed.is_empty() {
            return;
        }
        let now = Instant::now();
        for (name, queue) in queues {
            // A queue added by local control has no bit in the static map.
            if hints.unknown
                || (hints.configured != 0
                    && self
                        .hint_bits
                        .get(&wake_key(name))
                        .is_some_and(|bits| hints.configured & bits != 0))
            {
                queue.ready(ClaimWake::Notification);
            } else if hints.rescan {
                queue.ready(ClaimWake::Subscription);
            } else if queue.next_claim > now && freed.contains(name) {
                queue.ready(ClaimWake::Ready);
            }
        }
        freed.clear();
    }

    fn record_scan(&self, source: ClaimWake) {
        let mut state = self.state.lock();
        match source {
            ClaimWake::Ready => {}
            ClaimWake::Subscription => state.subscription_scans += 1,
            ClaimWake::Notification => state.notification_scans += 1,
            ClaimWake::Due => state.due_scans += 1,
            ClaimWake::Fallback => state.fallback_scans += 1,
        }
    }

    fn report(&self) -> serde_json::Value {
        let state = self.state.lock();
        serde_json::json!({
            "subscribed": state.subscribed,
            "subscriptions": state.subscriptions,
            "subscription_scans": state.subscription_scans,
            "notification_scans": state.notification_scans,
            "due_scans": state.due_scans,
            "fallback_scans": state.fallback_scans,
        })
    }
}

impl Runner {
    fn start_listener(&self, listener: JobListener) {
        let wakeups = Arc::clone(self.wakeups.as_ref().expect("listener wake state"));
        let dispatcher = std::thread::current();
        // Socket shutdown is synchronous. A canceled dial or DNS lookup can
        // finish independently and cannot register or subscribe afterward.
        if let Err(error) = std::thread::Builder::new()
            .name(format!("mesh-jobs-listen-{}", self.id))
            .spawn(move || {
                listener.run(&wakeups.stop, |event| {
                    if wakeups.event(event) {
                        dispatcher.unpark();
                    }
                });
            })
        {
            self.record_error(format!("could not start Jobs listener: {error}"));
        }
    }

    fn probe_due(&self, db: &mut PgJobDb<'_>, queue: &str, workers: &str) -> Option<Instant> {
        self.wakeups.as_ref()?;
        // Sample before claiming so a deadline crossing between the two reads
        // is either still on the timer or already visible to the claim.
        match next_due_ms(db, &self.config.namespace, queue, workers) {
            Ok(delay) => Instant::now().checked_add(Duration::from_millis(delay?)),
            Err(error) => {
                self.record_error(error);
                None
            }
        }
    }
}

fn maintain(runner: &Runner, pool: u64) {
    let config = &runner.config;
    let mut next_recovery = Instant::now();
    let mut next_schedule = Instant::now();
    let mut next_prune = Instant::now();
    let producer = format!(
        "{}:{}:{:032x}",
        std::process::id(),
        runner.id,
        rand::random::<u128>()
    );
    while matches!(runner.state.lock().phase, Phase::Running) {
        let now = Instant::now();
        if now >= next_recovery {
            match with_pool(pool, |db| {
                recover(db, &config.namespace, config.recovery_page_size, None)
            }) {
                Ok(recovered) => runner.record_maintenance(Operation::Recover, recovered),
                Err(error) => runner.storage_failure(Operation::Recover, error, None),
            }
            next_recovery = Instant::now()
                + Duration::from_millis(
                    config
                        .recovery_interval_ms
                        .unwrap_or_else(|| config.poll_ms.clamp(100, 1000)),
                );
        }
        if config.cron_interval_ms != 0 && now >= next_schedule {
            match with_pool(pool, |db| {
                cron::tick(
                    db,
                    &config.namespace,
                    &producer,
                    config.cron_page_size,
                    config.cron_schedule_page_size,
                )
            }) {
                Ok(scheduled) => runner.record_maintenance(Operation::Cron, u64::from(scheduled)),
                Err(error) => runner.storage_failure(Operation::Cron, error, None),
            }
            next_schedule = Instant::now() + Duration::from_millis(config.cron_interval_ms);
        }
        if let Some(age) = config.prune_max_age_ms.filter(|_| now >= next_prune) {
            match with_pool(pool, |db| {
                prune(
                    db,
                    &config.namespace,
                    None,
                    None,
                    Some(age),
                    Some(config.prune_page_size),
                )
            }) {
                Ok(count) => runner.record_maintenance(Operation::Prune, count),
                Err(error) => runner.storage_failure(Operation::Prune, error, None),
            }
            next_prune = Instant::now() + Duration::from_millis(config.prune_interval_ms);
        }
        std::thread::park_timeout(TICK);
    }
}

struct BatchRequest {
    queue: String,
    slots: usize,
}

type BatchReply = (String, Result<(Lease, ClaimBatch, Option<Instant>), String>);

struct BatchLane {
    requests: SyncSender<BatchRequest>,
    results: Receiver<BatchReply>,
}

impl BatchLane {
    fn request(&self, runner: &Runner, queue: String) -> Result<(), String> {
        let slots = reserve_claims(runner, &queue, MAX_CLAIM_BATCH);
        if slots == 0 {
            return Ok(());
        }
        if self
            .requests
            .try_send(BatchRequest { queue, slots })
            .is_err()
        {
            runner.state.lock().pending = None;
            runner.request_stop(Duration::from_millis(runner.config.shutdown_grace_ms));
            return Err("Jobs batch claim thread is unavailable".into());
        }
        Ok(())
    }

    fn poll(&self, runner: &Runner, attempts: &mut Vec<Attempt>, start_worker: &StartWorker) {
        if runner.state.lock().pending.is_none() {
            return;
        }
        match self.results.try_recv() {
            Ok((queue, result)) => match result {
                Ok((lease, batch, next_due)) => {
                    if let Some(row) = batch.oversized.first() {
                        runner.record_oversized(row);
                    }
                    if batch.claims.is_empty() {
                        let mut state = runner.state.lock();
                        state.pending = None;
                        state
                            .queues
                            .get_mut(&queue)
                            .expect("known queue")
                            .backoff(runner, next_due);
                    } else {
                        let mut state = runner.state.lock();
                        state.pending.as_mut().expect("reserved batch").slots = batch.claims.len();
                        state.counts.observe_queued_context_bytes(
                            batch.claims.iter().map(|claim| claim.context.len()).sum(),
                        );
                    }
                    for job in batch.claims {
                        let mut state = runner.state.lock();
                        let reservation = state.pending.as_mut().expect("reserved batch");
                        reservation.slots -= 1;
                        if reservation.slots == 0 {
                            state.pending = None;
                        }
                        attempts.push(launch_confirmed(
                            runner,
                            &mut state,
                            &queue,
                            lease,
                            job,
                            start_worker,
                        ));
                    }
                }
                Err(error) => {
                    {
                        let mut state = runner.state.lock();
                        state.pending = None;
                        state
                            .queues
                            .get_mut(&queue)
                            .expect("known queue")
                            .backoff(runner, None);
                    }
                    runner.storage_failure(Operation::Claim, error, None);
                }
            },
            Err(TryRecvError::Disconnected) => {
                runner.state.lock().pending = None;
                runner.storage_failure(
                    Operation::Claim,
                    "Jobs batch claim thread disconnected".into(),
                    None,
                );
                runner.request_stop(Duration::from_millis(runner.config.shutdown_grace_ms));
            }
            Err(TryRecvError::Empty) => {}
        }
    }
}

fn dispatch(
    runner: &Runner,
    pool: u64,
    workers: &str,
    start_worker: &StartWorker,
    sessions: Option<(JobSession, JobSession)>,
) {
    let Some((mut session, lifecycle)) = sessions else {
        dispatch_queues(runner, pool, workers, start_worker, None, None);
        return;
    };
    std::thread::scope(|scope| {
        let (requests, receive) = mpsc::sync_channel::<BatchRequest>(1);
        let (send, results) = mpsc::sync_channel(1);
        let config = &runner.config;
        let dispatcher = std::thread::current();
        let worker = std::thread::Builder::new()
            .name(format!("mesh-jobs-claim-{}", runner.id))
            .spawn_scoped(scope, move || {
                while let Ok(request) = receive.recv() {
                    let result = session.with(|db| {
                        let next_due = runner.probe_due(db, &request.queue, workers);
                        let lease = Lease::begin(Duration::from_millis(config.lease_ms))?;
                        let batch = claim_batch_with_timing(
                            db,
                            &config.namespace,
                            &request.queue,
                            workers,
                            config.lease_ms,
                            request.slots,
                            None,
                            runner.events.is_some(),
                        )?;
                        Ok((lease, batch, next_due))
                    });
                    if send.send((request.queue, result)).is_err() {
                        break;
                    }
                    dispatcher.unpark();
                }
            });
        match worker {
            Ok(_) => dispatch_queues(
                runner,
                pool,
                workers,
                start_worker,
                Some(BatchLane { requests, results }),
                Some(lifecycle),
            ),
            Err(error) => {
                runner.record_error(format!("could not start Jobs batch claim thread: {error}"));
                runner.request_stop(Duration::ZERO);
            }
        }
    });
}

fn dispatch_queues(
    runner: &Runner,
    pool: u64,
    workers: &str,
    start_worker: &StartWorker,
    batch: Option<BatchLane>,
    lifecycle: Option<JobSession>,
) {
    let config = &runner.config;
    let mut io = LifecycleIo {
        pool,
        session: lifecycle,
    };
    let mut last_queue = String::new();
    let mut attempts = Vec::<Attempt>::new();
    let drain_queues = config
        .drain
        .then(|| serde_json::json!(config.queues).to_string());
    let mut next_drain_recovery = Instant::now();
    let mut freed = HashSet::new();
    loop {
        if config.claim_batch && !unsafe { crate::db::pool::is_open(pool) } {
            runner.request_stop(Duration::from_millis(config.shutdown_grace_ms));
        }
        let previous_count = attempts.len();
        attempts.retain_mut(|attempt| {
            if attempt.poll(runner, &mut io) {
                true
            } else {
                runner.state.lock().jobs.remove(&attempt.token);
                if runner.wakeups.is_some() {
                    freed.insert(attempt.queue.clone());
                }
                false
            }
        });
        if let Some(lane) = &batch {
            lane.poll(runner, &mut attempts, start_worker);
        }
        if config.drain && attempts.len() != previous_count {
            for queue in runner.state.lock().queues.values_mut() {
                queue.ready(ClaimWake::Ready);
            }
            next_drain_recovery = Instant::now();
        }

        let (admitting, pending) = {
            let mut state = runner.state.lock();
            state.refresh_stopping_queues();
            let running = matches!(state.phase, Phase::Running);
            if attempts.is_empty() && state.pending.is_none() && !running {
                return;
            }
            (running, state.pending.is_some())
        };
        if !admitting {
            if let Some(wakeups) = &runner.wakeups {
                wakeups.close();
            }
        }
        if admitting && !pending {
            if let Some(wakeups) = &runner.wakeups {
                // Apply hints after the batch reply so an empty result from an
                // older snapshot cannot overwrite a hint arriving in flight.
                wakeups.apply(&mut runner.state.lock().queues, &mut freed);
            }
            // Take one fresh snapshot of all selected queues after recovery.
            // Individually empty claims can be stale or have skipped a lock.
            if let Some(selected) = drain_queues
                .as_deref()
                .filter(|_| attempts.is_empty() && Instant::now() >= next_drain_recovery)
            {
                match io.with(|db| {
                    match recover(
                        db,
                        &config.namespace,
                        config.recovery_page_size,
                        Some(selected),
                    ) {
                        Ok(recovered) => runner.record_maintenance(Operation::Recover, recovered),
                        Err(error) => {
                            runner.storage_failure(Operation::Recover, error, None);
                            return Ok(None);
                        }
                    }
                    drain_work(db, &config.namespace, selected, workers, config.claim_batch)
                        .map(Some)
                }) {
                    Ok(Some(work)) => {
                        if let Some(row) = &work.oversized {
                            runner.record_oversized(row);
                        }
                        if !work.pending {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => runner.storage_failure(Operation::DrainCheck, error, None),
                }
                next_drain_recovery = Instant::now()
                    + Duration::from_millis(
                        config
                            .recovery_interval_ms
                            .unwrap_or_else(|| config.poll_ms.clamp(10, 1000)),
                    );
            }
            let ready = {
                let state = runner.state.lock();
                let now = Instant::now();
                let eligible = |(name, queue): &(&String, &LocalQueue)| {
                    if state.jobs.len() >= MAX_IN_FLIGHT
                        || !matches!(queue.phase, QueuePhase::Running)
                        || queue.next_claim > now
                    {
                        return false;
                    }
                    let active = attempts.iter().filter(|attempt| &attempt.queue == *name);
                    !active.clone().any(Attempt::blocks_claims) && active.count() < queue.limit
                };
                state
                    .queues
                    .iter()
                    .filter(|(name, _)| *name > &last_queue)
                    .chain(
                        state
                            .queues
                            .iter()
                            .take_while(|(name, _)| *name <= &last_queue),
                    )
                    .find(eligible)
                    .map(|(name, _)| name.clone())
            };
            if let Some(queue) = ready {
                last_queue.clone_from(&queue);
                if let Some(lane) = &batch {
                    // Reserve lifecycle I/O before the claim thread can consume attempts.
                    if let Err(error) = io.with(|_| Ok(())) {
                        runner.record_error(error);
                        runner.defer_claim(&queue, None);
                        continue;
                    }
                    if let Err(error) = lane.request(runner, queue) {
                        runner.storage_failure(Operation::Claim, error, None);
                    }
                    continue;
                }
                match claim_one(runner, pool, workers, &queue, start_worker) {
                    Ok((Some(attempt), _)) => attempts.push(attempt),
                    Ok((None, next_due)) => runner.defer_claim(&queue, next_due),
                    Err(error) => {
                        runner.storage_failure(Operation::Claim, error, None);
                        runner.defer_claim(&queue, None);
                    }
                }
                continue;
            }
        }
        let tick = TICK.min(Duration::from_millis(config.poll_ms));
        let wait = if runner.wakeups.is_some() && admitting && attempts.is_empty() && !pending {
            runner
                .state
                .lock()
                .queues
                .values()
                .filter(|queue| matches!(queue.phase, QueuePhase::Running))
                .map(|queue| queue.next_claim.saturating_duration_since(Instant::now()))
                .min()
                .unwrap_or(tick)
        } else {
            tick
        };
        std::thread::park_timeout(wait);
    }
}

fn dispatch_bounded(
    runner: &Runner,
    pool: u64,
    workers: &str,
    start_worker: &StartWorker,
    options: DrainOptions,
) -> DrainResult {
    let config = &runner.config;
    let queues_json = serde_json::json!(config.queues).to_string();
    let selection = drain_store::Selection {
        namespace: &config.namespace,
        queues: &queues_json,
        workers,
        cutoff_ms: options
            .cutoff_ms
            .map(|cutoff| cutoff.to_string())
            .unwrap_or_default(),
    };
    let queues: Vec<_> = config.queues.keys().collect();
    let mut initial_ids = with_pool(pool, |db| {
        runner.state.lock().counts.recovered = selection.recover(db, options.max_jobs)?;
        if options.recursive {
            Ok(None)
        } else {
            selection.initial_ids(db, options.max_jobs).map(Some)
        }
    })?;

    let mut cursor = 0;
    let mut io = LifecycleIo {
        pool,
        session: None,
    };
    let reason = loop {
        {
            let state = runner.state.lock();
            if !matches!(state.phase, Phase::Running) {
                if let Some(error) = &state.last_error {
                    return Err(error.clone());
                }
                break "shutdown";
            }
            let counts = &state.counts;
            if counts.outcomes.total() != counts.claimed || counts.outcomes.released_unstarted > 0 {
                return Err(state.last_error.clone().unwrap_or_else(|| {
                    "Jobs.drain attempt did not commit a worker settlement".into()
                }));
            }
            if options.on_error == ErrorPolicy::Stop
                && (counts.outcomes.retryable > 0 || counts.outcomes.discarded > 0)
            {
                break "job_error";
            }
            if counts.claimed >= options.max_jobs {
                break "budget";
            }
        }
        if initial_ids.as_ref().is_some_and(Vec::is_empty) {
            break "idle";
        }

        let mut claimed = None;
        for offset in 0..queues.len() {
            let index = (cursor + offset) % queues.len();
            claimed = claim_attempt(runner, pool, queues[index], start_worker, |db| {
                selection.claim(
                    db,
                    queues[index],
                    config.lease_ms,
                    initial_ids.as_deref(),
                    runner.events.is_some(),
                )
            })?;
            if claimed.is_some() {
                cursor = (index + 1) % queues.len();
                break;
            }
            if !matches!(runner.state.lock().phase, Phase::Running) {
                break;
            }
        }
        let Some(mut attempt) = claimed else {
            if !matches!(runner.state.lock().phase, Phase::Running) {
                continue;
            }
            break "idle";
        };
        if let Some(ids) = &mut initial_ids {
            ids.retain(|id| *id != attempt.id);
        }
        while attempt.poll(runner, &mut io) {
            std::thread::park_timeout(TICK);
        }
        runner.state.lock().jobs.remove(&attempt.token);
    };

    let remaining = with_pool(pool, |db| selection.remaining(db))?;
    let state = runner.state.lock();
    let counts = &state.counts;
    Ok(serde_json::json!({
        "namespace": config.namespace,
        "stop_reason": reason,
        "budget_hit": counts.claimed >= options.max_jobs,
        "remaining": remaining,
        "claimed": counts.claimed,
        "finished": counts.outcomes.total(),
        "outcomes": counts.outcomes,
        "recovered": counts.recovered,
    })
    .to_string())
}

fn claim_one(
    runner: &Runner,
    pool: u64,
    workers: &str,
    queue: &str,
    start_worker: &StartWorker,
) -> Result<(Option<Attempt>, Option<Instant>), String> {
    if reserve_claims(runner, queue, 1) == 0 {
        return Ok((None, None));
    }
    let acquired = with_pool(pool, |db| {
        let next_due = runner.probe_due(db, queue, workers);
        let lease = Lease::begin(Duration::from_millis(runner.config.lease_ms))?;
        let job = claim_with_timing(
            db,
            &runner.config.namespace,
            queue,
            workers,
            runner.config.lease_ms,
            runner.events.is_some(),
        )?;
        Ok((lease, job, next_due))
    });
    let mut state = runner.state.lock();
    state.pending = None;
    let (lease, job, next_due) = acquired?;
    let attempt =
        job.map(|job| launch_confirmed(runner, &mut state, queue, lease, job, start_worker));
    Ok((attempt, next_due))
}

fn claim_attempt(
    runner: &Runner,
    pool: u64,
    queue: &str,
    start_worker: &StartWorker,
    acquire: impl FnOnce(&mut PgJobDb<'_>) -> Result<Option<Claim>, String>,
) -> Result<Option<Attempt>, String> {
    if reserve_claims(runner, queue, 1) == 0 {
        return Ok(None);
    }
    let acquired = with_pool(pool, |db| {
        let lease = Lease::begin(Duration::from_millis(runner.config.lease_ms))?;
        acquire(db).map(|job| (lease, job))
    });
    let mut state = runner.state.lock();
    state.pending = None;
    let (lease, job) = acquired?;
    Ok(job.map(|job| launch_confirmed(runner, &mut state, queue, lease, job, start_worker)))
}

fn reserve_claims(runner: &Runner, queue: &str, slots: usize) -> usize {
    if let Err(error) = (runner.check_worker)() {
        runner
            .state
            .lock()
            .fail_launch(error, runner.config.shutdown_grace_ms);
        return 0;
    }
    let mut state = runner.state.lock();
    if !matches!(state.phase, Phase::Running)
        || !matches!(state.queues[queue].phase, QueuePhase::Running)
        || state.pending.is_some()
        || crate::process_signal::shutdown_requested()
    {
        return 0;
    }
    let active = state.jobs.values().filter(|job| job.queue == queue).count();
    let slots = slots
        .min(state.queues[queue].limit.saturating_sub(active))
        .min(MAX_IN_FLIGHT.saturating_sub(state.jobs.len()));
    if slots > 0 {
        if let Some(wakeups) = &runner.wakeups {
            let queue = state.queues.get_mut(queue).expect("known queue");
            wakeups.record_scan(queue.source);
            queue.source = ClaimWake::Ready;
        }
        state.pending = Some(Reservation {
            queue: queue.into(),
            slots,
        });
    }
    slots
}

fn launch_confirmed(
    runner: &Runner,
    state: &mut State,
    queue: &str,
    lease: Lease,
    job: Claim,
    start_worker: &StartWorker,
) -> Attempt {
    let identity = runner.events.as_ref().map(|_| Identity::new(&job));
    let Claim {
        id,
        token,
        context,
        timeout_ms,
        ..
    } = job;
    state.counts.observe_queued_context_bytes(context.len());
    let now = Instant::now();
    let deadline = now.checked_add(Duration::from_millis(timeout_ms));
    let execution = if now >= lease.expires_at {
        Execution::acknowledging(Reply::Unstarted(FailureKind::LeaseLost), None)
    } else if !matches!(state.phase, Phase::Running)
        || !matches!(state.queues[queue].phase, QueuePhase::Running)
        || state.jobs.values().filter(|job| job.queue == queue).count() >= state.queues[queue].limit
        || crate::process_signal::shutdown_requested()
    {
        Execution::acknowledging(Reply::Unstarted(FailureKind::Shutdown), None)
    } else if let Some(deadline) = deadline {
        // The caller holds runner.state through registration, so the supervisor
        // cannot miss an attempt that replies during launch.
        match start_worker(
            context,
            deadline,
            lease.worker_expires_at,
            runner.reply_wake.clone(),
        ) {
            Ok(process) => {
                runner.publish(Kind::AttemptStarted, identity.as_ref());
                Execution::Running(Running {
                    process,
                    deadline,
                    lease_deadline: lease.expires_at,
                    reaped: false,
                    stop: None,
                    started_at: identity.as_ref().map(|_| now),
                })
            }
            Err(error) => {
                state.fail_launch(error, runner.config.shutdown_grace_ms);
                Execution::acknowledging(Reply::Unstarted(FailureKind::WorkerFailure), None)
            }
        }
    } else {
        Execution::acknowledging(
            Reply::Failed(
                FailureKind::WorkerFailure,
                "job timeout exceeds runtime timer range".into(),
            ),
            None,
        )
    };
    let mut job = ActiveJob {
        id: id.clone(),
        queue: queue.into(),
        execution,
        identity,
        reported_lease_loss: false,
    };
    job.observe_local_lease_loss(runner);
    state.jobs.insert(token.clone(), job);
    state.counts.claimed += 1;
    Attempt {
        id,
        token,
        queue: queue.into(),
        lease: Some(lease),
        submissions: 0,
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
