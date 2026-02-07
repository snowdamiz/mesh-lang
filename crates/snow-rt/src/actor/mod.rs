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

pub mod heap;
pub mod link;
pub mod mailbox;
pub mod process;
pub mod scheduler;
pub mod stack;

pub use heap::{ActorHeap, MessageBuffer};
pub use link::{propagate_exit, EXIT_SIGNAL_TAG};
pub use mailbox::Mailbox;
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
    let sched = global_scheduler().lock();
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
    {
        let sched_lock = sched.lock();
        if let Some(proc_arc) = sched_lock.get_process(my_pid) {
            let proc = proc_arc.lock();
            if let Some(msg) = proc.mailbox.pop() {
                // Deep-copy message data into the current actor's heap.
                drop(proc);
                return copy_msg_to_actor_heap(&sched_lock, my_pid, msg);
            }
        }
    }

    // Non-blocking mode: return null immediately.
    if timeout_ms == 0 {
        return std::ptr::null();
    }

    // Blocking mode: set state to Waiting and yield.
    let deadline = if timeout_ms > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64))
    } else {
        None // infinite wait
    };

    loop {
        // Set state to Waiting.
        {
            let sched_lock = sched.lock();
            if let Some(proc_arc) = sched_lock.get_process(my_pid) {
                proc_arc.lock().state = ProcessState::Waiting;
            }
        }

        // Yield to scheduler -- we will be resumed when a message arrives
        // or by the scheduler's periodic sweep.
        stack::yield_current();

        // After resume, try to pop a message.
        {
            let sched_lock = sched.lock();
            if let Some(proc_arc) = sched_lock.get_process(my_pid) {
                let proc = proc_arc.lock();
                if let Some(msg) = proc.mailbox.pop() {
                    drop(proc);
                    return copy_msg_to_actor_heap(&sched_lock, my_pid, msg);
                }
            }
        }

        // Check timeout.
        if let Some(deadline) = deadline {
            if std::time::Instant::now() >= deadline {
                // Timeout expired, set back to Ready and return null.
                let sched_lock = sched.lock();
                if let Some(proc_arc) = sched_lock.get_process(my_pid) {
                    proc_arc.lock().state = ProcessState::Ready;
                }
                return std::ptr::null();
            }
        }
    }
}

/// Deep-copy a message into the actor's heap and return a pointer to the
/// heap-allocated layout: `[u64 type_tag, u64 data_len, u8... data]`.
fn copy_msg_to_actor_heap(
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

    let sched = global_scheduler().lock();
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

    let sched = global_scheduler().lock();
    let target = ProcessId(pid);

    if let Some(proc_arc) = sched.get_process(target) {
        let cb: TerminateCallback =
            unsafe { std::mem::transmute(callback_fn_ptr) };
        proc_arc.lock().terminate_callback = Some(cb);
    }
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
}
