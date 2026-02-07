---
phase: 12-pipe-operator-closures
plan: 03
subsystem: typeck
tags: [pipe-operator, closures, type-inference, arity, gap-closure]

# Dependency graph
requires:
  - phase: 12-02
    provides: "Closure parsing, MIR lowering, and pipe desugaring in lower_pipe_expr"
provides:
  - "Pipe-aware call inference in infer_pipe for multi-arg functions"
  - "E2E verification of pipe+closure integration"
  - "Chained pipe+closure e2e test"
affects: [13-stdlib-list-operations]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Pipe-aware call inference: match CallExpr RHS in infer_pipe to prepend lhs_ty before arity check"]

key-files:
  created:
    - tests/e2e/pipe_chain_closures.snow
  modified:
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/tests/integration.rs
    - tests/e2e/closure_bare_params_pipe.snow
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "Pipe-aware inference handles CallExpr RHS directly in infer_pipe, not in infer_call"
  - "Untyped function parameters used in test to avoid Fun() annotation parsing limitation"

patterns-established:
  - "Pipe CallExpr path: infer callee + explicit args, prepend lhs_ty, unify as full function type"

# Metrics
duration: 4min
completed: 2026-02-07
---

# Phase 12 Plan 03: Pipe-Aware Type Checking Summary

**Pipe-aware call inference in infer_pipe enabling `list |> map(fn x -> x * 2 end)` to type check and execute correctly**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-07T23:20:54Z
- **Completed:** 2026-02-07T23:24:56Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- Fixed typeck pipe+call arity mismatch by adding CallExpr-aware path in `infer_pipe`
- Updated `closure_bare_params_pipe.snow` to use actual pipe syntax (`list |> map(fn x -> x * 2 end)`)
- Created chained pipe test: `list |> map(fn x -> x + 1 end) |> filter(fn x -> x > 3 end) |> reduce(0, fn acc, x -> acc + x end)`
- Zero test regressions across all crates (typeck 217, codegen 85, parser 210, fmt, e2e 21)

## Task Commits

Each task was committed atomically:

1. **Task 1: Pipe-aware call inference in infer_pipe + typeck unit test** - `23c55cd` (feat)
2. **Task 2: E2E tests with actual pipe+closure syntax** - `41132de` (test)

**Plan metadata:** TBD (docs: complete plan)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - Added CallExpr match arm in infer_pipe: infers callee + explicit args, prepends lhs_ty, unifies as full function type; includes where-clause constraint checking
- `crates/snow-typeck/tests/integration.rs` - Added 3 tests: pipe_call_arity, pipe_call_with_closure, pipe_bare_function_ref
- `tests/e2e/closure_bare_params_pipe.snow` - Updated from direct calls to pipe syntax for map and filter
- `tests/e2e/pipe_chain_closures.snow` - New: chained pipes with map, filter, reduce closures
- `crates/snowc/tests/e2e.rs` - Registered e2e_pipe_chain_closures test

## Decisions Made
- Pipe-aware inference is entirely within `infer_pipe` -- `infer_call` is unchanged. When RHS is a `CallExpr`, we extract the callee and explicit args, prepend `lhs_ty` to build the full arg list, and unify the callee type with `Fun([lhs_ty, ...explicit_args], ret_var)`.
- Used untyped function parameters in the typeck closure test because `Fun(Int, Int)` annotation syntax is not parsed as a function type by the annotation parser (it treats `Fun` as a type constructor). This is a pre-existing annotation limitation, not introduced by this plan.
- Kept `reduce` as direct call in `closure_bare_params_pipe.snow` since it tests multi-param closures (not pipe specifically). Used piped `reduce` in `pipe_chain_closures.snow` to verify 3-arg pipe.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
- The `Fun(Int, Int)` type annotation in the typeck test was not recognized as a function type by the annotation parser (it parses `Fun` as `Ty::Con("Fun")` without consuming the parenthesized args). Resolved by using untyped parameters and letting inference determine the closure type. The test still validates the pipe+closure path correctly.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Phase 12 gap closure complete: pipe+closure integration works end-to-end
- All Phase 12 success criteria verified:
  - SC1: `list |> map(fn x -> x * 2 end)` parses AND executes correctly
  - SC3: Multiple chained pipes with closures work
- Ready for Phase 13 (stdlib list operations)
- The `Fun()` annotation parser limitation is pre-existing and does not block any current functionality

## Self-Check: PASSED

---
*Phase: 12-pipe-operator-closures*
*Completed: 2026-02-07*
