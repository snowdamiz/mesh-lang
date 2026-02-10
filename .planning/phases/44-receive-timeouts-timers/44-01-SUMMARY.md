---
phase: 44-receive-timeouts-timers
plan: 01
subsystem: codegen
tags: [llvm, receive, timeout, actor, concurrency]

# Dependency graph
requires:
  - phase: 09-actors-runtime
    provides: "snow_actor_receive runtime function with timeout_ms parameter"
provides:
  - "Null-check branching in codegen_actor_receive for timeout body execution"
  - "codegen_recv_load_message and codegen_recv_process_arms helper methods"
  - "E2E tests proving receive-with-timeout works for Int and String types"
affects: [44-02 (Timer.sleep uses receive-with-timeout internally)]

# Tech tracking
tech-stack:
  added: []
  patterns: ["null-check branching after runtime call for timeout detection", "result_alloca + merge block pattern for receive with timeout"]

key-files:
  created: []
  modified:
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snowc/tests/e2e_concurrency_stdlib.rs

key-decisions:
  - "Extracted codegen_recv_load_message and codegen_recv_process_arms helpers to avoid code duplication between timeout and no-timeout paths"
  - "Used result_alloca + merge block pattern (same as codegen_if) for timeout vs message branching"

patterns-established:
  - "Receive timeout codegen: build_is_null on msg_ptr, branch to timeout_bb (null) vs msg_bb (non-null), merge at recv_merge_bb"

# Metrics
duration: 9min
completed: 2026-02-10
---

# Phase 44 Plan 01: Receive Timeout Codegen Summary

**Null-check branching after snow_actor_receive enabling timeout body execution instead of segfault on null pointer**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-10T02:35:59Z
- **Completed:** 2026-02-10T02:45:15Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Fixed the codegen gap where `timeout_body: _` was discarded, causing segfault on receive timeout
- Implemented null-check branching: when `snow_actor_receive` returns null (timeout), branches to timeout body instead of dereferencing
- Extracted `codegen_recv_load_message` and `codegen_recv_process_arms` helpers to share code between timeout and no-timeout paths
- Added 3 e2e tests: timeout fires (Int), message before timeout (Int), timeout returns String

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement receive-with-timeout codegen** - `f77f744` (feat)
2. **Task 2: Add e2e tests for receive-with-timeout** - `3e68d7d` (test)

## Files Created/Modified
- `crates/snow-codegen/src/codegen/expr.rs` - Added timeout_body parameter to codegen_actor_receive, null-check branching, extracted helper methods
- `crates/snowc/tests/e2e_concurrency_stdlib.rs` - 3 new e2e tests for receive-with-timeout behavior

## Decisions Made
- Extracted message loading and arm processing into separate helper methods (`codegen_recv_load_message`, `codegen_recv_process_arms`) to avoid duplicating the message-processing code between timeout and no-timeout paths
- Used `result_alloca` + merge block pattern (same idiom as `codegen_if`) for the branching structure
- Test timeout values use positive integers (99) instead of negative (-1) due to expression parsing after `->` in after clause

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Snow syntax in test source code**
- **Found during:** Task 2 (e2e tests)
- **Issue:** Plan specified incorrect Snow syntax (`fn`, `{ }`, `IO.println`, `Int.to_string`, `spawn worker()`, `send pid 42`). Actual Snow syntax uses `actor`, `do ... end`, `println`, `"${expr}"`, `spawn(name)`, `send(pid, val)`
- **Fix:** Rewrote all test source code using correct Snow syntax
- **Files modified:** crates/snowc/tests/e2e_concurrency_stdlib.rs
- **Verification:** All 3 tests compile and pass
- **Committed in:** 3e68d7d (Task 2 commit)

**2. [Rule 1 - Bug] Fixed `end` placement after `after` clause**
- **Found during:** Task 2 (e2e tests)
- **Issue:** Parser requires `end` on same line as `after` body expression (newline-sensitive parsing; `eat_newlines` not called between after clause and end check)
- **Fix:** Placed `end` on same line as `after timeout -> body end`
- **Files modified:** crates/snowc/tests/e2e_concurrency_stdlib.rs
- **Verification:** Parse errors resolved, all tests pass
- **Committed in:** 3e68d7d (Task 2 commit)

**3. [Rule 1 - Bug] Removed `receive` from `fn main` (non-actor context)**
- **Found during:** Task 2 (e2e tests)
- **Issue:** Plan used `receive` inside `fn main` to wait for workers, but Snow enforces `receive` only inside `actor` blocks. Error: "receive used outside actor block"
- **Fix:** Removed `receive` from main; relied on runtime's built-in wait-for-all-actors-before-exit behavior
- **Files modified:** crates/snowc/tests/e2e_concurrency_stdlib.rs
- **Verification:** All tests pass; runtime correctly waits for actor completion
- **Committed in:** 3e68d7d (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (3 bugs in plan's test source code)
**Impact on plan:** All auto-fixes were necessary to match actual Snow language syntax. No scope creep.

## Issues Encountered
None beyond the syntax corrections documented in deviations.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Receive-with-timeout codegen is complete and tested
- Plan 44-02 (Timer.sleep) can proceed -- it uses empty-arms receive-with-timeout internally, which is now supported by the `codegen_recv_process_arms` no-arms path

## Self-Check: PASSED

- [x] crates/snow-codegen/src/codegen/expr.rs exists
- [x] crates/snowc/tests/e2e_concurrency_stdlib.rs exists
- [x] .planning/phases/44-receive-timeouts-timers/44-01-SUMMARY.md exists
- [x] Commit f77f744 exists
- [x] Commit 3e68d7d exists

---
*Phase: 44-receive-timeouts-timers*
*Completed: 2026-02-10*
