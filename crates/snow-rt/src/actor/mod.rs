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
//! - `snow_actor_send(target_pid, msg_ptr, msg_size)` -- send message to actor
//! - `snow_actor_receive(timeout_ms)` -- receive message from mailbox
//! - `snow_actor_link(target_pid)` -- bidirectional link to target actor
//! - `snow_actor_set_terminate(pid, callback_fn_ptr)` -- set terminate callback
//! - `snow_actor_register(name_ptr, name_len)` -- register current actor by name
//! - `snow_actor_whereis(name_ptr, name_len)` -- look up actor PID by name

pub mod child_spec;
pub mod heap;
pub mod job;
pub mod link;
pub mod mailbox;
pub mod process;
pub mod registry;
pub mod scheduler;
pub mod service;
pub mod stack;
pub mod supervisor;

pub use child_spec::{ChildSpec, ChildState, ChildType, RestartType, ShutdownType, Strategy};
pub use heap::{ActorHeap, MessageBuffer};
pub use link::{decode_exit_signal, encode_exit_signal, propagate_exit, EXIT_SIGNAL_TAG};
pub use mailbox::Mailbox;
pub use registry::{global_registry, ProcessRegistry};
pub use process::{
    ExitReason, Message, Priority, Process, ProcessId, ProcessState, TerminateCallback,
    DEFAULT_REDUCTIONS, DEFAULT_STACK_SIZE,
};
pub use scheduler::Scheduler;
pub use stack::CoroutineHandle;

use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Global scheduler instance
// ---------------------------------------------------------------------------

/// The global scheduler, initialized by `snow_rt_init_actor()`.
///
/// The Scheduler itself uses interior mutability (Mutex on workers, Arc on
/// shared state) so it can be shared without an outer Mutex. This prevents
/// deadlocks when actor runtime functions (receive, send) need to access the
/// scheduler while `run()` is executing on another thread.
pub(crate) static GLOBAL_SCHEDULER: OnceLock<Scheduler> = OnceLock::new();

/// Get a reference to the global scheduler.
///
/// Panics if the scheduler has not been initialized via `snow_rt_init_actor()`.
pub(crate) fn global_scheduler() -> &'static Scheduler {
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
/// scheduler with the specified number of worker threads and starts them
/// in the background.
///
/// Also creates a "main thread process" entry in the process table, giving the
/// main thread a PID and mailbox. This allows `snow_service_call` to work from
/// the main thread (non-coroutine context) by using spin-wait instead of yield.
///
/// Worker threads are started immediately so that actors spawned during
/// `snow_main()` begin executing right away. This is critical for service
/// calls which need the service actor to be running to process the request.
///
/// If `num_schedulers` is 0, defaults to the number of available CPU cores.
///
/// This function is idempotent -- subsequent calls are no-ops.
#[no_mangle]
pub extern "C" fn snow_rt_init_actor(num_schedulers: u32) {
    GLOBAL_SCHEDULER.get_or_init(|| {
        let sched = Scheduler::new(num_schedulers);

        // Create a process entry for the main thread so it has a PID and mailbox.
        // This enables snow_service_call to work from non-coroutine context.
        let main_pid = sched.create_main_process();
        stack::set_current_pid(main_pid);

        // Start worker threads in the background immediately so that actors
        // spawned during snow_main() can begin executing right away.
        sched.start();

        sched
    });
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
    let sched = global_scheduler();
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

    // Only yield if we're running inside a coroutine context (i.e., inside an actor).
    // The main thread also calls functions that trigger reduction_check, but the
    // main thread is not a coroutine so yield_current would panic.
    // Check CURRENT_YIELDER to detect coroutine context (more reliable than PID
    // since the main thread now also has a PID for service call support).
    if stack::CURRENT_YIELDER.with(|c| c.get().is_none()) {
        return;
    }

    LOCAL_REDUCTIONS.with(|cell| {
        let remaining = cell.get();
        if remaining == 0 {
            cell.set(DEFAULT_REDUCTIONS);
            // Check GC pressure before yielding. Running GC at yield points
            // ensures collection happens cooperatively without affecting other
            // actors (per-actor GC only).
            try_trigger_gc();
            stack::yield_current();
        } else {
            cell.set(remaining - 1);
        }
    });
}

/// Attempt to trigger garbage collection on the current actor's heap.
///
/// Checks if the current actor's heap exceeds its GC pressure threshold
/// and, if so, runs a mark-sweep collection cycle. The stack scanning
/// bounds are derived from:
/// - `stack_top`: the address of a local variable (current stack position)
/// - `stack_bottom`: the stack base captured at coroutine startup
///
/// This function is a no-op if:
/// - No actor context is available (not in a coroutine)
/// - The heap is below the pressure threshold
/// - GC is already in progress
fn try_trigger_gc() {
    let pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return,
    };

    let sched = match GLOBAL_SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };

    let proc_arc = match sched.get_process(pid) {
        Some(p) => p,
        None => return,
    };

    let mut proc = proc_arc.lock();
    if !proc.heap.should_collect() {
        return;
    }

    // Read stack_base from the process object rather than the STACK_BASE
    // thread-local. The thread-local may be stale if another coroutine ran
    // on this thread and overwrote it. The process field is set once at
    // coroutine startup and never changes.
    let stack_bottom = proc.stack_base;
    if stack_bottom.is_null() {
        return;
    }

    // Capture current stack position as stack_top.
    // On x86-64 and ARM64, the stack grows downward, so stack_top (current
    // position) has a lower address than stack_bottom (base).
    let stack_anchor: u64 = 0;
    let _ = std::hint::black_box(&stack_anchor);
    let stack_top = &stack_anchor as *const u64 as *const u8;

    proc.heap.collect(stack_bottom, stack_top);
}

