---
phase: 77-from-into-conversion
plan: 03
subsystem: compiler
tags: [from-trait, result-type, struct-error, try-operator, codegen, gap-closure]

requires:
  - phase: 77-02
    provides: "From/Into codegen pipeline, ? operator From-aware type checking, monomorphized Result name parsing"
provides:
  - "Struct error types in Result Err variants work correctly at runtime"
  - "MirType::Struct normalized to Ptr in From conversion for Result variant layout"
  - "Struct-to-ptr coercion in codegen_call for user functions returning structs when MIR expects Ptr"
  - "E2E test proving From<String> for AppError with ? operator works end-to-end"
affects: ["error-handling", "result-type", "from-into"]

tech-stack:
  added: []
  patterns:
    - "Struct-to-Ptr normalization: struct error types in From conversion normalized to Ptr to match Result { i8, ptr } layout"
    - "Struct-to-ptr coercion: GC-allocate struct return values when codegen expects pointer type"

key-files:
  created:
    - tests/e2e/from_try_struct_error.mpl
  modified:
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/mesh-codegen/src/codegen/expr.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "Normalize MirType::Struct to MirType::Ptr in lower_try_result From conversion -- struct constructors return heap-allocated pointers matching Result { i8, ptr } layout"
  - "GC-allocate struct return values in codegen_call when MIR type is Ptr but LLVM function returns struct by value"
  - "Coercion added in both user-function and runtime-intrinsic call paths for completeness"

patterns-established:
  - "Struct-to-Ptr coercion pattern: when user function returns struct but MIR expects Ptr, GC-alloc and box the struct value"

duration: 12min
completed: 2026-02-14
---

# Phase 77 Plan 03: Gap Closure - Struct Error Types in Result Summary

**Fixed Result<T, E> monomorphization for struct error types, enabling ? operator to auto-convert String errors to AppError struct via From<String> with GC-allocated struct boxing**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-14T04:09:11Z
- **Completed:** 2026-02-14T04:21:48Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Fixed the ? operator to auto-convert error types when the target is a struct (e.g., `From<String> for AppError`)
- Two-layer fix: MIR normalization (Struct -> Ptr for Result variant layout) + codegen coercion (struct-to-ptr via GC alloc)
- Phase 77 success criterion #4 now fully verified with working E2E test
- All 138 E2E tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix Result variant layout for struct error types** - `6c48c111` (fix)
2. **Task 2: Add E2E test for struct error conversion via ? operator** - `f2182484` (test)

## Files Created/Modified
- `crates/mesh-codegen/src/mir/lower.rs` - Normalize MirType::Struct to MirType::Ptr in lower_try_result when From conversion targets a struct error type
- `crates/mesh-codegen/src/codegen/expr.rs` - Add struct-to-ptr coercion in codegen_call: GC-allocate struct value when MIR expects Ptr, for both user-function and runtime-intrinsic paths
- `tests/e2e/from_try_struct_error.mpl` - E2E test: `impl From<String> for AppError`, `risky()` returns `Int!String`, `process()` returns `Int!AppError`, `risky()?` auto-converts via From
- `crates/meshc/tests/e2e.rs` - Added `e2e_from_try_struct_error` test function, clarifying doc comment on `e2e_from_try_error_conversion`

## Decisions Made
- **Two-layer fix approach:** The MIR lowering normalizes struct types to Ptr (matching Result's generic layout), and the codegen adds struct-to-ptr coercion (GC-allocating the struct value and storing a pointer). Neither layer alone was sufficient -- the MIR fix tells the codegen what type to expect, and the codegen fix handles the actual LLVM type mismatch at runtime.
- **GC allocation for struct boxing:** Used `mesh_gc_alloc_actor` (not stack alloca) because the pointer stored in the Result Err variant may outlive the current stack frame when returned from a function.
- **Coercion in user-function call path:** The struct-to-ptr coercion was added specifically in the `self.functions.get(name)` path of `codegen_call`, which is the path for user-defined functions (like From implementations). Also added in the runtime intrinsic path for completeness.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Added struct-to-ptr coercion in codegen_call for user functions**
- **Found during:** Task 2 (E2E test initially failed with segfault)
- **Issue:** The plan's MIR fix (Struct -> Ptr normalization) was necessary but not sufficient. The user-defined From function's LLVM signature returns the struct by value (`%AppError`), but the MIR type says `Ptr`. The codegen stored the struct value directly into the ptr slot of the Result variant, causing a type mismatch and segfault.
- **Fix:** Added struct-to-ptr coercion in `codegen_call`: when MIR type is `Ptr` but call result is `StructValue`, GC-allocate space, store the struct, and return the pointer.
- **Files modified:** crates/mesh-codegen/src/codegen/expr.rs
- **Verification:** E2E test passes, binary outputs "something failed", all 138 tests pass
- **Committed in:** f2182484 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** The plan correctly identified the MIR normalization fix but missed the codegen-level coercion needed because user-defined functions return struct values by value (not as pointers). The codegen fix is a natural extension of the MIR fix.

## Issues Encountered
- The plan assumed struct constructors "already return pointers at LLVM level via mesh_gc_alloc", but user-defined From functions actually construct struct literals on the stack and return them by value. The GC allocation needed to happen at the call site, not in the From function itself.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 77 (From/Into Conversion) is now fully complete: all 4 success criteria verified
- From trait infrastructure, codegen pipeline, and struct error conversion all working end-to-end
- Ready for Phase 78 if planned

## Self-Check: PASSED

All created files verified present, all task commits verified in git log.

---
*Phase: 77-from-into-conversion*
*Completed: 2026-02-14*
