//! Process Control Block (PCB) for Mesh actors.
//!
//! Each Mesh actor is a lightweight process with its own PID, state, priority,
//! reduction counter, mailbox, and optional terminate callback. Processes are
//! multiplexed across OS threads by the M:N scheduler.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use super::heap::{ActorHeap, MessageBuffer};
use super::mailbox::Mailbox;

// ---------------------------------------------------------------------------
// ProcessId
// ---------------------------------------------------------------------------

/// Unique identifier for an actor process.
///
/// PIDs are assigned sequentially from a global atomic counter, unique among
/// a runtime's live processes. 0 is never a process: it is what a lookup that
/// finds none and a spawn that failed return, and a send to it goes nowhere.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u64);

/// The local counter's bits of a PID (bits 39..0).
const LOCAL_ID_MASK: u64 = 0x0000_00FF_FFFF_FFFF;

static NEXT_LOCAL_ID: AtomicU64 = AtomicU64::new(1);

impl ProcessId {
    /// A fresh local PID (see `next_unused`), for a process no table holds.
    pub fn next() -> Self {
        Self::next_unused(|_| false)
    }

    /// A fresh local PID that no live process holds (`in_use`). The counter
    /// is masked to its 40 bits; after 2^40 spawns (weeks, for a server
    /// spawning per request) it wraps, and then skips 0 and the ids of
    /// processes still running, such as actors started with the program.
    pub fn next_unused(in_use: impl Fn(ProcessId) -> bool) -> Self {
        next_local_id(&NEXT_LOCAL_ID, in_use)
    }

    /// Return the raw numeric value.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Extract the 16-bit node identifier (bits 63..48).
    ///
    /// A node_id of 0 means the PID belongs to the local node.
    #[inline]
    pub fn node_id(self) -> u16 {
        (self.0 >> 48) as u16
    }

    /// Extract the 8-bit creation counter (bits 47..40).
    ///
    /// The creation counter distinguishes different incarnations of the
    /// same node, preventing stale PID confusion after a node restart.
    #[inline]
    pub fn creation(self) -> u8 {
        ((self.0 >> 40) & 0xFF) as u8
    }

    /// Extract the 40-bit local process identifier (bits 39..0).
    #[inline]
    pub fn local_id(self) -> u64 {
        self.0 & LOCAL_ID_MASK
    }

    /// Check if this PID belongs to the local node (node_id == 0).
    #[inline]
    pub fn is_local(self) -> bool {
        self.0 >> 48 == 0
    }

    /// Construct a PID from remote node components.
    ///
    /// Layout: `[16-bit node_id | 8-bit creation | 40-bit local_id]`
    #[inline]
    pub fn from_remote(node_id: u16, creation: u8, local_id: u64) -> Self {
        debug_assert!(
            local_id < (1u64 << 40),
            "local_id exceeds 40 bits: {}",
            local_id
        );
        ProcessId((node_id as u64) << 48 | (creation as u64) << 40 | (local_id & LOCAL_ID_MASK))
    }
}

/// The next id of `counter` that is not 0 and, once the counter has wrapped
/// past the 40 bits, not `in_use`.
fn next_local_id(counter: &AtomicU64, in_use: impl Fn(ProcessId) -> bool) -> ProcessId {
    loop {
        let count = counter.fetch_add(1, Ordering::Relaxed);
        let pid = ProcessId(count & LOCAL_ID_MASK);
        if pid.0 != 0 && (count <= LOCAL_ID_MASK || !in_use(pid)) {
            return pid;
        }
    }
}

impl fmt::Debug for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PID({})", self.0)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let node = self.node_id();
        let creation = self.creation();
        if node == 0 && creation == 0 {
            // Backward-compatible format for local PIDs.
            write!(f, "<0.{}>", self.local_id())
        } else {
            // Extended format for remote PIDs: <node_id.local_id.creation>
            write!(f, "<{}.{}.{}>", node, self.local_id(), creation)
        }
    }
}

