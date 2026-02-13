---
phase: 64-node-connection-authentication
plan: 01
subsystem: dist
tags: [tls, rustls, ring, ecdsa, tcp, node-identity, distribution]

# Dependency graph
requires:
  - phase: 63-pid-encoding-wire-format
    provides: "PID bit-packing with node_id/creation fields, dist/wire.rs STF module"
provides:
  - "NodeState singleton (name, host, port, cookie, TLS configs, sessions map)"
  - "Ephemeral ECDSA P-256 self-signed certificate generation"
  - "TLS ServerConfig and ClientConfig for inter-node connections"
  - "SkipCertVerification for cookie-based trust model"
  - "TCP listener with accept loop on background thread"
  - "snow_node_start extern C entry point"
  - "parse_node_name utility for name@host:port parsing"
  - "NodeSession placeholder struct"
affects: [64-02-handshake, 64-03-heartbeat, 65-message-routing]

# Tech tracking
tech-stack:
  added: ["ring 0.17 (direct dep, was transitive)"]
  patterns: ["ephemeral self-signed TLS cert via ring ECDSA P-256", "SkipCertVerification for non-PKI trust", "OnceLock singleton for node state"]

key-files:
  created:
    - "crates/snow-rt/src/dist/node.rs"
  modified:
    - "crates/snow-rt/src/dist/mod.rs"
    - "crates/snow-rt/Cargo.toml"

key-decisions:
  - "ring added as direct dependency (zero compile cost, already transitive via rustls)"
  - "Ephemeral ECDSA P-256 cert generated programmatically with hand-crafted ASN.1 DER (no rcgen dependency)"
  - "SkipCertVerification delegates TLS 1.2/1.3 signature verification to ring default provider"
  - "Non-blocking accept loop with 100ms sleep/shutdown-check pattern"
  - "Port 0 support for OS-assigned ports (actual port stored in NodeState)"

patterns-established:
  - "NodeState OnceLock singleton: mirrors GLOBAL_SCHEDULER/GLOBAL_REGISTRY pattern"
  - "Cookie-based trust model: TLS encrypts, HMAC-SHA256 authenticates (not PKI)"
  - "Ephemeral cert: generated at node startup, never validated by peers"

# Metrics
duration: 5min
completed: 2026-02-13
---

# Phase 64 Plan 01: Node Identity & TLS Infrastructure Summary

**NodeState singleton with ephemeral ECDSA P-256 TLS certificate, TCP listener, and snow_node_start entry point**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-13T04:07:24Z
- **Completed:** 2026-02-13T04:12:34Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- NodeState global singleton holding node identity, TLS configs, and session registry
- Ephemeral ECDSA P-256 self-signed X.509 certificate generation using hand-crafted ASN.1 DER (no external dependency beyond ring)
- TLS ServerConfig (no client auth) and ClientConfig (skip cert verification) for the cookie-based trust model
- TCP listener on background thread with non-blocking accept loop and shutdown flag
- snow_node_start extern "C" entry point callable from compiled Snow programs
- 7 unit tests covering parse, cert generation, TLS config building, and TCP listener startup

## Task Commits

Each task was committed atomically:

1. **Task 1: NodeState singleton with TLS configs and ephemeral certificate** - `80c3299` (feat)
2. **Task 2: TCP listener thread and snow_node_start extern C entry point** - `6c3badb` (test)

## Files Created/Modified
- `crates/snow-rt/src/dist/node.rs` - Node identity, TLS configs, ephemeral cert, TCP listener, snow_node_start
- `crates/snow-rt/src/dist/mod.rs` - Re-exports node module alongside wire
- `crates/snow-rt/Cargo.toml` - Added ring 0.17 as direct dependency
- `Cargo.lock` - Updated lockfile

## Decisions Made
- **ring as direct dependency:** Already compiled as rustls transitive dep; adding it directly costs zero compile time and enables ECDSA key generation.
- **Hand-crafted ASN.1 DER for certificates:** Programmatic DER construction avoids rcgen dependency. The certificate is minimal (CN=snow-node, ECDSA P-256, validity 2020-2099) and only needs to be structurally valid for rustls acceptance.
- **SkipCertVerification with ring signature delegation:** Verifies TLS 1.2/1.3 handshake signatures using ring's default provider while skipping certificate chain validation. Trust comes from cookie authentication in Plan 02.
- **Non-blocking accept loop:** Uses set_nonblocking(true) with 100ms sleep between accept attempts, checking AtomicBool shutdown flag each iteration.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added ring as direct dependency**
- **Found during:** Task 1
- **Issue:** ring is only a transitive dependency of rustls; cannot import ring::signature directly
- **Fix:** Added `ring = "0.17"` to Cargo.toml (zero additional compile cost since already compiled)
- **Files modified:** crates/snow-rt/Cargo.toml, Cargo.lock
- **Verification:** cargo build succeeds
- **Committed in:** 80c3299 (Task 1 commit)

**2. [Rule 1 - Bug] Fixed raw pointer Send safety in accept loop spawn**
- **Found during:** Task 1
- **Issue:** Passing `*const AtomicBool` to spawned thread violates Send bound
- **Fix:** Access NODE_STATE OnceLock from within the thread closure instead of passing raw pointer
- **Files modified:** crates/snow-rt/src/dist/node.rs
- **Verification:** cargo build succeeds
- **Committed in:** 80c3299 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes necessary for correctness. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- NodeState singleton ready for Plan 02 to implement handshake protocol
- TLS server and client configs ready for wrapping TCP streams
- NodeSession placeholder ready to be fleshed out with TLS stream and reader thread
- accept_loop has stub comment marking where Plan 02's handshake code goes

---
*Phase: 64-node-connection-authentication*
*Plan: 01*
*Completed: 2026-02-13*
