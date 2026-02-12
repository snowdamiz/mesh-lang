---
phase: 55-postgresql-tls
plan: 01
subsystem: database
tags: [tls, rustls, postgresql, ssl, webpki, ring]

# Dependency graph
requires:
  - phase: 54-postgresql
    provides: "PostgreSQL wire protocol v3 client (TcpStream-based PgConn, SCRAM/MD5 auth, Extended Query)"
provides:
  - "PgStream enum abstracting Plain/Tls TCP connections"
  - "SSLRequest protocol handshake (sslmode=disable/prefer/require)"
  - "URL query string parsing for sslmode parameter"
  - "Ring CryptoProvider installation at runtime startup"
  - "TLS upgrade via rustls with webpki root certificates"
affects: [56-https-server, 57-connection-pool, 58-transactions]

# Tech tracking
tech-stack:
  added: [rustls 0.23, webpki-roots 0.26, rustls-pki-types 1]
  patterns: [enum-based stream abstraction, SSLRequest protocol negotiation, idempotent CryptoProvider install]

key-files:
  created: []
  modified:
    - crates/snow-rt/Cargo.toml
    - crates/snow-rt/src/gc.rs
    - crates/snow-rt/src/db/pg.rs

key-decisions:
  - "Used PgStream enum (Plain/Tls variants) instead of Box<dyn Read+Write> for zero-cost dispatch"
  - "Default sslmode=prefer for backward compatibility (tries TLS, falls back to plain)"
  - "CryptoProvider installed in snow_rt_init() to guarantee availability before any TLS operation"
  - "All three deps already compiled as transitive deps of ureq -- zero additional compile time"

patterns-established:
  - "PgStream enum pattern: all wire protocol functions operate on PgStream, not TcpStream"
  - "SSLRequest before StartupMessage: timeouts set on raw TcpStream before TLS wrapping"
  - "CVE-2021-23222 mitigation: read exactly 1 byte after SSLRequest, never more"

# Metrics
duration: 6min
completed: 2026-02-12
---

# Phase 55 Plan 01: PostgreSQL TLS Summary

**PgStream enum abstraction with SSLRequest handshake, sslmode URL parsing, and ring CryptoProvider for cloud PostgreSQL TLS connections**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-12T18:29:01Z
- **Completed:** 2026-02-12T18:35:27Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Added rustls/webpki-roots/rustls-pki-types as direct dependencies (zero compile cost, already transitive via ureq)
- Installed ring CryptoProvider in snow_rt_init() for both PostgreSQL TLS and ureq HTTP client
- Implemented PgStream enum with Read+Write delegation for transparent Plain/Tls stream handling
- Added SSLRequest protocol negotiation with sslmode=disable/prefer/require support
- Refactored all wire protocol functions to use PgStream instead of TcpStream
- URL parser handles ?sslmode= query parameter with Prefer as default for backward compatibility

## Task Commits

Each task was committed atomically:

1. **Task 1: Add TLS dependencies and install CryptoProvider** - `f663021` (chore)
2. **Task 2: Implement PgStream, SSLRequest, sslmode parsing, refactor connection flow** - `3d43e7c` (feat)

## Files Created/Modified
- `crates/snow-rt/Cargo.toml` - Added rustls, webpki-roots, rustls-pki-types direct dependencies
- `crates/snow-rt/src/gc.rs` - Ring CryptoProvider installation in snow_rt_init()
- `crates/snow-rt/src/db/pg.rs` - PgStream enum, SslMode enum, SSLRequest handshake, TLS upgrade, sslmode URL parsing, refactored connection flow

## Decisions Made
- Used PgStream enum with Plain/Tls variants for zero-cost dispatch (no dynamic dispatch overhead vs Box<dyn Read+Write>)
- Default sslmode=prefer ensures backward compatibility -- existing v2.0 URLs without ?sslmode= try TLS but fall back to plain
- CryptoProvider installed in snow_rt_init() rather than lazily, guaranteeing it runs before any TLS operation
- Used webpki root certificates (Mozilla CA bundle) for server certificate validation

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- TLS infrastructure complete, Snow programs can now connect to cloud PostgreSQL providers (AWS RDS, Supabase, Neon)
- Connection URL format: `postgres://user:pass@host:5432/db?sslmode=require`
- Ready for Phase 56 (HTTPS server), Phase 57 (connection pool), Phase 58 (transactions)

## Self-Check: PASSED

All files verified present. Both task commits (f663021, 3d43e7c) verified in git log.

---
*Phase: 55-postgresql-tls*
*Completed: 2026-02-12*
