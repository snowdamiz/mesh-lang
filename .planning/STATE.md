# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-12)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v4.0 WebSocket Support -- Phase 61 Production Hardening IN PROGRESS

## Current Position

Phase: 61 of 62 (Production Hardening) -- IN PROGRESS
Plan: 1 of 2 in current phase -- COMPLETE
Status: Plan 61-01 complete (TLS, heartbeat, fragmentation), ready for Plan 61-02
Last activity: 2026-02-12 -- Plan 61-01 executed (2 tasks, 4 files, 323 tests pass)

Progress: [██████░░░░] 55%

## Performance Metrics

**All-time Totals:**
- Plans completed: 167
- Phases completed: 60
- Milestones shipped: 12 (v1.0-v3.0)
- Lines of Rust: ~84,000
- Timeline: 8 days (2026-02-05 -> 2026-02-12)

## Accumulated Context

### Decisions

- [59-01] 64 MiB payload safety cap to prevent OOM; Phase 61 will tighten to 16 MiB
- [59-01] Frame codec uses read_exact on raw stream (no BufReader) to avoid buffering issues at protocol boundary
- [59-02] BufReader used for HTTP header parsing in perform_upgrade with explicit buffer-empty sanity check
- [59-02] process_frame echoes close code only (no reason) to minimize control frame size
- [59-02] Continuation opcode passed through -- Phase 61 handles reassembly
- [60-01] Modified perform_upgrade in-place to return (path, headers) instead of new function
- [60-01] Reserved type tags u64::MAX-1 through u64::MAX-4 for WS mailbox messages
- [60-01] Reader thread uses 5-second read timeout for periodic shutdown check
- [60-01] Both reader thread and actor share Arc<Mutex<TcpStream>> for writes to prevent frame interleaving
- [60-01] WsConnection stored on Rust heap via Box::into_raw, not GC heap
- [60-02] snow_ws_send known_functions uses Ptr (not MirType::String) for SnowString pointer, matching extern C signature convention
- [61-01] Unified Arc<Mutex<WsStream>> for both plain and TLS (replaces try_clone)
- [61-01] 100ms reader thread timeout balances mutex contention and responsiveness
- [61-01] build_server_config made pub(crate) for cross-module reuse (HTTP + WS TLS)
- [61-01] Pong handled before process_frame to validate heartbeat payload
- [61-01] MAX_PAYLOAD_SIZE reduced from 64 MiB to 16 MiB (supersedes [59-01] cap)
- [61-01] macOS EAGAIN detection added to timeout checks for short read timeouts

### Research Notes

- Reader thread bridge (novel architecture) is highest risk -- Phase 60 DONE
- TLS reuses existing rustls infrastructure (low risk) -- Phase 61 DONE (plan 01)
- Rooms follow existing process registry pattern (medium risk) -- Phase 62
- sha1 0.10 is the only new dependency needed

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-12
Stopped at: Completed 61-01-PLAN.md (TLS, heartbeat, fragmentation)
Resume file: None
Next action: Execute Plan 61-02 (codegen wiring for Ws.serve_tls)
