use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::pg::jobs_effect_session::{
    EffectAuthority, EffectFailure, EffectReceipt, EffectSession, IO_TIMEOUT,
};
use crate::db::quote_ident;
use crate::json::{bounded_json, MeshJson};

use super::effect_pool::{Checkout, CheckoutError, SessionPool};
use super::runner::JobProcess;
use super::worker::{read_frame, write_frame, WorkerPoll, WorkerProcess};

pub(super) const MAX_REGISTRY_BYTES: usize = 16 * 1024;
const MAX_FRAME: usize = 8 * 1024;
const MAX_OPERATIONS: usize = 16;
const MAX_ARGUMENTS: usize = 8;
const MAX_TEXT: usize = 512;
const UNKNOWN: &str = "Jobs effect outcome is unconfirmed";
const INVALID: &str = "Jobs effect name or arguments were rejected";
const REJECTED: &str = "Jobs effect was rejected by PostgreSQL";
const UNAVAILABLE: &str = "Jobs effect session is unavailable";
const CHECKOUT_REPLY_RESERVE: Duration = Duration::from_millis(100);
const PEER_PROBE: Duration = Duration::from_millis(10);
const CLAIM_ARGUMENTS: &[&str] = &["text", "uuid", "int8", "int4", "uuid"];

// Admission checks an exact, standalone, non-variadic application definer.
const CHECK_ROUTINE: &str = "SELECT EXISTS (
    SELECT 1 FROM pg_catalog.pg_proc p
    WHERE p.oid = pg_catalog.to_regprocedure($1::pg_catalog.text)
      AND p.prokind = 'f' AND p.prosecdef AND NOT p.proretset
      AND p.provolatile = 'v' AND p.provariadic = 0 AND p.pronargdefaults = 0
      AND p.prorettype = 'pg_catalog.bool'::pg_catalog.regtype
      AND p.proowner <> (SELECT oid FROM pg_catalog.pg_roles WHERE rolname = current_user)
      AND pg_catalog.has_function_privilege(p.oid, 'EXECUTE')
      AND pg_catalog.has_schema_privilege(p.pronamespace, 'USAGE')
      AND NOT pg_catalog.has_schema_privilege(p.pronamespace, 'CREATE')
) AS receipt";
const CHECK_ROLE: &str = "SELECT EXISTS (
    SELECT 1 FROM pg_catalog.pg_roles
    WHERE rolname = current_user AND rolcanlogin AND NOT rolsuper
      AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolbypassrls AND NOT rolreplication
      AND NOT pg_catalog.has_schema_privilege('mesh_jobs', 'USAGE')
      AND NOT pg_catalog.has_schema_privilege('mesh_jobs_effects', 'USAGE')
) AS receipt";

static CHILD_ENDPOINT: Mutex<Option<ChildEndpoint>> = Mutex::new(None);

enum ChildEndpoint {
    Ready(UnixStream),
    Unavailable,
    Uncertain,
}

impl ChildEndpoint {
    fn request(
        &mut self,
        name: &str,
        values: Vec<Value>,
        timeout: Duration,
    ) -> Result<bool, &'static str> {
        let socket = match self {
            Self::Ready(socket) => socket,
            Self::Unavailable => return Err(UNAVAILABLE),
            Self::Uncertain => return Err(UNKNOWN),
        };
        let request = Request::new(name, values, timeout).map_err(|_| UNAVAILABLE)?;
        let mut wire = TimedWire::new(socket, WireDeadline::Shared(request.deadline));
        // A write error leaves the relay with no complete frame. The unbuffered
        // flush cannot turn a completed frame into an unsent error.
        if write_frame(&mut wire, &request, MAX_FRAME).is_err() {
            *self = Self::Unavailable;
            return Err(UNAVAILABLE);
        }
        match wire.read_receipt() {
            Ok(Receipt::Confirmed(applied)) => Ok(applied),
            Ok(Receipt::Invalid) => Err(INVALID),
            Ok(Receipt::Unavailable) => Err(UNAVAILABLE),
            Ok(Receipt::Rejected) => Err(REJECTED),
            Err(_) => {
                *self = Self::Uncertain;
                Err(UNKNOWN)
            }
        }
    }

    fn finish(&mut self, reply: Result<String, String>) -> Result<String, String> {
        if matches!(self, Self::Uncertain) {
            return Err(UNKNOWN.into());
        }
        *self = Self::Unavailable;
        reply
    }
}

