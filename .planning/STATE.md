# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-12)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v4.0 WebSocket Support -- Phase 60 Actor Integration COMPLETE

## Current Position

Phase: 60 of 62 (Actor Integration) -- COMPLETE
Plan: 2 of 2 in current phase -- COMPLETE
Status: Phase 60 verified and complete, ready for Phase 61
Last activity: 2026-02-12 -- Phase 60 verified (12/12 must-haves, 5 integration tests)

Progress: [█████░░░░░] 50%

## Performance Metrics

**All-time Totals:**
- Plans completed: 166
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

### Research Notes

- Reader thread bridge (novel architecture) is highest risk -- Phase 60 DONE
- Critical pitfalls: blocking reader thread, mailbox type tag collision, masking direction -- addressed in Phases 59-60
- TLS reuses existing rustls infrastructure (low risk) -- Phase 61
- Rooms follow existing process registry pattern (medium risk) -- Phase 62
- sha1 0.10 is the only new dependency needed

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-12
Stopped at: Phase 60 complete and verified with integration tests
Resume file: None
Next action: Plan Phase 61 (Production Hardening)
