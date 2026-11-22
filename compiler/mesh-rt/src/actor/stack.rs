//! Corosensei-based stackful coroutine management for Mesh actors.
//!
//! Each actor runs as a stackful coroutine with a 64 KiB stack. The coroutine
//! yields when its reduction counter is exhausted, allowing the scheduler to
//! run other actors on the same OS thread.
//!
//! ## Thread-local State
//!
//! Thread-locals track the current execution context:
//! - `CURRENT_YIELDER`: pointer to the active coroutine's Yielder (for yield on reduction exhaustion)
//! - `CURRENT_PID`: the PID of the currently running actor (for `mesh_actor_self()`)
//! - `CURRENT_PROCESS`: that actor's process handle, valid during its timeslice
//!
//! The base of a coroutine's stack, where the collector's scan ends, is kept
//! on its process (`Process::stack_base`).

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use parking_lot::Mutex;

use super::process::{Process, ProcessId, DEFAULT_STACK_SIZE};

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Thread-local current-actor context
// ---------------------------------------------------------------------------

/// Private unwind marker for a normal stop, distinct from an actor failure.
pub(crate) struct ActorStopped;

// Part of the mesh_reduction_state_v1 ABI. Zero requests a runtime check.
pub(super) const HOST_REDUCTION_IDLE: u32 = u32::MAX;

/// The running coroutine's yielder and this thread's reduction countdown.
///
/// They share one thread-local because `mesh_reduction_check` reads both at
/// every call site and loop back-edge, and each thread-local access is a
/// function call on some targets.
pub struct YielderSlot {
    pub(crate) yielder: Cell<Option<*const ()>>,
    pub(crate) reductions: Cell<u32>,
    /// The host's next reduction check collects when this is zero.
    pub(super) host_gc: Cell<u32>,
}

struct CachedProcess {
    pid: ProcessId,
    handle: Arc<Mutex<Process>>,
    on_worker: bool,
}

thread_local! {
    /// Raw pointer to the current coroutine's Yielder.
    ///
    /// Set before resuming a coroutine, cleared after it yields or completes.
    /// Used by `mesh_reduction_check()` to yield the current actor.
    ///
    /// Safety: The pointer is valid only while the coroutine is running.
    /// We store it as `*const ()` to erase the lifetime; the Yielder is
    /// borrowed from within the coroutine body and remains valid for the
    /// duration of that resume.
    pub static CURRENT_YIELDER: YielderSlot = const {
        YielderSlot {
            yielder: Cell::new(None),
            reductions: Cell::new(super::process::DEFAULT_REDUCTIONS),
            host_gc: Cell::new(HOST_REDUCTION_IDLE),
        }
    };

    /// PID of the currently executing actor on this thread.
    pub static CURRENT_PID: Cell<Option<ProcessId>> = const { Cell::new(None) };
    static CURRENT_PROCESS: RefCell<Option<CachedProcess>> = const { RefCell::new(None) };
}

pub(crate) fn request_safepoint_gc() {
    CURRENT_YIELDER.with(|slot| slot.host_gc.set(0));
}

/// Set the current actor PID on this thread.
pub fn set_current_pid(pid: ProcessId) {
    CURRENT_PID.with(|c| c.set(Some(pid)));
}

/// Get the current actor PID on this thread.
pub fn get_current_pid() -> Option<ProcessId> {
    CURRENT_PID.with(|c| c.get())
}

pub(crate) fn set_current_process(pid: ProcessId, process: &Arc<Mutex<Process>>) {
    set_current_pid(pid);
    CURRENT_PROCESS.with(|slot| {
        *slot.borrow_mut() = Some(CachedProcess {
            pid,
            handle: Arc::clone(process),
            on_worker: true,
        });
    });
}

/// Only a worker owns its current process until it leaves the timeslice.
/// Host processes can be finalized by a different thread.
pub(crate) fn worker_process(pid: ProcessId) -> Option<Arc<Mutex<Process>>> {
    CURRENT_PROCESS
        .try_with(|slot| {
            slot.borrow()
                .as_ref()
                .filter(|cached| cached.on_worker && cached.pid == pid)
                .map(|cached| Arc::clone(&cached.handle))
        })
        .ok()
        .flatten()
}

