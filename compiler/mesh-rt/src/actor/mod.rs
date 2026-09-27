//! Actor runtime module for Mesh.
//!
//! Provides the core actor infrastructure: Process Control Blocks, M:N
//! work-stealing scheduler, and stackful coroutine management via corosensei.
//!
//! ## Architecture
//!
//! Mesh actors are lightweight processes multiplexed across OS threads:
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
//! Mesh programs:
//!
//! - `mesh_rt_init_actor(num_schedulers)` -- initialize the scheduler
//! - `mesh_actor_spawn(fn_ptr, args, args_size, priority)` -- spawn an actor
//! - `mesh_actor_self()` -- get current actor's PID
//! - `mesh_reduction_check()` -- decrement reductions, yield if exhausted
//! - `mesh_actor_send(target_pid, msg_ptr, msg_size)` -- send and return delivery status
//! - `mesh_actor_receive(timeout_ms)` -- receive message from mailbox
//! - `mesh_actor_link(target_pid)` -- bidirectional link to target actor
//! - `mesh_actor_set_terminate(pid, callback_fn_ptr)` -- set terminate callback

pub mod child_spec;
pub mod heap;
pub mod job;
pub mod link;
pub mod mailbox;
pub(crate) mod msg_shape;
pub mod process;
pub mod registry;
pub mod scheduler;
pub mod service;
pub mod stack;
pub mod supervisor;

pub use child_spec::{ChildSpec, ChildState, ChildType, RestartType, ShutdownType, Strategy};
pub use heap::{ActorHeap, MessageBuffer};
pub use link::{decode_exit_signal, encode_exit_signal, propagate_exit, EXIT_SIGNAL_TAG};
pub use mailbox::{Mailbox, MailboxPushError};
pub use process::{
    ExitReason, Message, Priority, Process, ProcessId, ProcessState, TerminateCallback,
    DEFAULT_REDUCTIONS, DEFAULT_STACK_SIZE,
};
pub use registry::{global_registry, ProcessRegistry};
pub use scheduler::Scheduler;
pub use stack::CoroutineHandle;

use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Global scheduler instance
// ---------------------------------------------------------------------------

/// The global scheduler, initialized by `mesh_rt_init_actor()`.
///
/// The Scheduler itself uses interior mutability (Mutex on workers, Arc on
/// shared state) so it can be shared without an outer Mutex. This prevents
/// deadlocks when actor runtime functions (receive, send) need to access the
/// scheduler while `run()` is executing on another thread.
pub(crate) static GLOBAL_SCHEDULER: OnceLock<Scheduler> = OnceLock::new();

/// Get a reference to the global scheduler.
///
/// Panics if the scheduler has not been initialized via `mesh_rt_init_actor()`.
pub(crate) fn global_scheduler() -> &'static Scheduler {
    GLOBAL_SCHEDULER
        .get()
        .expect("actor scheduler not initialized -- call mesh_rt_init_actor() first")
}

/// The process `pid` names, once the scheduler is running and while the
/// process lives.
pub(crate) fn process(pid: ProcessId) -> Option<std::sync::Arc<parking_lot::Mutex<Process>>> {
    GLOBAL_SCHEDULER.get()?.get_process(pid)
}

/// The process running on this thread.
pub(crate) fn current_process() -> Option<std::sync::Arc<parking_lot::Mutex<Process>>> {
    process(stack::get_current_pid()?)
}

/// The process running on this thread and its PID, for runtime functions
/// only compiled code calls: it always runs in a process, be it an actor,
/// `main` once the runtime has started, or a library call.
pub(crate) fn running_process() -> (ProcessId, std::sync::Arc<parking_lot::Mutex<Process>>) {
    let pid = stack::get_current_pid()
        .expect("compiled code runs in a process: an actor, `main` or a library call");
    let process = global_scheduler()
        .get_process(pid)
        .expect("a running process is in the process table");
    (pid, process)
}

/// A standard-library channel sender that wakes a suspended actor after a reply.
///
/// Distribution reader threads use this for request/reply protocols whose
/// caller may be running inside a Mesh coroutine. The value still travels over
/// an ordinary typed channel; the waiter identity only supplies the scheduler
/// wakeup that `std::sync::mpsc` does not know how to perform.
pub(crate) struct CooperativeSender<T> {
    sender: std::sync::mpsc::Sender<T>,
    waiter: Option<ProcessId>,
}

impl<T> Clone for CooperativeSender<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            waiter: self.waiter,
        }
    }
}

impl<T> CooperativeSender<T> {
    pub(crate) fn send(&self, value: T) -> Result<(), std::sync::mpsc::SendError<T>> {
        self.sender.send(value)?;
        // A waiter is an actor, so the scheduler is running.
        let Some(pid) = self.waiter else {
            return Ok(());
        };
        let scheduler = global_scheduler();
        if let Some(process) = scheduler.get_process(pid) {
            scheduler.wake_if_waiting(pid, process.lock());
        }
        Ok(())
    }
}

/// Create a reply channel that can suspend a Mesh actor without blocking its
/// scheduler worker. Outside a coroutine it behaves like a normal MPSC channel.
pub(crate) fn cooperative_channel<T>() -> (CooperativeSender<T>, std::sync::mpsc::Receiver<T>) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let waiter = stack::CURRENT_YIELDER
        .with(|current| current.yielder.get().is_some())
        .then(stack::get_current_pid)
        .flatten();
    (CooperativeSender { sender, waiter }, receiver)
}

/// Receive a reply with a monotonic timeout while yielding a Mesh coroutine.
///
/// This is the scheduler-aware equivalent of `Receiver::recv_timeout`: an
/// actor becomes Waiting and is resumed by either its reply sender or the
/// timer reactor. Non-actor callers retain the standard blocking behavior.
pub(crate) fn cooperative_recv_timeout<T>(
    receiver: &std::sync::mpsc::Receiver<T>,
    timeout: std::time::Duration,
) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
    use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
    let in_coroutine = stack::CURRENT_YIELDER.with(|current| current.yielder.get().is_some());
    if !in_coroutine {
        return receiver.recv_timeout(timeout);
    }

    let (pid, me) = running_process();
    let deadline = std::time::Instant::now() + timeout;
    wake_at(pid, deadline);
    loop {
        // Waiting before looking: a reply, or the deadline, that comes from
        // here on finds the actor Waiting and wakes it.
        me.lock().set_live_state(ProcessState::Waiting);
        let answer = match receiver.try_recv() {
            Ok(value) => Some(Ok(value)),
            Err(TryRecvError::Disconnected) => Some(Err(RecvTimeoutError::Disconnected)),
            Err(TryRecvError::Empty) => {
                (std::time::Instant::now() >= deadline).then_some(Err(RecvTimeoutError::Timeout))
            }
        };
        if let Some(answer) = answer {
            me.lock().set_live_state(ProcessState::Ready);
            return answer;
        }
        stack::yield_current();
    }
}

// ---------------------------------------------------------------------------
// ABI functions. Operations that can yield use C-unwind: dropping a cancelled
// coroutine unwinds its suspended Rust frames so owned values are released.
// ---------------------------------------------------------------------------

