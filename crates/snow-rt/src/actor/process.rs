//! Process Control Block (PCB) for Snow actors.
//!
//! Each Snow actor is a lightweight process with its own PID, state, priority,
//! reduction counter, mailbox, and optional terminate callback. Processes are
//! multiplexed across OS threads by the M:N scheduler.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// ProcessId
// ---------------------------------------------------------------------------

/// Unique identifier for an actor process.
///
/// PIDs are assigned sequentially from a global atomic counter, guaranteeing
/// uniqueness within a single runtime instance.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u64);

impl ProcessId {
    /// Generate a fresh, globally unique PID.
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        ProcessId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Return the raw numeric value.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PID({})", self.0)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<0.{}>", self.0)
    }
}

// ---------------------------------------------------------------------------
// ProcessState
// ---------------------------------------------------------------------------

/// The execution state of a process.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub enum ExitReason {
    /// Normal completion -- the actor's entry function returned.
    Normal,
    /// Runtime error (e.g., pattern match failure, division by zero).
    Error(String),
    /// Explicitly killed via `Process.exit(pid, :kill)`.
    Killed,
    /// Linked process exited, propagating its reason.
    Linked(ProcessId, Box<ExitReason>),
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
// Message (placeholder)
// ---------------------------------------------------------------------------

/// A message in an actor's mailbox.
///
/// This is a placeholder for Plan 03 which will add proper deep-copy
/// semantics and typed message handling.
#[derive(Debug, Clone)]
pub struct Message {
    /// Raw message payload.
    pub data: Vec<u8>,
    /// Type discriminator for pattern matching on message type.
    pub type_tag: u64,
}

// ---------------------------------------------------------------------------
// TerminateCallback
// ---------------------------------------------------------------------------

/// Callback invoked before an actor fully terminates.
///
/// The runtime calls this (if set) before exit-reason propagation to linked
/// processes. The compiled `terminate do ... end` block in a Snow actor
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

/// Default coroutine stack size: 64 KiB.
///
/// Virtual memory lazy-commits pages, so 100K actors each with 64 KiB
/// virtual stacks is feasible on modern systems.
pub const DEFAULT_STACK_SIZE: usize = 64 * 1024;

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
    /// propagated to all linked PIDs. Populated in Plan 06.
    pub links: Vec<ProcessId>,

    /// FIFO mailbox for incoming messages. Populated in Plan 03.
    pub mailbox: VecDeque<Message>,

    /// Optional cleanup callback invoked before termination.
    /// Set when the actor defines a `terminate do ... end` block.
    pub terminate_callback: Option<TerminateCallback>,
}

impl Process {
    /// Create a new process with the given PID and priority.
    pub fn new(pid: ProcessId, priority: Priority) -> Self {
        Process {
            pid,
            state: ProcessState::Ready,
            priority,
            reductions: DEFAULT_REDUCTIONS,
            links: Vec::new(),
            mailbox: VecDeque::new(),
            terminate_callback: None,
        }
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
        assert!(proc.mailbox.is_empty());
        assert!(proc.terminate_callback.is_none());
        assert!(matches!(proc.state, ProcessState::Ready));
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
}