// ---------------------------------------------------------------------------
// ProcessState
// ---------------------------------------------------------------------------

/// The execution state of a process.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessState {
    /// Ready to be scheduled (in a run queue).
    Ready,
    /// Currently executing on a worker thread.
    Running,
    /// Blocked waiting for a message (selective receive).
    Waiting,
    /// Terminated with the given reason.
    Exited(ExitReason),
}

// ---------------------------------------------------------------------------
// ExitReason
// ---------------------------------------------------------------------------

/// Why a process terminated.
#[derive(Debug, Clone, PartialEq)]
pub enum ExitReason {
    /// Normal completion -- the actor's entry function returned.
    Normal,
    /// Clean supervisor-initiated shutdown.
    ///
    /// Treated as non-crashing for exit propagation (like Normal).
    /// Transient children do NOT restart on Shutdown.
    Shutdown,
    /// Runtime error (e.g., pattern match failure, division by zero).
    Error(String),
    /// Explicitly killed via `Process.exit(pid, :kill)`.
    Killed,
    /// Linked process exited, propagating its reason.
    Linked(ProcessId, Box<ExitReason>),
    /// User-defined exit reason.
    ///
    /// Treated as crashing for exit propagation (like Error).
    Custom(String),
    /// Node connection lost -- the remote process may still be alive.
    /// Delivered to linked processes when the remote node disconnects.
    Noconnection,
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// Scheduling priority for a process.
///
/// Higher-priority processes are dequeued before normal and low-priority ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    High,
    Normal,
    Low,
}

impl Priority {
    /// Convert from a raw u8 (used in the extern "C" ABI).
    /// 0 = High, 1 = Normal (default), 2 = Low.
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Priority::High,
            2 => Priority::Low,
            _ => Priority::Normal,
        }
    }
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// A message in an actor's mailbox.
///
/// Contains a `MessageBuffer` with serialized data and a type tag for
/// pattern matching dispatch. Messages are deep-copied between actor heaps
/// on send to maintain complete isolation.
#[derive(Debug, Clone)]
pub struct Message {
    /// The serialized message payload with type tag.
    pub buffer: MessageBuffer,
}

// ---------------------------------------------------------------------------
// TerminateCallback
// ---------------------------------------------------------------------------

/// Callback invoked before an actor fully terminates.
///
/// The runtime calls this (if set) before exit-reason propagation to linked
/// processes. The compiled `terminate do ... end` block in a Mesh actor
/// generates a function matching this signature.
///
/// - `state_ptr`: pointer to the actor's current state (GenServer state, etc.)
/// - `reason_ptr`: pointer to a serialized `ExitReason`
pub type TerminateCallback = extern "C" fn(state_ptr: *const u8, reason_ptr: *const u8);

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default number of reductions before a process yields.
///
/// Chosen to balance responsiveness with context-switch overhead.
/// Matches BEAM's approach of preemptive reduction counting.
pub const DEFAULT_REDUCTIONS: u32 = 4000;

/// Default coroutine stack size: 512 KiB.
///
/// Virtual memory lazy-commits pages, so actors with 512 KiB virtual
/// stacks remain feasible on modern systems. The larger size accommodates
/// deep call chains in compiled Mesh handlers (service calls, DB queries,
/// JSON construction) that overflow 64 KiB stacks under concurrent load.
pub const DEFAULT_STACK_SIZE: usize = 512 * 1024;

// ---------------------------------------------------------------------------
// Process (the PCB)
// ---------------------------------------------------------------------------

/// The Process Control Block -- one per actor.
///
/// Contains all per-actor state: identity, scheduling metadata, mailbox,
/// linked processes, and an optional cleanup callback.
pub struct Process {
    /// Unique process identifier.
    pub pid: ProcessId,

    /// Current execution state.
    pub state: ProcessState,

    /// Scheduling priority.
    pub priority: Priority,

    /// Remaining reductions before this process yields.
    /// Reset to `DEFAULT_REDUCTIONS` after each yield.
    pub reductions: u32,

