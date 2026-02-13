---
phase: 66-remote-links-monitors-failure-handling
plan: 02
subsystem: actor
tags: [node-monitors, connection-loss, nodedown, nodeup, dist-monitor, fault-tolerance]

# Dependency graph
requires:
  - phase: 66-remote-links-monitors-failure-handling
    plan: 01
    provides: "Process monitors/monitored_by fields, DOWN_SIGNAL_TAG, encode_down_signal, pub(crate) encode_reason/decode_reason"
  - phase: 65-remote-send-distribution-router
    provides: "Remote PID routing, dist_send, node sessions, reader_loop_session"
provides:
  - "node_monitors registry on NodeState for :nodedown/:nodeup tracking"
  - "handle_node_disconnect: two-phase failure propagation for remote links and monitors"
  - "handle_node_connect: :nodeup delivery on session registration"
  - "DIST_MONITOR/DEMONITOR/MONITOR_EXIT wire message handlers in reader loop"
  - "snow_node_monitor extern C API for node monitoring"
  - "Remote snow_process_monitor sends DIST_MONITOR wire message"
  - "NODEDOWN_TAG and NODEUP_TAG reserved type tags"
  - "send_dist_monitor_exit helper for remote monitor exit notifications"
affects: [66-03-remote-links]

# Tech tracking
tech-stack:
  added: []
  patterns: [two-phase-disconnect-handling, node-event-delivery, remote-monitor-wire-protocol]

key-files:
  created: []
  modified:
    - crates/snow-rt/src/dist/node.rs
    - crates/snow-rt/src/actor/mod.rs
    - crates/snow-rt/src/lib.rs

key-decisions:
  - "Two-phase disconnect handling: collect under read lock, execute after dropping lock (avoids deadlocks)"
  - "NODEDOWN_TAG = u64::MAX - 2 and NODEUP_TAG = u64::MAX - 3 as reserved sentinel type tags"
  - "Node monitor is_once=false by default (persistent monitors, retained across events)"
  - "Remote monitor sends DIST_MONITOR wire message; if session not found, immediate DOWN(noconnection)"
  - "DIST_MONITOR on dead process immediately sends DIST_MONITOR_EXIT back with noproc reason"

patterns-established:
  - "Two-phase disconnect: collect actions under process table read lock, drop lock, then execute"
  - "Node event delivery: encode node_name as payload bytes with NODEDOWN_TAG/NODEUP_TAG type tags"
  - "Remote monitor wire protocol: DIST_MONITOR(0x16), DIST_DEMONITOR(0x17), DIST_MONITOR_EXIT(0x18)"
  - "handle_node_connect called at end of register_session for :nodeup delivery"

# Metrics
duration: 4min
completed: 2026-02-13
---

# Phase 66 Plan 02: Node Monitoring & Connection Loss Propagation Summary

**Node monitoring API, two-phase handle_node_disconnect firing :noconnection exit signals and DOWN messages, DIST_MONITOR/DEMONITOR/MONITOR_EXIT wire handlers, and :nodedown/:nodeup delivery**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-13T06:07:37Z
- **Completed:** 2026-02-13T06:12:19Z
- **Tasks:** 1
- **Files modified:** 3

## Accomplishments
- NodeState extended with node_monitors registry tracking which processes monitor which nodes
- handle_node_disconnect implements two-phase approach: collects remote links/monitors under process table read lock, then delivers :noconnection exit signals (respecting trap_exit), DOWN(noconnection) messages, and :nodedown events after dropping lock
- DIST_MONITOR/DEMONITOR/MONITOR_EXIT wire message handlers added to reader_loop_session
- snow_node_monitor extern C API registers calling process for nodedown/nodeup events
- snow_process_monitor extended to send DIST_MONITOR wire message for remote targets with DOWN(noconnection) fallback
- cleanup_session now triggers handle_node_disconnect, register_session triggers handle_node_connect

## Task Commits

Each task was committed atomically:

1. **Task 1: Add node_monitors, snow_node_monitor, handle_node_disconnect, wire handlers** - `316cbb0` (feat)

**Plan metadata:** (pending final commit)

## Files Created/Modified
- `crates/snow-rt/src/dist/node.rs` - Added node_monitors field, DIST_MONITOR/DEMONITOR/MONITOR_EXIT constants, NODEDOWN_TAG/NODEUP_TAG, handle_node_disconnect, handle_node_connect, deliver_node_event, send_dist_monitor_exit, reader loop handlers, cleanup_session wiring
- `crates/snow-rt/src/actor/mod.rs` - Added snow_node_monitor, send_dist_monitor helper, extended snow_process_monitor remote path
- `crates/snow-rt/src/lib.rs` - Re-exported snow_node_monitor

## Decisions Made
- Two-phase disconnect handling (collect under read lock, execute after drop) to avoid deadlocks between process table and scheduler locks
- NODEDOWN_TAG (u64::MAX - 2) and NODEUP_TAG (u64::MAX - 3) continue the reserved sentinel pattern from EXIT_SIGNAL_TAG (u64::MAX) and DOWN_SIGNAL_TAG (u64::MAX - 1)
- Node monitors default to persistent (is_once=false) -- retained across events, not auto-removed on first nodedown
- Remote snow_process_monitor sends DIST_MONITOR wire message; if session unavailable, delivers DOWN(noconnection) immediately
- DIST_MONITOR handler checks if target is already dead and immediately sends DIST_MONITOR_EXIT back with noproc reason

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Node monitoring and connection loss propagation complete and verified (393 tests pass, zero regressions)
- Plan 03 (remote link exit propagation) can build on: handle_node_disconnect (already handles remote links), DIST_EXIT wire message handling, send_dist_exit for cross-node exit signal forwarding
- The DIST_LINK/DIST_UNLINK/DIST_EXIT constants and send_dist_link/send_dist_exit helpers already exist (added by prior auto-fix), ready for Plan 03 wiring

## Self-Check: PASSED

All 3 modified files verified present. Task commit (316cbb0) verified in git log.

---
*Phase: 66-remote-links-monitors-failure-handling*
*Completed: 2026-02-13*