pub(super) fn enter_worker(socket: UnixStream) {
    *CHILD_ENDPOINT.lock() = Some(ChildEndpoint::Ready(socket));
}

pub(super) fn finish_worker(reply: Result<String, String>) -> Result<String, String> {
    match CHILD_ENDPOINT.lock().as_mut() {
        Some(endpoint) => endpoint.finish(reply),
        None => reply,
    }
}

pub(super) unsafe fn call(name: &str, values: *const MeshJson) -> Result<bool, &'static str> {
    if !identifier(name) {
        return Err(INVALID);
    }
    let mut endpoint = CHILD_ENDPOINT.lock();
    let endpoint = endpoint
        .as_mut()
        .ok_or("Jobs.effect requires an active effect worker")?;
    let values = bounded_json(values, 2, MAX_FRAME - 128).map_err(|_| INVALID)?;
    let values = serde_json::from_slice::<Vec<Value>>(&values).map_err(|_| INVALID)?;
    if values.len() > MAX_ARGUMENTS || values.iter().any(|value| scalar(value).is_none()) {
        return Err(INVALID);
    }
    endpoint.request(name, values, IO_TIMEOUT)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    name: String,
    values: Vec<Value>,
    deadline: MonotonicDeadline,
}

impl Request {
    fn new(name: &str, values: Vec<Value>, timeout: Duration) -> io::Result<Self> {
        Ok(Self {
            name: name.into(),
            values,
            deadline: MonotonicDeadline(
                monotonic_nanos()?.saturating_add(timeout.as_nanos() as u64),
            ),
        })
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(transparent)]
struct MonotonicDeadline(u64);

impl MonotonicDeadline {
    fn remaining(self) -> io::Result<Duration> {
        Ok(Duration::from_nanos(
            self.0.saturating_sub(monotonic_nanos()?),
        ))
    }

    fn clamp(self, parent_deadline: Instant) -> io::Result<Instant> {
        // Sample Instant first so preemption during conversion can only
        // shorten the relay's budget relative to the worker's wire deadline.
        let now = Instant::now();
        Ok(parent_deadline.min(now + self.remaining()?.min(IO_TIMEOUT)))
    }
}

fn monotonic_nanos() -> io::Result<u64> {
    let mut clock = std::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, clock.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let clock = unsafe { clock.assume_init() };
    Ok(clock.tv_sec as u64 * 1_000_000_000 + clock.tv_nsec as u64)
}

enum Receipt {
    Confirmed(bool),
    Invalid,
    Unavailable,
    Rejected,
}

impl Receipt {
    // One byte fits in a nonblocking write while the owner is fenced.
    fn byte(self) -> u8 {
        match self {
            Self::Confirmed(true) => 1,
            Self::Confirmed(false) => 2,
            Self::Invalid => 3,
            Self::Unavailable => 4,
            Self::Rejected => 5,
        }
    }
}

enum WireDeadline {
    Local(Instant),
    Shared(MonotonicDeadline),
}

struct TimedWire<'a> {
    socket: &'a UnixStream,
    deadline: WireDeadline,
}

impl<'a> TimedWire<'a> {
    fn new(socket: &'a UnixStream, deadline: WireDeadline) -> Self {
        Self { socket, deadline }
    }

    fn bound(&self) -> io::Result<Option<std::time::Duration>> {
        let left = match self.deadline {
            WireDeadline::Local(deadline) => deadline.saturating_duration_since(Instant::now()),
            WireDeadline::Shared(deadline) => deadline.remaining()?,
        };
        if left.is_zero() {
            Err(io::ErrorKind::TimedOut.into())
        } else {
            Ok(Some(left))
        }
    }

    fn read_receipt(&mut self) -> io::Result<Receipt> {
        let mut byte = [0];
        self.read_exact(&mut byte)?;
        match byte[0] {
            1 => Ok(Receipt::Confirmed(true)),
            2 => Ok(Receipt::Confirmed(false)),
            3 => Ok(Receipt::Invalid),
            4 => Ok(Receipt::Unavailable),
            5 => Ok(Receipt::Rejected),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}

impl Read for TimedWire<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.socket.set_read_timeout(self.bound()?)?;
        self.socket.read(buffer)
    }
}