/// Initialize the actor scheduler.
///
/// Must be called before any `mesh_actor_spawn()` calls. Sets up the global
/// scheduler with the specified number of worker threads and starts them
/// in the background.
///
/// Also creates a "main thread process" entry in the process table, giving the
/// main thread a PID and mailbox. This allows `mesh_service_call` to work from
/// the main thread (non-coroutine context) by using spin-wait instead of yield.
///
/// Worker threads are started immediately so that actors spawned during
/// `mesh_main()` begin executing right away. This is critical for service
/// calls which need the service actor to be running to process the request.
///
/// If `num_schedulers` is 0, defaults to the number of available CPU cores.
///
/// This function is idempotent -- subsequent calls are no-ops.
#[no_mangle]
pub extern "C" fn mesh_rt_init_actor(num_schedulers: u32) {
    let scheduler = GLOBAL_SCHEDULER.get_or_init(|| {
        let default_workers = if num_schedulers == 0 {
            std::thread::available_parallelism()
                .map(|count| count.get() as u32)
                .unwrap_or(1)
        } else {
            num_schedulers
        };
        let embedded =
            crate::dist::autonomous::embedded_autonomous_config().map(|config| &config.scheduler);
        let min_workers = std::env::var("MESH_SCHEDULER_MIN_WORKERS")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .or_else(|| embedded.map(|config| u32::from(config.min_workers)))
            .unwrap_or(default_workers);
        let max_workers = std::env::var("MESH_SCHEDULER_MAX_WORKERS")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .or_else(|| embedded.map(|config| u32::from(config.max_workers)))
            .unwrap_or(min_workers);
        let sched = Scheduler::new_elastic(min_workers, max_workers)
            .unwrap_or_else(|_| Scheduler::new(default_workers));

        // Create a process entry for the main thread so it has a PID and mailbox.
        // This enables mesh_service_call to work from non-coroutine context.
        let main_pid = sched.create_main_process();
        stack::set_current_pid(main_pid);
        // The main thread collects too. It has its own stack, not a coroutine's,
        // so record where a scan of it ends.
        if let Some(main) = sched.get_process(main_pid) {
            let mut main = main.lock();
            main.stack_base = stack::current_thread_stack_base();
            main.collects_at_safepoints = !main.stack_base.is_null();
        }

        // Start worker threads in the background immediately so that actors
        // spawned during mesh_main() can begin executing right away.
        sched.start();

        sched
    });
    crate::dist::telemetry::runtime_telemetry().set_scheduler(
        scheduler.active_workers().try_into().unwrap_or(u16::MAX),
        scheduler.worker_bounds().1.try_into().unwrap_or(u16::MAX),
        scheduler.runnable_count(),
    );
    crate::dist::scaling::start_local_scheduler_autoscaler(scheduler);
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
///
/// When `args` is an object on the calling actor's GC heap, as compiled `spawn`
/// produces, the actor receives its own copy of the buffer, and every heap the
/// argument words point into keeps those objects alive until the actor's
/// process is dropped (see `adopt_spawn_args` in the scheduler). Any other
/// pointer is handed over as is: the caller must keep it valid until the entry
/// function takes ownership of it or no longer accesses it.
#[no_mangle]
pub extern "C" fn mesh_actor_spawn(
    fn_ptr: *const u8,
    args: *const u8,
    args_size: u64,
    priority: u8,
) -> u64 {
    let sched = global_scheduler();
    sched.spawn(fn_ptr, args, args_size, priority).as_u64()
}

/// `mesh_actor_spawn` for arguments that reference heap values. `shape`
/// describes the 8-byte argument slots (see `msg_shape`), so the new actor gets
/// its own copy of everything the arguments reach.
#[no_mangle]
pub extern "C" fn mesh_actor_spawn_shaped(
    fn_ptr: *const u8,
    args: *const u8,
    args_size: u64,
    priority: u8,
    shape: *const u32,
) -> u64 {
    global_scheduler()
        .spawn_shaped(fn_ptr, args, args_size, priority, shape)
        .as_u64()
}

/// Get the PID of the currently running actor.
///
/// Returns the PID as a `u64`. Returns `u64::MAX` if called outside of an
/// actor context (should not happen in compiled Mesh programs).
#[no_mangle]
pub extern "C" fn mesh_actor_self() -> u64 {
    stack::get_current_pid()
        .map(|pid| pid.as_u64())
        .unwrap_or(u64::MAX)
}

/// A PID as `"#{pid}"` shows it: `<0.12>` for a local process, and
/// `<node.local.creation>` for a remote one.
#[no_mangle]
pub extern "C" fn mesh_pid_to_string(pid: u64) -> *mut crate::string::MeshString {
    let text = ProcessId(pid).to_string();
    crate::string::mesh_str(&text)
}

/// Decrement the current actor's reduction counter and yield if exhausted.
///
/// This function is inserted by the Mesh compiler at loop back-edges and
/// function call sites. When the reduction counter reaches zero, the actor
/// yields its timeslice to the scheduler, which can then run other actors.
///
/// The reduction counter is reset to `DEFAULT_REDUCTIONS` (4000) after yield.
#[no_mangle]
pub extern "C-unwind" fn mesh_reduction_check() {
    // This runs at every call site and loop back-edge. The yielder and a
    // thread-local shadow of the reduction counter share one thread-local, so
    // the check costs a single TLS access and never locks; the actual
    // Process.reductions field is updated by the scheduler after yield.
    //
    // The closure stays tiny, with the yield outside it, so the whole fast
    // path inlines into this function.
    const YIELD: u8 = 1;
    const COLLECT: u8 = 2;
    let action = stack::CURRENT_YIELDER.with(|slot| {
        // Only yield if we're running inside a coroutine context (i.e., inside an actor).
        // The main thread also calls functions that trigger reduction_check, but the
        // main thread is not a coroutine so yield_current would panic.
        // Check the yielder to detect coroutine context (more reliable than PID
        // since the main thread now also has a PID for service call support).
        // It collects here instead, when the allocator has asked it to.
        if slot.yielder.get().is_none() {
            return if slot.gc_wanted.get() { COLLECT } else { 0 };
        }

        let remaining = slot.reductions.get();
        if remaining == 0 {
            slot.reductions.set(DEFAULT_REDUCTIONS);
            YIELD
        } else {
            slot.reductions.set(remaining - 1);
            0
        }
    });
    if action == COLLECT {
        collect_at_safepoint();
    } else if action == YIELD {
        stack::yield_current();
    }
}

/// Collect the main thread's heap from a reduction check. Out of line and
/// cold: the check itself runs at every call site and loop back-edge.
#[cold]
#[inline(never)]
fn collect_at_safepoint() {
    stack::CURRENT_YIELDER.with(|slot| slot.gc_wanted.set(false));
    try_trigger_gc();
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
    let Some(proc_arc) = current_process() else {
        return;
    };

    let mut proc = proc_arc.lock();
    if !proc.heap.should_collect() {
        return;
    }

    // Set once, as the coroutine starts or, for `main`, as the runtime does,
    // and never changed. A process that collects here has one: an actor yields
    // only from its coroutine, and `main` collects only when it has a base.
    let stack_bottom = proc.stack_base;
    assert!(
        !stack_bottom.is_null(),
        "a collecting process has a stack base"
    );

    let register_roots = capture_register_roots();

    // Capture current stack position as stack_top.
    // On x86-64 and ARM64, the stack grows downward, so stack_top (current
    // position) has a lower address than stack_bottom (base).
    let stack_anchor: u64 = 0;
    let _ = std::hint::black_box(&stack_anchor);
    let stack_top = std::cmp::min(
        &stack_anchor as *const u64 as usize,
        register_roots.as_ptr() as usize,
    ) as *const u8;

    proc.heap.collect(stack_bottom, stack_top);
    std::hint::black_box(&register_roots);
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub(crate) fn capture_register_roots() -> [usize; 10] {
    let mut roots = [0usize; 10];
    unsafe {
        std::arch::asm!(
            "stp x19, x20, [x9, #0]",
            "stp x21, x22, [x9, #16]",
            "stp x23, x24, [x9, #32]",
            "stp x25, x26, [x9, #48]",
            "stp x27, x28, [x9, #64]",
            in("x9") roots.as_mut_ptr(),
            options(nostack, preserves_flags),
        );
    }
    roots
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub(crate) fn capture_register_roots() -> [usize; 8] {
    let mut roots = [0usize; 8];
    unsafe {
        std::arch::asm!(
            "mov [rax + 0], rbx",
            "mov [rax + 8], r12",
            "mov [rax + 16], r13",
            "mov [rax + 24], r14",
            "mov [rax + 32], r15",
            "mov [rax + 40], rdi",
            "mov [rax + 48], rsi",
            "mov [rax + 56], rbp",
            in("rax") roots.as_mut_ptr(),
            options(nostack, preserves_flags),
        );
    }
    roots
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
pub(crate) fn capture_register_roots() -> [usize; 0] {
    []
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
/// The message is tagged `PROGRAM_MESSAGE_TAG`, so its contents never make it
/// pass for one of the runtime's own messages.
#[no_mangle]
pub extern "C" fn mesh_actor_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) -> i64 {
    // Locality check: upper 16 bits == 0 means local PID.
    // Single shift+compare -- essentially free on modern CPUs.
    if target_pid >> 48 == 0 {
        local_send(target_pid, msg_ptr, msg_size)
    } else {
        dist_send(target_pid, msg_ptr, msg_size, std::ptr::null())
    }
}

/// Send a message that references heap values.
///
/// A plain send copies only the message's own bytes, which for anything but a
/// scalar includes pointers into the sender's heap: the receiver would go on
/// reading objects the sender is free to collect and reuse. `shape` is the
/// compiler's description of where those references are (see `msg_shape`), so
/// the receiver gets its own copy, on this node or another.
#[no_mangle]
pub extern "C" fn mesh_actor_send_shaped(
    target_pid: u64,
    msg_ptr: *const u8,
    msg_size: u64,
    shape: *const u32,
) -> i64 {
    if target_pid >> 48 == 0 {
        local_send_with_scheduler(global_scheduler(), target_pid, msg_ptr, msg_size, shape)
    } else {
        dist_send(target_pid, msg_ptr, msg_size, shape)
    }
}

/// Pacing for a wait on the main thread, which is not a coroutine and so
/// polls its mailbox instead of yielding to the scheduler.
///
/// A reply from a running actor arrives within a few microseconds, far sooner
/// than the 60 µs or more an OS takes to honour a 10 µs sleep: sleeping from
/// the first miss made every service call from `main` cost about 85 µs, against
/// 2.5 µs from inside an actor. So spin first, and sleep only once the message
/// is clearly not coming at once.
pub(crate) struct MainThreadWait {
    polls: u32,
}

impl MainThreadWait {
    /// Misses to spin through before sleeping: a few hundred microseconds.
    const SPINS: u32 = 2_000;

    pub(crate) fn new() -> Self {
        MainThreadWait { polls: 0 }
    }

    pub(crate) fn pause(&mut self) {
        if self.polls < Self::SPINS {
            self.polls += 1;
            std::hint::spin_loop();
        } else {
            std::thread::sleep(std::time::Duration::from_micros(10));
        }
    }
}

/// Keep `env`, a closure environment that a Rust-owned structure now points
/// at, alive for as long as the returned loan is held: no collector sees that
/// pointer. Empty for a null environment or outside an actor or `main`.
pub(crate) fn lend_closure_env(env: *mut u8) -> Vec<process::HeapBorrow> {
    if env.is_null() {
        return Vec::new();
    }
    stack::get_current_pid()
        .and_then(|pid| global_scheduler().get_process(pid))
        .map_or_else(Vec::new, |owner| {
            scheduler::lend_words(&owner, &[env as usize])
        })
}

/// `lend_closure_env` for a structure that is never freed, like a router.
// ponytail: the loan is never returned either; hand it to the holder
// (`lend_closure_env`) if one of these ever gets a destructor.
pub(crate) fn pin_closure_env(env: *mut u8) {
    std::mem::forget(lend_closure_env(env));
}

/// Detach a message from the sending actor's heap before it is queued.
///
/// What `shape` describes at `buffer.data[base..]` is copied into the buffer;
/// references it cannot describe are lent by the heaps that own them. A null
/// shape is a message of plain bits.
pub(crate) fn detach_from_sender(
    sched: &Scheduler,
    buffer: &mut MessageBuffer,
    base: usize,
    shape: *const u32,
) {
    if shape.is_null() {
        return;
    }
    let sender = stack::get_current_pid()
        .and_then(|pid| sched.get_process(pid))
        .expect("a message with a shape comes from compiled code, which runs in a process");
    let captured = unsafe { msg_shape::capture(&sender.lock().heap, &buffer.data, base, shape) };
    buffer.borrows = scheduler::lend_words(&sender, &captured.lend);
    buffer.captured = captured;
}

/// Local send path -- the original mesh_actor_send body, unchanged.
///
/// Deep-copies the message bytes into a `MessageBuffer`, pushes it into
/// the target actor's FIFO mailbox, and wakes the target if it is Waiting.
pub(crate) fn local_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) -> i64 {
    local_send_with_scheduler(
        global_scheduler(),
        target_pid,
        msg_ptr,
        msg_size,
        std::ptr::null(),
    )
}

fn local_send_with_scheduler(
    sched: &Scheduler,
    target_pid: u64,
    msg_ptr: *const u8,
    msg_size: u64,
    shape: *const u32,
) -> i64 {
    let buffer = message_buffer(sched, message_bytes(msg_ptr, msg_size), shape);
    deliver_local(sched, ProcessId(target_pid), Message { buffer })
}

/// A message of bytes `data`, detached from the running actor's heap as
/// `shape` describes.
fn message_buffer(sched: &Scheduler, data: Vec<u8>, shape: *const u32) -> MessageBuffer {
    let mut buffer = MessageBuffer::new(data, PROGRAM_MESSAGE_TAG);
    detach_from_sender(sched, &mut buffer, 0, shape);
    buffer
}

/// Header tag of a message a program sent. The runtime's own messages (exit
/// signals, job results, WebSocket frames) have tags of their own, near
/// `u64::MAX`, which those waiting for them select by.
pub(crate) const PROGRAM_MESSAGE_TAG: u64 = 0;

/// Queue a prepared message for a local actor and wake it. Returns the
/// observable send status.
fn deliver_local(sched: &Scheduler, pid: ProcessId, mut msg: Message) -> i64 {
    // Look up the target process and push message.
    if let Some(proc_arc) = sched.get_process(pid) {
        msg.buffer.addressed_to(&proc_arc);
        let proc = proc_arc.lock();
        if let Err(error) = proc.mailbox.try_push(msg) {
            return match error {
                MailboxPushError::Full => 2,
                MailboxPushError::MessageTooLarge => 3,
            };
        }

        // If the target is Waiting, wake it up.
        sched.wake_if_waiting(pid, proc);
        0
    } else {
        1
    }
}

/// Remote send path -- routes a message to a remote actor via the node's
/// TLS session.
///
/// Extracts the node_id from the upper 16 bits of the target PID, looks
/// up the corresponding NodeSession, and writes a DIST_SEND message to
/// the TLS stream. Returns a nonzero status when the node is unavailable
/// or the write fails.
#[cold]
fn dist_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64, shape: *const u32) -> i64 {
    let target = ProcessId(target_pid);
    let data = message_bytes(msg_ptr, msg_size);
    match capture_for_node(target, &data, shape) {
        Some(captured) => send_to_node(target, data, captured),
        None => 6,
    }
}

/// What a message for `target`, on another node, takes with it from the
/// running actor's heap. `None`, said on stderr, when it holds code or a
/// runtime object, which cannot leave this node.
fn capture_for_node(
    target: ProcessId,
    data: &[u8],
    shape: *const u32,
) -> Option<msg_shape::Captured> {
    // A message without a shape holds only plain bits.
    if shape.is_null() {
        return Some(msg_shape::Captured::default());
    }
    let (_, sender) = running_process();
    let captured = unsafe { msg_shape::capture_for_node(&sender.lock().heap, data, shape) };
    if captured.is_none() {
        eprintln!(
            "mesh: a message for {target} holds code or a runtime object, which cannot leave this node; it was not sent"
        );
    }
    captured
}

/// Send a message `capture_for_node` took to `target` on another node.
fn send_to_node(target: ProcessId, data: Vec<u8>, captured: msg_shape::Captured) -> i64 {
    let Some(session) = crate::dist::node::session_for_pid(target) else {
        return 4;
    };
    send_application_frame(
        &session,
        crate::dist::node::encode_dist_send(target, data, captured),
    )
}

/// A copy of the `msg_size` bytes at `msg_ptr`.
fn message_bytes(msg_ptr: *const u8, msg_size: u64) -> Vec<u8> {
    if msg_ptr.is_null() || msg_size == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) }.to_vec()
    }
}

