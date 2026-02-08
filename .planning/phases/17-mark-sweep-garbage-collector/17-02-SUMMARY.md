---
phase: 17-mark-sweep-garbage-collector
plan: 02
subsystem: runtime
tags: [gc, mark-sweep, conservative-stack-scanning, tricolor-marking, sweep, free-list]

key-decisions:
  - "Conservative stack scanning treats every 8-byte-aligned word as potential pointer"
  - "Worklist (tricolor marking) uses Rust Vec on system heap, not GC heap"
  - "find_object_containing handles interior pointers and skips freed objects"
  - "Sweep rebuilds all-objects list in-place using prev-pointer technique"
  - "Stack base captured as thread-local at coroutine body start"
  - "GC triggers at yield points (reduction check == 0) when heap exceeds threshold"
  - "snow_gc_collect forces collection regardless of pressure threshold"

patterns-established:
  - "collect(stack_bottom, stack_top) as the main GC entry point"
  - "STACK_BASE thread-local for coroutine stack scanning bounds"
  - "try_trigger_gc() integrated into snow_reduction_check() yield path"

dependency-graph:
  requires: ["17-01"]
  provides: ["mark-sweep GC with conservative scanning", "automatic GC trigger at yield points", "explicit GC via snow_gc_collect"]
  affects: ["17-03", "17-04"]

tech-stack:
  added: []
  patterns: ["conservative stack scanning", "worklist-based tricolor marking", "cooperative GC at yield points"]

file-tracking:
  key-files:
    created: []
    modified:
      - crates/snow-rt/src/actor/heap.rs
      - crates/snow-rt/src/actor/mod.rs
      - crates/snow-rt/src/actor/process.rs
      - crates/snow-rt/src/actor/stack.rs
      - crates/snow-rt/src/gc.rs

metrics:
  duration: "4m 38s"
  completed: "2026-02-08"
  tests-added: 8
  tests-total: 227
---

# Phase 17 Plan 02: Mark-Sweep GC Algorithm Summary

**Conservative stack scanning + worklist-based tricolor marking + sweep phase with automatic trigger at actor yield points**

## Accomplishments

### Task 1: Mark-Sweep Collection in ActorHeap
- Implemented `collect(stack_bottom, stack_top)` as the main GC entry point with re-entrancy guard
- Implemented `mark_from_roots()` with two-phase marking:
  - Phase 1: Conservative stack scanning -- walks every 8-byte-aligned word from stack_top to stack_bottom, checking if each looks like a pointer into this heap's pages
  - Phase 2: Worklist-based transitive marking -- scans each marked object's body for further heap pointers, marking transitively (tricolor algorithm)
- Implemented `find_object_containing(ptr)` for interior pointer support -- a pointer anywhere within an object's body identifies that object as reachable, with quick page-range pre-check
- Implemented `sweep()` that walks all-objects list, frees unmarked objects to free list, clears mark bits on survivors, and updates total_allocated
- Added 8 comprehensive tests covering: unreachable freeing, reachable retention, transitive reachability, total_bytes reduction, re-entrancy guard, interior pointers, out-of-range pointers, and post-GC reuse

### Task 2: GC Trigger Wiring and snow_gc_collect
- Added `stack_base: *const u8` field to Process struct for stack scanning bounds
- Added `STACK_BASE` thread-local in stack.rs with getter/setter, captured at coroutine body start via `black_box` anchor
- Implemented `try_trigger_gc()` in actor/mod.rs -- checks heap pressure via `should_collect()`, captures current stack position as stack_top, calls `heap.collect()`
- Wired `try_trigger_gc()` into `snow_reduction_check()` at the yield point (when reductions hit 0)
- Replaced `snow_gc_collect` no-op stub with full implementation that forces GC on the current actor's heap regardless of pressure threshold

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Mark-sweep collection in ActorHeap | 22b6b2b | collect(), mark_from_roots(), find_object_containing(), sweep(), 8 tests |
| 2 | GC trigger + snow_gc_collect | 1a7736a | stack_base on Process, STACK_BASE thread-local, try_trigger_gc(), snow_gc_collect |

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Conservative stack scanning (no type info) | Snow has no type maps yet; treating every 8-byte word as potential pointer is safe (may retain some garbage, never loses live objects) |
| Worklist on system heap (Rust Vec) | Avoids re-entrancy: allocating the worklist on the GC heap would trigger more GC allocations during GC |
| Interior pointer support | Snow values may contain pointers into the middle of allocated objects (e.g., string slices); find_object_containing checks the full data range |
| Stack base as thread-local | Simpler than threading it through the scheduler; set once at coroutine start, read at GC time |
| GC at yield points only | Cooperative approach: GC runs when actor voluntarily yields, never interrupting other actors |

## Deviations from Plan

None -- plan executed exactly as written.

## Verification Results

1. `cargo test -p snow-rt` -- 227/227 tests pass
2. `cargo build -p snow-rt` -- no new warnings
3. Mark-sweep correctly identifies and frees unreachable objects in unit tests
4. GC trigger compiles and integrates with the existing reduction check mechanism

## Next Phase Readiness

Plan 02 provides the complete mark-sweep GC algorithm. Plan 03 (write barrier / generational hints) and Plan 04 (e2e integration test) can proceed. The key interfaces are:
- `ActorHeap::collect(stack_bottom, stack_top)` -- called automatically at yield points and explicitly via `snow_gc_collect`
- `STACK_BASE` thread-local -- provides stack scanning bounds
- Free list populated by sweep -- enables memory reuse via Plan 01's free-list allocator

## Self-Check: PASSED
