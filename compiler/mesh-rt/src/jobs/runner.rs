use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::actor::scheduler::ObservedExit;
use crate::actor::{self, ExitReason, ProcessId, ProcessState, Scheduler};
use crate::string::MeshString;

use super::cron;
use super::store::{claim, finish, recover, renew, Claim, LeaseStatus, Outcome};
use super::with_pool;

const TICK: Duration = Duration::from_millis(10);
const MAX_GRACE_MS: u64 = 300_000;
const MAX_RUNNERS: usize = 256;
const MAX_IN_FLIGHT: usize = 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_DELAY_MS: u64 = 365 * 24 * 60 * 60 * 1000;
const TIMEOUT: &str = "job handler timed out";
const STOPPED: &str = "job runner shutdown deadline elapsed";
const CANCELLED: &str = "job cancelled";
const LEASE_LOST: &str = "job lease lost";

type Handler = unsafe extern "C-unwind" fn(*const MeshString) -> *const MeshString;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    namespace: String,
    queues: BTreeMap<String, usize>,
    workers: Vec<Worker>,
    lease_ms: u64,
    poll_ms: u64,
    shutdown_grace_ms: u64,
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
    running: Option<(ProcessId, Arc<Invocation>)>,
}

#[derive(Default)]
struct Counts {
    claimed: u64,
    finished: u64,
    stale_acks: u64,
    recovered: u64,
    scheduled: u64,
}

struct State {
    phase: Phase,
    jobs: BTreeMap<String, ActiveJob>,
    pending_claims: usize,
    counts: Counts,
    last_error: Option<String>,
}

struct Runner<'a> {
    id: u64,
    config: Config,
    scheduler: &'a Scheduler,
    state: Mutex<State>,
}

impl Runner<'_> {
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
        let state = self.state.lock();
        let now = Instant::now();
        let shutting_down = match state.phase {
            Phase::Running => false,
            Phase::Stopping { deadline } => now >= deadline,
            Phase::Stopped => return false,
        };
        for job in state.jobs.values() {
            if let Some((pid, invocation)) = &job.running {
                let mut clock = invocation.clock.lock();
                let reason = clock.overdue(now).or(shutting_down.then_some(STOPPED));
                if let Some(reason) = reason {
                    clock.forced = Some(reason);
                    abort(self.scheduler, *pid, reason);
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
                "last_error": state.last_error,
            })
            .to_string(),
        )
    }

    fn wait(&self) -> String {
        loop {
            self.supervise();
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
    runners: BTreeMap<u64, Arc<Runner<'static>>>,
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
    if fn_ptr.is_null() || !env_ptr.is_null() {
        return Err("runner handler must be a named fn(String) -> String".into());
    }
    let config = Config::parse(config_json)?;
    let workers = serde_json::to_string(&config.workers).map_err(|error| error.to_string())?;
    let scheduler = actor::GLOBAL_SCHEDULER
        .get()
        .ok_or("actor scheduler is unavailable")?;

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
        scheduler,
        state: Mutex::new(State {
            phase: Phase::Running,
            jobs: BTreeMap::new(),
            pending_claims: 0,
            counts: Counts::default(),
            last_error: None,
        }),
    });
    crate::process_signal::mesh_process_install_shutdown_signals();
    let handler = unsafe { std::mem::transmute::<*const u8, Handler>(fn_ptr) };
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
            dispatch(&dispatcher, pool, &workers, handler);
            let _ = supervisor.join();
        });
    if let Err(error) = thread {
        runner.state.lock().phase = Phase::Stopped;
        return Err(format!("could not start job runner: {error}"));
    }
    registry.runners.insert(id, runner);
    Ok(id)
}

fn lookup(id: u64) -> Result<Arc<Runner<'static>>, String> {
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
    }
}

