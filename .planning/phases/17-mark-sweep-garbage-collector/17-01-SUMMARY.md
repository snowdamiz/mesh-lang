---
phase: 17-mark-sweep-garbage-collector
plan: 01
subsystem: runtime
tags: [gc, mark-sweep, free-list, allocator, gc-header, actor-heap]

# Dependency graph
requires:
  - phase: 06-actor-model
    provides: Per-actor ActorHeap bump allocator, snow_gc_alloc_actor routing
provides:
  - GcHeader struct (16 bytes) with mark/free bits for mark-sweep GC
  - Free-list allocator that reuses freed blocks before bump-allocating
  - Intrusive all-objects linked list for sweep traversal
  - should_collect() GC pressure threshold check
  - snow_gc_collect stub entry point
affects: [17-02 mark phase, 17-03 sweep phase, 17-04 GC trigger integration]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "GcHeader prepended to every per-actor allocation (16-byte #[repr(C)] header)"
    - "Intrusive linked list via next pointer in GcHeader for all-objects tracking"
    - "Free-list with first-fit allocation strategy for memory reuse"
    - "Dual allocation paths: global arena (no headers) vs actor heap (with headers)"

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/actor/heap.rs"
    - "crates/snow-rt/src/gc.rs"

key-decisions:
  - "GcHeader is 16 bytes: u32 size + u8 flags + [u8;3] pad + *mut GcHeader next"
  - "next pointer serves dual purpose: all-objects list (when live) or free list (when freed)"
  - "Free-list uses first-fit strategy; larger blocks can satisfy smaller requests"
  - "total_bytes() now includes header overhead for accurate GC pressure tracking"
  - "Global arena remains completely unchanged -- no headers, no collection"
  - "Default GC threshold: 256 KiB per actor heap"

patterns-established:
  - "GcHeader::from_data_ptr / data_ptr round-trip for header <-> user pointer conversion"
  - "ActorHeap public API: all_objects_head(), free_list_head(), add_to_free_list(), set_all_objects_head() for sweep phase use"
  - "Alignment-correct bump allocation: user data pointer aligned, header placed 16 bytes before"

# Metrics
duration: 5min
completed: 2026-02-08
---

# Phase 17 Plan 01: GcHeader Free-List Allocator Summary

**16-byte GcHeader prepended to every actor heap allocation with intrusive all-objects list and first-fit free-list reuse**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-08T03:59:43Z
- **Completed:** 2026-02-08T04:04:36Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Replaced bump-only ActorHeap with GcHeader-aware free-list + bump hybrid allocator
- Every per-actor allocation now gets a 16-byte header with size, mark/free flags, and next pointer
- All live objects tracked in intrusive linked list enabling future sweep traversal
- Free-list allocation reuses freed blocks (first-fit) before bump-allocating new pages
- Global arena remains completely unchanged (no headers, no collection) per Research Pitfall 1
- Added snow_gc_collect no-op stub for Plan 02 implementation

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement GcHeader and free-list allocator in ActorHeap** - `f53e6cd` (feat)
2. **Task 2: Update gc.rs routing and ensure global arena unchanged** - `5e773df` (feat)

## Files Created/Modified
- `crates/snow-rt/src/actor/heap.rs` - GcHeader struct, free-list allocator, all-objects list, mark/sweep support methods, 14 tests
- `crates/snow-rt/src/gc.rs` - Updated docs, snow_gc_collect stub, tests verifying arena vs heap header behavior

## Decisions Made
- GcHeader `next` pointer serves dual purpose (all-objects vs free-list) rather than two separate pointers -- keeps header at 16 bytes
- Free-list reuse does not update total_allocated (memory was already counted when first bump-allocated)
- Larger free blocks satisfy smaller requests without splitting (size field retains original block size)
- User data alignment is computed first, then header is placed 16 bytes before -- ensures correct alignment for all powers of 2

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed alignment for alignments > 16 bytes**
- **Found during:** Task 1 (alignment test)
- **Issue:** Initial implementation aligned the header to max(8, align), but for align=32 this placed user data at header+16 which was not 32-byte aligned
- **Fix:** Changed to compute the aligned user data address first, then place header 16 bytes before it
- **Files modified:** crates/snow-rt/src/actor/heap.rs
- **Verification:** test_actor_heap_alignment passes for all alignments 1-64
- **Committed in:** f53e6cd (part of Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Bug fix necessary for correctness of alignment > 16. No scope creep.

## Issues Encountered
None -- all existing 216 tests passed after the changes, plus 3 new tests added.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- GcHeader and all-objects list are ready for Plan 02 (mark phase: conservative stack scanning + tricolor marking)
- Free list infrastructure is ready for Plan 03 (sweep phase: walk all-objects, free unmarked, build free list)
- should_collect() threshold check is ready for Plan 04 (GC trigger integration at reduction check points)
- No blockers or concerns

## Self-Check: PASSED

---
*Phase: 17-mark-sweep-garbage-collector*
*Completed: 2026-02-08*
