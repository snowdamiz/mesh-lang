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

pub mod process;

pub use process::{
    ExitReason, Message, Priority, Process, ProcessId, ProcessState, TerminateCallback,
    DEFAULT_REDUCTIONS, DEFAULT_STACK_SIZE,
};
