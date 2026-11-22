use std::collections::BTreeMap;
use std::ffi::{c_char, CStr};
use std::fs::{File, Metadata};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::Mutex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::db::pg::MAX_DB_VALUE_BYTES;
use crate::string::{mesh_str, MeshString};

#[cfg(any(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    all(target_os = "linux", test),
))]
#[path = "worker_effect_endpoint_linux.rs"]
mod effect_endpoint;

const WORKER_ARG: &str = "--mesh-internal-job-attempt-v1";
#[cfg(target_os = "linux")]
const DISABLE_THP_ENV: &str = "MESH_DIAGNOSTIC_JOB_DISABLE_THP";
const MAX_REQUEST_HEADER: usize = 1024 * 1024;
const MAX_OUTCOME: usize = 64 * 1024;
const MAX_REPLY_FRAME: usize = MAX_OUTCOME * 6 + 64;
const REAPED_REPLY_GRACE: Duration = Duration::from_secs(1);
const TIMEOUT: i32 = 171;
const LEASE_LOST: i32 = 172;
const PARENT_GONE: i32 = 173;
const BOOTSTRAP_ERROR: i32 = 174;

type Handler = unsafe extern "C-unwind" fn(*const MeshString) -> *const MeshString;
type Reply = Result<String, String>;

static HANDLERS: LazyLock<Mutex<BTreeMap<String, Handler>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static EXECUTABLE: OnceLock<Result<Executable, String>> = OnceLock::new();

struct Executable {
    path: PathBuf,
    image: Metadata,
}

impl Executable {
    fn locate() -> Result<Self, String> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let path = PathBuf::from("/proc/self/exe");
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let path = std::env::current_exe().map_err(|error| error.to_string())?;
        let image = path.metadata().map_err(|error| error.to_string())?;
        Ok(Self { path, image })
    }

    fn check(&self) -> Result<&Path, String> {
        let current = self.path.metadata().map_err(|error| error.to_string())?;
        if !same_image(&self.image, &current) {
            return Err("Mesh executable changed while its job runner was active".into());
        }
        Ok(&self.path)
    }
}

#[cfg(unix)]
fn same_image(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_image(left: &Metadata, right: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
        && left.file_size() == right.file_size()
}

#[no_mangle]
pub unsafe extern "C" fn mesh_jobs_register_handler(name: *const u8, len: u64, handler: *const u8) {
    let bytes = unsafe { std::slice::from_raw_parts(name, len as usize) };
    if let Ok(name) = std::str::from_utf8(bytes) {
        if !handler.is_null() {
            let handler = unsafe { std::mem::transmute::<*const u8, Handler>(handler) };
            HANDLERS.lock().insert(name.to_owned(), handler);
        }
    }
}

pub(super) fn handler_name(fn_ptr: *const u8, env_ptr: *const u8) -> Result<String, String> {
    if fn_ptr.is_null() || !env_ptr.is_null() {
        return Err("runner handler must be a named fn(String) -> String".into());
    }
    let name = HANDLERS
        .lock()
        .iter()
        .find(|(_, handler)| **handler as usize == fn_ptr as usize)
        .map(|(name, _)| name.clone())
        .ok_or("Jobs.start requires a handler in a compiled Mesh executable")?;
    executable()?;
    Ok(name)
}

fn executable() -> Result<&'static Path, String> {
    EXECUTABLE
        .get_or_init(Executable::locate)
        .as_ref()
        .map_err(String::clone)?
        .check()
}

pub(super) fn check_executable() -> Result<(), String> {
    executable().map(|_| ())
}

#[derive(Deserialize, Serialize)]
struct Request {
    handler: String,
    deadline: u64,
    lease_deadline: u64,
}

pub(super) enum WorkerPoll {
    Running,
    AwaitingReply,
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    AwaitingEffectRelay,
    Exited(Reply),
    TimedOut,
    LeaseExpired,
}

