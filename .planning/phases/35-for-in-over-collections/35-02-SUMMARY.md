---
phase: 35-for-in-over-collections
plan: 02
subsystem: compiler
tags: [for-in, iteration, list, map, set, codegen, llvm, comprehension, list-builder]

# Dependency graph
requires:
  - phase: 35-for-in-over-collections
    provides: runtime list builder API, parser destructuring, typeck, MIR variants, intrinsic declarations
  - phase: 34-for-in-over-range
    provides: for-in range loop structure, four-block codegen pattern
  - phase: 08-collections
    provides: List/Map/Set runtime (snow-rt), collection intrinsics
provides:
  - LLVM codegen for ForInList with indexed iteration and list builder result
  - LLVM codegen for ForInMap with {k, v} destructuring and list builder result
  - LLVM codegen for ForInSet with indexed iteration and list builder result
  - ForInRange updated to return List<T> via list builder (comprehension semantics)
  - convert_from_list_element helper for typed element extraction from runtime u64
  - E2E tests proving list/map/set iteration, break, continue, empty collections
  - Formatter support for {k, v} destructure binding syntax
affects: [36-future-phases]

# Tech tracking
tech-stack:
  added: []
  patterns: [four-block-loop-with-list-builder, convert-from-list-element-pattern, result-alloca-for-break-return]

key-files:
  created:
    - tests/e2e/for_in_list.snow
    - tests/e2e/for_in_map.snow
    - tests/e2e/for_in_set.snow
  modified:
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snowc/tests/e2e.rs
    - crates/snow-fmt/src/walker.rs

key-decisions:
  - "Result alloca pattern: list builder pointer stored in alloca so break returns partial list without special handling"
  - "convert_from_list_element as inverse of convert_to_list_element for typed runtime value extraction"
  - "Unit values stored as 0 in list elements (convert_to_list_element handles MirType::Unit)"

patterns-established:
  - "Collection for-in codegen: four-block loop (header/body/latch/merge) with pre-allocated list builder and per-element push"
  - "Result alloca: break jumps to merge, merge loads result_alloca to get partially-built list"
  - "Continue skips push: continue jumps to latch before the list_builder_push call, effectively filtering elements"

# Metrics
duration: 17min
completed: 2026-02-09
---

# Phase 35 Plan 02: For-In Over Collections Codegen Summary

**LLVM four-block loop codegen for for-in over List/Map/Set with O(N) list builder comprehension and ForInRange comprehension upgrade**

## Performance

- **Duration:** 17 min
- **Started:** 2026-02-09T09:24:33Z
- **Completed:** 2026-02-09T09:41:35Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments
- Full LLVM codegen for ForInList, ForInMap, ForInSet using four-block loop pattern with pre-allocated list builder
- ForInRange updated from placeholder empty list to real comprehension semantics with list builder
- E2E tests proving iteration, comprehension, break partial list, continue element skip, empty collections
- Formatter support for `{k, v}` destructure binding with round-trip and idempotency tests

## Task Commits

Each task was committed atomically:

1. **Task 1: LLVM codegen for for-in over collections and range comprehension** - `51cef87` (feat)
2. **Task 2: E2E tests and formatter for for-in over collections** - `19a398f` (test)

## Files Created/Modified
- `crates/snow-codegen/src/codegen/expr.rs` - Added codegen_for_in_list/map/set, convert_from_list_element, updated ForInRange to use list builder
- `crates/snow-codegen/src/codegen/intrinsics.rs` - Added test assertions for 5 new runtime intrinsics
- `crates/snow-codegen/src/codegen/mod.rs` - Added unit tests for ForInRange returns list, ForInList basic blocks
- `tests/e2e/for_in_list.snow` - List comprehension, continue skip, break partial result
- `tests/e2e/for_in_map.snow` - Map destructuring iteration collecting values
- `tests/e2e/for_in_set.snow` - Set element iteration collecting into list
- `crates/snowc/tests/e2e.rs` - 6 new e2e tests (for_in_list, for_in_map, for_in_set, range comprehension, empty map, empty set)
- `crates/snow-fmt/src/walker.rs` - walk_destructure_binding handler, 2 new formatter tests

## Decisions Made
- **Result alloca pattern:** The list builder pointer is stored in an alloca so that when `break` jumps directly to the merge block, loading the alloca returns the partially-built list. No special break-handling code needed.
- **Unit handling in convert_to_list_element:** Added explicit MirType::Unit case that stores 0, fixing a crash when for-in body is println (which returns Unit struct that cannot be cast to i64).
- **convert_from_list_element:** Created as inverse of convert_to_list_element, handling Int (direct), Bool (truncate), Float (bitcast), pointer types (inttoptr), and Unit (const zero).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed convert_to_list_element crash on Unit body expressions**
- **Found during:** Task 1 (ForInRange codegen update)
- **Issue:** Existing e2e tests (for_in_range_basic etc.) use println as for-in body, which returns Unit (struct {}). The convert_to_list_element function tried to call .into_int_value() on a struct value, causing a panic.
- **Fix:** Added explicit MirType::Unit handler that returns i64 const 0 instead of trying to cast the struct value.
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** All 5 existing for_in_range e2e tests pass, all 1,300 workspace tests pass
- **Committed in:** 51cef87 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Necessary fix for type system consistency. The plan didn't account for Unit-typed body expressions in for-in comprehensions.

## Issues Encountered
- Empty list literal `[]` in for-in causes "Undefined variable 'to_string'" error in the compiler. This is a pre-existing type inference issue, not introduced by this plan. Workaround: use non-empty lists or List.new() for empty collection tests.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All for-in collection iteration codegen is complete and tested
- The for-in over collections feature is fully functional: parser, typeck, MIR, codegen, runtime
- Ready for phase 36 or any remaining plan 03 in phase 35
- 1,300 tests passing across the workspace (up from 1,273)

## Self-Check: PASSED

All 8 files verified present. Both task commits (51cef87, 19a398f) verified in git log. All 1,300 workspace tests pass (0 failures).

---
*Phase: 35-for-in-over-collections*
*Completed: 2026-02-09*
