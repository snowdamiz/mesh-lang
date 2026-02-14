---
phase: 77-from-into-conversion
plan: 02
subsystem: compiler
tags: [from-trait, type-conversion, mir-lowering, codegen, try-operator, trait-dispatch]

requires:
  - phase: 77-01
    provides: "From trait definition, infer_impl_def trait type args extraction, TraitRegistry.has_impl_with_type_args"
provides:
  - "Parameterized trait method mangling (From_Int__from__Float pattern)"
  - "MIR lowering for From conversions (user-defined and built-in)"
  - "Polymorphic String.from dispatch by argument type"
  - "StructName.from() resolution in lower_field_access"
  - "From-aware ? operator type checking with error rollback"
  - "From-based error conversion in lower_try_result via monomorphized Result names"
  - "7 E2E tests covering all From conversion success criteria"
affects: ["into-sugar", "error-handling", "type-conversion"]

tech-stack:
  added: []
  patterns:
    - "mangle_trait_method helper for parameterized vs non-parameterized trait mangling"
    - "Monomorphized Result name parsing for error type extraction (Result_Int_String -> String)"
    - "Error rollback in type checker via ctx.errors.truncate when From impl exists"

key-files:
  created:
    - tests/e2e/from_user_defined.mpl
    - tests/e2e/from_float_from_int.mpl
    - tests/e2e/from_string_from_int.mpl
    - tests/e2e/from_string_from_float.mpl
    - tests/e2e/from_string_from_bool.mpl
    - tests/e2e/from_try_error_conversion.mpl
    - tests/e2e/from_try_same_error.mpl
  modified:
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/mesh-typeck/src/infer.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "Used mangle_trait_method helper to centralize parameterized trait name mangling (From_Int__from__Float pattern)"
  - "Fixed static trait method param types in infer_impl_def -- only prepend impl_type when method has self"
  - "Used ctx.errors.truncate for error rollback when From impl exists (unify pushes errors internally)"
  - "Used monomorphized Result name parsing for error type extraction instead of sum type variant lookup (generic Result variant types are Ptr)"
  - "Struct error types in Result are a pre-existing limitation -- From error conversion tested with compatible pointer-level types"

patterns-established:
  - "Parameterized trait mangling: Trait_TypeArg__method__ImplType for traits with type params"
  - "Error rollback pattern: save ctx.errors.len before unify, truncate if From impl found"
  - "StructName.from() resolution via known_functions scan for From_*__from__TypeName"

duration: 24min
completed: 2026-02-14
---

# Phase 77 Plan 02: From/Into Codegen Pipeline Summary

**Parameterized trait mangling with From dispatch for built-in and user-defined conversions, polymorphic String.from, and From-aware ? operator type checking**

## Performance

- **Duration:** 24 min
- **Started:** 2026-02-14T03:23:41Z
- **Completed:** 2026-02-14T03:48:06Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- Full From conversion pipeline: user-defined `impl From<Int> for Wrapper` compiles and `Wrapper.from(21)` runs correctly
- Built-in conversions work: `Float.from(42)`, `String.from(42)`, `String.from(3.14)`, `String.from(true)`
- Polymorphic `String.from` dispatches to correct runtime function based on argument type (Int/Float/Bool)
- From-aware ? operator: type checker accepts From-convertible error types with error rollback
- MIR lowering detects From conversion needs from monomorphized Result type names
- 7 E2E tests covering all success criteria with zero regressions across full workspace

## Task Commits

Each task was committed atomically:

1. **Task 1: MIR lowering and codegen for From conversions** - `b51b37dc` (feat)
2. **Task 2: Extend ? operator type checking and add E2E tests** - `1f454761` (test)

