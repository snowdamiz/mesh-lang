---
phase: 29
plan: 01
subsystem: typeck
tags: [qualified-types, higher-order, constraint-propagation, where-clause, HM-inference]
requires:
  - phase-25 (type-system-soundness: fn_constraints alias propagation)
  - phase-18 (where-clause enforcement: check_where_constraints)
provides:
  - Argument-level trait constraint checking in infer_call and infer_pipe
  - Higher-order constrained function passing: apply(show, 42)
  - Nested constraint propagation: wrap(apply, show, 42)
affects:
  - None (final phase in v1.5)
tech-stack:
  added: []
  patterns:
    - Call-site argument constraint check pattern (detect constrained NameRef arguments, resolve via instantiated function type)
key-files:
  created: []
  modified:
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/lower.rs
key-decisions:
  - 29-01-D1: Soft error collection for argument constraint violations (extend ctx.errors) vs hard Err return for callee constraint violations (backward compatible)
  - 29-01-D2: Only check NameRef arguments (covers all practical cases; complex expressions that return constrained functions are out of scope)
  - 29-01-D3: Filter to concrete types only (Ty::Var skipped) to prevent false positives on unresolved type variables
duration: 7min
completed: 2026-02-09
---

# Phase 29 Plan 01: Qualified Types Summary

Argument-level trait constraint checking in infer_call and infer_pipe so that constrained functions passed as higher-order arguments have their constraints verified at the outer call site after HM unification connects type variables to concrete types.

## Performance

- Duration: ~7 minutes
- Tests: 1,232 passing (5 new, 0 regressions)
- Lines changed: +319 (119 in infer.rs, 200 in lower.rs tests)

## Accomplishments

1. **Argument-level constraint check in infer_call**: After the existing callee-name constraint check, iterates over call arguments to detect NameRef arguments with fn_constraints entries. Resolves the constrained function's type parameters from its instantiated function type (after unification), filters to concrete types, and calls check_where_constraints. Errors are collected softly (ctx.errors.extend) rather than hard-failing.

2. **Argument-level constraint check in infer_pipe**: Mirrors the infer_call logic for the pipe operator's CallExpr arm. Handles explicit arguments only (piped lhs is not in arg_list.args()).

3. **origin.clone() fix**: Changed both infer_call and infer_pipe callee-name checks to use origin.clone() instead of consuming origin by value, making origin available for the subsequent argument-level check.

4. **Five e2e tests**: QUAL-01 (conforming higher-order apply), QUAL-03 (violation detection), QUAL-02 (nested propagation), conforming positive case, and let-alias + higher-order interaction.

## Task Commits

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Add argument-level constraint check in infer_call and infer_pipe | 056942b | crates/snow-typeck/src/infer.rs |
| 2 | E2e tests for higher-order constraint propagation | a39baff | crates/snow-codegen/src/mir/lower.rs |

## Files Modified

- `crates/snow-typeck/src/infer.rs` -- Added argument-level constraint check blocks in infer_call (after line 2750) and infer_pipe (after line 2930), plus origin.clone() in both callee-name check blocks
- `crates/snow-codegen/src/mir/lower.rs` -- Added 5 e2e tests after e2e_where_clause_alias_user_trait

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 29-01-D1 | Soft error collection for argument constraints | Callee check returns Err for backward compat; argument check uses extend to avoid aborting inference early when multiple arguments have issues |
| 29-01-D2 | Only check NameRef arguments | Covers show, f, g and let-aliased names; complex expressions would require constraint-carrying types |
| 29-01-D3 | Filter to concrete types (skip Ty::Var) | Prevents false positives when type params haven't resolved yet through unification chain |

## Deviations from Plan

None -- plan executed exactly as written.

## Issues Encountered

None.

## Next Phase Readiness

Phase 29 is the final phase in v1.5 Compiler Correctness. All known type system limitations have been addressed:
- Phase 25: Direct alias constraint propagation (let f = show; f(42))
- Phase 29: Higher-order argument constraint propagation (apply(show, 42))

The Snow type system now correctly enforces where-clause constraints through:
1. Direct calls (show(42)) -- Phase 18/19
2. Let aliases (let f = show; f(42)) -- Phase 25
3. Higher-order passing (apply(show, 42)) -- Phase 29
4. Nested higher-order (wrap(apply, show, 42)) -- Phase 29

## Self-Check: PASSED
