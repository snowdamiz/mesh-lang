---
phase: "06-actor-runtime"
plan: "07"
subsystem: "testing"
tags: ["e2e", "integration", "actors", "benchmark", "100k", "preemption", "linking", "typed-pid", "terminate"]

dependency-graph:
  requires: ["06-05", "06-06"]
  provides: ["e2e-actor-test-suite", "100k-actor-benchmark", "phase-6-verification"]
  affects: ["07-01", "07-02", "07-03"]

tech-stack:
  added: []
  patterns: ["e2e-test-harness-for-actors", "compiled-binary-output-assertion"]

key-files:
  created:
    - "crates/snowc/tests/e2e_actors.rs"
    - "tests/e2e/actors_basic.snow"
    - "tests/e2e/actors_messaging.snow"
    - "tests/e2e/actors_preemption.snow"
    - "tests/e2e/actors_linking.snow"
    - "tests/e2e/actors_typed_pid.snow"
    - "tests/e2e/actors_100k.snow"
    - "tests/e2e/actors_terminate.snow"
  modified:
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snow-codegen/src/codegen/mod.rs"
    - "crates/snow-rt/src/actor/scheduler.rs"
    - "crates/snow-codegen/src/lower.rs"
    - "crates/snow-codegen/src/intrinsics.rs"
    - "crates/snow-rt/src/gc.rs"
    - "crates/snow-rt/src/lib.rs"

decisions:
  - id: "SCHEDULER_INTERIOR_MUTABILITY"
    decision: "Scheduler run loop uses interior mutability to avoid holding lock during coroutine resume"
    reason: "Holding MutexGuard across coroutine suspend/resume caused deadlock"
  - id: "SNOW_MAIN_SYMBOL"
    decision: "Compiler emits snow_main as user entry point, runtime provides C main"
    reason: "Avoids LLVM main symbol collision with runtime's main()"
  - id: "ACTOR_FN_RETURNS_UNIT"
    decision: "Actor spawn functions return Unit (not Pid) at MIR/codegen level"
    reason: "Runtime allocates Pid; the spawned function body returns nothing"
  - id: "REDUCTION_CHECK_INSIDE_COROUTINE"
    decision: "Reduction check only yields when inside a coroutine context"
    reason: "Yielding outside coroutine (e.g., in main before scheduler starts) causes panic"

metrics:
  duration: "21min"
  completed: "2026-02-07"
---

# Phase 6 Plan 7: E2E Integration Tests and 100K Actor Benchmark Summary

**7 E2E test programs covering all actor features (spawn, messaging, preemption, linking, typed Pid, terminate callbacks) plus 100K actor benchmark completing in ~2.78s -- with 6 integration bugs fixed during bring-up**

## Performance

- **Duration:** 21 min
- **Started:** 2026-02-07T02:27:00Z
- **Completed:** 2026-02-07T02:48:00Z
- **Tasks:** 2 (1 auto + 1 checkpoint, both complete)
- **Files created:** 8
- **Files modified:** 7

## Accomplishments

- Complete E2E actor test suite exercising all 6 Phase 6 success criteria
- 100K actor benchmark spawns, messages, and collects responses in ~2.78s without OOM
- Typed Pid<Int> correctly rejects String sends at compile time (E0014)
- Preemptive scheduling verified: infinite-loop actor does not starve others
- Process linking delivers exit signals on crash
- Terminate callbacks run cleanup logic before actor exit
- 6 integration bugs discovered and fixed during bring-up (all auto-fixed, deviation rules 1/3)
- All 611 workspace tests pass with zero regressions

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | E2E actor test programs and integration test harness | dd5ff9c | e2e_actors.rs, 7 .snow test programs, 6 integration bug fixes |
| 2 | Checkpoint: human-verify | APPROVED | User verified all 7 E2E tests pass, 611 workspace tests pass, 100K benchmark ~2.78s |

## Files Created

- `crates/snowc/tests/e2e_actors.rs` -- E2E test harness: compiles .snow programs via snowc build, runs binaries, asserts stdout
- `tests/e2e/actors_basic.snow` -- SC1: Counter actor with increment/get messaging, expects "42"
- `tests/e2e/actors_messaging.snow` -- SC4: Greeter with pattern-matched receive arms, rename support
- `tests/e2e/actors_preemption.snow` -- SC3: Infinite Spinner does not block Reporter from responding
- `tests/e2e/actors_linking.snow` -- SC5: Linked actor crash delivers exit signal to partner
- `tests/e2e/actors_typed_pid.snow` -- SC2: Typed Pid<Int> send validation (passes + error case)
- `tests/e2e/actors_100k.snow` -- SC1: 100K actor spawn/message/collect benchmark
- `tests/e2e/actors_terminate.snow` -- SC6: Terminate callback runs cleanup before actor exit

## Files Modified (Integration Bug Fixes)

- `crates/snow-codegen/src/codegen/expr.rs` -- Fixed codegen_actor_receive message extraction
- `crates/snow-codegen/src/codegen/mod.rs` -- Fixed LLVM main symbol collision (main -> snow_main)
- `crates/snow-rt/src/actor/scheduler.rs` -- Fixed scheduler deadlock (interior mutability), added snow_rt_run_scheduler
- `crates/snow-codegen/src/lower.rs` -- Fixed actor function return type (Pid -> Unit)
- `crates/snow-codegen/src/intrinsics.rs` -- Added missing runtime function declarations
- `crates/snow-rt/src/gc.rs` -- Fixed reduction check outside coroutine guard
- `crates/snow-rt/src/lib.rs` -- Re-exported new runtime entry points

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Scheduler deadlock from holding MutexGuard across coroutine resume**
- **Found during:** Task 1 (actors_basic.snow integration)
- **Issue:** Scheduler held a MutexGuard on the process table while resuming a coroutine. When the coroutine yielded back, it tried to re-acquire the same lock, causing deadlock.
- **Fix:** Restructured scheduler run loop to use interior mutability -- extract process data under lock, drop lock, then resume coroutine.
- **Files modified:** crates/snow-rt/src/actor/scheduler.rs
- **Verification:** actors_basic.snow runs to completion without hanging
- **Committed in:** dd5ff9c

