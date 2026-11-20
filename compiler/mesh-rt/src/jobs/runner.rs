use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::actor;

use super::cron;
use super::store::{
    claim, drain_has_work, finish, prune, recover, renew, Claim, LeaseStatus, Outcome,
};
use super::with_pool;
use super::worker::{handler_name, WorkerProcess};

const TICK: Duration = Duration::from_millis(10);
const MAX_GRACE_MS: u64 = 300_000;
const MAX_RUNNERS: usize = 256;
const MAX_IN_FLIGHT: usize = 1024;
const MAX_DELAY_MS: u64 = 365 * 24 * 60 * 60 * 1000;
const TIMEOUT: &str = "job handler timed out";
const STOPPED: &str = "job runner shutdown deadline elapsed";
const CANCELLED: &str = "job cancelled";
const LEASE_LOST: &str = "job lease lost";

trait JobProcess: Send {
    fn poll(&mut self) -> Result<Option<Result<String, String>>, String>;
    fn renew(&mut self, lease_deadline: Instant) -> Result<(), String>;
    fn kill(&mut self) -> Result<(), String>;
}

impl JobProcess for WorkerProcess {
    fn poll(&mut self) -> Result<Option<Result<String, String>>, String> {
        self.poll()
    }

    fn renew(&mut self, lease_deadline: Instant) -> Result<(), String> {
        self.renew(lease_deadline)
    }

    fn kill(&mut self) -> Result<(), String> {
        self.kill()
    }
}

type StartWorker = dyn Fn(String, Instant, Instant) -> Result<Box<dyn JobProcess>, String> + Send;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    namespace: String,
    queues: BTreeMap<String, usize>,
    workers: Vec<Worker>,
    lease_ms: u64,
    poll_ms: u64,
    shutdown_grace_ms: u64,
    prune_max_age_ms: Option<u64>,
    drain: bool,
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
            prune_max_age_ms: None,
            drain: false,
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
            .prune_max_age_ms
            .is_some_and(|age| age > MAX_DELAY_MS)
        {
            return Err("runner prune_max_age_ms must be at most 365 days".into());
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
}

struct ActiveJob {
    id: String,
    queue: usize,
    execution: Execution,
}

#[derive(Default)]
struct Counts {
    claimed: u64,
    finished: u64,
    stale_acks: u64,
    recovered: u64,
    scheduled: u64,
    pruned: u64,
}

struct State {
    phase: Phase,
    jobs: BTreeMap<String, ActiveJob>,
    pending_claims: usize,
    counts: Counts,
    last_error: Option<String>,
}

struct Runner {
    id: u64,
    config: Config,
    state: Mutex<State>,
}

impl Runner {
    fn request_stop(&self, grace: Duration) {
        let mut state = self.state.lock();
        let deadline = Instant::now() + grace;
        match &mut state.phase {
            Phase::Running => state.phase = Phase::Stopping { deadline },
            Phase::Stopping { deadline: current } => *current = (*current).min(deadline),
            Phase::Stopped => {}
        }
    }

    fn supervise(&self) -> bool {
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
        let State {
            jobs, last_error, ..
        } = &mut *state;
        for job in jobs.values_mut() {
            if let Execution::Running(worker) = &mut job.execution {
                if let Some(reply) = worker.poll(now, shutting_down, last_error) {
                    job.execution = Execution::acknowledging(reply);
                }
            }
        }
        true
    }

    fn report(&self, wait: bool) -> Option<String> {
        let state = self.state.lock();
        let undrained = state.jobs.len() + state.pending_claims;
        if wait {
            match state.phase {
                Phase::Running => return None,
                Phase::Stopping { deadline } if Instant::now() < deadline => return None,
                _ => {}
            }
        }

        let queues: BTreeMap<_, _> = self
            .config
            .queues
            .iter()
            .enumerate()
            .map(|(index, (name, limit))| {
                let in_flight = state.jobs.values().filter(|job| job.queue == index).count();
                (
                    name,
                    serde_json::json!({ "limit": limit, "in_flight": in_flight }),
                )
            })
            .collect();
        let jobs: Vec<_> = state.jobs.values().map(|job| &job.id).collect();
        Some(
            serde_json::json!({
                "runner_id": self.id,
                "namespace": self.config.namespace,
                "state": state.phase.name(),
                "drained": undrained == 0,
                "undrained": undrained,
                "undrained_jobs": jobs,
                "pending_claims": state.pending_claims,
                "queues": queues,
                "claimed": state.counts.claimed,
                "finished": state.counts.finished,
                "stale_acks": state.counts.stale_acks,
                "recovered": state.counts.recovered,
                "scheduled": state.counts.scheduled,
                "pruned": state.counts.pruned,
                "last_error": state.last_error,
            })
            .to_string(),
        )
    }

