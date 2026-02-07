---
phase: 09-concurrency-standard-library
plan: 05
subsystem: testing
tags: [e2e, service, job, actor, concurrency, codegen, runtime]

# Dependency graph
requires:
  - phase: 09-03
    provides: Service MIR lowering and LLVM codegen
  - phase: 09-04
    provides: Job runtime functions and codegen wiring
provides:
  - E2E integration tests proving Service and Job work end-to-end
  - Main thread process support for service calls from non-coroutine context
  - Eager scheduler start for concurrent actor execution during snow_main
  - Graceful service actor shutdown via wake-and-null pattern
affects: [10-polish-release]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Main thread process entry for non-coroutine service call support"
    - "Spin-wait path in snow_actor_receive for main thread"
    - "Eager scheduler start during snow_rt_init_actor"
    - "Graceful shutdown via waking Waiting actors then returning null from receive"

key-files:
  created:
    - crates/snowc/tests/e2e_concurrency_stdlib.rs
    - tests/e2e/service_counter.snow
    - tests/e2e/service_call_cast.snow
    - tests/e2e/service_state_management.snow
    - tests/e2e/job_async_await.snow
  modified:
    - crates/snow-codegen/src/mir/mono.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-rt/src/actor/mod.rs
    - crates/snow-rt/src/actor/scheduler.rs
    - crates/snow-rt/src/actor/service.rs

key-decisions:
  - "Main thread gets PID and mailbox via create_main_process in snow_rt_init_actor for service call support"
  - "Scheduler workers start eagerly (during init, not after snow_main returns) so actors execute concurrently"
  - "snow_actor_receive detects coroutine context via CURRENT_YIELDER and uses spin-wait for main thread"
  - "Service actor shutdown via waking Waiting actors and returning null from receive (not force-drop which panics through extern C)"
  - "Reduction check uses CURRENT_YIELDER instead of CURRENT_PID to detect coroutine context"

patterns-established:
  - "E2E test pattern: compile_and_run_with_timeout with 10s timeout for service/job tests"
  - "Main thread spin-wait pattern for blocking operations from non-coroutine context"

# Metrics
duration: 13min
completed: 2026-02-07
---

# Phase 9 Plan 5: E2E Concurrency Integration Tests Summary

**Service and Job E2E tests with 10 auto-fixed compiler/runtime bugs enabling full service call/cast/state-management and async job execution**

## Performance

- **Duration:** 13 min
- **Started:** 2026-02-07T09:12:47Z
- **Completed:** 2026-02-07T09:26:13Z
- **Tasks:** 2
- **Files modified:** 13

## Accomplishments
- All 3 Service E2E tests pass: counter (start/get_count/increment/reset), call_cast (get/set/clear), state_management (accumulator)
- Job.async/Job.await E2E test passes: spawns async work, collects Ok(42)
- Fixed 10 compiler/runtime bugs discovered during E2E testing (monomorphization, type checker, tuple lowering, spawn args, scheduler)
- Zero regressions across all existing test suites (56 E2E + 730+ unit tests)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create Service E2E tests** - `d142340` (feat)
2. **Task 2: Create Job E2E tests** - `d40d312` (feat)

## Files Created/Modified
- `crates/snowc/tests/e2e_concurrency_stdlib.rs` - E2E test harness with compile_and_run_with_timeout, read_fixture, find_snowc helpers
- `tests/e2e/service_counter.snow` - Counter service with start, GetCount, Increment, Reset
- `tests/e2e/service_call_cast.snow` - Store service with Get, Set, Clear operations
- `tests/e2e/service_state_management.snow` - Accumulator service proving state persistence (1+2+3=6)
- `tests/e2e/job_async_await.snow` - Job.async spawns work, Job.await collects Result
- `crates/snow-codegen/src/mir/mono.rs` - Fixed monomorphization to traverse service_dispatch table
- `crates/snow-codegen/src/mir/lower.rs` - Fixed tuple lowering to heap allocation, handler return types, service loop params
- `crates/snow-codegen/src/codegen/expr.rs` - Added codegen_make_tuple, spawn args heap alloc, service loop null check
- `crates/snow-codegen/src/codegen/mod.rs` - Main wrapper unchanged (snow_main synchronous)
- `crates/snow-typeck/src/infer.rs` - Fixed type recording for service init/handler params
- `crates/snow-rt/src/actor/mod.rs` - Main thread process, eager scheduler start, spin-wait receive, reduction_check fix
- `crates/snow-rt/src/actor/scheduler.rs` - Added start()/wait() methods, create_main_process, graceful shutdown
- `crates/snow-rt/src/actor/service.rs` - Spin-wait path for main thread service calls

## Decisions Made
- Main thread gets PID and mailbox via create_main_process so service calls work without coroutine context
- Scheduler workers start eagerly during init (not after snow_main) so spawned actors execute concurrently
- Coroutine context detected via CURRENT_YIELDER (not CURRENT_PID, which main thread now also has)
- Graceful shutdown wakes Waiting actors, lets them detect null from receive and exit cleanly (avoids corosensei panic on force-drop through extern C)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Monomorphization removes service handler functions**
- **Found during:** Task 1 (service_counter.snow compilation)
- **Issue:** Reachability pass only traversed MIR expressions, not service_dispatch table. Handler functions removed as unreachable.
- **Fix:** Added service_dispatch table traversal to worklist in mono.rs
- **Files modified:** crates/snow-codegen/src/mir/mono.rs
- **Verification:** Handler functions present in compiled binary
- **Committed in:** d142340 (Task 1 commit)

