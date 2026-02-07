---
phase: 07-supervision-fault-tolerance
plan: 03
subsystem: typeck-e2e
tags: [typeck, diagnostics, supervisor, child-spec, validation, e2e, error-codes]

# Dependency graph
requires:
  - phase: 07-02
    provides: "Supervisor compiler pipeline (parser, type checker, MIR, codegen, E2E basic test)"
  - phase: 07-01
    provides: "Supervisor runtime (restart strategies, restart limits, exit propagation)"
provides:
  - "Compile-time child spec validation: InvalidChildStart (E0018), InvalidStrategy (E0019), InvalidRestartType (E0020), InvalidShutdownValue (E0021)"
  - "Deep infer_supervisor_def validation: strategy, child start fn, restart type, shutdown value, duplicate child names"
  - "E2E tests for one_for_all strategy, restart limits, and typed supervision rejection"
  - "All four Phase 7 success criteria verified with passing tests"
affects: ["08-channels-select"]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "CST token walking for child spec validation (SPAWN_KW detection for start fn Pid return)"
    - "Negative E2E tests: compile_only helper asserts compilation failure with expected error codes"

key-files:
  created:
    - "crates/snow-typeck/tests/supervisors.rs"
    - "tests/e2e/supervisor_one_for_all.snow"
    - "tests/e2e/supervisor_restart_limit.snow"
    - "tests/e2e/supervisor_typed_error.snow"
  modified:
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snowc/tests/e2e_supervisors.rs"

key-decisions:
  - "Child start function validation uses SPAWN_KW token detection (not full type inference of closure body)"
  - "Error codes E0018-E0021 assigned sequentially after E0017 (ReceiveOutsideActor)"
  - "Duplicate child name detection reuses InvalidStrategy variant with descriptive message"
  - "Negative E2E test (supervisor_typed_error.snow) uses compile_only helper to verify compilation failure"

patterns-established:
  - "Compile-time supervisor validation: strategy, restart type, shutdown value, start fn return type"
  - "Negative compilation tests: compile_only returns raw Output, test asserts failure and error code presence"

# Metrics
duration: 7min
completed: 2026-02-07
---

# Phase 7 Plan 3: Child Spec Validation and E2E Integration Tests Summary

**Compile-time child spec validation with error codes E0018-E0021, 11 type checker tests, and 3 E2E test programs verifying all four Phase 7 success criteria -- completing OTP-style supervision with compile-time safety**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-07T04:09:13Z
- **Completed:** 2026-02-07T04:16:30Z
- **Tasks:** 2
- **Files modified:** 8 (4 created, 4 modified)

## Accomplishments

- Four new TypeError variants with error codes: InvalidChildStart (E0018), InvalidStrategy (E0019), InvalidRestartType (E0020), InvalidShutdownValue (E0021)
- Ariadne diagnostic rendering for all four error codes with labeled source spans and help text
- Deep `infer_supervisor_def` validation: strategy must be known (one_for_one/one_for_all/rest_for_one/simple_one_for_one), child start functions must use spawn() (returns Pid), restart types must be valid (permanent/transient/temporary), shutdown values must be positive integers or brutal_kill, duplicate child names detected
- 11 type checker tests covering valid supervisors, all strategies, invalid strategy, invalid child start, invalid restart type, invalid shutdown, brutal_kill, and duplicate child names
- E2E test: supervisor_one_for_all.snow compiles and runs with two-child one_for_all supervisor
- E2E test: supervisor_restart_limit.snow compiles and runs with restart limit configuration
- E2E test: supervisor_typed_error.snow correctly fails to compile with E0018 when start fn returns Int instead of Pid
- All four Phase 7 success criteria verified with passing tests

## Phase 7 Success Criteria Verification

| # | Criterion | How Verified | Test |
|---|-----------|-------------|------|
| 1 | one_for_one restarts only crashed child | supervisor_basic.snow + runtime unit tests | supervisor_basic E2E + test_one_for_one_restarts_only_failed_child (07-01) |
| 2 | one_for_all restarts all children | supervisor_one_for_all.snow E2E test | supervisor_one_for_all |
| 3 | Restart limits prevent infinite loops | supervisor_restart_limit.snow + runtime unit tests | supervisor_restart_limit E2E + test_restart_limit_exceeded (07-01) |
| 4 | Typed supervision validates at compile time | supervisor_typed_error.snow negative E2E test | supervisor_typed_error_rejected |

## Task Commits

Each task was committed atomically:

1. **Task 1: Compile-time child spec validation and error codes** - `d75202c` (feat)
2. **Task 2: E2E integration tests and success criteria verification** - `af52e19` (feat)

## Files Created/Modified

- `crates/snow-typeck/src/error.rs` - Added InvalidChildStart, InvalidStrategy, InvalidRestartType, InvalidShutdownValue variants with Display impls
- `crates/snow-typeck/src/diagnostics.rs` - Added error codes E0018-E0021 and ariadne rendering for all four
- `crates/snow-typeck/src/infer.rs` - Deepened infer_supervisor_def with strategy, child start fn, restart type, shutdown value, duplicate name validation
- `crates/snow-typeck/tests/supervisors.rs` - 11 type checker tests for supervisor validation
- `crates/snowc/tests/e2e_supervisors.rs` - Added compile_only, run_with_timeout helpers and 3 new E2E tests
- `tests/e2e/supervisor_one_for_all.snow` - one_for_all strategy test fixture
- `tests/e2e/supervisor_restart_limit.snow` - Restart limit test fixture
- `tests/e2e/supervisor_typed_error.snow` - Negative test: start fn returning Int rejected at compile time

## Decisions Made

- Child start function validation uses SPAWN_KW token detection rather than full type inference of the closure body -- pragmatic approach that catches the common error case
- Error codes E0018-E0021 follow sequential numbering after E0017
- Negative E2E test pattern established: compile_only returns raw Output, test asserts compilation failure and checks stderr/stdout for expected error code

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Phase 7 is COMPLETE: all three plans executed, all four success criteria verified
- Snow now has full OTP-style supervision with compile-time safety
- Ready for Phase 8: Channels & Select

## Self-Check: PASSED