pub(super) struct WorkerProcess {
    child: Child,
    renewals: Sender<()>,
    lease_deadline: Arc<AtomicU64>,
    result: Receiver<Reply>,
    reply_deadline: Option<Instant>,
}

enum WorkerInput {
    Pipe,
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    Effects {
        peer: UnixStream,
        input: OwnedFd,
        endpoint: OwnedFd,
    },
}

impl WorkerProcess {
    pub(super) fn start(
        handler: &str,
        context: String,
        deadline: Instant,
        lease_deadline: u64,
        reply_wake: Sender<()>,
    ) -> Result<Self, String> {
        Self::launch(
            handler,
            context,
            deadline,
            lease_deadline,
            reply_wake,
            WorkerInput::Pipe,
        )
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    pub(super) fn start_effects(
        handler: &str,
        context: String,
        deadline: Instant,
        lease_deadline: u64,
        reply_wake: Sender<()>,
        endpoint: UnixStream,
    ) -> Result<Self, String> {
        let (peer, input) = UnixStream::pair().map_err(|error| error.to_string())?;
        Self::launch(
            handler,
            context,
            deadline,
            lease_deadline,
            reply_wake,
            WorkerInput::Effects {
                peer,
                input: input.into(),
                endpoint: endpoint.into(),
            },
        )
    }

    fn launch(
        handler: &str,
        context: String,
        deadline: Instant,
        lease_deadline: u64,
        reply_wake: Sender<()>,
        control: WorkerInput,
    ) -> Result<Self, String> {
        let request = Request {
            handler: handler.to_owned(),
            deadline: host_deadline(deadline)?,
            lease_deadline,
        };
        let mut command = Command::new(executable()?);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;

            command.process_group(0);
        }
        #[cfg(unix)]
        let (input, endpoint) = match control {
            WorkerInput::Pipe => {
                let (parent, worker) = UnixStream::pair().map_err(|error| {
                    format!("could not open job handler control socket: {error}")
                })?;
                command
                    .arg(WORKER_ARG)
                    .stdin(Stdio::from(OwnedFd::from(worker)));
                (parent, None::<OwnedFd>)
            }
            #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
            WorkerInput::Effects {
                peer,
                input,
                endpoint,
            } => {
                configure_effect_worker_env(
                    &mut command,
                    std::env::var_os(DISABLE_THP_ENV).as_deref(),
                );
                command
                    .arg(effect_endpoint::WORKER_ARG)
                    .stdin(Stdio::from(input));
                (peer, Some(endpoint))
            }
        };
        #[cfg(not(unix))]
        match control {
            WorkerInput::Pipe => {
                command.arg(WORKER_ARG).stdin(Stdio::piped());
            }
        }
        let child = command
            .env("MESH_SCHEDULER_MIN_WORKERS", "1")
            .env("MESH_SCHEDULER_MAX_WORKERS", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("could not start job handler process: {error}"))?;
        // Close the parent's child-side stdin before the control writer runs.
        drop(command);
        let (reply, result) = crossbeam_channel::bounded(1);
        let (renewals, renewal_requests) = crossbeam_channel::bounded(1);
        let lease_deadline = Arc::new(AtomicU64::new(request.lease_deadline));
        let mut worker = Self {
            child,
            renewals,
            lease_deadline,
            result,
            reply_deadline: None,
        };
        let output = worker.child.stdout.take().unwrap();
        std::thread::Builder::new()
            .name("mesh-job-output".into())
            .spawn(move || forward_reply(output, reply, reply_wake))
            .map_err(|error| error.to_string())?;

        #[cfg(not(unix))]
        let input = worker.child.stdin.take().unwrap();
        let latest = Arc::clone(&worker.lease_deadline);
        std::thread::Builder::new()
            .name("mesh-job-control".into())
            .spawn(move || {
                #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
                if let Some(endpoint) = endpoint {
                    let sent = effect_endpoint::send(&input, &endpoint);
                    drop(endpoint);
                    if sent.is_err() {
                        return;
                    }
                }
                #[cfg(all(
                    unix,
                    not(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))
                ))]
                let _ = endpoint;
                write_control(input, request, context, renewal_requests, latest);
            })
            .map_err(|error| error.to_string())?;
        Ok(worker)
    }

    pub(super) fn renew(&mut self, lease_deadline: u64) -> Result<(), String> {
        self.lease_deadline.store(lease_deadline, Ordering::Release);
        // A full notification already tells the writer to send the latest deadline.
        let _ = self.renewals.try_send(());
        Ok(())
    }

    pub(super) fn kill(&mut self) -> Result<(), String> {
        self.child.kill().map_err(|error| error.to_string())
    }

    pub(super) fn reply_pending(&self) -> bool {
        !self.result.is_empty()
    }

    pub(super) fn poll(&mut self, now: Instant) -> Result<WorkerPoll, String> {
        let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? else {
            return Ok(WorkerPoll::Running);
        };
        let reason = match status.code() {
            Some(TIMEOUT) => return Ok(WorkerPoll::TimedOut),
            Some(LEASE_LOST) => return Ok(WorkerPoll::LeaseExpired),
            Some(PARENT_GONE) => "job runner disconnected".into(),
            Some(BOOTSTRAP_ERROR) => "job handler could not enter its worker runtime".into(),
            Some(0) => return Ok(self.poll_reply(now)),
            _ => format!("job handler process exited: {status}"),
        };
        Ok(WorkerPoll::Exited(Err(reason)))
    }

    fn poll_reply(&mut self, now: Instant) -> WorkerPoll {
        match self.result.try_recv() {
            Ok(outcome) => return WorkerPoll::Exited(outcome),
            Err(TryRecvError::Empty) => {
                // An inherited writer can keep the pipe open after the child exits.
                let deadline = self.reply_deadline.get_or_insert(now + REAPED_REPLY_GRACE);
                if now < *deadline {
                    return WorkerPoll::AwaitingReply;
                }
            }
            Err(TryRecvError::Disconnected) => {}
        }
        WorkerPoll::Exited(Err("job handler returned no result".into()))
    }
}

