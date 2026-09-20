//! M:N work-stealing scheduler for Mesh actors.
//!
//! The scheduler multiplexes lightweight actor processes across bounded OS
//! worker threads. Workers can be activated or retired at runtime. Work
//! distribution uses crossbeam-deque for lock-free work-stealing.
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

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use super::link;
use super::process::{
    ExitReason, HeapBorrow, Priority, Process, ProcessId, ProcessState, TerminateCallback,
    DEFAULT_REDUCTIONS,
};
use super::registry;
use super::stack::{
    clear_current_pid, get_current_pid, set_current_pid, CoroutineHandle, CURRENT_YIELDER,
};

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
/// One per worker thread, so that whoever makes work can wake the worker that
/// should run it rather than wait out its poll interval.
#[derive(Default)]
struct WorkerSleep {
    thread: std::sync::OnceLock<std::thread::Thread>,
    /// Set from the worker's first park until it finds work again.
    idle: AtomicBool,
    /// Processes of this worker that were Waiting and have been made Ready.
    woken: Mutex<Vec<ProcessId>>,
}

impl WorkerSleep {
    /// Unpark the worker if it is parked. Cheap for a busy one: an atomic load.
    fn wake(&self) -> bool {
        let idle = self.idle.load(Ordering::SeqCst);
        if let (true, Some(thread)) = (idle, self.thread.get()) {
            thread.unpark();
        }
        idle
    }
}

/// A worker's stretch without work. It counts as idle for the whole stretch,
/// scans included, so a wake during a scan is not lost: the unpark token it
/// leaves makes the next park return at once. Only a wake that lands before
/// the first park of a stretch waits out that one park, as every wake used to.
struct IdleStretch<'a> {
    worker: &'a WorkerSleep,
}

impl IdleStretch<'_> {
    fn park(&mut self, timeout: std::time::Duration) {
        self.worker.idle.store(true, Ordering::SeqCst);
        std::thread::park_timeout(timeout);
    }

    fn end(&mut self) {
        self.worker.idle.store(false, Ordering::SeqCst);
    }
}

impl Drop for IdleStretch<'_> {
    fn drop(&mut self) {
        self.end();
    }
}

pub struct Scheduler {
    /// Maximum number of initialized OS worker threads.
    num_threads: usize,

    /// Minimum number of workers that must remain active.
    min_threads: usize,

    /// Current target number of active workers.
    active_threads: Arc<AtomicUsize>,

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
    /// Wrapped in Mutex so `run()` can take `&self` instead of `&mut self`,
    /// allowing the Scheduler to be shared without an outer Mutex.
    workers: Mutex<Vec<Option<Worker<SpawnRequest>>>>,

    /// Shared process table for PID lookup.
    process_table: ProcessTable,

    /// Shutdown flag -- set when the main actor exits.
    shutdown: Arc<AtomicBool>,

    /// Count of active (non-exited) processes.
    active_count: Arc<AtomicU64>,

    /// Handles for background worker threads (populated by `start()`).
    worker_handles: Mutex<Vec<std::thread::JoinHandle<()>>>,

    /// Where each worker stands, indexed like the workers themselves.
    sleepers: Arc<Vec<WorkerSleep>>,
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

