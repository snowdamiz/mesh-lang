---
phase: 59-protocol-core
plan: 02
subsystem: runtime
tags: [websocket, rfc6455, handshake, close-handshake, utf8-validation, http-upgrade]

# Dependency graph
requires:
  - phase: 59-protocol-core-01
    provides: "WsOpcode, WsFrame, read_frame, write_frame, apply_mask frame codec"
provides:
  - "perform_upgrade for HTTP-to-WebSocket upgrade with Sec-WebSocket-Accept"
  - "validate_upgrade_request for RFC 6455 header validation"
  - "write_bad_request for HTTP 400 on malformed upgrade requests"
  - "parse_close_payload and build_close_payload for close frame handling"
  - "send_close for sending close frames with status codes"
  - "validate_text_payload for UTF-8 validation on text frames"
  - "process_frame for protocol-level frame dispatch (text, binary, close, ping, pong)"
  - "WsCloseCode constants (1000, 1001, 1002, 1007, 1011)"
affects: [60-actor-ws, 61-tls-ws]

# Tech tracking
tech-stack:
  added: []
  patterns: ["RFC 6455 handshake with BufReader borrow scoping", "process_frame dispatch for protocol-level frame handling"]

key-files:
  created:
    - "crates/snow-rt/src/ws/handshake.rs"
    - "crates/snow-rt/src/ws/close.rs"
  modified:
    - "crates/snow-rt/src/ws/mod.rs"

key-decisions:
  - "BufReader used for HTTP header parsing in perform_upgrade, with explicit buffer-empty sanity check before dropping"
  - "process_frame echoes close code only (no reason) to minimize control frame size"
  - "Continuation opcode passed through to caller -- Phase 61 handles reassembly"

patterns-established:
  - "TestStream helper struct (read_buf + write_buf) for testing Read+Write generics"
  - "process_frame returns Ok(Some) for data, Ok(None) for handled control, Err for close/error"

# Metrics
duration: 5min
completed: 2026-02-12
---

# Phase 59 Plan 02: WebSocket Handshake, Close, and Validation Summary

**RFC 6455 HTTP upgrade handshake with Sec-WebSocket-Accept, close handshake with status code echo, UTF-8 text validation (1007), and protocol-level frame dispatch for all opcodes**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-12T22:21:49Z
- **Completed:** 2026-02-12T22:26:57Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- HTTP upgrade handshake validates GET method, Upgrade/Connection/Sec-WebSocket-Key/Version headers per RFC 6455 Section 4.2
- Sec-WebSocket-Accept computation matches RFC 6455 test vector exactly (SHA-1 + Base64)
- Close handshake parses and builds close payloads with proper truncation (125-byte control frame limit)
- Text frame UTF-8 validation rejects invalid payloads with close code 1007
- process_frame dispatches all 6 opcodes: text (UTF-8 check), binary (passthrough), close (echo), ping (pong reply), pong (ignore), continuation (passthrough)
- 30 total WebSocket tests across all three modules (frame: 9, handshake: 9, close: 12)

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement HTTP upgrade handshake** - `2669962` (feat)
2. **Task 2: Implement close handshake and text frame UTF-8 validation** - `6a91bcb` (feat)

## Files Created/Modified
- `crates/snow-rt/src/ws/handshake.rs` - HTTP upgrade: compute_accept_key, validate_upgrade_request, perform_upgrade, write_upgrade_response, write_bad_request + 9 tests
- `crates/snow-rt/src/ws/close.rs` - Close handshake: parse/build close payloads, validate_text_payload, send_close, process_frame, WsCloseCode + 12 tests
- `crates/snow-rt/src/ws/mod.rs` - Updated module declarations and re-exports for all three submodules

## Decisions Made
- BufReader used for HTTP header parsing in perform_upgrade with explicit buffer-empty sanity check before dropping (per research guidance on BufReader pitfall)
- process_frame echoes only the close code (no reason string) to minimize control frame payload size
- Continuation opcode recognized and passed through to the caller -- Phase 61 will handle message reassembly

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Phase 59 protocol layer is complete: frame codec (Plan 01) + handshake/close/validation (Plan 02)
- Phase 60 can import perform_upgrade, read_frame, write_frame, process_frame, send_close directly
- Clean module structure: ws/{mod.rs, frame.rs, handshake.rs, close.rs}
- All PROTO-01 through PROTO-09 requirements covered and tested
- Full workspace test suite passes with zero regressions

## Self-Check: PASSED

- All 4 files verified present on disk
- Commit 2669962 (Task 1) found in git log
- Commit 6a91bcb (Task 2) found in git log
- 30/30 WebSocket unit tests passing (frame: 9, handshake: 9, close: 12)

---
*Phase: 59-protocol-core*
*Completed: 2026-02-12*
