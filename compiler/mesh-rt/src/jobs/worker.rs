use std::collections::BTreeMap;
use std::ffi::{c_char, CStr};
use std::fs::{File, Metadata};
use std::io::{self, Read, Write};
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

const WORKER_ARG: &str = "--mesh-internal-job-attempt-v1";
const MAX_REQUEST_HEADER: usize = 1024 * 1024;
const MAX_OUTCOME: usize = 64 * 1024;
const MAX_REPLY_FRAME: usize = MAX_OUTCOME * 6 + 64;
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

pub(super) struct WorkerProcess {
    child: Child,
    renewals: Sender<()>,
    lease_deadline: Arc<AtomicU64>,
    result: Receiver<Reply>,
}

impl WorkerProcess {
    pub(super) fn start(
        handler: &str,
        context: String,
        deadline: Instant,
        lease_deadline: u64,
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
        let child = command
            .arg(WORKER_ARG)
            .env("MESH_SCHEDULER_MIN_WORKERS", "1")
            .env("MESH_SCHEDULER_MAX_WORKERS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("could not start job handler process: {error}"))?;
        let (reply, result) = crossbeam_channel::bounded(1);
        let (renewals, renewal_requests) = crossbeam_channel::bounded(1);
        let lease_deadline = Arc::new(AtomicU64::new(request.lease_deadline));
        let mut worker = Self {
            child,
            renewals,
            lease_deadline,
            result,
        };
        let mut output = worker.child.stdout.take().unwrap();
        std::thread::Builder::new()
            .name("mesh-job-output".into())
            .spawn(move || {
                let result = read_frame(&mut output, MAX_REPLY_FRAME)
                    .map_err(|error| format!("invalid job handler reply: {error}"));
                let _ = reply.send(result.unwrap_or_else(Err));
            })
            .map_err(|error| error.to_string())?;

        let mut input = worker.child.stdin.take().unwrap();
        let latest = Arc::clone(&worker.lease_deadline);
        std::thread::Builder::new()
            .name("mesh-job-control".into())
            .spawn(move || {
                if write_frame(&mut input, &request, MAX_REQUEST_HEADER)
                    .and_then(|()| write_bytes(&mut input, context.as_bytes(), MAX_DB_VALUE_BYTES))
                    .is_err()
                {
                    return;
                }
                while renewal_requests.recv().is_ok() {
                    let lease = latest.load(Ordering::Acquire);
                    if input.write_all(&lease.to_le_bytes()).is_err() {
                        break;
                    }
                }
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

    pub(super) fn poll(&mut self) -> Result<Option<Reply>, String> {
        let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        let reason = match status.code() {
            Some(TIMEOUT) => "job handler timed out".into(),
            Some(LEASE_LOST) => "job lease lost".into(),
            Some(PARENT_GONE) => "job runner disconnected".into(),
            Some(BOOTSTRAP_ERROR) => "job handler could not enter its worker runtime".into(),
            Some(0) => {
                return Ok(match self.result.try_recv() {
                    Ok(outcome) => Some(outcome),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        Some(Err("job handler returned no result".into()))
                    }
                });
            }
            _ => format!("job handler process exited: {status}"),
        };
        Ok(Some(Err(reason)))
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        // A live child retains authority until its owner stops or reaps it.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[no_mangle]
pub unsafe extern "C" fn mesh_jobs_enter_worker(argc: i32, argv: *const *const c_char) {
    if argc == 2 && unsafe { CStr::from_ptr(*argv.add(1)) }.to_bytes() == WORKER_ARG.as_bytes() {
        enter_worker();
    }
    if !HANDLERS.lock().is_empty() {
        EXECUTABLE.get_or_init(Executable::locate);
    }
}

fn enter_worker() -> ! {
    match run_worker() {
        Ok(()) => exit_worker(0),
        Err(error) => {
            eprintln!("Mesh job handler initialization failed: {error}");
            exit_worker(BOOTSTRAP_ERROR);
        }
    }
}

fn run_worker() -> Result<(), String> {
    crate::panic::ignore_sigpipe();
    crate::process_signal::mesh_process_install_shutdown_signals();
    let mut result = redirect_stdout().map_err(|error| error.to_string())?;
    let mut control = io::stdin();
    let request: Request =
        read_frame(&mut control, MAX_REQUEST_HEADER).map_err(|error| error.to_string())?;
    let context = String::from_utf8(
        read_bytes(&mut control, MAX_DB_VALUE_BYTES).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let handler = HANDLERS
        .lock()
        .get(&request.handler)
        .copied()
        .ok_or("job handler is missing from this executable")?;

    let (renew, updates) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("mesh-job-parent".into())
        .spawn(move || {
            let mut frame = [0; 8];
            while control.read_exact(&mut frame).is_ok() {
                if renew.send(u64::from_le_bytes(frame)).is_err() {
                    return;
                }
            }
            exit_worker(PARENT_GONE);
        })
        .map_err(|error| error.to_string())?;
    let deadline = request.deadline;
    let lease = request.lease_deadline;
    std::thread::Builder::new()
        .name("mesh-job-deadline".into())
        .spawn(move || enforce_deadlines(deadline, lease, updates))
        .map_err(|error| error.to_string())?;
    remaining(deadline, lease);

    let outcome = std::panic::catch_unwind(|| {
        let input = mesh_str(&context);
        let output = unsafe { handler(input).as_ref() }.ok_or("job handler returned no outcome")?;
        let bytes = unsafe { output.as_bytes() };
        if bytes.len() > MAX_OUTCOME {
            return Err("job handler outcome exceeds 64 KiB");
        }
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "job handler returned invalid UTF-8")
    });
    let reply: Reply = match outcome {
        Ok(value) => value.map_err(str::to_owned),
        Err(panic) => Err(crate::panic::mesh_panic_message(panic.as_ref())
            .unwrap_or("job handler panicked")
            .chars()
            .take(1024)
            .collect()),
    };
    remaining(deadline, u64::MAX);
    write_frame(&mut result, &reply, MAX_REPLY_FRAME).map_err(|error| error.to_string())?;
    Ok(())
}

fn enforce_deadlines(deadline: u64, mut lease: u64, updates: Receiver<u64>) {
    loop {
        let wait = remaining(deadline, lease).min(Duration::from_millis(20));
        match updates.recv_timeout(wait) {
            Ok(next) => {
                // A delayed renewal may not revive a worker whose lease already ended.
                remaining(deadline, lease);
                lease = lease.max(next);
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

fn read_frame<T: DeserializeOwned>(reader: &mut impl Read, max: usize) -> io::Result<T> {
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

fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T, max: usize) -> io::Result<()> {
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
