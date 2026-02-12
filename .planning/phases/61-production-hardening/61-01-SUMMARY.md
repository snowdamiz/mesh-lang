---
phase: 61-production-hardening
plan: 01
subsystem: runtime
tags: [websocket, tls, rustls, heartbeat, ping-pong, fragmentation, wss]

# Dependency graph
requires:
  - phase: 60-actor-integration
    provides: "WebSocket actor-per-connection with reader thread bridge"
  - phase: 59-websocket-protocol
    provides: "Frame codec, handshake, close handling"
  - phase: 56-http-tls
    provides: "HttpStream enum pattern, build_server_config, rustls infrastructure"
provides:
  - "WsStream enum (Plain/Tls) abstracting over TCP and TLS WebSocket connections"
  - "snow_ws_serve_tls entry point for wss:// connections"
  - "HeartbeatState with 30s ping interval and 10s pong timeout"
  - "FragmentState with 16 MiB limit and continuation frame reassembly"
  - "16 MiB MAX_PAYLOAD_SIZE production limit (down from 64 MiB)"
  - "WsCloseCode::MESSAGE_TOO_BIG (1009) constant"
affects: [62-rooms, codegen-ws-serve-tls]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Arc<Mutex<WsStream>> unified reader/writer pattern (replacing try_clone)"
    - "100ms reader thread timeout for low-contention mutex sharing"
    - "HeartbeatState inline with read timeout cycle (no separate timer thread)"
    - "FragmentState state machine with ReassembleResult enum"

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/ws/server.rs"
    - "crates/snow-rt/src/ws/frame.rs"
    - "crates/snow-rt/src/ws/close.rs"
    - "crates/snow-rt/src/http/server.rs"

key-decisions:
  - "Unified Arc<Mutex<WsStream>> for both plain and TLS (Decision 1 from research)"
  - "100ms reader thread timeout balances contention and responsiveness"
  - "build_server_config made pub(crate) for cross-module reuse (not duplicated)"
  - "Pong handled before process_frame to inspect payload (Decision 5 from research)"
  - "UTF-8 validation on fully reassembled payload, not individual fragments"

patterns-established:
  - "WsStream enum mirrors HttpStream for stream abstraction"
  - "ReassembleResult enum for fragment state machine outcomes"
  - "macOS EAGAIN detection in timeout checks (temporarily unavailable)"

# Metrics
duration: 14min
completed: 2026-02-12
---

# Phase 61 Plan 01: Production Hardening Summary

**WsStream enum with TLS support (wss://), ping/pong heartbeat (30s/10s), and continuation frame reassembly with 16 MiB limit**

## Performance

- **Duration:** 14 min
- **Started:** 2026-02-12T23:41:55Z
- **Completed:** 2026-02-12T23:56:01Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- WsStream enum abstracts over plain TCP and TLS (rustls StreamOwned), implementing Read + Write
- snow_ws_serve_tls accepts cert/key paths, builds TLS config via shared build_server_config, wraps connections in WsStream::Tls
- Reader thread replaced try_clone with unified Arc<Mutex<WsStream>> for both read and write access
- HeartbeatState sends periodic Ping (30s), validates Pong payload match, closes dead connections after 10s timeout
- FragmentState reassembles continuation frames into complete messages, handles interleaved control frames, enforces 16 MiB limit
- MAX_PAYLOAD_SIZE reduced from 64 MiB to 16 MiB production limit
- All 323 snow-rt tests pass, zero regressions across full workspace

## Task Commits

Each task was committed atomically:

1. **Task 1: Introduce WsStream enum and snow_ws_serve_tls** - `ab4c1a3` (feat)
2. **Task 2: Add heartbeat ping/pong and fragment reassembly** - `567fd93` (feat)

## Files Created/Modified
- `crates/snow-rt/src/ws/server.rs` - WsStream enum, snow_ws_serve_tls, HeartbeatState, FragmentState, reassemble(), unified reader thread loop
- `crates/snow-rt/src/ws/frame.rs` - MAX_PAYLOAD_SIZE reduced to 16 MiB
- `crates/snow-rt/src/ws/close.rs` - WsCloseCode::MESSAGE_TOO_BIG (1009) constant
- `crates/snow-rt/src/http/server.rs` - build_server_config changed from fn to pub(crate) fn

## Decisions Made
- **Unified mutex vs. conditional paths:** Chose Arc<Mutex<WsStream>> for both plain and TLS (Decision 1 from research). Single code path for reader thread, heartbeat, and fragmentation. Mutex overhead negligible compared to network I/O.
- **100ms reader thread timeout:** Balances mutex contention (worst-case 100ms for Ws.send) with responsive shutdown/heartbeat checks. Reduced from 5s to prevent blocking actor sends.
- **Pong before process_frame:** Pong handling happens before process_frame dispatch so the heartbeat can inspect the payload. Existing process_frame silently ignores Pong, which is insufficient for heartbeat validation.
- **build_server_config pub(crate):** Shared between HTTP and WS TLS servers via one-word visibility change. No duplication.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed macOS EAGAIN timeout detection for short read timeouts**
- **Found during:** Task 1 (WsStream refactoring)
- **Issue:** With 100ms read timeout, macOS returns "Resource temporarily unavailable" (EAGAIN, os error 35) instead of "timed out" (ETIMEDOUT). The existing error check only matched "timed out" and "WouldBlock", causing real read timeouts to be treated as I/O errors and disconnecting the client.
- **Fix:** Added "temporarily unavailable" to the timeout detection string match.
- **Files modified:** crates/snow-rt/src/ws/server.rs
- **Verification:** All 5 WS server integration tests pass (previously 4/5 failed).
- **Committed in:** ab4c1a3 (Task 1 commit)

**2. [Rule 1 - Bug] Fixed mutex contention blocking echo responses**
- **Found during:** Task 1 (WsStream refactoring)
- **Issue:** Replacing try_clone with Arc<Mutex<WsStream>> caused the reader thread to hold the mutex during blocking read_frame calls (up to 5s timeout). Actor's Ws.send echo responses were blocked, causing test timeouts.
- **Fix:** Reduced reader thread timeout to 100ms and added WsStream::set_read_timeout helper to change timeout after construction.
- **Files modified:** crates/snow-rt/src/ws/server.rs
- **Verification:** All server integration tests pass with sub-second response times.
- **Committed in:** ab4c1a3 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs)
**Impact on plan:** Both fixes required for the unified mutex approach to work correctly. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviations above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- TLS WebSocket server (wss://) ready for codegen wiring (snow_ws_serve_tls is #[no_mangle] pub extern "C")
- Heartbeat and fragmentation active for all connections (both plain and TLS)
- Phase 61 Plan 02 can wire Ws.serve_tls into the codegen pipeline (intrinsics, known_functions, map_builtin_name)
- Phase 62 (Rooms) can build on the production-hardened WebSocket infrastructure

## Self-Check: PASSED

All files verified present. All commit hashes verified in git log.

---
*Phase: 61-production-hardening*
*Completed: 2026-02-12*
