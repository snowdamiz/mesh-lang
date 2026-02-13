---
phase: 64-node-connection-authentication
plan: 02
subsystem: dist
tags: [hmac, sha256, tls, handshake, authentication, challenge-response, binary-protocol]

# Dependency graph
requires:
  - phase: 64-node-connection-authentication
    plan: 01
    provides: "NodeState singleton, TLS configs, TCP listener, NodeSession placeholder"
provides:
  - "HMAC-SHA256 challenge/response handshake protocol (4-message binary exchange)"
  - "perform_handshake function for both initiator and acceptor flows"
  - "snow_node_connect extern C function for outgoing connections"
  - "Accept loop TLS wrapping and cookie authentication"
  - "NodeStream enum (ServerTls/ClientTls) with Read+Write"
  - "NodeSession with TLS stream, shutdown flag, connected_at"
  - "register_session helper with duplicate connection tiebreaker"
affects: [64-03-heartbeat, 65-message-routing, 66-fault-tolerance]

# Tech tracking
tech-stack:
  added: []
  patterns: ["HMAC-SHA256 challenge/response with constant-time verify_slice", "Length-prefixed binary wire format (LE u32 + payload)", "4-message handshake: NAME/CHALLENGE/REPLY/ACK", "NodeStream enum for server/client TLS variants"]

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/dist/node.rs"

key-decisions:
  - "Constant-time HMAC verification via Mac::verify_slice (prevents timing attacks)"
  - "Little-endian wire format with 4KB max message size for handshake"
  - "Duplicate connection tiebreaker: lexicographically smaller name wins"
  - "NodeSession stream field is pub(crate) Mutex<NodeStream> (shared reader/writer access)"

patterns-established:
  - "Binary handshake protocol: tag byte + length-prefixed payload over TLS"
  - "Tiebreaker pattern: lexicographically smaller node name keeps its connection"
  - "Register session pattern: write-lock sessions, insert, write-lock node_id_map"

# Metrics
duration: 5min
completed: 2026-02-13
---

# Phase 64 Plan 02: Handshake Protocol Summary

**HMAC-SHA256 cookie challenge/response over TLS with 4-message binary handshake, snow_node_connect, and accept loop authentication**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-13T04:15:01Z
- **Completed:** 2026-02-13T04:19:48Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- HMAC-SHA256 challenge/response with constant-time comparison (verify_slice) for cookie authentication
- 4-message binary handshake protocol (NAME, CHALLENGE, REPLY, ACK) with length-prefixed wire format
- snow_node_connect extern "C" function: TCP connect, TLS wrap, initiator handshake, session registration
- Accept loop upgraded from stub to full TLS server handshake + cookie authentication
- NodeStream enum supporting both ServerTls and ClientTls variants with Read+Write delegation
- NodeSession expanded with Mutex<NodeStream>, connected_at timestamp
- register_session helper with duplicate connection detection and name-based tiebreaker
- 3 new unit tests for HMAC determinism and correct/wrong cookie verification

## Task Commits

Each task was committed atomically:

1. **Task 1: HMAC-SHA256 challenge/response and binary handshake protocol** - `9ada95d` (feat)
2. **Task 2: snow_node_connect and accept loop integration** - `398b9b4` (feat)

## Files Created/Modified
- `crates/snow-rt/src/dist/node.rs` - Handshake protocol, wire format, HMAC functions, snow_node_connect, accept loop TLS integration

## Decisions Made
- **Constant-time HMAC verification:** Uses `Mac::verify_slice` from the hmac crate, which performs constant-time comparison internally. Prevents timing attacks where an attacker could brute-force the cookie by measuring response times.
- **Little-endian wire format:** All multi-byte integers in the handshake are little-endian (u32 length prefix, u16 name length). Consistent with the platform-native byte order and the existing STF format.
- **4KB max handshake message:** Enforced in `read_msg` to prevent allocation bombs from malicious peers during handshake.
- **Duplicate connection tiebreaker:** Node with lexicographically smaller name keeps its existing connection. This resolves simultaneous connection attempts deterministically without coordination.
- **NodeSession.stream as pub(crate):** The stream field uses `pub(crate)` visibility since `NodeStream` is `pub(crate)`. Plan 03's reader/heartbeat threads will access it within the crate.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] NodeSession expanded in Task 1 instead of Task 2**
- **Found during:** Task 1
- **Issue:** register_session (Task 1) needs NodeSession with stream/connected_at fields, but plan puts NodeSession expansion in Task 2
- **Fix:** Expanded NodeSession in Task 1 alongside register_session to allow compilation
- **Files modified:** crates/snow-rt/src/dist/node.rs
- **Verification:** cargo build succeeds
- **Committed in:** 9ada95d (Task 1 commit)

**2. [Rule 1 - Bug] NodeStream and register_session moved to Task 1**
- **Found during:** Task 1
- **Issue:** perform_handshake needs register_session context, which needs NodeStream. Both are listed under Task 2 but are logically part of the handshake infrastructure.
- **Fix:** Implemented NodeStream enum and register_session in Task 1 so the handshake protocol is self-contained
- **Files modified:** crates/snow-rt/src/dist/node.rs
- **Verification:** All tests pass
- **Committed in:** 9ada95d (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Task boundary shift only. All planned functionality delivered across the two tasks. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Authenticated TLS connections fully functional between nodes
- NodeSession holds the stream for Plan 03 to spawn reader + heartbeat threads
- register_session creates Arc<NodeSession> ready for concurrent access
- snow_node_connect callable from compiled Snow code via LLVM codegen
- Accept loop authenticates incoming connections and registers sessions

---
*Phase: 64-node-connection-authentication*
*Plan: 02*
*Completed: 2026-02-13*
