# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-12)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v3.0 Production Backend -- Phase 57 (Connection Pooling & Transactions)

## Current Position

Phase: 57 of 58 (Connection Pooling & Transactions)
Plan: 3 of 3 in current phase (COMPLETE)
Status: Phase Complete
Last activity: 2026-02-12 -- Plan 57-03 complete (compiler pipeline wiring)

Progress: [██████████] 100%

## Performance Metrics

**All-time Totals:**
- Plans completed: 160
- Phases completed: 57 (including Phase 57 Connection Pooling & Transactions)
- Milestones shipped: 11 (v1.0-v2.0)
- Lines of Rust: 81,006
- Timeline: 8 days (2026-02-05 -> 2026-02-12)

## Accumulated Context

### Decisions

- Phase 55-01: Used PgStream enum (Plain/Tls) for zero-cost dispatch instead of Box<dyn Read+Write>
- Phase 55-01: Default sslmode=prefer for backward compatibility with existing v2.0 URLs
- Phase 55-01: CryptoProvider installed in snow_rt_init() to guarantee pre-TLS availability
- [Phase 56]: BufReader<&mut TcpStream> for parse-then-write pattern (borrow, don't consume)
- [Phase 56]: process_request returns (u16, Vec<u8>) tuple for I/O separation (enables TLS reuse in Plan 02)
- Phase 56-02: HttpStream enum (Plain/Tls) for zero-cost HTTP/HTTPS dispatch (mirrors PgStream pattern)
- Phase 56-02: Lazy TLS handshake via StreamOwned::new (no I/O in accept loop, handshake inside actor)
- Phase 56-02: Arc::into_raw leak for eternal ServerConfig (server runs forever, no cleanup needed)
- Phase 57-01: Simple Query protocol for BEGIN/COMMIT/ROLLBACK (simpler than Extended Query, no params needed)
- Phase 57-01: SnowResult tag read via struct cast, not raw u64 pointer read (tag is u8)
- Phase 57-01: sqlite3_exec FFI for bare SQL instead of prepare/step/finalize
- Phase 57-02: parking_lot::Mutex + Condvar pool (not std::sync) for consistency with scheduler
- Phase 57-02: Health check via pg_simple_command("SELECT 1") on checkout, reusing Plan 01 helper
- Phase 57-02: Optimistic slot reservation before dropping lock for connection creation I/O
- Phase 57-03: Pg.transaction uses mono Result<Unit, String> for callback (sufficient for most use cases)
- Phase 57-03: PoolHandle follows opaque u64 handle pattern (MirType::Int) like PgConn/SqliteConn
- Phase 57-03: Pool.checkout returns PgConn specifically (PG-focused pool in v3.0)

### Research Notes

v3.0 research completed (see .planning/research/SUMMARY.md):
- All TLS deps (rustls 0.23, ring, webpki-roots) already compiled as transitive deps of ureq 2
- tiny_http must be replaced (uses rustls 0.20, incompatible with 0.23)
- Pool + transactions are deeply coupled (pool must validate transaction state on checkin)
- Phase 57 needs research-phase for actor scheduler + pool checkout interaction
- Phase 56 may need research-phase for HTTPS thread pool architecture

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-12
Stopped at: Completed 57-03-PLAN.md (compiler pipeline wiring) -- Phase 57 complete
Resume file: None
Next action: Phase 58 or milestone completion
