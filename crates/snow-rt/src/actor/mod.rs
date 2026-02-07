//! Actor runtime module for Snow.
//!
//! Provides the core actor infrastructure: Process Control Blocks, M:N
//! work-stealing scheduler, and stackful coroutine management via corosensei.
//!
//! ## Architecture
//!
//! Snow actors are lightweight processes multiplexed across OS threads:
//!
//! - **Process** (`process.rs`): The PCB holding PID, state, priority,
//!   reductions, mailbox, links, and terminate callback.
//! - **Scheduler** (`scheduler.rs`): M:N work-stealing scheduler using
//!   crossbeam-deque for load distribution across CPU cores.
//! - **Stack** (`stack.rs`): Corosensei-based stackful coroutines with
//!   64 KiB stacks for cooperative preemption via reduction counting.
//!
//! ## extern "C" ABI
//!
//! The following functions form the actor runtime ABI called by compiled
//! Snow programs:
//!
//! - `snow_rt_init_actor(num_schedulers)` -- initialize the scheduler
//! - `snow_actor_spawn(fn_ptr, args, args_size, priority)` -- spawn an actor
//! - `snow_actor_self()` -- get current actor's PID
//! - `snow_reduction_check()` -- decrement reductions, yield if exhausted

pub mod heap;
pub mod process;
pub mod scheduler;
pub mod stack;

pub use heap::{ActorHeap, MessageBuffer};
pub use process::{
    ExitReason, Message, Priority, Process, ProcessId, ProcessState, TerminateCallback,
    DEFAULT_REDUCTIONS, DEFAULT_STACK_SIZE,
};
pub use scheduler::Scheduler;
pub use stack::CoroutineHandle;

use std::sync::OnceLock;

use parking_lot::Mutex;

// ---------------------------------------------------------------------------
// Global scheduler instance
// ---------------------------------------------------------------------------

/// The global scheduler, initialized by `snow_rt_init_actor()`.
pub(crate) static GLOBAL_SCHEDULER: OnceLock<Mutex<Scheduler>> = OnceLock::new();

/// Get a reference to the global scheduler.
///
/// Panics if the scheduler has not been initialized via `snow_rt_init_actor()`.
fn global_scheduler() -> &'static Mutex<Scheduler> {
    GLOBAL_SCHEDULER
        .get()
        .expect("actor scheduler not initialized -- call snow_rt_init_actor() first")
}

// ---------------------------------------------------------------------------
// extern "C" ABI functions
// ---------------------------------------------------------------------------

/// Initialize the actor scheduler.
///
/// Must be called before any `snow_actor_spawn()` calls. Sets up the global
/// scheduler with the specified number of worker threads.
///
/// If `num_schedulers` is 0, defaults to the number of available CPU cores.
///
/// This function is idempotent -- subsequent calls are no-ops.
#[no_mangle]
pub extern "C" fn snow_rt_init_actor(num_schedulers: u32) {
    GLOBAL_SCHEDULER.get_or_init(|| Mutex::new(Scheduler::new(num_schedulers)));
}

/// Spawn a new actor process.
///
/// The actor will run `fn_ptr(args)` on a worker thread. The entry function
/// must have the signature `extern "C" fn(args: *const u8)`.
///
/// Returns the PID of the new actor as a `u64`.
///
/// - `fn_ptr`: pointer to the actor's entry function
/// - `args`: pointer to the actor's arguments (opaque bytes)
/// - `args_size`: size of the arguments in bytes
/// - `priority`: 0 = High, 1 = Normal, 2 = Low
#[no_mangle]
pub extern "C" fn snow_actor_spawn(
    fn_ptr: *const u8,
    args: *const u8,
    args_size: u64,
    priority: u8,
) -> u64 {
    let sched = global_scheduler().lock();
    sched.spawn(fn_ptr, args, args_size, priority).as_u64()
}

/// Get the PID of the currently running actor.
///
/// Returns the PID as a `u64`. Returns `u64::MAX` if called outside of an
/// actor context (should not happen in compiled Snow programs).
#[no_mangle]
pub extern "C" fn snow_actor_self() -> u64 {
    stack::get_current_pid()
        .map(|pid| pid.as_u64())
        .unwrap_or(u64::MAX)
}

/// Decrement the current actor's reduction counter and yield if exhausted.
///
/// This function is inserted by the Snow compiler at loop back-edges and
/// function call sites. When the reduction counter reaches zero, the actor
/// yields its timeslice to the scheduler, which can then run other actors.
///
/// The reduction counter is reset to `DEFAULT_REDUCTIONS` (4000) after yield.
#[no_mangle]
pub extern "C" fn snow_reduction_check() {
    // Get the current actor's process from the process table.
    // We decrement a thread-local shadow counter to avoid locking on every
    // reduction check. The actual Process.reductions field is updated by
    // the scheduler after yield.
    thread_local! {
        static LOCAL_REDUCTIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(DEFAULT_REDUCTIONS) };
    }

    LOCAL_REDUCTIONS.with(|cell| {
        let remaining = cell.get();
        if remaining == 0 {
            cell.set(DEFAULT_REDUCTIONS);
            stack::yield_current();
        } else {
            cell.set(remaining - 1);
        }
    });
}