/// Borrow the current allocation handle. Host allocation uses a lazy cache.
/// The callback must not suspend the actor.
pub(crate) fn with_allocation_process<T>(f: impl FnOnce(&Arc<Mutex<Process>>) -> T) -> Option<T> {
    let pid = get_current_pid()?;
    CURRENT_PROCESS
        .try_with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_none_or(|cached| cached.pid != pid) {
                *slot = Some(CachedProcess {
                    pid,
                    handle: super::process(pid)?,
                    on_worker: false,
                });
            }
            slot.as_ref().map(|cached| f(&cached.handle))
        })
        .ok()
        .flatten()
}

/// Clear the current actor PID on this thread.
pub fn clear_current_pid() {
    CURRENT_PID.with(|c| c.set(None));
    let _ = CURRENT_PROCESS.try_with(|slot| slot.borrow_mut().take());
}

// ---------------------------------------------------------------------------
// Yield support
// ---------------------------------------------------------------------------

/// Yield the current coroutine (called from `mesh_reduction_check`).
///
/// After `suspend()` returns (coroutine is resumed), we re-install the yielder
/// into the thread-local because another coroutine may have run on this thread
/// in between and overwritten it.
///
/// # Safety
///
/// Must only be called from within a running coroutine (i.e., CURRENT_YIELDER
/// is set). Panics if called outside of a coroutine context.
// Never inlined: a context switch saves every callee-saved register, and
// inlining it would put that prologue on `mesh_reduction_check`'s fast path.
#[inline(never)]
pub fn yield_current() {
    // Every suspension is a safe cooperative collection point, including
    // blocking receive in long-lived actors that may never exhaust reductions.
    super::try_trigger_gc();

    suspend_current();
}

/// Suspend at a safe point that has already checked its actor's heap.
#[inline(never)]
pub(super) fn suspend_current() {
    CURRENT_YIELDER.with(|c| {
        let c = &c.yielder;
        let ptr = c
            .get()
            .expect("yield_current called outside of coroutine context");
        // Safety: The pointer is valid because we are inside the coroutine body
        // that set it, and the Yielder is borrowed for the duration of the body.
        let yielder: &Yielder<(), ()> = unsafe { &*(ptr as *const Yielder<(), ()>) };
        yielder.suspend(());
        // Re-install the yielder after resume. Another coroutine may have
        // overwritten the thread-local while we were suspended.
        c.set(Some(ptr));
    });
}

// ---------------------------------------------------------------------------
// CoroutineHandle
// ---------------------------------------------------------------------------

/// A handle wrapping a corosensei `Coroutine` for an actor.
///
/// The coroutine runs the actor's entry function on a dedicated 64 KiB stack.
/// It yields when its reduction counter is exhausted and resumes later.
///
/// `CoroutineHandle` is `!Send` because corosensei coroutines cannot be moved
/// across threads. The scheduler ensures coroutines stay on the thread that
/// created them.
pub struct CoroutineHandle {
    coro: Option<ActorCoroutine>,
    /// Lowest address of the coroutine's stack (its guard page).
    stack_limit: usize,
}

type PanicPayload = Box<dyn Any + Send>;
type ActorCoroutine = Coroutine<(), (), Result<(), PanicPayload>>;