    /// Linked processes. When this process exits, the exit reason is
    /// propagated to all linked PIDs.
    pub links: HashSet<ProcessId>,

    /// When true, exit signals from linked processes are delivered as
    /// regular messages instead of causing this process to crash.
    /// Used by supervisors to monitor child processes.
    pub trap_exit: bool,

    /// The monitors this process set up, by reference.
    pub monitors: FxHashMap<u64, Monitor>,
    /// Processes monitoring this process. Maps monitor_ref -> monitoring_pid.
    pub monitored_by: FxHashMap<u64, ProcessId>,

    /// FIFO mailbox for incoming messages.
    /// Wrapped in Arc for thread-safe access from sender threads.
    pub mailbox: Arc<Mailbox>,

    /// Per-actor bump allocator heap for memory allocation.
    /// Each actor has its own heap to avoid global arena contention
    /// and enable per-actor memory reclamation.
    pub heap: ActorHeap,

    /// Embedded calls have no coroutine GC and release their heap on return.
    pub(crate) library_call: bool,
    /// The main thread's process. It is not a coroutine and never yields, so
    /// instead of collecting at a yield it collects at a reduction check, once
    /// the allocator has asked for it (`YielderSlot::gc_wanted`).
    pub(crate) collects_at_safepoints: bool,
    /// The scheduler worker that runs this process, once one has started it:
    /// a coroutine never leaves the thread that created it. Lets a waker
    /// unpark that worker and no other.
    pub(crate) worker: Option<usize>,
    /// Spawn arguments can borrow an embedded call's heap beyond its return.
    pub(crate) library_heap_owner: Option<Arc<Mutex<Process>>>,

    /// This actor's own copy of its spawn-argument buffer; see `Scheduler::spawn`.
    pub(crate) spawn_args: Option<Box<[u64]>>,
    /// Heaps that this actor's spawn arguments and received messages point
    /// into. Held until the process is dropped, not merely exited: an actor
    /// it lent to may still reach those heaps through objects in this one.
    // ponytail: two actors that lend to each other keep each other's process
    // alive after both exit. Only uncopyable values are lent (closures,
    // opaque runtime objects); make closure environments self-describing so
    // they copy too if that ever shows up.
    pub(crate) heap_borrows: Vec<HeapBorrow>,

    /// Optional cleanup callback invoked before termination.
    /// Set when the actor defines a `terminate do ... end` block.
    pub terminate_callback: Option<TerminateCallback>,

    /// Set when the scheduler has claimed responsibility for final callbacks,
    /// notifications, process-table removal, and active-count accounting.
    exit_finalization_started: bool,

    /// Base address of this actor's coroutine stack (highest address).
    /// Set when the coroutine body starts executing. Used by the GC to
    /// determine stack scanning bounds.
    pub stack_base: *const u8,
}

// Process contains raw pointer (stack_base) but it is only used from the
// owning actor's thread context.
unsafe impl Send for Process {}

/// One actor's claim on objects in another actor's heap.
///
/// Used for references that cross actors without being copied. While this
/// exists the owner's collector treats the lent objects as roots, and the
/// owner's heap stays mapped even if the owner exits first.
#[derive(Clone)]
pub(crate) struct HeapBorrow {
    /// `None` for a loan from the borrower's own heap (a message to itself),
    /// which needs no pin and must not make the process own itself.
    pub(crate) owner: Option<Arc<Mutex<Process>>>,
    /// Dropping this ends the loan; see `ActorHeap::lend`.
    pub(crate) _lent: Arc<()>,
}

impl fmt::Debug for HeapBorrow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HeapBorrow")
    }
}

/// A monitor a process set up: the message it gets when `target` ends.
pub struct Monitor {
    pub target: ProcessId,
    pub message: super::heap::MessageBuffer,
}