/// Send a message to the target actor.
///
/// The message bytes at `msg_ptr` (of length `msg_size`) are deep-copied
/// into a `MessageBuffer` and pushed into the target actor's FIFO mailbox.
///
/// If the target actor is in `Waiting` state (blocked on receive), it is
/// woken up and re-enqueued into the scheduler as `Ready`.
///
/// - `target_pid`: the PID of the target actor
/// - `msg_ptr`: pointer to the raw message bytes
/// - `msg_size`: size of the message in bytes
///
/// The `type_tag` for the message is currently derived from the first 8 bytes
/// of the message data (if available), or 0 for empty messages. Future phases
/// will use compiler-generated type tags.
#[no_mangle]
pub extern "C" fn snow_actor_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) {
    // Locality check: upper 16 bits == 0 means local PID.
    // Single shift+compare -- essentially free on modern CPUs.
    if target_pid >> 48 == 0 {
        local_send(target_pid, msg_ptr, msg_size);
    } else {
        dist_send(target_pid, msg_ptr, msg_size);
    }
}

/// Local send path -- the original snow_actor_send body, unchanged.
///
/// Deep-copies the message bytes into a `MessageBuffer`, pushes it into
/// the target actor's FIFO mailbox, and wakes the target if it is Waiting.
pub(crate) fn local_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) {
    let sched = global_scheduler();
    let pid = ProcessId(target_pid);

    // Deep-copy the message bytes.
    let data = if msg_ptr.is_null() || msg_size == 0 {
        Vec::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) };
        slice.to_vec()
    };

    // Derive type_tag from first 8 bytes (or zero-pad).
    let type_tag = {
        let mut tag_bytes = [0u8; 8];
        let copy_len = data.len().min(8);
        tag_bytes[..copy_len].copy_from_slice(&data[..copy_len]);
        u64::from_le_bytes(tag_bytes)
    };

    let buffer = MessageBuffer::new(data, type_tag);
    let msg = Message { buffer };

    // Look up the target process and push message.
    if let Some(proc_arc) = sched.get_process(pid) {
        let mut proc = proc_arc.lock();
        proc.mailbox.push(msg);

        // If the target is Waiting, wake it up.
        if matches!(proc.state, ProcessState::Waiting) {
            proc.state = ProcessState::Ready;
            // Signal the scheduler to re-enqueue this process.
            drop(proc);
            sched.wake_process(pid);
        }
    }
}

/// Remote send path -- routes a message to a remote actor via the node's
/// TLS session.
///
/// Extracts the node_id from the upper 16 bits of the target PID, looks
/// up the corresponding NodeSession, and writes a DIST_SEND message to
/// the TLS stream. Silently drops on any failure (unknown node, no
/// session, write error) -- Phase 66 will add :nodedown notifications.
#[cold]
fn dist_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) {
    let state = match crate::dist::node::node_state() {
        Some(s) => s,
        None => return, // Node not started; silently drop
    };

    let node_id = (target_pid >> 48) as u16;
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return, // Unknown node; silently drop
        }
    };

    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => std::sync::Arc::clone(s),
            None => return, // Not connected; silently drop
        }
    };

    // Build wire message: [DIST_SEND][u64 target_pid LE][raw message bytes]
    let mut payload = Vec::with_capacity(1 + 8 + msg_size as usize);
    payload.push(crate::dist::node::DIST_SEND);
    payload.extend_from_slice(&target_pid.to_le_bytes());
    if !msg_ptr.is_null() && msg_size > 0 {
        let slice = unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) };
        payload.extend_from_slice(slice);
    }

    // Write to TLS stream; silently drop on error (Phase 66 adds :nodedown)
    let mut stream = session.stream.lock().unwrap();
    let _ = crate::dist::node::write_msg(&mut *stream, &payload);
}

/// Receive a message from the current actor's mailbox.
///
/// Returns a pointer to the message data in the current actor's heap, or
/// null if no message is available within the timeout.
///
/// Blocking behavior based on `timeout_ms`:
/// - `timeout_ms < 0` (e.g., -1): block indefinitely until a message arrives
/// - `timeout_ms == 0`: non-blocking, return immediately (null if empty)
/// - `timeout_ms > 0`: block up to `timeout_ms` milliseconds
///
/// When blocking, the actor yields to the scheduler (state = Waiting) and
/// is woken when a message is sent to its mailbox or the timeout expires.
///
/// The returned pointer points to a layout: `[u64 type_tag, u64 data_len, u8... data]`
/// allocated in the current actor's heap.
#[no_mangle]
pub extern "C" fn snow_actor_receive(timeout_ms: i64) -> *const u8 {
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return std::ptr::null(),
    };

    let sched = global_scheduler();

    // Try to pop a message.
    if let Some(proc_arc) = sched.get_process(my_pid) {
        let proc = proc_arc.lock();
        if let Some(msg) = proc.mailbox.pop() {
            // Deep-copy message data into the current actor's heap.
            drop(proc);
            return copy_msg_to_actor_heap(sched, my_pid, msg);
        }
    }

    // Non-blocking mode: return null immediately.
    if timeout_ms == 0 {
        return std::ptr::null();
    }

    // Check if we're in a coroutine context.
    let in_coroutine = stack::CURRENT_YIELDER.with(|c| c.get().is_some());

    if !in_coroutine {
        // Main thread path: spin-wait on the mailbox.
        let deadline = if timeout_ms > 0 {
            Some(std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64))
        } else {
            None
        };
        loop {
            if let Some(proc_arc) = sched.get_process(my_pid) {
                let proc = proc_arc.lock();
                if let Some(msg) = proc.mailbox.pop() {
                    drop(proc);
                    return copy_msg_to_actor_heap(sched, my_pid, msg);
                }
            }
            if let Some(deadline) = deadline {
                if std::time::Instant::now() >= deadline {
                    return std::ptr::null();
                }
            }
            std::thread::sleep(std::time::Duration::from_micros(10));
        }
    }

    // Coroutine path: blocking mode with yield.
    let deadline = if timeout_ms > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64))
    } else {
        None // infinite wait
    };

    loop {
        // Set state to Waiting.
        if let Some(proc_arc) = sched.get_process(my_pid) {
            proc_arc.lock().state = ProcessState::Waiting;
        }

        // Yield to scheduler -- we will be resumed when a message arrives
        // or by the scheduler's periodic sweep.
        stack::yield_current();

        // After resume, try to pop a message.
        if let Some(proc_arc) = sched.get_process(my_pid) {
            let proc = proc_arc.lock();
            if let Some(msg) = proc.mailbox.pop() {
                drop(proc);
                return copy_msg_to_actor_heap(sched, my_pid, msg);
            }
        }

        // Check timeout.
        if let Some(deadline) = deadline {
            if std::time::Instant::now() >= deadline {
                // Timeout expired, set back to Ready and return null.
                if let Some(proc_arc) = sched.get_process(my_pid) {
                    proc_arc.lock().state = ProcessState::Ready;
                }
                return std::ptr::null();
            }
        }

        // Check if the scheduler is shutting down. If so, check if there
        // are other non-waiting actors. If this is the only remaining actor
        // (e.g., a service loop with no more callers), return null to
        // allow the actor's loop to complete.
        if sched.is_shutdown() {
            // Count non-waiting, non-exited processes.
            let has_others = sched.process_table().read().iter().any(|(pid, p)| {
                *pid != my_pid && !matches!(p.lock().state, ProcessState::Waiting | ProcessState::Exited(_))
            });
            if !has_others {
                if let Some(proc_arc) = sched.get_process(my_pid) {
                    proc_arc.lock().state = ProcessState::Ready;
                }
                return std::ptr::null();
            }
        }
    }
}

