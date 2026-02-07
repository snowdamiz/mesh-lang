//! M:N work-stealing scheduler for Snow actors.
//!
//! The scheduler multiplexes lightweight actor processes across a fixed number
//! of OS threads (one per CPU core by default). Work distribution uses
//! crossbeam-deque for lock-free work-stealing.
//!
//! ## Design
//!
//! Since corosensei `Coroutine` is `!Send`, coroutines cannot move between
//! threads. The scheduler addresses this by:
//!
//! 1. **Spawn requests** (function pointer + args) are placed in the global
//!    queue and crossbeam-deque work-stealing deques. These are `Send`.
//! 2. **Each worker thread** pops spawn requests, creates coroutines locally,
//!    and runs them. Yielded coroutines stay in the worker's local suspended
//!    list and are resumed on the same thread.
//! 3. **Work-stealing** operates on spawn requests only -- new work is
//!    distributed, but running coroutines are thread-pinned.
//!
//! ## Priority
//!
//! Three priority levels: High, Normal, Low.
//! - High-priority spawn requests are placed in a dedicated channel and
//!   checked first by each worker.
//! - Low-priority requests go to the end of the global queue.
//! - Normal priority uses the work-stealing deques for best locality.

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use parking_lot::{Mutex, RwLock};
use rustc_hash::FxHashMap;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use super::process::{ExitReason, Priority, Process, ProcessId, ProcessState, DEFAULT_REDUCTIONS};
use super::stack::{clear_current_pid, set_current_pid, CoroutineHandle, CURRENT_YIELDER};

// ---------------------------------------------------------------------------
// SpawnRequest
// ---------------------------------------------------------------------------

/// A request to spawn a new actor. This is `Send` and can be distributed
/// across worker threads via work-stealing.
#[allow(dead_code)]
struct SpawnRequest {
    pid: ProcessId,
    fn_ptr: *const u8,
    args_ptr: *const u8,
    priority: Priority,
}

// Safety: The fn_ptr and args_ptr are owned by the runtime and the actor
// entry function is safe to call from any thread. The runtime guarantees
// these pointers remain valid until the actor completes.
unsafe impl Send for SpawnRequest {}

// ---------------------------------------------------------------------------
// ProcessTable
// ---------------------------------------------------------------------------

/// Shared process table for PID lookups across all worker threads.
type ProcessTable = Arc<RwLock<FxHashMap<ProcessId, Arc<Mutex<Process>>>>>;

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// The M:N work-stealing scheduler.
///
/// Manages a pool of OS worker threads, each with a local work-stealing deque.
/// New actors are enqueued as spawn requests and distributed to workers.
pub struct Scheduler {
    /// Number of OS worker threads.
    num_threads: usize,

    /// Global injector queue for spawn requests (normal + low priority).
    /// Workers steal from this when their local deque is empty.
    injector: Arc<Injector<SpawnRequest>>,

    /// High-priority channel -- checked first by all workers.
    high_priority_tx: Sender<SpawnRequest>,
    high_priority_rx: Receiver<SpawnRequest>,

    /// Stealers for each worker's local deque (for cross-thread stealing).
    stealers: Vec<Stealer<SpawnRequest>>,

    /// Worker deques are created per-thread; stealers are extracted at creation.
    /// We only store the stealers here; Workers are moved into their threads.
    /// This vec is populated during `new()` and consumed during `run()`.
    workers: Vec<Option<Worker<SpawnRequest>>>,

    /// Shared process table for PID lookup.
    process_table: ProcessTable,

    /// Shutdown flag -- set when the main actor exits.
    shutdown: Arc<AtomicBool>,

    /// Count of active (non-exited) processes.
    active_count: Arc<AtomicU64>,
}

impl Scheduler {
    /// Create a new scheduler with the given number of worker threads.
    ///
    /// If `num_threads` is 0, defaults to the number of available CPU cores.
    pub fn new(num_threads: u32) -> Self {
        let num_threads = if num_threads == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        } else {
            num_threads as usize
        };

        let injector = Arc::new(Injector::new());
        let (high_tx, high_rx) = crossbeam_channel::unbounded();

        let mut workers = Vec::with_capacity(num_threads);
        let mut stealers = Vec::with_capacity(num_threads);

        for _ in 0..num_threads {
            let w = Worker::new_lifo();
            stealers.push(w.stealer());
            workers.push(Some(w));
        }

