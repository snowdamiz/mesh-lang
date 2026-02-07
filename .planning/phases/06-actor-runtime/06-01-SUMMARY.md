---
phase: 06-actor-runtime
plan: 01
subsystem: runtime
tags: [actor, scheduler, coroutine, corosensei, crossbeam, work-stealing, pcb]

dependency_graph:
  requires: []
  provides:
    - "Process Control Block (PCB) with pid, state, priority, reductions, links, mailbox, terminate_callback"
    - "M:N work-stealing scheduler using crossbeam-deque"
    - "Corosensei-based stackful coroutines with 64KB stacks"
    - "extern C ABI: snow_rt_init_actor, snow_actor_spawn, snow_actor_self, snow_reduction_check"
  affects:
    - "06-02 (parser needs actor spawn/send/receive -- already done in parallel)"
    - "06-03 (message passing builds on mailbox placeholder)"
    - "06-04 (process registry builds on process table)"
    - "06-05 (supervision builds on exit reason propagation)"
    - "06-06 (linking builds on Process.links)"

tech_stack:
  added:
    - "crossbeam-deque 0.8 (lock-free work-stealing deques)"
    - "crossbeam-utils 0.8 (scoped threads)"
    - "crossbeam-channel 0.5 (high-priority MPMC channel)"
    - "corosensei 0.3 (stackful coroutines)"
    - "parking_lot 0.12 (fast mutexes/rwlocks)"
  patterns:
    - "M:N threading with work-stealing"
    - "SpawnRequest-based distribution (coroutines are !Send, stay thread-pinned)"
    - "Thread-local context (CURRENT_YIELDER, CURRENT_PID) for actor identity"
    - "Shadow reduction counter in thread-local for lock-free decrement"
    - "Exponential backoff on idle workers"

key_files:
  created:
    - "crates/snow-rt/src/actor/process.rs"
    - "crates/snow-rt/src/actor/scheduler.rs"
    - "crates/snow-rt/src/actor/stack.rs"
  modified:
    - "crates/snow-rt/Cargo.toml"
    - "crates/snow-rt/src/actor/mod.rs"
    - "crates/snow-rt/src/lib.rs"
    - "Cargo.lock"

decisions:
  - id: "06-01-D1"
    decision: "Coroutines are !Send -- work-stealing operates on SpawnRequest (Send) not Coroutine"
    context: "corosensei::Coroutine is !Send, cannot move between threads"
    outcome: "Workers steal spawn requests from global/other deques; once created, coroutines stay thread-pinned; yielded coroutines go to worker's local suspended list"
  - id: "06-01-D2"
    decision: "Thread-local shadow reduction counter avoids locking on every reduction check"
    context: "snow_reduction_check is called at every loop back-edge and function call -- must be fast"
    outcome: "Decrement a Cell<u32> thread-local; yield only when it hits 0; scheduler resets Process.reductions after yield"
  - id: "06-01-D3"
    decision: "Yielder re-installation after suspend to handle interleaved coroutines"
    context: "When coroutine A yields, coroutine B runs and overwrites CURRENT_YIELDER thread-local; when A resumes, yielder is gone"
    outcome: "yield_current() re-sets the thread-local after suspend() returns; scheduler clears CURRENT_YIELDER after each resume returns"

metrics:
  duration: "12min"
  completed: "2026-02-06"
  tests_added: 12
  tests_total: 27
---

# Phase 6 Plan 1: Actor Runtime Core Summary

**M:N work-stealing scheduler with corosensei coroutines, Process Control Block, and extern C ABI for actor spawn/yield/identity**

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Add actor runtime dependencies and create Process Control Block | `2e4a7f8` | Cargo.toml, Cargo.lock, actor/process.rs, actor/mod.rs |
| 2 | Implement M:N work-stealing scheduler with corosensei coroutines | `dde84c0` | actor/scheduler.rs, actor/stack.rs, actor/mod.rs, lib.rs |

## What Was Built

### Process Control Block (`process.rs`)
- **ProcessId**: Atomic u64 counter generating globally unique PIDs across threads
- **ProcessState**: Ready, Running, Waiting, Exited(ExitReason) -- full lifecycle
- **ExitReason**: Normal, Error(String), Killed, Linked(ProcessId, Box<ExitReason>)
- **Priority**: High, Normal, Low with u8 ABI conversion (0/1/2)
- **Process struct**: pid, state, priority, reductions (4000 default), links, mailbox, terminate_callback
- **TerminateCallback**: `extern "C" fn(state_ptr: *const u8, reason_ptr: *const u8)` type alias
- **Message**: Placeholder struct (`data: Vec<u8>, type_tag: u64`) for Plan 03