    fn wait(&self) -> String {
        loop {
            if let Some(report) = self.report(true) {
                return report;
            }
            actor::mesh_timer_sleep(TICK.as_millis() as i64);
        }
    }

    fn record_error(&self, error: String) {
        self.state.lock().last_error = Some(error.chars().take(1024).collect());
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
        Box::new(move |context, deadline, lease_deadline| {
            Ok(Box::new(WorkerProcess::start(
                &handler,
                context,
                deadline,
                lease_deadline,
            )?))
        }),
    )
}

fn start_runner(
    pool: u64,
    config_json: &str,
    start_worker: Box<StartWorker>,
) -> Result<u64, String> {
    let config = Config::parse(config_json)?;
    let workers = serde_json::to_string(&config.workers).map_err(|error| error.to_string())?;
    let scheduler = actor::GLOBAL_SCHEDULER
        .get()
        .ok_or("actor scheduler is unavailable")?;
    with_pool(pool, |db| {
        let rows = db.query(
            "SELECT (version = 1 AND to_regclass('mesh_jobs.cron_schedules') IS NOT NULL)::text
             FROM mesh_jobs.schema_version WHERE singleton",
            &[],
        )?;
        if rows
            .first()
            .and_then(|row| row.first())
            .is_some_and(|(_, ready)| ready == "true")
        {
            Ok(())
        } else {
            Err("Jobs requires the current schema. Run Jobs.migrate before starting workers".into())
        }
    })?;

    let mut registry = REGISTRY.lock();
    if !registry.accepting || scheduler.is_shutdown() || crate::process_signal::shutdown_requested()
    {
        return Err("runtime shutdown already requested".into());
    }
    if registry.runners.len() == MAX_RUNNERS {
        let stopped = registry.runners.iter().find_map(|(id, runner)| {
            matches!(runner.state.lock().phase, Phase::Stopped).then_some(*id)
        });
        registry
            .runners
            .remove(&stopped.ok_or("runner limit reached")?);
    }
    let id = registry.next_id;
    registry.next_id = id
        .checked_add(1)
        .filter(|&next| next <= i64::MAX as u64)
        .ok_or("runner identifiers exhausted")?;
    let runner = Arc::new(Runner {
        id,
        config,
        state: Mutex::new(State {
            phase: Phase::Running,
            jobs: BTreeMap::new(),
            pending_claims: 0,
            counts: Counts::default(),
            last_error: None,
        }),
    });
    crate::process_signal::mesh_process_install_shutdown_signals();
    let supervised = Arc::clone(&runner);
    let supervisor = std::thread::Builder::new()
        .name(format!("mesh-jobs-watch-{id}"))
        .spawn(move || {
            while supervised.supervise() {
                std::thread::park_timeout(TICK);
            }
        })
        .map_err(|error| format!("could not start job runner supervisor: {error}"))?;
    let dispatcher = Arc::clone(&runner);
    let thread = std::thread::Builder::new()
        .name(format!("mesh-jobs-{id}"))
        .spawn(move || {
            if dispatcher.config.drain {
                dispatch(&dispatcher, pool, &workers, &*start_worker);
            } else {
                let maintenance_runner = Arc::clone(&dispatcher);
                match std::thread::Builder::new()
                    .name(format!("mesh-jobs-maintenance-{id}"))
                    .spawn(move || maintain(&maintenance_runner, pool))
                {
                    Ok(maintenance) => {
                        dispatch(&dispatcher, pool, &workers, &*start_worker);
                        maintenance.thread().unpark();
                        let _ = maintenance.join();
                    }
                    Err(error) => {
                        dispatcher
                            .record_error(format!("could not start job maintenance: {error}"));
                    }
                }
            }
            dispatcher.state.lock().phase = Phase::Stopped;
            let _ = supervisor.join();
        });
    if let Err(error) = thread {
        runner.state.lock().phase = Phase::Stopped;
        return Err(format!("could not start job runner: {error}"));
    }
    registry.runners.insert(id, runner);
    Ok(id)
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
    Ok(lookup(id)?
        .report(false)
        .expect("status is always available"))
}