/// Queue a message that came from another node for local process `target`,
/// with the objects it references.
pub(crate) fn deliver_remote(
    target: ProcessId,
    data: Vec<u8>,
    captured: msg_shape::Captured,
) -> i64 {
    let mut buffer = MessageBuffer::new(data, PROGRAM_MESSAGE_TAG);
    buffer.captured = captured;
    deliver_local(global_scheduler(), target, Message { buffer })
}

/// Queue `payload` on a peer session: 0 once queued, 5 when the session
/// cannot take it.
fn send_application_frame(session: &crate::dist::node::NodeSession, payload: Vec<u8>) -> i64 {
    let class = crate::dist::node::OutboundClass::Application;
    session.send(class, payload).map_or(5, |()| 0)
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
pub extern "C-unwind" fn mesh_actor_receive(timeout_ms: i64) -> *const u8 {
    actor_receive_matching(timeout_ms, |_| true)
}

/// Stop a generated actor without returning a fabricated receive value.
#[no_mangle]
pub extern "C-unwind" fn mesh_actor_stop() -> ! {
    std::panic::resume_unwind(Box::new(stack::ActorStopped))
}

/// Receive the first mailbox message matching `predicate`, leaving all other
/// messages queued in their original order. Null when none comes before the
/// deadline, and, in an actor, when the program is ending and no other
/// actor is left running to send one.
pub(crate) fn actor_receive_matching<F>(timeout_ms: i64, predicate: F) -> *const u8
where
    F: Fn(&Message) -> bool,
{
    let sched = global_scheduler();
    let (my_pid, me) = running_process();
    let deadline = (timeout_ms > 0)
        .then(|| std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64));
    // `main` is not a coroutine: it polls its mailbox instead of yielding.
    let in_coroutine = stack::CURRENT_YIELDER.with(|c| c.yielder.get().is_some());
    if let Some(deadline) = deadline.filter(|_| in_coroutine) {
        wake_at(my_pid, deadline);
    }
    let mut wait = MainThreadWait::new();
    loop {
        let ending = in_coroutine && sched.is_shutdown() && !others_running(sched, my_pid);
        let mut proc = me.lock();
        if let Some(msg) = proc.mailbox.remove_first(&predicate) {
            drop(proc);
            return copy_msg_to_actor_heap(sched, my_pid, msg);
        }
        let expired = deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if timeout_ms == 0 || expired || ending {
            proc.set_live_state(ProcessState::Ready);
            return std::ptr::null();
        }
        if in_coroutine {
            // Published under the lock senders take: a message sent from
            // here on finds the actor Waiting and wakes it, as the deadline
            // does.
            proc.set_live_state(ProcessState::Waiting);
            drop(proc);
            stack::yield_current();
        } else {
            drop(proc);
            wait.pause();
        }
    }
}

/// Whether a process other than `me` is neither waiting nor gone: one that
/// could still send `me` a message.
fn others_running(sched: &Scheduler, me: ProcessId) -> bool {
    sched.process_table().read().iter().any(|(pid, process)| {
        *pid != me
            && !matches!(
                process.lock().state,
                ProcessState::Waiting | ProcessState::Exited(_)
            )
    })
}

// ── Timer functions (Phase 44 Plan 02) ──────────────────────────────

#[derive(Clone, Copy, Eq, PartialEq)]
struct TimerWake {
    deadline: std::time::Instant,
    pid: ProcessId,
}

impl Ord for TimerWake {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse the natural ordering so BinaryHeap pops the earliest timer.
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.pid.as_u64().cmp(&self.pid.as_u64()))
    }
}

impl PartialOrd for TimerWake {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

static TIMER_WAKE_SENDER: OnceLock<crossbeam_channel::Sender<TimerWake>> = OnceLock::new();

/// The timer reactor's queue. Unbounded: the reactor moves each timer into
/// a heap of its own as it arrives, so a bound here would bound nothing, and
/// a caller could only fall back to blocking its worker.
fn timer_wake_sender() -> &'static crossbeam_channel::Sender<TimerWake> {
    TIMER_WAKE_SENDER.get_or_init(|| {
        let (sender, receiver) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("mesh-timer-reactor".to_string())
            .spawn(move || timer_reactor(receiver))
            .expect("failed to start Mesh timer reactor");
        sender
    })
}

fn timer_reactor(receiver: crossbeam_channel::Receiver<TimerWake>) {
    let mut timers = std::collections::BinaryHeap::new();
    loop {
        let timeout = timers
            .peek()
            .map(|timer: &TimerWake| {
                timer
                    .deadline
                    .saturating_duration_since(std::time::Instant::now())
            })
            .unwrap_or(std::time::Duration::from_secs(60));
        match receiver.recv_timeout(timeout) {
            Ok(timer) => timers.push(timer),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        }
        while timers
            .peek()
            .is_some_and(|timer| timer.deadline <= std::time::Instant::now())
        {
            let timer = timers.pop().expect("timer was present");
            let scheduler = global_scheduler();
            if let Some(process) = scheduler.get_process(timer.pid) {
                scheduler.wake_if_waiting(timer.pid, process.lock());
            }
        }
    }
}

/// Sleep the current actor for `ms` milliseconds without blocking other actors.
///
/// Registers a bounded monotonic timer, marks the actor Waiting, and yields
/// once. The shared timer reactor makes it Ready at the deadline, so sleeping
/// actors do not create runnable pressure or busy-resume loops.
#[no_mangle]
pub extern "C-unwind" fn mesh_timer_sleep(ms: i64) {
    if ms <= 0 {
        return;
    }

    let in_coroutine = stack::CURRENT_YIELDER.with(|c| c.yielder.get().is_some());

    if !in_coroutine {
        // Main thread: just use thread::sleep
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
        return;
    }

    let (pid, me) = running_process();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms as u64);
    while std::time::Instant::now() < deadline {
        me.lock().set_live_state(ProcessState::Waiting);
        wake_at(pid, deadline);
        stack::yield_current();
        // A mailbox send may wake a sleeping actor early. Re-arm for the
        // remaining monotonic duration without consuming that message.
    }
}

/// Have the timer reactor make `pid` Ready at `deadline` if it is Waiting
/// then.
pub(crate) fn wake_at(pid: ProcessId, deadline: std::time::Instant) {
    timer_wake_sender()
        .send(TimerWake { deadline, pid })
        .expect("the timer reactor runs for as long as the program");
}

/// Schedule a message to be sent to `target_pid` after `ms` milliseconds.
///
/// Spawns a background OS thread that sleeps for `ms` then sends the message.
/// The message bytes are deep-copied at call time so the caller's stack frame
/// can be freed safely.
#[no_mangle]
pub extern "C" fn mesh_timer_send_after(
    target_pid: i64,
    ms: i64,
    msg_ptr: *const u8,
    msg_size: i64,
) {
    mesh_timer_send_after_shaped(target_pid, ms, msg_ptr, msg_size, std::ptr::null());
}