impl Write for TimedWire<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.socket.set_write_timeout(self.bound()?)?;
        self.socket.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        // UnixStream is unbuffered. Success means the frame was sent even if
        // the deadline has now elapsed and no reply can be read.
        Ok(())
    }
}

fn publish(socket: &UnixStream, receipt: Receipt) -> io::Result<()> {
    let byte = receipt.byte();
    match unsafe {
        libc::send(
            socket.as_raw_fd(),
            std::ptr::from_ref(&byte).cast(),
            1,
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        )
    } {
        1 => Ok(()),
        -1 => Err(io::Error::last_os_error()),
        _ => Err(io::ErrorKind::WriteZero.into()),
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Text,
    Int,
    Bool,
}

impl Kind {
    fn pg(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Int => "int8",
            Self::Bool => "bool",
        }
    }
}

fn scalar(value: &Value) -> Option<(Kind, String)> {
    Some(match value {
        Value::String(text) if text.len() <= MAX_TEXT && !text.contains('\0') => {
            (Kind::Text, text.clone())
        }
        Value::Number(number) => (Kind::Int, number.as_i64()?.to_string()),
        Value::Bool(value) => (Kind::Bool, value.to_string()),
        _ => return None,
    })
}

fn identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    !text.is_empty()
        && text.len() <= 63
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    name: String,
    routine: String,
    args: Vec<Kind>,
}

struct Operation {
    signature: String,
    sql: String,
    args: Vec<Kind>,
}

