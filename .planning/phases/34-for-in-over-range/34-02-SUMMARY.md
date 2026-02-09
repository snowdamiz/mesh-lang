---
phase: 34-for-in-over-range
plan: 02
subsystem: codegen
tags: [llvm, mir, for-in, range, loop, formatter]

# Dependency graph
requires:
  - phase: 34-01
    provides: "Parser and typechecker support for for-in/range syntax"
  - phase: 33-02
    provides: "While loop codegen pattern with loop_stack, break/continue, reduction check"
provides:
  - "MirExpr::ForInRange variant for desugared for-in over integer ranges"
  - "Four-block LLVM codegen (header/body/latch/merge) with alloca counter"
  - "Half-open range [start, end) semantics via SLT comparison"
  - "Correct break/continue via latch_bb push onto loop_stack"
  - "Reduction check in latch block for actor scheduler fairness"
  - "Formatter walk_for_in_expr for proper indentation"
  - "E2E tests for basic iteration, empty range, reverse range, break, continue"
affects: ["future-list-iteration", "future-iterator-protocol"]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Four-block loop structure (header/body/latch/merge) for counter-based loops"
    - "Latch block as continue target ensures counter increment and reduction check"
    - "DOT_DOT operator formatted without surrounding spaces"

key-files:
  created:
    - tests/e2e/for_in_range.snow
  modified:
    - crates/snow-codegen/src/mir/mod.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-codegen/src/mir/mono.rs
    - crates/snow-codegen/src/pattern/compile.rs
    - crates/snow-fmt/src/walker.rs
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "Continue target is latch_bb (not header), so counter always increments and reduction check always fires"
  - "Half-open range [start, end) using SLT comparison -- consistent with Rust/Python convention"
  - "Loop variable reuses counter alloca directly -- no extra alloca for binding"
  - "DOT_DOT formatted without spaces (0..10 not 0 .. 10)"
  - "E2E tests use string interpolation ${i} since println requires String type"

patterns-established:
  - "Four-block loop: header checks condition, body executes, latch increments+reduction, merge exits"
  - "Counter-based loops push (latch_bb, merge_bb) so continue goes to latch, break goes to merge"

# Metrics
duration: 11min
completed: 2026-02-09
---

# Phase 34 Plan 02: For-In over Range Summary

**Four-block LLVM codegen for for-in range loops with alloca counter, SLT half-open range, latch-based continue, and reduction check**

## Performance

- **Duration:** 11 min
- **Started:** 2026-02-09T08:21:00Z
- **Completed:** 2026-02-09T08:32:45Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- Complete for-in over range pipeline: users can write `for i in 0..10 do body end` and it compiles to zero-allocation integer arithmetic
- Four-block LLVM structure (header/body/latch/merge) with correct break/continue semantics
- Half-open range [start, end) via SLT comparison -- empty and reverse ranges produce zero iterations
- Comprehensive test coverage: 5 e2e tests, 4 codegen unit tests, 1 MIR lowering test, 3 formatter tests

## Task Commits

Each task was committed atomically:

1. **Task 1: Add MirExpr::ForInRange, MIR lowering, LLVM codegen, and formatter** - `52f0ada` (feat)
2. **Task 2: Add e2e tests and unit tests for for-in over range** - `bbd1108` (test)

## Files Created/Modified
- `crates/snow-codegen/src/mir/mod.rs` - Added ForInRange variant to MirExpr with var/start/end/body/ty fields
- `crates/snow-codegen/src/mir/lower.rs` - Added lower_for_in_expr extracting range bounds from DotDot binary expr; ForInRange arm in collect_free_vars; MIR lowering unit test
- `crates/snow-codegen/src/codegen/expr.rs` - Added codegen_for_in_range with four-block LLVM structure, alloca counter, SLT comparison, latch-based continue
- `crates/snow-codegen/src/codegen/mod.rs` - Added codegen unit tests for basic blocks, SLT comparison, reduction check
- `crates/snow-codegen/src/mir/mono.rs` - Added ForInRange arm to collect_function_refs
- `crates/snow-codegen/src/pattern/compile.rs` - Added ForInRange arm to compile_expr_patterns
- `crates/snow-fmt/src/walker.rs` - Added walk_for_in_expr formatter function; fixed DOT_DOT spacing; formatter tests
- `crates/snowc/tests/e2e.rs` - Added 5 e2e test functions for for-in range
- `tests/e2e/for_in_range.snow` - E2E fixture testing basic iteration and variable scoping

## Decisions Made
- **Continue target is latch_bb:** Pushing `(latch_bb, merge_bb)` onto loop_stack means continue always goes through the latch block, which increments the counter and emits reduction check. This prevents infinite loops from continue and ensures actor fairness.
- **Half-open range [start, end) via SLT:** Matches Rust and Python convention. `for i in 0..5` iterates 0,1,2,3,4.
- **Loop variable reuses counter alloca:** No separate alloca needed. The body reads from counter, latch increments it.
- **DOT_DOT formatted without spaces:** `0..10` not `0 .. 10`. Added special case in `walk_binary_expr`.
- **E2E tests use string interpolation:** `println("${i}")` instead of `println(Int.to_string(i))` since nested module-qualified calls have a pre-existing type checker limitation.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed DOT_DOT operator spacing in formatter**
- **Found during:** Task 2 (Formatter tests)
- **Issue:** The binary expr formatter was adding spaces around `..` operator (producing `0 .. 10` instead of `0..10`), causing formatter test failures
- **Fix:** Added special case for `SyntaxKind::DOT_DOT` in `walk_binary_expr` to emit `..` without surrounding spaces
- **Files modified:** `crates/snow-fmt/src/walker.rs`
- **Verification:** Formatter tests pass, idempotency confirmed
- **Committed in:** bbd1108 (Task 2 commit)

**2. [Rule 3 - Blocking] Changed e2e tests from Int.to_string to string interpolation**
- **Found during:** Task 2 (E2E tests)
- **Issue:** `println(Int.to_string(i))` fails due to pre-existing type checker limitation with nested module-qualified calls. This is NOT a for-in bug.
- **Fix:** Used string interpolation `println("${i}")` which is the standard pattern in the Snow codebase for printing integers
- **Files modified:** `tests/e2e/for_in_range.snow`, `crates/snowc/tests/e2e.rs`
- **Verification:** All 5 e2e tests pass
- **Committed in:** bbd1108 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes necessary for correctness. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviations.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- For-in over integer ranges is fully functional
- Foundation ready for future list/iterator iteration (would need new MirExpr variant and different codegen strategy)
- The four-block loop pattern is established as the model for future counter-based loops

## Self-Check: PASSED

All created files verified present. All commit hashes verified in git log. All key patterns verified in artifact files.

---
*Phase: 34-for-in-over-range*
*Completed: 2026-02-09*
