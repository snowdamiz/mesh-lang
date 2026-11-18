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
//! 1. **Spawn requests** (function pointer + args) are `Send`. Ordinary
//!    requests use a shared FIFO. Grouped requests use a worker's own FIFO.
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
//! - Normal- and low-priority requests share the global FIFO.

use crossbeam_channel::{Receiver, Sender};
use crossbeam_deque::{Injector, Steal};
use crossbeam_utils::CachePadded;
use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use rustc_hash::FxHashMap;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use super::link;
use super::process::{
    ExitReason, HeapBorrow, Placement, Priority, Process, ProcessId, ProcessState,
    TerminateCallback, DEFAULT_REDUCTIONS,
};
use super::registry;
use super::stack::{
    clear_current_pid, get_current_pid, set_current_process, CoroutineHandle, CURRENT_YIELDER,
};

// ---------------------------------------------------------------------------
// SpawnRequest
// ---------------------------------------------------------------------------

/// A request to spawn a new actor. This is `Send` and can be distributed
/// across worker threads via work-stealing.
#[allow(dead_code)]
struct SpawnRequest {
    step: Option<Box<BoundedStep>>,
    pid: ProcessId,
    fn_ptr: *const u8,
    args_ptr: *const u8,
    priority: Priority,
}

// Safety: The fn_ptr and args_ptr are owned by the runtime and the actor
// entry function is safe to call from any thread. The runtime guarantees
// these pointers remain valid until the actor completes.
unsafe impl Send for SpawnRequest {}

enum SpawnMode {
    Shared,
    Linked {
        parent: ProcessId,
        parent_traps: bool,
    },
    Grouped(Option<TerminateCallback>),
    Step(u32),
}

// Only a grouped actor resuming on its owner can hand off to another
// suspended grouped actor there. Overflow takes the normal notification path.
const MAX_GROUP_HANDOFFS: usize = 3;

#[derive(Clone, Copy)]
enum HandoffSlot {
    Disabled,
    Open(*const WorkerSleep),
    Woken(ProcessId),
}

impl HandoffSlot {
    fn offer(worker: &WorkerSleep, pid: ProcessId) -> bool {
        match GROUP_HANDOFF.get() {
            Self::Open(owner) if std::ptr::eq(owner, worker) => {
                GROUP_HANDOFF.set(Self::Woken(pid));
                true
            }
            _ => false,
        }
    }
}

thread_local! {
    static GROUP_HANDOFF: Cell<HandoffSlot> = const { Cell::new(HandoffSlot::Disabled) };
    // A terminate callback runs outside the actor coroutine. Carry only its
    // placement so actor APIs keep their existing callback context.
    static TERMINATING_GROUP_WORKER: Cell<Option<usize>> = const { Cell::new(None) };
    static CURRENT_WORKER: Cell<*const WorkerSleep> = const { Cell::new(std::ptr::null()) };
}

// Restore the ordinary wake path even if the scheduler unwinds mid-resume.
struct CaptureHandoff<'a>(&'a WorkerSleep);

impl Drop for CaptureHandoff<'_> {
    fn drop(&mut self) {
        if let HandoffSlot::Woken(pid) = GROUP_HANDOFF.replace(HandoffSlot::Disabled) {
            self.0.woken.lock().push(pid);
        }
    }
}

fn capture_handoff<T>(worker: &WorkerSleep, resume: impl FnOnce() -> T) -> (T, Option<ProcessId>) {
    let _reset = CaptureHandoff(worker);
    GROUP_HANDOFF.set(HandoffSlot::Open(worker));
    let result = resume();
    let next = match GROUP_HANDOFF.replace(HandoffSlot::Disabled) {
        HandoffSlot::Woken(pid) => Some(pid),
        _ => None,
    };
    (result, next)
}

pub(super) fn terminating_group_worker() -> Option<usize> {
    TERMINATING_GROUP_WORKER.get()
}

/// Only the owning worker drains this FIFO. The flag avoids an empty lock on
/// the usual shared path. Both flag transitions are serialized with the queue.
#[derive(Default)]
struct GroupedSpawns {
    pending: AtomicBool,
    queue: Mutex<VecDeque<SpawnRequest>>,
}

impl GroupedSpawns {
    fn push(&self, request: SpawnRequest) {
        let mut queue = self.queue.lock();
        queue.push_back(request);
        self.pending.store(true, Ordering::Release);
    }

    fn has_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    fn len(&self) -> usize {
        if self.has_pending() {
            self.queue.lock().len()
        } else {
            0
        }
    }

    fn pop(&self) -> Option<SpawnRequest> {
        if !self.has_pending() {
            return None;
        }
        let mut queue = self.queue.lock();
        let request = queue.pop_front();
        if queue.is_empty() {
            self.pending.store(false, Ordering::Release);
        }
        request
    }
}

// ---------------------------------------------------------------------------
// ProcessTable
// ---------------------------------------------------------------------------

const PID_SHARDS: usize = 16;
type ProcessMap = FxHashMap<ProcessId, Arc<Mutex<Process>>>;

#[derive(Default)]
struct ProcessShard {
    entries: RwLock<ProcessMap>,
    /// Changed under the write lock whenever a previously valid hit can go stale.
    revision: AtomicU64,
}

impl ProcessShard {
    fn invalidate_peers(&self) {
        let next = self.revision.load(Ordering::Relaxed).saturating_add(1);
        self.revision.store(next, Ordering::Release);
    }
}

struct Peer {
    pid: ProcessId,
    revision: u64,
    handle: Weak<Mutex<Process>>,
}

const PEER_SLOTS: usize = 64;
struct PeerCache {
    table: Weak<ProcessTable>,
    slots: [Option<Peer>; PEER_SLOTS],
}

thread_local! {
    static PEERS: RefCell<PeerCache> = const { RefCell::new(PeerCache {
        table: Weak::new(),
        slots: [const { None }; PEER_SLOTS],
    }) };
}

/// Shared process table for PID lookups across all worker threads.
pub struct ProcessTable {
    shards: [CachePadded<ProcessShard>; PID_SHARDS],
}

impl ProcessTable {
    fn new() -> Self {
        Self {
            shards: std::array::from_fn(|_| CachePadded::new(ProcessShard::default())),
        }
    }

    fn shard(&self, pid: ProcessId) -> &ProcessShard {
        // Local PIDs are sequential, so their low bits spread across shards.
        &self.shards[pid.local_id() as usize % PID_SHARDS]
    }

    pub fn contains_key(&self, pid: ProcessId) -> bool {
        self.shard(pid).entries.read().contains_key(&pid)
    }

    pub fn get(&self, pid: ProcessId) -> Option<Arc<Mutex<Process>>> {
        self.with_process(pid, Arc::clone)
    }

    /// Keep this PID in the table during a process-local operation. The
    /// callback may lock the process but must not reenter the table.
    fn with_process<T>(
        &self,
        pid: ProcessId,
        f: impl FnOnce(&Arc<Mutex<Process>>) -> T,
    ) -> Option<T> {
        self.shard(pid).entries.read().get(&pid).map(f)
    }

    pub fn insert(&self, pid: ProcessId, process: Arc<Mutex<Process>>) {
        let shard = self.shard(pid);
        let mut entries = shard.entries.write();
        if entries.insert(pid, process).is_some() {
            shard.invalidate_peers();
        }
    }

    pub fn remove(&self, pid: ProcessId) {
        let shard = self.shard(pid);
        let mut entries = shard.entries.write();
        if entries.remove(&pid).is_some() {
            shard.invalidate_peers();
        }
    }

    /// Freeze membership before visiting any process. Take shards in index
    /// order and keep them all locked for the full scan. While holding this
    /// guard, access entries through the guard itself.
    pub fn read_all(&self) -> ProcessTableRead<'_> {
        ProcessTableRead(std::array::from_fn(|index| {
            self.shards[index].entries.read()
        }))
    }

    /// Check whether actors can still send during shutdown. A receive excludes
    /// itself, while an idle worker checks every process.
    pub(super) fn has_shutdown_work(&self, except: Option<ProcessId>) -> bool {
        let table = self.read_all();
        let processes = || table.iter().filter(|(pid, _)| Some(**pid) != except);
        if processes().any(|(_, process)| {
            process
                .try_lock()
                .is_none_or(|process| process.has_shutdown_work())
        }) {
            return true;
        }

        // Work can move to an actor we already checked. Confirm quiescence
        // with all process states locked at once. Never wait for another
        // lock while holding these guards.
        let mut quiet = Vec::new();
        for (_, process) in processes() {
            let Some(process) = process.try_lock() else {
                return true;
            };
            if process.has_shutdown_work() {
                return true;
            }
            quiet.push(process);
        }
        false
    }
}

pub struct ProcessTableRead<'a>([RwLockReadGuard<'a, ProcessMap>; PID_SHARDS]);

impl ProcessTableRead<'_> {
    pub fn iter(&self) -> impl Iterator<Item = (&ProcessId, &Arc<Mutex<Process>>)> {
        self.0.iter().flat_map(|shard| shard.iter())
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// One per worker thread, so that whoever makes work can wake the worker that
/// should run it rather than wait out its poll interval.
#[derive(Default)]
struct WorkerSleep {
    thread: std::sync::OnceLock<std::thread::Thread>,
    /// Set from the worker's first park until it finds work again.
    idle: AtomicBool,
    /// Approximate number of actors already pinned to this worker.
    residents: AtomicUsize,
    /// Advances whenever this worker takes an ordinary shared spawn.
    shared_claims: AtomicUsize,
    /// Processes on this worker that need attention after waiting.
    woken: Mutex<Vec<ProcessId>>,
    grouped: GroupedSpawns,
}

fn notify_worker(sleepers: &[CachePadded<WorkerSleep>], worker: Option<usize>, pid: ProcessId) {
    if let Some(worker) = worker.and_then(|index| sleepers.get(index)) {
        worker.notify(pid);
    }
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

    fn notify(&self, pid: ProcessId) {
        self.woken.lock().push(pid);
        if !std::ptr::eq(CURRENT_WORKER.get(), self) {
            self.wake();
        }
    }

    fn enqueue_grouped(&self, request: SpawnRequest) {
        self.grouped.push(request);
        // Retired workers can park outside the ordinary idle stretch.
        if let Some(thread) = self.thread.get() {
            thread.unpark();
        }
    }
}

/// A worker's stretch without work. It counts as idle for the whole stretch,
/// scans included. After publishing idle, scan once more before parking.
/// An earlier push is visible to the scan. A later push can unpark the worker.
struct IdleStretch<'a> {
    worker: &'a WorkerSleep,
    published: bool,
}

impl IdleStretch<'_> {
    fn park(&mut self, timeout: std::time::Duration) {
        if !self.published {
            self.worker.idle.store(true, Ordering::SeqCst);
            self.published = true;
            return;
        }
        std::thread::park_timeout(timeout);
    }

    fn end(&mut self) {
        if self.published {
            self.worker.idle.store(false, Ordering::SeqCst);
            self.published = false;
        }
    }
}

impl Drop for IdleStretch<'_> {
    fn drop(&mut self) {
        self.end();
    }
}

/// The M:N scheduler. New shared actors stay in a single FIFO until claimed.
pub struct Scheduler {
    /// Maximum number of initialized OS worker threads.
    num_threads: usize,

    /// Minimum number of workers that must remain active.
    min_threads: usize,

    /// Current target number of active workers.
    active_threads: Arc<AtomicUsize>,

    next_group_worker: AtomicUsize,

    /// Global injector queue for spawn requests (normal + low priority).
    injector: Arc<Injector<SpawnRequest>>,

    /// High-priority channel -- checked first by all workers.
    high_priority_tx: Sender<SpawnRequest>,
    high_priority_rx: Receiver<SpawnRequest>,

    started: AtomicBool,

    /// Shared process table for PID lookup.
    process_table: Arc<ProcessTable>,

    /// Shutdown flag -- set when the main actor exits.
    shutdown: Arc<AtomicBool>,

