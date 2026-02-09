---
phase: 31-extended-method-support
plan: 01
subsystem: compiler
tags: [typeck, method-resolution, dot-syntax, traits, stdlib-modules]

# Dependency graph
requires:
  - phase: 30-core-method-resolution
    provides: "Retry-based method resolution in infer_call, TraitRegistry dispatch, build_method_fn_type"
provides:
  - "Non-struct concrete type NoSuchField error in infer_field_access triggers retry for primitives/collections"
  - "Stdlib module method fallback in is_method_call=true path (String, List, Map, Set, Range)"
  - "Display trait registered for List<T>, Map<K,V>, Set in TraitRegistry"
affects: [31-02-PLAN, MIR-lowering, e2e-method-tests]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Concrete-type gating: Ty::Con/Ty::App -> Err(NoSuchField), Ty::Var -> Ok(fresh_var)"
    - "Type-to-module mapping for stdlib method fallback (receiver type determines module lookup)"

key-files:
  created: []
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/builtins.rs"

key-decisions:
  - "Return Err(NoSuchField) for all Ty::Con and Ty::App non-struct types, not just primitives"
  - "Stdlib module fallback maps receiver type to module name generically (String, List, Map, Set, Range)"
  - "Display registered for List<T>, Map<K,V>, Set -- covers all collection types uniformly"

patterns-established:
  - "Non-struct field access on concrete types always errors (NoSuchField), only Ty::Var gets fresh_var"
  - "Method resolution order: trait methods first, stdlib module methods second, NoSuchMethod last"

# Metrics
duration: 3min
completed: 2026-02-09
---

# Phase 31 Plan 01: Extended Method Support - Type Checker Summary

**Fixed non-struct method resolution gap (NoSuchField for concrete types) and added stdlib module method fallback plus Display impls for collections**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-09T04:25:26Z
- **Completed:** 2026-02-09T04:28:24Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Fixed the core method resolution gap: `infer_field_access` now returns `Err(NoSuchField)` for concrete non-struct types (`Ty::Con`, `Ty::App`), triggering the retry mechanism in `infer_call` for `42.to_string()`, `true.to_string()`, etc.
- Added stdlib module method fallback in the `is_method_call=true` path so `"hello".length()`, `my_list.length()`, and similar stdlib module function calls work via dot-syntax
- Registered Display trait impls for `List<T>`, `Map<K,V>`, and `Set` in `builtins.rs` so `my_list.to_string()` passes type checking

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix non-struct method resolution gap and add stdlib module method fallback** - `4c117c5` (feat)
2. **Task 2: Register Display impl for List<T>, Map<K,V>, Set in builtins** - `d29588a` (feat)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - Added concrete-type NoSuchField error at end of infer_field_access; added stdlib module method fallback in is_method_call=true path with type-to-module mapping
- `crates/snow-typeck/src/builtins.rs` - Registered Display trait impls for List<T>, Map<K,V>, Set after existing primitive Display registrations

## Decisions Made
- Return `Err(NoSuchField)` for ALL `Ty::Con` and `Ty::App` types not in the struct registry, not just the four primitive types. This is more correct and future-proof -- any concrete non-struct type should fail field access.
- Stdlib module fallback maps receiver types generically: `String -> "String"`, `List<T> -> "List"`, `Map<K,V> -> "Map"`, `Set -> "Set"`, `Range -> "Range"`. This enables dot-syntax for all existing stdlib module functions without per-function registration.
- Display for collections uses `impl_type_name: "List"` (not `"List<T>"`) matching the pattern established by Eq/Ord registration for List<T> at lines 682-725.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Type checker now correctly handles method dot-syntax for non-struct types (primitives, collections, stdlib module functions)
- Phase 31-02 (MIR lowering for stdlib module method fallback) can proceed -- the MIR lowering needs a corresponding stdlib module fallback in `resolve_trait_callee`
- All 233 typeck tests pass, full workspace compiles

## Self-Check: PASSED

- All modified files exist on disk
- All task commits verified in git history (4c117c5, d29588a)
- SUMMARY.md created at expected path

---
*Phase: 31-extended-method-support*
*Completed: 2026-02-09*