impl Operation {
    fn parse(registry: &[u8]) -> Result<BTreeMap<String, Self>, String> {
        let invalid = "invalid Jobs effect registry";
        let entries: Vec<Declaration> = serde_json::from_slice(registry).map_err(|_| invalid)?;
        if entries.is_empty() || entries.len() > MAX_OPERATIONS {
            return Err(invalid.into());
        }
        let mut operations = BTreeMap::new();
        for Declaration {
            name,
            routine,
            args,
        } in entries
        {
            let (schema, function) = routine.split_once('.').ok_or(invalid)?;
            if !identifier(&name)
                || !identifier(schema)
                || !identifier(function)
                || schema.starts_with("pg_")
                || matches!(
                    schema,
                    "information_schema" | "mesh_jobs" | "mesh_jobs_effects"
                )
                || args.len() > MAX_ARGUMENTS
            {
                return Err(invalid.into());
            }
            let routine = format!("{}.{}", quote_ident(schema), quote_ident(function));
            let types = CLAIM_ARGUMENTS
                .iter()
                .copied()
                .chain(args.iter().map(|arg| arg.pg()))
                .map(|kind| format!("pg_catalog.{kind}"))
                .collect::<Vec<_>>();
            let signature = format!("{routine}({})", types.join(","));
            let parameters = types
                .iter()
                .enumerate()
                .map(|(index, kind)| format!("${}::{kind}", index + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("SELECT {routine}({parameters}) AS receipt");
            if operations
                .insert(
                    name,
                    Self {
                        signature,
                        sql,
                        args,
                    },
                )
                .is_some()
            {
                return Err(invalid.into());
            }
        }
        Ok(operations)
    }

    fn bind(&self, values: &[Value]) -> Option<Vec<String>> {
        if self.args.len() != values.len() {
            return None;
        }
        self.args
            .iter()
            .zip(values)
            .map(|(expected, value)| {
                let (actual, text) = scalar(value)?;
                (*expected == actual).then_some(text)
            })
            .collect()
    }
}

struct Identity {
    namespace: String,
    id: String,
    run: String,
    attempt: String,
    token: String,
}

impl Identity {
    fn from_claim(namespace: &str, id: &str, token: &str, context: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Context<'a> {
            id: &'a str,
            effect_key: &'a str,
            attempt: i32,
        }

        let invalid = "invalid confirmed Jobs claim identity";
        let context: Context<'_> = serde_json::from_str(context).map_err(|_| invalid)?;
        let (effect_id, run) = context.effect_key.split_once('/').ok_or(invalid)?;
        if id != context.id
            || id != effect_id
            || context.attempt < 1
            || !run.parse::<i64>().is_ok_and(|run| run > 0)
        {
            return Err(invalid.into());
        }
        Ok(Self {
            namespace: namespace.into(),
            id: id.into(),
            run: run.into(),
            attempt: context.attempt.to_string(),
            token: token.into(),
        })
    }

    fn parameters(&self) -> [&str; 5] {
        [
            &self.namespace,
            &self.id,
            &self.run,
            &self.attempt,
            &self.token,
        ]
    }
}

struct Shared {
    operations: BTreeMap<String, Operation>,
    sessions: SessionPool<EffectSession>,
}

pub(super) struct Effects {
    shared: Arc<Shared>,
    handler: String,
}

impl Effects {
    pub(super) fn open(
        handler: String,
        jobs_url: &str,
        app_url: &str,
        operations: &[u8],
        lanes: usize,
    ) -> Result<Self, String> {
        let operations = Operation::parse(operations)?;
        let mut sessions = Vec::with_capacity(lanes);
        for _ in 0..lanes {
            let mut session = EffectSession::open(app_url, jobs_url)?;
            if !session.check(CHECK_ROLE, &[]) {
                return Err(
                    "Jobs effect login is missing or exceeds the application role scope".into(),
                );
            }
            for (name, operation) in &operations {
                if !session.check(CHECK_ROUTINE, &[&operation.signature]) {
                    return Err(format!(
                        "Jobs effect operation {name} requires an executable named application definer"
                    ));
                }
            }
            if !session.reset(Instant::now() + IO_TIMEOUT, None) {
                return Err("could not reset the warm Jobs effect session".into());
            }
            sessions.push(session);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                operations,
                sessions: SessionPool::new(sessions),
            }),
            handler,
        })
    }

    #[cfg(test)]
    pub(super) fn retired_fixture() -> Self {
        Self {
            shared: Arc::new(Shared {
                operations: BTreeMap::new(),
                sessions: SessionPool::new(vec![]),
            }),
            handler: String::new(),
        }
    }

    pub(super) fn exhausted_permanently(&self) -> bool {
        self.shared.sessions.exhausted_permanently()
    }

    pub(super) fn close(&self) {
        self.shared.sessions.close();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        &self,
        namespace: &str,
        id: &str,
        token: &str,
        context: String,
        deadline: Instant,
        lease: Instant,
        worker_lease: u64,
        reply_wake: Sender<()>,
    ) -> Result<Box<dyn JobProcess>, String> {
        let identity = Identity::from_claim(namespace, id, token, &context)?;
        let authority = Arc::new(EffectAuthority::new(deadline, lease));
        let (relay, child) = UnixStream::pair().map_err(|error| error.to_string())?;
        let shutdown = relay.try_clone().map_err(|error| error.to_string())?;
        let owner = authority.clone();
        let shared = self.shared.clone();
        let (activate, activated) = crossbeam_channel::bounded(1);
        let relay_done = Arc::new(AtomicBool::new(false));
        let completion = RelayCompletion {
            done: relay_done.clone(),
            wake: reply_wake.clone(),
        };
        let _relay_thread = thread::Builder::new()
            .name("mesh-job-effects".into())
            .spawn(move || {
                let _completion = completion;
                let relay = Relay(relay);
                if activated.recv().is_ok() {
                    serve(relay, deadline, &identity, &owner, &shared);
                }
            })
            .map_err(|error| error.to_string())?;
        // If worker launch fails, dropping activate releases the inactive
        // relay. The runner holds its state lock here.
        let worker = WorkerProcess::start_effects(
            &self.handler,
            context,
            deadline,
            worker_lease,
            reply_wake,
            child,
        )?;
        Ok(Box::new(EffectProcess {
            worker,
            authority,
            shared: self.shared.clone(),
            activate: Some(activate),
            shutdown,
            relay_done,
            waiting_for_relay: false,
            terminal: None,
        }))
    }
}

struct Relay(UnixStream);

impl Drop for Relay {
    fn drop(&mut self) {
        // The supervisor still owns a duplicate. Shut down the connection
        // so further requests fail to write. The peer can drain queued receipts.
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

struct RelayCompletion {
    done: Arc<AtomicBool>,
    wake: Sender<()>,
}

impl Drop for RelayCompletion {
    fn drop(&mut self) {
        // The checkout and relay socket are gone before the supervisor can
        // act on this wake, including if the relay unwinds.
        self.done.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
    }
}

fn worker_endpoint_open(socket: &UnixStream) -> bool {
    let mut fd = libc::pollfd {
        fd: socket.as_raw_fd(),
        events: libc::POLLRDHUP,
        revents: 0,
    };
    // Linux reports hangup even with unread request bytes. Poll errors also
    // deny checkout. The exact child's exit is observed separately by try_wait.
    unsafe { libc::poll(&mut fd, 1, 0) == 0 }
}

fn checkout_for_call<'a, S>(
    sessions: &'a SessionPool<S>,
    socket: &UnixStream,
    owner: &EffectAuthority,
    io_deadline: Instant,
) -> Result<Checkout<'a, S>, CheckoutError> {
    // Leave time for a no-SQL receipt while the worker is still reading.
    sessions.checkout(io_deadline, CHECKOUT_REPLY_RESERVE, PEER_PROBE, || {
        owner.live() && worker_endpoint_open(socket)
    })
}