    /// Count of active (non-exited) processes.
    active_count: Arc<AtomicU64>,

    /// Handles for background worker threads (populated by `start()`).
    worker_handles: Mutex<Vec<std::thread::JoinHandle<()>>>,

    /// Where each worker stands, indexed like the workers themselves.
    sleepers: Arc<Vec<CachePadded<WorkerSleep>>>,
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

        Scheduler {
            num_threads: max_threads,
            min_threads,
            active_threads: Arc::new(AtomicUsize::new(min_threads)),
            next_group_worker: AtomicUsize::new(0),
            injector,
            high_priority_tx: high_tx,
            high_priority_rx: high_rx,
            started: AtomicBool::new(false),
            process_table: Arc::new(ProcessTable::new()),
            shutdown: Arc::new(AtomicBool::new(false)),
            active_count: Arc::new(AtomicU64::new(0)),
            worker_handles: Mutex::new(Vec::new()),
            sleepers: Arc::new((0..max_threads).map(|_| CachePadded::default()).collect()),
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
        self.spawn_process(
            fn_ptr,
            args_ptr,
            args_size,
            Priority::from_u8(priority),
            shape,
            SpawnMode::Shared,
        )
    }

    pub(super) fn spawn_step(
        &self,
        entry: *const u8,
        bound: u32,
        args: *const u8,
        size: u64,
        shape: *const u32,
    ) -> ProcessId {
        self.spawn_process(
            entry,
            args,
            size,
            Priority::Normal,
            shape,
            SpawnMode::Step(bound),
        )
    }

    pub(super) fn spawn_grouped(
        &self,
        fn_ptr: *const u8,
        args_ptr: *const u8,
        args_size: u64,
        shape: *const u32,
        terminate: Option<TerminateCallback>,
    ) -> ProcessId {
        self.spawn_process(
            fn_ptr,
            args_ptr,
            args_size,
            Priority::Normal,
            shape,
            SpawnMode::Grouped(terminate),
        )
    }

    /// `spawn` of an actor linked to `parent` from the start: a link made once
    /// it is queued could come after it has ended, and its end go unnoticed.
    /// With `parent_traps`, the parent gets the actor's abnormal exit as a
    /// message instead of dying of it (see `Process::trapped_links`).
    pub fn spawn_linked(
        &self,
        fn_ptr: *const u8,
        args_ptr: *const u8,
        args_size: u64,
        priority: u8,
        parent: ProcessId,
        parent_traps: bool,
    ) -> ProcessId {
        self.spawn_process(
            fn_ptr,
            args_ptr,
            args_size,
            Priority::from_u8(priority),
            std::ptr::null(),
            SpawnMode::Linked {
                parent,
                parent_traps,
            },
        )
    }

