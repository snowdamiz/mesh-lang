---
phase: 47-extended-collection-operations
plan: 02
subsystem: collections
tags: [map, set, merge, to_list, from_list, difference, runtime, codegen, conversions]

# Dependency graph
requires:
  - phase: 47-extended-collection-operations
    plan: 01
    provides: alloc_pair heap tuple helper, list builder, Tuple/Ptr type fixes
  - phase: 46-core-collection-operations
    provides: Map/Set baseline runtime + compiler pipeline
provides:
  - Map.merge, Map.to_list, Map.from_list runtime functions
  - Set.difference, Set.to_list, Set.from_list runtime functions
  - Full 4-layer compiler registration for all 6 ops
  - Bidirectional Map/Set <-> List conversion capability
affects: [47-03, future collection phases]

# Tech tracking
tech-stack:
  added: []
  patterns: [Map.to_list uses alloc_pair for tuple creation, Set/Map from_list reads list header layout directly]

key-files:
  created:
    - tests/e2e/stdlib_map_conversions.snow
    - tests/e2e/stdlib_set_conversions.snow
  modified:
    - crates/snow-rt/src/collections/map.rs
    - crates/snow-rt/src/collections/set.rs
    - crates/snow-rt/src/lib.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snowc/tests/e2e_stdlib.rs

key-decisions:
  - "Map.from_list defaults to KEY_TYPE_INT (0) since runtime cannot detect key type from list of tuples"
  - "No bare name mappings for to_list/from_list (shared name across Map and Set would collide)"
  - "Map.merge copies a then iterates b with snow_map_put for correct overwrite semantics"

patterns-established:
  - "alloc_pair reuse: Map.to_list creates tuples using same alloc_pair helper as List.zip/enumerate"
  - "List header direct access: from_list functions read list data via (ptr as *const u64).add(2) to skip len+cap"

# Metrics
duration: 5min
completed: 2026-02-10
---

# Phase 47 Plan 02: Map/Set Conversion Operations Summary

**6 Map/Set conversion functions (merge, to_list, from_list, difference) enabling bidirectional Map/Set <-> List transformations with full compiler pipeline and e2e tests**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-10T09:49:28Z
- **Completed:** 2026-02-10T09:54:11Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- Implemented 6 new Map/Set runtime functions: merge, to_list, from_list, difference, set_to_list, set_from_list
- Registered all operations across typeck module map, flat env, MIR name/known_functions, and LLVM intrinsics
- All 72 e2e tests pass including 2 new conversion tests, zero regressions
- Map.merge correctly overwrites duplicates; Set.from_list correctly deduplicates

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement 6 Map and Set runtime functions** - `136054b` (feat)
2. **Task 2: Register Map/Set ops across typeck/MIR/codegen and add e2e tests** - `f1955f7` (feat)

## Files Created/Modified
- `crates/snow-rt/src/collections/map.rs` - snow_map_merge, snow_map_to_list, snow_map_from_list
- `crates/snow-rt/src/collections/set.rs` - snow_set_difference, snow_set_to_list, snow_set_from_list
- `crates/snow-rt/src/lib.rs` - Re-exports for all 6 new functions
- `crates/snow-typeck/src/infer.rs` - Module map entries for merge/to_list/from_list on Map and Set
- `crates/snow-typeck/src/builtins.rs` - Flat env entries for map_merge through set_from_list
- `crates/snow-codegen/src/mir/lower.rs` - Name mappings, known_functions, bare name mappings for merge/difference
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM intrinsic declarations + test assertions
- `crates/snowc/tests/e2e_stdlib.rs` - 2 new e2e test functions
- `tests/e2e/stdlib_map_conversions.snow` - Map merge/to_list/from_list test
- `tests/e2e/stdlib_set_conversions.snow` - Set difference/to_list/from_list test

## Decisions Made
- Map.from_list defaults to KEY_TYPE_INT since the runtime cannot detect key types from a list of tuples (acceptable per research decision)
- No bare name mappings for `to_list` and `from_list` to avoid ambiguity between Map.to_list and Set.to_list
- Added bare name mappings for `merge` (-> snow_map_merge) and `difference` (-> snow_set_difference) since these are unique

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All 6 Map/Set conversion operations fully functional and tested
- Ready for Plan 03 (additional collection operations or phase completion)
- Bidirectional conversion between Map/Set and List types now complete (COLL-11, COLL-12)

## Self-Check: PASSED

All 10 key files verified present. Both task commits (136054b, f1955f7) verified in git log.

---
*Phase: 47-extended-collection-operations*
*Plan: 02*
*Completed: 2026-02-10*
