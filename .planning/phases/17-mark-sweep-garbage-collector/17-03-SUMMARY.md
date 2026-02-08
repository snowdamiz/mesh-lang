# Phase 17 Plan 03: Allocation Migration Summary

**All runtime and codegen allocations migrated from snow_gc_alloc to snow_gc_alloc_actor for per-actor GC-managed heap allocation with GcHeader tracking**

---
phase: 17-mark-sweep-garbage-collector
plan: 03
subsystem: runtime-allocation
tags: [gc, allocation, actor-heap, migration]

dependency-graph:
  requires: [17-01]
  provides: [actor-aware-allocation-everywhere]
  affects: [17-04]

tech-stack:
  added: []
  patterns: [actor-aware-allocation]

key-files:
  created: []
  modified:
    - crates/snow-rt/src/string.rs
    - crates/snow-rt/src/collections/list.rs
    - crates/snow-rt/src/collections/map.rs
    - crates/snow-rt/src/collections/set.rs
    - crates/snow-rt/src/collections/queue.rs
    - crates/snow-rt/src/collections/range.rs
    - crates/snow-rt/src/io.rs
    - crates/snow-rt/src/file.rs
    - crates/snow-rt/src/env.rs
    - crates/snow-rt/src/json.rs
    - crates/snow-rt/src/http/server.rs
    - crates/snow-rt/src/http/client.rs
    - crates/snow-rt/src/actor/job.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/mir/lower.rs

decisions:
  - id: test-code-keeps-snow-gc-alloc
    title: Test code retains snow_gc_alloc
    rationale: Tests run outside actor context, so snow_gc_alloc (global arena) is correct; snow_gc_alloc_actor would also work (it falls back) but keeping snow_gc_alloc is more explicit

metrics:
  duration: 5min
  completed: 2026-02-08
---

## Accomplishments

- Migrated all 13 runtime source files from `snow_gc_alloc` to `snow_gc_alloc_actor`
- Updated codegen intrinsic declaration from `snow_gc_alloc` to `snow_gc_alloc_actor`
- Updated all 4 codegen call sites in `expr.rs` to emit `snow_gc_alloc_actor`
- Updated comment in `lower.rs` to reference `snow_gc_alloc_actor`
- Verified zero production code references to `snow_gc_alloc` outside gc.rs definition and test code

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Migrate runtime modules | f6e8033 | 13 runtime files: import + call site changes |
| 2 | Update codegen emission | b849360 | intrinsics.rs declaration, expr.rs 4 call sites, lower.rs comment |

## Verification Results

- `cargo test -p snow-rt`: 227 tests passed
- `cargo test -p snow-codegen`: 85 tests passed
- `cargo test -p snowc --test e2e`: 25 tests passed
- grep confirms zero production `snow_gc_alloc` references outside gc.rs and test code
- grep confirms zero `snow_gc_alloc` references in codegen crate

## What Changed

### Runtime (snow-rt)

Every runtime module that allocates heap memory now calls `snow_gc_alloc_actor` instead of `snow_gc_alloc`. The behavioral change:

- **In actor context**: Allocations go to the per-actor heap with a 16-byte GcHeader prepended. Objects are tracked in the actor's all-objects list, enabling mark-sweep collection.
- **Outside actor context**: `snow_gc_alloc_actor` falls back to `snow_gc_alloc` (global arena, no headers). Behavior is identical to before.

Files migrated: string.rs, list.rs, map.rs, set.rs, queue.rs, range.rs, io.rs, file.rs, env.rs, json.rs, http/server.rs, http/client.rs, actor/job.rs.

### Codegen (snow-codegen)

The LLVM IR emitted by the compiler now references `snow_gc_alloc_actor` instead of `snow_gc_alloc` for:
- Closure environment allocation (zero-capture sentinel and capture structs)
- Actor spawn argument buffer allocation
- Runtime tuple heap allocation

### Unchanged

- `tuple.rs` test helper keeps `snow_gc_alloc` (tests run outside actor context)
- `gc.rs` retains both functions (definition + fallback)
- `lib.rs` re-exports both functions

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Test code keeps snow_gc_alloc | Tests run outside actor context; explicit global arena usage is clearer |

## Deviations from Plan

None -- plan executed exactly as written.

## Next Phase Readiness

Plan 17-04 (sweep phase integration) can proceed. All allocations in actor context now produce GcHeader-tracked objects on the per-actor heap, making them visible to the mark-sweep collector.

## Self-Check: PASSED