fn abort(scheduler: &Scheduler, pid: ProcessId, reason: &str) {
    if let Some(process) = scheduler.get_process(pid) {
        if let Some(process) = process.try_lock() {
            if !matches!(process.state, ProcessState::Exited(_)) {
                scheduler.force_exit(pid, process, ExitReason::Custom(reason.into()));
            }
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
    pid: ProcessId,
    invocation: Arc<Invocation>,
    exited: Receiver<ObservedExit>,
}

struct InvocationClock {
    deadline: Instant,
    lease_until: Instant,
    forced: Option<&'static str>,
}

impl InvocationClock {
    fn overdue(&self, now: Instant) -> Option<&'static str> {
        self.forced
            .or((now >= self.lease_until).then_some(LEASE_LOST))
            .or((now >= self.deadline).then_some(TIMEOUT))
    }
}

struct Invocation {
    clock: Mutex<InvocationClock>,
    result: Mutex<Option<String>>,
}

impl Running {
    fn start(
        scheduler: &Scheduler,
        handler: Handler,
        context: String,
        deadline: Instant,
        lease_until: Instant,
    ) -> Self {
        let invocation = Arc::new(Invocation {
            clock: Mutex::new(InvocationClock {
                deadline,
                lease_until,
                forced: None,
            }),
            result: Mutex::new(None),
        });
        let args = Arc::clone(&invocation);
        let (pid, exited) = scheduler.spawn_observed(move || {
            if args.clock.lock().overdue(Instant::now()).is_some() {
                return;
            }
            let input = crate::string::mesh_str(&context);
            let output = unsafe { handler(input).as_ref() };
            let output = output.and_then(|text| {
                let bytes = unsafe { text.as_bytes() };
                if bytes.len() > MAX_OUTPUT_BYTES {
                    return None;
                }
                std::str::from_utf8(bytes).ok().map(str::to_owned)
            });
            *args.result.lock() = output;
        });
        Self {
            pid,
            invocation,
            exited,
        }
    }

    fn poll(&self) -> Option<Reply> {
        match self.exited.try_recv() {
            Ok(exit) => Some(match exit.reason {
                ExitReason::Custom(reason) if reason == CANCELLED => Reply::Cancelled,
                ExitReason::Custom(reason) if reason == STOPPED || reason == LEASE_LOST => {
                    Reply::Failed(reason)
                }
                _ if exit.at >= self.invocation.clock.lock().deadline => {
                    Reply::Failed(TIMEOUT.into())
                }
                ExitReason::Normal if exit.returned_normally => {
                    self.invocation.result.lock().take().map_or_else(
                        || {
                            Reply::Failed(
                                "job handler did not return a valid outcome string".into(),
                            )
                        },
                        Reply::Returned,
                    )
                }
                ExitReason::Error(reason) | ExitReason::Custom(reason) => Reply::Failed(reason),
                reason => Reply::Failed(format!("job handler exited: {reason:?}")),
            }),
            Err(TryRecvError::Disconnected) => {
                Some(Reply::Failed("job actor exit observer disconnected".into()))
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    fn force(&self, scheduler: &Scheduler, reason: &'static str) {
        let mut clock = self.invocation.clock.lock();
        let reason = *clock.forced.get_or_insert(reason);
        abort(scheduler, self.pid, reason);
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

struct Attempt {
    id: String,
    token: String,
    queue: usize,
    lease: Option<Lease>,
    execution: Execution,
}

impl Attempt {
    fn poll(&mut self, runner: &Runner<'_>, pool: u64) -> bool {
        if let Execution::Running(running) = &self.execution {
            if let Some(reply) = running.poll() {
                if let Some(job) = runner.state.lock().jobs.get_mut(&self.token) {
                    job.running = None;
                }
                self.execution = Execution::Acknowledging {
                    reply,
                    retry_at: None,
                };
            }
        }
        let now = Instant::now();
        let config = &runner.config;
        let Some(lease) = &mut self.lease else {
            return matches!(self.execution, Execution::Running(_));
        };
        match &mut self.execution {
            Execution::Acknowledging { reply, retry_at } => {
                // A local lease deadline fences user execution. PostgreSQL
                // still decides whether this token may settle the attempt.
                if let Some(retry) = *retry_at {
                    if now >= lease.expires_at {
                        return false;
                    }
                    if now < retry {
                        return true;
                    }
                }
                match with_pool(pool, |db| {
                    finish(
                        db,
                        &config.namespace,
                        &self.id,
                        &self.token,
                        reply.outcome(),
                    )
                }) {
                    Ok(finished) => {
                        let mut state = runner.state.lock();
                        if finished {
                            state.counts.finished += 1;
                        } else {
                            state.counts.stale_acks += 1;
                        }
                        return false;
                    }
                    Err(error) => {
                        runner.record_error(error);
                        *retry_at = Some(Instant::now() + retry_interval(config));
                    }
                }
            }
            Execution::Running(running) => {
                if now >= lease.expires_at {
                    running.force(runner.scheduler, LEASE_LOST);
                    return true;
                }
                if lease.renew_at.is_none_or(|due| now < due) {
                    return true;
                }
                match with_pool(pool, |db| {
                    let started = Instant::now();
                    renew(
                        db,
                        &config.namespace,
                        &self.id,
                        &self.token,
                        config.lease_ms,
                    )
                    .map(|status| (started, status))
                }) {
                    Ok((started, LeaseStatus::Active)) => {
                        *lease = Lease::new(started, Duration::from_millis(config.lease_ms));
                        running.invocation.clock.lock().lease_until = lease.expires_at;
                    }
                    Ok((_, LeaseStatus::Cancelled)) => {
                        lease.renew_at = None;
                        running.force(runner.scheduler, CANCELLED);
                    }
                    Ok((_, LeaseStatus::Lost)) => {
                        self.lease = None;
                        running.force(runner.scheduler, LEASE_LOST);
                    }
                    Err(error) => {
                        runner.record_error(error);
                        lease.renew_at = Some(Instant::now() + retry_interval(config));
                    }
                }
            }
        }
        true
    }

    fn blocks_claims(&self) -> bool {
        self.lease
            .as_ref()
            .is_none_or(|lease| lease.renew_at.is_none() || Instant::now() >= lease.expires_at)
            && matches!(self.execution, Execution::Running(_))
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

fn dispatch(runner: &Runner<'_>, pool: u64, workers: &str, handler: Handler) {
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
    let mut next_recovery = Instant::now();
    let mut next_schedule = Instant::now();
    let producer = format!(
        "{}:{}:{:032x}",
        std::process::id(),
        runner.id,
        rand::random::<u128>()
    );
    loop {
        attempts.retain_mut(|attempt| {
            if attempt.poll(runner, pool) {
                true
            } else {
                runner.state.lock().jobs.remove(&attempt.token);
                false
            }
        });

        let admitting = {
            let mut state = runner.state.lock();
            let running = matches!(state.phase, Phase::Running);
            if !running && attempts.is_empty() {
                state.phase = Phase::Stopped;
                return;
            }
            running
        };
        if admitting {
            let now = Instant::now();
            if now >= next_recovery {
                match with_pool(pool, |db| recover(db, &config.namespace, 64)) {
                    Ok(recovered) => runner.state.lock().counts.recovered += recovered,
                    Err(error) => runner.record_error(error),
                }
                next_recovery =
                    Instant::now() + Duration::from_millis(config.poll_ms.clamp(100, 1000));
                continue;
            }
            if now >= next_schedule {
                match with_pool(pool, |db| cron::tick(db, &config.namespace, &producer, 64)) {
                    Ok(scheduled) => runner.state.lock().counts.scheduled += u64::from(scheduled),
                    Err(error) => runner.record_error(error),
                }
                next_schedule = Instant::now() + Duration::from_secs(1);
                continue;
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
                if let Some(attempt) =
                    claim_one(runner, pool, workers, index, queues[index].name, handler)
                {
                    attempts.push(attempt);
                } else {
                    queues[index].next_claim =
                        Instant::now() + Duration::from_millis(config.poll_ms);
                }
                continue;
            }
        }
        std::thread::park_timeout(TICK.min(Duration::from_millis(config.poll_ms)));
    }
}

fn claim_one(
    runner: &Runner<'_>,
    pool: u64,
    workers: &str,
    queue: usize,
    queue_name: &str,
    handler: Handler,
) -> Option<Attempt> {
    let config = &runner.config;
    {
        let mut state = runner.state.lock();
        if !matches!(state.phase, Phase::Running) || crate::process_signal::shutdown_requested() {
            return None;
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
        Ok((_, None)) => return None,
        Err(error) => {
            drop(state);
            runner.record_error(error);
            return None;
        }
    };

    let now = Instant::now();
    let lease = Lease::new(started, Duration::from_millis(config.lease_ms));
    let deadline = now.checked_add(Duration::from_millis(timeout_ms));
    let (execution, running) = if now >= lease.expires_at
        || !matches!(state.phase, Phase::Running)
        || crate::process_signal::shutdown_requested()
    {
        (
            Execution::Acknowledging {
                reply: Reply::Unstarted,
                retry_at: None,
            },
            None,
        )
    } else if let Some(deadline) = deadline {
        let running = Running::start(
            runner.scheduler,
            handler,
            context,
            deadline,
            lease.expires_at,
        );
        let observed = Some((running.pid, Arc::clone(&running.invocation)));
        (Execution::Running(running), observed)
    } else {
        let reply = Reply::Failed("job timeout exceeds runtime timer range".into());
        (
            Execution::Acknowledging {
                reply,
                retry_at: None,
            },
            None,
        )
    };
    state.jobs.insert(
        token.clone(),
        ActiveJob {
            id: id.clone(),
            queue,
            running,
        },
    );
    state.counts.claimed += 1;
    Some(Attempt {
        id,
        token,
        queue,
        lease: Some(lease),
        execution,
    })
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
