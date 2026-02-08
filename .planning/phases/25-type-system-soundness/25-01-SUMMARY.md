---
phase: 25-type-system-soundness
plan: 01
subsystem: typeck
tags: [type-inference, where-clause, constraint-propagation, soundness, let-binding]

# Dependency graph
requires:
  - phase: 18-19
    provides: "Where-clause enforcement via fn_constraints map and TraitRegistry.check_where_constraints"
provides:
  - "Constraint propagation through let bindings -- fn_constraints entries clone to aliases"
  - "Chain alias support -- let f = show; let g = f; g(x) checks constraints"
  - "3 e2e tests proving alias, chain-alias, and user-trait constraint preservation"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Clone-locally pattern: infer_block clones fn_constraints to avoid &mut cascade through all callers"
    - "NameRef detection in let bindings for constraint propagation"

key-files:
  created: []
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"

key-decisions:
  - "Clone-locally strategy for fn_constraints in infer_block (avoids &mut cascade to 10+ callers)"
  - "Propagate constraints only for NameRef initializers (closures already work via inner call)"

patterns-established:
  - "Constraint alias propagation: when let binding RHS is NameRef to constrained fn, clone FnConstraints entry"

# Metrics
duration: 8min
completed: 2026-02-08
---

# Phase 25 Plan 01: Type System Soundness Summary

**Where-clause constraint propagation through let-binding aliases via fn_constraints cloning in infer_let_binding and infer_block**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-08T20:18:15Z
- **Completed:** 2026-02-08T20:25:52Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Fixed TSND-01 soundness bug: `let f = show; f(non_display_value)` now produces compile-time TraitNotSatisfied error
- Chain aliases work: `let f = show; let g = f; g(42)` also correctly errors
- User-defined trait constraints preserved through aliases (not just stdlib traits)
- No false positives: aliased constrained functions called with conforming types still compile
- All 1,206 tests pass (1,203 existing + 3 new), zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Propagate fn_constraints through let bindings** - `9eb3477` (fix)
2. **Task 2: End-to-end tests for constraint alias propagation** - `7a79fa9` (test)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - Constraint propagation in infer_let_binding + infer_block local clone strategy
- `crates/snow-codegen/src/mir/lower.rs` - 3 new e2e tests for alias, chain-alias, and user-trait constraint preservation

## Decisions Made
- **Clone-locally strategy for fn_constraints in infer_block:** Instead of changing infer_block's parameter to `&mut` (which would cascade to 10+ callers: infer_fn_def, infer_if, infer_case, infer_closure, etc.), we clone fn_constraints into a local `let mut local_fn_constraints` at the top of infer_block. This contains the mutability to a single function.
- **Propagate only for NameRef initializers:** Closures like `let f = fn(x) do show(x) end` already work because the inner `show(x)` call checks constraints directly. Only bare function aliases (`let f = show`) need constraint propagation.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- TSND-01 (the last v1.4 known limitation) is now closed
- Type system soundness for where-clause aliases is verified by 4 e2e tests
- Higher-order function constraint propagation (e.g., `apply(show, value)`) remains a known limitation documented in research -- would require qualified types or constraint-carrying function types
- v1.4 Compiler Polish milestone is feature-complete

## Self-Check: PASSED

---
*Phase: 25-type-system-soundness*
*Completed: 2026-02-08*