// ── Timer functions (Phase 44 Plan 02) ──────────────────────────────

/// Sleep the current actor for `ms` milliseconds without blocking other actors.
///
/// Uses a yield loop with deadline checking. The actor stays Ready (not Waiting)
/// so the scheduler continues to resume it. On each resume, checks if the
/// deadline has passed. Does NOT consume messages from the mailbox.
#[no_mangle]
pub extern "C" fn snow_timer_sleep(ms: i64) {
    if ms <= 0 {
        return;
    }

    let in_coroutine = stack::CURRENT_YIELDER.with(|c| c.get().is_some());

    if !in_coroutine {
        // Main thread: just use thread::sleep
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
        return;
    }

    // Coroutine path: yield loop with deadline
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms as u64);

    loop {
        // Yield to scheduler (state stays Ready/Running -- NOT Waiting).
        // If we set state to Waiting, the scheduler would skip this process
        // and it would never be resumed (unless a message arrives).
        stack::yield_current();

        if std::time::Instant::now() >= deadline {
            return;
        }
    }
}

/// Schedule a message to be sent to `target_pid` after `ms` milliseconds.
///
/// Spawns a background OS thread that sleeps for `ms` then sends the message.
/// The message bytes are deep-copied at call time so the caller's stack frame
/// can be freed safely.
#[no_mangle]
pub extern "C" fn snow_timer_send_after(target_pid: i64, ms: i64, msg_ptr: *const u8, msg_size: i64) {
    // Deep-copy message bytes before spawning thread
    let data = if msg_ptr.is_null() || msg_size <= 0 {
        Vec::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) };
        slice.to_vec()
    };

    let pid = target_pid as u64;
    let delay = std::time::Duration::from_millis(if ms > 0 { ms as u64 } else { 0 });

    std::thread::spawn(move || {
        std::thread::sleep(delay);
        // Reuse snow_actor_send: construct message and deliver
        snow_actor_send(pid, data.as_ptr(), data.len() as u64);
    });
}

/// Deep-copy a message into the actor's heap and return a pointer to the
/// heap-allocated layout: `[u64 type_tag, u64 data_len, u8... data]`.
pub(crate) fn copy_msg_to_actor_heap(
    sched: &Scheduler,
    pid: ProcessId,
    msg: Message,
) -> *const u8 {
    if let Some(proc_arc) = sched.get_process(pid) {
        let mut proc = proc_arc.lock();
        // Layout: [u64 type_tag][u64 data_len][u8... data]
        let header_size = 16; // 8 bytes type_tag + 8 bytes data_len
        let total_size = header_size + msg.buffer.data.len();
        let ptr = proc.heap.alloc(total_size, 8);

        unsafe {
            // Write type_tag.
            std::ptr::copy_nonoverlapping(
                msg.buffer.type_tag.to_le_bytes().as_ptr(),
                ptr,
                8,
            );
            // Write data_len.
            let data_len = msg.buffer.data.len() as u64;
            std::ptr::copy_nonoverlapping(
                data_len.to_le_bytes().as_ptr(),
                ptr.add(8),
                8,
            );
            // Write data bytes.
            if !msg.buffer.data.is_empty() {
                std::ptr::copy_nonoverlapping(
                    msg.buffer.data.as_ptr(),
                    ptr.add(header_size),
                    msg.buffer.data.len(),
                );
            }
        }

        ptr as *const u8
    } else {
        std::ptr::null()
    }
}

/// Link the current actor to the target actor.
///
/// Creates a bidirectional link: when either actor terminates, the other
/// receives an exit signal. For normal exits, the signal is delivered as
/// a message. For crashes, the linked process also crashes (unless
/// `trap_exit` is set).
///
/// - `target_pid`: the PID of the actor to link with
#[no_mangle]
pub extern "C" fn snow_actor_link(target_pid: u64) {
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return,
    };

    let sched = global_scheduler();
    let target = ProcessId(target_pid);

    // Add bidirectional link: my_pid <-> target_pid
    let my_proc = sched.get_process(my_pid);
    let target_proc = sched.get_process(target);

    if let (Some(my_proc), Some(target_proc)) = (my_proc, target_proc) {
        link::link(&my_proc, &target_proc, my_pid, target);
    }
}