### Stack Management (`stack.rs`)
- **CoroutineHandle**: Wraps `corosensei::Coroutine` with 64KB `DefaultStack`
- **Thread-locals**: `CURRENT_YIELDER` (pointer to active Yielder) and `CURRENT_PID` (current actor's ProcessId)
- **yield_current()**: Calls `yielder.suspend()` with re-installation of yielder pointer after resume to handle interleaved coroutine execution
- Key insight: `Coroutine` is `!Send`, so coroutines stay on the thread that created them

### M:N Scheduler (`scheduler.rs`)
- **SpawnRequest**: Send-able struct with fn_ptr, args, pid, priority -- distributed via work-stealing
- **Worker loop**: 3-phase cycle per iteration:
  1. Resume suspended coroutines (local, thread-pinned)
  2. Pick up new SpawnRequests (high-priority channel -> local deque -> global injector -> steal from peers)
  3. Check shutdown condition
- **Crossbeam integration**: `Injector` for global queue, `Worker`/`Stealer` pairs for per-thread deques
- **Priority**: High-priority requests go through dedicated `crossbeam_channel`, checked first
- **Shutdown**: AtomicBool flag + active_count tracking; workers exit when both conditions met
- **Backoff**: Spin-loop -> 100us sleep -> 1ms sleep progression on idle

### extern "C" ABI (`mod.rs`)
- `snow_rt_init_actor(num_schedulers: u32)`: Initialize global scheduler (OnceLock)
- `snow_actor_spawn(fn_ptr, args, args_size, priority) -> u64`: Spawn actor, returns PID
- `snow_actor_self() -> u64`: Read thread-local CURRENT_PID
- `snow_reduction_check()`: Thread-local shadow counter, yield via yield_current() when exhausted

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Coroutine `!Send` required SpawnRequest-based distribution**
- **Found during:** Task 2 design
- **Issue:** `corosensei::Coroutine` does not implement `Send`, cannot be placed in crossbeam-deque
- **Fix:** Work-stealing operates on `SpawnRequest` (which is `Send`); coroutines are created locally on each worker thread and stay thread-pinned
- **Impact:** Clean separation of concerns -- distribution happens at task level, not continuation level

**2. [Rule 1 - Bug] CURRENT_YIELDER overwritten by interleaved coroutines**
- **Found during:** Task 2 testing
- **Issue:** When coroutine A yields and coroutine B runs to completion on the same thread, B's cleanup cleared the CURRENT_YIELDER thread-local, causing A to panic on resume
- **Fix:** `yield_current()` re-installs yielder pointer after `suspend()` returns; scheduler clears CURRENT_YIELDER after each resume; removed clearing from coroutine body
- **Files modified:** stack.rs, scheduler.rs
- **Commit:** `dde84c0`

**3. [Rule 1 - Bug] Test counter interference from parallel test execution**
- **Found during:** Task 2 testing
- **Issue:** Static `AtomicU64` counters shared across concurrent test threads caused exact assertions to fail
- **Fix:** Each test uses a dedicated static counter (defined inside the test function) or uses delta-based assertions with `>=`
- **Files modified:** stack.rs, scheduler.rs
- **Commit:** `dde84c0`

## Tests

| Module | Test | Verifies |
|--------|------|----------|
| process | test_pid_unique | 100 sequential PIDs are all distinct |
| process | test_pid_concurrent_unique | 800 PIDs from 8 threads are all distinct |
| process | test_process_new | PCB fields have correct defaults |
| process | test_priority_from_u8 | Priority::from_u8 ABI conversion |
| process | test_process_debug | Debug formatting works |
| stack | test_coroutine_runs_to_completion | Simple entry function completes in one resume |
| stack | test_coroutine_yield_and_resume | yield_current() suspends, resume continues |
| stack | test_current_pid_thread_local | set/get/clear PID thread-local |
| scheduler | test_spawn_unique_pids | 10 spawned actors get distinct PIDs |
| scheduler | test_single_actor_completes | One actor runs to completion |
| scheduler | test_multiple_actors_complete | 10 actors all complete |
| scheduler | test_work_stealing_distributes | 100 actors across 4 threads use multiple OS threads |
| scheduler | test_reduction_yield | Yielding actor still completes |
| scheduler | test_reduction_yield_does_not_starve | Yielding actor does not starve simple actors |
| scheduler | test_high_priority | All priority levels (High/Normal/Low) complete |
| scheduler | test_100_actors_no_hang | 100 actors complete without hanging |

## Verification Results

- `cargo build -p snow-rt`: Clean build, no warnings
- `cargo test -p snow-rt`: 27/27 passed (12 new + 15 existing)
- `cargo test -p snow-rt -- --test-threads=1`: 27/27 passed (thread-safe)
- `cargo test --workspace`: All workspace tests pass, zero regressions
- 100+ actors spawn and complete without hanging or crashing

## Next Phase Readiness

Plan 06-01 delivers the scheduler foundation. Subsequent plans build on it:
- **Plan 03** (Message Passing): Will flesh out `Message` struct and `Process.mailbox` with deep-copy semantics
- **Plan 04** (Process Registry): Will use `ProcessTable` for named process lookup
- **Plan 05** (Supervision): Will use `ExitReason`, `Process.links`, and `TerminateCallback`
- **Plan 06** (Linking): Will populate `Process.links` for bidirectional link/monitor

## Self-Check: PASSED
