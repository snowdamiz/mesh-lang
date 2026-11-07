//! Scheduler-owned native invocations. A handle owns lifetime and an outcome,
//! never permission for an effect or native publication.

use super::*;
use crate::actor::process::ExitReason;
use crate::authority::{InvocationContext, NativeAuthority, REFUSED};
use std::sync::Arc;

pub(crate) const PENDING: i32 = 12;
pub(crate) const TAKEN: i32 = 13;
#[cfg(test)]
static FAIL_PREPARATION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Outcome {
    status: i32,
    bytes: Zeroizing<Vec<u8>>,
}
impl Outcome {
    fn status(status: i32) -> Self {
        Self {
            status,
            bytes: Zeroizing::new(Vec::new()),
        }
    }
}
enum Completion {
    Running(Option<Outcome>),
    Ready(Outcome),
    Taken,
}

pub struct OwnedInvocation {
    pub(crate) context: Arc<InvocationContext>,
    entrypoint: MeshLibraryEntrypoint,
    input: Zeroizing<Vec<u8>>,
    max_output: u64,
    completion: Mutex<Completion>,
}

impl OwnedInvocation {
    pub(crate) fn finish(&self, reason: &ExitReason) {
        let mut completion = self.completion.lock();
        let Completion::Running(staged) = &mut *completion else {
            return;
        };
        let outcome = if !self.context.live() {
            Outcome::status(REFUSED)
        } else {
            staged.take().unwrap_or_else(|| {
                Outcome::status(match reason {
                    ExitReason::Normal => MESH_LIBRARY_ERR_INVALID_ARGUMENT,
                    _ => MESH_LIBRARY_ERR_PANIC,
                })
            })
        };
        *completion = Completion::Ready(outcome);
    }

    pub(crate) fn cancel(&self) -> i32 {
        if crate::authority::in_callback() || crate::authority::current().is_some() {
            return MESH_LIBRARY_ERR_BUSY;
        }
        // Serialize against the once-only transfer, not against executing work.
        let mut completion = self.completion.lock();
        self.context.cancel();
        if matches!(*completion, Completion::Ready(_)) {
            *completion = Completion::Ready(Outcome::status(REFUSED));
        }
        drop(completion);
        actor::global_scheduler().cancel_invocation(self.context.id);
        MESH_LIBRARY_OK
    }

    pub(crate) unsafe fn take(&self, output: *mut MeshLibraryBytes) -> i32 {
        if output.is_null() {
            return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
        }
        output.write(MeshLibraryBytes::default());
        if crate::authority::in_callback() || crate::authority::current().is_some() {
            return MESH_LIBRARY_ERR_BUSY;
        }
        let mut completion = self.completion.lock();
        match &*completion {
            Completion::Running(_) => return PENDING,
            Completion::Taken => return TAKEN,
            Completion::Ready(_) => {}
        }
        let Completion::Ready(outcome) = std::mem::replace(&mut *completion, Completion::Taken)
        else {
            unreachable!()
        };
        if !self.context.live() {
            return REFUSED;
        }
        if !outcome.bytes.is_empty() {
            let data = malloc(outcome.bytes.len());
            if data.is_null() {
                return MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE;
            }
            ptr::copy_nonoverlapping(outcome.bytes.as_ptr(), data, outcome.bytes.len());
            output.write(MeshLibraryBytes {
                data,
                len: outcome.bytes.len() as u64,
            });
        }
        outcome.status
    }
}

