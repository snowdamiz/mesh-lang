---
phase: 30-core-method-resolution
plan: 01
subsystem: typechecker
tags: [type-inference, method-resolution, trait-registry, diagnostics]

# Dependency graph
requires:
  - phase: 22-auto-derive-stretch
    provides: TraitRegistry with resolve_trait_method and find_method_traits
provides:
  - NoSuchMethod error variant with E0030 code and diagnostic rendering
  - Method resolution fallback in infer_field_access (step 5 in resolution priority)
  - Method-call detection in infer_call with receiver prepending
  - build_method_fn_type helper for constructing method function types
  - find_method_sig accessor on TraitRegistry
affects: [30-02-PLAN, 31-mir-method-lowering, 32-e2e-integration]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Retry-based method resolution: infer_call tries normal inference first, falls back to method-call context on NoSuchField"
    - "Resolution priority chain: module > service > variant > struct field > method > error"
    - "is_method_call parameter threads call context through infer_field_access"

key-files:
  created: []
  modified:
    - crates/snow-typeck/src/error.rs
    - crates/snow-typeck/src/diagnostics.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/traits.rs

key-decisions:
  - "Retry-based method resolution: normal inference attempted first, method-call context used only when NoSuchField occurs"
  - "build_method_fn_type uses fresh type vars for non-self params since ImplMethodSig only stores param_count"
  - "find_method_sig added to TraitRegistry as public accessor rather than exposing private impls field"

patterns-established:
  - "Method resolution is step 5 in resolution priority, ensuring backward compatibility"
  - "is_method_call flag preserves existing NoSuchField behavior for plain field access"

# Metrics
duration: 6min
completed: 2026-02-09
---

# Phase 30 Plan 01: Core Method Resolution Summary

**Method resolution via TraitRegistry in infer_field_access with NoSuchMethod diagnostics and retry-based detection in infer_call**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-09T03:29:01Z
- **Completed:** 2026-02-09T03:35:12Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- NoSuchMethod error variant with E0030 code and ariadne diagnostic rendering ("no method `x` on type `Y`" with help text)
- Method resolution fallback in infer_field_access as step 5 in the resolution priority chain, preserving module > service > variant > struct field ordering
- Method-call detection in infer_call using retry-based approach: tries normal inference first, falls back to method-call context when struct field lookup fails
- AmbiguousMethod error wired for multi-trait conflicts in method resolution path
- All 390 existing tests (233 typeck + 157 codegen) pass with 0 regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Add NoSuchMethod error variant and diagnostic rendering** - `754d23a` (feat)
2. **Task 2: Add method resolution fallback and method-call detection** - `fbdda27` (feat)

## Files Created/Modified
- `crates/snow-typeck/src/error.rs` - Added NoSuchMethod variant with ty, method_name, span fields and Display impl
- `crates/snow-typeck/src/diagnostics.rs` - Added E0030 error code, span extraction, and ariadne diagnostic rendering with help text
- `crates/snow-typeck/src/infer.rs` - Added is_method_call parameter to infer_field_access, method resolution fallback after struct field lookup, build_method_fn_type helper, and method-call detection in infer_call
- `crates/snow-typeck/src/traits.rs` - Added find_method_sig public accessor to TraitRegistry

## Decisions Made
- Used retry-based approach in infer_call rather than always-method-call approach. Normal inference is attempted first; only when it fails with NoSuchField on a FieldAccess callee does the method-call path activate. This preserves backward compatibility with variant constructors (Shape.Circle) and module-qualified access (String.length) which also use FieldAccess syntax.
- build_method_fn_type creates fresh type variables for non-self parameters because ImplMethodSig stores param_count (not individual param types). Unification in infer_call resolves these fresh vars against actual argument types.
- Added find_method_sig as a public method on TraitRegistry rather than making the private impls field pub(crate). This maintains TraitRegistry's encapsulation.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Retry-based method resolution instead of always-method-call**
- **Found during:** Task 2 (method-call detection in infer_call)
- **Issue:** The plan specified always intercepting FieldAccess callees with is_method_call=true, but this broke variant constructors like Shape.Circle(5.0) because the method-call path tried to infer a "receiver" for the type name base expression
- **Fix:** Changed to retry-based approach: normal inference first, method-call fallback only on NoSuchField error. This correctly handles variant constructors (which resolve through the early return paths) while still activating method resolution for actual method calls.
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** All 16 exhaustiveness integration tests pass (which exercise variant constructor patterns via FieldAccess)
- **Committed in:** fbdda27 (Task 2 commit)

**2. [Rule 3 - Blocking] Adapted build_method_fn_type for ImplMethodSig structure**
- **Found during:** Task 2 (build_method_fn_type implementation)
- **Issue:** Plan referenced method_sig.param_types which does not exist on ImplMethodSig; the struct only has param_count and return_type
- **Fix:** Used param_count to create fresh type variables for non-self parameters instead of looking up individual param types. Added find_method_sig accessor to TraitRegistry.
- **Files modified:** crates/snow-typeck/src/infer.rs, crates/snow-typeck/src/traits.rs
- **Verification:** cargo check passes, function types correctly constructed
- **Committed in:** fbdda27 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes necessary for correctness. The retry approach is architecturally cleaner than the plan's always-intercept approach. No scope creep.

## Issues Encountered
None beyond the deviations documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Type checker method resolution complete, ready for Plan 02 (end-to-end integration tests)
- MIR lowering (Phase 31) can build on the method-call detection pattern established here
- Non-struct types (primitives) return fresh vars from the normal path; Phase 31 will handle MIR lowering for those cases

## Self-Check: PASSED

All 5 files verified present. Both task commits (754d23a, fbdda27) verified in git log.

---
*Phase: 30-core-method-resolution*
*Completed: 2026-02-09*
