---
phase: 18-trait-infrastructure
plan: 01
subsystem: type-system
tags: [trait-registry, structural-matching, unification, generic-impls]
dependency-graph:
  requires: []
  provides:
    - "TraitRegistry with structural type matching via temporary unification"
    - "freshen_type_params for replacing type parameters with fresh inference variables"
    - "Vec-based impl storage keyed by trait name"
  affects:
    - "18-02 (duplicate impl detection iterates Vec<ImplDef>)"
    - "18-03 (TraitRegistry exposure to codegen)"
    - "19-xx (trait codegen relies on structural find_impl)"
tech-stack:
  added: []
  patterns:
    - "Temporary InferCtx for structural type matching (create, unify, discard)"
    - "Type parameter freshening via single-uppercase-letter heuristic"
    - "Vec-based impl storage with linear scan + unification"
key-files:
  created: []
  modified:
    - crates/snow-typeck/src/traits.rs
key-decisions:
  - decision: "Storage refactored from FxHashMap<(String,String), ImplDef> to FxHashMap<String, Vec<ImplDef>>"
    rationale: "String keys cannot match generic types; Vec per trait allows structural iteration"
  - decision: "Type parameter heuristic: single uppercase ASCII letter (A-Z) only"
    rationale: "Covers T, U, V, K, E etc. without false-positiving on Int, Float, List, Option"
  - decision: "Temporary InferCtx created and discarded per match attempt"
    rationale: "Must not pollute any shared inference state during trait lookup"
metrics:
  duration: "5min"
  completed: "2026-02-08"
---

# Phase 18 Plan 01: Structural Type Matching Summary

TraitRegistry refactored from string-based type_to_key lookup to structural matching via temporary InferCtx unification, enabling generic impl resolution (e.g. `impl Display for List<T>` matches `List<Int>`).

## Performance

- Duration: ~5 minutes
- All 1,097 workspace tests pass with zero regressions
- 9 trait-specific tests pass (3 original preserved + 1 builtins + 5 new structural matching tests)

## Accomplishments

1. **Storage refactor**: Changed `impls` from `FxHashMap<(String, String), ImplDef>` to `FxHashMap<String, Vec<ImplDef>>` keyed by trait name only. This allows multiple impls per trait and removes the need for string type keys entirely.

2. **Structural matching engine**: Added `freshen_type_params()` and `freshen_recursive()` functions that walk a `Ty` and replace any `Ty::Con` with a single uppercase ASCII letter name (T, U, V, K, E, etc.) with a fresh `Ty::Var` from a temporary `InferCtx`. A local `FxHashMap<String, Ty>` ensures the same parameter name maps to the same fresh variable within one pass.

3. **Rewrote all lookup methods**: `has_impl`, `find_impl`, and `resolve_trait_method` now create a throwaway `InferCtx`, freshen the impl's stored type, and attempt unification against the query type. The temporary context is discarded after each attempt, ensuring no corruption of shared state.

4. **Removed `type_to_key`**: The string-based type key function is completely removed. Zero references remain.

5. **New tests**: Added 5 tests validating generic matching, no false positives, simple type regression, resolve_trait_method with generics, and find_impl with generics.

## Task Commits

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Refactor impl storage and add structural type matching | `7ac23d8` | `crates/snow-typeck/src/traits.rs` |

## Files Modified

- `crates/snow-typeck/src/traits.rs` -- 553 lines (was 294), +311/-52

## Decisions Made

1. **Vec-based storage over string keys**: FxHashMap<String, Vec<ImplDef>> keyed by trait name. Linear scan per trait with unification is correct for all type shapes and the number of impls per trait in practice is small.

2. **Single-uppercase-letter heuristic for type parameters**: `c.name.len() == 1 && c.name.as_bytes()[0].is_ascii_uppercase()` covers all common type parameter names (T, U, V, K, E) without false-positiving on concrete types like Int, Float, String, List, Option, Result.

3. **Throwaway InferCtx per match**: Each candidate impl gets its own fresh InferCtx. After the match check, the context is discarded. This prevents unification side-effects from leaking between candidates or into the main inference context.

## Deviations from Plan

None -- plan executed exactly as written.

## Issues Encountered

None.

## Next Phase Readiness

- **18-02 (Duplicate impl detection)**: Ready. The Vec<ImplDef> storage enables iteration for duplicate checking. `find_impl` with structural matching can detect overlapping impls before insertion.
- **18-03 (TraitRegistry exposure)**: Ready. No blockers. TraitRegistry API is stable and uses the new structural matching internally.

## Self-Check: PASSED
