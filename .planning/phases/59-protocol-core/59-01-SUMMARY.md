---
phase: 59-protocol-core
plan: 01
subsystem: runtime
tags: [websocket, rfc6455, frame-codec, xor-masking, binary-protocol]

# Dependency graph
requires:
  - phase: 54-postgres-wire
    provides: "sha2/base64 RustCrypto pattern and Cargo.toml dependency structure"
provides:
  - "WsOpcode enum with 6 standard WebSocket opcodes"
  - "WsFrame struct for parsed frame representation"
  - "read_frame() for parsing masked client frames (3 length encodings)"
  - "write_frame() for unmasked server frame output"
  - "apply_mask() symmetric XOR masking"
  - "ws/ module structure in snow-rt"
affects: [59-02, 60-actor-ws, 61-tls-ws]

# Tech tracking
tech-stack:
  added: ["sha1 0.10"]
  patterns: ["RFC 6455 frame codec with read_exact I/O", "64 MiB payload safety cap"]

key-files:
  created:
    - "crates/snow-rt/src/ws/mod.rs"
    - "crates/snow-rt/src/ws/frame.rs"
  modified:
    - "crates/snow-rt/Cargo.toml"
    - "crates/snow-rt/src/lib.rs"

key-decisions:
  - "64 MiB payload safety cap to prevent OOM (Phase 61 will tighten to 16 MiB)"
  - "read_frame uses read_exact on raw stream, no BufReader (per research anti-pattern)"

patterns-established:
  - "ws/ module follows same structure as http/ (mod.rs + submodules + re-exports)"
  - "Frame codec is generic over Read/Write traits for testability with Cursor"

# Metrics
duration: 3min
completed: 2026-02-12
---

# Phase 59 Plan 01: WebSocket Frame Codec Summary

**RFC 6455 frame codec with WsOpcode enum, read_frame/write_frame for all 3 payload length encodings, XOR masking, and 9 unit tests**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-12T22:15:48Z
- **Completed:** 2026-02-12T22:18:36Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- WsOpcode enum recognizes all 6 standard opcodes and rejects unknown opcodes with clear errors
- read_frame correctly parses masked client frames with 7-bit, 16-bit, and 64-bit payload length encodings
- write_frame produces unmasked server frames with correct header encoding for all three length ranges
- apply_mask implements symmetric XOR masking per RFC 6455 Section 5.3
- RSV bit validation rejects frames with non-zero RSV bits (no extensions negotiated)
- 64 MiB safety cap prevents OOM from malicious 64-bit payload lengths
- 9 comprehensive unit tests cover all code paths using in-memory Cursor buffers

## Task Commits

Each task was committed atomically:

1. **Task 1: Add sha1 dependency and create ws/ module skeleton** - `170b3dc` (chore)
2. **Task 2: Implement frame codec (read_frame, write_frame, apply_mask)** - `8d3476e` (feat)

## Files Created/Modified
- `crates/snow-rt/Cargo.toml` - Added sha1 0.10 dependency for RFC 6455 Sec-WebSocket-Accept
- `crates/snow-rt/src/ws/mod.rs` - WebSocket module declarations and re-exports
- `crates/snow-rt/src/ws/frame.rs` - Frame codec: WsOpcode, WsFrame, read_frame, write_frame, apply_mask + 9 tests
- `crates/snow-rt/src/lib.rs` - Added pub mod ws declaration

## Decisions Made
- 64 MiB payload safety cap chosen as generous limit for Phase 59; Phase 61 will tighten to 16 MiB for production
- Used read_exact on raw stream (not BufReader) per research anti-pattern guidance to avoid buffering issues at protocol boundary
- Continuation opcode (0x0) recognized as valid but not reassembled -- Phase 61 handles fragmentation

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Frame codec is complete and tested, ready for Plan 02 (handshake + close)
- Plan 02 will add handshake.rs (HTTP upgrade + Sec-WebSocket-Accept) and close.rs (close handshake state machine)
- The ws/mod.rs is structured for easy addition of handshake and close submodules

## Self-Check: PASSED

- All 5 files verified present on disk
- Commit 170b3dc (Task 1) found in git log
- Commit 8d3476e (Task 2) found in git log
- 9/9 unit tests passing

---
*Phase: 59-protocol-core*
*Completed: 2026-02-12*