/// Run `fn_ptr(env_ptr)` after `ms` milliseconds, in an actor of its own.
///
/// `Timer.send_after` delivers a plain message, which a service's `cast`
/// handler never sees: a service dispatches on a tag only its generated
/// functions know. With this, a delayed cast is just a function that casts.
///
/// The environment is lent to the new actor like any spawn argument. The
/// caller is not linked to it: a failing callback does not take the caller down.
#[no_mangle]
pub extern "C" fn mesh_timer_apply_after(ms: i64, fn_ptr: *const u8, env_ptr: *const u8) {
    // [u64 fn_ptr][u64 env_ptr][i64 ms], on the caller's heap so that
    // `adopt_spawn_args` copies it and lends what it points at.
    let words = [fn_ptr as u64, env_ptr as u64, ms.max(0) as u64];
    let args = crate::gc::mesh_gc_alloc_actor(24, 8) as *mut u64;
    unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), args, words.len()) };
    global_scheduler().spawn(timer_apply_entry as *const u8, args as *const u8, 24, 1);
}

extern "C-unwind" fn timer_apply_entry(args: *const u8) {
    let word = |index: usize| unsafe { (args.add(8 * index) as *const u64).read_unaligned() };
    let (fn_ptr, env_ptr, ms) = (word(0), word(1), word(2));
    mesh_timer_sleep(ms as i64);
    // A null environment marks a plain function, which takes none.
    unsafe {
        if env_ptr == 0 {
            let function: extern "C-unwind" fn() -> i64 = std::mem::transmute(fn_ptr as *const u8);
            function();
        } else {
            let function: extern "C-unwind" fn(*const u8) -> i64 =
                std::mem::transmute(fn_ptr as *const u8);
            function(env_ptr as *const u8);
        }
    }
}

/// `mesh_timer_send_after` for a message that references heap values.
///
/// The message is detached from the caller's heap now, while the caller is
/// the running actor: by the time the timer fires, the sender may have
/// collected, or exited.
#[no_mangle]
pub extern "C" fn mesh_timer_send_after_shaped(
    target_pid: i64,
    ms: i64,
    msg_ptr: *const u8,
    msg_size: i64,
    shape: *const u32,
) {
    let data = message_bytes(msg_ptr, msg_size.max(0) as u64);
    let target = ProcessId(target_pid as u64);
    let delay = std::time::Duration::from_millis(if ms > 0 { ms as u64 } else { 0 });

    let prepared = if target.is_local() {
        SendOnTimer::Local(message_buffer(global_scheduler(), data, shape))
    } else {
        match capture_for_node(target, &data, shape) {
            Some(captured) => SendOnTimer::Remote(data, captured),
            None => return,
        }
    };

    std::thread::spawn(move || {
        std::thread::sleep(delay);
        match prepared {
            SendOnTimer::Local(buffer) => {
                deliver_local(global_scheduler(), target, Message { buffer });
            }
            SendOnTimer::Remote(data, captured) => {
                send_to_node(target, data, captured);
            }
        }
    });
}

/// A detached message waiting on a timer thread. Its loans hold `Process`
/// handles, which are only ever touched under their own locks.
enum SendOnTimer {
    Local(MessageBuffer),
    Remote(Vec<u8>, msg_shape::Captured),
}
unsafe impl Send for SendOnTimer {}

/// Deep-copy a message into the actor's heap and return a pointer to the
/// heap-allocated layout: `[u64 type_tag, u64 data_len, u8... data]`.
pub(crate) fn copy_msg_to_actor_heap(
    sched: &Scheduler,
    pid: ProcessId,
    mut msg: Message,
) -> *const u8 {
    let proc_arc = sched
        .get_process(pid)
        .expect("a message is copied into the running process, which is in the table");
    let mut proc = proc_arc.lock();
    // Layout: [u64 type_tag][u64 data_len][u8... data]
    let header_size = 16; // 8 bytes type_tag + 8 bytes data_len
    let total_size = header_size + msg.buffer.data.len();
    let ptr = proc.heap.alloc(total_size, 8);

    unsafe {
        // Write type_tag.
        std::ptr::copy_nonoverlapping(msg.buffer.type_tag.to_le_bytes().as_ptr(), ptr, 8);
        // Write data_len.
        let data_len = msg.buffer.data.len() as u64;
        std::ptr::copy_nonoverlapping(data_len.to_le_bytes().as_ptr(), ptr.add(8), 8);
        // Write data bytes.
        std::ptr::copy_nonoverlapping(
            msg.buffer.data.as_ptr(),
            ptr.add(header_size),
            msg.buffer.data.len(),
        );
        // Heap values the message references arrive detached from the
        // sender; rebuild them here and point the message at the copies.
        msg.buffer
            .captured
            .materialize(&mut proc.heap, ptr.add(header_size));
    }
    proc.heap_borrows.append(&mut msg.buffer.borrows);

    ptr as *const u8
}

