---
phase: 48-tail-call-elimination
plan: 02
subsystem: compiler
tags: [codegen, tail-call-elimination, tce, llvm, loop-wrapping, e2e-tests]

# Dependency graph
requires:
  - "48-01: MirExpr::TailCall variant, MirFunction.has_tail_calls flag, rewrite_tail_calls pass"
provides:
  - "TCE loop wrapping in compile_function (tce_loop block when has_tail_calls is true)"
  - "TailCall codegen: two-phase arg eval, store to param allocas, reduction check, branch to loop header"
  - "Entry-block alloca hoisting for safe stack usage in TCE loops"
  - "4 comprehensive e2e tests: countdown 1M, param swap, case arms, actor context"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Entry-block alloca hoisting via build_entry_alloca helper (prevents stack growth in TCE loops)"
    - "Two-phase argument evaluation for TailCall (evaluate all args THEN store all values)"

key-files:
  modified:
    - "crates/snow-codegen/src/codegen/mod.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snowc/tests/e2e.rs"
  created:
    - "tests/e2e/tce_countdown.snow"
    - "tests/e2e/tce_param_swap.snow"
    - "tests/e2e/tce_case_arms.snow"
    - "tests/e2e/tce_actor_loop.snow"

key-decisions:
  - "Two-phase arg evaluation: clone tce_param_names to avoid borrow checker issues while iterating and accessing self.locals"
  - "Entry-block alloca hoisting only when tce_loop_header is set (minimal change to non-TCE codegen paths)"
  - "Actor test uses recursive function inside actor context rather than self-recursive actor (type system treats actor names as returning Pid)"
  - "Snow case/receive arms must be single expressions (no multi-line blocks) -- adapted test designs accordingly"

patterns-established:
  - "build_entry_alloca helper: saves/restores builder position to create allocas in the function entry block"
  - "TCE codegen pattern: tce_loop_header + tce_param_names fields on CodeGen, cleared after each function"

# Metrics
duration: 13min
completed: 2026-02-10
---

# Phase 48 Plan 02: TCE Codegen Loop Wrapping Summary

**LLVM codegen for tail-call elimination: loop wrapping, two-phase arg evaluation, entry-block alloca hoisting, and 4 e2e tests proving 1M-iteration recursion**

## Performance

- **Duration:** 13 min
- **Started:** 2026-02-10T17:29:00Z
- **Completed:** 2026-02-10T17:42:37Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Implemented TCE loop wrapping in compile_function: creates tce_loop block when has_tail_calls is true
- Implemented TailCall codegen: two-phase argument evaluation (evaluate all args THEN store to param allocas), reduction check, branch to loop header
- Fixed alloca hoisting for TCE loops: if/let/match/receive result allocas placed in entry block to prevent stack growth
- Added build_entry_alloca helper to CodeGen for safe alloca placement
- Created 4 e2e tests covering countdown (1M iterations), param swap, case arms, and actor context
- All 122 e2e tests pass (118 pre-existing + 4 new) with zero regressions
- All 175 unit tests pass unchanged

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement loop wrapping in compile_function and TailCall codegen in expr.rs** - `2c9082b` (feat)
2. **Task 2: Add comprehensive e2e tests for tail-call elimination** - `3656cef` (feat)

## Files Created/Modified
- `crates/snow-codegen/src/codegen/mod.rs` - tce_loop_header/tce_param_names fields, TCE loop wrapping in compile_function, build_entry_alloca helper
- `crates/snow-codegen/src/codegen/expr.rs` - TailCall codegen (two-phase eval + store + branch), entry-block alloca hoisting for if/let/match/receive
- `tests/e2e/tce_countdown.snow` - 1M iteration countdown without stack overflow
- `tests/e2e/tce_param_swap.snow` - Parameter swap correctness (100,001 swaps)
- `tests/e2e/tce_case_arms.snow` - Tail calls through case/match arms
- `tests/e2e/tce_actor_loop.snow` - TCE function inside actor context (1M iterations)
- `crates/snowc/tests/e2e.rs` - 4 new test functions registered

## Decisions Made
- Two-phase argument evaluation with `tce_param_names.clone()` to satisfy Rust borrow checker (iterating param names while accessing self.locals)
- Entry-block alloca hoisting conditional on `tce_loop_header.is_some()` to minimize impact on non-TCE codegen paths
- Actor test redesigned: Snow's type system treats actor names as returning Pid, preventing direct self-recursive actor calls. Used recursive function inside actor context instead.
- Snow case/receive arms require single expressions -- adapted test designs with return values in main instead of multi-line print blocks

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed alloca placement inside TCE loops causing stack overflow**
- **Found during:** Task 2 (e2e testing of tce_countdown)
- **Issue:** codegen_if/codegen_let/codegen_match placed alloca instructions inside the tce_loop block. In LLVM, alloca in a loop creates new stack frames each iteration, causing stack overflow at 1M iterations even with TCE.
- **Fix:** Added `build_entry_alloca` helper that saves/restores builder position to create allocas in the function entry block. Applied to if_result, let bindings, match scrutinee/result, receive result, and pattern binding allocas when inside TCE context.
- **Files modified:** crates/snow-codegen/src/codegen/mod.rs, crates/snow-codegen/src/codegen/expr.rs
- **Verification:** tce_countdown with 1M iterations completes successfully; LLVM IR shows alloca in entry block before tce_loop
- **Committed in:** 3656cef (Task 2 commit)

**2. [Rule 1 - Bug] Fixed Snow syntax in test programs**
- **Found during:** Task 2 (writing e2e test programs)
- **Issue:** Plan's Snow code used `if COND then` syntax, but Snow uses `if COND do`. Plan also used `pid <- msg` (not valid Snow) and multi-line case arms (not supported).
- **Fix:** Adapted all 4 test programs to valid Snow syntax: `if COND do...else...end`, `send(pid, msg)`, single-expression case arms with return values captured in main.
- **Files modified:** All 4 .snow test files
- **Verification:** All 4 tests compile and produce expected output
- **Committed in:** 3656cef (Task 2 commit)

**3. [Rule 1 - Bug] Redesigned actor test due to type system constraint**
- **Found during:** Task 2 (writing tce_actor_loop.snow)
- **Issue:** Plan's actor test used self-recursive `counter(n + 1)` inside actor body, but Snow's type checker treats actor names as `() -> Pid`, causing type mismatch when used as self-recursive call.
- **Fix:** Redesigned test to use a tail-recursive function `count_loop(0, 1000000)` called from within an actor's receive handler, proving TCE works in actor context.
- **Files modified:** tests/e2e/tce_actor_loop.snow, crates/snowc/tests/e2e.rs
- **Verification:** Test prints "1000000" from 1M iterations inside actor without stack overflow
- **Committed in:** 3656cef (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (3 bugs)
**Impact on plan:** All fixes necessary for correctness. The alloca hoisting was a genuine LLVM codegen bug that would have made TCE useless for deep recursion. The Snow syntax and actor test fixes adapted the plan's example code to valid Snow language constructs.

## Issues Encountered
None beyond the deviations documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 48 (Tail-Call Elimination) is fully complete
- Plan 01 provided MIR infrastructure (TailCall variant, rewrite pass)
- Plan 02 provides codegen (loop wrapping, arg evaluation, alloca hoisting)
- All success criteria met: 1M countdown, parameter swap, case arms, actor context
- TCE is transparent: all 118 pre-existing e2e tests pass unchanged

## Self-Check: PASSED

All files exist, both commits verified in git log.

---
*Phase: 48-tail-call-elimination*
*Completed: 2026-02-10*