**2. [Rule 3 - Blocking] Missing scheduler run loop (snow_rt_run_scheduler)**
- **Found during:** Task 1 (actors_basic.snow integration)
- **Issue:** Compiled Snow programs had no way to start the scheduler event loop. The runtime had individual functions but no top-level run entry point.
- **Fix:** Added `snow_rt_run_scheduler` extern "C" function that runs the M:N scheduler loop until all actors complete.
- **Files modified:** crates/snow-rt/src/actor/scheduler.rs, crates/snow-rt/src/lib.rs
- **Verification:** Compiled binaries start scheduler and run actors
- **Committed in:** dd5ff9c

**3. [Rule 1 - Bug] Actor function return type: Pid -> Unit**
- **Found during:** Task 1 (actors_basic.snow integration)
- **Issue:** MIR lowering expected actor spawn functions to return Pid, but the runtime allocates and returns the Pid externally. The actor function body should return Unit.
- **Fix:** Changed actor function lowering to produce Unit return type.
- **Files modified:** crates/snow-codegen/src/lower.rs
- **Verification:** Type mismatch error resolved, actors spawn correctly
- **Committed in:** dd5ff9c

**4. [Rule 1 - Bug] LLVM main symbol collision (main -> snow_main)**
- **Found during:** Task 1 (actors_basic.snow integration)
- **Issue:** Compiler emitted a `main` symbol for the user entry point, colliding with the C runtime's main provided by snow-rt. Linker error: duplicate symbol.
- **Fix:** Compiler now emits `snow_main` as the user entry point. Runtime's C main calls snow_main.
- **Files modified:** crates/snow-codegen/src/codegen/mod.rs
- **Verification:** No more linker duplicate symbol errors
- **Committed in:** dd5ff9c

**5. [Rule 1 - Bug] codegen_actor_receive message extraction incorrect**
- **Found during:** Task 1 (actors_messaging.snow integration)
- **Issue:** LLVM codegen for receive blocks was not correctly extracting the message payload from the mailbox message buffer. Pattern matching on received messages failed.
- **Fix:** Fixed message extraction in codegen_actor_receive to properly decode the message buffer layout (type_tag + data_len + data).
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** actors_messaging.snow correctly dispatches to matching arms
- **Committed in:** dd5ff9c

**6. [Rule 1 - Bug] Reduction check yielding outside coroutine context**
- **Found during:** Task 1 (actors_preemption.snow integration)
- **Issue:** Reduction check instrumentation inserted yield points that could fire before the scheduler had started (e.g., during main function initialization), causing a panic because there was no coroutine context to yield from.
- **Fix:** Added guard in reduction check: only yield when inside a coroutine context (actor), skip when running in bare main thread.
- **Files modified:** crates/snow-rt/src/gc.rs
- **Verification:** actors_preemption.snow runs without panic, preemption works correctly
- **Committed in:** dd5ff9c

---

**Total deviations:** 6 auto-fixed (5 Rule 1 bugs, 1 Rule 3 blocking)
**Impact on plan:** All fixes were necessary for correct actor runtime integration. No scope creep. These are the expected integration issues when connecting independently-developed components (runtime, compiler frontend, type checker, MIR lowering, LLVM codegen) for the first time.

## Issues Encountered

Integration testing revealed 6 issues that were all resolved during Task 1. This is expected and healthy -- the purpose of plan 06-07 was specifically to discover and fix these integration issues. All fixes were committed atomically in the single task commit.

## Phase 6 Success Criteria Verification

| SC | Criterion | Test Program | Result |
|----|-----------|-------------|--------|
| SC1 | 100K actors hold state and respond | actors_100k.snow | PASS -- 100K actors in ~2.78s |
| SC2 | Wrong-type send rejected at compile time | actors_typed_pid.snow | PASS -- E0014 error |
| SC3 | Infinite actor does not block others | actors_preemption.snow | PASS -- Reporter responds |
| SC4 | Receive pattern matching dispatches correctly | actors_messaging.snow | PASS -- correct arm selected |
| SC5 | Linked actor crash delivers exit signal | actors_linking.snow | PASS -- exit signal received |
| SC6 | Terminate callback runs before exit | actors_terminate.snow | PASS -- cleanup message sent |

All 6 success criteria verified by automated E2E tests and human verification.

## User Setup Required

None -- no external service configuration required.

## Next Phase Readiness

Phase 6 (Actor Runtime) is **complete**. All 7 plans executed successfully:
- 06-01: M:N work-stealing scheduler with coroutines
- 06-02: Compiler frontend (actor/spawn/send/receive syntax)
- 06-03: Per-actor heaps and message passing
- 06-04: Typed Pid<M> with compile-time send validation
- 06-05: AST-to-MIR lowering and LLVM codegen for actors
- 06-06: Process linking, exit propagation, named registry, terminate callbacks
- 06-07: E2E integration tests and 100K actor benchmark

Phase 7 (Supervision & Fault Tolerance) can proceed. It will build on:
- Process linking and exit signal propagation (06-06)
- trap_exit flag on processes (06-06, unused but ready)
- Named process registry (06-06)
- Terminate callback infrastructure (06-05, 06-06)
- Complete E2E test patterns (06-07)

No blockers identified.

## Self-Check: PASSED

---
*Phase: 06-actor-runtime*
*Completed: 2026-02-07*
