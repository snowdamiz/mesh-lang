---
phase: 76-iterator-protocol
plan: 01
subsystem: typeck, runtime
tags: [iterator, iterable, traits, associated-types, for-in, collections]

# Dependency graph
requires:
  - phase: 74-associated-types
    provides: "AssocTypeDef, resolve_associated_type, type Item/Iter resolution"
  - phase: 75-numeric-traits
    provides: "Output associated type pattern for arithmetic traits"
provides:
  - "Iterator trait with type Item and fn next(self)"
  - "Iterable trait with type Item, type Iter, and fn iter(self)"
  - "Built-in Iterable impls for List, Map, Set, Range"
  - "Iterator impls for ListIterator, MapIterator, SetIterator, RangeIterator"
  - "Runtime iterator handle functions (8 new extern C functions)"
  - "Iterable/Iterator fallback in infer_for_in for user-defined types"
affects: [76-02-iterator-codegen, 77-collect-from-into, 78-pipe-combinators]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Iterator handle pattern: GC-allocated struct with collection pointer + index + cached length"
    - "Two-trait iterator protocol: Iterable (collection -> iterator) + Iterator (stateful next)"
    - "Opaque iterator type names (ListIterator, etc.) for trait resolution and name mangling"

key-files:
  created: []
  modified:
    - "crates/mesh-typeck/src/builtins.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-rt/src/collections/list.rs"
    - "crates/mesh-rt/src/collections/map.rs"
    - "crates/mesh-rt/src/collections/set.rs"
    - "crates/mesh-rt/src/collections/range.rs"

key-decisions:
  - "Used opaque TyCon names (ListIterator, MapIterator, etc.) for iterator handle types -- maps to MirType::Ptr at MIR level"
  - "Map iterator yields GC-allocated pair tuples via alloc_pair, consistent with existing map_to_list pattern"
  - "Range iterator takes (start, end) directly rather than a Range pointer, matching range_new signature"
  - "Set uses untyped Ty::Con('Set') for Iterable impl since sets are monomorphic (Int elements only)"

patterns-established:
  - "Iterator handle pattern: #[repr(C)] struct with collection ptr + index + cached size, allocated via mesh_gc_alloc_actor"
  - "Two-trait protocol: Iterable for collections, Iterator for stateful cursor -- consistent with Rust's IntoIterator/Iterator"

# Metrics
duration: 6min
completed: 2026-02-14
---

# Phase 76 Plan 01: Iterator/Iterable Foundation Summary

**Iterator and Iterable trait definitions with associated types, runtime iterator handles for all 4 collection types, and type checker Iterable resolution in for-in expressions**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-14T02:09:08Z
- **Completed:** 2026-02-14T02:15:15Z
- **Tasks:** 3
- **Files modified:** 6

## Accomplishments
- Registered Iterator trait (type Item, fn next) and Iterable trait (type Item, type Iter, fn iter) as compiler-known traits
- Registered Iterable impls for List<T>, Map<K,V>, Set, and Range with correct associated type mappings
- Registered Iterator impls for ListIterator, MapIterator, SetIterator, RangeIterator with matching Item types
- Added 8 runtime extern "C" iterator functions: mesh_{list,map,set,range}_iter_{new,next}
- Extended infer_for_in to resolve Item types for Iterable/Iterator types before falling back to Int
- Verified zero regressions across all 19 existing for-in E2E tests and full workspace test suite

## Task Commits

Each task was committed atomically:

1. **Task 1: Register Iterator and Iterable traits + built-in impls** - `24b80954` (feat)
2. **Task 2: Add runtime iterator handle functions** - `54a2b337` (feat)
3. **Task 3: Extend infer_for_in with Iterable/Iterator fallback** - `dde61771` (feat)

## Files Created/Modified
- `crates/mesh-typeck/src/builtins.rs` - Iterator/Iterable trait defs + 8 built-in impls (Iterable for List/Map/Set/Range, Iterator for their handle types)
- `crates/mesh-typeck/src/infer.rs` - Iterable/Iterator fallback in infer_for_in CollectionType::Unknown arm
- `crates/mesh-rt/src/collections/list.rs` - ListIterator struct + mesh_list_iter_new/next
- `crates/mesh-rt/src/collections/map.rs` - MapIterator struct + mesh_map_iter_new/next (yields pair tuples)
- `crates/mesh-rt/src/collections/set.rs` - SetIterator struct + mesh_set_iter_new/next
- `crates/mesh-rt/src/collections/range.rs` - RangeIterator struct + mesh_range_iter_new/next

## Decisions Made
- Used opaque TyCon names (ListIterator, MapIterator, SetIterator, RangeIterator) as compiler-internal types for trait resolution and name mangling. These are not real user-visible types -- they exist only in the trait registry.
- Map iterator yields GC-allocated pair tuples via the existing `alloc_pair` helper, consistent with mesh_map_to_list.
- Range iterator constructor takes `(start: i64, end: i64)` directly rather than a Range pointer, since Range is just a thin wrapper over two i64 values.
- Set uses monomorphic `Ty::Con("Set")` for its Iterable impl since sets currently only support Int elements.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Iterator/Iterable traits and runtime handles are ready for Plan 02 (ForInIterator MIR node + codegen)
- Type checker correctly resolves Item types via Iterable/Iterator for user-defined types
- All existing for-in paths preserved unchanged (zero regressions confirmed)
- Ready for: ForInIterator MIR node, lower_for_in_iterator, codegen_for_in_iterator, Iter.from() entry point

## Self-Check: PASSED

All 6 modified files exist. All 3 task commits verified in git log. SUMMARY.md present.

---
*Phase: 76-iterator-protocol*
*Completed: 2026-02-14*