thread_local! {
    static DROPPING_STACK_LIMIT: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn is_cancelling_coroutine() -> bool {
    DROPPING_STACK_LIMIT.get() != 0
}

impl CoroutineHandle {
    /// Create a new coroutine that will call `entry_fn(args_ptr)`.
    ///
    /// The entry function signature matches the Mesh actor ABI:
    /// `extern "C" fn(args: *const u8)`.
    ///
    /// The coroutine installs its Yielder into the thread-local before
    /// calling the entry function, so `mesh_reduction_check()` can yield.
    pub fn new(entry_fn: *const u8, args_ptr: *const u8) -> Self {
        let stack =
            DefaultStack::new(DEFAULT_STACK_SIZE).expect("failed to allocate coroutine stack");
        let stack_limit = {
            use corosensei::stack::Stack;
            stack.limit().get()
        };

        // Capture the function pointer and args for the closure.
        let fn_ptr = entry_fn as usize;
        let args = args_ptr as usize;

        let coro = Coroutine::with_stack(stack, move |yielder: &Yielder<(), ()>, _input: ()| {
            // Capture the stack base at the very start of the coroutine body.
            // This local variable is near the base of the coroutine stack, so
            // its address serves as the upper bound for GC stack scanning.
            let stack_anchor: u64 = 0;
            let _ = std::hint::black_box(&stack_anchor);
            if let Some(process) = crate::actor::current_process() {
                process.lock().stack_base = &stack_anchor as *const u64 as *const u8;
            }

            // Install yielder in thread-local so mesh_reduction_check can access it.
            CURRENT_YIELDER.with(|c| {
                c.yielder
                    .set(Some(yielder as *const Yielder<(), ()> as *const ()));
            });

            // Call the actor entry function.
            // Safety: The fn_ptr was provided by the scheduler from a valid extern "C" fn.
            let func: extern "C-unwind" fn(*const u8) =
                unsafe { std::mem::transmute::<usize, extern "C-unwind" fn(*const u8)>(fn_ptr) };

            // During cancellation corosensei must distinguish its own unwind
            // from a new panic. During resume the scheduler can handle the
            // payload as a return value across the stack switch.
            // The scheduler clears CURRENT_YIELDER after either outcome.
            match catch_unwind(AssertUnwindSafe(|| func(args as *const u8))) {
                Err(panic) if DROPPING_STACK_LIMIT.get() == stack_limit => resume_unwind(panic),
                result => result,
            }
        });

        CoroutineHandle {
            coro: Some(coro),
            stack_limit,
        }
    }

    /// Resume the coroutine.
    ///
    /// Returns `true` if the coroutine yielded (still has work to do),
    /// `false` if it completed (returned from entry function).
    pub fn resume(&mut self) -> bool {
        match self.resume_once() {
            CoroutineResult::Yield(()) => true,
            CoroutineResult::Return(Ok(())) => false,
            CoroutineResult::Return(Err(panic)) => resume_unwind(panic),
        }
    }

    fn resume_once(&mut self) -> CoroutineResult<(), Result<(), PanicPayload>> {
        let limit = self.stack_limit;
        let coro = self.coro.as_mut().expect("coroutine exists until drop");
        crate::stack_overflow::on_stack(limit, || coro.resume(()))
    }

    /// Resume an actor while converting an unwinding Mesh panic into an exit
    /// reason that the scheduler can propagate to links and supervisors.
    pub(crate) fn resume_catching_panic(&mut self) -> Result<bool, String> {
        let panic = match catch_unwind(AssertUnwindSafe(|| self.resume_once())) {
            Ok(CoroutineResult::Yield(())) => return Ok(true),
            Ok(CoroutineResult::Return(Ok(()))) => return Ok(false),
            Ok(CoroutineResult::Return(Err(panic))) => panic,
            Err(panic) => {
                crate::stack_overflow::back_on_thread_stack();
                panic
            }
        };
        if panic.is::<ActorStopped>() {
            return Ok(false);
        }
        Err(crate::panic::actor_exit_reason(&*panic))
    }

    /// Check whether the coroutine has finished.
    pub fn done(&self) -> bool {
        self.coro
            .as_ref()
            .expect("coroutine exists until drop")
            .done()
    }
}

impl Drop for CoroutineHandle {
    fn drop(&mut self) {
        let coro = self.coro.take().expect("coroutine exists until drop");
        if !coro.done() {
            let previous = DROPPING_STACK_LIMIT.replace(self.stack_limit);
            drop(coro);
            DROPPING_STACK_LIMIT.set(previous);
        }
    }
}

/// The highest address of the calling thread's stack, where a conservative
/// scan of it has to stop, or null where the platform offers no way to ask
/// (the thread then never collects, as the main thread used not to).
pub(crate) fn current_thread_stack_base() -> *const u8 {
    current_thread_stack_bounds().1 as *const u8
}

/// The lowest and the highest address of the calling thread's stack, or
/// zeros where the platform does not say.
pub(crate) fn current_thread_stack_bounds() -> (usize, usize) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    unsafe {
        let thread = libc::pthread_self();
        let top = libc::pthread_get_stackaddr_np(thread) as usize;
        (
            top.saturating_sub(libc::pthread_get_stacksize_np(thread)),
            top,
        )
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        // Unknown (zeros) if either query fails: nothing is written then.
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        let mut lowest: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) == 0 {
            libc::pthread_attr_getstack(&attr, &mut lowest, &mut size);
            libc::pthread_attr_destroy(&mut attr);
        }
        (lowest as usize, lowest as usize + size)
    }
    #[cfg(windows)]
    unsafe {
        extern "system" {
            fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
        }
        let (mut low, mut high) = (0usize, 0usize);
        GetCurrentThreadStackLimits(&mut low, &mut high);
        (low, high)
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android",
        windows
    )))]
    {
        (0, 0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic that carries neither a String nor a &str is still an exit
    /// reason.
    #[test]
    fn a_panic_of_any_other_payload_is_an_actor_panic() {
        extern "C-unwind" fn panics(_args: *const u8) {
            std::panic::panic_any(42);
        }
        let mut handle = CoroutineHandle::new(panics as *const u8, std::ptr::null());
        assert_eq!(
            handle.resume_catching_panic(),
            Err("actor panicked".to_string())
        );
    }
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn dropping_a_suspended_coroutine_runs_its_destructors() {
        use std::sync::atomic::AtomicBool;

        static DROPPED: AtomicBool = AtomicBool::new(false);
        static RESUMED: AtomicBool = AtomicBool::new(false);

        struct OnStack;

        impl Drop for OnStack {
            fn drop(&mut self) {
                DROPPED.store(true, Ordering::Relaxed);
            }
        }

        extern "C-unwind" fn suspends(_args: *const u8) {
            let _guard = OnStack;
            yield_current();
            RESUMED.store(true, Ordering::Relaxed);
        }

        let mut handle = CoroutineHandle::new(suspends as *const u8, std::ptr::null());
        assert!(handle.resume());
        assert!(!DROPPED.load(Ordering::Relaxed));
        drop(handle);
        assert!(DROPPED.load(Ordering::Relaxed));
        assert!(!RESUMED.load(Ordering::Relaxed));
    }

    #[cfg(unix)]
    #[test]
    fn dropping_a_coroutine_with_a_replacement_panic_aborts() {
        use std::os::unix::process::ExitStatusExt;

        const CHILD: &str = "MESH_TEST_COROUTINE_DROP_PANIC";
        if std::env::var_os(CHILD).is_some() {
            extern "C-unwind" fn suspends_then_panics(_args: *const u8) {
                let _ = catch_unwind(AssertUnwindSafe(yield_current));
                panic!("replacement panic during coroutine cancellation");
            }

            let mut handle =
                CoroutineHandle::new(suspends_then_panics as *const u8, std::ptr::null());
            assert!(handle.resume());
            drop(handle);
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "actor::stack::tests::dropping_a_coroutine_with_a_replacement_panic_aborts",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .expect("run cancellation panic probe");
        assert_eq!(output.status.signal(), Some(libc::SIGABRT), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("replacement panic during coroutine cancellation"),
            "{stderr}"
        );
        assert!(
            stderr.contains("coroutine panic with #![no_std]"),
            "{stderr}"
        );
    }

    #[test]
    fn test_coroutine_runs_to_completion() {
        // Use a test-specific counter to avoid interference from concurrent tests.
        static COMPLETION_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn completion_entry(_args: *const u8) {
            COMPLETION_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        COMPLETION_COUNTER.store(0, Ordering::SeqCst);
        let mut handle = CoroutineHandle::new(completion_entry as *const u8, std::ptr::null());

        let yielded = handle.resume();
        // Simple function should run to completion without yielding
        assert!(!yielded, "simple function should complete without yielding");
        assert!(handle.done());
        assert_eq!(COMPLETION_COUNTER.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_coroutine_yield_and_resume() {
        // Use a test-specific counter to avoid interference from concurrent tests.
        static YIELD_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn yield_entry(_args: *const u8) {
            YIELD_COUNTER.fetch_add(1, Ordering::SeqCst);
            yield_current();
            YIELD_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        YIELD_COUNTER.store(0, Ordering::SeqCst);
        let mut handle = CoroutineHandle::new(yield_entry as *const u8, std::ptr::null());

        // First resume: runs to yield point
        let yielded = handle.resume();
        assert!(yielded);
        assert!(!handle.done());
        assert_eq!(YIELD_COUNTER.load(Ordering::SeqCst), 1);

        // Second resume: completes
        let yielded = handle.resume();
        assert!(!yielded);
        assert!(handle.done());
        assert_eq!(YIELD_COUNTER.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_current_pid_thread_local() {
        assert!(get_current_pid().is_none());
        let pid = ProcessId::next();
        set_current_pid(pid);
        assert_eq!(get_current_pid().unwrap(), pid);
        clear_current_pid();
        assert!(get_current_pid().is_none());
    }
}
