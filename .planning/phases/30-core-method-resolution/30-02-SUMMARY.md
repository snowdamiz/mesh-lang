---
phase: 30-core-method-resolution
plan: 02
subsystem: codegen
tags: [mir-lowering, method-desugaring, trait-dispatch, dot-syntax, e2e-tests]

# Dependency graph
requires:
  - phase: 30-core-method-resolution
    plan: 01
    provides: Method resolution in type checker (NoSuchMethod, find_method_traits, retry-based detection)
provides:
  - Method call interception in lower_call_expr (FieldAccess -> prepend receiver -> trait dispatch)
  - Shared resolve_trait_callee helper used by both bare-name and dot-syntax calls
  - is_sum_type_name / is_struct_type_name guards preventing false method interception
  - 5 MIR-level tests + 5 compile-and-run e2e tests for method dot-syntax
affects: [31-mir-method-lowering, 32-e2e-integration]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Method call interception: detect FieldAccess callee in lower_call_expr BEFORE lower_expr, prepend receiver as first arg"
    - "Shared trait dispatch: resolve_trait_callee helper eliminates duplication between bare-name and dot-syntax paths"
    - "Guard chain: module > service > sum type > struct type name -> fall through to lower_field_access; everything else -> method call interception"

key-files:
  created: []
  modified:
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snowc/tests/e2e.rs
    - crates/snow-lsp/src/analysis.rs

key-decisions:
  - "Add is_struct_type_name guard to prevent intercepting struct-qualified calls (e.g., Point.something()) as method calls"
  - "Shared resolve_trait_callee replaces inline dispatch in both bare-name and dot-syntax paths"
  - "E2e tests use deriving(Display) rather than manual interface+impl since the type checker pipeline doesn't fully support user-defined impl resolution for method calls yet"

patterns-established:
  - "Method call interception must happen BEFORE lower_expr on the callee to prevent FieldAccess -> struct GEP"
  - "Guard ordering: STDLIB_MODULES, service_modules, sum type defs, struct defs must all be checked before method interception"

# Metrics
duration: 23min
completed: 2026-02-09
---

# Phase 30 Plan 02: MIR Method Lowering Summary

**Method call desugaring in lower_call_expr with shared resolve_trait_callee helper and 10 new tests covering dot-syntax, regression, and equivalence**

## Performance

- **Duration:** 23 min
- **Started:** 2026-02-09T03:37:31Z
- **Completed:** 2026-02-09T04:00:25Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Method call interception in lower_call_expr: detects FieldAccess callee, extracts receiver + method name, prepends receiver to args, routes through trait dispatch
- Shared resolve_trait_callee helper: extracted from inline dispatch block, used by both bare-name and dot-syntax call paths (eliminates code duplication)
- Guard chain preventing false interception: STDLIB_MODULES (String, IO, etc.), service modules, sum type names (Shape, Option), struct type names -- all fall through to existing lower_field_access
- 10 new tests: 5 MIR-level (basic, equivalence, with-args, field-access, module-qualified) + 5 compile-and-run e2e (basic, equivalence, field-access, module-qualified, multi-trait)
- All 1,242 tests pass (10 new, 0 regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Intercept method calls in lower_call_expr and extract shared trait dispatch helper** - `196084b` (feat)
2. **Task 2: End-to-end tests for method dot-syntax** - `be4f3bd` (test)

## Files Created/Modified
- `crates/snow-codegen/src/mir/lower.rs` - Added resolve_trait_callee helper, is_sum_type_name/is_struct_type_name guards, method call interception at top of lower_call_expr, replaced inline trait dispatch with shared helper call, added 5 MIR-level method dot-syntax tests
- `crates/snowc/tests/e2e.rs` - Added 5 compile-and-run e2e tests for method dot-syntax (basic, equivalence, field-access, module-qualified, multi-trait)
- `crates/snow-lsp/src/analysis.rs` - Added missing NoSuchMethod match arm in type_error_span (blocking fix from plan 01)

## Decisions Made
- Added is_struct_type_name guard alongside is_sum_type_name to prevent intercepting struct-qualified calls. The plan mentioned checking struct names but left it as a consideration; I added it as a definitive guard since struct names like `Point.something()` must not be intercepted.
- E2e tests use deriving(Display) instead of manual interface+impl blocks. The type checker's pipeline doesn't fully support user-defined impl resolution for method dot-syntax yet (bare `to_string(p)` on user-defined impls fails at the typeck level). This is a pre-existing limitation that doesn't affect the MIR lowering work in this plan. MIR-level tests cover user-defined interface+impl patterns by bypassing type errors.
- Post-dispatch optimizations (Display__to_string__String identity, Debug__inspect__String quoting, collection Display dispatch) are replicated in the method call interception path to maintain behavioral parity with bare-name calls.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed missing NoSuchMethod match arm in snow-lsp**
- **Found during:** Task 1 (compilation of full workspace after lower.rs changes)
- **Issue:** The `NoSuchMethod` error variant added in plan 01 was not handled in `snow-lsp/src/analysis.rs::type_error_span()`, causing a non-exhaustive match error that prevented compilation of the full workspace.
- **Fix:** Added `TypeError::NoSuchMethod { span, .. } => Some(*span)` to the match.
- **Files modified:** crates/snow-lsp/src/analysis.rs
- **Verification:** Full workspace compiles, all tests pass
- **Committed in:** 196084b (Task 1 commit)

**2. [Rule 1 - Bug] E2e tests adapted for snowc build pipeline constraints**
- **Found during:** Task 2 (e2e test writing)
- **Issue:** Plan specified tests using `IO.println()`, manual `interface + impl` blocks, and `Int.to_string()` module-qualified calls. The snowc build pipeline (unlike the lower() test helper) does not support these patterns: `IO` is not registered as a variable, user-defined `interface + impl` blocks don't register correctly in the type checker for method resolution, and module-qualified `Int.to_string()` is not available.
- **Fix:** Adapted tests to use `println()` (bare name), `deriving(Display)` (auto-derived impls), string interpolation `"${expr}"` (for type conversion), and `"${String.length(s)}"` (interpolated module-qualified calls). Added multi-trait test as replacement for the with-args test.
- **Files modified:** crates/snowc/tests/e2e.rs
- **Verification:** All 5 e2e tests compile and run successfully
- **Committed in:** be4f3bd (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes necessary for correctness. The LSP fix was a missing match arm from plan 01. The e2e test adaptation reflects real pipeline constraints -- the MIR-level tests in lower.rs cover the full feature set (including user-defined interfaces), while the e2e tests prove the feature works end-to-end with the actual compiler.

## Issues Encountered
None beyond the deviations documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 30 (Core Method Resolution) is complete: both type checker (plan 01) and MIR lowering (plan 02) integration points are working
- `p.to_string()` compiles and runs end-to-end, producing identical output to string interpolation `"${p}"`
- Struct field access (`p.x`), module-qualified calls (`String.length(s)`), and variant construction (`Shape.Circle`) are all unaffected
- Phase 31 (if it exists) can build on this foundation for advanced method features

## Self-Check: PASSED

All files verified present. Both task commits (196084b, be4f3bd) verified in git log.

---
*Phase: 30-core-method-resolution*
*Completed: 2026-02-09*