impl Process {
    /// Queue the message of monitor `monitor_ref`, whose process has ended,
    /// and forget the monitor. False when there is none, as after
    /// `Process.demonitor`.
    pub(crate) fn fire_monitor(&mut self, monitor_ref: u64) -> bool {
        let Some(monitor) = self.monitors.remove(&monitor_ref) else {
            return false;
        };
        // A notice the process may be waiting on, as a service call waits on
        // its service: it goes in even when the mailbox is full.
        let _ = self.mailbox.try_push_control(Message {
            buffer: monitor.message,
        });
        true
    }

    /// Create a new process with the given PID and priority.
    pub fn new(pid: ProcessId, priority: Priority) -> Self {
        Process {
            pid,
            state: ProcessState::Ready,
            priority,
            reductions: DEFAULT_REDUCTIONS,
            links: HashSet::new(),
            trap_exit: false,
            monitors: FxHashMap::default(),
            monitored_by: FxHashMap::default(),
            mailbox: Arc::new(Mailbox::new()),
            heap: ActorHeap::new(),
            library_call: false,
            collects_at_safepoints: false,
            worker: None,
            library_heap_owner: None,
            spawn_args: None,
            heap_borrows: Vec::new(),
            terminate_callback: None,
            exit_finalization_started: false,
            stack_base: std::ptr::null(),
        }
    }

    /// Move a live actor between scheduler states without resurrecting an exit.
    pub(crate) fn set_live_state(&mut self, state: ProcessState) -> bool {
        debug_assert!(!matches!(&state, ProcessState::Exited(_)));
        if matches!(self.state, ProcessState::Exited(_)) {
            return false;
        }
        self.state = state;
        true
    }

    /// Transition this process to an exited state, preserving the first reason,
    /// and immediately destroy actor-owned secrets for forced exits.
    pub(crate) fn mark_exited(&mut self, reason: ExitReason) -> bool {
        if self.exit_finalization_started {
            return false;
        }
        let transitioned = if matches!(self.state, ProcessState::Exited(_)) {
            false
        } else {
            self.state = ProcessState::Exited(reason);
            true
        };
        crate::secret::destroy_owned(self.pid);
        transitioned
    }

    /// Claim exactly-once scheduler finalization and resolve the authoritative
    /// exit reason. Natural exits defer secret cleanup until after terminate.
    pub(crate) fn begin_exit_finalization(
        &mut self,
        fallback_reason: ExitReason,
    ) -> Option<ExitReason> {
        if self.exit_finalization_started {
            return None;
        }
        let reason = match &self.state {
            ProcessState::Exited(reason) => reason.clone(),
            ProcessState::Ready | ProcessState::Running | ProcessState::Waiting => {
                self.state = ProcessState::Exited(fallback_reason.clone());
                fallback_reason
            }
        };
        self.exit_finalization_started = true;
        Some(reason)
    }
}

impl fmt::Debug for Process {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("pid", &self.pid)
            .field("state", &self.state)
            .field("priority", &self.priority)
            .field("reductions", &self.reductions)
            .field("links", &self.links)
            .field("mailbox_len", &self.mailbox.len())
            .field("heap_bytes", &self.heap.total_bytes())
            .field("has_terminate_cb", &self.terminate_callback.is_some())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid_unique() {
        let pids: Vec<ProcessId> = (0..100).map(|_| ProcessId::next()).collect();
        // All PIDs should be distinct.
        let mut seen = std::collections::HashSet::new();
        for pid in &pids {
            assert!(seen.insert(pid.0), "Duplicate PID: {}", pid.0);
        }
    }