**2. [Rule 1 - Bug] Type checker not recording service param types**
- **Found during:** Task 1 (LLVM type mismatch on init function)
- **Issue:** infer_service_def didn't insert param types into the types map, causing init to return {} instead of i64
- **Fix:** Added types.insert() for init params, call handler params, cast handler params, and init function type
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** Service functions have correct LLVM types
- **Committed in:** d142340 (Task 1 commit)

**3. [Rule 1 - Bug] Tuple lowering produces Block not heap tuple**
- **Found during:** Task 1 (SIGSEGV on handler return)
- **Issue:** lower_tuple_expr created MirExpr::Block(elements) which evaluates to last element, not a tuple
- **Fix:** Changed to __snow_make_tuple synthetic intrinsic that allocates GC heap tuple {len, elements...}
- **Files modified:** crates/snow-codegen/src/mir/lower.rs, crates/snow-codegen/src/codegen/expr.rs
- **Verification:** Handler returns proper tuple pointer
- **Committed in:** d142340 (Task 1 commit)

**4. [Rule 1 - Bug] Handler return type mismatch (i64 vs ptr)**
- **Found during:** Task 1 (LLVM verifier error)
- **Issue:** Handler declared i64 return but body returns ptr (tuple pointer)
- **Fix:** Detect body type and set handler return_type accordingly
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Verification:** No LLVM type mismatch
- **Committed in:** d142340 (Task 1 commit)

**5. [Rule 1 - Bug] Spawn args use-after-free**
- **Found during:** Task 1 (SIGSEGV - printing pointer values)
- **Issue:** Args allocated on stack (alloca) but actor runs async after caller returns
- **Fix:** Allocate spawn args on GC heap via snow_gc_alloc
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** Correct values printed
- **Committed in:** d142340 (Task 1 commit)

**6. [Rule 1 - Bug] Service loop receives pointer as i64**
- **Found during:** Task 1 (SIGSEGV on args dereference)
- **Issue:** Loop function param was MirType::Int but receives a pointer to args buffer
- **Fix:** Changed to MirType::Ptr with proper dereferencing in codegen
- **Files modified:** crates/snow-codegen/src/mir/lower.rs, crates/snow-codegen/src/codegen/expr.rs
- **Verification:** Initial state correctly read
- **Committed in:** d142340 (Task 1 commit)

**7. [Rule 1 - Bug] Program hangs - scheduler not running during snow_main**
- **Found during:** Task 1 (deadlock - main thread waits for reply but service actor not started)
- **Issue:** Scheduler workers only started by run() which is called AFTER snow_main returns
- **Fix:** Added start()/wait() methods to Scheduler, start workers eagerly in snow_rt_init_actor
- **Files modified:** crates/snow-rt/src/actor/scheduler.rs, crates/snow-rt/src/actor/mod.rs
- **Verification:** Service calls complete successfully
- **Committed in:** d142340 (Task 1 commit)

**8. [Rule 2 - Missing Critical] Main thread has no PID for service calls**
- **Found during:** Task 1 (snow_service_call gets null PID)
- **Issue:** snow_service_call needs caller PID to receive reply, but main thread had no PID
- **Fix:** Create main thread process entry in snow_rt_init_actor with PID and mailbox
- **Files modified:** crates/snow-rt/src/actor/mod.rs, crates/snow-rt/src/actor/scheduler.rs
- **Verification:** Service calls work from main thread
- **Committed in:** d142340 (Task 1 commit)

**9. [Rule 2 - Missing Critical] snow_actor_receive panics on main thread**
- **Found during:** Task 1 (yield_current called outside coroutine)
- **Issue:** snow_actor_receive calls yield_current which requires coroutine context
- **Fix:** Added spin-wait path when CURRENT_YIELDER is not set (main thread)
- **Files modified:** crates/snow-rt/src/actor/mod.rs, crates/snow-rt/src/actor/service.rs
- **Verification:** Job.await and service calls work from main thread
- **Committed in:** d142340 (Task 1 commit)

**10. [Rule 1 - Bug] Reduction check panics on main thread (regression)**
- **Found during:** Task 1 (actors_100k test fails after main thread PID addition)
- **Issue:** snow_reduction_check checked CURRENT_PID to skip non-actor context, but main thread now has PID
- **Fix:** Changed to check CURRENT_YIELDER instead (only set inside coroutines)
- **Files modified:** crates/snow-rt/src/actor/mod.rs
- **Verification:** actors_100k test passes again
- **Committed in:** d142340 (Task 1 commit)

---

**Total deviations:** 10 auto-fixed (8 bugs, 2 missing critical)
**Impact on plan:** All fixes necessary for correct service/job execution. No scope creep -- every fix addressed a real failure discovered during E2E testing.

## Issues Encountered
- Corosensei panics when dropping a coroutine suspended inside extern "C" function (force_unwind cannot cross FFI boundary). Solved by waking actors instead of force-dropping.
- Service loop actor hangs after snow_main completes because no shutdown mechanism existed for Waiting actors. Solved by Phase 3 shutdown in worker_loop that wakes Waiting actors, which then detect null from receive and exit cleanly.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All Phase 9 success criteria verified through E2E tests
- Service: defined, started, called synchronously, cast asynchronously, state managed functionally
- Job: spawned async, awaited with Result<T, String>
- Phase 10 (Polish and Release) can proceed
- No blockers

## Self-Check: PASSED

---
*Phase: 09-concurrency-standard-library*
*Completed: 2026-02-07*