        Scheduler {
            num_threads,
            injector,
            high_priority_tx: high_tx,
            high_priority_rx: high_rx,
            stealers,
            workers,
            process_table: Arc::new(RwLock::new(FxHashMap::default())),
            shutdown: Arc::new(AtomicBool::new(false)),
            active_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Spawn a new actor process.
    ///
    /// Creates a Process entry in the process table and enqueues a spawn
    /// request for a worker thread to pick up.
    ///
    /// Returns the PID of the new process.
    pub fn spawn(
        &self,
        fn_ptr: *const u8,
        args_ptr: *const u8,
        _args_size: u64,
        priority: u8,
    ) -> ProcessId {
        let pid = ProcessId::next();
        let priority = Priority::from_u8(priority);

        // Create process entry in the table.
        let process = Process::new(pid, priority);
        let process = Arc::new(Mutex::new(process));
        self.process_table.write().insert(pid, process);

        // Track active process count.
        self.active_count.fetch_add(1, Ordering::SeqCst);

        // Enqueue spawn request.
        let request = SpawnRequest {
            pid,
            fn_ptr,
            args_ptr,
            priority,
        };

        match priority {
            Priority::High => {
                let _ = self.high_priority_tx.send(request);
            }
            _ => {
                self.injector.push(request);
            }
        }

        pid
    }

    /// Run the scheduler, spawning worker threads and blocking until shutdown.
    ///
    /// Workers run in a loop, picking up spawn requests, creating coroutines,
    /// and executing actors. The scheduler shuts down when the shutdown flag
    /// is set and all active processes have exited.
    pub fn run(&mut self) {
        let num_threads = self.num_threads;

        crossbeam_utils::thread::scope(|scope| {
            for i in 0..num_threads {
                let worker = self.workers[i]
                    .take()
                    .expect("worker already consumed");

                let injector = Arc::clone(&self.injector);
                let high_rx = self.high_priority_rx.clone();
                let stealers: Vec<_> = self
                    .stealers
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| *idx != i)
                    .map(|(_, s)| s.clone())
                    .collect();
                let shutdown = Arc::clone(&self.shutdown);
                let active_count = Arc::clone(&self.active_count);
                let process_table = Arc::clone(&self.process_table);

                scope.spawn(move |_| {
                    worker_loop(
                        worker,
                        injector,
                        high_rx,
                        stealers,
                        shutdown,
                        active_count,
                        process_table,
                    );
                });
            }
        })
        .expect("scheduler threads panicked");
    }