/// Link the current actor to the target actor.
///
/// Creates a bidirectional link: when either actor terminates, the other
/// receives an exit signal. For normal exits, the signal is delivered as
/// a message. For crashes, the linked process also crashes (unless
/// `trap_exit` is set).
///
/// Supports both local and remote PIDs:
/// - Local: adds to both processes' link sets directly
/// - Remote: adds to local process's link set, sends DIST_LINK to remote node
///
/// - `target_pid`: the PID of the actor to link with
#[no_mangle]
pub extern "C" fn mesh_actor_link(target_pid: u64) {
    let (my_pid, me) = running_process();
    let target = ProcessId(target_pid);

    if target.node_id() == 0 {
        // Local link: add to both processes' link sets directly. A process
        // that has already ended has nothing to link.
        if let Some(target_proc) = global_scheduler().get_process(target) {
            link::link(&me, &target_proc, my_pid, target);
        }
    } else {
        // Remote link: record locally + send DIST_LINK to remote node.
        me.lock().links.insert(target);
        crate::dist::node::send_dist_link(my_pid, target);
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
pub extern "C" fn mesh_actor_set_terminate(pid: u64, callback_fn_ptr: *const u8) {
    let sched = global_scheduler();
    let target = ProcessId(pid);

    if let Some(proc_arc) = sched.get_process(target) {
        let cb: TerminateCallback = unsafe { std::mem::transmute(callback_fn_ptr) };
        proc_arc.lock().terminate_callback = Some(cb);
    }
}

/// Signal the scheduler to shut down and wait for all workers to finish.
///
/// This function must be called after `mesh_main()` returns. It signals
/// shutdown (allowing workers to terminate Waiting actors) and joins the
/// worker threads that were started by `mesh_rt_init_actor()`.
///
/// The scheduler shuts down when the active process count reaches zero
/// (i.e., all spawned actors have completed or been force-terminated).
fn exit_main_process(sched: &Scheduler, main_pid: ProcessId) {
    sched
        .get_process(main_pid)
        .expect("`main` has a process until the runtime ends")
        .lock()
        .mark_exited(ExitReason::Normal);
}

#[no_mangle]
pub extern "C" fn mesh_rt_run_scheduler() {
    // Preserve the main actor context until its owned resources are destroyed.
    let main_pid = stack::get_current_pid()
        .expect("`main` has a PID from mesh_rt_init_actor, which runs first");
    let sched = GLOBAL_SCHEDULER
        .get()
        .expect("actor scheduler not initialized -- call mesh_rt_init_actor() first");

    // Mark the main thread process as Exited so the scheduler doesn't
    // count it as a Ready/Running process during shutdown.
    exit_main_process(sched, main_pid);

    // Clear the main thread's PID after actor-owned resource cleanup.
    stack::clear_current_pid();

    // Signal shutdown so workers know to terminate Waiting actors when
    // no Ready/Running actors remain.
    sched.signal_shutdown();

    // Wait for all worker threads to complete.
    sched.wait();
}

/// Register an actor under a name (MeshString variant).
///
/// Called from compiled Mesh code for `Process.register(name, pid)`.
/// Takes a MeshString pointer for the name and a raw PID u64.
/// Returns 0 on success, 1 on error.
#[no_mangle]
pub extern "C" fn mesh_process_register(name: *const crate::string::MeshString, pid: u64) -> u64 {
    if pid == 0 {
        return 1;
    }
    let name_str = unsafe { (*name).as_str().to_string() };
    let pid_val = process::ProcessId(pid);
    match registry::global_registry().register(name_str, pid_val) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Look up a registered actor by name (MeshString variant).
///
/// Called from compiled Mesh code for `Process.whereis(name)`.
/// Takes a MeshString pointer for the name.
/// Returns the PID as u64, or 0 if not found.
#[no_mangle]
pub extern "C" fn mesh_process_whereis(name: *const crate::string::MeshString) -> u64 {
    let name_str = unsafe { (*name).as_str() };
    match registry::global_registry().whereis(name_str) {
        Some(pid) => pid.as_u64(),
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// Supervisor extern "C" ABI functions
// ---------------------------------------------------------------------------

/// A supervisor's actor: it restarts its children as their exit signals
/// arrive. `args` is its state, an `Arc` handed over by `mesh_supervisor_start`.
extern "C-unwind" fn supervisor_entry(args: *const u8) {
    let state = unsafe {
        std::sync::Arc::from_raw(args as *const parking_lot::Mutex<supervisor::SupervisorState>)
    };
    let (supervisor_pid, _) = running_process();
    loop {
        let message = mesh_actor_receive(-1);
        if message.is_null() {
            break;
        }
        // A program's own message means nothing to a supervisor.
        let type_tag = unsafe { std::ptr::read_unaligned(message.cast::<u64>()) };
        if type_tag != link::EXIT_SIGNAL_TAG {
            continue;
        }
        let data_len = unsafe { std::ptr::read_unaligned(message.add(8).cast::<u64>()) } as usize;
        let data = unsafe { std::slice::from_raw_parts(message.add(16), data_len) };
        let (child_pid, reason) = link::decode_exit_signal(data)
            .expect("the runtime encodes every exit signal it sends, and only it tags one");
        let sched = global_scheduler();
        let mut state = state.lock();
        if supervisor::handle_child_exit(&mut state, child_pid, &reason, sched, supervisor_pid)
            .is_err()
        {
            break;
        }
    }

    supervisor::remove_supervisor_state(supervisor_pid);
}

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
/// Returns the supervisor PID as `u64`. The compiler writes the config, so a
/// malformed one is a bug: it panics.
#[no_mangle]
pub extern "C-unwind" fn mesh_supervisor_start(config_ptr: *const u8, config_size: u64) -> u64 {
    let data = unsafe { std::slice::from_raw_parts(config_ptr, config_size as usize) };
    let config = parse_supervisor_config(data);
    let sched = global_scheduler();

    let mut sup_state =
        supervisor::SupervisorState::new(config.strategy, config.max_restarts, config.max_seconds);
    sup_state.children = config
        .child_specs
        .into_iter()
        .map(|spec| child_spec::ChildState {
            spec,
            pid: None,
            running: false,
        })
        .collect();

    // The actor takes its state with it; the registry is for everyone else.
    let state = std::sync::Arc::new(parking_lot::Mutex::new(sup_state));
    let args = std::sync::Arc::into_raw(std::sync::Arc::clone(&state));
    let sup_pid = sched.spawn(supervisor_entry as *const u8, args.cast(), 0, 1);
    // Before any child is linked to it, so it gets their exits as messages.
    sched
        .get_process(sup_pid)
        .expect("the supervisor is spawned and blocks in receive")
        .lock()
        .trap_exit = true;
    supervisor::register_supervisor_state(sup_pid, std::sync::Arc::clone(&state));

    supervisor::start_children_from(&mut state.lock(), 0, sched, sup_pid);
    sup_pid.as_u64()
}

/// Start a dynamic child under a simple_one_for_one supervisor: `u64::MAX`,
/// as there is no child template to start one from. A supervisor's children
/// come from its `child` clauses; no Mesh code can name a template yet.
#[no_mangle]
pub extern "C-unwind" fn mesh_supervisor_start_child(
    _sup_pid: u64,
    _args_ptr: *const u8,
    _args_size: u64,
) -> u64 {
    u64::MAX
}

/// Terminate a specific child under a supervisor.
///
/// Looks up the supervisor state, finds the child by PID, terminates it,
/// and removes it from the children list.
///
/// Returns 0 on success, 1 on failure.
#[no_mangle]
pub extern "C" fn mesh_supervisor_terminate_child(sup_pid: u64, child_pid: u64) -> u64 {
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
pub extern "C" fn mesh_supervisor_count_children(sup_pid: u64) -> u64 {
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
pub extern "C" fn mesh_actor_trap_exit() {
    running_process().1.lock().trap_exit = true;
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
pub(crate) fn deliver_exit_signal(sched: &Scheduler, pid: ProcessId, reason: ExitReason) {
    if let Some(proc_arc) = sched.get_process(pid) {
        let mut proc = proc_arc.lock();

        // Skip already-exited processes.
        if matches!(proc.state, ProcessState::Exited(_)) {
            return;
        }

        // Killed is untrappable.
        if matches!(reason, ExitReason::Killed) {
            proc.mark_exited(ExitReason::Killed);
            return;
        }

        if proc.trap_exit {
            // Deliver as a message.
            let signal_data = link::encode_exit_signal(pid, &reason);
            let buffer = heap::MessageBuffer::new(signal_data, link::EXIT_SIGNAL_TAG);
            proc.mailbox.push(Message { buffer });

            // Wake if Waiting.
            sched.wake_if_waiting(pid, proc);
        } else {
            // Terminate immediately.
            proc.mark_exited(reason);
        }
    }
}

#[no_mangle]
pub extern "C" fn mesh_actor_exit(target_pid: u64, reason_tag: u8) {
    let reason = match reason_tag {
        0 => ExitReason::Normal,
        1 => ExitReason::Error("exit signal".to_string()),
        2 => ExitReason::Killed,
        4 => ExitReason::Shutdown,
        5 => ExitReason::Custom("exit signal".to_string()),
        _ => ExitReason::Error(format!("unknown exit reason tag: {}", reason_tag)),
    };
    deliver_exit_signal(global_scheduler(), ProcessId(target_pid), reason);
}

/// Monitor a target process: when it ends, for whatever reason, the calling
/// actor is sent `msg` (`msg_size` bytes, whose heap references `shape`
/// describes). It is sent at once when there is no such process, or no node
/// to ask about one. Returns the reference `Process.demonitor` takes.
#[no_mangle]
pub extern "C" fn mesh_process_monitor(
    target_pid: u64,
    msg_ptr: *const u8,
    msg_size: u64,
    shape: *const u32,
) -> u64 {
    let sched = global_scheduler();
    let (my_pid, me) = running_process();
    let message = message_buffer(sched, message_bytes(msg_ptr, msg_size), shape);
    watch(sched, &me, my_pid, ProcessId(target_pid), message)
}

/// Have `message` queued for `me` (`my_pid`) when `target` ends, or at once
/// when there is no such process, or no node to ask about one. Returns the
/// monitor's reference.
pub(crate) fn watch(
    sched: &Scheduler,
    me: &std::sync::Arc<parking_lot::Mutex<Process>>,
    my_pid: ProcessId,
    target: ProcessId,
    mut message: MessageBuffer,
) -> u64 {
    let monitor_ref = link::next_monitor_ref();
    message.addressed_to(me);
    // Recorded first: the target may end as soon as it knows of the monitor.
    me.lock()
        .monitors
        .insert(monitor_ref, process::Monitor { target, message });
    let watched = if target.is_local() {
        sched.get_process(target).is_some_and(|target_arc| {
            let mut target_proc = target_arc.lock();
            let alive = !matches!(target_proc.state, ProcessState::Exited(_));
            if alive {
                target_proc.monitored_by.insert(monitor_ref, my_pid);
            }
            alive
        })
    } else {
        send_monitor_frame(crate::dist::node::DIST_MONITOR, my_pid, target, monitor_ref)
    };
    if !watched {
        me.lock().fire_monitor(monitor_ref);
    }
    monitor_ref
}

/// `[tag][u64 from][u64 to][u64 ref]` to the node `to` is on: false when
/// there is no session to it.
fn send_monitor_frame(tag: u8, from: ProcessId, to: ProcessId, monitor_ref: u64) -> bool {
    let Some(session) = crate::dist::node::session_for_pid(to) else {
        return false;
    };
    let mut payload = vec![tag];
    payload.extend_from_slice(&from.as_u64().to_le_bytes());
    payload.extend_from_slice(&to.as_u64().to_le_bytes());
    payload.extend_from_slice(&monitor_ref.to_le_bytes());
    send_application_frame(&session, payload) == 0
}

/// Remove a monitor, so its message is never sent. Returns 0 on success, 1
/// for a reference the actor does not hold.
#[no_mangle]
pub extern "C" fn mesh_process_demonitor(monitor_ref: u64) -> u64 {
    let (my_pid, _) = running_process();
    u64::from(!unwatch(global_scheduler(), my_pid, monitor_ref))
}

/// Remove `my_pid`'s monitor `monitor_ref`, so its message is never sent:
/// false when there is none, as once it has fired and queued its message.
pub(crate) fn unwatch(sched: &Scheduler, my_pid: ProcessId, monitor_ref: u64) -> bool {
    let Some(monitor) = sched
        .get_process(my_pid)
        .and_then(|me| me.lock().monitors.remove(&monitor_ref))
    else {
        return false;
    };
    if monitor.target.is_local() {
        if let Some(target_arc) = sched.get_process(monitor.target) {
            target_arc.lock().monitored_by.remove(&monitor_ref);
        }
    } else {
        send_monitor_frame(
            crate::dist::node::DIST_DEMONITOR,
            my_pid,
            monitor.target,
            monitor_ref,
        );
    }
    true
}

/// Monitor a node: when it disconnects, the calling actor is sent `msg`
/// (as for `mesh_process_monitor`), once. It is sent at once when the node
/// is not connected. Returns 0 on success, 1 before this node has started.
#[no_mangle]
pub extern "C" fn mesh_node_monitor(
    node_ptr: *const u8,
    node_len: u64,
    msg_ptr: *const u8,
    msg_size: u64,
    shape: *const u32,
) -> u64 {
    let sched = global_scheduler();
    let (my_pid, me) = running_process();
    let Some(state) = crate::dist::node::node_state() else {
        return 1;
    };
    let node_name = mesh_str(node_ptr, node_len);
    let mut message = message_buffer(sched, message_bytes(msg_ptr, msg_size), shape);
    message.addressed_to(&me);
    // Checked and recorded under the lock a disconnect takes its monitors
    // under, so the disconnect either finds this one or came first.
    let mut monitors = state.node_monitors.write();
    if state.sessions.read().contains_key(node_name) {
        monitors
            .entry(node_name.to_string())
            .or_default()
            .push((my_pid, message));
    } else {
        me.lock().mailbox.push(Message { buffer: message });
    }
    0
}

// ---------------------------------------------------------------------------
// Global registry extern "C" ABI functions (Phase 68)
// ---------------------------------------------------------------------------

/// Register a process globally across the cluster.
///
/// The name is specified as a pointer to UTF-8 bytes and a length.
/// The `pid` argument is the raw u64 PID value of the process to register.
///
/// On success, broadcasts `DIST_GLOBAL_REGISTER` to all connected nodes
/// and returns 0. Returns 1 when the name is taken or empty.
///
/// - `name_ptr`: pointer to UTF-8 name bytes
/// - `name_len`: length of the name in bytes
/// - `pid`: raw u64 PID value
#[no_mangle]
pub extern "C" fn mesh_global_register(name_ptr: *const u8, name_len: u64, pid: u64) -> u64 {
    let Some(name) = global_name(name_ptr, name_len) else {
        return 1;
    };
    let pid = process::ProcessId(pid);
    // Determine our node name for the owning_node field.
    let node_name = crate::dist::node::node_state()
        .map_or_else(|| "nonode@nohost".to_string(), |state| state.name.clone());

    let registry = crate::dist::global::global_name_registry();
    match registry.register(name.to_string(), pid, node_name.clone()) {
        Ok(()) => {
            // Broadcast to all connected nodes.
            crate::dist::global::broadcast_global_register(name, pid, &node_name);
            0
        }
        Err(_) => 1,
    }
}

/// The text of a Mesh string compiled code passes as its bytes.
fn mesh_str<'a>(ptr: *const u8, len: u64) -> &'a str {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    std::str::from_utf8(bytes).expect("a Mesh string is UTF-8")
}

/// A global name compiled code passes: `None` for the empty name, which is
/// never registered.
fn global_name<'a>(ptr: *const u8, len: u64) -> Option<&'a str> {
    Some(mesh_str(ptr, len)).filter(|name| !name.is_empty())
}

/// Look up a globally registered process by name.
///
/// Returns the PID of the process registered under the given name, or 0
/// if no process is registered with that name.
///
/// This is always a local lookup -- no network call is made.
///
/// - `name_ptr`: pointer to UTF-8 name bytes
/// - `name_len`: length of the name in bytes
#[no_mangle]
pub extern "C" fn mesh_global_whereis(name_ptr: *const u8, name_len: u64) -> u64 {
    global_name(name_ptr, name_len)
        .and_then(|name| crate::dist::global::global_name_registry().whereis(name))
        .map_or(0, ProcessId::as_u64)
}

/// Unregister a globally registered name.
///
/// On success, broadcasts `DIST_GLOBAL_UNREGISTER` to all connected nodes
/// and returns 0. Returns 1 if the name was not registered.
///
/// - `name_ptr`: pointer to UTF-8 name bytes
/// - `name_len`: length of the name in bytes
#[no_mangle]
pub extern "C" fn mesh_global_unregister(name_ptr: *const u8, name_len: u64) -> u64 {
    let Some(name) = global_name(name_ptr, name_len) else {
        return 1;
    };
    let registry = crate::dist::global::global_name_registry();
    if registry.unregister(name) {
        crate::dist::global::broadcast_global_unregister(name);
        0
    } else {
        1
    }
}

/// Reads the supervisor config the compiler wrote, in order.
struct ConfigReader<'a>(&'a [u8]);

impl<'a> ConfigReader<'a> {
    fn bytes(&mut self, len: usize) -> &'a [u8] {
        let (head, rest) = self
            .0
            .split_at_checked(len)
            .expect("the compiler writes a whole supervisor config");
        self.0 = rest;
        head
    }

    fn u8(&mut self) -> usize {
        usize::from(self.bytes(1)[0])
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes(4).try_into().unwrap())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.bytes(8).try_into().unwrap())
    }
}

/// Parse the `SupervisorConfig` the compiler wrote (see
/// `mesh_supervisor_start`). A tag out of range panics, as a short config
/// does: the compiler writes neither.
fn parse_supervisor_config(data: &[u8]) -> supervisor::SupervisorConfig {
    use child_spec::{ChildType, RestartType, ShutdownType, Strategy};
    let mut config = ConfigReader(data);
    let strategy = [
        Strategy::OneForOne,
        Strategy::OneForAll,
        Strategy::RestForOne,
        Strategy::SimpleOneForOne,
    ][config.u8()];
    let max_restarts = config.u32();
    let max_seconds = config.u64();
    let children = config.u32();
    let child_specs = (0..children)
        .map(|_| {
            let id_len = config.u32() as usize;
            let id = String::from_utf8(config.bytes(id_len).to_vec())
                .expect("a child's id is a Mesh identifier");
            let start_fn = config.u64() as *const u8;
            let start_args_ptr = config.u64() as *const u8;
            let start_args_size = config.u64();
            let restart_type = [
                RestartType::Permanent,
                RestartType::Transient,
                RestartType::Temporary,
            ][config.u8()];
            let shutdown_tag = config.u8();
            let timeout = config.u64();
            let shutdown = [ShutdownType::BrutalKill, ShutdownType::Timeout(timeout)][shutdown_tag];
            let child_type = [ChildType::Worker, ChildType::Supervisor][config.u8()];
            child_spec::ChildSpec {
                id,
                start_fn,
                start_args_ptr,
                start_args_size,
                restart_type,
                shutdown,
                child_type,
            }
        })
        .collect();

    supervisor::SupervisorConfig {
        strategy,
        max_restarts,
        max_seconds,
        child_specs,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Run `body` as an actor of the global scheduler, for a test, and return
/// what it returns.
#[cfg(test)]
pub(crate) fn in_actor<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    type Body = Box<dyn FnOnce() + Send>;
    extern "C-unwind" fn entry(args: *const u8) {
        let body = unsafe { Box::from_raw(args as *mut Body) };
        body();
    }
    mesh_rt_init_actor(1);
    let (sender, receiver) = std::sync::mpsc::channel();
    let body: Body = Box::new(move || {
        let _ = sender.send(body());
    });
    let args = Box::into_raw(Box::new(body));
    global_scheduler().spawn(entry as *const u8, args.cast(), 0, 1);
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the actor ran to its end")
}

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
    fn forced_runtime_exit_destroys_waiting_actor_secrets() {
        let sched = Scheduler::new(1);
        let pid = create_test_process(&sched);
        sched.get_process(pid).unwrap().lock().state = ProcessState::Waiting;
        crate::secret::insert_test_secret(pid);

        deliver_exit_signal(&sched, pid, ExitReason::Killed);

        let state_is_killed = matches!(
            sched.get_process(pid).unwrap().lock().state,
            ProcessState::Exited(ExitReason::Killed)
        );
        let remaining = crate::secret::owned_secret_count_for_test(pid);
        crate::secret::destroy_owned(pid);
        assert!(state_is_killed);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn main_scheduler_shutdown_destroys_owned_secrets() {
        let sched = Scheduler::new(1);
        let main_pid = sched.create_main_process();
        crate::secret::insert_test_secret(main_pid);

        exit_main_process(&sched, main_pid);

        let state_is_exited = matches!(
            sched.get_process(main_pid).unwrap().lock().state,
            ProcessState::Exited(ExitReason::Normal)
        );
        let remaining = crate::secret::owned_secret_count_for_test(main_pid);
        crate::secret::destroy_owned(main_pid);
        assert!(state_is_exited);
        assert_eq!(remaining, 0);
    }

    #[inline(never)]
    fn allocate_receive_garbage() {
        for _ in 0..5 {
            let ptr = crate::gc::mesh_gc_alloc_actor(128 * 1024, 8);
            unsafe { std::ptr::write_volatile(ptr, 1) };
        }
    }

    extern "C" fn allocate_then_receive(_args: *const u8) {
        allocate_receive_garbage();
        mesh_actor_receive(-1);
    }

    #[test]
    fn blocking_receive_collects_long_lived_actor_heap() {
        mesh_rt_init_actor(1);
        let sched = global_scheduler();
        let pid = sched.spawn(allocate_then_receive as *const u8, std::ptr::null(), 0, 1);

        for _ in 0..100 {
            let waiting = sched
                .get_process(pid)
                .map(|process| matches!(process.lock().state, ProcessState::Waiting))
                .unwrap_or(false);
            if waiting {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));

        let (retained, threshold) = {
            let process = sched.get_process(pid).expect("actor should be waiting");
            let process = process.lock();
            (process.heap.total_bytes(), process.heap.gc_threshold())
        };
        local_send(pid.as_u64(), std::ptr::null(), 0);

        assert!(
            retained < threshold,
            "blocking receive retained {retained} bytes above the {threshold}-byte threshold"
        );
    }

    struct ReceiveHandshake {
        ready: std::sync::mpsc::Sender<()>,
        received: std::sync::mpsc::Sender<bool>,
    }

    extern "C" fn receive_handshakes(args: *const u8) {
        let channels = unsafe { Box::from_raw(args as *mut ReceiveHandshake) };
        for _ in 0..10_000 {
            if channels.ready.send(()).is_err() {
                return;
            }
            let message = mesh_actor_receive(-1);
            if channels.received.send(!message.is_null()).is_err() {
                return;
            }
        }
    }

    #[test]
    fn blocking_receive_does_not_lose_concurrent_sends() {
        mesh_rt_init_actor(1);
        let sched = global_scheduler();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (received_tx, received_rx) = std::sync::mpsc::channel();
        let args = Box::into_raw(Box::new(ReceiveHandshake {
            ready: ready_tx,
            received: received_tx,
        }));
        let pid = sched.spawn(receive_handshakes as *const u8, args.cast(), 0, 1);
        let timeout = std::time::Duration::from_secs(5);
        let result = (|| {
            for _ in 0..10_000 {
                ready_rx.recv_timeout(timeout)?;
                assert_eq!(local_send(pid.as_u64(), std::ptr::null(), 0), 0);
                assert!(received_rx.recv_timeout(timeout)?);
            }
            Ok::<(), std::sync::mpsc::RecvTimeoutError>(())
        })();
        drop(ready_rx);
        drop(received_rx);
        let _ = local_send(pid.as_u64(), std::ptr::null(), 0);
        assert!(result.is_ok(), "receive lost a concurrent send: {result:?}");
    }

    extern "C" fn receive_with_deadline(args: *const u8) {
        let sender = unsafe {
            Box::from_raw(args as *mut std::sync::mpsc::Sender<(bool, std::time::Duration)>)
        };
        let start = std::time::Instant::now();
        let timed_out = mesh_actor_receive(20).is_null();
        let _ = sender.send((timed_out, start.elapsed()));
    }

    /// A reply that is there, one that comes while the actor waits, a sender
    /// that has gone and a deadline that passes each end the wait.
    #[test]
    fn cooperative_recv_timeout_answers_every_way_a_wait_ends() {
        use std::sync::mpsc::RecvTimeoutError;
        use std::time::Duration;
        let answers = in_actor(|| {
            let (sender, receiver) = cooperative_channel();
            sender.send(5).unwrap();
            let ready = cooperative_recv_timeout(&receiver, Duration::from_secs(5));
            let later = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                sender.send(9).unwrap();
            });
            let waited = cooperative_recv_timeout(&receiver, Duration::from_secs(5));
            later.join().unwrap();
            let gone = cooperative_recv_timeout(&receiver, Duration::from_secs(5));
            let (_sender, silent) = cooperative_channel::<i32>();
            let timed_out = cooperative_recv_timeout(&silent, Duration::from_millis(20));
            [ready, waited, gone, timed_out]
        });
        assert_eq!(
            answers,
            [
                Ok(5),
                Ok(9),
                Err(RecvTimeoutError::Disconnected),
                Err(RecvTimeoutError::Timeout)
            ]
        );
    }

    #[test]
    fn blocking_receive_timeout_wakes_without_a_message() {
        mesh_rt_init_actor(1);
        let sched = global_scheduler();
        let (sender, receiver) = std::sync::mpsc::channel::<(bool, std::time::Duration)>();
        let args = Box::into_raw(Box::new(sender));
        let pid = sched.spawn(receive_with_deadline as *const u8, args.cast(), 0, 1);
        let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
        drop(receiver);
        let _ = local_send(pid.as_u64(), std::ptr::null(), 0);
        let (timed_out, elapsed) = result.expect("receive deadline did not wake the actor");
        assert!(timed_out);
        assert!(elapsed >= std::time::Duration::from_millis(20));
    }

    #[test]
    fn test_send_delivers_to_mailbox() {
        let sched = Scheduler::new(1);
        let target_pid = create_test_process(&sched);

        // Manually push a message (simulating mesh_actor_send logic).
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
    fn observable_send_reports_missing_and_bounded_mailboxes() {
        let sched = Scheduler::new(1);
        let target_pid = create_test_process(&sched);
        let process = sched.get_process(target_pid).unwrap();
        process.lock().mailbox = Arc::new(Mailbox::bounded(1, 4));
        let four_bytes = [1, 2, 3, 4];
        let five_bytes = [1, 2, 3, 4, 5];

        assert_eq!(
            local_send_with_scheduler(
                &sched,
                target_pid.as_u64(),
                four_bytes.as_ptr(),
                four_bytes.len() as u64,
                std::ptr::null(),
            ),
            0
        );
        assert_eq!(
            local_send_with_scheduler(
                &sched,
                target_pid.as_u64(),
                four_bytes.as_ptr(),
                four_bytes.len() as u64,
                std::ptr::null(),
            ),
            2
        );
        process.lock().mailbox.pop();
        assert_eq!(
            local_send_with_scheduler(
                &sched,
                target_pid.as_u64(),
                five_bytes.as_ptr(),
                five_bytes.len() as u64,
                std::ptr::null(),
            ),
            3
        );
        assert_eq!(
            local_send_with_scheduler(&sched, u64::MAX, std::ptr::null(), 0, std::ptr::null()),
            1
        );
    }

    #[test]
    fn shaped_send_copies_the_string_into_the_receivers_heap() {
        let sched = Scheduler::new(1);
        let sender_pid = create_test_process(&sched);
        let target_pid = create_test_process(&sched);
        let sender = sched.get_process(sender_pid).unwrap();
        let text = "payload-42";
        let sent = {
            let object = sender.lock().heap.alloc(8 + text.len(), 8);
            unsafe {
                (object as *mut u64).write(text.len() as u64);
                std::ptr::copy_nonoverlapping(text.as_ptr(), object.add(8), text.len());
            }
            object
        };
        let message = [sent as u64];
        let shape = [2, msg_shape::LEAF];

        stack::set_current_pid(sender_pid);
        let status = local_send_with_scheduler(
            &sched,
            target_pid.as_u64(),
            message.as_ptr() as *const u8,
            8,
            shape.as_ptr(),
        );
        stack::clear_current_pid();
        assert_eq!(status, 0);
        // The sender's string is overwritten, as if collected and reused.
        unsafe { std::ptr::write_bytes(sent.add(8), b'x', text.len()) };

        let process = sched.get_process(target_pid).unwrap();
        let queued = process.lock().mailbox.pop().expect("message queued");
        let delivered = copy_msg_to_actor_heap(&sched, target_pid, queued);
        let received = unsafe { *(delivered.add(16) as *const *const crate::string::MeshString) };
        assert_ne!(received as *const u8, sent as *const u8);
        assert_eq!(unsafe { (*received).as_str() }, text);
        assert!(process
            .lock()
            .heap
            .is_live_allocation(received as *const u8, 8 + text.len()));
        assert!(process.lock().heap_borrows.is_empty(), "copied, not lent");
    }

    #[test]
    fn timer_send_carries_its_own_copy_of_the_message() {
        mesh_rt_init_actor(1);
        let sched = global_scheduler();
        let sender_pid = sched.create_main_process();
        let target_pid = sched.create_main_process();
        let sender = sched.get_process(sender_pid).unwrap();
        let text = "payload-42";
        let sent = {
            let object = sender.lock().heap.alloc(8 + text.len(), 8);
            unsafe {
                (object as *mut u64).write(text.len() as u64);
                std::ptr::copy_nonoverlapping(text.as_ptr(), object.add(8), text.len());
            }
            object
        };
        let message = [sent as u64];
        let shape = [2, msg_shape::LEAF];

        let previous = stack::get_current_pid();
        stack::set_current_pid(sender_pid);
        mesh_timer_send_after_shaped(
            target_pid.as_u64() as i64,
            20,
            message.as_ptr() as *const u8,
            8,
            shape.as_ptr(),
        );
        match previous {
            Some(pid) => stack::set_current_pid(pid),
            None => stack::clear_current_pid(),
        }
        // The sender forgets the string, and it is reused, before the timer fires.
        unsafe { std::ptr::write_bytes(sent, 0, 8 + text.len()) };

        let target = sched.get_process(target_pid).unwrap();
        let queued = (0..200)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(5));
                target.lock().mailbox.pop()
            })
            .expect("timer delivered the message");
        let delivered = copy_msg_to_actor_heap(sched, target_pid, queued);
        let received = unsafe { *(delivered.add(16) as *const *const crate::string::MeshString) };
        assert_eq!(unsafe { (*received).as_str() }, text);
    }

    #[test]
    fn shaped_send_lends_what_it_cannot_copy_and_never_pins_itself() {
        let sched = Scheduler::new(1);
        let sender_pid = create_test_process(&sched);
        let target_pid = create_test_process(&sched);
        let sender = sched.get_process(sender_pid).unwrap();
        let environment = sender.lock().heap.alloc(16, 8);
        let message = [environment as u64];
        let shape = [2, msg_shape::SHARED];
        let send_to = |target: ProcessId| {
            stack::set_current_pid(sender_pid);
            let status = local_send_with_scheduler(
                &sched,
                target.as_u64(),
                message.as_ptr() as *const u8,
                8,
                shape.as_ptr(),
            );
            stack::clear_current_pid();
            assert_eq!(status, 0);
            let process = sched.get_process(target).unwrap();
            let queued = process.lock().mailbox.pop().expect("message queued");
            copy_msg_to_actor_heap(&sched, target, queued)
        };

        // To another actor: same pointer, kept alive by a loan that pins the sender.
        let delivered = send_to(target_pid);
        assert_eq!(
            unsafe { *(delivered.add(16) as *const u64) },
            environment as u64
        );
        let dummy: u64 = 0;
        let stack = &dummy as *const u64 as *const u8;
        sender.lock().heap.collect(stack, stack);
        assert!(sender.lock().heap.is_live_allocation(environment, 16));
        let target = sched.get_process(target_pid).unwrap();
        assert!(target.lock().heap_borrows[0].owner.is_some());

        // To itself: still kept alive, but the process must not own itself.
        send_to(sender_pid);
        assert!(sender
            .lock()
            .heap_borrows
            .iter()
            .all(|loan| loan.owner.is_none()));
    }

    /// A message's header tag says what the runtime sent it as (an exit
    /// signal, a job result, a WebSocket frame); a program's own message
    /// never passes for one of those, whatever its first word holds.
    #[test]
    fn a_program_message_is_never_tagged_as_a_runtime_message() {
        let sched = Scheduler::new(1);
        let target = create_test_process(&sched);
        for first_word in [link::EXIT_SIGNAL_TAG, job::JOB_RESULT_TAG, 7] {
            let data = first_word.to_le_bytes();
            let status = local_send_with_scheduler(
                &sched,
                target.as_u64(),
                data.as_ptr(),
                8,
                std::ptr::null(),
            );
            assert_eq!(status, 0);
            let message = sched.get_process(target).unwrap().lock().mailbox.pop();
            assert_eq!(message.unwrap().buffer.type_tag, PROGRAM_MESSAGE_TAG);
        }
    }

    #[test]
    fn runtime_message_tags_are_distinct() {
        let tags = [
            PROGRAM_MESSAGE_TAG,
            link::EXIT_SIGNAL_TAG,
            job::JOB_RESULT_TAG,
            crate::ws::WS_TEXT_TAG,
            crate::ws::WS_BINARY_TAG,
            crate::ws::WS_DISCONNECT_TAG,
            crate::ws::WS_CONNECT_TAG,
        ];
        let distinct: std::collections::HashSet<u64> = tags.into_iter().collect();
        assert_eq!(distinct.len(), tags.len());
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
            assert_eq!(
                msg.buffer.type_tag, i as u64,
                "FIFO order violated at {}",
                i
            );
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

        // Push message and wake (simulating mesh_actor_send).
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
    #[should_panic(expected = "compiled code runs in a process")]
    fn a_receive_outside_any_process_is_a_bug_in_its_caller() {
        mesh_rt_init_actor(1);
        stack::clear_current_pid();
        mesh_actor_receive(0);
    }

    /// `main` polls its mailbox: for a message already there, one that comes
    /// while it waits, none by a deadline, and none for a receive that does
    /// not wait at all.
    #[test]
    fn main_thread_receive_polls_its_mailbox() {
        mesh_rt_init_actor(1);
        let me = global_scheduler().create_main_process();
        let send = move |word: u64| local_send(me.as_u64(), word.to_le_bytes().as_ptr(), 8);
        let word = |message: *const u8| unsafe { (message.add(16) as *const u64).read() };
        stack::set_current_pid(me);
        send(7);
        let queued = word(mesh_actor_receive(0));
        let empty = mesh_actor_receive(0).is_null();
        let timed_out = mesh_actor_receive(5).is_null();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            send(9);
        });
        let waited = word(mesh_actor_receive(-1));
        sender.join().unwrap();
        stack::clear_current_pid();
        assert_eq!((queued, empty, timed_out, waited), (7, true, true, 9));
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
        let receiver_data = unsafe { std::slice::from_raw_parts(ptr_in_receiver, data.len()) };
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
    fn test_exit_propagation_normal_is_ignored_without_trap_exit() {
        let sched = Scheduler::new(1);
        let pid_a = create_test_process(&sched);
        let pid_b = create_test_process(&sched);

        let proc_a = sched.get_process(pid_a).unwrap();
        let proc_b = sched.get_process(pid_b).unwrap();
        link::link(&proc_a, &proc_b, pid_a, pid_b);

        let linked_pids = std::mem::take(&mut proc_a.lock().links);
        link::propagate_exit(pid_a, &ExitReason::Normal, linked_pids, |pid| {
            sched.get_process(pid)
        });

        // Process B should NOT be crashed.
        assert!(
            !matches!(proc_b.lock().state, ProcessState::Exited(_)),
            "Normal exit should not crash linked process"
        );

        // Nor get a message: its receive would read the signal as its own.
        assert!(proc_b.lock().mailbox.pop().is_none());
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

    /// Run `body` as a new process of the global scheduler, on this thread.
    fn as_process<T>(body: impl FnOnce(ProcessId) -> T) -> T {
        mesh_rt_init_actor(1);
        let pid = global_scheduler().create_main_process();
        stack::set_current_pid(pid);
        let result = body(pid);
        stack::clear_current_pid();
        result
    }

    fn mesh_string(text: &str) -> *const crate::string::MeshString {
        crate::string::mesh_string_new(text.as_ptr(), text.len() as u64)
    }

    /// A pid on node 3, which this process has never heard of.
    fn unknown_node_pid() -> ProcessId {
        ProcessId::from_remote(3, 1, 9)
    }

    /// A message for another node needs a session to it (4) and must hold
    /// nothing that cannot leave this node (6); a delayed one of the latter
    /// is dropped as it is sent.
    #[test]
    fn a_message_for_another_node_reports_why_it_cannot_go() {
        let word = 7u64.to_le_bytes();
        // `{fn, env}`: a closure, which cannot leave this node.
        let closure = [1u64, 0];
        let shape = [2, msg_shape::CLOSURE];
        let remote = unknown_node_pid().as_u64();
        let (plain, code) = as_process(|_| {
            let closure = closure.as_ptr().cast();
            mesh_timer_send_after_shaped(remote as i64, 0, closure, 16, shape.as_ptr());
            (
                mesh_actor_send(remote, word.as_ptr(), 8),
                mesh_actor_send_shaped(remote, closure, 16, shape.as_ptr()),
            )
        });
        assert_eq!((plain, code), (4, 6));
    }

    static PLAIN_FUNCTION_RAN: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    extern "C-unwind" fn plain_function() -> i64 {
        PLAIN_FUNCTION_RAN.store(true, std::sync::atomic::Ordering::SeqCst);
        0
    }

    /// A plain function (no environment) runs after its delay too, and a
    /// sleep of no time returns at once.
    #[test]
    fn timer_apply_after_runs_a_plain_function() {
        mesh_rt_init_actor(1);
        mesh_timer_sleep(0);
        mesh_timer_sleep(-5);
        mesh_timer_apply_after(1, plain_function as *const u8, std::ptr::null());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !PLAIN_FUNCTION_RAN.load(std::sync::atomic::Ordering::SeqCst)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(PLAIN_FUNCTION_RAN.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// A link to another node is recorded here; one to a process that has
    /// ended is not made, and neither is a terminate callback for one.
    #[test]
    fn links_to_another_node_and_to_an_ended_process() {
        extern "C" fn callback(_state: *const u8, _reason: *const u8) {}
        let gone = ProcessId(u64::MAX >> 24);
        let links = as_process(|me| {
            mesh_actor_link(unknown_node_pid().as_u64());
            mesh_actor_link(gone.as_u64());
            mesh_actor_set_terminate(gone.as_u64(), callback as *const u8);
            let me = global_scheduler().get_process(me).unwrap();
            let links = me.lock().links.clone();
            links
        });
        assert_eq!(links, [unknown_node_pid()].into_iter().collect());
    }

    #[test]
    fn process_names_refuse_pid_zero_and_a_taken_name() {
        let name = mesh_string("names-refuse-pid-zero");
        assert_eq!(mesh_process_register(name, 0), 1);
        assert_eq!(mesh_process_register(name, 5), 0);
        assert_eq!(mesh_process_register(name, 6), 1);
        assert_eq!(mesh_process_whereis(name), 5);
    }

    /// `trap_exit` makes an exit signal a message, whatever its reason but
    /// a kill, which ends the process; a process that has ended, or never
    /// was, takes none.
    #[test]
    fn exit_signals_reach_a_trapping_process_as_messages() {
        let reasons = as_process(|me| {
            mesh_actor_trap_exit();
            for tag in [0, 1, 4, 5, 9] {
                mesh_actor_exit(me.as_u64(), tag);
            }
            let process = global_scheduler().get_process(me).unwrap();
            let mut reasons = Vec::new();
            while let Some(message) = process.lock().mailbox.pop() {
                assert_eq!(message.buffer.type_tag, link::EXIT_SIGNAL_TAG);
                reasons.push(link::decode_exit_signal(&message.buffer.data).unwrap().1);
            }
            mesh_actor_exit(me.as_u64(), 2);
            mesh_actor_exit(me.as_u64(), 1);
            mesh_actor_exit(u64::MAX >> 24, 1);
            let state = process.lock().state.clone();
            let queued = process.lock().mailbox.len();
            (reasons, state, queued)
        });
        let error = |text: &str| ExitReason::Error(text.to_string());
        assert_eq!(
            reasons,
            (
                vec![
                    ExitReason::Normal,
                    error("exit signal"),
                    ExitReason::Shutdown,
                    ExitReason::Custom("exit signal".to_string()),
                    error("unknown exit reason tag: 9"),
                ],
                ProcessState::Exited(ExitReason::Killed),
                0
            )
        );
    }

    /// A monitor on a process of a node this one has no session to fires at
    /// once; a reference nobody holds does not demonitor.
    #[test]
    fn a_monitor_of_an_unreachable_process_fires_at_once() {
        let message = 3u64.to_le_bytes();
        let (queued, demonitored) = as_process(|me| {
            let remote = unknown_node_pid().as_u64();
            mesh_process_monitor(remote, message.as_ptr(), 8, std::ptr::null());
            let process = global_scheduler().get_process(me).unwrap();
            let queued = process.lock().mailbox.pop().map(|m| m.buffer.data);
            (queued, mesh_process_demonitor(u64::MAX))
        });
        assert_eq!(queued, Some(message.to_vec()));
        assert_eq!(demonitored, 1);
    }

    /// The empty global name is never registered, found or unregistered, and
    /// a taken name is not registered again.
    #[test]
    fn global_names_refuse_the_empty_name_and_a_taken_one() {
        let (empty, name) = ("", "global-names-refuse-a-taken-one");
        let call = |text: &str, f: extern "C" fn(*const u8, u64) -> u64| {
            f(text.as_ptr(), text.len() as u64)
        };
        assert_eq!(mesh_global_register(empty.as_ptr(), 0, 5), 1);
        assert_eq!(call(empty, mesh_global_whereis), 0);
        assert_eq!(call(empty, mesh_global_unregister), 1);
        assert_eq!(mesh_global_register(name.as_ptr(), name.len() as u64, 5), 0);
        assert_eq!(mesh_global_register(name.as_ptr(), name.len() as u64, 6), 1);
        assert_eq!(call(name, mesh_global_whereis), 5);
        assert_eq!(call(name, mesh_global_unregister), 0);
        assert_eq!(call(name, mesh_global_unregister), 1);
    }

    // -----------------------------------------------------------------------
    // Supervisor config parser tests (Phase 69)
    // -----------------------------------------------------------------------

    /// The supervisor config the compiler writes for `children`: one for one,
    /// each a permanent worker killed outright.
    fn supervisor_config(children: &[extern "C-unwind" fn(*const u8)]) -> Vec<u8> {
        let mut config = vec![0];
        config.extend_from_slice(&3u32.to_le_bytes());
        config.extend_from_slice(&5u64.to_le_bytes());
        config.extend_from_slice(&(children.len() as u32).to_le_bytes());
        for child in children {
            config.extend_from_slice(&7u32.to_le_bytes());
            config.extend_from_slice(b"worker1");
            config.extend_from_slice(&(*child as usize as u64).to_le_bytes());
            config.extend_from_slice(&[0; 16]);
            config.extend_from_slice(&[0, 0]);
            config.extend_from_slice(&0u64.to_le_bytes());
            config.push(0);
        }
        config
    }

    extern "C-unwind" fn idle_child(_args: *const u8) {
        mesh_actor_receive(-1);
    }

    #[test]
    fn a_supervisor_config_of_one_child_parses() {
        let mut config = supervisor_config(&[idle_child]);
        let parsed = parse_supervisor_config(&config);
        assert_eq!(parsed.child_specs.len(), 1);
        assert_eq!(parsed.child_specs[0].id, "worker1");
        config.pop();
        let cut_off = std::panic::catch_unwind(|| parse_supervisor_config(&config));
        assert!(cut_off.is_err(), "a config cut short is a compiler bug");
    }

    /// A supervisor passes over a program's messages, and answers for its
    /// children by PID; it has no template to start a dynamic child from.
    #[test]
    fn a_supervisor_ignores_program_messages_and_answers_for_its_children() {
        mesh_rt_init_actor(1);
        let config = supervisor_config(&[idle_child]);
        let sup = mesh_supervisor_start(config.as_ptr(), config.len() as u64);
        let state = supervisor::get_supervisor_state(ProcessId(sup)).unwrap();
        let child = state.lock().children[0].pid.unwrap().as_u64();
        let nobody = u64::MAX >> 24;
        assert_eq!(local_send(sup, 7u64.to_le_bytes().as_ptr(), 8), 0);

        assert_eq!(mesh_supervisor_count_children(sup), 1);
        assert_eq!(mesh_supervisor_count_children(nobody), 0);
        assert_eq!(
            mesh_supervisor_start_child(sup, std::ptr::null(), 0),
            u64::MAX
        );
        assert_eq!(mesh_supervisor_terminate_child(sup, nobody), 1);
        assert_eq!(mesh_supervisor_terminate_child(nobody, child), 1);
        assert_eq!(mesh_supervisor_terminate_child(sup, child), 0);
        assert_eq!(mesh_supervisor_count_children(sup), 0);
        let supervisor = global_scheduler().get_process(ProcessId(sup)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !supervisor.lock().mailbox.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            supervisor.lock().mailbox.is_empty(),
            "the message was taken"
        );
        assert!(!matches!(supervisor.lock().state, ProcessState::Exited(_)));
    }
}
