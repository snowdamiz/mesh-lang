---
phase: 77-from-into-conversion
plan: 01
subsystem: typeck
tags: [traits, from-into, parameterized-traits, synthetic-impl, conversion]

# Dependency graph
requires:
  - phase: 74-associated-types
    provides: "Trait registry with associated types, ImplDef struct, register_impl validation"
provides:
  - "trait_type_args field on ImplDef for parameterized trait support"
  - "find_impl_with_type_args and has_impl_with_type_args for parameterized lookup"
  - "Synthetic Into generation from From registrations"
  - "From<T> and Into<T> trait definitions registered as compiler-known"
  - "Built-in From impls: Int->Float, Int->String, Float->String, Bool->String"
  - "GENERIC_ARG_LIST extraction in infer_impl_def for user-written impl blocks"
  - "Float.from and String.from stdlib module entries"
affects: [77-02, codegen-from-dispatch, mir-lowering]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Parameterized trait storage via trait_type_args: Vec<Ty> on ImplDef"
    - "Synthetic impl generation (From -> Into) in register_impl post-hook"
    - "Polymorphic stdlib module entry with TyVar for String.from(T)"

key-files:
  modified:
    - "crates/mesh-typeck/src/traits.rs"
    - "crates/mesh-typeck/src/builtins.rs"
    - "crates/mesh-typeck/src/infer.rs"

key-decisions:
  - "Synthetic Into generation inserts directly into impls HashMap to avoid infinite recursion (no re-entry to register_impl)"
  - "Duplicate detection compares trait_type_args via unification -- From<Int> and From<Float> for same type are distinct"
  - "String.from uses polymorphic TyVar(91100) input to accept Int/Float/Bool without overloading"
  - "Float.from is monomorphic (only accepts Int) since that is the only built-in Float conversion"

patterns-established:
  - "Parameterized trait pattern: trait_type_args on ImplDef enables From<Int> vs From<Float> distinction"
  - "Synthetic impl generation: register_impl auto-creates Into when From is registered"

# Metrics
duration: 14min
completed: 2026-02-14
---

# Phase 77 Plan 01: From/Into Trait Infrastructure Summary

**Parameterized trait support with trait_type_args on ImplDef, From/Into trait registration, 4 built-in primitive From impls, synthetic Into generation, and GENERIC_ARG_LIST extraction in infer_impl_def**

## Performance

- **Duration:** 14 min
- **Started:** 2026-02-14T03:05:18Z
- **Completed:** 2026-02-14T03:19:36Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Extended TraitRegistry with parameterized trait support (trait_type_args on ImplDef, find_impl_with_type_args, has_impl_with_type_args)
- Registered From<T> and Into<T> as compiler-known traits with 4 built-in From impls and automatic synthetic Into generation
- Wired GENERIC_ARG_LIST extraction in infer_impl_def so user-written `impl From<X> for Y` blocks correctly store trait type arguments
- Added Float.from and String.from to stdlib modules for type-checking static conversion calls

## Task Commits

Each task was committed atomically:

1. **Task 1: Extend TraitRegistry with parameterized trait support** - `aafa360c` (feat)
2. **Task 2: Register From/Into traits and built-in impls, wire type checking** - `b5a74ae5` (feat)

## Files Created/Modified
- `crates/mesh-typeck/src/traits.rs` - Added trait_type_args field to ImplDef, find_impl_with_type_args/has_impl_with_type_args methods, synthetic Into generation in register_impl, updated duplicate detection for parameterized traits
- `crates/mesh-typeck/src/builtins.rs` - Registered From<T> and Into<T> trait definitions, 4 built-in From impls (Int->Float, Int->String, Float->String, Bool->String), added trait_type_args: vec![] to all existing ImplDef constructions
- `crates/mesh-typeck/src/infer.rs` - Added GENERIC_ARG_LIST extraction in infer_impl_def, added Float.from and polymorphic String.from to stdlib modules, added trait_type_args: vec![] to all existing TraitImplDef constructions

## Decisions Made
- Synthetic Into generation inserts directly into the impls HashMap rather than calling register_impl recursively, avoiding infinite recursion when a From registration triggers Into generation which could trigger From generation
- String.from uses a polymorphic type variable (TyVar 91100) to accept Int, Float, or Bool arguments, since Mesh does not support method overloading and a single module entry must handle all source types
- Float.from is monomorphic (Int -> Float only) since Int-to-Float is the only built-in Float conversion
- Duplicate detection for parameterized traits compares trait_type_args via unification in addition to impl_type, so From<Int> for String and From<Float> for String are correctly treated as distinct impls

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added trait_type_args to all ImplDef sites in builtins.rs and infer.rs during Task 1**
- **Found during:** Task 1 (Extend TraitRegistry)
- **Issue:** Plan assigned this to Task 2, but adding the field to ImplDef in Task 1 made all existing ImplDef constructions across all files fail to compile
- **Fix:** Added trait_type_args: vec![] to all ImplDef constructions in builtins.rs and infer.rs as part of Task 1 to maintain compilability
- **Files modified:** crates/mesh-typeck/src/builtins.rs, crates/mesh-typeck/src/infer.rs
- **Verification:** cargo test -p mesh-typeck passed after fix
- **Committed in:** aafa360c (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Necessary to maintain compile-time correctness. Task 2 still added its new registrations and behavior changes separately.

## Issues Encountered
- Python script used for mechanical trait_type_args insertion was too aggressive, inserting the field into TypeError variants and a function signature that also had `trait_name` fields. Fixed by manually removing the false positives.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Trait registry fully supports parameterized traits; ready for Plan 02 (MIR lowering, codegen dispatch, ? operator error conversion)
- From/Into trait definitions and built-in impls are registered; codegen needs dispatch mapping (From_Int__from__Float -> sitofp, etc.)
- infer_impl_def correctly extracts trait type args from user-written impl blocks

---
*Phase: 77-from-into-conversion*
*Completed: 2026-02-14*

## Self-Check: PASSED

All files verified present, all commits verified in git log.
