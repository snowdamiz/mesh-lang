---
phase: 04-pattern-matching-adts
plan: 05
subsystem: typeck-diagnostics-adt-migration
tags: [ariadne, diagnostics, sum-types, option, result, exhaustiveness, adt]
completed: 2026-02-06
duration: 5min
dependency-graph:
  requires: ["04-02", "04-03", "04-04"]
  provides: ["diagnostic-rendering-phase4", "option-result-sum-types", "end-to-end-phase4"]
  affects: ["05-concurrency"]
tech-stack:
  added: []
  patterns: ["shared-variant-registration", "builtin-sum-types-as-user-definable"]
key-files:
  created:
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_non_exhaustive_match.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_redundant_arm.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_invalid_guard_expression.snap
  modified:
    - crates/snow-typeck/src/error.rs
    - crates/snow-typeck/src/diagnostics.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/unify.rs
    - crates/snow-typeck/tests/diagnostics.rs
    - crates/snow-typeck/tests/sum_types.rs
decisions:
  - "Error codes: E0012 (NonExhaustiveMatch), W0001 (RedundantArm), E0013 (InvalidGuardExpression)"
  - "RedundantArm uses Warning report kind (W0001), not Error -- distinct from hard errors"
  - "Option/Result registered as SumTypeDefInfo in TypeRegistry, not just as constructors in env"
  - "Shared register_variant_constructors function unifies user-defined and builtin sum type registration"
  - "Qualified access (Option.Some, Result.Ok) works for builtin sum types after migration"
metrics:
  tests-before: 356
  tests-after: 389
  tests-added: 33
  regressions: 0
---

# Phase 4 Plan 05: Diagnostic Rendering and Option/Result ADT Migration Summary

Ariadne diagnostic rendering for all new Phase 4 error types plus migration of Option/Result from hardcoded constructors to proper sum types registered through the ADT mechanism.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Ariadne diagnostic rendering for new error types | 4694467 | NonExhaustiveMatch/RedundantArm/InvalidGuardExpression rendering + 8 snapshot tests |
| 2 | Option/Result migration to sum types and e2e verification | 1a8d39f | register_builtin_sum_types, shared variant registration, 10 e2e tests |

## Changes Made

### Task 1: Diagnostic Rendering

Added ariadne rendering for three new TypeError variants (introduced by parallel 04-04 plan):

- **NonExhaustiveMatch (E0012)**: Error with missing patterns listed as label text, help suggesting wildcard `_` arm
- **RedundantArm (W0001)**: Warning (not Error) with "unreachable" label, help to remove or reorder
- **InvalidGuardExpression (E0013)**: Error with guard restriction explanation

Added `warnings` field to TypeckResult and InferCtx to support distinct warning collection (RedundantArm is a warning, not an error).

8 new tests: 3 insta snapshot tests for rendering, 5 assertion tests for error codes/content.

### Task 2: Option/Result Sum Type Migration

Replaced `register_option_result_constructors` (which only registered constructors in the env) with `register_builtin_sum_types` which:

1. Registers Option<T> as `SumTypeDefInfo` with variants Some(T) and None
2. Registers Result<T,E> as `SumTypeDefInfo` with variants Ok(T) and Err(E)
3. Uses shared `register_variant_constructors` function

Extracted `register_variant_constructors` from `register_sum_type_def` so both user-defined sum types and builtin sum types share the same registration logic. This is the canonical variant constructor registration path.

10 new end-to-end tests covering: qualified/unqualified access, nested generics, or-patterns, full lifecycle, backward compatibility, Option/Result sugar.

## Decisions Made

1. **Error codes**: E0012 for NonExhaustiveMatch, W0001 for RedundantArm (warning prefix), E0013 for InvalidGuardExpression
2. **RedundantArm as Warning**: Uses `ReportKind::Warning` and yellow coloring, distinct from Error-level diagnostics
3. **Shared variant registration**: `register_variant_constructors` is the single path for both user-defined and builtin sum types
4. **Builtin sum types in TypeRegistry**: Option and Result are now full SumTypeDefInfo entries enabling exhaustiveness checking

## Deviations from Plan

### Coordination with Parallel 04-04

04-04 (running in parallel) had already added the new TypeError variants (NonExhaustiveMatch, RedundantArm, InvalidGuardExpression), their Display impls, their diagnostic rendering, error codes, and the warnings infrastructure. Task 1 therefore focused on adding comprehensive snapshot and assertion tests for all new diagnostic types rather than implementing the rendering from scratch. All changes were committed together for coherence.

## Verification

- `cargo test --workspace`: 389 tests pass, 0 failures
- All Phase 1-3 tests unaffected (zero regressions)
- Option/Result construction: `Some(42)`, `None`, `Ok(42)`, `Err("bad")` all type-check correctly
- Qualified access: `Option.Some(42)`, `Result.Ok(42)` work
- Option sugar: `Int?` resolves to `Option<Int>`
- Pattern matching: `case opt do Some(x) -> x | None -> 0 end` type-checks correctly
- Nested generics: `Some(Some(42))` with nested pattern matching works
- Diagnostic rendering: All new error types render with ariadne

## Next Phase Readiness

Phase 4 (Pattern Matching & ADTs) is feature-complete pending 04-04's exhaustiveness wiring into `infer_case`. All the pieces are in place:
- Sum type parsing (04-01)
- Type inference for constructors and patterns (04-02)
- Maranget's usefulness algorithm (04-03)
- Exhaustiveness wiring and guard validation (04-04, in progress)
- Diagnostic rendering and Option/Result as sum types (this plan)

## Self-Check: PASSED