fn serve(
    relay: Relay,
    deadline: Instant,
    identity: &Identity,
    owner: &EffectAuthority,
    shared: &Shared,
) {
    let mut socket = &relay.0;
    while owner.live() {
        // Idle handler time owns no checkout. Once the first byte arrives,
        // the rest of the framed exchange has a fresh bounded I/O deadline.
        let mut first = [0];
        if socket.set_read_timeout(None).is_err() || socket.read_exact(&mut first).is_err() {
            return;
        }
        let io_deadline = deadline.min(Instant::now() + IO_TIMEOUT);
        let mut frame =
            io::Cursor::new(first).chain(TimedWire::new(socket, WireDeadline::Local(io_deadline)));
        let Ok(request) = read_frame::<Request>(&mut frame, MAX_FRAME) else {
            return;
        };
        let Ok(io_deadline) = request.deadline.clamp(io_deadline) else {
            return;
        };
        let Some((op, values)) = shared
            .operations
            .get(&request.name)
            .and_then(|op| Some((op, op.bind(&request.values)?)))
        else {
            if !owner.publish_without_checkout(|| publish(socket, Receipt::Invalid)) {
                return;
            }
            continue;
        };
        let mut checkout = match checkout_for_call(&shared.sessions, socket, owner, io_deadline) {
            Ok(checkout) => checkout,
            Err(CheckoutError::Cancelled) => return,
            Err(CheckoutError::Unavailable) => {
                if !owner.publish_without_checkout(|| publish(socket, Receipt::Unavailable)) {
                    return;
                }
                continue;
            }
        };
        if !worker_endpoint_open(socket) {
            checkout.reuse();
            return;
        }
        if checkout.session().checkout(owner).is_err() {
            checkout.reuse();
            owner.publish_without_checkout(|| publish(socket, Receipt::Unavailable));
            return;
        }
        let mut params = identity.parameters().to_vec();
        params.extend(values.iter().map(String::as_str));
        let completed = match checkout
            .session()
            .invoke(&op.sql, &params, io_deadline, owner)
        {
            Ok(receipt) => {
                let receipt = match receipt {
                    EffectReceipt::Confirmed(applied) => Receipt::Confirmed(applied),
                    EffectReceipt::Rejected => Receipt::Rejected,
                };
                owner.release(|| publish(socket, receipt))
            }
            Err(EffectFailure::BeforeSubmission) => {
                // Retire the TLS session, which may still buffer request bytes.
                let _ = owner.release(|| publish(socket, Receipt::Unavailable));
                Err(EffectFailure::BeforeSubmission)
            }
            Err(error) => Err(error),
        };
        if completed.is_ok() {
            checkout.reuse();
        } else {
            // Retire on any uncertain progress. The next request never replays it.
            owner.revoke();
            return;
        }
    }
}

struct EffectProcess {
    worker: WorkerProcess,
    authority: Arc<EffectAuthority>,
    shared: Arc<Shared>,
    activate: Option<Sender<()>>,
    shutdown: UnixStream,
    relay_done: Arc<AtomicBool>,
    waiting_for_relay: bool,
    terminal: Option<WorkerPoll>,
}

impl EffectProcess {
    fn revoke(&self) -> bool {
        let interrupted = self.authority.revoke();
        let _ = self.shutdown.shutdown(Shutdown::Both);
        self.shared.sessions.interrupt_waiters();
        interrupted
    }
}

impl JobProcess for EffectProcess {
    fn activate_effects(&mut self) {
        if let Some(activate) = self.activate.take() {
            let _ = activate.try_send(());
        }
    }

    fn confirm_effect_lease(&mut self, expires: Instant) {
        self.authority.renew(expires);
    }

