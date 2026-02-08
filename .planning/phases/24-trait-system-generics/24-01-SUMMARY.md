---
phase: 24-trait-system-generics
plan: 01
subsystem: codegen
tags: [mir, display, callback, collection, nested, wrapper, llvm]

# Dependency graph
requires:
  - phase: 22-auto-derive-stretch
    provides: Display/Debug trait generation for structs and sum types
provides:
  - Recursive resolve_to_string_callback with synthetic wrapper function generation
  - LLVM codegen fix for runtime intrinsic function pointer references
  - Collection Display via string interpolation (now works end-to-end)
affects:
  - 24-02 (generic type deriving -- will enable List<List<Int>> e2e test)
  - Any future phase using collection Display or nested type rendering

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Synthetic MIR wrapper functions for bridging callback signatures (__display_{kind}_{type}_to_str)"
    - "Dedup wrapper generation via known_functions check before generation"
    - "module.get_function() fallback in codegen_var for intrinsic fn ptr references"

key-files:
  created:
    - tests/e2e/nested_collection_display.snow
  modified:
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "Generate synthetic MIR wrapper functions for nested collection callbacks instead of inline closures"
  - "Use known_functions dedup check to prevent duplicate wrapper generation"
  - "Fall back to Debug__inspect when Display__to_string is not available for sum type callbacks"
  - "Fix codegen_var to check module.get_function() for runtime intrinsics used as fn ptrs"

patterns-established:
  - "Synthetic wrapper pattern: __display_{collection}_{elem_type}_to_str bridges fn(u64)->ptr callback signature"
  - "Recursive callback resolution: resolve_to_string_callback calls itself for nested element types"

# Metrics
duration: 21min
completed: 2026-02-08
---

# Phase 24 Plan 01: Nested Collection Display Summary

**Recursive resolve_to_string_callback with synthetic MIR wrapper generation for nested collection Display, plus codegen fix for runtime intrinsic function pointer references**

## Performance

- **Duration:** 21 min
- **Started:** 2026-02-08T18:57:50Z
- **Completed:** 2026-02-08T19:19:48Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments

- Made resolve_to_string_callback recursive: handles Ty::App for nested List, Set, Map, and sum types
- Generated synthetic MIR wrapper functions (__display_list_Int_to_str etc.) that bridge the fn(u64)->ptr callback signature expected by the runtime
- Fixed pre-existing LLVM codegen bug: codegen_var now resolves runtime intrinsic functions (snow_int_to_string etc.) when used as function pointer arguments
- Collection Display via string interpolation now works end-to-end (was broken at LLVM level)
- Added e2e test proving flat list Display produces [10, 20, 30] at runtime
- Added MIR unit test verifying correct callback resolution for flat and nested types

## Task Commits

Each task was committed atomically:

1. **Task 1: Make resolve_to_string_callback recursive with synthetic wrapper generation** - `4508f64` (feat)
2. **Task 2: Add e2e tests for collection Display, fix codegen fn ptr resolution** - `c40db35` (feat)

**Plan metadata:** (pending)

## Files Created/Modified

- `crates/snow-codegen/src/mir/lower.rs` - Recursive resolve_to_string_callback, synthetic wrapper generation, mangle_ty_for_display helper, &self -> &mut self signature change, MIR unit test
- `crates/snow-codegen/src/codegen/expr.rs` - codegen_var fallback to module.get_function() for intrinsic fn ptr references
- `crates/snowc/tests/e2e.rs` - e2e_nested_collection_display test (flat list Display regression check)
- `tests/e2e/nested_collection_display.snow` - Snow fixture for list Display via string interpolation

## Decisions Made

- **Synthetic wrapper functions over inline closures:** The runtime expects `fn(u64) -> *mut u8` callbacks. For nested collections, the inner Display needs two arguments (the element + the inner callback). A synthetic named function bridges this gap, and can be deduplicated across multiple uses of the same nested type.
- **&self to &mut self propagation:** resolve_to_string_callback, wrap_collection_to_string, and wrap_to_string changed from &self to &mut self because wrapper generation needs to push to self.functions. All callers already had &mut self.
- **Debug__inspect fallback for sum types:** When resolving callbacks for sum type elements, falls back to Debug__inspect if Display__to_string is not available.
- **codegen_var intrinsic resolution:** Added module.get_function() check as fallback after self.functions check, before local variable lookup. This correctly resolves runtime intrinsics used as function pointer arguments.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed LLVM codegen: runtime intrinsics unresolvable as function pointers**
- **Found during:** Task 2 (e2e test for list Display)
- **Issue:** codegen_var only checked self.functions (MIR functions) and self.locals for variable resolution. Runtime intrinsics declared via declare_intrinsics were in the LLVM module but not in self.functions, causing "Undefined variable 'snow_int_to_string'" when used as a callback function pointer.
- **Fix:** Added module.get_function(name) fallback check in codegen_var, between the self.functions check and the self.locals check.
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** e2e test compiles and runs; all 1,202 tests pass
- **Committed in:** c40db35 (Task 2 commit)

**2. [Rule 1 - Bug] Fixture syntax correction: Snow has no [x,y] list literal syntax**
- **Found during:** Task 2 (writing e2e fixture)
- **Issue:** Plan specified `[[1, 2], [3, 4]]` syntax which does not exist in Snow. Lists are created via List.new()/List.append(). Additionally, List.append is typed as (List, Int) -> List, preventing List<List<Int>> creation.
- **Fix:** Rewrote fixture to use List.append API for flat list, documented that nested list e2e requires generic collection elements (TGEN-02).
- **Files modified:** tests/e2e/nested_collection_display.snow, crates/snowc/tests/e2e.rs
- **Verification:** e2e test passes with flat list Display
- **Committed in:** c40db35 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs)
**Impact on plan:** Bug fix #1 was essential for collection Display to work at all at the LLVM level. Bug fix #2 adapted the test to the actual language syntax. No scope creep. The MIR-level recursive callback resolution is correctly implemented and will be exercised end-to-end once the type system supports generic collection elements (Plan 02).

## Issues Encountered

- **List<List<Int>> cannot be created in Snow yet:** The type system types List.append as (List, Int) -> List, preventing nested list creation. The recursive callback MIR infrastructure is in place and tested at the unit level, but the full e2e test for `to_string([[1, 2], [3, 4]])` requires generic collection element types from Plan 02 (TGEN-02).
- **to_string is not a standalone function:** The plan referenced `to_string(expr)` as a function call, but in Snow it's a trait method resolved through Display. String interpolation `"${expr}"` works as an alternative and is the correct way to test collection Display.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- MIR infrastructure for recursive nested Display callbacks is complete
- LLVM codegen correctly handles intrinsic function pointers
- Ready for Plan 02: generic type deriving (TGEN-02)
- Plan 02 should extend List.append typing to support generic element types, enabling full nested collection Display e2e testing

## Self-Check: PASSED

---
*Phase: 24-trait-system-generics*
*Completed: 2026-02-08*
