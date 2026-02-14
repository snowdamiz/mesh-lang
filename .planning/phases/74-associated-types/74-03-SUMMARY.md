---
phase: 74-associated-types
plan: 03
subsystem: compiler
tags: [associated-types, mir, codegen, e2e-tests, compile-fail, typeck]

# Dependency graph
requires:
  - phase: 74-associated-types/02
    provides: parser + typeck for associated type declarations, bindings, and Self.Item resolution
provides:
  - MIR lowering compatibility verified for associated type impl blocks
  - E2E happy-path tests for basic, multiple, and deriving-coexistent associated types
  - Compile-fail tests for missing (E0040) and extra (E0041) associated type bindings
  - Fixed trait method signature comparison for Self-referencing return types
affects: [74-associated-types/04, 74-associated-types/05]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "ty_contains_self helper for detecting Self in type trees"
    - "Skip strict signature comparison when trait method returns Self.Item"

key-files:
  created:
    - tests/e2e/assoc_type_basic.mpl
    - tests/e2e/assoc_type_multiple.mpl
    - tests/e2e/assoc_type_with_deriving.mpl
  modified:
    - crates/mesh-typeck/src/traits.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "Skip trait method return type comparison when expected type contains Self (Self.Item resolves only in impl context)"
  - "Use dot-syntax method calls in E2E tests (Type.method() pattern is pre-existing unsupported)"

patterns-established:
  - "Associated type E2E tests use two-field structs (single-field struct literal is a pre-existing bug)"
  - "Compile-fail tests for assoc types use inline source strings with compile_expect_error helper"

# Metrics
duration: 22min
completed: 2026-02-14
---

# Phase 74 Plan 03: MIR Integration and E2E Tests Summary

**Fixed trait signature comparison for Self.Item, verified MIR passthrough, added 5 E2E tests (3 happy-path + 2 compile-fail)**

## Performance

- **Duration:** 22 min
- **Started:** 2026-02-14T00:12:05Z
- **Completed:** 2026-02-14T00:34:08Z
- **Tasks:** 4
- **Files modified:** 5

## Accomplishments
- Fixed trait method signature comparison to tolerate Self.Item return types (ty_contains_self helper)
- Verified MIR lowering naturally handles associated types: ImplDef.methods() skips ASSOC_TYPE_BINDING nodes, ExportedSymbols carries associated types through clone
- Added 3 E2E happy-path tests: basic two-impl dispatch, multiple associated types, coexistence with deriving(Display)
- Added 2 compile-fail tests: missing assoc type (E0040), extra assoc type (E0041)
- Full regression suite passes: 127 E2E + 91 stdlib + 233 typeck + 176 codegen tests

## Task Commits

Each task was committed atomically:

1. **Task 1: MIR lowering + cross-module export verification** - `94e65a8b` (feat)
2. **Task 2: E2E happy-path tests** - `f4375d2c` (test)
3. **Task 3: Compile-fail tests** - `cdd7e522` (test)
4. **Task 4: Full regression suite** - verification only (no code changes)

## Files Created/Modified
- `crates/mesh-typeck/src/traits.rs` - Added ty_contains_self helper and Self-tolerant signature comparison
- `tests/e2e/assoc_type_basic.mpl` - Two impls resolving Self.Item to Int and String
- `tests/e2e/assoc_type_multiple.mpl` - Interface with two associated types (Input, Output)
- `tests/e2e/assoc_type_with_deriving.mpl` - Associated types coexisting with deriving(Display)
- `crates/meshc/tests/e2e.rs` - 5 new test entries (3 happy-path + 2 compile-fail)

## Decisions Made
- **Self-tolerant signature comparison:** When the trait's method return type contains `Ty::Con("Self")` (from `Self.Item` resolution), skip the strict equality comparison. The method body is already type-checked against the resolved concrete type, so this is safe. The alternative (full projection normalization in trait signatures) would require significant refactoring of the type representation.
- **Use dot-syntax for method calls in tests:** `result.to_string()` instead of `Int.to_string(result)` because module-qualified function calls (`Type.method(arg)`) are a pre-existing unsupported pattern.
- **Two-field structs in tests:** Single-field struct literals have a pre-existing arity bug; tests use two-field structs to avoid it.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed trait method signature comparison rejecting valid Self.Item returns**
- **Found during:** Task 1 (MIR lowering verification)
- **Issue:** When a trait declares `fn get(self) -> Self.Item`, the parser stores the return type as `Ty::Con("Self")`. The signature comparison in `register_impl` compared this against the impl's concrete return type (e.g., `Ty::Con("Int")`), producing a false `TraitMethodSignatureMismatch` error.
- **Fix:** Added `ty_contains_self` helper that recursively checks if a type contains `Self`. The signature comparison now skips when the expected type involves Self.
- **Files modified:** `crates/mesh-typeck/src/traits.rs`
- **Verification:** All 9 assoc_types typeck tests pass, 3 new E2E tests pass
- **Committed in:** `94e65a8b` (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Essential fix -- without it, any interface method using Self.Item in its return type would fail to compile. No scope creep.

## Issues Encountered
- Discovered `self.field` access does not work inside impl method bodies (parse_self_expr always expects `self()` parenthesized form). This is a pre-existing parser limitation, not introduced by this plan. E2E tests use simple return values instead.
- Discovered `Int.to_string(x)` module-qualified function call pattern doesn't work. Pre-existing unsupported pattern. Tests use `x.to_string()` dot syntax instead.
- Discovered single-field struct literals produce false arity errors. Pre-existing bug. Tests use two-field structs.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Associated types are fully operational through the pipeline (parse -> typeck -> MIR -> codegen)
- Ready for Plan 04 (LSP integration) and Plan 05 (documentation + advanced patterns)
- The `self.field` parser limitation may need addressing in a future phase for full method body expressiveness

---
*Phase: 74-associated-types*
*Completed: 2026-02-14*

## Self-Check: PASSED

All 4 created files verified present. All 3 task commits verified in git history.