    fn limit_effect_deadline(&mut self, deadline: Instant) {
        self.authority.limit_deadline(deadline);
        self.shared.sessions.interrupt_waiters();
    }

    fn reply_pending(&self) -> bool {
        self.worker.reply_pending()
            || (self.waiting_for_relay && self.relay_done.load(Ordering::Acquire))
    }

    fn poll(&mut self, now: Instant) -> Result<WorkerPoll, String> {
        let poll = match self.terminal.take() {
            Some(poll) => poll,
            None => self.worker.poll(now)?,
        };
        if matches!(poll, WorkerPoll::Running) {
            return Ok(poll);
        }
        let interrupted = self.revoke();
        let poll = if interrupted && matches!(poll, WorkerPoll::Exited(Ok(_))) {
            WorkerPoll::Exited(Err(UNKNOWN.into()))
        } else {
            poll
        };
        self.waiting_for_relay = !self.relay_done.load(Ordering::Acquire);
        if self.waiting_for_relay {
            if !matches!(poll, WorkerPoll::AwaitingReply) {
                self.terminal = Some(poll);
            }
            return Ok(WorkerPoll::AwaitingEffectRelay);
        }
        Ok(poll)
    }

    fn renew(&mut self, lease_deadline: u64) -> Result<(), String> {
        self.worker.renew(lease_deadline)
    }

    fn kill(&mut self) -> Result<(), String> {
        self.revoke();
        self.worker.kill()
    }
}

impl Drop for EffectProcess {
    fn drop(&mut self) {
        self.activate.take();
        self.revoke();
        // The supervisor retains the attempt until the relay exits. Drop
        // can run under runner.state and must never wait for relay I/O.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serve_without_session(socket: UnixStream, operations: BTreeMap<String, Operation>) {
        let identity = Identity::from_claim(
            "namespace",
            "job",
            "token",
            r#"{"id":"job","effect_key":"job/1","attempt":1}"#,
        )
        .unwrap();
        let shared = Shared {
            operations,
            sessions: SessionPool::new(vec![]),
        };
        let deadline = Instant::now() + IO_TIMEOUT;
        serve(
            Relay(socket),
            deadline,
            &identity,
            &EffectAuthority::new(deadline, deadline),
            &shared,
        );
    }

    #[test]
    fn hangup_is_visible_with_a_complete_orphan_request_buffered() {
        let (mut child, mut relay) = UnixStream::pair().unwrap();
        let request = Request::new("append", vec![], IO_TIMEOUT).unwrap();
        write_frame(&mut child, &request, MAX_FRAME).unwrap();
        assert!(worker_endpoint_open(&relay));
        drop(child);

        assert!(!worker_endpoint_open(&relay));
        let buffered: Request = read_frame(&mut relay, MAX_FRAME).unwrap();
        assert_eq!(buffered.name, request.name);
        assert!(!worker_endpoint_open(&relay));
    }

