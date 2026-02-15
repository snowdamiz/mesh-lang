---
phase: 90-real-time-streaming
plan: 03
subsystem: streaming
tags: [websocket, backpressure, buffer, drain-ticker, service, mesh]

# Dependency graph
requires:
  - phase: 90-01
    provides: "StreamManager service with ConnectionState (buffer/buffer_len/max_buffer fields)"
provides:
  - "BufferMessage cast handler for queuing messages with drop-oldest backpressure"
  - "DrainBuffers cast handler for periodic buffer flush via Ws.send"
  - "stream_drain_ticker actor for 250ms periodic drain cycle"
affects: [90-02-PLAN, mesher ingestion pipeline, ws_handler]

# Tech tracking
tech-stack:
  added: []
  patterns: ["define-before-use function ordering for helper chains", "buffer_if_client guard extraction from cast body"]

key-files:
  created: []
  modified:
    - mesher/services/stream_manager.mpl
    - mesher/ingestion/pipeline.mpl

key-decisions:
  - "Functions ordered bottom-up (leaf first, callers after) to satisfy Mesh define-before-use requirement"
  - "buffer_if_client helper extracted from BufferMessage cast body to avoid parser if/else limitation in cast handlers"
  - "250ms drain interval for responsive buffer flushing (4 flushes/second, WS sends are cheap)"

patterns-established:
  - "define-before-use ordering: When helper functions call each other in a chain, define leaf functions first and callers last"
  - "cast-guard-extraction: Extract if/else logic from cast handler bodies into helper functions to avoid Mesh parser limitations"

# Metrics
duration: 3min
completed: 2026-02-15
---

# Phase 90 Plan 03: Backpressure Buffer Drain Summary

**BufferMessage/DrainBuffers handlers with drop-oldest backpressure and stream_drain_ticker actor (250ms periodic flush via Ws.send)**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-15T04:56:27Z
- **Completed:** 2026-02-15T04:59:55Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- BufferMessage cast handler queues messages for slow clients with drop-oldest when max_buffer exceeded
- DrainBuffers cast handler iterates all connections, sends buffered messages via Ws.send, removes connections on send failure
- stream_drain_ticker actor follows established flush_ticker pattern (Timer.sleep + recursive call)
- Pipeline spawns drain ticker alongside StreamManager in both start_pipeline and restart_all_services

## Task Commits

Each task was committed atomically:

1. **Task 1: Add buffer management and drain ticker to StreamManager** - `a31a0ffb` (feat)
2. **Task 2: Wire stream_drain_ticker into pipeline startup** - `595f67d6` (feat)

## Files Created/Modified
- `mesher/services/stream_manager.mpl` - Added buffer_message_for_conn, buffer_if_client, send_buffer_loop, drain_single_connection, drain_connections_loop, drain_all_buffers helpers; BufferMessage/DrainBuffers cast handlers; stream_drain_ticker actor
- `mesher/ingestion/pipeline.mpl` - Import stream_drain_ticker; spawn in start_pipeline and restart_all_services with 250ms interval

## Decisions Made
- **Define-before-use ordering:** Mesh requires functions to be defined before they are referenced. Helper chains (drain_all_buffers -> drain_connections_loop -> drain_single_connection -> send_buffer_loop) must be ordered leaf-first. This is a known Mesh compiler behavior.
- **Cast guard extraction:** The BufferMessage handler needed an if/else to guard against non-streaming clients. Mesh parser rejects if/else with multiple branches inside cast handler bodies. Extracted to buffer_if_client helper function, following the established pattern from other services.
- **250ms drain interval:** Chosen for responsive buffer flushing (4x/second). WS sends are cheap compared to DB writes, so a shorter interval than flush_ticker (which uses longer intervals for expensive DB flushes) is appropriate.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Parser rejects if/else in cast handler body**
- **Found during:** Task 1 (BufferMessage handler)
- **Issue:** Plan specified inline `if is_stream_client(...) do ... else state end` inside the cast handler. Mesh parser reports "expected DO_KW" at the else branch.
- **Fix:** Extracted the guard logic into a `buffer_if_client` helper function; cast body calls the single helper
- **Files modified:** mesher/services/stream_manager.mpl
- **Verification:** Build passes with no stream_manager.mpl errors
- **Committed in:** a31a0ffb (Task 1 commit)

**2. [Rule 3 - Blocking] Forward reference errors (define-before-use)**
- **Found during:** Task 1 (buffer/drain helpers)
- **Issue:** Plan listed functions in caller-first order (buffer_if_client before buffer_message_for_conn, drain_all_buffers before drain_connections_loop, etc.). Mesh compiler reported "undefined variable" for each forward-referenced function.
- **Fix:** Reordered all helper functions bottom-up: leaf functions defined first, callers defined after
- **Files modified:** mesher/services/stream_manager.mpl
- **Verification:** Build passes with no stream_manager.mpl errors
- **Committed in:** a31a0ffb (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both auto-fixes necessary for compilation. No scope change -- identical functionality, different ordering and extraction.

## Issues Encountered
None beyond the auto-fixed deviations.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- STREAM-05 backpressure mechanism is complete: slow clients get messages buffered, old events dropped at capacity, periodic drain via Ws.send
- StreamManager now has full buffer lifecycle (buffer -> drain -> clear/remove)
- No blockers for phase completion

## Self-Check: PASSED

- [x] mesher/services/stream_manager.mpl exists with BufferMessage, DrainBuffers, stream_drain_ticker
- [x] mesher/ingestion/pipeline.mpl exists with stream_drain_ticker import and spawn
- [x] 90-03-SUMMARY.md created
- [x] Commit a31a0ffb exists (Task 1)
- [x] Commit 595f67d6 exists (Task 2)
- [x] No compilation errors from stream_manager.mpl or pipeline.mpl

---
*Phase: 90-real-time-streaming*
*Completed: 2026-02-15*