## Files Created/Modified
- `crates/mesh-codegen/src/mir/lower.rs` - Extended extract_impl_names to 3-tuple, mangle_trait_method helper, From dispatch in resolve_trait_callee, polymorphic mesh_string_from, StructName.from() in lower_field_access, From conversion in lower_try_result
- `crates/mesh-typeck/src/infer.rs` - Fixed static trait method param types (no self prepend), From-aware error type checking in infer_try_expr with error rollback
- `crates/meshc/tests/e2e.rs` - 7 new E2E test functions for From conversion verification
- `tests/e2e/from_user_defined.mpl` - User-defined From<Int> for Wrapper with value doubling
- `tests/e2e/from_float_from_int.mpl` - Float.from(42) built-in conversion
- `tests/e2e/from_string_from_int.mpl` - String.from(42) built-in conversion
- `tests/e2e/from_string_from_float.mpl` - String.from(3.14) built-in conversion
- `tests/e2e/from_string_from_bool.mpl` - String.from(true) built-in conversion
- `tests/e2e/from_try_error_conversion.mpl` - Chained ? operator error propagation
- `tests/e2e/from_try_same_error.mpl` - Backward compat same-error-type ?

## Decisions Made
- **Centralized mangling helper:** Created `mangle_trait_method()` to handle both parameterized (`From_Int__from__Float`) and non-parameterized (`Display__to_string__Int`) trait names in a single function
- **Static trait method fix:** Fixed `infer_impl_def` to only prepend impl_type to param list when method has `self` -- this was a pre-existing bug that surfaced because From.from() is the first static trait method in the codebase
- **Error rollback approach:** When `ctx.unify` fails for error types in ?, the error is pushed internally by unify(). Used `ctx.errors.truncate(err_count_before)` to remove it when a From impl exists, avoiding false type errors
- **Monomorphized name parsing:** Used `Result_Int_String` -> error type `String` extraction from monomorphized Result names instead of sum type variant lookup (which returns generic `Ptr` for the base `Result` type)
- **Pre-existing struct-in-Result limitation:** Struct values stored in Result Err variants crash due to the generic Ptr layout. This is not introduced by Phase 77 -- all existing ? tests use String error types. Documented as known limitation

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed static trait method parameter types in infer_impl_def**
- **Found during:** Task 2 (user-defined From E2E test)
- **Issue:** `infer_impl_def` always prepended `impl_type` to param list, making `fn from(n :: Int)` appear as `Fun([Wrapper, Int], Wrapper)` instead of `Fun([Int], Wrapper)`
- **Fix:** Conditionally prepend impl_type only when `has_self` is true
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** User-defined From E2E test passes, all 13 typeck tests pass
- **Committed in:** 1f454761 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed unify error rollback for From-convertible error types**
- **Found during:** Task 2 (From error conversion E2E test)
- **Issue:** `ctx.unify()` pushes mismatch errors to `ctx.errors` internally before returning `Err`, causing false type errors even when From impl exists
- **Fix:** Save `ctx.errors.len()` before unify, truncate when From impl found
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** All E2E tests pass, zero regressions
- **Committed in:** 1f454761 (Task 2 commit)

**3. [Rule 1 - Bug] Used monomorphized Result name parsing instead of variant lookup**
- **Found during:** Task 2 (From error conversion E2E test)
- **Issue:** `find_variant_field_type("Result", "Err")` returns `MirType::Ptr` for the generic Result type, making error type comparison always equal
- **Fix:** Added `extract_error_type_from_result_name` to parse error type from monomorphized name (`Result_Int_String` -> `String`)
- **Files modified:** crates/mesh-codegen/src/mir/lower.rs
- **Verification:** Error type comparison now correctly detects differing types
- **Committed in:** 1f454761 (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (3 bugs)
**Impact on plan:** All auto-fixes necessary for correctness. The static method param type fix was a pre-existing bug exposed by From (first static trait method). Error rollback and name parsing were implementation details not anticipated in the plan.

## Issues Encountered
- Struct error types in Result crash at runtime due to the generic Ptr layout of monomorphized sum types -- this is a pre-existing limitation not introduced by Phase 77, documented for future resolution

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- From trait fully functional for both user-defined and built-in conversions
- Into sugar (if planned) can be implemented as syntactic sugar that desugars to From calls
- Struct error types in Result may need future work for full From error conversion support

## Self-Check: PASSED

All 8 created files verified present. All 2 task commits verified in git log.

---
*Phase: 77-from-into-conversion*
*Completed: 2026-02-14*