    /// Signal the scheduler to shut down.
    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Check if shutdown has been signaled.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Get the number of active (non-exited) processes.
    pub fn active_count(&self) -> u64 {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Look up a process by PID.
    pub fn get_process(&self, pid: ProcessId) -> Option<Arc<Mutex<Process>>> {
        self.process_table.read().get(&pid).cloned()
    }
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("num_threads", &self.num_threads)
            .field("shutdown", &self.shutdown.load(Ordering::Relaxed))
            .field("active_count", &self.active_count.load(Ordering::Relaxed))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

/// The main loop for each worker thread.
///
/// 1. Check high-priority channel
/// 2. Pop from local deque (LIFO for cache locality)
/// 3. Try to steal from global injector
/// 4. Try to steal from other workers' deques
/// 5. If a spawn request is found: create coroutine, run actor
/// 6. After actor yields: add to local suspended list, re-run later
/// 7. After actor completes: mark exited, decrement active count
fn worker_loop(
    local: Worker<SpawnRequest>,
    injector: Arc<Injector<SpawnRequest>>,
    high_rx: Receiver<SpawnRequest>,
    stealers: Vec<Stealer<SpawnRequest>>,
    shutdown: Arc<AtomicBool>,
    active_count: Arc<AtomicU64>,
    process_table: ProcessTable,
) {
    // Local list of suspended coroutines (yielded, waiting to resume).
    // These are !Send so they must stay on this thread.
    let mut suspended: Vec<(ProcessId, CoroutineHandle)> = Vec::new();

    let mut spin_count: u32 = 0;

    loop {
        let mut did_work = false;

        // --- Phase 1: Run suspended coroutines (they have priority) ---
        // Drain suspended list, resuming each. If still not done, re-add.
        let mut still_suspended = Vec::new();
        for (pid, mut handle) in suspended.drain(..) {
            did_work = true;

            // Set thread-local PID for snow_actor_self().
            set_current_pid(pid);

            let yielded = handle.resume();

            // Clear thread-local context after resume returns.
            clear_current_pid();
            CURRENT_YIELDER.with(|c| c.set(None));

            if yielded {
                // Still running -- re-suspend.
                // Reset reductions for next timeslice.
                if let Some(proc) = process_table.read().get(&pid) {
                    let mut proc = proc.lock();
                    proc.reductions = DEFAULT_REDUCTIONS;
                    proc.state = ProcessState::Ready;
                }
                still_suspended.push((pid, handle));
            } else {
                // Actor completed.
                mark_exited(&process_table, pid, ExitReason::Normal);
                active_count.fetch_sub(1, Ordering::SeqCst);
            }
        }
        suspended = still_suspended;

        // --- Phase 2: Try to get new spawn requests ---
        let request = try_get_request(&local, &injector, &high_rx, &stealers);

        if let Some(req) = request {
            did_work = true;

            // Create coroutine on this thread.
            let mut handle = CoroutineHandle::new(req.fn_ptr, req.args_ptr);

            // Mark process as running.
            if let Some(proc) = process_table.read().get(&req.pid) {
                proc.lock().state = ProcessState::Running;
            }

            // Set thread-local PID.
            set_current_pid(req.pid);

            let yielded = handle.resume();

            // Clear thread-local context after resume returns.
            clear_current_pid();
            CURRENT_YIELDER.with(|c| c.set(None));

            if yielded {
                // Actor yielded -- add to suspended list.
                if let Some(proc) = process_table.read().get(&req.pid) {
                    let mut proc = proc.lock();
                    proc.reductions = DEFAULT_REDUCTIONS;
                    proc.state = ProcessState::Ready;
                }
                suspended.push((req.pid, handle));
            } else {
                // Actor completed on first run.
                mark_exited(&process_table, req.pid, ExitReason::Normal);
                active_count.fetch_sub(1, Ordering::SeqCst);
            }
        }

        // --- Phase 3: Check shutdown ---
        if shutdown.load(Ordering::SeqCst) && active_count.load(Ordering::SeqCst) == 0 {
            break;
        }

        // Backoff when idle to avoid burning CPU.
        if !did_work {
            spin_count += 1;
            if spin_count > 100 {
                std::thread::sleep(std::time::Duration::from_micros(100));
                if spin_count > 1000 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            } else {
                std::hint::spin_loop();
            }
        } else {
            spin_count = 0;
        }
    }
}

/// Try to get a spawn request from available sources.
///
/// Priority order:
/// 1. High-priority channel
/// 2. Local deque (LIFO for cache locality)
/// 3. Global injector
/// 4. Steal from other workers
fn try_get_request(
    local: &Worker<SpawnRequest>,
    injector: &Injector<SpawnRequest>,
    high_rx: &Receiver<SpawnRequest>,
    stealers: &[Stealer<SpawnRequest>],
) -> Option<SpawnRequest> {
    // 1. High priority
    match high_rx.try_recv() {
        Ok(req) => return Some(req),
        Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
    }

    // 2. Local deque
    if let Some(req) = local.pop() {
        return Some(req);
    }

    // 3. Global injector
    loop {
        match injector.steal_batch_and_pop(local) {
            Steal::Success(req) => return Some(req),
            Steal::Empty => break,
            Steal::Retry => continue,
        }
    }

    // 4. Steal from other workers
    for stealer in stealers {
        loop {
            match stealer.steal() {
                Steal::Success(req) => return Some(req),
                Steal::Empty => break,
                Steal::Retry => continue,
            }
        }
    }

    None
}

/// Mark a process as exited in the process table.
fn mark_exited(process_table: &ProcessTable, pid: ProcessId, reason: ExitReason) {
    if let Some(proc) = process_table.read().get(&pid) {
        let mut proc = proc.lock();
        proc.state = ProcessState::Exited(reason);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static SPAWN_COUNTER: AtomicU64 = AtomicU64::new(0);

    extern "C" fn increment_entry(_args: *const u8) {
        SPAWN_COUNTER.fetch_add(1, Ordering::SeqCst);
    }

    /// Stable thread identifier using Hash of ThreadId.
    fn thread_id_hash() -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn test_spawn_unique_pids() {
        let sched = Scheduler::new(2);
        let pids: Vec<ProcessId> = (0..10)
            .map(|_| sched.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1))
            .collect();

        let mut seen = std::collections::HashSet::new();
        for pid in &pids {
            assert!(seen.insert(pid.as_u64()), "Duplicate PID: {}", pid);
        }
        assert_eq!(seen.len(), 10);
    }

    #[test]
    fn test_single_actor_completes() {
        let initial = SPAWN_COUNTER.load(Ordering::SeqCst);
        let mut sched = Scheduler::new(1);
        sched.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1);
        sched.signal_shutdown();
        sched.run();

        let delta = SPAWN_COUNTER.load(Ordering::SeqCst) - initial;
        assert!(delta >= 1, "Expected at least 1 actor to complete, got delta={}", delta);
    }

    #[test]
    fn test_multiple_actors_complete() {
        let initial = SPAWN_COUNTER.load(Ordering::SeqCst);
        let num_actors = 10;
        let mut sched = Scheduler::new(2);
        for _ in 0..num_actors {
            sched.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1);
        }
        sched.signal_shutdown();
        sched.run();

        let final_count = SPAWN_COUNTER.load(Ordering::SeqCst) - initial;
        assert!(
            final_count >= num_actors,
            "Expected at least {} actors to complete, got {}",
            num_actors,
            final_count
        );
    }

