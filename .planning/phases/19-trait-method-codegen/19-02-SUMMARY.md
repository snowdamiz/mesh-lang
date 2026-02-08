---
phase: 19-trait-method-codegen
plan: 02
subsystem: mir-lowering
tags: [mir, call-site-rewriting, operator-dispatch, trait-method-resolution, name-mangling]

# Dependency graph
requires:
  - phase: 19-trait-method-codegen
    plan: 01
    provides: ImplDef method lowering with mangled names and pre-registration in known_functions
  - phase: 18-trait-infrastructure
    provides: TraitRegistry with find_method_traits and has_impl for structural type matching
provides:
  - Call-site rewriting of trait method calls to mangled Trait__Method__Type names
  - Binary operator dispatch on user types through trait method calls
  - mir_type_to_ty helper for MirType to Ty reverse conversion
  - mir_type_to_impl_name helper for type name extraction in mangled names
affects: [19-03, 19-04, 20-eq-ord-protocols]

# Tech tracking
tech-stack:
  added: []
  patterns: [call-site-rewriting, operator-dispatch-via-traits, mir-type-reverse-mapping]

key-files:
  created: []
  modified:
    - crates/snow-codegen/src/mir/types.rs
    - crates/snow-codegen/src/mir/lower.rs

key-decisions:
  - "mir_type_to_ty as separate function in types.rs (not a method on MirType) for consistency with resolve_type"
  - "Use first matching trait when find_method_traits returns multiple (typeck already reported ambiguity)"
  - "Operator dispatch for Add/Sub/Mul/Eq/Lt only; other ops use BinOp (sufficient for v1.3 protocols)"
  - "fn_ty for operator dispatch constructed as FnPtr(lhs_ty, rhs_ty) -> result_ty"

patterns-established:
  - "Call-site rewriting: check known_functions first, then find_method_traits, then mangle"
  - "Operator dispatch: match BinOp to (trait_name, method_name), check has_impl, emit MirExpr::Call"
  - "Primitive operators unchanged: only Struct/SumType trigger trait-based dispatch"

# Metrics
duration: 3min
completed: 2026-02-08
---

# Phase 19 Plan 02: Call-Site Resolution Summary

**Trait method calls rewritten to mangled Trait__Method__Type names via TraitRegistry lookup, binary operators on user types dispatched through trait method calls instead of hardware BinOp.**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-08T06:47:45Z
- **Completed:** 2026-02-08T06:50:37Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments

- `mir_type_to_ty` converts MirType back to Ty for TraitRegistry lookups (Int -> Ty::int(), Struct("Point") -> Ty::Con(TyCon::new("Point")))
- `mir_type_to_impl_name` extracts type name strings for mangled name construction
- `lower_call_expr` detects bare method names not in known_functions, queries find_method_traits with first arg type, rewrites to mangled name which IS in known_functions for direct call dispatch
- `lower_binary_expr` checks if lhs is Struct/SumType, maps BinOp to trait (Add/Sub/Mul/Eq/Lt), checks has_impl, emits MirExpr::Call instead of BinOp
- Primitive operators (Int+Int, Float+Float) unchanged -- still produce BinOp for hardware ops
- 100 total snow-codegen tests pass (3 new + 97 existing, no regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add mir_type_to_ty and mir_type_to_impl_name helpers** - `0e1efd8` (feat)
2. **Task 2: Rewrite trait method calls and dispatch user-type binops** - `fb44219` (feat)

## Files Created/Modified

- `crates/snow-codegen/src/mir/types.rs` - Added mir_type_to_ty() and mir_type_to_impl_name() helpers with 11 unit tests
- `crates/snow-codegen/src/mir/lower.rs` - Modified lower_call_expr (trait method rewriting), lower_binary_expr (operator dispatch), added 3 integration tests

## Decisions Made

1. **mir_type_to_ty in types.rs:** Placed alongside resolve_type (the forward mapping) for symmetry. Uses Ty::Con(TyCon::new("Unknown")) as fallback for complex types since trait impls for tuples/closures are not expected in v1.3.
2. **First-match for ambiguous trait methods:** When find_method_traits returns multiple traits, we take the first. This is safe because typeck already reports ambiguity errors; the lowerer just needs to produce valid (if arbitrary) code for error recovery.
3. **Operator dispatch for 5 operators:** Add, Sub, Mul map to arithmetic trait calls; Eq, Lt map to comparison trait calls. Div, Mod, NotEq, Gt, GtEq, And, Or, Concat left as BinOp (sufficient for Phase 20 Eq/Ord protocols).
4. **FnPtr type for operator dispatch:** Constructed as MirType::FnPtr(vec![lhs_ty, rhs_ty], Box::new(result_ty)) to give the MirExpr::Call a proper function type for codegen.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

Phase 19-03 and 19-04 prerequisites are met:
- Trait method calls at call sites resolve to mangled names via TraitRegistry lookup
- Binary operators on user types dispatch through trait method calls
- mir_type_to_ty and mir_type_to_impl_name available for any future phase needing MirType-to-Ty conversion
- 100 codegen tests pass with no regressions
- CODEGEN-02 (call-site resolution) is complete

---
*Phase: 19-trait-method-codegen*
*Completed: 2026-02-08*

## Self-Check: PASSED