fn write_control(
    mut input: impl Write,
    request: Request,
    context: String,
    renewal_requests: Receiver<()>,
    latest: Arc<AtomicU64>,
) {
    if write_frame(&mut input, &request, MAX_REQUEST_HEADER)
        .and_then(|()| write_bytes(&mut input, context.as_bytes(), MAX_DB_VALUE_BYTES))
        .is_err()
    {
        return;
    }
    drop(context);
    while renewal_requests.recv().is_ok() {
        let lease = latest.load(Ordering::Acquire);
        if input.write_all(&lease.to_le_bytes()).is_err() {
            break;
        }
    }
}

fn forward_reply(mut output: impl Read, reply: Sender<Reply>, reply_wake: Sender<()>) {
    let result = read_frame(&mut output, MAX_REPLY_FRAME)
        .unwrap_or_else(|error| Err(format!("invalid job handler reply: {error}")));
    if reply.send(result).is_ok() {
        let _ = reply_wake.try_send(());
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        // A live child retains authority until its owner stops or reaps it.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
impl WorkerProcess {
    fn fixture(mut child: Child) -> (Self, Sender<Reply>, std::process::ChildStdout) {
        let output = child.stdout.take().unwrap();
        let (publish, result) = crossbeam_channel::bounded(1);
        let worker = Self {
            child,
            renewals: crossbeam_channel::bounded(1).0,
            lease_deadline: Arc::new(AtomicU64::new(0)),
            result,
            reply_deadline: None,
        };
        (worker, publish, output)
    }

    pub(super) fn reaped_with_held_reply() -> (Self, Sender<Reply>, Reply) {
        let mut child = Command::new("/bin/sh")
            .args(["-c", r#"printf '\015\000\000\000{"Ok":"done"}'"#])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        let (worker, publish, mut output) = Self::fixture(child);
        let reply = read_frame(&mut output, MAX_REPLY_FRAME).unwrap();
        (worker, publish, reply)
    }
}

#[cfg(all(test, unix))]
#[test]
fn worker_reply_wake_does_not_bypass_reap() {
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            r#"printf '\015\000\000\000{"Ok":"done"}'; read -r line"#,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let (mut worker, publish, output) = WorkerProcess::fixture(child);
    let (reply_wake, reply_wakes) = crossbeam_channel::bounded(1);

    assert!(!worker.reply_pending());
    forward_reply(output, publish, reply_wake);
    assert_eq!(reply_wakes.try_recv(), Ok(()));
    assert!(matches!(
        worker.poll(Instant::now()),
        Ok(WorkerPoll::Running)
    ));
    assert!(worker.reply_pending());
    input.write_all(b"exit\n").unwrap();
    assert!(worker.child.wait().unwrap().success());
    assert!(matches!(
        worker.poll(Instant::now()),
        Ok(WorkerPoll::Exited(Ok(output))) if output == "done"
    ));
}

#[cfg(all(test, unix))]
#[test]
fn reaped_worker_bounds_wait_for_an_inherited_pipe() {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exec 3<&0; /bin/cat <&3 & exit 0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    assert!(child.wait().unwrap().success());
    let (mut worker, publish, mut output) = WorkerProcess::fixture(child);
    let reader = std::thread::spawn(move || {
        let reply = read_frame::<Reply>(&mut output, MAX_REPLY_FRAME)
            .unwrap_or_else(|error| Err(error.to_string()));
        let _ = publish.send(reply);
    });
    let now = Instant::now();
    assert!(matches!(worker.poll(now), Ok(WorkerPoll::AwaitingReply)));
    assert!(matches!(
        worker.poll(now + REAPED_REPLY_GRACE),
        Ok(WorkerPoll::Exited(Err(reason))) if reason == "job handler returned no result"
    ));
    drop(input);
    reader.join().unwrap();
}

fn worker_uses_effect_stdin(arg: &[u8]) -> Option<bool> {
    if arg == WORKER_ARG.as_bytes() {
        return Some(false);
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    if arg == effect_endpoint::WORKER_ARG.as_bytes() {
        return Some(true);
    }
    None
}

#[cfg(target_os = "linux")]
fn thp_disable_requested(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("1"))
}

#[cfg(all(
    target_os = "linux",
    any(all(target_arch = "aarch64", target_env = "gnu"), test)
))]
fn configure_effect_worker_env(command: &mut Command, thp: Option<&std::ffi::OsStr>) {
    command.env_clear();
    if thp_disable_requested(thp) {
        command.env(DISABLE_THP_ENV, "1");
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn apply_worker_startup_policy() {
    static STARTUP_POLICY: std::sync::Once = std::sync::Once::new();
    STARTUP_POLICY.call_once(|| {
        if thp_disable_requested(std::env::var_os(DISABLE_THP_ENV).as_deref())
            && matches!(is_current_worker(), Ok(true))
        {
            disable_worker_thp();
        }
    });
}

#[cfg(target_os = "linux")]
fn disable_worker_thp() {
    static WORKER_THP_POLICY: std::sync::Once = std::sync::Once::new();
    WORKER_THP_POLICY.call_once(|| {
        let zero: libc::c_ulong = 0;
        let one: libc::c_ulong = 1;
        if unsafe { libc::prctl(libc::PR_SET_THP_DISABLE, one, zero, zero, zero) } != 0
            || unsafe { libc::prctl(libc::PR_GET_THP_DISABLE, zero, zero, zero, zero) } != 1
        {
            // The worker watchdogs are not armed yet. Exit without waiting on stderr.
            exit_worker(BOOTSTRAP_ERROR);
        }
    });
}

#[cfg(target_os = "linux")]
fn is_current_worker() -> io::Result<bool> {
    use std::os::unix::ffi::OsStrExt;

    let mut args = std::env::args_os();
    if args.len() != 0 {
        return Ok(args.len() == 2
            && args
                .nth(1)
                .is_some_and(|arg| worker_uses_effect_stdin(arg.as_bytes()).is_some()));
    }

    // C main on musl can leave Rust's argv empty. Read through Linux's
    // maximum argv[0] (32 pages, at most 2 MiB), the longest accepted marker,
    // its NUL, and at least one byte of any extra argument.
    #[cfg(all(target_arch = "aarch64", target_env = "gnu"))]
    const MARKER_LEN: usize = effect_endpoint::WORKER_ARG.len();
    #[cfg(not(all(target_arch = "aarch64", target_env = "gnu")))]
    const MARKER_LEN: usize = WORKER_ARG.len();
    const LIMIT: u64 = 2 * 1024 * 1024 + MARKER_LEN as u64 + 2;
    let mut command_line = Vec::new();
    File::open("/proc/self/cmdline")?
        .take(LIMIT)
        .read_to_end(&mut command_line)?;
    Ok(proc_cmdline_selects_worker(&command_line))
}

#[cfg(target_os = "linux")]
fn proc_cmdline_selects_worker(command_line: &[u8]) -> bool {
    let Some(command_line) = command_line.strip_suffix(b"\0") else {
        return false;
    };
    let mut args = command_line.splitn(3, |byte| *byte == 0);
    args.nth(1)
        .is_some_and(|arg| worker_uses_effect_stdin(arg).is_some())
        && args.next().is_none()
}

#[cfg(all(test, target_os = "linux"))]
mod thp_tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn worker_selectors_require_exact_argument_and_count() {
        const EFFECT_ARG: &str = "--mesh-internal-job-attempt-effect-v1";
        let effect_supported = cfg!(all(target_arch = "aarch64", target_env = "gnu"));
        assert_eq!(worker_uses_effect_stdin(WORKER_ARG.as_bytes()), Some(false));
        assert_eq!(
            worker_uses_effect_stdin(EFFECT_ARG.as_bytes()),
            effect_supported.then_some(true)
        );
        assert!(!proc_cmdline_selects_worker(b""));
        assert!(!proc_cmdline_selects_worker(b"mesh\0"));
        for (marker, supported) in [(WORKER_ARG, true), (EFFECT_ARG, effect_supported)] {
            let exact = format!("mesh\0{marker}\0");
            assert_eq!(proc_cmdline_selects_worker(exact.as_bytes()), supported);
            for invalid in [
                format!("{marker}\0"),
                format!("mesh\0{marker}"),
                format!("mesh\0{marker}0\0"),
                format!("mesh\0{marker}\0\0"),
                format!("mesh\0{marker}\0extra\0"),
                format!("mesh\0extra\0{marker}\0"),
            ] {
                assert!(
                    !proc_cmdline_selects_worker(invalid.as_bytes()),
                    "{invalid:?}"
                );
            }
        }
    }

    #[test]
    fn effect_worker_env_forwards_only_exact_thp_opt_in() {
        let cases: [(Option<&[u8]>, bool); 8] = [
            (None, false),
            (Some(b""), false),
            (Some(b"0"), false),
            (Some(b"01"), false),
            (Some(b" 1"), false),
            (Some(b"1 "), false),
            (Some(b"1\xff"), false),
            (Some(b"1"), true),
        ];
        for (raw, enabled) in cases {
            let mut command = Command::new("unused");
            command.env("MESH_EFFECT_APP_URL", "test-sentinel");
            configure_effect_worker_env(&mut command, raw.map(OsStr::from_bytes));
            let expected: Vec<_> = enabled
                .then_some((OsStr::new(DISABLE_THP_ENV), Some(OsStr::new("1"))))
                .into_iter()
                .collect();
            assert_eq!(command.get_envs().collect::<Vec<_>>(), expected, "{raw:?}");
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mesh_jobs_enter_worker(argc: i32, argv: *const *const c_char) {
    if argc == 2 {
        let arg = unsafe { CStr::from_ptr(*argv.add(1)) }.to_bytes();
        if let Some(effect_stdin) = worker_uses_effect_stdin(arg) {
            #[cfg(target_os = "linux")]
            if thp_disable_requested(std::env::var_os(DISABLE_THP_ENV).as_deref()) {
                // C argv identifies the worker even when early procfs inspection failed.
                disable_worker_thp();
            }
            enter_worker(effect_stdin);
        }
    }
    if !HANDLERS.lock().is_empty() {
        EXECUTABLE.get_or_init(Executable::locate);
    }
}

pub(crate) fn is_worker_process() -> bool {
    let mut args = std::env::args_os();
    args.len() == 2
        && args
            .nth(1)
            .is_some_and(|arg| worker_uses_effect_stdin(arg.as_encoded_bytes()).is_some())
}

fn enter_worker(effect_stdin: bool) -> ! {
    match run_worker(effect_stdin) {
        Ok(()) => exit_worker(0),
        Err(error) => {
            eprintln!("Mesh job handler initialization failed: {error}");
            exit_worker(BOOTSTRAP_ERROR);
        }
    }
}

fn run_worker(_effect_stdin: bool) -> Result<(), String> {
    crate::panic::ignore_sigpipe();
    crate::process_signal::mesh_process_install_shutdown_signals();
    let mut result = redirect_stdout().map_err(|error| error.to_string())?;
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    if _effect_stdin {
        let socket = effect_endpoint::receive_stdin().map_err(|error| error.to_string())?;
        super::effects::enter_worker(socket);
    }
    let (request, context, lease) = read_control_and_watch().map_err(|error| error.to_string())?;
    let handler = HANDLERS
        .lock()
        .get(&request.handler)
        .copied()
        .ok_or("job handler is missing from this executable")?;

    let deadline = request.deadline;
    remaining(deadline, u64::MAX);
    let reply = invoke(context, |input| {
        // Input preparation and the lease lock can both take time.
        remaining(deadline, *lease.lock());
        unsafe { handler(input) }
    });
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    let reply = if _effect_stdin {
        super::effects::finish_worker(reply)
    } else {
        reply
    };
    remaining(deadline, u64::MAX);
    write_frame(&mut result, &reply, MAX_REPLY_FRAME).map_err(|error| error.to_string())?;
    Ok(())
}

fn read_request(control: &mut impl Read) -> io::Result<(Request, String)> {
    let request = read_frame(control, MAX_REQUEST_HEADER)?;
    let context = String::from_utf8(read_bytes(control, MAX_DB_VALUE_BYTES)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((request, context))
}

#[cfg(unix)]
struct ParentControl<'a>(BorrowedFd<'a>);

#[cfg(unix)]
impl ParentControl<'_> {
    fn receive(&self, bytes: &mut [u8], flags: libc::c_int) -> io::Result<usize> {
        let count = unsafe {
            libc::recv(
                self.0.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                flags,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(count as usize)
    }
}

#[cfg(unix)]
impl Read for ParentControl<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.receive(bytes, 0)
    }
}

type LoadedRequest = (Request, String, Arc<Mutex<u64>>);

#[cfg(unix)]
fn read_control_and_watch() -> io::Result<LoadedRequest> {
    let (loaded, input) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("mesh-job-watchdog".into())
        .spawn(move || {
            let stdin = io::stdin();
            // Reserve stdin for control. Read directly so buffered stdin cannot
            // hide any renewal bytes from the descriptor readiness check.
            let _stdin = stdin.lock();
            let mut control = ParentControl(stdin.as_fd());
            match read_request(&mut control) {
                Ok((request, context)) => {
                    let deadline = request.deadline;
                    let lease = Arc::new(Mutex::new(request.lease_deadline));
                    if loaded
                        .send(Ok((request, context, Arc::clone(&lease))))
                        .is_ok()
                    {
                        watch_parent(control, deadline, &lease);
                    }
                }
                Err(error) => {
                    let _ = loaded.send(Err(error));
                }
            }
        })?;
    input.recv().map_err(io::Error::other)?
}

#[cfg(unix)]
fn watch_parent(control: ParentControl<'_>, deadline: u64, lease: &Mutex<u64>) {
    let mut parent = libc::pollfd {
        fd: control.0.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let mut frame = [0; 8];
    let mut filled = 0;
    loop {
        // A timed poll may exclude time spent suspended. Recheck the host
        // clock at the same cadence as the portable watchdog.
        let wait = remaining(deadline, *lease.lock()).as_millis().min(20) as libc::c_int;
        let ready = unsafe { libc::poll(&mut parent, 1, wait) };
        if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            exit_worker(BOOTSTRAP_ERROR);
        }
        if ready <= 0 {
            continue;
        }
        // A native read can consume readiness. Keep blocking mode in the
        // child and make only this receive nonblocking.
        match control.receive(&mut frame[filled..], libc::MSG_DONTWAIT) {
            Ok(0) => exit_worker(PARENT_GONE),
            Ok(count) => filled += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => exit_worker(PARENT_GONE),
        }
        if filled == frame.len() {
            let mut accepted = lease.lock();
            remaining(deadline, *accepted);
            *accepted = (*accepted).max(u64::from_le_bytes(frame));
            filled = 0;
        }
    }
}

#[cfg(not(unix))]
fn read_control_and_watch() -> io::Result<LoadedRequest> {
    let mut control = io::stdin();
    let (request, context) = read_request(&mut control)?;
    let (renew, updates) = crossbeam_channel::bounded(1);
    let (claim_stdin, stdin_claimed) = crossbeam_channel::bounded(0);
    std::thread::Builder::new()
        .name("mesh-job-parent".into())
        .spawn(move || {
            // The private parent stream uses stdin. Keep it locked across
            // renewal frames so handler reads cannot consume control bytes.
            let mut control = control.lock();
            if claim_stdin.send(()).is_err() {
                return;
            }
            let mut frame = [0; 8];
            while control.read_exact(&mut frame).is_ok() {
                if renew.send(u64::from_le_bytes(frame)).is_err() {
                    return;
                }
            }
            exit_worker(PARENT_GONE);
        })?;
    let deadline = request.deadline;
    let lease = Arc::new(Mutex::new(request.lease_deadline));
    let watchdog_lease = Arc::clone(&lease);
    std::thread::Builder::new()
        .name("mesh-job-deadline".into())
        .spawn(move || enforce_deadlines(deadline, &watchdog_lease, updates))?;
    stdin_claimed.recv().map_err(io::Error::other)?;
    Ok((request, context, lease))
}

pub(super) fn invoke(
    context: String,
    handler: impl FnOnce(*const MeshString) -> *const MeshString,
) -> Reply {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let input = mesh_str(&context);
        drop(context);
        let output = unsafe { handler(input).as_ref() }.ok_or("job handler returned no outcome")?;
        let bytes = unsafe { output.as_bytes() };
        if bytes.len() > MAX_OUTCOME {
            return Err("job handler outcome exceeds 64 KiB");
        }
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "job handler returned invalid UTF-8")
    }));
    match outcome {
        Ok(value) => value.map_err(str::to_owned),
        Err(panic) => Err(crate::panic::MeshPanic::of(panic.as_ref())
            .map_or("job handler panicked", crate::panic::MeshPanic::reason)
            .chars()
            .take(1024)
            .collect()),
    }
}

