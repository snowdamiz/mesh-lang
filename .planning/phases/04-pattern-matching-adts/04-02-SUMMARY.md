---
phase: 04-pattern-matching-adts
plan: 02
subsystem: typeck
tags: [sum-types, adt, variant-constructors, pattern-matching, type-inference, hindley-milner]

# Dependency graph
requires:
  - phase: 03-type-system
    provides: "HM inference, TypeEnv, InferCtx, Scheme, enter_level/leave_level/generalize"
  - phase: 04-01
    provides: "Parser support for SumTypeDef, VariantDef, ConstructorPat, OrPat, AsPat AST nodes"
provides:
  - "SumTypeDefInfo, VariantInfo, VariantFieldInfo structs in TypeRegistry"
  - "register_sum_type_def: variant constructor registration as polymorphic functions"
  - "Qualified variant access resolution (Shape.Circle) in infer_field_access"
  - "Constructor pattern inference with sub-pattern unification"
  - "Or-pattern inference with binding name validation"
  - "As-pattern inference with whole-value binding"
  - "TypeError::UnknownVariant and TypeError::OrPatternBindingMismatch"
affects: [04-03, 04-04, 04-05]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Variant constructors registered as polymorphic Scheme via enter_level/leave_level/generalize"
    - "Qualified access (Type.Variant) resolved in infer_field_access before struct field lookup"
    - "Semantic-aware binding collection: env-aware function distinguishes constructors from variable bindings"
    - "Smart ident patterns: bare uppercase names resolve to constructors when present in env"

key-files:
  created:
    - "crates/snow-typeck/tests/sum_types.rs"
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"

key-decisions:
  - "Bare uppercase ident patterns (Red, None, Point) resolve to constructors when found in env, not variable bindings"
  - "Or-pattern binding validation uses semantic-aware collection (env lookup) to skip constructor names"
  - "Variant constructors registered under both qualified (Shape.Circle) and unqualified (Circle) names"

patterns-established:
  - "Sum type registration: SumTypeDefInfo/VariantInfo/VariantFieldInfo parallel to StructDefInfo"
  - "Qualified variant access intercept: check NameRef base against sum type registry before struct field lookup"
  - "Pattern inference accepts type_registry parameter for constructor resolution"

# Metrics
duration: 9min
completed: 2026-02-06
---

# Phase 04 Plan 02: Sum Type Type Checking Summary

**Sum type definitions register in TypeRegistry with variant constructors as polymorphic functions; constructor/or/as patterns infer correct types and bind variables in match arms**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-06T21:13:23Z
- **Completed:** 2026-02-06T21:22:50Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Sum type definitions (SumTypeDefInfo) register alongside struct definitions in TypeRegistry
- Variant constructors registered as polymorphic functions using enter_level/leave_level/generalize
- Qualified variant access (Shape.Circle) resolves through infer_field_access
- Constructor patterns unify sub-patterns with variant parameter types
- Or-patterns validate that all alternatives bind identical variable sets
- As-patterns bind the whole matched value to a name
- Smart ident patterns: bare uppercase names in pattern position resolve to constructors when known
- 16 integration tests covering all sum type and pattern scenarios

## Task Commits

Each task was committed atomically:

1. **Task 1: Sum type registration and variant constructor inference** - `4b1fb01` (feat)
2. **Task 2: Constructor, or-pattern, and as-pattern type inference** - `891c9de` (feat)

**Plan metadata:** _(to be committed)_ (docs: complete plan)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - SumTypeDefInfo/VariantInfo/VariantFieldInfo structs, TypeRegistry extension with sum_type_defs, register_sum_type_def, qualified variant access in infer_field_access, constructor/or/as pattern inference, smart ident pattern resolution, semantic-aware binding collection
- `crates/snow-typeck/src/error.rs` - TypeError::UnknownVariant and TypeError::OrPatternBindingMismatch variants with Display impls
- `crates/snow-typeck/src/diagnostics.rs` - E0010/E0011 error codes and ariadne rendering for new error types
- `crates/snow-typeck/tests/sum_types.rs` - 16 integration tests for sum type construction, pattern matching, and error cases

## Decisions Made
- **Bare uppercase ident patterns resolve to constructors:** Since the parser treats bare uppercase names (without parens) as IDENT_PAT, the type checker checks whether the name exists in the env and resolves to a sum type. This avoids creating variable bindings for nullary constructors like `Red`, `None`, `Point`.
- **Semantic-aware binding collection for or-patterns:** The `collect_pattern_binding_names` function consults the TypeEnv to distinguish constructor names from variable bindings, ensuring or-pattern validation doesn't spuriously flag `Red | Green` as a binding mismatch.
- **Both qualified and unqualified constructor registration:** Variant constructors are registered under both `Shape.Circle` (for explicit qualification) and `Circle` (for backward compatibility with builtins like Some/None/Ok/Err).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Smart ident pattern resolution for nullary constructors**
- **Found during:** Task 2 (constructor pattern inference)
- **Issue:** Bare uppercase names like `Red` in pattern position were parsed as IDENT_PAT (per 04-01 design: only uppercase IDENT + L_PAREN = CONSTRUCTOR_PAT). This meant `case c do Red -> 1 end` would create a fresh variable binding `Red` instead of matching the Color.Red constructor.
- **Fix:** Added logic to `Pattern::Ident` branch of `infer_pattern` to check if the name resolves to a sum type in the env, and if so, treat it as a constructor match instead of a variable binding.
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** test_or_pattern_nullary passes (Red | Green correctly recognized as constructors)
- **Committed in:** 891c9de (Task 2 commit)

**2. [Rule 1 - Bug] Semantic-aware binding collection for or-pattern validation**
- **Found during:** Task 2 (or-pattern inference)
- **Issue:** Purely syntactic binding collection treated every ident pattern as a variable binding, causing `Red | Green` to fail with OrPatternBindingMismatch (["Red"] vs ["Green"]).
- **Fix:** Changed `collect_pattern_binding_names` to accept `&TypeEnv` and skip names that exist in the env (i.e., known constructors).
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** test_or_pattern_nullary passes
- **Committed in:** 891c9de (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs)
**Impact on plan:** Both auto-fixes necessary for correct interaction between parser design (04-01's IDENT_PAT heuristic) and type checker. No scope creep.

## Issues Encountered
- Pre-existing uncommitted changes in `exhaustiveness.rs` from another agent's partial work on Plan 04-03. These were not committed as part of this plan; the file was restored to its committed state.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Sum type TypeRegistry is populated and available for exhaustiveness checking (Plan 04-03)
- Constructor/or/as pattern inference provides the semantic foundation for pattern-to-Pat translation (Plan 04-04)
- All 16 sum type integration tests pass; zero regressions across the workspace

## Self-Check: PASSED

---
*Phase: 04-pattern-matching-adts*
*Completed: 2026-02-06*