    fn spawn_process(
        &self,
        fn_ptr: *const u8,
        args_ptr: *const u8,
        args_size: u64,
        priority: Priority,
        shape: *const u32,
        mode: SpawnMode,
    ) -> ProcessId {
        let pid = self.fresh_pid();

        let spawner = get_current_pid().and_then(|pid| self.get_process(pid));
        // ponytail: pin the entire call heap for actors borrowing its arguments;
        // use typed argument copying if long-lived actors make retention costly.
        let (caller_placement, library_heap_owner) =
            spawner.as_ref().map_or((Placement::Shared, None), |owner| {
                let parent = owner.lock();
                let heap_owner = if parent.library_call {
                    Some(Arc::clone(owner))
                } else {
                    parent.library_heap_owner.clone()
                };
                (parent.placement, heap_owner)
            });
        let placement = if matches!(mode, SpawnMode::Grouped(_) | SpawnMode::Step(_)) {
            let worker = match caller_placement {
                Placement::Grouped { worker } => worker,
                Placement::Shared => terminating_group_worker().unwrap_or_else(|| {
                    let ticket = self.next_group_worker.fetch_add(1, Ordering::Relaxed);
                    // A concurrent retirement can follow this selection. The
                    // selected worker drains its FIFO through retirement.
                    ticket % self.active_workers()
                }),
            };
            Placement::Grouped { worker }
        } else {
            Placement::Shared
        };

        let mut process = Process::with_placement(pid, priority, placement);
        process.library_heap_owner = library_heap_owner;
        if let SpawnMode::Grouped(callback) = mode {
            process.terminate_callback = callback;
        }
        if let SpawnMode::Linked {
            parent,
            parent_traps,
        } = mode
        {
            process.links.insert(parent);
            if let Some(parent) = self.get_process(parent) {
                let mut parent = parent.lock();
                parent.links.insert(pid);
                if parent_traps {
                    parent.trapped_links.insert(pid);
                }
            }
        }
        let args_ptr = match &spawner {
            Some(spawner) => adopt_spawn_args(&mut process, spawner, args_ptr, args_size, shape),
            None => args_ptr,
        };
        // Take the relocated argument words when adoption produced them. Foreign
        // argument storage is valid during spawn, so copy it before enqueue.
        let step = if let SpawnMode::Step(bound) = mode {
            let frame = process.spawn_args.take().unwrap_or_else(|| {
                let mut frame = vec![0u64; args_size as usize / 8].into_boxed_slice();
                if !frame.is_empty() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            args_ptr,
                            frame.as_mut_ptr().cast(),
                            args_size as usize,
                        )
                    };
                }
                frame
            });
            Some(Box::new(BoundedStep {
                deferred: false,
                call: unsafe { std::mem::transmute::<*const u8, StepFn>(fn_ptr) },
                bound,
                frame,
            }))
        } else {
            None
        };
        let process = Arc::new(Mutex::new(process));
        self.process_table.insert(pid, process);

        // Track active process count.
        let active_count = self.active_count.fetch_add(1, Ordering::SeqCst) + 1;
        crate::dist::telemetry::runtime_telemetry().set_runnable_actors(active_count);

        // Enqueue spawn request.
        let request = SpawnRequest {
            step,
            pid,
            fn_ptr,
            args_ptr,
            priority,
        };

        if let Placement::Grouped { worker } = placement {
            self.sleepers[worker].enqueue_grouped(request);
        } else {
            if priority == Priority::High {
                let _ = self.high_priority_tx.send(request);
            } else {
                self.injector.push(request);
            }
            // Any worker can take a shared request: wake one that has none.
            let active_workers = self.active_threads.load(Ordering::Acquire);
            let _ = self.sleepers[..active_workers]
                .iter()
                .any(|worker| worker.wake());
        }

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
        assert!(
            !self.started.swap(true, Ordering::Relaxed),
            "worker already consumed"
        );

        for i in 0..num_threads {
            let injector = Arc::clone(&self.injector);
            let high_rx = self.high_priority_rx.clone();
            let shutdown = Arc::clone(&self.shutdown);
            let active_count = Arc::clone(&self.active_count);
            let process_table = Arc::clone(&self.process_table);
            let active_threads = Arc::clone(&self.active_threads);
            let sleepers = Arc::clone(&self.sleepers);

            let handle = std::thread::spawn(move || {
                crate::stack_overflow::install_for_current_thread();
                let shared = WorkerLoopShared {
                    injector,
                    shutdown,
                    active_count,
                    process_table,
                    active_threads,
                    sleepers,
                };
                worker_loop(i, high_rx, shared);
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
        assert!(
            !self.started.swap(true, Ordering::Relaxed),
            "worker already consumed"
        );
        let num_threads = self.num_threads;

        crossbeam_utils::thread::scope(|scope| {
            for i in 0..num_threads {
                let injector = Arc::clone(&self.injector);
                let high_rx = self.high_priority_rx.clone();
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
                    worker_loop(i, high_rx, shared);
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
            .read_all()
            .iter()
            .filter(|(_, process)| {
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
        let worker_depths: Vec<_> = self
            .sleepers
            .iter()
            .map(|worker| worker.grouped.len())
            .collect();
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

    /// A PID no process in the table holds.
    fn fresh_pid(&self) -> ProcessId {
        ProcessId::next_unused(|pid| self.process_table.contains_key(pid))
    }

    /// Create a process entry for the main thread.
    ///
    /// This gives the main thread a PID and mailbox so that `mesh_service_call`
    /// can work from non-coroutine context. The main thread process is NOT
    /// counted in active_count because it is not managed by the scheduler --
    /// its lifetime is controlled by the C main function.
    pub fn create_main_process(&self) -> ProcessId {
        let pid = self.fresh_pid();
        let mut process = Process::new(pid, Priority::Normal);
        process.host_managed = true;
        process.set_live_state(ProcessState::Running);
        let process = Arc::new(Mutex::new(process));
        self.process_table.insert(pid, process);
        // Do NOT increment active_count -- main thread is not scheduler-managed.
        pid
    }

    pub(crate) fn finalize_host_process(&self, pid: ProcessId) {
        handle_process_exit(&self.process_table, &self.sleepers, pid, ExitReason::Normal);
    }

    /// Look up a process by PID.
    pub fn get_process(&self, pid: ProcessId) -> Option<Arc<Mutex<Process>>> {
        self.process_table.get(pid)
    }

    /// A weak handle alone cannot establish membership: heap loans can keep
    /// a removed process alive. Match both the table and the shard revision
    /// observed under its read lock. A hit linearizes at the revision load,
    /// just as an ordinary lookup can race with removal after the read.
    pub(super) fn send_target(&self, pid: ProcessId) -> Option<Arc<Mutex<Process>>> {
        let table = &self.process_table;
        PEERS
            .try_with(|cache| {
                let mut cache = cache.borrow_mut();
                if !std::ptr::eq(cache.table.as_ptr(), Arc::as_ptr(table)) {
                    cache.slots.fill_with(|| None);
                    cache.table = Arc::downgrade(table);
                }
                let shard = table.shard(pid);
                let slot = &mut cache.slots[pid.local_id() as usize % PEER_SLOTS];
                if let Some(peer) = slot {
                    if peer.pid == pid && peer.revision == shard.revision.load(Ordering::Acquire) {
                        if let Some(process) = peer.handle.upgrade() {
                            return Some(process);
                        }
                    }
                }

                let entries = shard.entries.read();
                let process = entries.get(&pid)?;
                let revision = shard.revision.load(Ordering::Relaxed);
                // Leave caching disabled on saturation, so a revision never repeats.
                if revision != u64::MAX {
                    *slot = Some(Peer {
                        pid,
                        revision,
                        handle: Arc::downgrade(process),
                    });
                }
                Some(Arc::clone(process))
            })
            .unwrap_or_else(|_| table.get(pid))
    }

    /// Get a reference to the process table (for full-table scans).
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
    /// Tell the worker that runs `pid` that a waiting process needs attention.
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
        notify_worker(&self.sleepers, worker, pid);
    }

    /// Force an exit without leaving a parked actor for the fallback sweep.
    pub(crate) fn force_exit(
        &self,
        pid: ProcessId,
        mut process: parking_lot::MutexGuard<'_, Process>,
        reason: ExitReason,
    ) {
        let waiting = matches!(process.state, ProcessState::Waiting);
        if process.mark_exited(reason) && waiting {
            let worker = process.worker;
            drop(process);
            self.wake_worker(worker, pid);
        }
    }

    /// Make `pid` Ready if it is Waiting and have its worker resume it.
    /// `process` is `pid`'s process, locked; the lock is released.
    pub fn wake_if_waiting(
        &self,
        pid: ProcessId,
        mut process: parking_lot::MutexGuard<'_, Process>,
    ) {
        if matches!(process.state, ProcessState::Waiting)
            && process.set_live_state(ProcessState::Ready)
        {
            let worker = process.worker;
            if matches!(process.placement, Placement::Grouped { .. })
                && worker
                    .and_then(|i| self.sleepers.get(i))
                    .is_some_and(|owner| HandoffSlot::offer(owner, pid))
            {
                return;
            }
            drop(process);
            self.wake_worker(worker, pid);
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

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

struct WorkerLoopShared {
    injector: Arc<Injector<SpawnRequest>>,
    shutdown: Arc<AtomicBool>,
    active_count: Arc<AtomicU64>,
    process_table: Arc<ProcessTable>,
    active_threads: Arc<AtomicUsize>,
    sleepers: Arc<Vec<CachePadded<WorkerSleep>>>,
}

type StepFn = extern "C-unwind" fn(*mut u64, *const u8) -> u8;

struct BoundedStep {
    deferred: bool,
    call: StepFn,
    bound: u32,
    frame: Box<[u64]>,
}

enum Execution {
    Stack(CoroutineHandle),
    Step(Box<BoundedStep>),
}

impl Execution {
    fn resume(
        &mut self,
        process: &Arc<Mutex<Process>>,
        table: &ProcessTable,
        shutdown: &AtomicBool,
    ) -> Result<bool, String> {
        match self {
            Self::Stack(handle) => handle.resume_catching_panic(),
            Self::Step(step) => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    step.run(process, table, shutdown)
                }));
                match result {
                    Err(e) if e.is::<super::stack::ActorStopped>() => Ok(false),
                    other => other.map_err(|e| {
                        e.downcast_ref::<String>()
                            .cloned()
                            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                            .unwrap_or_else(|| "actor panicked".into())
                    }),
                }
            }
        }
    }
}

impl BoundedStep {
    fn run(
        &mut self,
        process: &Arc<Mutex<Process>>,
        table: &ProcessTable,
        shutdown: &AtomicBool,
    ) -> bool {
        // Another actor can spend the worker's refill before we run again.
        // Guarantee this deferred actor one complete segment on its next turn.
        if std::mem::take(&mut self.deferred) {
            CURRENT_YIELDER.with(|slot| slot.reductions.set(slot.reductions.get().max(self.bound)));
        }
        loop {
            // Reserve enough reductions for the entire acyclic handler before
            // dequeue. All live continuation values are in the frame here.
            if CURRENT_YIELDER.with(|slot| slot.reductions.get() < self.bound) {
                self.deferred = true;
                CURRENT_YIELDER.with(|slot| slot.reductions.set(DEFAULT_REDUCTIONS));
                return true;
            }
            let ending = shutdown.load(Ordering::Relaxed)
                && !table.has_shutdown_work(Some(get_current_pid().unwrap()));
            let mut scratch = std::mem::MaybeUninit::uninit();
            let payload = {
                let mut p = process.lock();
                // Like receive_with, a running actor may consume ready messages
                // until it suspends. resume_process observes a forced exit at
                // either edge of the turn, including a failed Waiting transition.
                if p.heap.should_collect() {
                    let range = self.frame.as_ptr_range();
                    p.heap.collect(range.end.cast(), range.start.cast());
                }
                let Some(msg) = p
                    .mailbox
                    .remove_first(|m| m.buffer.type_tag == super::PROGRAM_MESSAGE_TAG)
                else {
                    if ending {
                        return false;
                    }
                    p.set_live_state(ProcessState::Waiting);
                    return true;
                };
                // A handler can load a byte or a word. The inline byte is
                // already zero-padded, and short materialized payloads also
                // need a full readable word when their sender used another type.
                if let Some(bits) = msg.buffer.plain_scalar(1) {
                    scratch.write(bits);
                    scratch.as_ptr().cast()
                } else {
                    super::receive_scalar_payload(&mut p, msg, 8, &mut scratch)
                }
            };
            if (self.call)(self.frame.as_mut_ptr(), payload) == 0 {
                return false;
            }
        }
    }
}

struct ScheduledActor {
    process: Arc<Mutex<Process>>,
    handle: Execution,
}

// Keep suspended coroutines in place across turns. Queue entries carry only a
// PID. A parked actor can accept exactly one wake before its next resume.
struct SuspendedActor {
    ordinary: bool,
    actor: ScheduledActor,
    parked: bool,
}

fn worker_loop(worker_index: usize, high_rx: Receiver<SpawnRequest>, shared: WorkerLoopShared) {
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
    let mut ordinary_residents = 0usize;
    let mut ready_heads = worker_index;
    let mut recorded_residents = 0;
    let mut shared_claim = SharedClaim::default();
    let mut runnable: Vec<ProcessId> = Vec::new();
    let mut ready = Vec::new();
    let mut woken = Vec::new();
    let mut suspended: FxHashMap<ProcessId, SuspendedActor> = FxHashMap::default();
    let mut next_sweep = Instant::now();
    let mut prefer_grouped = false;

    let mut spin_count: u32 = 0;
    let sleep = &*sleepers[worker_index];
    CURRENT_WORKER.set(sleep);
    let _ = sleep.thread.set(std::thread::current());
    let mut idle = IdleStretch {
        worker: sleep,
        published: false,
    };

    loop {
        let cycle_started_at = Instant::now();
        let mut did_work = false;
        let active_workers = active_threads.load(Ordering::Acquire);
        let accepting_new_work = worker_index < active_workers;

        if !accepting_new_work
            && runnable.is_empty()
            && suspended.is_empty()
            && !sleep.grouped.has_pending()
        {
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
        {
            let mut notifications = sleep.woken.lock();
            if !notifications.is_empty() {
                std::mem::swap(&mut woken, &mut notifications);
            }
        }
        for pid in woken.drain(..) {
            if let Some(entry) = suspended.get_mut(&pid) {
                if std::mem::take(&mut entry.parked) {
                    runnable.push(pid);
                }
            }
        }
        // The fallback sweep catches state changes without a notification.
        // Every nonparked entry is in runnable at this point.
        if suspended.len() > runnable.len() && cycle_started_at >= next_sweep {
            for (&pid, entry) in &mut suspended {
                if entry.parked
                    && !matches!(entry.actor.process.lock().state, ProcessState::Waiting)
                {
                    entry.parked = false;
                    runnable.push(pid);
                }
            }
            let cost = cycle_started_at.elapsed();
            next_sweep = Instant::now() + (cost * 8).max(std::time::Duration::from_millis(1));
        }
        std::mem::swap(&mut ready, &mut runnable);
        if !ready.is_empty() {
            idle.end();
        }
        for mut pid in ready.drain(..) {
            did_work = true;
            let mut entry = suspended.get_mut(&pid).expect("ready actor is suspended");
            // After the last handoff, all wakes go back to the worker FIFO.
            // Return one handoff to the FIFO every eighth ready actor. Fixed-length
            // chains can leave the same peers waiting on successive passes.
            ready_heads = ready_heads.wrapping_add(1);
            // On a worker running only grouped actors, every other chain may use
            // one extra turn. Check for queued starts at the handoff itself.
            let spare = ordinary_residents == 0 && ready_heads % 2 == 1;
            let budget =
                MAX_GROUP_HANDOFFS + usize::from(spare) - usize::from(ready_heads % 8 == 0);
            for turn in 0..=budget {
                let can_continue = turn < budget
                    && (turn < MAX_GROUP_HANDOFFS
                        || (high_rx.is_empty()
                            && injector.is_empty()
                            && !sleep.grouped.has_pending()));
                let capture = can_continue.then_some(sleep);
                let (outcome, next) = resume_process(
                    &process_table,
                    &active_count,
                    &sleepers,
                    pid,
                    &mut entry.actor,
                    capture,
                    &shutdown,
                );
                match outcome {
                    Resumed::Runnable => runnable.push(pid),
                    Resumed::Waiting => entry.parked = true,
                    Resumed::Finished => {
                        ordinary_residents -= usize::from(entry.ordinary);
                        suspended.remove(&pid);
                    }
                }
                let Some(next_pid) = next else { break };
                let Some(target) = suspended.get_mut(&next_pid) else {
                    break;
                };
                if !std::mem::take(&mut target.parked) {
                    break;
                }
                pid = next_pid;
                entry = target;
            }
        }

        // --- Phase 2: Try to get new spawn requests ---
        if suspended.len() != recorded_residents {
            recorded_residents = suspended.len();
            sleep.residents.store(recorded_residents, Ordering::Relaxed);
        }
        let mut defer_shared = false;
        let request = if accepting_new_work {
            high_rx.try_recv().ok().or_else(|| {
                let mut shared = || {
                    defer_shared = active_workers > 1
                        && recorded_residents >= 3
                        && !injector.is_empty()
                        && shared_claim.defer(
                            Instant::now(),
                            recorded_residents,
                            &sleepers[..active_workers],
                        );
                    if defer_shared {
                        return None;
                    }
                    let request = try_get_shared_request(&injector);
                    if request.is_some() {
                        sleep.shared_claims.fetch_add(1, Ordering::Relaxed);
                    }
                    request
                };
                if !sleep.grouped.has_pending() {
                    return shared();
                }
                // Each pass starts at most one actor. Alternate first choice
                // while both normal-priority sources are competing.
                let grouped = || sleep.grouped.pop();
                prefer_grouped = !prefer_grouped;
                if prefer_grouped {
                    grouped().or_else(shared)
                } else {
                    shared().or_else(grouped)
                }
            })
        } else {
            sleep.grouped.pop()
        };

        if let Some(req) = request {
            idle.end();
            did_work = true;
            let process = process_table.get(req.pid).expect(IN_TABLE);
            let (exited_reason, ordinary) = {
                let mut process = process.lock();
                let ordinary = matches!(process.placement, Placement::Shared);
                if let Placement::Grouped { worker } = process.placement {
                    debug_assert_eq!(worker, worker_index);
                }
                let exited = match &process.state {
                    ProcessState::Exited(reason) => Some(reason.clone()),
                    ProcessState::Ready | ProcessState::Running | ProcessState::Waiting => {
                        process.set_live_state(ProcessState::Running);
                        process.worker = Some(worker_index);
                        None
                    }
                };
                (exited, ordinary)
            };

            if let Some(reason) = exited_reason {
                finalize_managed_process(&process_table, &active_count, &sleepers, req.pid, reason);
            } else {
                // Create the coroutine only after confirming the queued actor is live.
                let mut actor = ScheduledActor {
                    process,
                    handle: match req.step {
                        Some(step) => Execution::Step(step),
                        None => Execution::Stack(CoroutineHandle::new(req.fn_ptr, req.args_ptr)),
                    },
                };
                let outcome = resume_process(
                    &process_table,
                    &active_count,
                    &sleepers,
                    req.pid,
                    &mut actor,
                    None,
                    &shutdown,
                )
                .0;
                if !matches!(outcome, Resumed::Finished) {
                    let parked = matches!(outcome, Resumed::Waiting);
                    ordinary_residents += usize::from(ordinary);
                    suspended.insert(
                        req.pid,
                        SuspendedActor {
                            actor,
                            parked,
                            ordinary,
                        },
                    );
                    if !parked {
                        runnable.push(req.pid);
                    }
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
            let all_waiting = runnable.is_empty() && !suspended.is_empty();

            if all_waiting && !process_table.has_shutdown_work(None) {
                // No processes can send more messages. Wake all Waiting
                // actors so they can detect shutdown and exit gracefully.
                // The mesh_actor_receive function checks is_shutdown()
                // and returns null when no other actors are active,
                // causing the service loop to exit cleanly.
                for (&pid, entry) in &mut suspended {
                    entry
                        .actor
                        .process
                        .lock()
                        .set_live_state(ProcessState::Ready);
                    entry.parked = false;
                    runnable.push(pid);
                }
                idle.end();
                did_work = true;
                // The actors will be resumed in Phase 1 on the next
                // iteration, and will exit when receive returns null.
            }

            // Also: if this worker has an empty suspended list, no pending
            // requests, and shutdown is active, exit the worker loop.
            if runnable.is_empty()
                && suspended.is_empty()
                && !did_work
                && active_count.load(Ordering::SeqCst) == 0
            {
                break;
            }
        }

        // Backoff when idle to avoid burning CPU: spin, then park for as long
        // as this used to sleep. `WorkerSleep::wake` cuts the park short.
        if !did_work && defer_shared {
            std::thread::yield_now();
        } else if !did_work {
            spin_count = spin_count.saturating_add(1);
            if spin_count > 100 {
                idle.park(std::time::Duration::from_micros(1_100));
            } else {
                std::hint::spin_loop();
            }
        } else {
            spin_count = 0;
        }
        crate::dist::telemetry::runtime_telemetry()
            .record_scheduler_cycle(did_work, cycle_started_at.elapsed());
    }
    CURRENT_WORKER.set(std::ptr::null());
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
    // A u64 and a usize are the same width on every target.
    let size = args_size as usize;
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
/// Why a worker's actor is in the process table: only that worker, once the
/// actor ends, takes it out (`finalize_managed_process`).
const IN_TABLE: &str = "an actor stays in the process table until its worker finalizes it";

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
    sleepers: &[CachePadded<WorkerSleep>],
    pid: ProcessId,
    actor: &mut ScheduledActor,
    handoff_worker: Option<&WorkerSleep>,
    shutdown: &AtomicBool,
) -> (Resumed, Option<ProcessId>) {
    let (exited_reason, placement) = {
        let process = actor.process.lock();
        let reason = match &process.state {
            ProcessState::Exited(reason) => Some(reason.clone()),
            _ => None,
        };
        (reason, process.placement)
    };
    if let Some(reason) = exited_reason {
        finalize_managed_process(process_table, active_count, sleepers, pid, reason);
        return (Resumed::Finished, None);
    }

    set_current_process(pid, &actor.process);
    let (result, next) = match placement {
        Placement::Shared => (
            actor.handle.resume(&actor.process, process_table, shutdown),
            None,
        ),
        Placement::Grouped { .. } => {
            let mut resume = || {
                crate::panic::defer_diagnostics(|| {
                    actor.handle.resume(&actor.process, process_table, shutdown)
                })
            };
            match handoff_worker {
                Some(worker) => capture_handoff(worker, resume),
                None => (resume(), None),
            }
        }
    };
    clear_current_pid();
    CURRENT_YIELDER.with(|current| current.yielder.set(None));

    let outcome = match result {
        Ok(true) => {
            let mut outcome = Resumed::Runnable;
            let exited_reason = {
                let mut process = actor.process.lock();
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
            };
            if let Some(reason) = exited_reason {
                finalize_managed_process(process_table, active_count, sleepers, pid, reason);
                return (Resumed::Finished, next);
            }
            outcome
        }
        result => {
            let reason = match result {
                Ok(false) => ExitReason::Normal,
                Err(message) => ExitReason::Error(message),
                Ok(true) => unreachable!(),
            };
            finalize_managed_process(process_table, active_count, sleepers, pid, reason);
            Resumed::Finished
        }
    };
    (outcome, next)
}

fn finalize_managed_process(
    process_table: &ProcessTable,
    active_count: &AtomicU64,
    sleepers: &[CachePadded<WorkerSleep>],
    pid: ProcessId,
    reason: ExitReason,
) {
    if !handle_process_exit(process_table, sleepers, pid, reason) {
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

const SHARED_CLAIM_GRACE: Duration = Duration::from_micros(24);
const SHARED_CLAIM_RETRY: Duration = Duration::from_millis(1);

#[derive(Default)]
struct SharedClaim {
    offer: Option<ClaimOffer>,
    cooldowns: Vec<Option<ClaimOfferStatus>>,
    next_peer: usize,
}

#[derive(Clone, Copy)]
struct ClaimOfferStatus {
    claims: usize,
    retry_at: Instant,
}

struct ClaimOffer {
    peer: usize,
    claims: usize,
    deadline: Instant,
}

impl SharedClaim {
    /// Give a much lighter worker one chance to take work from the same FIFO.
    /// Check peers only when ordinary work is queued and this worker is loaded.
    fn defer(
        &mut self,
        now: Instant,
        residents: usize,
        active: &[CachePadded<WorkerSleep>],
    ) -> bool {
        if let Some(offer) = &self.offer {
            if let Some(peer) = active.get(offer.peer) {
                if now < offer.deadline {
                    return true;
                }
                if peer.shared_claims.load(Ordering::Relaxed) == offer.claims {
                    self.cooldowns[offer.peer] = Some(ClaimOfferStatus {
                        claims: offer.claims,
                        retry_at: offer.deadline + SHARED_CLAIM_RETRY,
                    });
                }
            }
            // Always allow a claim between two offers.
            self.offer = None;
            return false;
        }

        if self.cooldowns.len() < active.len() {
            self.cooldowns.resize(active.len(), None);
        }
        let start = self.next_peer % active.len();
        for index in (start..active.len()).chain(0..start) {
            let peer = &active[index];
            if peer.residents.load(Ordering::Relaxed).saturating_add(2) >= residents {
                continue;
            }
            let claims = peer.shared_claims.load(Ordering::Relaxed);
            if self.cooldowns[index]
                .is_some_and(|cooldown| now < cooldown.retry_at && cooldown.claims == claims)
            {
                continue;
            }
            self.next_peer = index + 1;
            self.offer = Some(ClaimOffer {
                peer: index,
                claims,
                deadline: now + SHARED_CLAIM_GRACE,
            });
            peer.wake();
            return true;
        }
        false
    }
}

fn try_get_shared_request(injector: &Injector<SpawnRequest>) -> Option<SpawnRequest> {
    loop {
        match injector.steal() {
            Steal::Success(req) => return Some(req),
            Steal::Empty => return None,
            Steal::Retry => continue,
        }
    }
}

/// Handle process exit: invoke terminate callback, propagate exit to links,
/// clean up from process table.
///
/// 1. Claim finalization and preserve the first recorded exit reason
/// 2. Invoke terminate_callback once (panic-safe)
/// 3. Destroy remaining secrets, notify links/monitors, and remove the process
fn handle_process_exit(
    process_table: &ProcessTable,
    sleepers: &[CachePadded<WorkerSleep>],
    pid: ProcessId,
    fallback_reason: ExitReason,
) -> bool {
    // Claim finalization and extract callback/notification state under one lock.
    let Some((reason, terminate_cb, linked_pids, monitored_by_entries)) = process_table
        .with_process(pid, |proc_arc| {
            let mut proc = proc_arc.lock();
            let reason = proc.begin_exit_finalization(fallback_reason)?;
            let cb = proc
                .terminate_callback
                .take()
                .map(|cb| (cb, proc.placement));
            let links = std::mem::take(&mut proc.links);
            let monitored_by = std::mem::take(&mut proc.monitored_by);
            Some((reason, cb, links, monitored_by))
        })
        .flatten()
    else {
        return false;
    };

    // Step 1: Invoke terminate callback (panic-safe).
    if let Some((cb, placement)) = terminate_cb {
        match placement {
            Placement::Shared => invoke_terminate_callback(cb, &reason),
            Placement::Grouped { worker } => {
                let previous = TERMINATING_GROUP_WORKER.replace(Some(worker));
                invoke_terminate_callback(cb, &reason);
                TERMINATING_GROUP_WORKER.set(previous);
            }
        }
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
        process_table.get(linked_pid)
    });

    for linked_pid in woken {
        let worker = process_table
            .get(linked_pid)
            .and_then(|process| process.lock().worker);
        notify_worker(sleepers, worker, linked_pid);
    }

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
            let worker = process_table
                .with_process(*monitoring_pid, |mon_proc_arc| {
                    let mut mon_proc = mon_proc_arc.lock();
                    if mon_proc.fire_monitor(*monitor_ref)
                        && matches!(mon_proc.state, ProcessState::Waiting)
                    {
                        mon_proc.set_live_state(ProcessState::Ready);
                        mon_proc.worker
                    } else {
                        None
                    }
                })
                .flatten();
            notify_worker(sleepers, worker, *monitoring_pid);
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
    process_table.remove(pid);
    true
}

/// Invoke a terminate callback, catching any panics to prevent them from
/// crashing the runtime.
fn invoke_terminate_callback(cb: TerminateCallback, reason: &ExitReason) {
    // Encode the reason as a simple tag byte for the callback.
    let reason_tag = reason.tag();

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
    use super::super::stack::set_current_pid;
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
    fn send_target_tracks_table_membership_while_removed_handles_remain_live() {
        let scheduler = Scheduler::new(1);
        let other = Scheduler::new(1);
        let pid = ProcessId::next();
        let process = || Arc::new(Mutex::new(Process::new(pid, Priority::Normal)));
        let first = process();
        let second = process();
        let table = scheduler.process_table();

        table.insert(pid, first.clone());
        for _ in 0..2 {
            assert!(Arc::ptr_eq(&scheduler.send_target(pid).unwrap(), &first));
        }
        table.insert(pid, second.clone());
        assert!(Arc::ptr_eq(&scheduler.send_target(pid).unwrap(), &second));
        assert!(other.send_target(pid).is_none());
        other.process_table().insert(pid, first.clone());
        assert!(Arc::ptr_eq(&other.send_target(pid).unwrap(), &first));
        assert!(Arc::ptr_eq(&scheduler.send_target(pid).unwrap(), &second));

        std::thread::scope(|scope| {
            scope.spawn(|| table.remove(pid)).join().unwrap();
        });
        assert!(scheduler.send_target(pid).is_none());
        table.insert(pid, first.clone());
        assert!(Arc::ptr_eq(&scheduler.send_target(pid).unwrap(), &first));
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
        let process_table = ProcessTable::new();
        process_table.insert(
            pid,
            Arc::new(Mutex::new(Process::new(pid, Priority::Normal))),
        );
        let active_count = AtomicU64::new(1);
        let mut actor = ScheduledActor {
            process: process_table.get(pid).unwrap(),
            handle: Execution::Stack(CoroutineHandle::new(
                yield_once as *const u8,
                std::ptr::null(),
            )),
        };

        assert!(matches!(
            resume_process(
                &process_table,
                &active_count,
                &[],
                pid,
                &mut actor,
                None,
                &AtomicBool::new(false)
            )
            .0,
            Resumed::Runnable
        ));
        assert_eq!(ENTRY_COUNT.load(Ordering::SeqCst), 1);
        process_table
            .with_process(pid, |process| {
                process.lock().mark_exited(ExitReason::Killed)
            })
            .unwrap();

        assert!(matches!(
            resume_process(
                &process_table,
                &active_count,
                &[],
                pid,
                &mut actor,
                None,
                &AtomicBool::new(false)
            )
            .0,
            Resumed::Finished
        ));
        assert_eq!(ENTRY_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(active_count.load(Ordering::SeqCst), 0);
        assert!(!process_table.contains_key(pid));
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
        let process_table = ProcessTable::new();
        process_table.insert(
            pid,
            Arc::new(Mutex::new(Process::new(pid, Priority::Normal))),
        );
        let active_count = AtomicU64::new(1);
        let mut actor = ScheduledActor {
            process: process_table.get(pid).unwrap(),
            handle: Execution::Stack(CoroutineHandle::new(
                yield_after_exit as *const u8,
                std::ptr::null(),
            )),
        };

        let retained = std::thread::scope(|scope| {
            scope.spawn(|| {
                while ENTRY_COUNT.load(Ordering::SeqCst) == 0 {
                    std::thread::yield_now();
                }
                process_table
                    .with_process(pid, |process| {
                        process.lock().mark_exited(ExitReason::Killed)
                    })
                    .unwrap();
                MAY_YIELD.store(true, Ordering::SeqCst);
            });
            resume_process(
                &process_table,
                &active_count,
                &[],
                pid,
                &mut actor,
                None,
                &AtomicBool::new(false),
            )
            .0
        });

        assert!(matches!(retained, Resumed::Finished));
        assert_eq!(ENTRY_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(active_count.load(Ordering::SeqCst), 0);
        assert!(!process_table.contains_key(pid));
    }

    #[test]
    fn queued_exited_actor_is_finalized_once_without_starting() {
        static ENTRY_COUNT: AtomicU64 = AtomicU64::new(0);

        extern "C" fn should_not_run(_args: *const u8) {
            ENTRY_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        extern "C-unwind" fn should_not_step(_: *mut u64, _: *const u8) -> u8 {
            ENTRY_COUNT.fetch_add(1, Ordering::SeqCst);
            0
        }

        for step in [false, true] {
            ENTRY_COUNT.store(0, Ordering::SeqCst);
            FORCED_TERMINATE_COUNT.store(0, Ordering::SeqCst);
            FORCED_TERMINATE_REASON.store(u64::MAX, Ordering::SeqCst);
            let scheduler = Scheduler::new(1);
            let target = if step {
                scheduler.spawn_step(
                    should_not_step as *const u8,
                    1,
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                )
            } else {
                scheduler.spawn(should_not_run as *const u8, std::ptr::null(), 0, 1)
            };
            let target_handle = scheduler.get_process(target).unwrap();
            target_handle.lock().mailbox.push(crate::actor::Message {
                buffer: crate::actor::MessageBuffer::from_slice(
                    &[1],
                    crate::actor::PROGRAM_MESSAGE_TAG,
                ),
            });
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
                observer_process.monitors.insert(
                    monitor_ref,
                    crate::actor::process::Monitor {
                        target,
                        message: crate::actor::heap::MessageBuffer::new(vec![7], 99),
                    },
                );
            }
            crate::secret::insert_test_secret(target);
            scheduler
                .get_process(target)
                .unwrap()
                .lock()
                .mark_exited(ExitReason::Killed);

            scheduler.signal_shutdown();
            scheduler.run();

            assert_eq!(target_handle.lock().mailbox.len(), 1);
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
                [link::EXIT_SIGNAL_TAG, 99],
                "the exit signal, then the monitor's own message"
            );
        }
    }

    #[test]
    fn managed_process_finalization_is_idempotent() {
        IDEMPOTENT_TERMINATE_COUNT.store(0, Ordering::SeqCst);
        let pid = ProcessId::next();
        let process_table = ProcessTable::new();
        let mut process = Process::new(pid, Priority::Normal);
        process.terminate_callback = Some(observe_idempotent_terminate);
        process.mark_exited(ExitReason::Killed);
        process_table.insert(pid, Arc::new(Mutex::new(process)));
        let active_count = AtomicU64::new(1);

        finalize_managed_process(&process_table, &active_count, &[], pid, ExitReason::Normal);
        finalize_managed_process(&process_table, &active_count, &[], pid, ExitReason::Normal);

        assert_eq!(IDEMPOTENT_TERMINATE_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(active_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_repeated_exit_claim_leaves_pending_work_to_the_finalizer() {
        let pid = ProcessId::next();
        let table = ProcessTable::new();
        let mut process = Process::new(pid, Priority::Normal);
        assert_eq!(
            process.begin_exit_finalization(ExitReason::Killed),
            Some(ExitReason::Killed),
        );
        table.insert(pid, Arc::new(Mutex::new(process)));
        let active_count = AtomicU64::new(1);

        finalize_managed_process(&table, &active_count, &[], pid, ExitReason::Normal);

        let process = table.get(pid).unwrap();
        assert_eq!(
            process.lock().state,
            ProcessState::Exited(ExitReason::Killed)
        );
        assert!(process.lock().has_shutdown_work());
        assert_eq!(active_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_linked_exit_queues_the_waiting_recipients_worker() {
        let scheduler = Scheduler::new(2);
        let exiting_pid = scheduler.create_main_process();
        let recipient_pid = scheduler.create_main_process();
        let exiting = scheduler.get_process(exiting_pid).unwrap();
        let recipient = scheduler.get_process(recipient_pid).unwrap();
        link::link(&exiting, &recipient, exiting_pid, recipient_pid);
        {
            let mut process = recipient.lock();
            process.trap_exit = true;
            process.worker = Some(1);
            process.set_live_state(ProcessState::Waiting);
        }

        scheduler.finalize_host_process(exiting_pid);

        let process = recipient.lock();
        assert!(matches!(process.state, ProcessState::Ready));
        let signal = process.mailbox.pop().unwrap();
        assert_eq!(signal.buffer.type_tag, link::EXIT_SIGNAL_TAG);
        assert!(scheduler.sleepers[0].woken.lock().is_empty());
        assert_eq!(*scheduler.sleepers[1].woken.lock(), [recipient_pid]);
    }

    #[test]
    fn shutdown_receive_waits_for_a_linked_exit_before_its_worker_claims_it() {
        use crate::actor::MessageBuffer;

        struct Observer<'a> {
            scheduler: &'a Scheduler,
            received: Mutex<Option<MessageBuffer>>,
        }

        extern "C-unwind" fn observe(args: *const u8) {
            let observer = unsafe { &*args.cast::<Observer<'_>>() };
            super::super::receive_with(observer.scheduler, -1, |process| {
                *observer.received.lock() = Some(process.mailbox.pop()?.buffer);
                Some(std::ptr::dangling())
            });
        }

        extern "C" fn noop(_: *const u8) {}

        for monitor in [false, true] {
            let scheduler = Scheduler::new(2);
            let spawn = || scheduler.spawn(noop as *const u8, std::ptr::null(), 0, 1);
            let (a, b, c) = (spawn(), spawn(), spawn());
            let (proc_a, proc_b, proc_c) = (
                scheduler.get_process(a).unwrap(),
                scheduler.get_process(b).unwrap(),
                scheduler.get_process(c).unwrap(),
            );
            {
                let mut proc_b = proc_b.lock();
                proc_b.worker = Some(0);
                proc_b.set_live_state(ProcessState::Waiting);
            }
            proc_c.lock().worker = Some(1);
            link::link(&proc_a, &proc_b, a, b);
            if monitor {
                super::super::watch(
                    &scheduler,
                    &proc_c,
                    c,
                    b,
                    MessageBuffer::from_slice(b"DOWN", super::super::PROGRAM_MESSAGE_TAG),
                );
            } else {
                proc_c.lock().trap_exit = true;
                link::link(&proc_b, &proc_c, b, c);
            }

            let observer = Observer {
                scheduler: &scheduler,
                received: Mutex::new(None),
            };
            let mut actor = ScheduledActor {
                process: proc_c,
                handle: Execution::Stack(CoroutineHandle::new(
                    observe as *const u8,
                    std::ptr::from_ref(&observer).cast(),
                )),
            };
            let resume_observer = |actor: &mut ScheduledActor| {
                resume_process(
                    scheduler.process_table(),
                    &scheduler.active_count,
                    &scheduler.sleepers,
                    c,
                    actor,
                    None,
                    &scheduler.shutdown,
                )
                .0
            };
            assert!(matches!(resume_observer(&mut actor), Resumed::Waiting));

            let timeout = Duration::from_secs(10);
            let (a_removed, wait_for_a) = crossbeam_channel::bounded(1);
            let (release_b, wait_for_c) = crossbeam_channel::bounded(1);
            let (during_exit, b_wakes) = std::thread::scope(|scope| {
                let scheduler = &scheduler;
                scope.spawn(move || {
                    let finalize = |pid, reason| {
                        finalize_managed_process(
                            scheduler.process_table(),
                            &scheduler.active_count,
                            &scheduler.sleepers,
                            pid,
                            reason,
                        );
                    };
                    finalize(a, ExitReason::Killed);
                    a_removed.send(()).unwrap();
                    wait_for_c.recv_timeout(timeout).unwrap();
                    finalize(b, ExitReason::Normal);
                });
                wait_for_a.recv_timeout(timeout).unwrap();
                scheduler.signal_shutdown();
                // Give C a receive turn while B's owner is still held before its claim.
                let during_exit = resume_observer(&mut actor);
                let b_wakes = std::mem::take(&mut *scheduler.sleepers[0].woken.lock());
                release_b.send(()).unwrap();
                (during_exit, b_wakes)
            });

            assert!(
                matches!(during_exit, Resumed::Waiting),
                "C ended its receive before B could send its exit notification",
            );
            assert_eq!(b_wakes, [b]);
            assert_eq!(*scheduler.sleepers[1].woken.lock(), [c]);
            assert!(matches!(resume_observer(&mut actor), Resumed::Finished));
            let received = observer.received.into_inner().unwrap();
            if monitor {
                assert_eq!(received.data(), b"DOWN");
            } else {
                assert_eq!(received.type_tag, link::EXIT_SIGNAL_TAG);
                assert_eq!(
                    link::decode_exit_signal(received.data()),
                    Some((b, ExitReason::Linked(a, Box::new(ExitReason::Killed)))),
                );
            }
            assert_eq!(scheduler.active_count(), 0);
        }
    }

    #[test]
    fn shutdown_waits_when_it_cannot_check_a_process_state() {
        let scheduler = Scheduler::new(1);
        let pid = scheduler.create_main_process();
        let process = scheduler.get_process(pid).unwrap();
        let mut locked = process.lock();
        locked.set_live_state(ProcessState::Waiting);

        assert!(scheduler.process_table().has_shutdown_work(None));

        drop(locked);
        assert!(!scheduler.process_table().has_shutdown_work(None));
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
        sched.process_table.remove(child_pid);
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
        sched.process_table.remove(parent_pid);
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

    /// No worker count means one per core; elastic bounds must be positive
    /// and ordered; a worker above the active count leaves at shutdown too.
    #[test]
    fn worker_counts_and_an_elastic_scheduler_shutting_down() {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert_eq!(Scheduler::new(0).worker_bounds().1, cores);
        assert!(Scheduler::new_elastic(0, 1).is_err());
        assert!(Scheduler::new_elastic(2, 1).is_err());
        let elastic = Scheduler::new_elastic(1, 2).unwrap();
        elastic.start();
        elastic.signal_shutdown();
        elastic.wait();
    }

    type SpawnTrace = Mutex<Vec<(usize, std::thread::ThreadId)>>;

    struct SpawnProbe<'a> {
        label: usize,
        trace: &'a SpawnTrace,
    }

    extern "C" fn record_spawn(args: *const u8) {
        let probe = unsafe { &*args.cast::<SpawnProbe<'_>>() };
        probe
            .trace
            .lock()
            .push((probe.label, std::thread::current().id()));
    }

    #[test]
    fn ordinary_spawn_fifo_keeps_enqueue_order_behind_high_priority() {
        let scheduler = Scheduler::new(1);
        let trace = SpawnTrace::default();
        let probes: [_; 12] = std::array::from_fn(|label| SpawnProbe {
            label,
            trace: &trace,
        });
        for probe in &probes {
            let priority = match probe.label {
                11 => 0,
                n if n % 2 == 0 => 1,
                _ => 2,
            };
            scheduler.spawn(
                record_spawn as *const u8,
                std::ptr::from_ref(probe).cast(),
                0,
                priority,
            );
        }
        scheduler.signal_shutdown();
        scheduler.run();

        let labels: Vec<_> = trace
            .into_inner()
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert_eq!(labels, std::iter::once(11).chain(0..11).collect::<Vec<_>>());
    }

    #[test]
    fn shared_claim_timeout_allows_work_until_lighter_peer_progresses() {
        let workers: [CachePadded<WorkerSleep>; 2] = std::array::from_fn(|_| Default::default());
        workers[0].residents.store(3, Ordering::Relaxed);
        let mut claim = SharedClaim::default();
        let start = Instant::now();
        let deadline = start + SHARED_CLAIM_GRACE;

        assert!(claim.defer(start, 3, &workers));
        assert!(claim.defer(deadline - Duration::from_micros(1), 3, &workers));
        for i in 0..5 {
            assert!(!claim.defer(deadline + Duration::from_micros(i), 3, &workers));
        }

        workers[1].shared_claims.fetch_add(1, Ordering::Relaxed);
        let now = deadline + Duration::from_micros(5);
        assert!(claim.defer(now, 3, &workers));
        assert!(!claim.defer(now, 3, &workers[..1]));

        assert!(claim.defer(now, 3, &workers));
        let retry = now + SHARED_CLAIM_GRACE + SHARED_CLAIM_RETRY;
        assert!(!claim.defer(retry, 3, &workers));
        assert!(claim.defer(retry, 3, &workers));
    }

    #[test]
    fn an_unresponsive_peer_does_not_delay_an_offer_to_another_worker() {
        let workers: [CachePadded<WorkerSleep>; 3] = std::array::from_fn(|_| Default::default());
        workers[0].residents.store(4, Ordering::Relaxed);
        let mut claim = SharedClaim::default();
        let start = Instant::now();
        assert!(claim.defer(start, 4, &workers));
        assert_eq!(claim.offer.as_ref().unwrap().peer, 1);

        let deadline = start + SHARED_CLAIM_GRACE;
        assert!(!claim.defer(deadline, 4, &workers));
        assert!(claim.defer(deadline, 4, &workers));
        assert_eq!(claim.offer.as_ref().unwrap().peer, 2);

        let next = deadline + SHARED_CLAIM_GRACE;
        assert!(!claim.defer(next, 4, &workers));
        assert!(!claim.defer(next, 4, &workers));
        workers[1].shared_claims.fetch_add(1, Ordering::Relaxed);
        assert!(claim.defer(next, 4, &workers));
        assert_eq!(claim.offer.as_ref().unwrap().peer, 1);
    }

    #[test]
    fn a_worker_stops_advertising_idle_before_entering_an_actor() {
        use std::sync::mpsc::{channel, Receiver, Sender};

        struct Blocker {
            entered: Sender<bool>,
            release: Mutex<Receiver<()>>,
        }

        extern "C-unwind" fn block(args: *const u8) {
            let blocker = unsafe { &*args.cast::<Blocker>() };
            let worker = unsafe { CURRENT_WORKER.get().as_ref().unwrap() };
            blocker
                .entered
                .send(worker.idle.load(Ordering::SeqCst))
                .unwrap();
            let _ = blocker.release.lock().recv();
        }

        let scheduler = Scheduler::new(2);
        let (entered, entry_event) = channel();
        let (release, released) = channel();
        let blocker = Blocker {
            entered,
            release: Mutex::new(released),
        };
        let timeout = Duration::from_secs(5);
        scheduler.start();
        let deadline = Instant::now() + timeout;
        let all_idle = || {
            scheduler
                .sleepers
                .iter()
                .all(|worker| worker.idle.load(Ordering::SeqCst))
        };
        while !all_idle() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let result = all_idle().then(|| {
            scheduler.spawn(
                block as *const u8,
                std::ptr::from_ref(&blocker).cast(),
                0,
                1,
            );
            entry_event.recv_timeout(timeout)
        });
        scheduler.signal_shutdown();
        drop(release);
        scheduler.wait();
        assert_eq!(result, Some(Ok(false)));
    }

    #[test]
    fn ordinary_children_progress_while_lighter_worker_is_blocked_after_resize() {
        use std::sync::mpsc::{channel, Receiver, Sender};

        const CHILDREN: usize = 16;
        const TIMEOUT: Duration = Duration::from_secs(5);

        struct Scenario {
            scheduler: Scheduler,
            parked: Sender<()>,
            completed: Sender<std::thread::ThreadId>,
        }

        struct Blocker {
            entered: Sender<std::thread::ThreadId>,
            release: Mutex<Receiver<()>>,
        }

        extern "C-unwind" fn park(args: *const u8) {
            let scenario = unsafe { &*args.cast::<Scenario>() };
            let process = scenario
                .scheduler
                .get_process(get_current_pid().unwrap())
                .unwrap();
            process.lock().set_live_state(ProcessState::Waiting);
            scenario.parked.send(()).unwrap();
            super::super::stack::yield_current();
        }

        extern "C-unwind" fn block(args: *const u8) {
            let blocker = unsafe { &*args.cast::<Blocker>() };
            blocker.entered.send(std::thread::current().id()).unwrap();
            let _ = blocker.release.lock().recv();
        }

        extern "C-unwind" fn child(args: *const u8) {
            let scenario = unsafe { &*args.cast::<Scenario>() };
            scenario
                .completed
                .send(std::thread::current().id())
                .unwrap();
        }

        extern "C-unwind" fn parent(args: *const u8) {
            let scenario = unsafe { &*args.cast::<Scenario>() };
            for _ in 0..CHILDREN {
                scenario.scheduler.spawn(child as *const u8, args, 0, 1);
            }
        }

        let (parked, park_events) = channel();
        let (completed, child_events) = channel();
        let scenario = Scenario {
            scheduler: Scheduler::new_elastic(1, 2).unwrap(),
            parked,
            completed,
        };
        let scheduler = &scenario.scheduler;
        let args = std::ptr::from_ref(&scenario).cast();
        let blocker = || {
            let (entered, entry_event) = channel();
            let (release, wait) = channel();
            (
                Blocker {
                    entered,
                    release: Mutex::new(wait),
                },
                entry_event,
                release,
            )
        };
        let (first, first_entry, release_first) = blocker();
        let (second, second_entry, release_second) = blocker();

        // Keep all the borrowed actor arguments alive until the scheduler joins.
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| scheduler.run());
            let progress = (|| {
                for _ in 0..3 {
                    scheduler.spawn(park as *const u8, args, 0, 1);
                    park_events.recv_timeout(TIMEOUT)?;
                }
                scheduler.spawn(block as *const u8, std::ptr::from_ref(&first).cast(), 0, 1);
                let loaded = first_entry.recv_timeout(TIMEOUT)?;

                // With the sole active worker blocked, the second blocker is queued
                // until resize activates a lighter worker to claim it.
                scheduler.spawn(block as *const u8, std::ptr::from_ref(&second).cast(), 0, 1);
                scheduler.resize(2).unwrap();
                let lighter = second_entry.recv_timeout(TIMEOUT)?;

                scheduler.spawn(parent as *const u8, args, 0, 1);
                drop(release_first);
                let children = (0..CHILDREN)
                    .map(|_| child_events.recv_timeout(TIMEOUT))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<_, std::sync::mpsc::RecvTimeoutError>((loaded, lighter, children))
            })();
            scheduler.signal_shutdown();
            drop(release_second);
            progress
        });
        let (loaded, lighter, children) =
            outcome.expect("children finish while the lighter worker is blocked");
        assert_ne!(loaded, lighter);
        assert_eq!(children, vec![loaded; CHILDREN]);
        assert_eq!(scheduler.active_count(), 0);
    }

    #[test]
    fn grouped_roots_distribute_in_worker_fifo_order() {
        let scheduler = Scheduler::new(3);
        let trace = SpawnTrace::default();
        let probes: [_; 12] = std::array::from_fn(|label| SpawnProbe {
            label,
            trace: &trace,
        });
        for probe in &probes {
            scheduler.spawn_grouped(
                record_spawn as *const u8,
                std::ptr::from_ref(probe).cast(),
                0,
                std::ptr::null(),
                None,
            );
        }
        scheduler.signal_shutdown();
        scheduler.run();

        let trace = trace.lock();
        for (worker, sleep) in scheduler.sleepers.iter().enumerate() {
            let thread = sleep.thread.get().unwrap().id();
            let labels: Vec<_> = trace
                .iter()
                .filter_map(|&(label, id)| (id == thread).then_some(label))
                .collect();
            assert_eq!(
                labels,
                (worker..probes.len()).step_by(3).collect::<Vec<_>>()
            );
        }
        assert_eq!(scheduler.active_count(), 0);
    }

    #[test]
    fn grouped_spawns_share_the_spawn_budget_behind_high_priority() {
        let scheduler = Scheduler::new(1);
        let trace = SpawnTrace::default();
        let probes: [_; 7] = std::array::from_fn(|label| SpawnProbe {
            label,
            trace: &trace,
        });
        for probe in &probes {
            let args = std::ptr::from_ref(probe).cast();
            match probe.label {
                0..=2 => {
                    scheduler.spawn_grouped(
                        record_spawn as *const u8,
                        args,
                        0,
                        std::ptr::null(),
                        None,
                    );
                }
                3..=5 => {
                    scheduler.spawn(record_spawn as *const u8, args, 0, 1);
                }
                _ => {
                    scheduler.spawn(record_spawn as *const u8, args, 0, 0);
                }
            }
        }
        scheduler.signal_shutdown();
        scheduler.run();

        let labels: Vec<_> = trace
            .into_inner()
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert_eq!(labels[0], 6);
        assert_eq!([labels[1], labels[3], labels[5]], [0, 1, 2]);
        assert!(labels[2..]
            .iter()
            .step_by(2)
            .all(|label| (3..=5).contains(label)));
    }

    #[test]
    fn grouped_descendants_drain_after_resize_while_ordinary_spawns_use_active_worker() {
        struct GroupTree<'a> {
            scheduler: &'a Scheduler,
            grouped: SpawnProbe<'a>,
            ordinary: SpawnProbe<'a>,
            next: Option<&'a GroupTree<'a>>,
        }

        extern "C" fn spawn_descendant(args: *const u8) {
            let node = unsafe { &*args.cast::<GroupTree<'_>>() };
            record_spawn(std::ptr::from_ref(&node.grouped).cast());
            node.scheduler.spawn(
                record_spawn as *const u8,
                std::ptr::from_ref(&node.ordinary).cast(),
                0,
                1,
            );
            if let Some(next) = node.next {
                node.scheduler.spawn_grouped(
                    spawn_descendant as *const u8,
                    std::ptr::from_ref(next).cast(),
                    0,
                    std::ptr::null(),
                    None,
                );
            }
        }

        let scheduler = Scheduler::new_elastic(1, 2).unwrap();
        let trace = SpawnTrace::default();
        let node = |label, next| GroupTree {
            scheduler: &scheduler,
            grouped: SpawnProbe {
                label,
                trace: &trace,
            },
            ordinary: SpawnProbe {
                label: label + 1,
                trace: &trace,
            },
            next,
        };
        let grandchild = node(4, None);
        let child = node(2, Some(&grandchild));
        let parent = node(0, Some(&child));
        let root = SpawnProbe {
            label: 6,
            trace: &trace,
        };
        let fresh = SpawnProbe {
            label: 7,
            trace: &trace,
        };
        scheduler.resize(2).unwrap();
        scheduler.spawn_grouped(
            record_spawn as *const u8,
            std::ptr::from_ref(&root).cast(),
            0,
            std::ptr::null(),
            None,
        );
        scheduler.spawn_grouped(
            spawn_descendant as *const u8,
            std::ptr::from_ref(&parent).cast(),
            0,
            std::ptr::null(),
            None,
        );
        scheduler.resize(1).unwrap();
        scheduler.spawn_grouped(
            record_spawn as *const u8,
            std::ptr::from_ref(&fresh).cast(),
            0,
            std::ptr::null(),
            None,
        );
        scheduler.signal_shutdown();
        scheduler.run();

        let mut trace = trace.into_inner();
        trace.sort_unstable_by_key(|&(label, _)| label);
        let [active, retired] = [0, 1].map(|i| scheduler.sleepers[i].thread.get().unwrap().id());
        assert_eq!(
            trace,
            vec![
                (0, retired),
                (1, active),
                (2, retired),
                (3, active),
                (4, retired),
                (5, active),
                (6, active),
                (7, active)
            ]
        );
        assert_eq!(scheduler.active_count(), 0);
    }

    /// The monitor of an actor that ends is dropped when the actor watching
    /// it has already left the table.
    #[test]
    fn an_ending_actor_passes_over_a_watcher_that_is_gone() {
        let scheduler = Scheduler::new(1);
        let target = scheduler.create_main_process();
        let gone = ProcessId(u64::MAX >> 24);
        let process = scheduler.get_process(target).unwrap();
        process.lock().monitored_by.insert(5, gone);
        scheduler.finalize_host_process(target);
        assert!(scheduler.get_process(target).is_none());
    }

    /// A linked spawn links both ways before the actor can run; a parent
    /// that has already gone leaves the child's side of the link unanswered.
    #[test]
    fn spawn_linked_links_both_ways_before_the_actor_runs() {
        let sched = Scheduler::new(1);
        let entry = increment_entry as *const u8;
        let parent = sched.create_main_process();
        let child = sched.spawn_linked(entry, std::ptr::null(), 0, 1, parent, false);
        let gone = ProcessId(u64::MAX >> 24);
        let orphan = sched.spawn_linked(entry, std::ptr::null(), 0, 1, gone, true);

        let links = |pid| sched.get_process(pid).unwrap().lock().links.clone();
        assert!(links(parent).contains(&child));
        assert!(links(child).contains(&parent));
        assert!(links(orphan).contains(&gone));
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
            sched.process_table.read_all().iter().next().is_none(),
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
    fn ordinary_spawns_can_be_claimed_while_another_worker_is_busy() {
        use std::sync::mpsc::{channel, Receiver, Sender};
        use std::thread::{self, ThreadId};

        struct Probe {
            claimed: Sender<ThreadId>,
            release: Mutex<Receiver<()>>,
        }

        extern "C-unwind" fn record_claim(args: *const u8) {
            let probe = unsafe { &*args.cast::<Probe>() };
            probe.claimed.send(thread::current().id()).unwrap();
            let _ = probe.release.lock().recv();
        }

        let (claimed, claims) = channel();
        let (release, wait) = channel();
        let probe = Probe {
            claimed,
            release: Mutex::new(wait),
        };
        let scheduler = Scheduler::new(2);
        let args = std::ptr::from_ref(&probe).cast();
        for _ in 0..2 {
            scheduler.spawn(record_claim as *const u8, args, 0, 1);
        }
        scheduler.signal_shutdown();

        let (first, second) = thread::scope(|scope| {
            scope.spawn(|| scheduler.run());
            let timeout = Duration::from_secs(5);
            let first = claims.recv_timeout(timeout);
            let second = claims.recv_timeout(timeout);
            drop(release);
            (first, second)
        });
        assert_ne!(
            first.expect("first worker claims an actor"),
            second.expect("second worker claims an actor while the first is blocked")
        );
        assert_eq!(scheduler.active_count(), 0);
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

    mod handoff_fairness {
        use super::*;
        use crate::actor::stack::yield_current;

        const CHAIN: usize = 12;
        const ORDINARY: usize = 3;

        struct Scenario {
            scheduler: Scheduler,
            ordinary: std::sync::OnceLock<Vec<ProcessId>>,
            chain: std::sync::OnceLock<Vec<ProcessId>>,
            trace: Mutex<Vec<&'static str>>,
            started: AtomicBool,
            panic_after_wakes: bool,
            steps: bool,
            block_request: std::sync::mpsc::SyncSender<()>,
            unblock: Mutex<std::sync::mpsc::Receiver<()>>,
        }

        impl Scenario {
            fn park(&self) {
                let process = self
                    .scheduler
                    .get_process(get_current_pid().unwrap())
                    .unwrap();
                process.lock().set_live_state(ProcessState::Waiting);
                yield_current();
            }

            fn wake(&self, pid: ProcessId) {
                let process = self.scheduler.get_process(pid).unwrap();
                let proc = process.lock();
                if self.steps && self.chain.get().unwrap().contains(&pid) {
                    proc.mailbox.push(crate::actor::Message {
                        buffer: crate::actor::MessageBuffer::from_slice(
                            &[1],
                            crate::actor::PROGRAM_MESSAGE_TAG,
                        ),
                    });
                }
                self.scheduler.wake_if_waiting(pid, proc);
            }
        }

        extern "C-unwind" fn ordinary(args: *const u8) {
            let s = unsafe { &*args.cast::<Scenario>() };
            let pid = get_current_pid().unwrap();
            s.park();
            let index = s
                .ordinary
                .get()
                .unwrap()
                .iter()
                .position(|&p| p == pid)
                .unwrap();
            s.trace.lock().push(["remote", "local", "overflow"][index]);
            if index == 1 {
                s.block_request.send(()).unwrap();
                s.unblock.lock().recv().unwrap();
            }
        }

        extern "C-unwind" fn chain(args: *const u8) {
            let s = unsafe { &*args.cast::<Scenario>() };
            let pid = get_current_pid().unwrap();
            s.park();
            chain_hop(s, pid);
        }

        extern "C-unwind" fn step_hop(frame: *mut u64, payload: *const u8) -> u8 {
            let s = unsafe { &*((*frame) as *const Scenario) };
            assert_eq!(unsafe { payload.cast::<u64>().read() }, 1);
            crate::actor::mesh_step_charge();
            chain_hop(s, get_current_pid().unwrap());
            0
        }

        fn chain_hop(s: &Scenario, pid: ProcessId) {
            s.trace.lock().push("hop");
            let index = s
                .chain
                .get()
                .unwrap()
                .iter()
                .position(|&p| p == pid)
                .unwrap();
            if let Some(&next) = s.chain.get().unwrap().get(index + 1) {
                s.wake(next);
            }
        }

        extern "C-unwind" fn ready(args: *const u8) {
            let s = unsafe { &*args.cast::<Scenario>() };
            while !s.started.load(Ordering::Acquire) {
                yield_current();
            }
            let process = s.scheduler.get_process(get_current_pid().unwrap()).unwrap();
            let label = match process.lock().placement {
                Placement::Shared => "ordinary ready",
                Placement::Grouped { .. } => "grouped ready",
            };
            s.trace.lock().push(label);
        }

        extern "C-unwind" fn spawned(args: *const u8) {
            let s = unsafe { &*args.cast::<Scenario>() };
            s.trace.lock().push("spawn");
        }

        extern "C-unwind" fn sender(args: *const u8) {
            let s = unsafe { &*args.cast::<Scenario>() };
            // Handoffs are available for resumed actors. All recipients have parked.
            yield_current();
            std::thread::scope(|scope| {
                scope.spawn(|| s.wake(s.ordinary.get().unwrap()[0]));
            });
            s.wake(s.ordinary.get().unwrap()[1]);
            s.wake(s.chain.get().unwrap()[0]);
            s.wake(s.ordinary.get().unwrap()[2]);
            s.scheduler.spawn(spawned as *const u8, args, 0, 0);
            s.started.store(true, Ordering::Release);
            if s.panic_after_wakes {
                panic!("sender exited after waking its peers");
            }
        }

        fn run(panic_after_wakes: bool, steps: bool) -> Vec<&'static str> {
            let (request, blocking) = std::sync::mpsc::sync_channel(0);
            let (release, unblock) = std::sync::mpsc::channel();
            let scenario = Scenario {
                scheduler: Scheduler::new(1),
                ordinary: Default::default(),
                chain: Default::default(),
                trace: Mutex::default(),
                started: AtomicBool::new(false),
                panic_after_wakes,
                steps,
                block_request: request,
                unblock: Mutex::new(unblock),
            };
            // Actor arguments are borrowed until run joins the worker.
            let args = std::ptr::from_ref(&scenario).cast();
            scenario
                .ordinary
                .set(
                    (0..ORDINARY)
                        .map(|_| scenario.scheduler.spawn(ordinary as *const u8, args, 0, 0))
                        .collect(),
                )
                .unwrap();
            scenario.scheduler.spawn(ready as *const u8, args, 0, 0);
            let grouped = |entry: *const u8| {
                scenario
                    .scheduler
                    .spawn_grouped(entry, args, 0, std::ptr::null(), None)
            };
            scenario
                .chain
                .set(
                    (0..CHAIN)
                        .map(|_| {
                            if steps {
                                let frame = args as u64;
                                scenario.scheduler.spawn_step(
                                    step_hop as *const u8,
                                    1,
                                    std::ptr::from_ref(&frame).cast(),
                                    8,
                                    std::ptr::null(),
                                )
                            } else {
                                grouped(chain as *const u8)
                            }
                        })
                        .collect(),
                )
                .unwrap();
            grouped(ready as *const u8);
            grouped(sender as *const u8);
            std::thread::scope(|scope| {
                scope.spawn(move || {
                    blocking
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                    release.send(()).unwrap();
                });
                scenario.scheduler.signal_shutdown();
                scenario.scheduler.run();
            });
            assert_eq!(scenario.scheduler.active_count(), 0);
            scenario.trace.into_inner()
        }

        #[test]
        fn ordinary_wakes_remain_fifo_across_remote_local_and_overflow() {
            let trace = run(false, false);
            let ordinary: Vec<_> = trace
                .into_iter()
                .filter(|label| matches!(*label, "remote" | "local" | "overflow"))
                .collect();
            assert_eq!(ordinary, ["remote", "local", "overflow"]);
        }

        #[test]
        fn grouped_chain_yields_to_ready_actors_and_spawn_after_three_handoffs() {
            for (panic_after_wakes, steps) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let trace = run(panic_after_wakes, steps);
                let wake_order: Vec<_> = trace
                    .iter()
                    .copied()
                    .filter(|label| matches!(*label, "remote" | "local" | "overflow"))
                    .collect();
                assert_eq!(wake_order, ["remote", "local", "overflow"]);
                let position = |label| trace.iter().position(|&entry| entry == label).unwrap();
                assert_eq!(trace.iter().filter(|&&entry| entry == "hop").count(), CHAIN);
                assert_eq!(position("spawn"), 3, "{trace:?}");
                for label in ["ordinary ready", "grouped ready", "remote", "overflow"] {
                    let hops = trace[..position(label)]
                        .iter()
                        .filter(|&&entry| entry == "hop")
                        .count();
                    assert_eq!(hops, 3, "{label} waited for {hops} hops: {trace:?}");
                }
            }
        }

        #[test]
        fn unwind_requeues_captured_wake_and_disarms_the_slot() {
            let scheduler = Scheduler::new(2);
            let other = Scheduler::new(1);
            let wake = |sched: &Scheduler, worker| {
                let pid = ProcessId::next();
                let mut p =
                    Process::with_placement(pid, Priority::Normal, Placement::Grouped { worker });
                p.worker = Some(worker);
                p.set_live_state(ProcessState::Waiting);
                sched.wake_if_waiting(pid, Mutex::new(p).lock());
                pid
            };
            let owner = &scheduler.sleepers[0];
            let mut captured = None;
            let mut overflow = None;
            let mut foreign = None;
            let mut remote = None;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capture_handoff(owner, || {
                    foreign = Some(wake(&other, 0));
                    remote = Some(wake(&scheduler, 1));
                    captured = Some(wake(&scheduler, 0));
                    overflow = Some(wake(&scheduler, 0));
                    panic!("unwind with an occupied slot");
                });
            }));
            assert!(result.is_err());
            let after = wake(&scheduler, 0);
            assert_eq!(*other.sleepers[0].woken.lock(), [foreign.unwrap()]);
            assert_eq!(*scheduler.sleepers[1].woken.lock(), [remote.unwrap()]);
            assert_eq!(
                *owner.woken.lock(),
                [overflow.unwrap(), captured.unwrap(), after]
            );
        }
    }
}

#[cfg(test)]
mod stackless_probe {
    use super::*;
    use crate::actor::{mesh_actor_self, Message, MessageBuffer};

    extern "C-unwind" fn step(frame: *mut u64, message: *const u8) -> u8 {
        assert!(CURRENT_YIELDER.with(|tls| tls.yielder.get().is_none()));
        let frame = unsafe { std::slice::from_raw_parts_mut(frame, 2) };
        assert_eq!(mesh_actor_self(), frame[0]);
        let current = unsafe { message.cast::<u64>().read() };
        match current {
            17 => {
                super::super::mesh_step_charge();
                super::super::mesh_step_charge();
                // Only the loaded value may escape the caller-owned payload.
                assert_eq!(
                    crate::actor::current_process()
                        .unwrap()
                        .lock()
                        .heap
                        .total_bytes(),
                    0
                );
                let owned = crate::gc::mesh_gc_alloc_actor(8, 8).cast::<u64>();
                unsafe { owned.write(current) };
                frame[1] = owned as u64;
                1
            }
            23 => {
                let previous = unsafe { (frame[1] as *const u64).read() };
                assert_eq!(previous, 17);
                panic!("probe crash after resume");
            }
            _ => panic!("unexpected message"),
        }
    }

    fn push(actor: &Arc<Mutex<Process>>, value: u64, tag: u64) {
        actor.lock().mailbox.push(Message {
            buffer: MessageBuffer::from_slice(&value.to_le_bytes(), tag),
        });
    }

    #[test]
    fn kill_before_or_after_wake_finalizes_without_dequeuing() {
        struct Kill {
            scheduler: Scheduler,
            pid: ProcessId,
            wake_first: bool,
        }

        extern "C-unwind" fn wake_and_kill(args: *const u8) {
            let kill = unsafe { &*args.cast::<Kill>() };
            let target = kill.scheduler.get_process(kill.pid).unwrap();
            assert!(matches!(target.lock().state, ProcessState::Waiting));
            let wake = || {
                push(&target, 23, 0);
                kill.scheduler.wake_if_waiting(kill.pid, target.lock());
            };
            if kill.wake_first {
                wake();
            }
            crate::actor::deliver_exit_signal(&kill.scheduler, kill.pid, ExitReason::Killed);
            if !kill.wake_first {
                wake();
            }
        }

        extern "C-unwind" fn finished(_: *mut u64, _: *const u8) -> u8 {
            0
        }
        for wake_first in [true, false] {
            let scheduler = Scheduler::new(1);
            let pid = scheduler.spawn_step(
                finished as *const u8,
                2,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
            let target = scheduler.get_process(pid).unwrap();
            let kill = Kill {
                scheduler,
                pid,
                wake_first,
            };
            kill.scheduler.spawn_grouped(
                wake_and_kill as *const u8,
                std::ptr::from_ref(&kill).cast(),
                0,
                std::ptr::null(),
                None,
            );
            kill.scheduler.signal_shutdown();
            kill.scheduler.run();
            assert_eq!(kill.scheduler.active_count(), 0);
            assert!(kill.scheduler.get_process(pid).is_none());
            let proc = target.lock();
            assert!(matches!(
                proc.state,
                ProcessState::Exited(ExitReason::Killed)
            ));
            assert!(proc
                .mailbox
                .remove_first(|m| m.buffer.type_tag == 0)
                .is_some());
        }
    }

    #[test]
    fn running_kill_takes_effect_at_suspension_or_completion() {
        use crate::actor::{
            deliver_exit_signal, mesh_reduction_check, receive_scalar_payload, receive_with,
            PROGRAM_MESSAGE_TAG,
        };
        use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
        use std::time::Duration;

        #[derive(Clone, Copy, Debug)]
        enum End {
            Return,
            Panic,
            Wait,
            Budget,
        }

        struct Probe<'a> {
            scheduler: &'a Scheduler,
            entered: SyncSender<()>,
            killed: Mutex<Receiver<()>>,
            effects: Mutex<Vec<u64>>,
            end: End,
        }

        impl Probe<'_> {
            fn handle(&self, value: u64) -> u8 {
                self.effects.lock().push(value);
                if value == 1 {
                    self.entered.send(()).unwrap();
                    self.killed
                        .lock()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                }
                match (value, self.end) {
                    (2, End::Return) => 0,
                    (2, End::Panic) => panic!("after forced exit"),
                    _ => 1,
                }
            }
        }

        extern "C-unwind" fn coroutine(args: *const u8) {
            let probe = unsafe { &*args.cast::<Probe>() };
            loop {
                mesh_reduction_check();
                let mut scratch = std::mem::MaybeUninit::uninit();
                let payload = receive_with(probe.scheduler, -1, |proc| {
                    let msg = proc
                        .mailbox
                        .remove_first(|m| m.buffer.type_tag == PROGRAM_MESSAGE_TAG)?;
                    Some(receive_scalar_payload(proc, msg, 8, &mut scratch))
                });
                if payload.is_null() || probe.handle(unsafe { payload.cast::<u64>().read() }) == 0 {
                    return;
                }
            }
        }

        extern "C-unwind" fn step(frame: *mut u64, payload: *const u8) -> u8 {
            crate::actor::mesh_step_charge();
            let probe = unsafe { &*(*frame as *const Probe) };
            probe.handle(unsafe { payload.cast::<u64>().read() })
        }

        static TERMINATIONS: AtomicU64 = AtomicU64::new(0);
        static REASON: AtomicU64 = AtomicU64::new(0);
        extern "C" fn terminate(_: *const u8, reason: *const u8) {
            REASON.store(unsafe { *reason }.into(), Ordering::SeqCst);
            TERMINATIONS.fetch_add(1, Ordering::SeqCst);
        }

        for stackless in [false, true] {
            for linked in [false, true] {
                for end in [End::Return, End::Panic, End::Wait, End::Budget] {
                    let scheduler = Scheduler::new(1);
                    let pid = scheduler.create_main_process();
                    let target = scheduler.get_process(pid).unwrap();
                    {
                        let mut process = target.lock();
                        process.placement = Placement::Grouped { worker: 0 };
                        process.terminate_callback = Some(terminate);
                    }
                    let sender = scheduler.create_main_process();
                    let reason = if linked {
                        link::link(
                            &target,
                            &scheduler.get_process(sender).unwrap(),
                            pid,
                            sender,
                        );
                        ExitReason::Linked(sender, Box::new(ExitReason::Killed))
                    } else {
                        ExitReason::Killed
                    };

                    let (entered, running) = sync_channel(1);
                    let (release, killed) = sync_channel(1);
                    let probe = Probe {
                        scheduler: &scheduler,
                        entered,
                        killed: Mutex::new(killed),
                        effects: Mutex::default(),
                        end,
                    };
                    let args = std::ptr::from_ref(&probe);
                    let handle = if stackless {
                        Execution::Step(Box::new(BoundedStep {
                            deferred: false,
                            call: step,
                            bound: 1,
                            frame: vec![args as u64].into(),
                        }))
                    } else {
                        Execution::Stack(CoroutineHandle::new(coroutine as *const u8, args.cast()))
                    };
                    let mut actor = ScheduledActor {
                        process: target.clone(),
                        handle,
                    };
                    push(&target, 1, 0);
                    push(&target, 2, 0);
                    if matches!(end, End::Budget) {
                        push(&target, 3, 0);
                    }
                    let count = AtomicU64::new(1);
                    CURRENT_YIELDER.with(|tls| {
                        tls.reductions.set(if matches!(end, End::Budget) {
                            2
                        } else {
                            DEFAULT_REDUCTIONS
                        })
                    });
                    let before = TERMINATIONS.load(Ordering::SeqCst);
                    std::thread::scope(|scope| {
                        let scheduler = &scheduler;
                        scope.spawn(move || {
                            running.recv_timeout(Duration::from_secs(10)).unwrap();
                            if linked {
                                assert!(handle_process_exit(
                                    scheduler.process_table(),
                                    &scheduler.sleepers,
                                    sender,
                                    ExitReason::Killed
                                ));
                            } else {
                                deliver_exit_signal(scheduler, pid, ExitReason::Killed);
                            }
                            // A later signal must preserve the first reason.
                            deliver_exit_signal(scheduler, pid, ExitReason::Shutdown);
                            release.send(()).unwrap();
                        });
                        let outcome = resume_process(
                            scheduler.process_table(),
                            &count,
                            &scheduler.sleepers,
                            pid,
                            &mut actor,
                            None,
                            &AtomicBool::new(false),
                        )
                        .0;
                        assert!(matches!(outcome, Resumed::Finished));
                    });
                    assert_eq!(
                        *probe.effects.lock(),
                        [1, 2],
                        "stackless={stackless} linked={linked} end={end:?}"
                    );
                    assert_eq!(
                        target.lock().mailbox.len(),
                        usize::from(matches!(end, End::Budget))
                    );
                    assert_eq!(TERMINATIONS.load(Ordering::SeqCst), before + 1);
                    assert_eq!(REASON.load(Ordering::SeqCst), u64::from(reason.tag()));
                    assert_eq!(target.lock().state, ProcessState::Exited(reason));
                    assert!(scheduler.get_process(pid).is_none());
                    assert_eq!(count.load(Ordering::SeqCst), 0);
                }
            }
        }
    }

    #[test]
    fn continuation_survives_gc_wait_and_linked_exit() {
        let table = ProcessTable::new();
        let pid = ProcessId::next();
        let neighbor = ProcessId::next();
        let neighbor_state = Arc::new(Mutex::new(Process::new(neighbor, Priority::Normal)));
        table.insert(neighbor, neighbor_state.clone());
        let process = Arc::new(Mutex::new(Process::with_placement(
            pid,
            Priority::Normal,
            Placement::Grouped { worker: 0 },
        )));
        table.insert(pid, process.clone());
        super::link::link(&process, &neighbor_state, pid, neighbor);
        let mut actor = ScheduledActor {
            process: process.clone(),
            handle: Execution::Step(Box::new(BoundedStep {
                deferred: false,
                call: step,
                bound: 2,
                frame: vec![pid.as_u64(), 0].into(),
            })),
        };
        push(&process, 0, 99);
        push(&process, 17, 0);
        push(&process, 23, 0);
        let count = AtomicU64::new(2);
        CURRENT_YIELDER.with(|slot| slot.reductions.set(1));
        assert!(matches!(
            resume_process(
                &table,
                &count,
                &[],
                pid,
                &mut actor,
                None,
                &AtomicBool::new(false)
            )
            .0,
            Resumed::Runnable
        ));
        let Execution::Step(step) = &actor.handle else {
            unreachable!()
        };
        assert_eq!(step.frame[1], 0, "insufficient budget executed an effect");
        assert_eq!(
            CURRENT_YIELDER.with(|slot| slot.reductions.get()),
            DEFAULT_REDUCTIONS
        );
        // An intervening actor spent the entire worker refill.
        CURRENT_YIELDER.with(|slot| slot.reductions.set(0));
        assert!(matches!(
            resume_process(
                &table,
                &count,
                &[],
                pid,
                &mut actor,
                None,
                &AtomicBool::new(false)
            )
            .0,
            Resumed::Runnable
        ));
        assert!(get_current_pid().is_none());
        let Execution::Step(step) = &actor.handle else {
            unreachable!()
        };
        let saved = step.frame[1] as *const u8;
        // Two messages were ready. The forced reduction yield left the second queued.
        let pending = process
            .lock()
            .mailbox
            .remove_first(|m| m.buffer.type_tag == 0)
            .unwrap();
        let garbage = {
            let mut proc = process.lock();
            let garbage = proc.heap.alloc(512, 8);
            while !proc.heap.should_collect() {
                proc.heap.alloc(512, 8);
            }
            garbage
        };
        assert!(matches!(
            resume_process(
                &table,
                &count,
                &[],
                pid,
                &mut actor,
                None,
                &AtomicBool::new(false)
            )
            .0,
            Resumed::Waiting
        ));
        {
            let proc = process.lock();
            assert!(proc.heap.is_live_allocation(saved, 8));
            assert!(!proc.heap.is_live_allocation(garbage, 512));
            assert!(proc
                .mailbox
                .remove_first(|m| m.buffer.type_tag == 99)
                .is_some());
        }
        {
            let mut proc = process.lock();
            proc.mailbox.push(pending);
            proc.set_live_state(ProcessState::Ready);
        }
        assert!(matches!(
            resume_process(
                &table,
                &count,
                &[],
                pid,
                &mut actor,
                None,
                &AtomicBool::new(false)
            )
            .0,
            Resumed::Finished
        ));
        assert!(get_current_pid().is_none());
        assert!(table.get(pid).is_none());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(matches!(&neighbor_state.lock().state,
            ProcessState::Exited(ExitReason::Linked(from, reason))
                if *from == pid && matches!(reason.as_ref(), ExitReason::Error(msg) if msg == "probe crash after resume")));
    }
}