/// Called only by the generated native companion, with its fixed entry thunk
/// and compiler-declared bounds. Native request bytes never select those values.
#[no_mangle]
pub unsafe extern "C" fn mesh_library_submit_bounded_v2(
    authority: *const NativeAuthority,
    entrypoint: Option<MeshLibraryEntrypoint>,
    input: *const u8,
    input_len: u64,
    output: *mut *const OwnedInvocation,
    max_input: u64,
    max_output: u64,
) -> i32 {
    if output.is_null() {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    }
    output.write(ptr::null());
    let Some(entrypoint) = entrypoint else {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    };
    match catch_unwind(AssertUnwindSafe(|| {
        submit(
            authority, entrypoint, input, input_len, max_input, max_output,
        )
    })) {
        Ok(Ok(owned)) => {
            output.write(Arc::into_raw(owned));
            MESH_LIBRARY_OK
        }
        Ok(Err(status)) => status,
        Err(_) => MESH_LIBRARY_ERR_PANIC,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mesh_library_invocation_take_v2(
    handle: *const OwnedInvocation,
    output: *mut MeshLibraryBytes,
) -> i32 {
    if !output.is_null() {
        output.write(MeshLibraryBytes::default());
    }
    let Some(owned) = handle.as_ref() else {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    };
    owned.take(output)
}

#[no_mangle]
pub unsafe extern "C" fn mesh_library_invocation_cancel_v2(handle: *const OwnedInvocation) -> i32 {
    let Some(owned) = handle.as_ref() else {
        return MESH_LIBRARY_ERR_INVALID_ARGUMENT;
    };
    owned.cancel()
}

#[no_mangle]
pub unsafe extern "C" fn mesh_library_invocation_release_v2(handle: *const OwnedInvocation) {
    if !handle.is_null() {
        drop(Arc::from_raw(handle));
    }
}

pub(crate) unsafe fn submit(
    authority: *const NativeAuthority,
    entrypoint: MeshLibraryEntrypoint,
    input: *const u8,
    input_len: u64,
    max_input: u64,
    max_output: u64,
) -> Result<Arc<OwnedInvocation>, i32> {
    if authority.is_null()
        || (input.is_null() && input_len != 0)
        || !(1..=MAX_EXPORT_BYTES).contains(&max_input)
        || !(1..=MAX_EXPORT_BYTES).contains(&max_output)
    {
        return Err(MESH_LIBRARY_ERR_INVALID_ARGUMENT);
    }
    if input_len > max_input {
        return Err(MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE);
    }
    if crate::authority::in_callback() || crate::authority::current().is_some() {
        return Err(MESH_LIBRARY_ERR_BUSY);
    }
    let lifecycle = LIFECYCLE.lock();
    if !matches!(*lifecycle, Lifecycle::Running(_)) {
        return Err(MESH_LIBRARY_ERR_NOT_INITIALIZED);
    }
    if actor::global_scheduler().worker_bounds() != (1, 1) {
        return Err(crate::authority::UNSUPPORTED);
    }
    #[cfg(test)]
    if FAIL_PREPARATION.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return Err(MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE);
    }
    let context = InvocationContext::bind(authority, true)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(input_len as usize)
        .map_err(|_| MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE)?;
    if input_len != 0 {
        bytes.extend_from_slice(std::slice::from_raw_parts(input, input_len as usize));
    }
    let owned = Arc::new(OwnedInvocation {
        context,
        entrypoint,
        input: Zeroizing::new(bytes),
        max_output,
        completion: Mutex::new(Completion::Running(None)),
    });
    actor::global_scheduler().spawn_owned_invocation(Arc::clone(&owned));
    drop(lifecycle);
    Ok(owned)
}

pub(crate) extern "C-unwind" fn run(_args: *const u8) {
    let owned = actor::current_process()
        .expect("scheduled root")
        .lock()
        .owned_invocation
        .as_ref()
        .expect("owned before enqueue")
        .clone();
    let input = mesh_bytes_new(owned.input.as_ptr(), owned.input.len() as u64);
    let mut result = MeshLibraryCallResult {
        tag: u8::MAX,
        _padding: [0; 7],
        value: ptr::null_mut(),
    };
    // Panic and forced cancellation unwind to the scheduler, which finalizes
    // the root only after its Rust frames and process resources are released.
    unsafe { (owned.entrypoint)(input, &mut result) };
    let outcome = if !owned.context.live() {
        Outcome::status(REFUSED)
    } else if result.tag == 1 {
        Outcome::status(MESH_LIBRARY_ERR_APPLICATION)
    } else if result.tag != 0 || result.value.is_null() {
        Outcome::status(MESH_LIBRARY_ERR_INVALID_ARGUMENT)
    } else {
        let bytes = unsafe { &*result.value.cast::<MeshBytes>() };
        if bytes.len > owned.max_output {
            Outcome::status(MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE)
        } else {
            Outcome {
                status: MESH_LIBRARY_OK,
                bytes: Zeroizing::new(unsafe { bytes.as_slice() }.to_vec()),
            }
        }
    };
    *owned.completion.lock() = Completion::Running(Some(outcome));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    static AFTER_HTTP: AtomicBool = AtomicBool::new(false);

    #[test]
    fn same_original_service_and_root_descendants_reject_new_invocation_work() {
        if std::env::var_os("MESH_SERVICE_ORIGIN_CHILD").is_none() {
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "library::invocation::tests::same_original_service_and_root_descendants_reject_new_invocation_work"])
                .env("MESH_SERVICE_ORIGIN_CHILD", "1")
                .env("MESH_SCHEDULER_MIN_WORKERS", "1").env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status().unwrap().success());
            return;
        }
        static CHILD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static AFTER_WAIT: AtomicBool = AtomicBool::new(false);
        static ROOT: Mutex<Option<std::sync::Weak<Mutex<crate::actor::process::Process>>>> =
            Mutex::new(None);
        extern "C-unwind" fn service(_: *const u8) {
            let message = actor::mesh_actor_receive(-1);
            unsafe {
                let caller = message.add(24).cast::<u64>().read_unaligned();
                actor::service::mesh_service_reply(caller, message.add(32), 8);
            }
            actor::mesh_timer_sleep(60_000);
            AFTER_WAIT.store(true, Ordering::SeqCst);
        }
        unsafe extern "C-unwind" fn root(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            *ROOT.lock() = Some(Arc::downgrade(&actor::current_process().unwrap()));
            let child = actor::mesh_actor_spawn(service as *const u8, ptr::null(), 0, 1);
            CHILD.store(child, Ordering::SeqCst);
            let value = 42u64;
            let reply =
                actor::service::mesh_service_call(child, 1, (&value as *const u64).cast(), 8);
            assert_eq!(reply.add(16).cast::<u64>().read_unaligned(), 42);
            echo(input, output);
        }
        unsafe extern "C-unwind" fn foreign(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            let child = CHILD.load(Ordering::SeqCst);
            let value = 43u64;
            let data = (&value as *const u64).cast();
            match *input.cast::<u8>().add(8) {
                0 => {
                    actor::service::mesh_service_call(child, 1, data, 8);
                }
                1 => actor::service::mesh_service_cast_shaped(child, data, 8, ptr::null()),
                2 => actor::service::mesh_service_reply(child, data, 8),
                3 => {
                    assert_eq!(actor::mesh_actor_send(child, data, 8), REFUSED as i64);
                }
                4 => actor::mesh_actor_link(child),
                _ => unreachable!(),
            }
            echo(input, output);
        }
        assert_eq!(mesh_library_init(), 0);
        let authority = crate::authority::test_invocation();
        unsafe {
            let original = Arc::as_ptr(&authority.original);
            let owned = submit(original, root, b"root".as_ptr(), 4, 32, 32).unwrap();
            assert_eq!(terminal(&owned), (0, b"root".to_vec()));
            let child = actor::global_scheduler()
                .get_process(actor::ProcessId(CHILD.load(Ordering::SeqCst)))
                .unwrap();
            assert!(
                ROOT.lock().as_ref().unwrap().upgrade().is_some(),
                "child retains its original root heap loan"
            );
            for mode in 0u8..5 {
                let new = submit(original, foreign, &mode, 1, 32, 32).unwrap();
                assert_eq!(terminal(&new), (REFUSED, vec![]));
                assert!(child.lock().mailbox.is_empty());
            }
            let fresh = submit(original, echo, b"fresh".as_ptr(), 5, 32, 32).unwrap();
            assert_eq!(terminal(&fresh), (0, b"fresh".to_vec()));
            drop(child);
            assert_eq!(
                owned.cancel(),
                0,
                "cancel after Taken still stops original descendants"
            );
            let mut output = MeshLibraryBytes::default();
            assert_eq!(owned.take(&mut output), TAKEN);
            let deadline = Instant::now() + Duration::from_secs(5);
            while ROOT.lock().as_ref().unwrap().upgrade().is_some() {
                assert!(
                    Instant::now() < deadline,
                    "cancelled descendant retained root heap"
                );
                std::thread::yield_now();
            }
            assert!(!AFTER_WAIT.load(Ordering::SeqCst));
        }
        assert_eq!(mesh_library_shutdown(), 0);
    }

    #[test]
    fn completion_waits_for_root_removal_and_reclaims_unborrowed_heap() {
        if std::env::var_os("MESH_ROOT_REMOVAL_CHILD").is_none() {
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "library::invocation::tests::completion_waits_for_root_removal_and_reclaims_unborrowed_heap"])
                .env("MESH_ROOT_REMOVAL_CHILD", "1")
                .env("MESH_SCHEDULER_MIN_WORKERS", "1").env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status().unwrap().success());
            return;
        }
        static ROOT: Mutex<Option<std::sync::Weak<Mutex<crate::actor::process::Process>>>> =
            Mutex::new(None);
        unsafe extern "C-unwind" fn entry(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            *ROOT.lock() = Some(Arc::downgrade(&actor::current_process().unwrap()));
            echo(input, output);
        }
        let (arrived, reached) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        *actor::scheduler::BEFORE_OWNED_REMOVAL.lock() = Some(Box::new(move || {
            arrived.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
        }));
        assert_eq!(mesh_library_init(), 0);
        let authority = crate::authority::test_invocation();
        unsafe {
            let owned = submit(
                Arc::as_ptr(&authority.original),
                entry,
                b"root".as_ptr(),
                4,
                32,
                32,
            )
            .unwrap();
            reached.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut output = MeshLibraryBytes::default();
            let before_removal = owned.take(&mut output);
            mesh_library_free_returned_bytes(&mut output);
            release.send(()).unwrap();
            let answer = terminal(&owned);
            let root_reclaimed = ROOT.lock().as_ref().unwrap().upgrade().is_none();
            assert_eq!(mesh_library_shutdown(), 0);
            assert_eq!(
                before_removal, PENDING,
                "root is still registered at the cleanup barrier"
            );
            assert_eq!(answer, (0, b"root".to_vec()));
            assert!(
                root_reclaimed,
                "unborrowed root heap must be gone before terminal take"
            );
        }
    }

    #[test]
    fn all_nine_callbacks_and_lifetimes_use_original_context_and_native_stack() {
        if std::env::var_os("MESH_NINE_CALLBACK_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "library::invocation::tests::all_nine_callbacks_and_lifetimes_use_original_context_and_native_stack", "--nocapture"])
                .env("MESH_NINE_CALLBACK_CHILD", "1")
                .env("MESH_SCHEDULER_MIN_WORKERS", "1").env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status().unwrap();
            assert!(status.success());
            return;
        }
        use crate::authority::*;
        type Probe = Mutex<Vec<(u8, u64, bool)>>;
        fn record(probe: &Probe, kind: u8) {
            let marker = 0u8;
            let address = &marker as *const u8 as usize;
            let (low, high) = stack::current_thread_stack_bounds();
            probe.lock().push((
                kind,
                mesh_library_current_invocation_v2(),
                (low..high).contains(&address),
            ));
        }
        unsafe extern "C" fn retain(context: *mut c_void) {
            Arc::increment_strong_count(context.cast::<Probe>());
            record(&*context.cast::<Probe>(), 0);
        }
        unsafe extern "C" fn release(context: *mut c_void) {
            let probe = Arc::from_raw(context.cast::<Probe>());
            record(&probe, 10);
        }
        unsafe extern "C" fn callback(
            context: *mut c_void,
            input: *const u8,
            _: u64,
            output: *mut u8,
            _: u64,
            length: *mut u64,
        ) -> i32 {
            record(&*context.cast::<Probe>(), *input);
            *length = 1;
            *output = *input;
            0
        }
        unsafe extern "C-unwind" fn sweep(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            for capability in 1..=9 {
                let mut bytes = [0u8; 1];
                assert_eq!(
                    call_host_callback(capability, &[capability as u8], &mut bytes).unwrap(),
                    1
                );
                assert_eq!(bytes[0], capability as u8);
            }
            echo(input, output);
        }
        let probe: Arc<Probe> = Arc::new(Mutex::new(vec![]));
        unsafe {
            let mut domain = ptr::null();
            let mut generation = ptr::null();
            let mut authority = ptr::null();
            assert_eq!(mesh_authority_domain_create_v2(&mut domain), 0);
            assert_eq!(mesh_authority_generation_v2(domain, &mut generation), 0);
            let callbacks = MeshLibraryHostCallbacksV2 {
                abi_version: 2,
                struct_size: std::mem::size_of::<MeshLibraryHostCallbacksV2>() as u32,
                callbacks: MeshLibraryHostCallbacksV1 {
                    context: Arc::as_ptr(&probe) as *mut c_void,
                    secure_store_put: Some(callback),
                    secure_store_get: Some(callback),
                    secure_store_delete: Some(callback),
                    push_get_token: Some(callback),
                    background_schedule: Some(callback),
                    network_state: Some(callback),
                    monotonic_clock: Some(callback),
                    wall_clock: Some(callback),
                    log_redacted: Some(callback),
                    ..Default::default()
                },
                retain: Some(retain),
                release: Some(release),
            };
            assert_eq!(
                mesh_authority_bind_v2(domain, &generation, 1, &callbacks, &mut authority),
                0
            );
            assert_eq!(mesh_library_init(), 0);
            assert_eq!(
                mesh_library_register_host_callbacks(&MeshLibraryHostCallbacksV1::default()),
                0
            );
            let owned = submit(authority, sweep, b"original".as_ptr(), 8, 32, 32).unwrap();
            let id = owned.context.id;
            mesh_authority_release_v2(authority);
            assert_eq!(terminal(&owned), (0, b"original".to_vec()));
            drop(owned);
            assert_eq!(mesh_library_shutdown(), 0);
            let records = probe.lock();
            assert_eq!(records.len(), 11);
            assert_eq!(
                records.iter().map(|r| r.0).collect::<Vec<_>>(),
                (0..=10).collect::<Vec<_>>()
            );
            assert!(
                records.iter().all(|r| r.2),
                "every operation and lifetime callback uses the actual native stack"
            );
            assert!(records[1..10].iter().all(|r| r.1 == id && r.1 != 0));
            assert_eq!(Arc::strong_count(&probe), 1);
            mesh_authority_generation_release_v2(generation);
            mesh_authority_domain_release_v2(domain);
        }
    }

    #[test]
    fn owned_submission_failures_and_once_only_results() {
        if std::env::var_os("MESH_SUBMIT_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "library::invocation::tests::owned_submission_failures_and_once_only_results",
                    "--nocapture",
                ])
                .env("MESH_SUBMIT_CHILD", "1")
                .env("MESH_SCHEDULER_MIN_WORKERS", "1")
                .env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let authority = crate::authority::test_invocation();
        unsafe {
            let original = Arc::as_ptr(&authority.original);
            let mut handle = std::ptr::dangling();
            let call = |handle: *mut *const OwnedInvocation| {
                mesh_library_submit_bounded_v2(
                    original,
                    Some(echo),
                    b"owned".as_ptr(),
                    5,
                    handle,
                    32,
                    32,
                )
            };
            assert_eq!(call(&mut handle), 2);
            assert!(handle.is_null());
            assert_eq!(mesh_library_init(), 0);
            FAIL_PREPARATION.store(true, Ordering::SeqCst);
            assert_eq!(call(&mut handle), 6);
            assert!(handle.is_null());
            crate::actor::stack::FAIL_NEXT_STACK.store(true, Ordering::SeqCst);
            assert_eq!(call(&mut handle), 0);
            assert_eq!(terminal(&*handle), (4, vec![]));
            mesh_library_invocation_release_v2(handle);
            assert_eq!(call(&mut handle), 0);
            let owned = Arc::from_raw(handle);
            let until = Instant::now() + Duration::from_secs(5);
            while matches!(*owned.completion.lock(), Completion::Running(_)) {
                assert!(Instant::now() < until);
                std::thread::yield_now();
            }
            let mut output = MeshLibraryBytes::default();
            assert_eq!(owned.take(ptr::null_mut()), 1);
            {
                let _callback = crate::authority::callback_scope(true);
                assert_eq!(owned.take(&mut output), 3);
                assert!(output.data.is_null());
                assert_eq!(owned.cancel(), 3);
            }
            assert!(owned.context.live());
            let left = Arc::clone(&owned);
            let right = Arc::clone(&owned);
            let first = std::thread::spawn(move || terminal(&left));
            let second = std::thread::spawn(move || terminal(&right));
            let mut answers = [first.join().unwrap(), second.join().unwrap()];
            answers.sort_by_key(|answer| answer.0);
            assert_eq!(answers, [(0, b"owned".to_vec()), (TAKEN, vec![])]);
            assert_eq!(owned.cancel(), 0);
            assert_eq!(owned.take(&mut output), TAKEN);
            drop(owned);
            assert_eq!(call(&mut handle), 0);
            let owned = Arc::from_raw(handle);
            while matches!(*owned.completion.lock(), Completion::Running(_)) {
                std::thread::yield_now();
            }
            assert_eq!(owned.cancel(), 0);
            assert_eq!(terminal(&owned), (REFUSED, vec![]));
            drop(owned);
            assert_eq!(mesh_library_shutdown(), 0);
            assert_eq!(call(&mut handle), 2);
            assert!(handle.is_null());
            assert_eq!(mesh_library_invocation_take_v2(ptr::null(), &mut output), 1);
            assert!(output.data.is_null());
            assert_eq!(mesh_library_invocation_cancel_v2(ptr::null()), 1);
            mesh_library_invocation_release_v2(ptr::null());
        }
    }

    #[test]
    fn cancellation_settles_accepted_writes_and_never_starts_cancelled_queued_roots() {
        if std::env::var_os("MESH_FINITE_CANCEL_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "library::invocation::tests::cancellation_settles_accepted_writes_and_never_starts_cancelled_queued_roots", "--nocapture"])
                .env("MESH_FINITE_CANCEL_CHILD", "1")
                .env("MESH_SCHEDULER_MIN_WORKERS", "1").env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status().unwrap();
            assert!(status.success());
            return;
        }
        use crate::authority::*;
        use std::sync::atomic::AtomicUsize;
        struct Host {
            arrived: std::sync::mpsc::SyncSender<()>,
            resume: Mutex<std::sync::mpsc::Receiver<()>>,
            writes: AtomicUsize,
            releases: AtomicUsize,
            checks: Mutex<Vec<i32>>,
        }
        unsafe extern "C" fn retain(context: *mut c_void) {
            Arc::increment_strong_count(context.cast::<Host>());
        }
        unsafe extern "C" fn release(context: *mut c_void) {
            let host = Arc::from_raw(context.cast::<Host>());
            host.checks
                .lock()
                .extend([mesh_library_init(), mesh_library_shutdown()]);
            assert!(
                LIFECYCLE.try_lock().is_some(),
                "foreign release under lifecycle lock"
            );
            assert!(
                actor::global_scheduler()
                    .process_table()
                    .try_write()
                    .is_some(),
                "foreign release under process table lock"
            );
            host.releases.fetch_add(1, Ordering::SeqCst);
        }
        unsafe extern "C" fn write(
            context: *mut c_void,
            _: *const u8,
            _: u64,
            _: *mut u8,
            _: u64,
            length: *mut u64,
        ) -> i32 {
            let host = &*context.cast::<Host>();
            *length = 0;
            host.checks.lock().extend([
                mesh_library_init(),
                mesh_library_register_host_callbacks(&MeshLibraryHostCallbacksV1::default()),
                mesh_library_shutdown(),
            ]);
            host.arrived.send(()).unwrap();
            host.resume
                .lock()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            host.writes.fetch_add(1, Ordering::SeqCst);
            0
        }
        unsafe extern "C-unwind" fn store(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            mesh_host_secure_store_put(input);
            mesh_host_secure_store_put(input);
            echo(input, output);
        }
        static STARTED: AtomicBool = AtomicBool::new(false);
        unsafe extern "C-unwind" fn must_not_start(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            STARTED.store(true, Ordering::SeqCst);
            echo(input, output);
        }
        let (arrived, wait) = std::sync::mpsc::sync_channel(1);
        let (resume, receiver) = std::sync::mpsc::sync_channel(1);
        let host = Arc::new(Host {
            arrived,
            resume: Mutex::new(receiver),
            writes: AtomicUsize::new(0),
            releases: AtomicUsize::new(0),
            checks: Mutex::new(vec![]),
        });
        unsafe {
            let mut domain = ptr::null();
            let mut generation = ptr::null();
            let mut authority = ptr::null();
            assert_eq!(mesh_authority_domain_create_v2(&mut domain), 0);
            assert_eq!(mesh_authority_generation_v2(domain, &mut generation), 0);
            let callbacks = MeshLibraryHostCallbacksV2 {
                abi_version: 2,
                struct_size: std::mem::size_of::<MeshLibraryHostCallbacksV2>() as u32,
                callbacks: MeshLibraryHostCallbacksV1 {
                    context: Arc::as_ptr(&host) as *mut c_void,
                    secure_store_put: Some(write),
                    ..Default::default()
                },
                retain: Some(retain),
                release: Some(release),
            };
            assert_eq!(
                mesh_authority_bind_v2(domain, &generation, 1, &callbacks, &mut authority),
                0
            );
            assert_eq!(mesh_library_init(), 0);
            let executing = submit(authority, store, b"finite".as_ptr(), 6, 32, 32).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            let queued = submit(authority, must_not_start, b"queued".as_ptr(), 6, 32, 32).unwrap();
            assert_eq!(queued.cancel(), 0);
            assert_eq!(executing.cancel(), 0);
            assert_eq!(mesh_authority_local_inflight_v2(generation), 1);
            assert_eq!(executing.take(&mut MeshLibraryBytes::default()), PENDING);
            assert!(!STARTED.load(Ordering::SeqCst));
            resume.send(()).unwrap();
            assert_eq!(terminal(&executing), (REFUSED, vec![]));
            assert_eq!(terminal(&queued), (REFUSED, vec![]));
            assert_eq!(host.writes.load(Ordering::SeqCst), 1);
            assert_eq!(mesh_authority_local_inflight_v2(generation), 0);
            assert!(!STARTED.load(Ordering::SeqCst));
            mesh_authority_release_v2(authority);
            drop(executing);
            drop(queued);
            assert_eq!(mesh_library_shutdown(), 0);
            let deadline = Instant::now() + Duration::from_secs(2);
            while host.releases.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert_eq!(host.releases.load(Ordering::SeqCst), 1);
            assert_eq!(*host.checks.lock(), [3, 3, 3, 3, 3]);
            assert_eq!(Arc::strong_count(&host), 1);
            mesh_authority_generation_release_v2(generation);
            mesh_authority_domain_release_v2(domain);
        }
    }

    #[test]
    fn unsupported_captures_do_not_retain_root_heaps_or_context() {
        if std::env::var_os("MESH_CAPTURE_CHILD").is_none() {
            let outcomes: Vec<_> = ["router", "timer"].into_iter().map(|mode| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "library::invocation::tests::unsupported_captures_do_not_retain_root_heaps_or_context", "--nocapture"])
                    .env("MESH_CAPTURE_CHILD", mode)
                    .env("MESH_SCHEDULER_MIN_WORKERS", "1").env("MESH_SCHEDULER_MAX_WORKERS", "1")
                    .status().unwrap().success()
            }).collect();
            assert_eq!(outcomes, [true, true]);
            return;
        }
        static ROOT: Mutex<Option<std::sync::Weak<Mutex<crate::actor::process::Process>>>> =
            Mutex::new(None);
        unsafe extern "C-unwind" fn capture(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            *ROOT.lock() = Some(Arc::downgrade(&actor::current_process().unwrap()));
            if std::env::var("MESH_CAPTURE_CHILD").unwrap() == "timer" {
                actor::mesh_timer_send_after_shaped(
                    actor::mesh_actor_self() as i64,
                    i64::MAX,
                    (&input as *const *mut MeshBytes).cast(),
                    8,
                    ptr::null(),
                );
            } else {
                let router = crate::http::router::mesh_http_router();
                crate::http::router::mesh_http_route(
                    router,
                    crate::string::mesh_str("/private"),
                    ptr::null_mut(),
                    input.cast(),
                );
            }
            echo(input, output);
        }
        assert_eq!(mesh_library_init(), 0);
        let authority = crate::authority::test_invocation();
        unsafe {
            let owned = submit(
                Arc::as_ptr(&authority.original),
                capture,
                b"owned-request".as_ptr(),
                13,
                32,
                32,
            )
            .unwrap();
            let lifetime = Arc::downgrade(&owned.context);
            let answer = terminal(&owned);
            owned.cancel();
            drop(owned);
            assert_eq!(answer, (REFUSED, vec![]));
            let until = Instant::now() + Duration::from_secs(2);
            while ROOT.lock().as_ref().unwrap().upgrade().is_some() && Instant::now() < until {
                std::thread::yield_now();
            }
            assert!(
                ROOT.lock().as_ref().unwrap().upgrade().is_none(),
                "unsupported capture pinned the root heap"
            );
            assert!(
                lifetime.upgrade().is_none(),
                "cancelled dormant work retained original context"
            );
        }
        assert_eq!(mesh_library_shutdown(), 0);
    }

    #[test]
    fn writer_transaction_aborts_http_before_dispatch() {
        if std::env::var_os("MESH_TXN_SUSPEND_CHILD").is_none() {
            for mode in [
                "http",
                "sleep",
                "receive",
                "job",
                "service",
                "cooperative",
                "timeout",
                "reduction",
                "resource",
            ] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "library::invocation::tests::writer_transaction_aborts_http_before_dispatch",
                    "--nocapture",
                ])
                .env("MESH_TXN_SUSPEND_CHILD", mode)
                .env("MESH_SCHEDULER_MIN_WORKERS", "1")
                .env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status()
                .unwrap();
                assert!(status.success());
            }
            return;
        }
        static RESOURCE: Mutex<()> = Mutex::new(());
        unsafe extern "C-unwind" fn transaction(
            input: *mut MeshBytes,
            output: *mut MeshLibraryCallResult,
        ) {
            let opened = &*crate::db::sqlite::mesh_sqlite_open(crate::string::mesh_str(":memory:"))
                .cast::<MeshResult>();
            assert_eq!(opened.tag, 0);
            let connection = *opened.value.cast::<u64>();
            let mode = std::env::var("MESH_TXN_SUSPEND_CHILD").unwrap();
            let _resource = (mode == "resource").then(|| crate::authority::resource_scope());
            let _lock = (mode == "resource").then(|| RESOURCE.lock());
            if mode != "resource" {
                assert_eq!(
                    (*crate::db::sqlite::mesh_sqlite_begin(connection).cast::<MeshResult>()).tag,
                    0
                );
            }
            match mode.as_str() {
                "http" => http(input, output),
                "sleep" | "resource" => actor::mesh_timer_sleep(1),
                "receive" => {
                    actor::mesh_actor_receive(1);
                }
                "job" => {
                    actor::job::mesh_job_await(actor::mesh_actor_self());
                }
                "service" => {
                    actor::service::mesh_service_call(actor::mesh_actor_self(), 1, ptr::null(), 0);
                }
                "cooperative" => {
                    let (_send, recv) = actor::cooperative_channel::<()>();
                    let _ = actor::cooperative_recv(&recv);
                }
                "timeout" => {
                    let (_send, recv) = actor::cooperative_channel::<()>();
                    let _ = actor::cooperative_recv_timeout(&recv, Duration::from_millis(1));
                }
                "reduction" => {
                    for _ in 0..10_000 {
                        actor::mesh_reduction_check();
                    }
                }
                _ => unreachable!(),
            }
            AFTER_HTTP.store(true, Ordering::SeqCst);
            echo(input, output);
        }
        unsafe {
            use crate::authority::*;
            let mut domain = ptr::null();
            let mut generation = ptr::null();
            let mut authority = ptr::null();
            assert_eq!(mesh_authority_domain_create_v2(&mut domain), 0);
            assert_eq!(mesh_authority_generation_v2(domain, &mut generation), 0);
            assert_eq!(
                mesh_authority_bind_v2(domain, &generation, 1, ptr::null(), &mut authority),
                0
            );
            assert_eq!(mesh_library_init(), 0);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/must-not-send", listener.local_addr().unwrap());
            let owned = submit(
                authority,
                transaction,
                url.as_ptr(),
                url.len() as u64,
                1024,
                1024,
            )
            .unwrap();
            assert_eq!(terminal(&owned), (REFUSED, vec![]));
            assert!(
                !AFTER_HTTP.load(Ordering::SeqCst),
                "a refused suspension must unwind even when Mesh ignores the error"
            );
            assert_eq!(
                owned.context.transactions.load(Ordering::SeqCst),
                0,
                "root cleanup rolled back its transaction"
            );
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            assert!(
                RESOURCE.try_lock().is_some(),
                "resource mutex released during unwind"
            );
            let fresh = submit(authority, echo, b"fresh".as_ptr(), 5, 32, 32).unwrap();
            assert_eq!(terminal(&fresh), (0, b"fresh".to_vec()));
            drop(owned);
            drop(fresh);
            assert_eq!(mesh_library_shutdown(), 0);
            mesh_authority_release_v2(authority);
            mesh_authority_generation_release_v2(generation);
            mesh_authority_domain_release_v2(domain);
        }
    }

    #[test]
    fn standalone_callback_preserves_init_and_registration() {
        if std::env::var_os("MESH_CALLBACK_COMPAT_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "library::invocation::tests::standalone_callback_preserves_init_and_registration", "--nocapture"])
                .env("MESH_CALLBACK_COMPAT_CHILD", "1")
                .env("MESH_SCHEDULER_MIN_WORKERS", "1")
                .env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status().unwrap();
            assert!(status.success());
            return;
        }
        unsafe extern "C" fn callback(
            context: *mut c_void,
            _: *const u8,
            _: u64,
            _: *mut u8,
            _: u64,
            length: *mut u64,
        ) -> i32 {
            let statuses = &mut *context.cast::<Vec<i32>>();
            *length = 0;
            statuses.push(mesh_library_init());
            statuses.push(mesh_library_register_host_callbacks(
                &MeshLibraryHostCallbacksV1::default(),
            ));
            statuses.push(mesh_library_shutdown());
            0
        }
        let mut statuses = Vec::<i32>::new();
        assert_eq!(mesh_library_init(), 0);
        let callbacks = MeshLibraryHostCallbacksV1 {
            context: (&mut statuses as *mut Vec<i32>).cast(),
            network_state: Some(callback),
            ..Default::default()
        };
        assert_eq!(mesh_library_register_host_callbacks(&callbacks), 0);
        assert!(call_host_callback(6, b"", &mut []).is_ok());
        assert_eq!(statuses, [0, 0, 3]);
        assert_eq!(mesh_library_shutdown(), 0);
    }
    unsafe extern "C-unwind" fn echo(input: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
        (*output).tag = 0;
        (*output).value = input.cast();
    }
    unsafe extern "C-unwind" fn http(input: *mut MeshBytes, output: *mut MeshLibraryCallResult) {
        let url = std::str::from_utf8((*input).as_slice()).unwrap();
        let request = crate::http::client::mesh_http_build(
            crate::string::mesh_str("get"),
            crate::string::mesh_str(url),
        );
        crate::http::client::mesh_http_timeout(request, 10000);
        crate::http::client::mesh_http_send(request);
        AFTER_HTTP.store(true, Ordering::SeqCst);
        echo(input, output);
    }
    unsafe fn terminal(owned: &OwnedInvocation) -> (i32, Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut output = MeshLibraryBytes::default();
            let status = owned.take(&mut output);
            if status != PENDING {
                let bytes = if output.data.is_null() {
                    vec![]
                } else {
                    std::slice::from_raw_parts(output.data, output.len as usize).to_vec()
                };
                mesh_library_free_returned_bytes(&mut output);
                return (status, bytes);
            }
            assert!(Instant::now() < deadline, "root completion timeout");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn actual_http_cancellation_keeps_fresh_process_runnable() {
        // The runtime is process-global; isolate this real scheduler/lifecycle
        // test from the unit binary's unrelated parallel scheduler fixtures.
        if std::env::var_os("MESH_OWNED_INVOCATION_TEST_CHILD").is_none() {
            for mode in ["success", "error"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "library::invocation::tests::actual_http_cancellation_keeps_fresh_process_runnable", "--nocapture"])
                .env("MESH_OWNED_INVOCATION_TEST_CHILD", mode)
                .env("MESH_SCHEDULER_MIN_WORKERS", "1")
                .env("MESH_SCHEDULER_MAX_WORKERS", "1")
                .status().unwrap();
                assert!(status.success());
            }
            return;
        }
        use std::sync::atomic::AtomicUsize;
        unsafe extern "C" fn retain(context: *mut c_void) {
            Arc::increment_strong_count(context.cast::<AtomicUsize>());
        }
        unsafe extern "C" fn release_context(context: *mut c_void) {
            let count = Arc::from_raw(context.cast::<AtomicUsize>());
            count.fetch_add(1, Ordering::SeqCst);
        }
        let releases = Arc::new(AtomicUsize::new(0));
        unsafe {
            use crate::authority::*;
            let mut domain = ptr::null();
            let mut generation = ptr::null();
            let mut authority = ptr::null();
            assert_eq!(mesh_authority_domain_create_v2(&mut domain), 0);
            assert_eq!(mesh_authority_generation_v2(domain, &mut generation), 0);
            let callbacks = MeshLibraryHostCallbacksV2 {
                abi_version: 2,
                struct_size: std::mem::size_of::<MeshLibraryHostCallbacksV2>() as u32,
                callbacks: MeshLibraryHostCallbacksV1 {
                    context: Arc::as_ptr(&releases) as *mut c_void,
                    ..Default::default()
                },
                retain: Some(retain),
                release: Some(release_context),
            };
            assert_eq!(
                mesh_authority_bind_v2(domain, &generation, 1, &callbacks, &mut authority),
                0
            );
            assert_eq!(
                submit(authority, echo, ptr::null(), 0, 32, 32).err(),
                Some(2)
            );
            assert_eq!(mesh_library_init(), 0);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/held", listener.local_addr().unwrap());
            let (arrived, held) = std::sync::mpsc::sync_channel(1);
            let (release, resume) = std::sync::mpsc::sync_channel(1);
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                }
                arrived.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                if std::env::var("MESH_OWNED_INVOCATION_TEST_CHILD").unwrap() == "success" {
                    let _ = socket.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                }
            });
            let old = submit(authority, http, url.as_ptr(), url.len() as u64, 1024, 1024).unwrap();
            held.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut source = b"fresh".to_vec();
            let fresh = submit(
                authority,
                echo,
                source.as_ptr(),
                source.len() as u64,
                32,
                32,
            )
            .unwrap();
            source.fill(b'x');
            assert_eq!(terminal(&fresh), (0, b"fresh".to_vec()));
            let mut output = MeshLibraryBytes::default();
            assert_eq!(fresh.take(&mut output), TAKEN);
            assert!(!AFTER_HTTP.load(Ordering::SeqCst));
            assert_eq!(old.cancel(), 0);
            assert_eq!(old.cancel(), 0);
            assert_eq!(terminal(&old), (REFUSED, vec![]));
            assert!(!AFTER_HTTP.load(Ordering::SeqCst));
            // Shutdown joins scheduler cleanup while admitted HTTP is still held.
            assert_eq!(mesh_library_shutdown(), 0);
            assert_eq!(
                submit(authority, echo, ptr::null(), 0, 32, 32).err(),
                Some(2)
            );
            assert_eq!(old.take(&mut output), TAKEN);
            drop(old);
            drop(fresh);
            mesh_authority_release_v2(authority);
            assert_eq!(
                releases.load(Ordering::SeqCst),
                0,
                "HTTP still owns the original Host context after shutdown"
            );
            release.send(()).unwrap();
            server.join().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while releases.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert_eq!(releases.load(Ordering::SeqCst), 1);
            assert_eq!(Arc::strong_count(&releases), 1);
            mesh_authority_generation_release_v2(generation);
            mesh_authority_domain_release_v2(domain);
        }
    }
}
