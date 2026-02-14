---
phase: 78-lazy-combinators-terminals
plan: 01
subsystem: runtime
tags: [iterator, lazy, adapter, combinator, terminal, generic-dispatch, type-tag]

# Dependency graph
requires:
  - phase: 76-iterator-protocol
    provides: ListIterator, MapIterator, SetIterator, RangeIterator handles with _new/_next functions; MeshOption; alloc_option; alloc_pair
provides:
  - Type-tagged iterator handles for uniform dispatch (ITER_TAG_LIST through ITER_TAG_ZIP_ADAPTER)
  - mesh_iter_generic_next: dispatches by first-byte type tag to correct _next function
  - 6 lazy combinator adapters: MapAdapter, FilterAdapter, TakeAdapter, SkipAdapter, EnumerateAdapter, ZipAdapter
  - 6 terminal operations: mesh_iter_count, mesh_iter_sum, mesh_iter_any, mesh_iter_all, mesh_iter_find, mesh_iter_reduce
  - Closure calling pattern (BareFn/ClosureFn) reused from list.rs for adapter/terminal functions
affects: [78-02-PLAN, 78-03-PLAN, codegen-intrinsics, typeck-stdlib-modules]

# Tech tracking
tech-stack:
  added: []
  patterns: [type-tag-dispatch, lazy-adapter-chain, terminal-loop-consume]

key-files:
  created:
    - crates/mesh-rt/src/iter.rs
  modified:
    - crates/mesh-rt/src/collections/list.rs
    - crates/mesh-rt/src/collections/map.rs
    - crates/mesh-rt/src/collections/set.rs
    - crates/mesh-rt/src/collections/range.rs
    - crates/mesh-rt/src/lib.rs

key-decisions:
  - "Type-tag dispatch (Option A from research) chosen over function-pointer dispatch (Option C) for uniformity -- tag as first byte of all iterator handles"
  - "Combined Task 1 and Task 2 into single atomic implementation -- adapter _next functions called from generic_next dispatch, requiring all code to exist for compilation"
  - "Tag constants 0-3 for collection iterators, 10-15 for adapter iterators (gap allows future collection types at 4-9)"

patterns-established:
  - "Type-tag dispatch: all iterator handles have u8 tag as first repr(C) field; mesh_iter_generic_next matches on tag to call correct _next"
  - "Lazy adapter pattern: adapter struct stores source ptr + state; _next delegates to generic_next(source) and transforms result"
  - "Terminal loop pattern: loop calling generic_next until MeshOption tag==1 (None), accumulating result"

# Metrics
duration: 3min
completed: 2026-02-14
---

# Phase 78 Plan 01: Runtime Iterator Infrastructure Summary

**Type-tagged generic dispatch for all iterator handles, 6 lazy combinator adapters (map/filter/take/skip/enumerate/zip), and 6 terminal operations (count/sum/any/all/find/reduce) in mesh-rt**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-14T05:00:18Z
- **Completed:** 2026-02-14T05:03:51Z
- **Tasks:** 2 (consolidated into 1 commit)
- **Files modified:** 6

## Accomplishments
- Added type tag (u8) as first field to all 4 existing Phase 76 iterator handles (ListIterator, MapIterator, SetIterator, RangeIterator) enabling uniform dispatch
- Created `crates/mesh-rt/src/iter.rs` with 19 new extern "C" functions: generic dispatch, 6 combinator constructors, 6 combinator _next functions, 6 terminal operations
- All combinators are truly lazy -- no intermediate collection allocation; each _next delegates through generic dispatch to source iterator
- All 421 existing runtime tests pass with new struct layouts (type tag addition is backward-compatible for direct _next function calls)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add type tags to existing iterator handles and create generic dispatch** - `5952a315` (feat)
   - Includes Task 2 work (adapters + terminals) consolidated for compilation atomicity

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/mesh-rt/src/iter.rs` - All adapter structs, generic dispatch, terminal operations (new file, ~510 lines)
- `crates/mesh-rt/src/collections/list.rs` - Added tag: u8 to ListIterator, write tag=0 in mesh_list_iter_new
- `crates/mesh-rt/src/collections/map.rs` - Added tag: u8 to MapIterator, write tag=1 in mesh_map_iter_new
- `crates/mesh-rt/src/collections/set.rs` - Added tag: u8 to SetIterator, write tag=2 in mesh_set_iter_new
- `crates/mesh-rt/src/collections/range.rs` - Added tag: u8 to RangeIterator, write tag=3 in mesh_range_iter_new
- `crates/mesh-rt/src/lib.rs` - Added mod iter declaration

## Decisions Made
- **Type-tag dispatch over function-pointer dispatch:** Chose Option A from research (tag as first byte of all handles) over Option C (function pointer per adapter). Type tags are simpler, uniform, and avoid an extra pointer field per adapter. The modification to Phase 76 handles is mechanical (add one field + write in _new).
- **Tag numbering gap 4-9:** Collection iterator tags use 0-3, adapter tags use 10-15, leaving room for future collection types (e.g., TreeIterator=4) without renumbering.
- **Tasks consolidated:** Tasks 1 and 2 were implemented as a single atomic unit because generic_next dispatch calls adapter _next functions, requiring them to exist for compilation. No stubs needed.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Consolidated Task 1 and Task 2 into single implementation**
- **Found during:** Task 1 (creating iter.rs with generic dispatch)
- **Issue:** mesh_iter_generic_next dispatches to adapter _next functions (tags 10-15) which are planned for Task 2. Without the actual implementations (or stub functions), the code would not compile.
- **Fix:** Implemented all adapter structs and terminal operations in the same file creation, producing one atomic commit instead of two.
- **Files modified:** crates/mesh-rt/src/iter.rs
- **Verification:** cargo build -p mesh-rt and cargo test -p mesh-rt both pass cleanly
- **Committed in:** 5952a315

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** No scope creep. All planned functionality delivered. Task consolidation was necessary for compilation correctness.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All runtime C functions for lazy iterators are ready
- Phase 78 Plan 02 can wire these into the type checker (stdlib_modules), MIR lowerer (map_builtin_name), and codegen (intrinsic declarations)
- Phase 78 Plan 03 can add E2E tests exercising full pipeline from Mesh source to lazy execution

## Self-Check: PASSED

- FOUND: crates/mesh-rt/src/iter.rs
- FOUND: 78-01-SUMMARY.md
- FOUND: commit 5952a315

---
*Phase: 78-lazy-combinators-terminals*
*Completed: 2026-02-14*
