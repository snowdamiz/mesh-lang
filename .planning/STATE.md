# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-12)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v5.0 Distributed Actors -- Phase 64: Node Connection & Authentication

## Current Position

Phase: 64 of 69 (Node Connection & Authentication)
Plan: 1 of 3 in current phase
Status: Executing
Last activity: 2026-02-13 -- Completed 64-01 (Node Identity & TLS Infrastructure)

Progress: [██████████] 100%

## Performance Metrics

**All-time Totals:**
- Plans completed: 174
- Phases completed: 63
- Milestones shipped: 14 (v1.0-v4.0)
- Lines of Rust: ~84,400
- Timeline: 8 days (2026-02-05 -> 2026-02-13)

## Accumulated Context

### Decisions

- 63-01: Mask PID counter to 40 bits defensively (prevents silent corruption at 2^40)
- 63-01: Display format <0.N> for local PIDs (backward compat), <node.N.creation> for remote
- 63-01: dist_send_stub silently drops (no panic) -- remote PIDs unreachable in Phase 63
- 63-02: UTF-8 validation on string decode (reject invalid wire data, not trust)
- 63-02: Container/composite stubs return InvalidTag(0) for Plan 03 to replace
- 63-03: Inline pointer math for collection layout reading (no private imports)
- 63-03: Recursive encode/decode (shallow nesting typical for messages)
- 63-03: MAX_NAME_LEN (u16::MAX) for struct/sum type field name bounds
- 64-01: ring added as direct dep (zero compile cost, enables ECDSA key gen for ephemeral certs)
- 64-01: Hand-crafted ASN.1 DER for minimal self-signed cert (no rcgen dependency)
- 64-01: SkipCertVerification with ring signature delegation (cookie-based trust model)
- 64-01: Non-blocking accept loop with 100ms sleep/shutdown-check pattern

### Research Notes

- PID encoding: 16-bit node_id in upper bits of existing u64 (backward compatible)
- Wire format: Custom Snow Term Format (STF), not Erlang ETF
- Auth: HMAC-SHA256 challenge/response using existing sha2+hmac crates
- Zero new crate dependencies for entire milestone
- Reader-thread-bridge pattern from WebSocket reused for NodeSession

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-13
Stopped at: Completed 64-01-PLAN.md (Node Identity & TLS Infrastructure)
Resume file: None
Next action: Execute 64-02-PLAN.md (Handshake Protocol)