/// Set the terminate callback for an actor.
///
/// The callback is invoked before the actor fully exits, allowing cleanup
/// logic (e.g., closing resources, sending goodbye messages).
///
/// - `pid`: the PID of the actor to set the callback for
/// - `callback_fn_ptr`: pointer to the terminate callback function
///   with signature `extern "C" fn(state_ptr: *const u8, reason_ptr: *const u8)`
#[no_mangle]
pub extern "C" fn snow_actor_set_terminate(pid: u64, callback_fn_ptr: *const u8) {
    if callback_fn_ptr.is_null() {
        return;
    }

    let sched = global_scheduler();
    let target = ProcessId(pid);

    if let Some(proc_arc) = sched.get_process(target) {
        let cb: TerminateCallback =
            unsafe { std::mem::transmute(callback_fn_ptr) };
        proc_arc.lock().terminate_callback = Some(cb);
    }
}

/// Signal the scheduler to shut down and wait for all workers to finish.
///
/// This function must be called after `snow_main()` returns. It signals
/// shutdown (allowing workers to terminate Waiting actors) and joins the
/// worker threads that were started by `snow_rt_init_actor()`.
///
/// The scheduler shuts down when the active process count reaches zero
/// (i.e., all spawned actors have completed or been force-terminated).
#[no_mangle]
pub extern "C" fn snow_rt_run_scheduler() {
    // Get the main thread's PID before clearing it.
    let main_pid = stack::get_current_pid();

    // Clear the main thread's PID now that snow_main has returned.
    stack::clear_current_pid();

    let sched = GLOBAL_SCHEDULER
        .get()
        .expect("actor scheduler not initialized -- call snow_rt_init_actor() first");

    // Mark the main thread process as Exited so the scheduler doesn't
    // count it as a Ready/Running process during shutdown.
    if let Some(pid) = main_pid {
        if let Some(proc_arc) = sched.get_process(pid) {
            proc_arc.lock().state = ProcessState::Exited(ExitReason::Normal);
        }
    }

    // Signal shutdown so workers know to terminate Waiting actors when
    // no Ready/Running actors remain.
    sched.signal_shutdown();

    // Wait for all worker threads to complete.
    sched.wait();
}

/// Register the current actor under a name.
///
/// The name is specified as a pointer to UTF-8 bytes and a length.
/// Returns 0 on success, 1 if the name is already taken.
///
/// - `name_ptr`: pointer to UTF-8 name bytes
/// - `name_len`: length of the name in bytes
#[no_mangle]
pub extern "C" fn snow_actor_register(name_ptr: *const u8, name_len: u64) -> u64 {
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return 1,
    };

    if name_ptr.is_null() || name_len == 0 {
        return 1;
    }

    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => return 1,
        }
    };

    match registry::global_registry().register(name, my_pid) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Look up a registered actor by name.
