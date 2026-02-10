---
phase: 45-error-propagation
plan: 03
subsystem: testing
tags: [e2e, diagnostics, error-codes, try-operator, compile-errors]

# Dependency graph
requires:
  - phase: 45-01
    provides: "E0036/E0037 error variants and type-checking logic for ? operator"
  - phase: 45-02
    provides: "MIR lowering and codegen for ? operator (existing e2e tests)"
provides:
  - "E2e test coverage for ? operator error diagnostics (E0036, E0037)"
  - "Regression protection for ERR-03 error emission"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns: [compile_expect_error with read_fixture for error diagnostic tests]

key-files:
  created:
    - tests/e2e/try_error_incompatible_return.snow
    - tests/e2e/try_error_non_result_option.snow
  modified:
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "Asserted on both error code (E0036/E0037) and message text with || for resilience"

patterns-established:
  - "compile_expect_error + read_fixture pattern for testing compile-time error diagnostics"

# Metrics
duration: 2min
completed: 2026-02-10
---

# Phase 45 Plan 03: ERR-03 Gap Closure Summary

**E2e compile_expect_error tests for E0036 (TryIncompatibleReturn) and E0037 (TryOnNonResultOption) closing the last ERR-03 verification gap**

## Performance

- **Duration:** 2 min
- **Started:** 2026-02-10T08:14:23Z
- **Completed:** 2026-02-10T08:16:28Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Created test fixtures triggering E0036 (? in fn returning Int) and E0037 (? on plain Int)
- Added 2 compile_expect_error e2e tests verifying error diagnostic emission
- All 7 Phase 45 e2e tests pass (5 existing + 2 new)
- Full snowc test suite passes with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Create test fixtures for ? operator error cases** - `c0a6a08` (test)
2. **Task 2: Add compile_expect_error e2e tests for ERR-03** - `fb18ba3` (test)

## Files Created/Modified
- `tests/e2e/try_error_incompatible_return.snow` - Fixture: ? used in fn returning Int (triggers E0036)
- `tests/e2e/try_error_non_result_option.snow` - Fixture: ? used on plain Int value (triggers E0037)
- `crates/snowc/tests/e2e.rs` - Added e2e_try_incompatible_return_type and e2e_try_on_non_result_option tests

## Decisions Made
- Asserted on both error code (E0036/E0037) and message text substring with `||` for resilience against format changes
- Used `read_fixture` + `compile_expect_error` pattern (matching existing Phase 45 test conventions)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 45 (Error Propagation) is fully complete with all verification gaps closed
- ERR-01 (type checking), ERR-02 (MIR lowering + codegen), and ERR-03 (error diagnostics) all have e2e coverage
- Ready to proceed to Phase 46

## Self-Check: PASSED

- [x] tests/e2e/try_error_incompatible_return.snow - FOUND
- [x] tests/e2e/try_error_non_result_option.snow - FOUND
- [x] crates/snowc/tests/e2e.rs - FOUND
- [x] Commit c0a6a08 (Task 1) - FOUND
- [x] Commit fb18ba3 (Task 2) - FOUND

---
*Phase: 45-error-propagation*
*Completed: 2026-02-10*