#[cfg(not(unix))]
fn enforce_deadlines(deadline: u64, lease: &Mutex<u64>, updates: Receiver<u64>) {
    loop {
        let wait = remaining(deadline, *lease.lock()).min(Duration::from_millis(20));
        match updates.recv_timeout(wait) {
            Ok(next) => {
                // A delayed renewal may not revive a worker whose lease already ended.
                let mut accepted = lease.lock();
                remaining(deadline, *accepted);
                *accepted = (*accepted).max(next);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => exit_worker(PARENT_GONE),
        }
    }
}

fn remaining(deadline: u64, lease: u64) -> Duration {
    let now = boot_millis().unwrap_or_else(|_| exit_worker(BOOTSTRAP_ERROR));
    if now >= deadline.min(lease) {
        exit_worker(if deadline <= lease {
            TIMEOUT
        } else {
            LEASE_LOST
        });
    }
    Duration::from_millis(deadline.min(lease) - now)
}

#[cfg(unix)]
fn exit_worker(code: i32) -> ! {
    // Native code may be holding a process-global lock needed by normal exit.
    unsafe { libc::_exit(code) }
}

#[cfg(windows)]
fn exit_worker(code: i32) -> ! {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn TerminateProcess(process: *mut c_void, code: u32) -> i32;
    }
    unsafe { TerminateProcess(GetCurrentProcess(), code as u32) };
    std::process::abort();
}

