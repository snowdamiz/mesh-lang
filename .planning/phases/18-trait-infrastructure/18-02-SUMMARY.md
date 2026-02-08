---
phase: 18-trait-infrastructure
plan: 02
subsystem: type-system
tags: [duplicate-impl, ambiguity-detection, error-variants, trait-registry, diagnostics]
dependency-graph:
  requires:
    - "18-01 (Vec-based impl storage, structural matching via InferCtx)"
  provides:
    - "DuplicateImpl and AmbiguousMethod error variants with diagnostic rendering"
    - "Duplicate impl detection in register_impl via structural overlap checking"
    - "find_method_traits helper for ambiguity diagnostics at call sites"
  affects:
    - "18-03 (TraitRegistry exposure -- new error variants available to callers)"
    - "19-xx (trait codegen can rely on duplicate-free impls)"
    - "20-xx (method call ambiguity detection at call sites via find_method_traits)"
tech-stack:
  added: []
  patterns:
    - "Check-before-insert with structural matching for impl overlap detection"
    - "Span-less error diagnostics using full-source fallback span"
key-files:
  created: []
  modified:
    - crates/snow-typeck/src/error.rs
    - crates/snow-typeck/src/diagnostics.rs
    - crates/snow-typeck/src/traits.rs
key-decisions:
  - decision: "Duplicate detection uses same structural matching as lookup (freshen + unify in throwaway InferCtx)"
    rationale: "Reuses proven mechanism from 18-01; consistent definition of 'overlap' between registration and lookup"
  - decision: "find_method_traits as separate helper rather than modifying resolve_trait_method"
    rationale: "Preserves backward compatibility; ambiguity check belongs at call site (infer.rs) where error context exists"
  - decision: "Still push impl even when duplicate detected"
    rationale: "Later lookups should still find the impl for downstream error recovery; error is returned to caller"
metrics:
  duration: "8min"
  completed: "2026-02-08"
---

# Phase 18 Plan 02: Duplicate Impl Detection Summary

DuplicateImpl/AmbiguousMethod error variants with full diagnostic rendering, check-before-insert duplicate detection in register_impl using structural type overlap, and find_method_traits helper for call-site ambiguity diagnostics.

## Performance

- Duration: ~8 minutes
- All 1,101+ workspace tests pass with zero regressions
- 13 trait-specific tests pass (9 from 18-01 + 4 new duplicate/ambiguity tests)

## Accomplishments

1. **DuplicateImpl error variant**: Added to `TypeError` with `trait_name`, `impl_type`, and `first_impl` fields. Display format: `"duplicate impl: \`Printable\` is already implemented for \`Int\` (previously defined for \`Int\`)"`. Diagnostic rendering with error code E0026 and help suggestion "remove one of the conflicting impl blocks".

2. **AmbiguousMethod error variant**: Added to `TypeError` with `method_name`, `candidate_traits` (Vec<String>), and `ty` fields. Display format: `"ambiguous method \`to_string\` for type \`Int\`: candidates from traits [Printable, Displayable]"`. Diagnostic rendering with error code E0027 and help suggestion "use qualified syntax: TraitName.method_name(value)".

3. **Duplicate detection in register_impl**: Before pushing a new impl, iterates existing impls for the same trait and uses structural matching (freshen both types in a shared throwaway InferCtx, then unify) to detect overlap. Returns DuplicateImpl error when overlap found. Still pushes the impl for error recovery.

4. **find_method_traits helper**: New method `find_method_traits(&self, method_name: &str, ty: &Ty) -> Vec<String>` that returns all trait names providing a given method for a given type. Iterates all impls across all traits, collecting matches via structural unification. Useful for ambiguity diagnostics at call sites.

5. **Four new tests**: `duplicate_impl_detected`, `no_false_duplicate_for_different_types`, `find_method_traits_single`, `find_method_traits_multiple`.

## Task Commits

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Add DuplicateImpl and AmbiguousMethod error variants | `d6bc822` | `error.rs`, `diagnostics.rs` |
| 2 | Integrate duplicate detection and ambiguity handling into TraitRegistry | `cad88a3` | `traits.rs` |

## Files Modified

- `crates/snow-typeck/src/error.rs` -- +34 lines (2 new variants + Display impls)
- `crates/snow-typeck/src/diagnostics.rs` -- +59 lines (error codes + ariadne rendering for both variants)
- `crates/snow-typeck/src/traits.rs` -- +231/-2 lines (duplicate detection, find_method_traits, 4 tests)

## Decisions Made

1. **Structural overlap detection reuses 18-01 mechanism**: Both the existing impl type and new impl type are freshened in a shared throwaway InferCtx, then unified. If unification succeeds, the types overlap. This is consistent with how `has_impl` and `find_impl` determine matches.

2. **find_method_traits as separate helper**: Rather than modifying `resolve_trait_method` to return errors, added a separate `find_method_traits` method. The ambiguity check belongs at the call site in `infer.rs` where the full error context (spans, source positions) exists. This preserves backward compatibility for existing callers.

3. **Push impl even on duplicate**: The duplicate impl is still stored so that method resolution and error recovery continue to work. The DuplicateImpl error is returned to the caller for reporting.

## Deviations from Plan

None -- plan executed exactly as written.

## Issues Encountered

None.

## Next Phase Readiness

- **18-03 (TraitRegistry exposure)**: Ready. TraitRegistry now has complete error detection for duplicate impls and the `find_method_traits` diagnostic helper. No blockers.
- **19-xx (trait codegen)**: DuplicateImpl errors will be caught during trait registration, ensuring codegen receives clean, non-overlapping impl sets.

## Self-Check: PASSED
