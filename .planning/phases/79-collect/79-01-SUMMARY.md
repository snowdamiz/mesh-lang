---
phase: 79-collect
plan: 01
subsystem: runtime, compiler
tags: [iterator, collect, terminal, list, map, set, string, llvm, intrinsics]

# Dependency graph
requires:
  - phase: 78-lazy-combinators-terminals
    provides: "Type-tag dispatch, mesh_iter_generic_next, terminal operation pattern, adapter infrastructure"
provides:
  - "mesh_list_collect: materialize iterator into List via safe Vec intermediary"
  - "mesh_map_collect: materialize tuple iterator into Map via mesh_map_put loop"
  - "mesh_set_collect: materialize iterator into Set with deduplication"
  - "mesh_string_collect: materialize string iterator into concatenated String"
  - "List.collect, Map.collect, Set.collect, String.collect type signatures in stdlib"
  - "MIR name mappings and LLVM intrinsic declarations for all four collect functions"
affects: [79-02-PLAN, e2e-tests]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Collect terminal pattern: loop mesh_iter_generic_next until None, accumulate into target collection"
    - "Safe Vec intermediary for List.collect (avoids list builder bounds overflow)"

key-files:
  created: []
  modified:
    - "crates/mesh-rt/src/iter.rs"
    - "crates/mesh-rt/src/lib.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"

key-decisions:
  - "Used safe Rust Vec<u64> intermediary for mesh_list_collect instead of mesh_list_builder (builder has no bounds checking)"
  - "Map.collect defaults to integer keys via mesh_map_new() (string-keyed maps deferred to future extension)"

patterns-established:
  - "Collect terminal: loop generic_next -> accumulate -> return collection (same structure for all four types)"

# Metrics
duration: 11min
completed: 2026-02-14
---

# Phase 79 Plan 01: Collect Runtime Functions & Compiler Wiring Summary

**Four collect terminal operations (List/Map/Set/String) with full compiler pipeline wiring via safe Vec intermediary for List and direct accumulation for Map/Set/String**

## Performance

- **Duration:** 11 min
- **Started:** 2026-02-14T05:50:56Z
- **Completed:** 2026-02-14T06:02:32Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- Four runtime extern "C" collect functions that loop mesh_iter_generic_next and accumulate into target collections
- mesh_list_collect uses safe Rust Vec<u64> to avoid list builder buffer overflow on unknown-length iterators
- Full compiler pipeline wiring: type checker signatures, MIR name mappings, LLVM intrinsic declarations
- Unit tests for list collect (verify elements), map collect (verify size after enumerate), set collect (verify deduplication)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add four collect runtime functions to iter.rs** - `ee156fe2` (feat)
2. **Task 2: Wire collect functions through compiler pipeline** - `e942eb8e` (feat)

## Files Created/Modified
- `crates/mesh-rt/src/iter.rs` - Four collect extern "C" functions + unit tests
- `crates/mesh-rt/src/lib.rs` - Export mesh_list_collect, mesh_map_collect, mesh_set_collect, mesh_string_collect
- `crates/mesh-typeck/src/infer.rs` - collect type signatures in List, Map, Set, String stdlib modules
- `crates/mesh-codegen/src/mir/lower.rs` - list_collect/map_collect/set_collect/string_collect -> mesh_* name mappings
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - Four fn(ptr)->ptr LLVM extern declarations + test assertions

## Decisions Made
- Used safe Rust Vec<u64> intermediary for mesh_list_collect: collects all elements into a growable Vec, then builds the final GC-allocated list via mesh_list_from_array in one shot. This avoids mesh_list_builder_push which has NO bounds checking and would corrupt memory for unknown-length iterators.
- Map.collect defaults to integer keys via mesh_map_new(). String-keyed map collection deferred to future extension.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All four collect runtime functions are compiled and tested
- Compiler pipeline accepts List.collect(iter), Map.collect(iter), Set.collect(iter), String.collect(iter) syntax
- Ready for Plan 02 (E2E tests) to verify full end-to-end functionality

---
*Phase: 79-collect*
*Completed: 2026-02-14*