pub(super) fn host_deadline(deadline: Instant) -> Result<u64, String> {
    let now = boot_millis().map_err(|error| error.to_string())?;
    Ok(now.saturating_add(
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64,
    ))
}

#[cfg(unix)]
fn boot_millis() -> io::Result<u64> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let clock = libc::CLOCK_BOOTTIME;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let clock = libc::CLOCK_MONOTONIC;
    let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(clock, value.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let value = unsafe { value.assume_init() };
    Ok(value.tv_sec as u64 * 1000 + value.tv_nsec as u64 / 1_000_000)
}

#[cfg(windows)]
fn boot_millis() -> io::Result<u64> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount64() -> u64;
    }
    Ok(unsafe { GetTickCount64() })
}

pub(super) fn read_frame<T: DeserializeOwned>(reader: &mut impl Read, max: usize) -> io::Result<T> {
    serde_json::from_slice(&read_bytes(reader, max)?).map_err(io::Error::other)
}

fn read_bytes(reader: &mut impl Read, max: usize) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header) as usize;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "job IPC frame exceeds its limit",
        ));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn write_frame<T: Serialize>(
    writer: &mut impl Write,
    value: &T,
    max: usize,
) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_bytes(writer, &payload, max)
}

fn write_bytes(writer: &mut impl Write, payload: &[u8], max: usize) -> io::Result<()> {
    if payload.len() > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "job IPC frame exceeds its limit",
        ));
    }
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

#[cfg(unix)]
fn redirect_stdout() -> io::Result<File> {
    use std::os::fd::FromRawFd;

    let fd = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(result)
}

#[cfg(windows)]
fn redirect_stdout() -> io::Result<File> {
    use std::ffi::c_void;
    use std::os::windows::io::FromRawHandle;

    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn GetStdHandle(id: u32) -> Handle;
        fn SetStdHandle(id: u32, handle: Handle) -> i32;
        fn DuplicateHandle(
            src: Handle,
            handle: Handle,
            dst: Handle,
            output: *mut Handle,
            access: u32,
            inherit: i32,
            options: u32,
        ) -> i32;
    }
    const STDOUT: u32 = -11i32 as u32;
    const STDERR: u32 = -12i32 as u32;
    let mut output = std::ptr::null_mut();
    let process = unsafe { GetCurrentProcess() };
    if unsafe { DuplicateHandle(process, GetStdHandle(STDOUT), process, &mut output, 0, 0, 2) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { File::from_raw_handle(output) };
    if unsafe { SetStdHandle(STDOUT, GetStdHandle(STDERR)) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(result)
}