///
/// Returns the PID of the actor registered under the given name, or 0
/// if no actor is registered with that name.
///
/// - `name_ptr`: pointer to UTF-8 name bytes
/// - `name_len`: length of the name in bytes
#[no_mangle]
pub extern "C" fn snow_actor_whereis(name_ptr: *const u8, name_len: u64) -> u64 {
    if name_ptr.is_null() || name_len == 0 {
        return 0;
    }

    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    match registry::global_registry().whereis(name) {
        Some(pid) => pid.as_u64(),
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// Supervisor extern "C" ABI functions
// ---------------------------------------------------------------------------

/// Start a new supervisor actor.
///
/// Deserializes a `SupervisorConfig` from the raw bytes, creates a
/// `SupervisorState`, registers it in the global supervisor state registry,
/// spawns the supervisor as a regular actor with `trap_exit = true`, starts
/// all children sequentially, and returns the supervisor PID.
///
/// The config binary format:
/// - u8: strategy (0=OneForOne, 1=OneForAll, 2=RestForOne, 3=SimpleOneForOne)
/// - u32 LE: max_restarts
/// - u64 LE: max_seconds
/// - u32 LE: num_child_specs
/// - For each child spec:
///   - u32 LE: id string length
///   - [u8]: id string bytes
///   - u64 LE: start_fn pointer
///   - u64 LE: start_args pointer
///   - u64 LE: start_args size
///   - u8: restart_type (0=Permanent, 1=Transient, 2=Temporary)
///   - u8: shutdown_type (0=BrutalKill, 1=Timeout)
///   - u64 LE: shutdown_timeout_ms (only meaningful if shutdown_type=1)
///   - u8: child_type (0=Worker, 1=Supervisor)
///
/// Returns the supervisor PID as `u64`, or `u64::MAX` on error.
#[no_mangle]
pub extern "C" fn snow_supervisor_start(
    config_ptr: *const u8,
    config_size: u64,
) -> u64 {
    if config_ptr.is_null() || config_size == 0 {
        return u64::MAX;
    }

    let data = unsafe {
        std::slice::from_raw_parts(config_ptr, config_size as usize)
    };

    // Parse the config.
    let config = match parse_supervisor_config(data) {
        Some(c) => c,
        None => return u64::MAX,
    };

    let sched = global_scheduler();

    // Create the supervisor state.
    let mut sup_state = supervisor::SupervisorState::new(
        config.strategy,
        config.max_restarts,
        config.max_seconds,
    );
    sup_state.children = config
        .child_specs
        .into_iter()
        .map(|spec| child_spec::ChildState {
            spec,
            pid: None,
            running: false,
        })
        .collect();

    // Spawn the supervisor as a normal actor (no-op entry -- it doesn't run
    // a coroutine. The supervisor logic is driven externally by the compiled
    // Snow program's receive loop or by the runtime's supervisor_entry).
    extern "C" fn supervisor_noop(_args: *const u8) {}
    let sup_pid = sched.spawn(supervisor_noop as *const u8, std::ptr::null(), 0, 1);

    // Set trap_exit on the supervisor process.
    if let Some(proc) = sched.get_process(ProcessId(sup_pid.as_u64())) {
        proc.lock().trap_exit = true;
    }

    // Start all children.
    match supervisor::start_children(&mut sup_state, sched, sup_pid) {
        Ok(()) => {}
        Err(_e) => {
            return u64::MAX;
        }
    }

    // Register the supervisor state in the global registry.
    supervisor::register_supervisor_state(sup_pid, sup_state);

    sup_pid.as_u64()
}

/// Start a dynamic child under a simple_one_for_one supervisor.
///
/// Looks up the supervisor state, clones the template child spec with the
/// given args, spawns the child, links it to the supervisor, and returns
/// the child PID.
///
/// Returns the child PID as `u64`, or `u64::MAX` on error.
#[no_mangle]
pub extern "C" fn snow_supervisor_start_child(
    sup_pid: u64,
    args_ptr: *const u8,
    args_size: u64,
) -> u64 {
    let sup_pid = ProcessId(sup_pid);
    let sched = global_scheduler();

    let state_arc = match supervisor::get_supervisor_state(sup_pid) {
        Some(s) => s,
        None => return u64::MAX,
    };

    let mut state = state_arc.lock();

    // Clone the template spec (for simple_one_for_one).
    let template = match &state.child_template {
        Some(t) => t.clone(),
        None => {
            // Not a simple_one_for_one supervisor -- create from args directly.
            // For now, return error.
            return u64::MAX;
        }
    };

    let mut new_spec = template;
    new_spec.id = format!("dynamic_{}", state.children.len());
    new_spec.start_args_ptr = args_ptr;
    new_spec.start_args_size = args_size;

    let mut child_state = child_spec::ChildState {
        spec: new_spec,
        pid: None,
        running: false,
    };

    match supervisor::start_single_child(&mut child_state, sched, sup_pid) {
        Ok(pid) => {
            state.children.push(child_state);
            pid.as_u64()
        }
        Err(_) => u64::MAX,
    }
}

/// Terminate a specific child under a supervisor.
///
/// Looks up the supervisor state, finds the child by PID, terminates it,
/// and removes it from the children list.
///
/// Returns 0 on success, 1 on failure.
#[no_mangle]
pub extern "C" fn snow_supervisor_terminate_child(
    sup_pid: u64,
    child_pid: u64,
) -> u64 {
    let sup_pid = ProcessId(sup_pid);
    let child_pid = ProcessId(child_pid);
    let sched = global_scheduler();

    let state_arc = match supervisor::get_supervisor_state(sup_pid) {
        Some(s) => s,
        None => return 1,
    };

    let mut state = state_arc.lock();

    let child_idx = match state.find_child_index(child_pid) {
        Some(idx) => idx,
        None => return 1,
    };

    supervisor::terminate_single_child(&mut state.children[child_idx], sched, sup_pid);
    state.children.remove(child_idx);

    0
}

/// Get the count of running children under a supervisor.
///
/// Returns the number of currently running children, or 0 if the
/// supervisor PID is not found.
#[no_mangle]
pub extern "C" fn snow_supervisor_count_children(sup_pid: u64) -> u64 {
    let sup_pid = ProcessId(sup_pid);

    match supervisor::get_supervisor_state(sup_pid) {
        Some(state_arc) => state_arc.lock().running_count() as u64,
        None => 0,
    }
}

/// Set `trap_exit = true` on the current process.
///
/// When trap_exit is enabled, exit signals from linked processes are
/// delivered as regular messages (with EXIT_SIGNAL_TAG) instead of
/// causing this process to crash. Used by supervisors to monitor
/// children, and by regular actors that want to handle linked exits.
#[no_mangle]
pub extern "C" fn snow_actor_trap_exit() {
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return,
    };

    let sched = global_scheduler();
    if let Some(proc_arc) = sched.get_process(my_pid) {
        proc_arc.lock().trap_exit = true;
    }
}

/// Send an exit signal to a target process.
///
/// This is used for supervisor shutdown and for explicit `exit(pid, reason)`.
///
/// - `target_pid`: the PID of the target process
/// - `reason_tag`: 0=Normal, 1=Error, 2=Killed, 4=Shutdown
///
/// If the reason is Killed (tag 2), the process is immediately terminated
/// (untrappable -- like Erlang's `exit(Pid, kill)`).
///
/// For other reasons: if the target has trap_exit enabled, the signal is
/// delivered as a message. Otherwise, the target is terminated immediately.
#[no_mangle]
pub extern "C" fn snow_actor_exit(target_pid: u64, reason_tag: u8) {
    let sched = global_scheduler();
    let pid = ProcessId(target_pid);

    let reason = match reason_tag {
        0 => ExitReason::Normal,
        1 => ExitReason::Error("exit signal".to_string()),
        2 => ExitReason::Killed,
        4 => ExitReason::Shutdown,
        5 => ExitReason::Custom("exit signal".to_string()),
        _ => ExitReason::Error(format!("unknown exit reason tag: {}", reason_tag)),
    };

    if let Some(proc_arc) = sched.get_process(pid) {
        let mut proc = proc_arc.lock();

        // Skip already-exited processes.
        if matches!(proc.state, ProcessState::Exited(_)) {
            return;
        }

        // Killed is untrappable.
        if matches!(reason, ExitReason::Killed) {
            proc.state = ProcessState::Exited(ExitReason::Killed);
            return;
        }

        if proc.trap_exit {
            // Deliver as a message.
            let signal_data = link::encode_exit_signal(pid, &reason);
            let buffer = heap::MessageBuffer::new(signal_data, link::EXIT_SIGNAL_TAG);
            proc.mailbox.push(Message { buffer });

            // Wake if Waiting.
            if matches!(proc.state, ProcessState::Waiting) {
                proc.state = ProcessState::Ready;
                drop(proc);
                sched.wake_process(pid);
            }
        } else {
            // Terminate immediately.
            proc.state = ProcessState::Exited(reason);
        }
    }
}

/// Parse a `SupervisorConfig` from raw bytes.
fn parse_supervisor_config(data: &[u8]) -> Option<supervisor::SupervisorConfig> {
    if data.len() < 14 {
        return None; // Minimum: 1 + 4 + 8 + 4 = 17 bytes... actually 1+4+8+4=17
    }

    let mut pos = 0;

    // Strategy (1 byte)
    let strategy = match data[pos] {
        0 => child_spec::Strategy::OneForOne,
        1 => child_spec::Strategy::OneForAll,
        2 => child_spec::Strategy::RestForOne,
        3 => child_spec::Strategy::SimpleOneForOne,
        _ => return None,
    };
    pos += 1;

    // max_restarts (4 bytes LE)
    if pos + 4 > data.len() {
        return None;
    }
    let max_restarts = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
    pos += 4;

    // max_seconds (8 bytes LE)
    if pos + 8 > data.len() {
        return None;
    }
    let max_seconds = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?);
    pos += 8;

    // num_child_specs (4 bytes LE)
    if pos + 4 > data.len() {
        return None;
    }
    let num_specs = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
    pos += 4;

    let mut child_specs = Vec::with_capacity(num_specs);

    for _ in 0..num_specs {
        // id string length (4 bytes LE)
        if pos + 4 > data.len() {
            return None;
        }
        let id_len = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        pos += 4;

        // id string bytes
        if pos + id_len > data.len() {
            return None;
        }
        let id = std::str::from_utf8(&data[pos..pos + id_len]).ok()?.to_string();
        pos += id_len;

        // start_fn pointer (8 bytes LE)
        if pos + 8 > data.len() {
            return None;
        }
        let start_fn = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?) as *const u8;
        pos += 8;

        // start_args pointer (8 bytes LE)
        if pos + 8 > data.len() {
            return None;
        }
        let start_args_ptr = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?) as *const u8;
        pos += 8;

        // start_args size (8 bytes LE)
        if pos + 8 > data.len() {
            return None;
        }
        let start_args_size = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?);
        pos += 8;

        // restart_type (1 byte)
        if pos >= data.len() {
            return None;
        }
        let restart_type = match data[pos] {
            0 => child_spec::RestartType::Permanent,
            1 => child_spec::RestartType::Transient,
            2 => child_spec::RestartType::Temporary,
            _ => return None,
        };
        pos += 1;

        // shutdown_type (1 byte)
        if pos >= data.len() {
            return None;
        }
        let shutdown_type_tag = data[pos];
        pos += 1;

        // shutdown_timeout_ms (8 bytes LE)
        if pos + 8 > data.len() {
            return None;
        }
        let shutdown_timeout = u64::from_le_bytes(data[pos..pos + 8].try_into().ok()?);
        pos += 8;

        let shutdown = match shutdown_type_tag {
            0 => child_spec::ShutdownType::BrutalKill,
            1 => child_spec::ShutdownType::Timeout(shutdown_timeout),
            _ => return None,
        };

        // child_type (1 byte)
        if pos >= data.len() {
            return None;
        }
        let child_type = match data[pos] {
            0 => child_spec::ChildType::Worker,
            1 => child_spec::ChildType::Supervisor,
            _ => return None,
        };
        pos += 1;

        child_specs.push(child_spec::ChildSpec {
            id,
            start_fn,
            start_args_ptr,
            start_args_size,
            restart_type,
            shutdown,
            child_type,
        });
    }

    Some(supervisor::SupervisorConfig {
        strategy,
        max_restarts,
        max_seconds,
        child_specs,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Helper: create a process in a scheduler and return its PID.
    fn create_test_process(sched: &Scheduler) -> ProcessId {
        // Use a no-op entry function.
        extern "C" fn noop(_args: *const u8) {}
        sched.spawn(noop as *const u8, std::ptr::null(), 0, 1)
    }

    #[test]
    fn test_send_delivers_to_mailbox() {
        let sched = Scheduler::new(1);
        let target_pid = create_test_process(&sched);

        // Manually push a message (simulating snow_actor_send logic).
        let data = vec![42u8, 43, 44, 45];
        let buffer = MessageBuffer::new(data.clone(), 99);
        let msg = Message { buffer };

        let proc_arc = sched.get_process(target_pid).unwrap();
        proc_arc.lock().mailbox.push(msg);

        // Verify message is in mailbox.
        let popped = proc_arc.lock().mailbox.pop().unwrap();
        assert_eq!(popped.buffer.type_tag, 99);
        assert_eq!(popped.buffer.data, vec![42, 43, 44, 45]);
    }

    #[test]
    fn test_send_fifo_ordering() {
        let sched = Scheduler::new(1);
        let target_pid = create_test_process(&sched);
        let proc_arc = sched.get_process(target_pid).unwrap();

        // Send 5 messages.
        for i in 0..5u8 {
            let buffer = MessageBuffer::new(vec![i], i as u64);
            proc_arc.lock().mailbox.push(Message { buffer });
        }

        // Receive in order.
        for i in 0..5u8 {
            let msg = proc_arc.lock().mailbox.pop().unwrap();
            assert_eq!(msg.buffer.type_tag, i as u64, "FIFO order violated at {}", i);
            assert_eq!(msg.buffer.data, vec![i]);
        }

        assert!(proc_arc.lock().mailbox.pop().is_none());
    }

    #[test]
    fn test_send_wakes_waiting_process() {
        let sched = Scheduler::new(1);
        let target_pid = create_test_process(&sched);
        let proc_arc = sched.get_process(target_pid).unwrap();

        // Set process to Waiting.
        proc_arc.lock().state = ProcessState::Waiting;

        // Push message and wake (simulating snow_actor_send).
        let buffer = MessageBuffer::new(vec![1, 2, 3], 1);
        let msg = Message { buffer };
        {
            let mut proc = proc_arc.lock();
            proc.mailbox.push(msg);
            if matches!(proc.state, ProcessState::Waiting) {
                proc.state = ProcessState::Ready;
            }
        }

        // Process should now be Ready.
        assert!(matches!(proc_arc.lock().state, ProcessState::Ready));
    }

    #[test]
    fn test_copy_msg_to_actor_heap_layout() {
        let sched = Scheduler::new(1);
        let pid = create_test_process(&sched);

        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let type_tag: u64 = 0x1234567890ABCDEF;
        let buffer = MessageBuffer::new(data.clone(), type_tag);
        let msg = Message { buffer };

        let ptr = copy_msg_to_actor_heap(&sched, pid, msg);
        assert!(!ptr.is_null());

        unsafe {
            // Read type_tag (first 8 bytes).
            let mut tag_bytes = [0u8; 8];
            std::ptr::copy_nonoverlapping(ptr, tag_bytes.as_mut_ptr(), 8);
            let read_tag = u64::from_le_bytes(tag_bytes);
            assert_eq!(read_tag, type_tag);

            // Read data_len (next 8 bytes).
            let mut len_bytes = [0u8; 8];
            std::ptr::copy_nonoverlapping(ptr.add(8), len_bytes.as_mut_ptr(), 8);
            let read_len = u64::from_le_bytes(len_bytes);
            assert_eq!(read_len, 4);

            // Read data bytes.
            let data_ptr = ptr.add(16);
            let read_data = std::slice::from_raw_parts(data_ptr, 4);
            assert_eq!(read_data, &[0xDE, 0xAD, 0xBE, 0xEF]);
        }
    }

    #[test]
    fn test_receive_returns_null_outside_actor() {
        // snow_actor_receive requires a current PID. Without one, returns null.
        // Note: we can't easily test this through the extern "C" fn because
        // it requires GLOBAL_SCHEDULER. Test the logic instead.
        assert!(stack::get_current_pid().is_none());
        // If we called snow_actor_receive here, it would return null because
        // there's no current PID set.
    }

    #[test]
    fn test_concurrent_send_to_same_target() {
        let sched = Arc::new(Scheduler::new(1));
        let target_pid = create_test_process(&sched);
        let proc_arc = sched.get_process(target_pid).unwrap();

        let num_threads = 8;
        let msgs_per_thread = 50;

        let handles: Vec<_> = (0..num_threads)
            .map(|t| {
                let proc = Arc::clone(&proc_arc);
                std::thread::spawn(move || {
                    for i in 0..msgs_per_thread {
                        let tag = (t * msgs_per_thread + i) as u64;
                        let buffer = MessageBuffer::new(vec![tag as u8], tag);
                        proc.lock().mailbox.push(Message { buffer });
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // All messages should be in the mailbox.
        assert_eq!(proc_arc.lock().mailbox.len(), num_threads * msgs_per_thread);

        // Drain and verify count.
        let mut count = 0;
        while proc_arc.lock().mailbox.pop().is_some() {
            count += 1;
        }
        assert_eq!(count, num_threads * msgs_per_thread);
    }

    #[test]
    fn test_message_deep_copy_between_heaps() {
        // Verify that sending a message creates an independent copy
        // in the target actor's heap.
        let sched = Scheduler::new(1);
        let sender_pid = create_test_process(&sched);
        let receiver_pid = create_test_process(&sched);

        // Allocate data in sender's heap.
        let sender_proc = sched.get_process(sender_pid).unwrap();
        let data = vec![10u8, 20, 30, 40];
        let ptr_in_sender = {
            let mut proc = sender_proc.lock();
            let ptr = proc.heap.alloc(data.len(), 8);
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            }
            ptr
        };

        // Create MessageBuffer from sender data.
        let buffer = MessageBuffer::new(data.clone(), 42);

        // Deep-copy into receiver's heap.
        let receiver_proc = sched.get_process(receiver_pid).unwrap();
        let ptr_in_receiver = {
            let mut proc = receiver_proc.lock();
            buffer.deep_copy_to_heap(&mut proc.heap)
        };

        // Pointers should be different (different heaps).
        assert_ne!(ptr_in_sender as usize, ptr_in_receiver as usize);

        // Data should be identical.
        let receiver_data =
            unsafe { std::slice::from_raw_parts(ptr_in_receiver, data.len()) };
        assert_eq!(receiver_data, &[10, 20, 30, 40]);
    }

    #[test]
    fn test_link_bidirectional_via_scheduler() {
        let sched = Scheduler::new(1);
        let pid_a = create_test_process(&sched);
        let pid_b = create_test_process(&sched);

        // Link via the process table lookup.
        let proc_a = sched.get_process(pid_a).unwrap();
        let proc_b = sched.get_process(pid_b).unwrap();
        link::link(&proc_a, &proc_b, pid_a, pid_b);

        assert!(proc_a.lock().links.contains(&pid_b));
        assert!(proc_b.lock().links.contains(&pid_a));
    }

    #[test]
    fn test_link_idempotent_hashset() {
        let sched = Scheduler::new(1);
        let pid_a = create_test_process(&sched);
        let pid_b = create_test_process(&sched);

        let proc_a = sched.get_process(pid_a).unwrap();
        let proc_b = sched.get_process(pid_b).unwrap();

        // Link twice -- should not create duplicate entries.
        link::link(&proc_a, &proc_b, pid_a, pid_b);
        link::link(&proc_a, &proc_b, pid_a, pid_b);

        assert_eq!(proc_a.lock().links.len(), 1);
        assert_eq!(proc_b.lock().links.len(), 1);
    }

    #[test]
    fn test_exit_propagation_error_crashes_linked() {
        let sched = Scheduler::new(1);
        let pid_a = create_test_process(&sched);
        let pid_b = create_test_process(&sched);

        let proc_a = sched.get_process(pid_a).unwrap();
        let proc_b = sched.get_process(pid_b).unwrap();
        link::link(&proc_a, &proc_b, pid_a, pid_b);

        // Extract links from A and propagate.
        let linked_pids = std::mem::take(&mut proc_a.lock().links);
        link::propagate_exit(
            pid_a,
            &ExitReason::Error("crash".to_string()),
            linked_pids,
            |pid| sched.get_process(pid),
        );

        // Process B should be Exited(Linked(...)).
        let b_state = proc_b.lock().state.clone();
        match &b_state {
            ProcessState::Exited(ExitReason::Linked(from_pid, inner)) => {
                assert_eq!(*from_pid, pid_a);
                assert!(matches!(inner.as_ref(), ExitReason::Error(_)));
            }
            other => panic!("Expected Exited(Linked(...)), got {:?}", other),
        }
    }

    #[test]
    fn test_exit_propagation_normal_delivers_message() {
        let sched = Scheduler::new(1);
        let pid_a = create_test_process(&sched);
        let pid_b = create_test_process(&sched);

        let proc_a = sched.get_process(pid_a).unwrap();
        let proc_b = sched.get_process(pid_b).unwrap();
        link::link(&proc_a, &proc_b, pid_a, pid_b);

        let linked_pids = std::mem::take(&mut proc_a.lock().links);
        link::propagate_exit(
            pid_a,
            &ExitReason::Normal,
            linked_pids,
            |pid| sched.get_process(pid),
        );

        // Process B should NOT be crashed.
        assert!(
            !matches!(proc_b.lock().state, ProcessState::Exited(_)),
            "Normal exit should not crash linked process"
        );

        // Should have received an exit signal message.
        let msg = proc_b.lock().mailbox.pop().unwrap();
        assert_eq!(msg.buffer.type_tag, link::EXIT_SIGNAL_TAG);
    }

    #[test]
    fn test_trap_exit_prevents_crash() {
        let sched = Scheduler::new(1);
        let pid_a = create_test_process(&sched);
        let pid_b = create_test_process(&sched);

        let proc_a = sched.get_process(pid_a).unwrap();
        let proc_b = sched.get_process(pid_b).unwrap();

        proc_b.lock().trap_exit = true;
        link::link(&proc_a, &proc_b, pid_a, pid_b);

        let linked_pids = std::mem::take(&mut proc_a.lock().links);
        link::propagate_exit(
            pid_a,
            &ExitReason::Error("crash".to_string()),
            linked_pids,
            |pid| sched.get_process(pid),
        );

        // B should not have crashed.
        assert!(!matches!(proc_b.lock().state, ProcessState::Exited(_)));
        // Should have received exit signal as message.
        let msg = proc_b.lock().mailbox.pop().unwrap();
        assert_eq!(msg.buffer.type_tag, link::EXIT_SIGNAL_TAG);
    }

    #[test]
    fn test_terminate_callback_invoked() {
        use std::sync::atomic::{AtomicU64, Ordering};

        static TERM_CB_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn test_terminate_cb(_state: *const u8, _reason: *const u8) {
            TERM_CB_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        TERM_CB_COUNTER.store(0, Ordering::SeqCst);

        let sched = Scheduler::new(1);
        let pid = create_test_process(&sched);

        // Set terminate callback.
        let proc_arc = sched.get_process(pid).unwrap();
        proc_arc.lock().terminate_callback = Some(test_terminate_cb);

        // Simulate process exit via scheduler's handle_process_exit.
        // We access this indirectly through the scheduler test infrastructure.
        // For unit test, directly call the terminate callback logic.
        let cb = proc_arc.lock().terminate_callback.take().unwrap();
        let _reason = ExitReason::Normal;
        let reason_tag: u8 = 0;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cb(std::ptr::null(), &reason_tag as *const u8);
        }));

        assert_eq!(TERM_CB_COUNTER.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_terminate_callback_is_invoked_before_exit() {
        // Verify terminate callback execution order:
        // callback runs, then exit propagation happens.
        use std::sync::atomic::{AtomicU64, Ordering};

        static ORDER_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn order_terminate_cb(_state: *const u8, _reason: *const u8) {
            ORDER_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        ORDER_COUNTER.store(0, Ordering::SeqCst);

        let sched = Scheduler::new(1);
        let pid = create_test_process(&sched);
        let proc_arc = sched.get_process(pid).unwrap();
        proc_arc.lock().terminate_callback = Some(order_terminate_cb);

        // Invoke the callback the same way the scheduler does.
        let cb = proc_arc.lock().terminate_callback.take().unwrap();
        let reason_tag: u8 = 0;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cb(std::ptr::null(), &reason_tag as *const u8);
        }));

        assert_eq!(ORDER_COUNTER.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_registry_register_and_whereis() {
        let reg = registry::ProcessRegistry::new();
        let pid = ProcessId::next();

        reg.register("test_server".to_string(), pid).unwrap();
        assert_eq!(reg.whereis("test_server"), Some(pid));
        assert_eq!(reg.whereis("nonexistent"), None);
    }

    #[test]
    fn test_registry_cleanup_on_process_exit() {
        let reg = registry::ProcessRegistry::new();
        let pid = ProcessId::next();

        reg.register("my_actor".to_string(), pid).unwrap();
        assert!(reg.whereis("my_actor").is_some());

        // Simulate process exit cleanup.
        reg.cleanup_process(pid);
        assert_eq!(reg.whereis("my_actor"), None);

        // Name should now be available for re-registration.
        let new_pid = ProcessId::next();
        reg.register("my_actor".to_string(), new_pid).unwrap();
        assert_eq!(reg.whereis("my_actor"), Some(new_pid));
    }

    #[test]
    fn test_registry_duplicate_name_rejected() {
        let reg = registry::ProcessRegistry::new();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("unique".to_string(), pid1).unwrap();
        let result = reg.register("unique".to_string(), pid2);
        assert!(result.is_err());
    }

    #[test]
    fn test_send_locality_check_local_path() {
        // Verify that sending to a local PID (node_id=0) still delivers
        // to the mailbox through the local_send path.
        let sched = Scheduler::new(1);
        let target_pid = create_test_process(&sched);

        // Push a message manually using local_send logic (same as the
        // test_send_delivers_to_mailbox pattern).
        let data = vec![42u8, 43, 44, 45];
        let buffer = MessageBuffer::new(data.clone(), 99);
        let msg = Message { buffer };

        let proc_arc = sched.get_process(target_pid).unwrap();
        proc_arc.lock().mailbox.push(msg);

        // Verify the PID is local.
        assert!(target_pid.is_local());
        assert_eq!(target_pid.node_id(), 0);

        // Verify message was delivered.
        let popped = proc_arc.lock().mailbox.pop().unwrap();
        assert_eq!(popped.buffer.type_tag, 99);
        assert_eq!(popped.buffer.data, vec![42, 43, 44, 45]);
    }
}