    #[test]
    fn relay_rejects_buffered_request_when_worker_closes_its_write_side() {
        let (mut child, relay) = UnixStream::pair().unwrap();
        let _supervisor = relay.try_clone().unwrap();
        let operations =
            Operation::parse(br#"[{"name":"append","routine":"app.append","args":[]}]"#).unwrap();
        let request = Request::new("append", vec![], IO_TIMEOUT).unwrap();
        write_frame(&mut child, &request, MAX_FRAME).unwrap();
        // Keep the reader open so any receipt would expose a checkout attempt.
        child.shutdown(Shutdown::Write).unwrap();
        child.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        serve_without_session(relay, operations);
        assert_eq!(child.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn exhausted_effect_checkout_observes_hangup_without_a_release() {
        let (child, relay) = UnixStream::pair().unwrap();
        let pool = SessionPool::new(vec![()]);
        let deadline = Instant::now() + IO_TIMEOUT;
        let owner = EffectAuthority::new(deadline, deadline);
        let busy = pool
            .checkout(deadline, Duration::ZERO, PEER_PROBE, || true)
            .unwrap();
        thread::scope(|scope| {
            let (probing, probed) = crossbeam_channel::bounded(1);
            let (done, completed) = crossbeam_channel::bounded(1);
            let pool = &pool;
            let owner = &owner;
            scope.spawn(move || {
                let result = pool.checkout(deadline, Duration::ZERO, PEER_PROBE, || {
                    let live = owner.live() && worker_endpoint_open(&relay);
                    let _ = probing.try_send(live);
                    live
                });
                done.send(matches!(result, Err(CheckoutError::Cancelled)))
                    .unwrap();
            });
            assert!(probed.recv_timeout(IO_TIMEOUT / 2).unwrap());
            drop(child);
            assert!(completed.recv_timeout(IO_TIMEOUT / 2).unwrap());
            assert!(!pool.exhausted_permanently());
        });
        busy.reuse();
    }

    #[test]
    fn relay_exit_preserves_queued_receipt_and_rejects_the_next_write() {
        let (mut child, relay) = UnixStream::pair().unwrap();
        let _supervisor = relay.try_clone().unwrap();
        let request = Request::new("append", vec![], IO_TIMEOUT).unwrap();
        write_frame(&mut child, &request, MAX_FRAME).unwrap();
        // Force a read error on the next frame, after the relay queues a receipt.
        child
            .write_all(&(MAX_FRAME as u32 + 1).to_le_bytes())
            .unwrap();
        serve_without_session(relay, BTreeMap::new());

        assert!(matches!(
            TimedWire::new(&child, WireDeadline::Shared(request.deadline)).read_receipt(),
            Ok(Receipt::Invalid)
        ));
        let mut endpoint = ChildEndpoint::Ready(child);
        assert_eq!(
            endpoint.request("append", vec![], IO_TIMEOUT),
            Err(UNAVAILABLE)
        );
        let reply = Ok(r#"{"action":"complete"}"#.into());
        assert_eq!(endpoint.finish(reply.clone()), reply);
    }

    #[test]
    fn missing_receipt_after_full_request_prevents_completion() {
        let (child, mut relay) = UnixStream::pair().unwrap();
        relay.shutdown(Shutdown::Write).unwrap();
        relay.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        let mut endpoint = ChildEndpoint::Ready(child);
        assert_eq!(endpoint.request("append", vec![], IO_TIMEOUT), Err(UNKNOWN));
        let sent: Request = read_frame(&mut relay, MAX_FRAME).unwrap();
        assert_eq!(sent.name, "append");
        assert_eq!(endpoint.request("append", vec![], IO_TIMEOUT), Err(UNKNOWN));
        assert_eq!(
            endpoint.finish(Ok(r#"{"action":"complete"}"#.into())),
            Err(UNKNOWN.into())
        );
    }

    #[test]
    fn known_validation_and_unsent_errors_preserve_handler_reply() {
        let reply = Ok(r#"{"action":"complete"}"#.to_string());
        let (child, mut relay) = UnixStream::pair().unwrap();
        publish(&relay, Receipt::Invalid).unwrap();
        let mut endpoint = ChildEndpoint::Ready(child);
        assert_eq!(endpoint.request("append", vec![], IO_TIMEOUT), Err(INVALID));

        relay.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        let _: Request = read_frame(&mut relay, MAX_FRAME).unwrap();
        assert_eq!(
            endpoint.request("append", vec![], Duration::ZERO),
            Err(UNAVAILABLE)
        );
        assert_eq!(relay.read(&mut [0]).unwrap(), 0);
        assert_eq!(endpoint.finish(reply.clone()), reply);
    }

    #[test]
    fn delayed_request_uses_the_workers_original_deadline() {
        let (mut child, mut relay) = UnixStream::pair().unwrap();
        let request = Request::new("append", vec![], CHECKOUT_REPLY_RESERVE).unwrap();
        write_frame(&mut child, &request, MAX_FRAME).unwrap();
        let decoded: Request = read_frame(&mut relay, MAX_FRAME).unwrap();
        assert_eq!(decoded.deadline.0, request.deadline.0);

        let handler_deadline = Instant::now() + IO_TIMEOUT;
        let owner = EffectAuthority::new(handler_deadline, handler_deadline);
        let pool = SessionPool::new(vec![()]);
        let busy = checkout_for_call(&pool, &relay, &owner, handler_deadline).unwrap();
        let io_deadline = decoded.deadline.clamp(handler_deadline).unwrap();
        assert!(io_deadline <= Instant::now() + CHECKOUT_REPLY_RESERVE);
        assert!(matches!(
            checkout_for_call(&pool, &relay, &owner, io_deadline),
            Err(CheckoutError::Unavailable)
        ));
        assert!(owner.live());
        assert!(worker_endpoint_open(&relay));
        busy.reuse();

        // The parent can impose a tighter deadline even on a far-future frame.
        let parent_deadline = Instant::now();
        assert_eq!(
            MonotonicDeadline(u64::MAX).clamp(parent_deadline).unwrap(),
            parent_deadline
        );
    }

    #[test]
    fn live_relay_without_a_session_returns_known_unavailable() {
        let (child, relay) = UnixStream::pair().unwrap();
        let ops =
            Operation::parse(br#"[{"name":"append","routine":"app.append","args":[]}]"#).unwrap();
        thread::scope(|scope| {
            scope.spawn(move || serve_without_session(relay, ops));
            let mut endpoint = ChildEndpoint::Ready(child);
            assert_eq!(
                endpoint.request("append", vec![], IO_TIMEOUT),
                Err(UNAVAILABLE)
            );
            let reply = Ok(r#"{"action":"complete"}"#.into());
            assert_eq!(endpoint.finish(reply.clone()), reply);
        });
    }

    #[test]
    fn relay_completion_wakes_after_the_child_reply_has_been_consumed() {
        for (complete_before_poll, wake_already_queued) in
            [(true, false), (false, false), (false, true)]
        {
            let (worker, publish_reply, reply) = WorkerProcess::reaped_with_held_reply();
            let (wake, wakes) = crossbeam_channel::bounded(1);
            let (shutdown, _peer) = UnixStream::pair().unwrap();
            let done = Arc::new(AtomicBool::new(false));
            let mut completion = Some(RelayCompletion {
                done: done.clone(),
                wake: wake.clone(),
            });
            let deadline = Instant::now() + IO_TIMEOUT;
            let mut process = EffectProcess {
                worker,
                authority: Arc::new(EffectAuthority::new(deadline, deadline)),
                shared: Effects::retired_fixture().shared,
                activate: None,
                shutdown,
                relay_done: done,
                waiting_for_relay: false,
                terminal: None,
            };
            publish_reply.send(reply).unwrap();
            assert!(process.reply_pending());
            if complete_before_poll {
                drop(completion.take());
            } else {
                assert!(matches!(
                    process.poll(Instant::now()),
                    Ok(WorkerPoll::AwaitingEffectRelay)
                ));
                assert!(!process.reply_pending());
                if wake_already_queued {
                    wake.try_send(()).unwrap();
                }
                drop(completion.take());
                assert!(process.reply_pending());
            }
            assert_eq!(wakes.try_recv(), Ok(()));
            assert!(
                matches!(process.poll(Instant::now()), Ok(WorkerPoll::Exited(Ok(reply))) if reply == "done")
            );
            assert!(!process.reply_pending());
        }
    }

    #[test]
    fn declared_operation_binds_types_and_never_compiles_value_text() {
        let registry =
            br#"[{"name":"append","routine":"app.append","args":["text","int","bool"]}]"#;
        let operations = Operation::parse(registry).unwrap();
        let op = &operations["append"];
        assert_eq!(
            op.sql,
            r#"SELECT "app"."append"($1::pg_catalog.text,$2::pg_catalog.uuid,$3::pg_catalog.int8,$4::pg_catalog.int4,$5::pg_catalog.uuid,$6::pg_catalog.text,$7::pg_catalog.int8,$8::pg_catalog.bool) AS receipt"#,
        );
        let values = serde_json::json!(["'; SELECT pg_sleep(10);", 12, true]);
        assert_eq!(
            op.bind(values.as_array().unwrap()).unwrap(),
            ["'; SELECT pg_sleep(10);", "12", "true"]
        );
        let wrong = serde_json::json!(["text", "12", true]);
        assert!(op.bind(wrong.as_array().unwrap()).is_none());
        assert!(
            Operation::parse(br#"[{"name":"x","routine":"pg_catalog.get","args":[]}]"#).is_err()
        );
        assert!(
            Operation::parse(br#"[{"name":"x","routine":"app.x; SELECT 1","args":[]}]"#).is_err()
        );
    }

    #[test]
    fn identity_uses_confirmed_job_and_token() {
        let context = r#"{"id":"job","effect_key":"job/7","attempt":2,"args":{"token":"fake"}}"#;
        let identity = Identity::from_claim("namespace", "job", "real", context).unwrap();
        assert_eq!(
            identity.parameters(),
            ["namespace", "job", "7", "2", "real"]
        );
        assert!(Identity::from_claim("namespace", "other", "real", context).is_err());
    }
}