    #[test]
    fn test_work_stealing_distributes() {
        // Use a test-specific counter and thread-ID list to avoid
        // interference from concurrent tests sharing the global statics.
        static WS_COUNTER: AtomicU64 = AtomicU64::new(0);
        static WS_THREAD_IDS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

        extern "C" fn ws_record_entry(_args: *const u8) {
            WS_COUNTER.fetch_add(1, Ordering::SeqCst);
            let tid = thread_id_hash();
            WS_THREAD_IDS.lock().push(tid);
        }

        WS_COUNTER.store(0, Ordering::SeqCst);
        WS_THREAD_IDS.lock().clear();

        let num_actors = 100;
        let mut sched = Scheduler::new(4);
        for _ in 0..num_actors {
            sched.spawn(ws_record_entry as *const u8, std::ptr::null(), 0, 1);
        }
        sched.signal_shutdown();
        sched.run();

        let thread_ids = WS_THREAD_IDS.lock();
        let unique_threads: std::collections::HashSet<u64> = thread_ids.iter().cloned().collect();

        // With 100 actors across 4 threads, we should see work on multiple threads.
        // Allow at least 2 since work-stealing is best-effort.
        assert!(
            unique_threads.len() >= 2,
            "Expected work on at least 2 threads, got {} (thread IDs: {:?})",
            unique_threads.len(),
            unique_threads
        );
    }

    #[test]
    fn test_reduction_yield() {
        // The tight_loop_entry yields 5 times then increments counter.
        // It should still complete, proving yield/resume works.
        // Use a dedicated counter to avoid interference from concurrent tests.
        static YIELD_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn yield_entry(_args: *const u8) {
            for _ in 0..5 {
                super::super::stack::yield_current();
            }
            YIELD_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        YIELD_COUNTER.store(0, Ordering::SeqCst);
        let mut sched = Scheduler::new(2);
        sched.spawn(yield_entry as *const u8, std::ptr::null(), 0, 1);
        sched.signal_shutdown();
        sched.run();

        assert_eq!(
            YIELD_COUNTER.load(Ordering::SeqCst),
            1,
            "Yielding actor should still complete"
        );
    }

    #[test]
    fn test_reduction_yield_does_not_starve() {
        // Spawn a tight-loop actor and several simple actors.
        // All should complete, proving the yielding actor doesn't starve others.
        // Use a dedicated counter to avoid interference from concurrent tests.
        static STARVE_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn starve_yield_entry(_args: *const u8) {
            for _ in 0..5 {
                super::super::stack::yield_current();
            }
            STARVE_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        extern "C" fn starve_simple_entry(_args: *const u8) {
            STARVE_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        STARVE_COUNTER.store(0, Ordering::SeqCst);
        let mut sched = Scheduler::new(2);

        // One yielding actor
        sched.spawn(starve_yield_entry as *const u8, std::ptr::null(), 0, 1);
        // Five simple actors
        for _ in 0..5 {
            sched.spawn(starve_simple_entry as *const u8, std::ptr::null(), 0, 1);
        }

        sched.signal_shutdown();
        sched.run();

        assert_eq!(
            STARVE_COUNTER.load(Ordering::SeqCst),
            6,
            "All 6 actors (1 yielding + 5 simple) should complete"
        );
    }

    #[test]
    fn test_high_priority() {
        // Use a dedicated counter to avoid interference from concurrent tests.
        static PRIO_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn prio_entry(_args: *const u8) {
            PRIO_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        PRIO_COUNTER.store(0, Ordering::SeqCst);
        let mut sched = Scheduler::new(1);
        // Spawn low-priority actors
        for _ in 0..5 {
            sched.spawn(prio_entry as *const u8, std::ptr::null(), 0, 2); // Low
        }
        // Spawn high-priority actor
        sched.spawn(prio_entry as *const u8, std::ptr::null(), 0, 0); // High
        sched.signal_shutdown();
        sched.run();

        assert_eq!(PRIO_COUNTER.load(Ordering::SeqCst), 6, "All priority levels should complete");
    }

    #[test]
    fn test_100_actors_no_hang() {
        let initial = SPAWN_COUNTER.load(Ordering::SeqCst);
        let num_actors: u64 = 100;
        let mut sched = Scheduler::new(4);
        for _ in 0..num_actors {
            sched.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1);
        }
        sched.signal_shutdown();
        sched.run();

        let completed = SPAWN_COUNTER.load(Ordering::SeqCst) - initial;
        assert!(
            completed >= num_actors,
            "Expected at least {} actors, got {}",
            num_actors,
            completed
        );
    }
}