    #[test]
    fn test_pid_concurrent_unique() {
        use std::sync::Arc;
        use std::sync::Mutex;

        let all_pids = Arc::new(Mutex::new(Vec::new()));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let pids = Arc::clone(&all_pids);
                std::thread::spawn(move || {
                    let local: Vec<u64> = (0..100).map(|_| ProcessId::next().as_u64()).collect();
                    pids.lock().unwrap().extend(local);
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let pids = all_pids.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        for &pid in pids.iter() {
            assert!(seen.insert(pid), "Duplicate PID under concurrency: {}", pid);
        }
        assert_eq!(pids.len(), 800);
    }

    #[test]
    fn test_process_new() {
        let pid = ProcessId::next();
        let proc = Process::new(pid, Priority::Normal);
        assert_eq!(proc.reductions, DEFAULT_REDUCTIONS);
        assert!(proc.links.is_empty());
        assert!(proc.mailbox.is_empty()); // Mailbox::is_empty()
        assert!(proc.terminate_callback.is_none());
        assert!(matches!(proc.state, ProcessState::Ready));
    }

    #[test]
    fn exited_process_preserves_first_reason_and_rejects_live_transitions() {
        let mut process = Process::new(ProcessId::next(), Priority::Normal);

        assert!(process.mark_exited(ExitReason::Killed));
        assert!(!process.set_live_state(ProcessState::Ready));
        assert!(!process.mark_exited(ExitReason::Normal));

        assert!(matches!(
            process.state,
            ProcessState::Exited(ExitReason::Killed)
        ));
    }

    #[test]
    fn test_priority_from_u8() {
        assert_eq!(Priority::from_u8(0), Priority::High);
        assert_eq!(Priority::from_u8(1), Priority::Normal);
        assert_eq!(Priority::from_u8(2), Priority::Low);
        assert_eq!(Priority::from_u8(255), Priority::Normal); // default
    }

    #[test]
    fn test_process_debug() {
        let pid = ProcessId::next();
        let proc = Process::new(pid, Priority::High);
        let dbg = format!("{:?}", proc);
        assert!(dbg.contains("Process"));
        assert!(dbg.contains("High"));
    }

    #[test]
    fn test_pid_bit_packing_roundtrip() {
        let pid = ProcessId::from_remote(5, 3, 42);
        assert_eq!(pid.node_id(), 5);
        assert_eq!(pid.creation(), 3);
        assert_eq!(pid.local_id(), 42);
    }

    #[test]
    fn test_pid_local_is_local() {
        let pid = ProcessId::next();
        assert!(pid.is_local());
        assert_eq!(pid.node_id(), 0);
        assert_eq!(pid.creation(), 0);
    }

    #[test]
    fn test_pid_remote_is_not_local() {
        let pid = ProcessId::from_remote(1, 0, 99);
        assert!(!pid.is_local());
    }

    #[test]
    fn test_pid_display_local_unchanged() {
        // Local PID with raw value 42 should display as "<0.42>".
        let pid = ProcessId(42);
        assert_eq!(format!("{}", pid), "<0.42>");
    }

    #[test]
    fn test_pid_display_remote() {
        let pid = ProcessId::from_remote(5, 2, 42);
        assert_eq!(format!("{}", pid), "<5.42.2>");
    }

    /// Once the counter wraps, a fresh PID skips 0 and every id a live
    /// process holds; before, nothing is looked up.
    #[test]
    fn a_wrapped_counter_skips_zero_and_live_ids() {
        let counter = AtomicU64::new(LOCAL_ID_MASK - 1);
        let never = |_: ProcessId| -> bool { panic!("looked up before the counter wrapped") };
        assert_eq!(next_local_id(&counter, never), ProcessId(LOCAL_ID_MASK - 1));
        assert_eq!(next_local_id(&counter, never), ProcessId(LOCAL_ID_MASK));
        let live = [ProcessId(1), ProcessId(2), ProcessId(4)];
        let in_use = |pid: ProcessId| live.contains(&pid);
        assert_eq!(next_local_id(&counter, in_use), ProcessId(3));
        assert_eq!(next_local_id(&counter, in_use), ProcessId(5));
    }

    #[test]
    fn test_pid_next_masked() {
        // Verify that ProcessId::next() produces a value where local_id
        // equals the raw value (no spillover into creation/node_id bits).
        let pid = ProcessId::next();
        assert_eq!(pid.local_id(), pid.as_u64());
    }
}