        Self::build(num_threads, num_threads)
    }

    /// Create a scheduler with runtime-resizable worker capacity.
    pub fn new_elastic(min_threads: u32, max_threads: u32) -> Result<Self, String> {
        if min_threads == 0 || max_threads == 0 || min_threads > max_threads {
            return Err("scheduler_worker_bounds_invalid".to_string());
        }
        Ok(Self::build(min_threads as usize, max_threads as usize))
    }

    fn build(min_threads: usize, max_threads: usize) -> Self {
        let injector = Arc::new(Injector::new());
        let (high_tx, high_rx) = crossbeam_channel::unbounded();

        let mut workers = Vec::with_capacity(max_threads);
        let mut stealers = Vec::with_capacity(max_threads);

        for _ in 0..max_threads {
            let w = Worker::new_lifo();
            stealers.push(w.stealer());
            workers.push(Some(w));
        }

        Scheduler {
            num_threads: max_threads,
            min_threads,
            active_threads: Arc::new(AtomicUsize::new(min_threads)),
            injector,
            high_priority_tx: high_tx,
            high_priority_rx: high_rx,
            stealers,
            workers: Mutex::new(workers),
            process_table: Arc::new(RwLock::new(FxHashMap::default())),
            shutdown: Arc::new(AtomicBool::new(false)),
            active_count: Arc::new(AtomicU64::new(0)),
            worker_handles: Mutex::new(Vec::new()),
            sleepers: Arc::new((0..max_threads).map(|_| WorkerSleep::default()).collect()),
        }
    }

    /// Change the active worker target without rebuilding scheduler state.
    pub fn resize(&self, desired: usize) -> Result<usize, String> {
        if desired < self.min_threads || desired > self.num_threads {
            return Err(format!(
                "scheduler_worker_target_out_of_bounds:{desired}:{}..={}",
                self.min_threads, self.num_threads
            ));
        }
        self.active_threads.store(desired, Ordering::Release);
        crate::dist::telemetry::runtime_telemetry().set_scheduler(
            desired.try_into().unwrap_or(u16::MAX),
            self.num_threads.try_into().unwrap_or(u16::MAX),
            self.runnable_count(),
        );
        Ok(desired)
    }

    pub fn worker_bounds(&self) -> (usize, usize) {
        (self.min_threads, self.num_threads)
    }

    pub fn active_workers(&self) -> usize {
        self.active_threads.load(Ordering::Acquire)
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
        args_size: u64,
        priority: u8,
    ) -> ProcessId {
        self.spawn_shaped(fn_ptr, args_ptr, args_size, priority, std::ptr::null())
    }

    /// `spawn` for arguments that reference heap values: `shape` describes the
    /// argument slots (see `msg_shape`) so the new actor gets its own copy.
    pub fn spawn_shaped(
        &self,
        fn_ptr: *const u8,
        args_ptr: *const u8,
        args_size: u64,
        priority: u8,
        shape: *const u32,
    ) -> ProcessId {
        let pid = ProcessId::next();
        let priority = Priority::from_u8(priority);

        // Create process entry in the table.
        let mut process = Process::new(pid, priority);
        let spawner = get_current_pid().and_then(|pid| self.get_process(pid));
        // ponytail: pin the entire call heap for actors borrowing its arguments;
        // use typed argument copying if long-lived actors make retention costly.
        process.library_heap_owner = spawner.as_ref().and_then(|owner| {
            let parent = owner.lock();
            if parent.library_call {
                Some(Arc::clone(owner))
            } else {
                parent.library_heap_owner.clone()
            }
        });
        let args_ptr = match &spawner {
            Some(spawner) => adopt_spawn_args(&mut process, spawner, args_ptr, args_size, shape),
            None => args_ptr,
        };
        let process = Arc::new(Mutex::new(process));
        self.process_table.write().insert(pid, process);

        // Track active process count.
        let active_count = self.active_count.fetch_add(1, Ordering::SeqCst) + 1;
        crate::dist::telemetry::runtime_telemetry().set_runnable_actors(active_count);

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
        // Any worker can take a queued request: wake one that has none.
        let _ = self.sleepers.iter().any(WorkerSleep::wake);

        pid
    }

    /// Start worker threads in the background.
    ///
    /// Workers run in a loop, picking up spawn requests, creating coroutines,
    /// and executing actors. Unlike `run()`, this returns immediately -- the
    /// worker threads run in the background. Call `wait()` to join them.
    ///
    /// This is used when the main thread needs to call into services (which
    /// require the scheduler to be running) before `mesh_main` returns.
    pub fn start(&self) {
        let num_threads = self.num_threads;
        let mut handles = self.worker_handles.lock();

        for i in 0..num_threads {
            let worker = self.workers.lock()[i]
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
            let active_threads = Arc::clone(&self.active_threads);
            let sleepers = Arc::clone(&self.sleepers);

            let handle = std::thread::spawn(move || {
                let shared = WorkerLoopShared {
                    injector,
                    shutdown,
                    active_count,
                    process_table,
                    active_threads,
                    sleepers,
                };
                worker_loop(i, worker, high_rx, stealers, shared);
            });
            handles.push(handle);
        }
    }

    /// Wait for all worker threads to complete.
    ///
    /// This blocks until all workers have exited (after shutdown is signaled
    /// and all active processes complete).
    pub fn wait(&self) {
        let handles: Vec<_> = self.worker_handles.lock().drain(..).collect();
        for handle in handles {
            let _ = handle.join();
        }
    }

    /// Run the scheduler, spawning worker threads and blocking until shutdown.
    ///
    /// Workers run in a loop, picking up spawn requests, creating coroutines,
    /// and executing actors. The scheduler shuts down when the shutdown flag
    /// is set and all active processes have exited.
    pub fn run(&self) {
        let num_threads = self.num_threads;

        crossbeam_utils::thread::scope(|scope| {
            for i in 0..num_threads {
                let worker = self.workers.lock()[i]
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
                let active_threads = Arc::clone(&self.active_threads);
                let sleepers = Arc::clone(&self.sleepers);

                scope.spawn(move |_| {
                    let shared = WorkerLoopShared {
                        injector,
                        shutdown,
                        active_count,
                        process_table,
                        active_threads,
                        sleepers,
                    };
                    worker_loop(i, worker, high_rx, stealers, shared);
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

    /// Count actors that can consume scheduler time now. Waiting actors remain
    /// live but are intentionally excluded from runnable pressure.
    pub fn runnable_count(&self) -> u64 {
        self.process_table
            .read()
            .values()
            .filter(|process| {
                matches!(
                    process.lock().state,
                    ProcessState::Ready | ProcessState::Running
                )
            })
            .count()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    /// Refresh the bounded scheduler signals consumed by load reports and the
    /// authenticated operator runtime view.
    pub(crate) fn refresh_telemetry(&self) {
        let worker_depths: Vec<_> = self.stealers.iter().map(Stealer::len).collect();
        let global_depth = self
            .injector
            .len()
            .saturating_add(self.high_priority_rx.len());
        let telemetry = crate::dist::telemetry::runtime_telemetry();
        telemetry.set_scheduler(
            self.active_workers().try_into().unwrap_or(u16::MAX),
            self.num_threads.try_into().unwrap_or(u16::MAX),
            self.runnable_count(),
        );
        telemetry.set_scheduler_queues(global_depth, &worker_depths);
    }

    /// Create a process entry for the main thread.
    ///
    /// This gives the main thread a PID and mailbox so that `mesh_service_call`
    /// can work from non-coroutine context. The main thread process is NOT
    /// counted in active_count because it is not managed by the scheduler --
    /// its lifetime is controlled by the C main function.
    pub fn create_main_process(&self) -> ProcessId {
        let pid = ProcessId::next();
        let mut process = Process::new(pid, Priority::Normal);
        process.set_live_state(ProcessState::Running);
        let process = Arc::new(Mutex::new(process));
        self.process_table.write().insert(pid, process);
        // Do NOT increment active_count -- main thread is not scheduler-managed.
        pid
    }

    pub(crate) fn finalize_host_process(&self, pid: ProcessId) {
        handle_process_exit(&self.process_table, pid, ExitReason::Normal);
    }

    /// Look up a process by PID.
    pub fn get_process(&self, pid: ProcessId) -> Option<Arc<Mutex<Process>>> {
        self.process_table.read().get(&pid).cloned()
    }

    /// Get a reference to the process table (for shutdown checks).
    pub fn process_table(&self) -> &ProcessTable {
        &self.process_table
    }

    /// Wake a process that was in Waiting state.
    ///
    /// This is called by `mesh_actor_send` after setting the process state
    /// to Ready. Since coroutines are `!Send` and thread-pinned, the actual
    /// resumption happens in the worker loop when it notices the state change.
    ///
    /// The wake mechanism is cooperative: the worker thread that owns the
    /// coroutine will see the Ready state on its next iteration and resume it.
    /// Tell the worker that runs `pid` that it has become Ready.
    ///
    /// The state change, made by the caller under the process lock, is what
    /// makes the process runnable. A worker does not look at its Waiting
    /// coroutines on every pass, though, and parks when it has nothing to run:
    /// this puts the process back on its run list and wakes it. A worker also
    /// sweeps its Waiting coroutines now and then, so a path that forgets to
    /// call this costs latency, not liveness. `worker` is `Process::worker`,
    /// read while the caller still held the lock; a process no worker has
    /// started yet sits in a queue, which has woken one already.
    pub fn wake_worker(&self, worker: Option<usize>, pid: ProcessId) {
        if let Some(worker) = worker.and_then(|index| self.sleepers.get(index)) {
            worker.woken.lock().push(pid);
            worker.wake();
        }
    }

    /// `wake_worker` for a caller that has already let go of the process lock.
    pub fn wake_process(&self, pid: ProcessId) {
        let worker = self
            .get_process(pid)
            .and_then(|process| process.lock().worker);
        self.wake_worker(worker, pid);
    }
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("num_threads", &self.num_threads)
            .field("min_threads", &self.min_threads)
            .field("active_threads", &self.active_workers())
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
struct WorkerLoopShared {
    injector: Arc<Injector<SpawnRequest>>,
    shutdown: Arc<AtomicBool>,
    active_count: Arc<AtomicU64>,
    process_table: ProcessTable,
    active_threads: Arc<AtomicUsize>,
    sleepers: Arc<Vec<WorkerSleep>>,
}

fn worker_loop(
    worker_index: usize,
    local: Worker<SpawnRequest>,
    high_rx: Receiver<SpawnRequest>,
    stealers: Vec<Stealer<SpawnRequest>>,
    shared: WorkerLoopShared,
) {
    let WorkerLoopShared {
        injector,
        shutdown,
        active_count,
        process_table,
        active_threads,
        sleepers,
    } = shared;
    // Coroutines this worker has started and not finished. They are !Send, so
    // they stay on this thread. The runnable ones are resumed on every pass.
    // The waiting ones are blocked in a receive or a sleep and are left alone
    // until something makes them Ready: looking at each of them on every pass,
    // a table lookup and a lock apiece, is what let a backlog of waiting actors
    // bring every worker to a crawl.
    let mut runnable: Vec<(ProcessId, CoroutineHandle)> = Vec::new();
    let mut waiting: FxHashMap<ProcessId, CoroutineHandle> = FxHashMap::default();
    let mut next_sweep = Instant::now();

    let mut spin_count: u32 = 0;
    let sleep = &sleepers[worker_index];
    let _ = sleep.thread.set(std::thread::current());
    let mut idle = IdleStretch { worker: sleep };

    loop {
        let cycle_started_at = Instant::now();
        let mut did_work = false;
        let accepting_new_work = worker_index < active_threads.load(Ordering::Acquire);

        if !accepting_new_work && runnable.is_empty() && waiting.is_empty() {
            if shutdown.load(Ordering::SeqCst) && active_count.load(Ordering::SeqCst) == 0 {
                break;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(1));
            crate::dist::telemetry::runtime_telemetry()
                .record_scheduler_cycle(false, cycle_started_at.elapsed());
            continue;
        }

        // --- Phase 1: Run suspended coroutines (they have priority) ---
        // Whoever made a waiting process Ready said so (`wake_worker`).
        let woken = std::mem::take(&mut *sleep.woken.lock());
        for pid in woken {
            if let Some(handle) = waiting.remove(&pid) {
                runnable.push((pid, handle));
            }
        }
        // Not every path does (links, supervisors, shutdown), so look at all of
        // them now and then: seldom enough that however many are waiting, the
        // looking stays a small part of this worker's time.
        if !waiting.is_empty() && cycle_started_at >= next_sweep {
            let ready: Vec<ProcessId> = waiting
                .keys()
                .copied()
                .filter(|pid| {
                    !process_table
                        .read()
                        .get(pid)
                        .is_some_and(|p| matches!(p.lock().state, ProcessState::Waiting))
                })
                .collect();
            for pid in ready {
                if let Some(handle) = waiting.remove(&pid) {
                    runnable.push((pid, handle));
                }
            }
            let cost = cycle_started_at.elapsed();
            next_sweep = Instant::now() + (cost * 8).max(std::time::Duration::from_millis(1));
        }
        for (pid, mut handle) in std::mem::take(&mut runnable) {
            did_work = true;
            match resume_process(&process_table, &active_count, pid, &mut handle) {
                Resumed::Runnable => runnable.push((pid, handle)),
                Resumed::Waiting => {
                    waiting.insert(pid, handle);
                }
                Resumed::Finished => {}
            }
        }

        // --- Phase 2: Try to get new spawn requests ---
        let request = accepting_new_work
            .then(|| try_get_request(&local, &injector, &high_rx, &stealers))
            .flatten();

        if let Some(req) = request {
            did_work = true;
            let exited_reason = process_table.read().get(&req.pid).and_then(|process| {
                let mut process = process.lock();
                match &process.state {
                    ProcessState::Exited(reason) => Some(reason.clone()),
                    ProcessState::Ready | ProcessState::Running | ProcessState::Waiting => {
                        process.set_live_state(ProcessState::Running);
                        process.worker = Some(worker_index);
                        None
                    }
                }
            });

            if let Some(reason) = exited_reason {
                finalize_managed_process(&process_table, &active_count, req.pid, reason);
            } else if process_table.read().contains_key(&req.pid) {
                // Create the coroutine only after confirming the queued actor is live.
                let mut handle = CoroutineHandle::new(req.fn_ptr, req.args_ptr);
                match resume_process(&process_table, &active_count, req.pid, &mut handle) {
                    Resumed::Runnable => runnable.push((req.pid, handle)),
                    Resumed::Waiting => {
                        waiting.insert(req.pid, handle);
                    }
                    Resumed::Finished => {}
                }
            }
        }

        // --- Phase 3: Check shutdown ---
        if shutdown.load(Ordering::SeqCst) {
            if active_count.load(Ordering::SeqCst) == 0 {
                break;
            }

            // Check if all locally suspended actors are in Waiting state with
            // no Ready actors remaining. If so, force-terminate them. This
            // handles service loops that block forever on receive after the
            // main actor has exited.
            let all_waiting = runnable.is_empty() && !waiting.is_empty();

            if all_waiting {
                // Check globally: are there any non-waiting active processes?
                // Count Ready/Running processes in the process table.
                let has_ready = process_table.read().values().any(|p| {
                    let state = p.lock().state.clone();
                    matches!(state, ProcessState::Ready | ProcessState::Running)
                });

                if !has_ready {
                    // No Ready/Running processes remain. Wake all Waiting
                    // actors so they can detect shutdown and exit gracefully.
                    // The mesh_actor_receive function checks is_shutdown()
                    // and returns null when no other actors are active,
                    // causing the service loop to exit cleanly.
                    for (pid, handle) in waiting.drain() {
                        if let Some(proc_arc) = process_table.read().get(&pid) {
                            let mut proc = proc_arc.lock();
                            if matches!(proc.state, ProcessState::Waiting) {
                                proc.set_live_state(ProcessState::Ready);
                            }
                        }
                        runnable.push((pid, handle));
                    }
                    // The actors will be resumed in Phase 1 on the next
                    // iteration, and will exit when receive returns null.
                }
            }

            // Also: if this worker has an empty suspended list, no pending
            // requests, and shutdown is active, exit the worker loop.
            if runnable.is_empty()
                && waiting.is_empty()
                && !did_work
                && active_count.load(Ordering::SeqCst) == 0
            {
                break;
            }
        }

        // Backoff when idle to avoid burning CPU: spin, then park for as long
        // as this used to sleep. `WorkerSleep::wake` cuts the park short.
        if !did_work {
            spin_count += 1;
            if spin_count > 1000 {
                idle.park(std::time::Duration::from_micros(1_100));
            } else if spin_count > 100 {
                idle.park(std::time::Duration::from_micros(100));
            } else {
                std::hint::spin_loop();
            }
        } else {
            spin_count = 0;
            idle.end();
        }
        crate::dist::telemetry::runtime_telemetry()
            .record_scheduler_cycle(did_work, cycle_started_at.elapsed());
    }
}

/// Have every heap that `words` point into keep those objects alive.
///
/// Covers `lender`'s own heap and, because it may be passing on something it
/// was itself lent, every heap it borrows from. Words that point nowhere
/// (integers, static data) produce no loan. Locks one process at a time.
pub(crate) fn lend_words(lender: &Arc<Mutex<Process>>, words: &[usize]) -> Vec<HeapBorrow> {
    let mut loans = Vec::new();
    if words.is_empty() {
        return loans;
    }
    let mut seen = Vec::new();
    let mut pending = vec![Arc::clone(lender)];
    while let Some(owner) = pending.pop() {
        let mut process = owner.lock();
        if seen.contains(&process.pid) {
            continue;
        }
        seen.push(process.pid);
        pending.extend(process.heap_borrows.iter().filter_map(|b| b.owner.clone()));
        if let Some(lent) = process.heap.lend(words) {
            drop(process);
            loans.push(HeapBorrow {
                owner: Some(owner),
                _lent: lent,
            });
        }
    }
    loans
}

/// Give a spawned actor arguments that survive its spawner's collections.
///
/// Compiled `spawn` packs its arguments into a buffer on the spawner's GC heap
/// and passes pointer-typed arguments by reference, but the child runs later,
/// on another thread, and a collector only scans its own actor's roots. So:
///
/// - The buffer is copied for the child when it is an object on the spawner's
///   heap. Any other pointer (a Rust `Box`, the global arena) belongs to the
///   caller and is handed over untouched.
/// - With a `shape`, what the arguments reference is copied into the child's
///   own heap, which nothing else can touch yet. Only what the shape cannot
///   describe (a closure's environment, an opaque runtime object) stays
///   shared.
/// - Whatever stays shared is lent by the heap that owns it, and that heap is
///   pinned by the child: the spawner's own, or an ancestor's when the spawner
///   passes on something it was itself given. Without a shape every argument
///   word is treated this way; words that point nowhere borrow nothing.
///
/// Returns the pointer the child's entry function should receive.
// ponytail: a lent heap stays whole until the borrower's process is dropped, so
// a chain of short-lived spawners that each hand an uncopyable value to the
// next pins every ancestor. Make closure environments self-describing so they
// copy too if that ever matters.
fn adopt_spawn_args(
    child: &mut Process,
    spawner: &Arc<Mutex<Process>>,
    args_ptr: *const u8,
    args_size: u64,
    shape: *const u32,
) -> *const u8 {
    let Ok(size) = usize::try_from(args_size) else {
        return args_ptr;
    };
    if args_ptr.is_null() || size == 0 {
        return args_ptr;
    }
    // Read once, up front: the buffer is alive for the duration of this call.
    let bytes = unsafe { std::slice::from_raw_parts(args_ptr, size) };
    let owns_buffer = spawner.lock().heap.is_live_allocation(args_ptr, size);

    let mut copy = vec![0u64; size.div_ceil(8)].into_boxed_slice();
    unsafe { std::ptr::copy_nonoverlapping(args_ptr, copy.as_mut_ptr() as *mut u8, size) };

    let shared: Vec<usize> = if owns_buffer && !shape.is_null() {
        let captured = unsafe { super::msg_shape::capture(&spawner.lock().heap, bytes, 0, shape) };
        unsafe { captured.materialize(&mut child.heap, copy.as_mut_ptr() as *mut u8) };
        captured.lend
    } else {
        copy[..size / 8].iter().map(|&word| word as usize).collect()
    };
    child.heap_borrows = lend_words(spawner, &shared);

    if !owns_buffer {
        return args_ptr;
    }
    let copied = copy.as_ptr() as *const u8;
    child.spawn_args = Some(copy);
    copied
}

/// Resume one actor timeslice and turn a Mesh panic into a linked error exit.
/// What became of a coroutine that was resumed.
enum Resumed {
    Finished,
    /// Preempted or yielded: resume it again on the next pass.
    Runnable,
    /// Blocked in a receive or a sleep until something makes it Ready.
    Waiting,
}

fn resume_process(
    process_table: &ProcessTable,
    active_count: &AtomicU64,
    pid: ProcessId,
    handle: &mut CoroutineHandle,
) -> Resumed {
    let exited_reason =
        process_table
            .read()
            .get(&pid)
            .and_then(|process| match &process.lock().state {
                ProcessState::Exited(reason) => Some(reason.clone()),
                _ => None,
            });
    if let Some(reason) = exited_reason {
        finalize_managed_process(process_table, active_count, pid, reason);
        return Resumed::Finished;
    }

    set_current_pid(pid);
    let result = handle.resume_catching_panic();
    clear_current_pid();
    CURRENT_YIELDER.with(|current| current.yielder.set(None));

    match result {
        Ok(true) => {
            let mut outcome = Resumed::Runnable;
            let exited_reason = if let Some(process) = process_table.read().get(&pid) {
                let mut process = process.lock();
                process.reductions = DEFAULT_REDUCTIONS;
                match &process.state {
                    ProcessState::Exited(reason) => Some(reason.clone()),
                    ProcessState::Waiting => {
                        outcome = Resumed::Waiting;
                        None
                    }
                    ProcessState::Ready | ProcessState::Running => {
                        process.set_live_state(ProcessState::Ready);
                        None
                    }
                }
            } else {
                None
            };
            if let Some(reason) = exited_reason {
                finalize_managed_process(process_table, active_count, pid, reason);
                return Resumed::Finished;
            }
            outcome
        }
        result => {
            let reason = match result {
                Ok(false) => ExitReason::Normal,
                Err(message) => ExitReason::Error(message),
                Ok(true) => unreachable!(),
            };
            finalize_managed_process(process_table, active_count, pid, reason);
            Resumed::Finished
        }
    }
}

fn finalize_managed_process(
    process_table: &ProcessTable,
    active_count: &AtomicU64,
    pid: ProcessId,
    reason: ExitReason,
) {
    if !handle_process_exit(process_table, pid, reason) {
        return;
    }
    let previous = active_count
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            Some(count.saturating_sub(1))
        })
        .expect("active-count update cannot fail");
    let remaining = previous.saturating_sub(1);
    crate::dist::telemetry::runtime_telemetry().set_runnable_actors(remaining);
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

/// Handle process exit: invoke terminate callback, propagate exit to links,
/// clean up from process table.
///
/// 1. Claim finalization and preserve the first recorded exit reason
/// 2. Invoke terminate_callback once (panic-safe)
/// 3. Destroy remaining secrets, notify links/monitors, and remove the process
fn handle_process_exit(
    process_table: &ProcessTable,
    pid: ProcessId,
    fallback_reason: ExitReason,
) -> bool {
    // Claim finalization and extract callback/notification state under one lock.
    let (reason, terminate_cb, linked_pids, monitored_by_entries) = {
        if let Some(proc_arc) = process_table.read().get(&pid) {
            let mut proc = proc_arc.lock();
            let Some(reason) = proc.begin_exit_finalization(fallback_reason) else {
                return false;
            };
            let cb = proc.terminate_callback.take();
            let links = std::mem::take(&mut proc.links);
            let monitored_by = std::mem::take(&mut proc.monitored_by);
            (reason, cb, links, monitored_by)
        } else {
            return false;
        }
    };

    // Step 1: Invoke terminate callback (panic-safe).
    if let Some(cb) = terminate_cb {
        invoke_terminate_callback(cb, &reason);
    }

    // Natural exits keep resources live through terminate. Forced exits may
    // already have destroyed them; this second cleanup is intentionally idempotent.
    crate::secret::destroy_owned(pid);

    // Step 3: Partition links into local and remote, then propagate.
    let (local_links, remote_links): (
        std::collections::HashSet<ProcessId>,
        std::collections::HashSet<ProcessId>,
    ) = linked_pids
        .into_iter()
        .partition(|linked_pid| linked_pid.node_id() == 0);

    // Propagate exit signals to local linked processes.
    let woken = link::propagate_exit(pid, &reason, local_links, |linked_pid| {
        process_table.read().get(&linked_pid).cloned()
    });

    // Wake processes that were in Waiting state.
    let _ = woken;

    // Send DIST_EXIT for each remote link. Silently drops if node disconnected
    // (handle_node_disconnect already synthesized :noconnection locally).
    for remote_pid in &remote_links {
        crate::dist::node::send_dist_exit(pid, *remote_pid, &reason);
    }

    // Step 2.5: Deliver DOWN messages to monitoring processes.
    // Partition into local and remote monitors.
    for (monitor_ref, monitoring_pid) in &monitored_by_entries {
        if monitoring_pid.node_id() == 0 {
            // Local monitor: deliver DOWN message directly.
            if let Some(mon_proc_arc) = process_table.read().get(monitoring_pid) {
                let mut mon_proc = mon_proc_arc.lock();
                mon_proc.monitors.remove(monitor_ref);
                let down_data = link::encode_down_signal(*monitor_ref, pid, &reason);
                let buffer = super::heap::MessageBuffer::new(down_data, link::DOWN_SIGNAL_TAG);
                mon_proc.mailbox.push(super::process::Message { buffer });
                if matches!(mon_proc.state, ProcessState::Waiting) {
                    mon_proc.set_live_state(ProcessState::Ready);
                }
            }
        } else {
            // Remote monitor: send DIST_MONITOR_EXIT wire message.
            crate::dist::node::send_dist_monitor_exit_by_pid(
                pid,
                *monitoring_pid,
                *monitor_ref,
                &reason,
            );
        }
    }

    // Step 3: Clean up named registrations (local).
    registry::global_registry().cleanup_process(pid);

    // Step 3.5: Clean up global registrations for the exiting process (Phase 68).
    let removed_global_names = crate::dist::global::global_name_registry().cleanup_process(pid);
    if !removed_global_names.is_empty() {
        for name in &removed_global_names {
            crate::dist::global::broadcast_global_unregister(name);
        }
    }

    // Step 4: Release the completed actor and its private heap.
    process_table.write().remove(&pid);
    true
}

/// Invoke a terminate callback, catching any panics to prevent them from
/// crashing the runtime.
fn invoke_terminate_callback(cb: TerminateCallback, reason: &ExitReason) {
    // Encode the reason as a simple tag byte for the callback.
    let reason_tag: u8 = match reason {
        ExitReason::Normal => 0,
        ExitReason::Error(_) => 1,
        ExitReason::Killed => 2,
        ExitReason::Linked(_, _) => 3,
        ExitReason::Shutdown => 4,
        ExitReason::Custom(_) => 5,
        ExitReason::Noconnection => 6,
    };

    // catch_unwind ensures a panicking terminate callback does not unwind
    // through the scheduler.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cb(std::ptr::null(), &reason_tag as *const u8);
    }));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    static SPAWN_COUNTER: AtomicU64 = AtomicU64::new(0);
    static TERMINATE_CALLBACK_PID: AtomicU64 = AtomicU64::new(0);
    static TERMINATE_CALLBACK_SECRET_COUNT: AtomicUsize = AtomicUsize::new(usize::MAX);
    static FORCED_TERMINATE_COUNT: AtomicU64 = AtomicU64::new(0);
    static FORCED_TERMINATE_REASON: AtomicU64 = AtomicU64::new(u64::MAX);
    static IDEMPOTENT_TERMINATE_COUNT: AtomicU64 = AtomicU64::new(0);

    extern "C" fn increment_entry(_args: *const u8) {
        SPAWN_COUNTER.fetch_add(1, Ordering::SeqCst);
    }

    extern "C-unwind" fn panic_entry(_args: *const u8) {
        panic!("lifecycle test panic");
    }

    extern "C" fn observe_terminate_resources(_state: *const u8, _reason: *const u8) {
        let pid = ProcessId(TERMINATE_CALLBACK_PID.load(Ordering::SeqCst));
        TERMINATE_CALLBACK_SECRET_COUNT.store(
            crate::secret::owned_secret_count_for_test(pid),
            Ordering::SeqCst,
        );
    }

    extern "C" fn observe_forced_terminate(_state: *const u8, reason: *const u8) {
        FORCED_TERMINATE_COUNT.fetch_add(1, Ordering::SeqCst);
        FORCED_TERMINATE_REASON.store(unsafe { *reason } as u64, Ordering::SeqCst);
    }

    extern "C" fn observe_idempotent_terminate(_state: *const u8, _reason: *const u8) {
        IDEMPOTENT_TERMINATE_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn exited_suspended_actor_is_finalized_without_reentry() {
        static ENTRY_COUNT: AtomicU64 = AtomicU64::new(0);

        extern "C-unwind" fn yield_once(_args: *const u8) {
            ENTRY_COUNT.fetch_add(1, Ordering::SeqCst);
            super::super::stack::yield_current();
            ENTRY_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        ENTRY_COUNT.store(0, Ordering::SeqCst);
        let pid = ProcessId::next();
        let process_table: ProcessTable = Arc::new(RwLock::new(FxHashMap::default()));
        process_table.write().insert(
            pid,
            Arc::new(Mutex::new(Process::new(pid, Priority::Normal))),
        );
        let active_count = AtomicU64::new(1);
        let mut handle = CoroutineHandle::new(yield_once as *const u8, std::ptr::null());

        assert!(matches!(
            resume_process(&process_table, &active_count, pid, &mut handle),
            Resumed::Runnable
        ));
        assert_eq!(ENTRY_COUNT.load(Ordering::SeqCst), 1);
        process_table
            .read()
            .get(&pid)
            .unwrap()
            .lock()
            .mark_exited(ExitReason::Killed);

        assert!(matches!(
            resume_process(&process_table, &active_count, pid, &mut handle),
            Resumed::Finished
        ));
        assert_eq!(ENTRY_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(active_count.load(Ordering::SeqCst), 0);
        assert!(!process_table.read().contains_key(&pid));
    }

    #[test]
    fn exit_set_during_timeslice_is_observed_at_yield() {
        static ENTRY_COUNT: AtomicU64 = AtomicU64::new(0);
        static MAY_YIELD: AtomicBool = AtomicBool::new(false);

        extern "C-unwind" fn yield_after_exit(_args: *const u8) {
            ENTRY_COUNT.fetch_add(1, Ordering::SeqCst);
            while !MAY_YIELD.load(Ordering::SeqCst) {
                std::hint::spin_loop();
            }
            super::super::stack::yield_current();
            ENTRY_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        ENTRY_COUNT.store(0, Ordering::SeqCst);
        MAY_YIELD.store(false, Ordering::SeqCst);
        let pid = ProcessId::next();
        let process_table: ProcessTable = Arc::new(RwLock::new(FxHashMap::default()));
        process_table.write().insert(
            pid,
            Arc::new(Mutex::new(Process::new(pid, Priority::Normal))),
        );
        let active_count = AtomicU64::new(1);
        let mut handle = CoroutineHandle::new(yield_after_exit as *const u8, std::ptr::null());

        let retained = std::thread::scope(|scope| {
            scope.spawn(|| {
                while ENTRY_COUNT.load(Ordering::SeqCst) == 0 {
                    std::thread::yield_now();
                }
                process_table
                    .read()
                    .get(&pid)
                    .unwrap()
                    .lock()
                    .mark_exited(ExitReason::Killed);
                MAY_YIELD.store(true, Ordering::SeqCst);
            });
            resume_process(&process_table, &active_count, pid, &mut handle)
        });

        assert!(matches!(retained, Resumed::Finished));
        assert_eq!(ENTRY_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(active_count.load(Ordering::SeqCst), 0);
        assert!(!process_table.read().contains_key(&pid));
    }

    #[test]
    fn queued_exited_actor_is_finalized_once_without_starting() {
        static ENTRY_COUNT: AtomicU64 = AtomicU64::new(0);

        extern "C" fn should_not_run(_args: *const u8) {
            ENTRY_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        ENTRY_COUNT.store(0, Ordering::SeqCst);
        FORCED_TERMINATE_COUNT.store(0, Ordering::SeqCst);
        FORCED_TERMINATE_REASON.store(u64::MAX, Ordering::SeqCst);
        let scheduler = Scheduler::new(1);
        let target = scheduler.spawn(should_not_run as *const u8, std::ptr::null(), 0, 1);
        let observer = scheduler.create_main_process();
        let monitor_ref = 77;

        {
            let target_process = scheduler.get_process(target).unwrap();
            let mut target_process = target_process.lock();
            target_process.terminate_callback = Some(observe_forced_terminate);
            target_process.links.insert(observer);
            target_process.monitored_by.insert(monitor_ref, observer);
        }
        {
            let observer_process = scheduler.get_process(observer).unwrap();
            let mut observer_process = observer_process.lock();
            observer_process.trap_exit = true;
            observer_process.links.insert(target);
            observer_process.monitors.insert(monitor_ref, target);
        }
        crate::secret::insert_test_secret(target);
        scheduler
            .get_process(target)
            .unwrap()
            .lock()
            .mark_exited(ExitReason::Killed);

        scheduler.signal_shutdown();
        scheduler.run();

        assert_eq!(ENTRY_COUNT.load(Ordering::SeqCst), 0);
        assert_eq!(FORCED_TERMINATE_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(FORCED_TERMINATE_REASON.load(Ordering::SeqCst), 2);
        assert_eq!(scheduler.active_count(), 0);
        assert!(scheduler.get_process(target).is_none());
        assert_eq!(crate::secret::owned_secret_count_for_test(target), 0);

        let observer_process = scheduler.get_process(observer).unwrap();
        let observer_process = observer_process.lock();
        assert_eq!(observer_process.mailbox.len(), 2);
        let first_tag = observer_process.mailbox.pop().unwrap().buffer.type_tag;
        let second_tag = observer_process.mailbox.pop().unwrap().buffer.type_tag;
        assert_eq!(
            [first_tag, second_tag],
            [link::EXIT_SIGNAL_TAG, link::DOWN_SIGNAL_TAG]
        );
    }

    #[test]
    fn managed_process_finalization_is_idempotent() {
        IDEMPOTENT_TERMINATE_COUNT.store(0, Ordering::SeqCst);
        let pid = ProcessId::next();
        let process_table: ProcessTable = Arc::new(RwLock::new(FxHashMap::default()));
        let mut process = Process::new(pid, Priority::Normal);
        process.terminate_callback = Some(observe_idempotent_terminate);
        process.mark_exited(ExitReason::Killed);
        process_table
            .write()
            .insert(pid, Arc::new(Mutex::new(process)));
        let active_count = AtomicU64::new(1);

        finalize_managed_process(&process_table, &active_count, pid, ExitReason::Normal);
        finalize_managed_process(&process_table, &active_count, pid, ExitReason::Normal);

        assert_eq!(IDEMPOTENT_TERMINATE_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(active_count.load(Ordering::SeqCst), 0);
    }

    /// Stable thread identifier using Hash of ThreadId.
    fn thread_id_hash() -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn spawn_copies_gc_args_and_lends_what_they_point_into() {
        let sched = Scheduler::new(1);
        let spawner_pid = sched.create_main_process();
        let spawner = sched.get_process(spawner_pid).unwrap();
        set_current_pid(spawner_pid);

        // A heap value, and a compiled-style argument buffer naming it.
        let (value, args) = {
            let mut process = spawner.lock();
            let value = process.heap.alloc(24, 8);
            let args = process.heap.alloc(16, 8) as *mut usize;
            unsafe {
                args.write(value as usize);
                args.add(1).write(7);
            }
            (value, args as *const u8)
        };
        let child_pid = sched.spawn(increment_entry as *const u8, args, 16, 1);

        {
            let child = sched.get_process(child_pid).unwrap();
            let child = child.lock();
            let copy = child.spawn_args.as_ref().expect("buffer copied for child");
            assert_eq!(&copy[..], &[value as u64, 7]);
            assert_ne!(copy.as_ptr() as *const u8, args);
            assert_eq!(child.heap_borrows.len(), 1);
            assert!(Arc::ptr_eq(
                child.heap_borrows[0].owner.as_ref().unwrap(),
                &spawner
            ));
        }

        // The spawner forgets both and collects: the value is on loan, while
        // the buffer was copied and is ordinary garbage.
        let dummy: u64 = 0;
        let stack = &dummy as *const u64 as *const u8;
        spawner.lock().heap.collect(stack, stack);
        assert!(spawner.lock().heap.is_live_allocation(value, 24));
        assert!(!spawner.lock().heap.is_live_allocation(args, 16));

        // Once the child's process is gone the loan ends.
        sched.process_table.write().remove(&child_pid);
        spawner.lock().heap.collect(stack, stack);
        assert!(!spawner.lock().heap.is_live_allocation(value, 24));

        clear_current_pid();
    }

    #[test]
    fn shaped_spawn_gives_the_child_its_own_copy() {
        use crate::actor::msg_shape::{AGG, LEAF, SHARED};

        let sched = Scheduler::new(1);
        let spawner_pid = sched.create_main_process();
        let spawner = sched.get_process(spawner_pid).unwrap();
        set_current_pid(spawner_pid);

        // Arguments: a string, an integer, and something only a loan can cover.
        let (value, opaque, args) = {
            let mut process = spawner.lock();
            let value = process.heap.alloc(24, 8);
            unsafe { (value as *mut u64).write(16) };
            let opaque = process.heap.alloc(32, 8);
            let args = process.heap.alloc(24, 8) as *mut usize;
            unsafe {
                args.write(value as usize);
                args.add(1).write(7);
                args.add(2).write(opaque as usize);
            }
            (value, opaque, args as *const u8)
        };
        let shape = [9, AGG, 2, 0, 7, 16, 8, LEAF, SHARED];
        let child_pid =
            sched.spawn_shaped(increment_entry as *const u8, args, 24, 1, shape.as_ptr());
        clear_current_pid();

        let child = sched.get_process(child_pid).unwrap();
        let child = child.lock();
        let copy = child.spawn_args.as_ref().expect("buffer copied for child");
        assert_ne!(copy[0], value as u64, "the string was copied");
        assert!(child.heap.is_live_allocation(copy[0] as *const u8, 24));
        assert_eq!(unsafe { *(copy[0] as *const u64) }, 16);
        assert_eq!(copy[1], 7);
        assert_eq!(copy[2], opaque as u64, "the opaque value stays shared");
        assert_eq!(child.heap_borrows.len(), 1);

        // The spawner may drop and collect the string; the loan keeps the rest.
        let dummy: u64 = 0;
        let stack = &dummy as *const u64 as *const u8;
        spawner.lock().heap.collect(stack, stack);
        assert!(!spawner.lock().heap.is_live_allocation(value, 24));
        assert!(spawner.lock().heap.is_live_allocation(opaque, 32));
    }

    #[test]
    fn spawn_borrows_nothing_for_integers_and_leaves_foreign_buffers_alone() {
        let sched = Scheduler::new(1);
        let spawner_pid = sched.create_main_process();
        let spawner = sched.get_process(spawner_pid).unwrap();
        set_current_pid(spawner_pid);

        let ints = {
            let args = spawner.lock().heap.alloc(16, 8) as *mut usize;
            unsafe {
                args.write(30);
                args.add(1).write(12);
            }
            args as *const u8
        };
        let child_pid = sched.spawn(increment_entry as *const u8, ints, 16, 1);
        {
            let child = sched.get_process(child_pid).unwrap();
            let child = child.lock();
            assert_eq!(&child.spawn_args.as_ref().unwrap()[..], &[30, 12]);
            assert!(child.heap_borrows.is_empty(), "integers pin no heap");
        }

        // Runtime callers hand over a `Box` the entry function takes ownership
        // of; copying it would make that `Box::from_raw` free the wrong block.
        let boxed = Box::into_raw(Box::new([1u64, 2])) as *const u8;
        let child_pid = sched.spawn(increment_entry as *const u8, boxed, 16, 1);
        {
            let child = sched.get_process(child_pid).unwrap();
            let child = child.lock();
            assert!(child.spawn_args.is_none());
            assert!(child.heap_borrows.is_empty());
        }
        drop(unsafe { Box::from_raw(boxed as *mut [u64; 2]) });

        clear_current_pid();
    }

    #[test]
    fn spawn_lends_a_value_passed_on_from_the_spawners_own_arguments() {
        let sched = Scheduler::new(1);
        let grandparent_pid = sched.create_main_process();
        let grandparent = sched.get_process(grandparent_pid).unwrap();

        // Grandparent -> parent: a heap value.
        set_current_pid(grandparent_pid);
        let (value, args) = {
            let mut process = grandparent.lock();
            let value = process.heap.alloc(24, 8);
            let args = process.heap.alloc(8, 8) as *mut usize;
            unsafe { args.write(value as usize) };
            (value, args as *const u8)
        };
        let parent_pid = sched.spawn(increment_entry as *const u8, args, 8, 1);
        let parent = sched.get_process(parent_pid).unwrap();

        // Parent -> child: the same value, from a buffer on the parent's heap.
        set_current_pid(parent_pid);
        let args = {
            let args = parent.lock().heap.alloc(8, 8) as *mut usize;
            unsafe { args.write(value as usize) };
            args as *const u8
        };
        let child_pid = sched.spawn(increment_entry as *const u8, args, 8, 1);
        clear_current_pid();

        {
            let child = sched.get_process(child_pid).unwrap();
            let child = child.lock();
            assert_eq!(child.heap_borrows.len(), 1);
            assert!(Arc::ptr_eq(
                child.heap_borrows[0].owner.as_ref().unwrap(),
                &grandparent
            ));
        }

        // The parent goes away first; the child still holds the value.
        drop(parent);
        sched.process_table.write().remove(&parent_pid);
        let dummy: u64 = 0;
        let stack = &dummy as *const u64 as *const u8;
        grandparent.lock().heap.collect(stack, stack);
        assert!(grandparent.lock().heap.is_live_allocation(value, 24));
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
        let sched = Scheduler::new(1);
        sched.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1);
        sched.signal_shutdown();
        sched.run();

        let delta = SPAWN_COUNTER.load(Ordering::SeqCst) - initial;
        assert!(
            delta >= 1,
            "Expected at least 1 actor to complete, got delta={}",
            delta
        );
    }

    #[test]
    fn test_multiple_actors_complete() {
        let initial = SPAWN_COUNTER.load(Ordering::SeqCst);
        let num_actors = 10;
        let sched = Scheduler::new(2);
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
    fn completed_actors_leave_the_process_table() {
        let sched = Scheduler::new(2);
        for _ in 0..100 {
            sched.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1);
        }
        sched.signal_shutdown();
        sched.run();

        assert!(
            sched.process_table.read().is_empty(),
            "completed actor heaps must not remain reachable"
        );
    }

    #[test]
    fn normal_and_panicking_actors_destroy_owned_secrets() {
        let sched = Scheduler::new(1);
        let normal_pid = sched.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1);
        let panic_pid = sched.spawn(panic_entry as *const u8, std::ptr::null(), 0, 1);
        TERMINATE_CALLBACK_PID.store(normal_pid.as_u64(), Ordering::SeqCst);
        TERMINATE_CALLBACK_SECRET_COUNT.store(usize::MAX, Ordering::SeqCst);
        sched
            .get_process(normal_pid)
            .unwrap()
            .lock()
            .terminate_callback = Some(observe_terminate_resources);
        crate::secret::insert_test_secret(normal_pid);
        crate::secret::insert_test_secret(panic_pid);

        sched.signal_shutdown();
        sched.run();

        let normal_remaining = crate::secret::owned_secret_count_for_test(normal_pid);
        let panic_remaining = crate::secret::owned_secret_count_for_test(panic_pid);
        crate::secret::destroy_owned(normal_pid);
        crate::secret::destroy_owned(panic_pid);
        assert_eq!(
            TERMINATE_CALLBACK_SECRET_COUNT.load(Ordering::SeqCst),
            1,
            "terminate callback must run before resource cleanup"
        );
        assert_eq!(normal_remaining, 0);
        assert_eq!(panic_remaining, 0);
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
        let sched = Scheduler::new(4);
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
        let sched = Scheduler::new(2);
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
        let sched = Scheduler::new(2);

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
        let sched = Scheduler::new(1);
        // Spawn low-priority actors
        for _ in 0..5 {
            sched.spawn(prio_entry as *const u8, std::ptr::null(), 0, 2); // Low
        }
        // Spawn high-priority actor
        sched.spawn(prio_entry as *const u8, std::ptr::null(), 0, 0); // High
        sched.signal_shutdown();
        sched.run();

        assert_eq!(
            PRIO_COUNTER.load(Ordering::SeqCst),
            6,
            "All priority levels should complete"
        );
    }

    #[test]
    fn test_100_actors_no_hang() {
        let initial = SPAWN_COUNTER.load(Ordering::SeqCst);
        let num_actors: u64 = 100;
        let sched = Scheduler::new(4);
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

    #[test]
    fn elastic_scheduler_enforces_worker_bounds() {
        let scheduler = Scheduler::new_elastic(2, 4).expect("valid bounds");

        assert_eq!(
            scheduler.resize(1),
            Err("scheduler_worker_target_out_of_bounds:1:2..=4".to_string())
        );
        assert_eq!(
            scheduler.resize(5),
            Err("scheduler_worker_target_out_of_bounds:5:2..=4".to_string())
        );
    }

    #[test]
    fn runnable_count_excludes_live_waiting_actors() {
        let scheduler = Scheduler::new(1);
        let pid = scheduler.spawn(increment_entry as *const u8, std::ptr::null(), 0, 1);
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.runnable_count(), 1);

        scheduler.get_process(pid).unwrap().lock().state = ProcessState::Waiting;
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.runnable_count(), 0);
    }

    #[test]
    fn elastic_scheduler_activates_parked_workers_without_restart() {
        static ELASTIC_COUNTER: AtomicU64 = AtomicU64::new(0);

        extern "C" fn elastic_entry(_args: *const u8) {
            ELASTIC_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        ELASTIC_COUNTER.store(0, Ordering::SeqCst);
        let scheduler = Scheduler::new_elastic(1, 3).expect("valid bounds");
        scheduler.start();
        scheduler.resize(3).expect("activate parked workers");
        for _ in 0..30 {
            scheduler.spawn(elastic_entry as *const u8, std::ptr::null(), 0, 1);
        }
        scheduler.signal_shutdown();
        scheduler.wait();

        assert_eq!(ELASTIC_COUNTER.load(Ordering::SeqCst), 30);
    }
}
