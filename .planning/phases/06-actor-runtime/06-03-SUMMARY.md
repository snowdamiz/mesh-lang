---
phase: 06-actor-runtime
plan: 03
subsystem: runtime
tags: [actor-heap, mailbox, message-passing, bump-allocator, fifo, send-receive]

dependency_graph:
  requires: ["06-01"]
  provides: ["per-actor-heap", "fifo-mailbox", "snow_actor_send", "snow_actor_receive", "message-deep-copy"]
  affects: ["06-04", "06-05", "06-06", "06-07"]

tech_stack:
  added: []
  patterns: ["per-actor-bump-allocator", "message-deep-copy-isolation", "cooperative-blocking-receive"]

file_tracking:
  key_files:
    created:
      - crates/snow-rt/src/actor/heap.rs
      - crates/snow-rt/src/actor/mailbox.rs
    modified:
      - crates/snow-rt/src/actor/process.rs
      - crates/snow-rt/src/actor/mod.rs
      - crates/snow-rt/src/actor/scheduler.rs
      - crates/snow-rt/src/gc.rs
      - crates/snow-rt/src/lib.rs

decisions:
  - id: "06-03-01"
    description: "Per-actor heap uses same bump allocation algorithm as global Arena in gc.rs"
    rationale: "Proven correct, simple, and consistent with existing allocator"
  - id: "06-03-02"
    description: "MessageBuffer stores raw bytes + u64 type_tag for pattern matching"
    rationale: "Simple tag scheme sufficient for Phase 6; compiler-generated tags come later"
  - id: "06-03-03"
    description: "Mailbox uses Mutex<VecDeque<Message>> for thread-safe FIFO"
    rationale: "Simple, correct, and sufficient for current workload; lock-free upgrade possible later"
  - id: "06-03-04"
    description: "Blocking receive yields to scheduler (sets Waiting state), woken by state transition"
    rationale: "Cooperative blocking avoids busy-polling; worker loop skips Waiting coroutines"
  - id: "06-03-05"
    description: "copy_msg_to_actor_heap uses [u64 type_tag, u64 data_len, u8... data] layout"
    rationale: "Fixed-width header enables efficient pointer arithmetic in compiled Snow code"
  - id: "06-03-06"
    description: "snow_gc_alloc_actor falls back to global arena when no actor context"
    rationale: "Backward compatibility for non-actor code paths (Phase 5 programs)"

metrics:
  duration: 7min
  completed: 2026-02-07
  tests_added: 19
  tests_total: 46
---

# Phase 6 Plan 03: Per-Actor Heaps and FIFO Message Passing Summary

Per-actor bump allocator heaps with FIFO mailboxes and blocking send/receive for Snow actors.

## Task Commits

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Per-actor heap and message deep-copy | 31b6dc2 | heap.rs, process.rs, gc.rs, mod.rs, lib.rs |
| 2 | FIFO mailbox and send/receive with scheduler blocking | 763e933 | mailbox.rs, mod.rs, process.rs, scheduler.rs, lib.rs |

## What Was Built

### ActorHeap (heap.rs)
Per-actor bump allocator replacing global arena for actor-allocated memory. Each actor gets its own heap with 64KB initial pages. Same allocation algorithm as the global Arena: bump pointer within pages, new page allocation when exhausted. `reset()` drops all pages for actor cleanup.

### MessageBuffer (heap.rs)
Serialized message representation for cross-heap copying. Contains raw bytes and a u64 type tag. `deep_copy_to_heap()` allocates in the target actor's heap and copies data, ensuring complete heap isolation between actors.

### Mailbox (mailbox.rs)
Thread-safe FIFO queue backed by `Mutex<VecDeque<Message>>`. Strict ordering: push appends to back, pop removes from front. All operations are O(1) amortized.

### snow_actor_send (mod.rs)
Extern "C" function that deep-copies message bytes into a MessageBuffer, pushes to target actor's mailbox, and wakes the target if it was Waiting (blocked on receive).

### snow_actor_receive (mod.rs)
Extern "C" function with three timeout modes:
- `timeout_ms == 0`: non-blocking, returns null if empty
- `timeout_ms < 0`: blocks indefinitely until message arrives
- `timeout_ms > 0`: blocks up to N milliseconds

Blocking sets process state to Waiting and yields to scheduler. Worker loop skips Waiting coroutines (no CPU burn). When a message arrives via send, state transitions to Ready and worker resumes the coroutine.

### snow_gc_alloc_actor (gc.rs)
Allocates from the current actor's per-actor heap. Falls back to global arena when called outside of an actor context (backward compatible with Phase 5 code).

### Scheduler Updates (scheduler.rs)
- `wake_process()`: cooperative wake via state transition
- Worker loop: checks process state before resuming; skips Waiting coroutines
- After yield: only sets Ready if not Waiting (receive may have set Waiting)

## Decisions Made

1. **Per-actor heap reuses Arena algorithm** -- Same bump allocation as gc.rs Arena. Consistency and proven correctness.
2. **MessageBuffer with raw bytes + type_tag** -- Simple tag scheme for Phase 6. Type tags derived from first 8 bytes of message data.
3. **Mutex-protected VecDeque for mailbox** -- Simple and correct. Lock-free upgrade is possible but unnecessary at this stage.
4. **Cooperative blocking via state machine** -- Waiting actors yield and don't consume CPU. State transitions (Waiting -> Ready) serve as the wake signal.
5. **Heap message layout: [type_tag, data_len, data]** -- Fixed 16-byte header enables efficient pointer arithmetic.
6. **snow_gc_alloc_actor fallback** -- Non-actor code still uses global arena seamlessly.

## Deviations from Plan

None -- plan executed exactly as written.

## Verification Results

- `cargo test -p snow-rt`: 46 tests pass (27 existing + 19 new)
- `cargo build -p snow-rt`: compiles successfully
- `cargo test --workspace`: all crates pass (2 pre-existing failures in snow-typeck actor tests are placeholder stubs from 06-02, not regressions)

## Next Phase Readiness

Plan 06-03 provides the foundation for:
- **06-04**: Supervision trees (need mailbox for monitor messages)
- **06-05**: Actor linking and exit signal propagation
- **06-06**: Per-actor GC (can now collect per-actor heaps)
- **06-07**: Typed message channels (builds on MessageBuffer type tags)

## Self-Check: PASSED