pub(super) fn stop(id: u64, grace_ms: u64) -> Result<String, String> {
    let runner = lookup(id)?;
    runner.request_stop(grace(grace_ms)?);
    Ok(runner.wait())
}

pub(super) fn join(id: u64) -> Result<String, String> {
    Ok(lookup(id)?.wait())
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
        runner.wait();
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

struct Lease {
    renew_at: Option<Instant>,
    expires_at: Instant,
}

impl Lease {
    fn new(from: Instant, duration: Duration) -> Self {
        // PostgreSQL starts the lease during the request, so its start time
        // cannot precede our own monotonic request start.
        Self {
            renew_at: Some(from + duration / 3),
            expires_at: from + duration,
        }
    }
}

struct Running {
    process: Box<dyn JobProcess>,
    deadline: Instant,
    lease_deadline: Instant,
    stop: Option<Reply>,
}

impl Running {
    fn renew_lease(&mut self, lease_deadline: Instant) -> Result<bool, String> {
        if Instant::now() >= self.lease_deadline {
            return Ok(false);
        }
        self.process.renew(lease_deadline)?;
        self.lease_deadline = lease_deadline;
        Ok(true)
    }

    fn poll(
        &mut self,
        now: Instant,
        shutting_down: bool,
        last_error: &mut Option<String>,
    ) -> Option<Reply> {
        let poll_error = match self.process.poll() {
            Ok(Some(result)) => {
                return Some(self.stop.take().unwrap_or_else(|| match result {
                    Ok(output) => Reply::Returned(output),
                    Err(error) => Reply::Failed(error),
                }));
            }
            Ok(None) => None,
            Err(error) => {
                *last_error = Some(error.chars().take(1024).collect());
                Some(error)
            }
        };
        if self.stop.is_none() {
            self.stop = if now >= self.lease_deadline && self.lease_deadline <= self.deadline {
                Some(Reply::Failed(LEASE_LOST.into()))
            } else if now >= self.deadline {
                Some(Reply::Failed(TIMEOUT.into()))
            } else if shutting_down {
                Some(Reply::Failed(STOPPED.into()))
            } else {
                poll_error.map(Reply::Failed)
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

enum Reply {
    Returned(String),
    Failed(String),
    Cancelled,
    Unstarted,
}

impl Reply {
    fn outcome(&self) -> Outcome {
        match self {
            Self::Returned(json) => match serde_json::from_str(json) {
                Ok(Outcome::RetryAfter { delay_ms, .. } | Outcome::Snooze { delay_ms })
                    if delay_ms > MAX_DELAY_MS =>
                {
                    Outcome::Fail {
                        reason: "job handler outcome delay exceeds 365 days".into(),
                    }
                }
                Ok(outcome) => outcome,
                Err(_) => Outcome::Fail {
                    reason: "invalid job handler outcome JSON".into(),
                },
            },
            Self::Failed(reason) => Outcome::Fail {
                reason: reason.chars().take(1024).collect(),
            },
            Self::Cancelled => Outcome::Cancel {
                reason: CANCELLED.into(),
            },
            Self::Unstarted => Outcome::Snooze { delay_ms: 1 },
        }
    }
}

enum Execution {
    Running(Running),
    Acknowledging {
        reply: Reply,
        retry_at: Option<Instant>,
    },
}

impl Execution {
    fn acknowledging(reply: Reply) -> Self {
        Self::Acknowledging {
            reply,
            retry_at: None,
        }
    }
}

struct Attempt {
    id: String,
    token: String,
    queue: usize,
    lease: Option<Lease>,
}

impl Attempt {
    fn poll(&mut self, runner: &Runner, pool: u64) -> bool {
        let outcome = {
            let state = runner.state.lock();
            match &state.jobs[&self.token].execution {
                Execution::Running(_) => None,
                Execution::Acknowledging { reply, retry_at } => {
                    let Some(lease) = &self.lease else {
                        return false;
                    };
                    // The local deadline fences execution. PostgreSQL decides
                    // whether the token can still settle the reaped attempt.
                    if let Some(retry) = retry_at {
                        let now = Instant::now();
                        if now >= lease.expires_at {
                            return false;
                        }
                        if now < *retry {
                            return true;
                        }
                    }
                    Some(reply.outcome())
                }
            }
        };
        let Some(outcome) = outcome else {
            self.renew(runner, pool);
            return true;
        };
        match with_pool(pool, |db| {
            finish(db, &runner.config.namespace, &self.id, &self.token, outcome)
        }) {
            Ok(finished) => {
                let mut state = runner.state.lock();
                if finished {
                    state.counts.finished += 1;
                } else {
                    state.counts.stale_acks += 1;
                }
                false
            }
            Err(error) => {
                runner.record_error(error);
                let mut state = runner.state.lock();
                let job = state.jobs.get_mut(&self.token).expect("tracked attempt");
                if let Execution::Acknowledging { retry_at, .. } = &mut job.execution {
                    *retry_at = Some(Instant::now() + retry_interval(&runner.config));
                }
                true
            }
        }
    }

    fn renew(&mut self, runner: &Runner, pool: u64) {
        let Some(lease) = &mut self.lease else {
            return;
        };
        let now = Instant::now();
        if now >= lease.expires_at || lease.renew_at.is_none_or(|due| now < due) {
            return;
        }
        let config = &runner.config;
        let renewal = with_pool(pool, |db| {
            let started = Instant::now();
            if started >= lease.expires_at {
                return Ok(None);
            }
            renew(
                db,
                &config.namespace,
                &self.id,
                &self.token,
                config.lease_ms,
            )
            .map(|status| Some((started, status)))
        });
        let (started, status) = match renewal {
            Ok(Some(result)) => result,
            Ok(None) => return,
            Err(error) => {
                runner.record_error(error);
                lease.renew_at = Some(Instant::now() + retry_interval(config));
                return;
            }
        };
        let mut state = runner.state.lock();
        let State {
            jobs, last_error, ..
        } = &mut *state;
        let job = jobs.get_mut(&self.token).expect("tracked attempt");
        let running = match &mut job.execution {
            Execution::Running(worker) => Some(worker),
            Execution::Acknowledging { .. } => None,
        };
        match status {
            LeaseStatus::Active => {
                let renewed = Lease::new(started, Duration::from_millis(config.lease_ms));
                if let Some(worker) = running {
                    match worker.renew_lease(renewed.expires_at) {
                        Ok(true) => {}
                        Ok(false) => {
                            worker.stop(Reply::Failed(LEASE_LOST.into()), last_error);
                            return;
                        }
                        Err(error) => {
                            *last_error = Some(error.chars().take(1024).collect());
                            worker.stop(Reply::Failed(error), last_error);
                            return;
                        }
                    }
                }
                *lease = renewed;
            }
            LeaseStatus::Cancelled => {
                lease.renew_at = None;
                if let Some(worker) = running {
                    worker.stop(Reply::Cancelled, last_error);
                }
            }
            LeaseStatus::Lost => {
                self.lease = None;
                if let Some(worker) = running {
                    worker.stop(Reply::Failed(LEASE_LOST.into()), last_error);
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

struct Queue<'a> {
    name: &'a str,
    limit: usize,
    next_claim: Instant,
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
            match with_pool(pool, |db| recover(db, &config.namespace, 64, None)) {
                Ok(recovered) => runner.state.lock().counts.recovered += recovered,
                Err(error) => runner.record_error(error),
            }
            next_recovery = Instant::now() + Duration::from_millis(config.poll_ms.clamp(100, 1000));
        }
        if now >= next_schedule {
            match with_pool(pool, |db| cron::tick(db, &config.namespace, &producer, 64)) {
                Ok(scheduled) => runner.state.lock().counts.scheduled += u64::from(scheduled),
                Err(error) => runner.record_error(error),
            }
            next_schedule = Instant::now() + Duration::from_secs(1);
        }
        if let Some(age) = config.prune_max_age_ms.filter(|_| now >= next_prune) {
            match with_pool(pool, |db| {
                prune(db, &config.namespace, None, None, Some(age), Some(1000))
            }) {
                Ok(count) => runner.state.lock().counts.pruned += count,
                Err(error) => runner.record_error(error),
            }
            next_prune = Instant::now() + Duration::from_secs(60);
        }
        std::thread::park_timeout(TICK);
    }
}

fn dispatch(runner: &Runner, pool: u64, workers: &str, start_worker: &StartWorker) {
    let config = &runner.config;
    let mut queues: Vec<_> = config
        .queues
        .iter()
        .map(|(name, limit)| Queue {
            name,
            limit: *limit,
            next_claim: Instant::now(),
        })
        .collect();
    let mut cursor = 0;
    let mut attempts = Vec::<Attempt>::new();
    let drain_queues = config
        .drain
        .then(|| serde_json::json!(config.queues).to_string());
    let mut next_drain_recovery = Instant::now();
    loop {
        let previous_count = attempts.len();
        attempts.retain_mut(|attempt| {
            if attempt.poll(runner, pool) {
                true
            } else {
                runner.state.lock().jobs.remove(&attempt.token);
                false
            }
        });
        if config.drain && attempts.len() != previous_count {
            for queue in &mut queues {
                queue.next_claim = Instant::now();
            }
            next_drain_recovery = Instant::now();
        }

        let admitting = {
            let state = runner.state.lock();
            let running = matches!(state.phase, Phase::Running);
            if attempts.is_empty() && !running {
                return;
            }
            running
        };
        if admitting {
            // Take one fresh snapshot of all selected queues after recovery.
            // Individually empty claims can be stale or have skipped a lock.
            if let Some(selected) = drain_queues
                .as_deref()
                .filter(|_| attempts.is_empty() && Instant::now() >= next_drain_recovery)
            {
                match with_pool(pool, |db| {
                    let recovered = recover(db, &config.namespace, 64, Some(selected))?;
                    runner.state.lock().counts.recovered += recovered;
                    drain_has_work(db, &config.namespace, selected, workers)
                }) {
                    Ok(false) => return,
                    Ok(true) => {}
                    Err(error) => runner.record_error(error),
                }
                next_drain_recovery =
                    Instant::now() + Duration::from_millis(config.poll_ms.clamp(10, 1000));
            }
            let ready = (0..queues.len())
                .map(|offset| (cursor + offset) % queues.len())
                .find(|&index| {
                    let queue = &queues[index];
                    let active = attempts.iter().filter(|attempt| attempt.queue == index);
                    queue.next_claim <= Instant::now()
                        && !active.clone().any(Attempt::blocks_claims)
                        && active.count() < queue.limit
                });
            if let Some(index) = ready {
                cursor = (index + 1) % queues.len();
                match claim_one(
                    runner,
                    pool,
                    workers,
                    index,
                    queues[index].name,
                    start_worker,
                ) {
                    Ok(Some(attempt)) => attempts.push(attempt),
                    Ok(None) => {
                        queues[index].next_claim =
                            Instant::now() + Duration::from_millis(config.poll_ms);
                    }
                    Err(error) => {
                        runner.record_error(error);
                        queues[index].next_claim =
                            Instant::now() + Duration::from_millis(config.poll_ms);
                    }
                }
                continue;
            }
        }
        std::thread::park_timeout(TICK.min(Duration::from_millis(config.poll_ms)));
    }
}

fn claim_one(
    runner: &Runner,
    pool: u64,
    workers: &str,
    queue: usize,
    queue_name: &str,
    start_worker: &StartWorker,
) -> Result<Option<Attempt>, String> {
    let config = &runner.config;
    {
        let mut state = runner.state.lock();
        if !matches!(state.phase, Phase::Running) || crate::process_signal::shutdown_requested() {
            return Ok(None);
        }
        state.pending_claims = 1;
    }

    let acquired = with_pool(pool, |db| {
        let started = Instant::now();
        claim(db, &config.namespace, queue_name, workers, config.lease_ms).map(|job| (started, job))
    });
    let mut state = runner.state.lock();
    state.pending_claims = 0;
    let (
        started,
        Claim {
            id,
            token,
            context,
            timeout_ms,
        },
    ) = match acquired {
        Ok((started, Some(job))) => (started, job),
        Ok((_, None)) => return Ok(None),
        Err(error) => return Err(error),
    };

    let now = Instant::now();
    let lease = Lease::new(started, Duration::from_millis(config.lease_ms));
    let deadline = now.checked_add(Duration::from_millis(timeout_ms));
    let execution = if now >= lease.expires_at
        || !matches!(state.phase, Phase::Running)
        || crate::process_signal::shutdown_requested()
    {
        Execution::acknowledging(Reply::Unstarted)
    } else if let Some(deadline) = deadline {
        match start_worker(context, deadline, lease.expires_at) {
            Ok(process) => Execution::Running(Running {
                process,
                deadline,
                lease_deadline: lease.expires_at,
                stop: None,
            }),
            Err(error) => Execution::acknowledging(Reply::Failed(error)),
        }
    } else {
        Execution::acknowledging(Reply::Failed(
            "job timeout exceeds runtime timer range".into(),
        ))
    };
    state.jobs.insert(
        token.clone(),
        ActiveJob {
            id: id.clone(),
            queue,
            execution,
        },
    );
    state.counts.claimed += 1;
    Ok(Some(Attempt {
        id,
        token,
        queue,
        lease: Some(lease),
    }))
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
