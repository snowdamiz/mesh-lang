---
phase: 65-remote-send-distribution-router
plan: 01
subsystem: dist
tags: [distribution, message-routing, tls, wire-format, actor-send]

# Dependency graph
requires:
  - phase: 63-pid-encoding-wire-format
    provides: "PID encoding with 16-bit node_id in upper bits, dist_send_stub"
  - phase: 64-node-connection-authentication
    provides: "NodeState, NodeSession, TLS streams, reader_loop_session, heartbeat"
provides:
  - "dist_send: real remote message routing via NodeSession TLS stream"
  - "DIST_SEND (0x10) and DIST_REG_SEND (0x11) wire format constants"
  - "read_dist_msg: 16MB limit post-handshake message reader"
  - "Reader loop DIST_SEND/DIST_REG_SEND dispatch to local actors"
  - "snow_actor_send_named: extern C API for send({name, node}, msg)"
affects: [66-nodedown-monitor, 67-remote-spawn, 68-cluster-pg]

# Tech tracking
tech-stack:
  added: []
  patterns: ["DIST_SEND/DIST_REG_SEND binary wire protocol", "node_id extraction from PID upper bits for routing"]

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/actor/mod.rs"
    - "crates/snow-rt/src/dist/node.rs"
    - "crates/snow-rt/src/lib.rs"

key-decisions:
  - "Silent drop on all failure paths (unknown node, disconnected session, write error) -- Phase 66 adds :nodedown"
  - "read_dist_msg with 16MB limit replaces read_msg (4KB) in reader loop post-handshake"
  - "snow_actor_send_named handles local self-send via registry lookup before checking remote path"

patterns-established:
  - "DIST_SEND wire format: [tag 0x10][u64 target_pid LE][raw message bytes]"
  - "DIST_REG_SEND wire format: [tag 0x11][u16 name_len LE][name bytes][raw message bytes]"
  - "Silent drop pattern for distribution failures (consistent with Erlang behavior)"

# Metrics
duration: 3min
completed: 2026-02-13
---

# Phase 65 Plan 01: Remote Send & Distribution Router Summary

**Real dist_send routing via NodeSession TLS streams with DIST_SEND/DIST_REG_SEND wire protocol and snow_actor_send_named extern C API**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-13T04:55:21Z
- **Completed:** 2026-02-13T04:58:51Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Replaced dist_send_stub with real dist_send that routes messages to remote nodes via NodeSession TLS streams
- Added DIST_SEND and DIST_REG_SEND message handlers to the reader loop for incoming message delivery
- Added read_dist_msg with 16MB limit for post-handshake distribution messages (replacing 4KB handshake limit)
- Added snow_actor_send_named extern "C" API supporting both local self-send and remote named process send

## Task Commits

Each task was committed atomically:

1. **Task 1: Replace dist_send_stub with real dist_send and add reader loop message handlers** - `a82bf4f` (feat)
2. **Task 2: Add snow_actor_send_named extern "C" API for send({name, node}, msg)** - `277b072` (feat)

## Files Created/Modified
- `crates/snow-rt/src/actor/mod.rs` - Real dist_send, pub(crate) local_send, snow_actor_send_named
- `crates/snow-rt/src/dist/node.rs` - DIST_SEND/DIST_REG_SEND constants, read_dist_msg, reader loop dispatch
- `crates/snow-rt/src/lib.rs` - Re-export snow_actor_send_named

## Decisions Made
- Silent drop on all failure paths (unknown node, disconnected session, write error) -- consistent with Erlang behavior, Phase 66 will add :nodedown notifications
- read_dist_msg with 16MB limit replaces read_msg (4KB) in the reader loop post-handshake, allowing full-size actor messages between nodes
- snow_actor_send_named handles local self-send via registry lookup before checking the remote path, avoiding unnecessary network round-trips

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Remote message routing is fully functional for both PID-addressed and name-addressed sends
- Reader loop delivers incoming messages to local actors via local_send
- Ready for Plan 02 (snow_node_connect outbound connection improvements) and Plan 03 (monitoring/nodedown)

---
*Phase: 65-remote-send-distribution-router*
*Completed: 2026-02-13*
