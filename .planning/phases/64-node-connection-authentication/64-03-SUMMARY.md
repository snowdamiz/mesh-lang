---
phase: 64-node-connection-authentication
plan: 03
subsystem: dist
tags: [heartbeat, ping-pong, dead-connection-detection, session-lifecycle, reader-thread, cleanup]

# Dependency graph
requires:
  - phase: 64-node-connection-authentication
    plan: 02
    provides: "Authenticated TLS connections, NodeSession with stream/shutdown, accept loop, snow_node_connect"
provides:
  - "HeartbeatState with configurable ping interval and pong timeout"
  - "HEARTBEAT_PING (0xF0) / HEARTBEAT_PONG (0xF1) wire format"
  - "reader_loop_session: dedicated reader thread per node session"
  - "heartbeat_loop_session: periodic ping sender with dead connection detection"
  - "spawn_session_threads: wired into accept_loop and snow_node_connect"
  - "cleanup_session: removes disconnected node from NodeState maps"
  - "7 integration tests covering heartbeat timing, handshake, wire format, and full lifecycle"
affects: [65-message-routing, 66-fault-tolerance]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Shared HeartbeatState via Arc<Mutex> between reader and heartbeat threads", "100ms read timeout for non-blocking shutdown checks", "Session-based thread spawning with Arc<NodeSession> sharing"]

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/dist/node.rs"

key-decisions:
  - "Arc<Mutex<HeartbeatState>> shared between reader and heartbeat threads (simpler than channel)"
  - "reader_loop_session and heartbeat_loop_session operate on Arc<NodeSession> directly"
  - "100ms read timeout pattern for shutdown responsiveness (matches ws/server.rs)"
  - "Heartbeat loop polls at 500ms intervals to avoid busy-wait"

patterns-established:
  - "HeartbeatState pattern reused from ws/server.rs with 8-byte payload (vs 4-byte for WS)"
  - "Per-session named threads: snow-node-reader-{name} and snow-node-heartbeat-{name}"
  - "cleanup_session pattern: remove from sessions map, then node_id_map"

# Metrics
duration: 8min
completed: 2026-02-13
---

# Phase 64 Plan 03: Heartbeat & Session Management Summary

**Heartbeat-based dead connection detection with 60s ping / 15s timeout, reader thread for incoming messages, and session cleanup on disconnect**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-13T04:22:10Z
- **Completed:** 2026-02-13T04:30:48Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- HeartbeatState with configurable intervals (default 60s ping, 15s pong timeout) detects dead connections
- Reader thread per session handles HEARTBEAT_PING (auto-responds with PONG) and HEARTBEAT_PONG (updates HeartbeatState)
- Heartbeat thread per session sends periodic pings with random 8-byte payloads and monitors for timely pongs
- spawn_session_threads wired into both accept_loop (incoming) and snow_node_connect (outgoing)
- cleanup_session removes disconnected node from sessions and node_id_map
- 7 new integration tests: heartbeat timing, wire format roundtrip, in-memory handshake, wrong cookie detection, full TLS lifecycle, heartbeat wire format, cleanup graceful handling
- All 382 snow-rt tests pass (375 existing + 7 new)

## Task Commits

Each task was committed atomically:

1. **Task 1: Heartbeat ping/pong and reader thread** - `1d9c5e4` (feat)
2. **Task 2: Integration tests for node connection lifecycle** - `55c9d21` (test)

## Files Created/Modified
- `crates/snow-rt/src/dist/node.rs` - HeartbeatState, HEARTBEAT_PING/PONG constants, reader_loop_session, heartbeat_loop_session, spawn_session_threads, cleanup_session, NodeStream.set_read_timeout, 7 integration tests

## Decisions Made
- **Arc<Mutex<HeartbeatState>> over channel:** Sharing heartbeat state via mutex between reader and heartbeat threads is simpler than a channel approach and matches the ws/server.rs pattern. The reader updates `last_pong_received` when it sees a matching pong; the heartbeat thread checks for overdue pongs.
- **Session-based thread functions:** Created `reader_loop_session` and `heartbeat_loop_session` that take `Arc<NodeSession>` directly, rather than extracting the stream into a separate `Arc<Mutex<NodeStream>>`. This avoids the complexity of wrapping an already-mutex'd field in another Arc.
- **100ms read timeout:** Matches the ws/server.rs pattern. Provides responsive shutdown checks without busy-waiting, and keeps mutex contention window small.
- **500ms heartbeat poll interval:** Balances responsiveness with CPU overhead. Dead connections detected within ping_interval + pong_timeout (worst case 75s).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Eliminated generic reader_loop/heartbeat_loop in favor of session-based variants**
- **Found during:** Task 1
- **Issue:** The plan specified `reader_loop` and `heartbeat_loop` taking `Arc<Mutex<NodeStream>>`, but NodeSession already wraps stream in `Mutex<NodeStream>`. Wrapping that in another Arc<Mutex> would create double-locking complexity.
- **Fix:** Created `reader_loop_session` and `heartbeat_loop_session` that take `Arc<NodeSession>` directly and lock `session.stream` inline. Removed the unused generic variants.
- **Files modified:** crates/snow-rt/src/dist/node.rs
- **Verification:** cargo build (no warnings), cargo test (382 tests pass)
- **Committed in:** 1d9c5e4 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Simplified design that avoids unnecessary double-mutex wrapping. All planned functionality delivered.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 64 complete: nodes can start, connect with TLS + cookie auth, and maintain heartbeat-monitored connections
- Reader thread ready for Phase 65 to add message routing (non-heartbeat messages currently ignored)
- cleanup_session ready for Phase 66 to add `:nodedown` notification
- HeartbeatState intervals configurable for future NODE-08 configuration

## Self-Check: PASSED

---
*Phase: 64-node-connection-authentication*
*Plan: 03*
*Completed: 2026-02-13*
