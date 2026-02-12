---
phase: 56-https-server
plan: 01
subsystem: http
tags: [http-parser, tcp, tiny-http-removal, rustls-compat]

# Dependency graph
requires:
  - phase: 55-postgresql-tls
    provides: "rustls 0.23 as direct dependency, CryptoProvider initialization"
provides:
  - "Hand-rolled HTTP/1.1 request parser (parse_request)"
  - "HTTP/1.1 response writer (write_response)"
  - "TcpListener-based server loop replacing tiny_http"
  - "process_request function separating routing from I/O"
  - "Dependency tree free of rustls 0.20 conflict"
affects: [56-02-PLAN, https-server-tls]

# Tech tracking
tech-stack:
  added: []
  patterns: ["BufReader<&mut TcpStream> for parse-then-write pattern", "process_request returns (status, body) tuple for I/O separation"]

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/http/server.rs"
    - "crates/snow-rt/src/http/mod.rs"
    - "crates/snow-rt/Cargo.toml"
    - "Cargo.lock"

key-decisions:
  - "BufReader<&mut TcpStream> instead of BufReader<TcpStream> to allow writing response on same stream after parsing"
  - "process_request returns (u16, Vec<u8>) tuple instead of writing response directly, enabling clean I/O separation"
  - "30-second read timeout on accepted connections to prevent actor starvation"
  - "8KB header limit and 100 header max for DoS protection"

patterns-established:
  - "parse_request/write_response pair: parse from &mut TcpStream, write to same stream after parsing"
  - "process_request separates routing/handler logic from I/O, returns (status, body) for any stream type"

# Metrics
duration: 3min
completed: 2026-02-12
---

# Phase 56 Plan 01: HTTP Parser Summary

**Hand-rolled HTTP/1.1 request parser and response writer replacing tiny_http, eliminating rustls 0.20 version conflict**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-12T19:01:16Z
- **Completed:** 2026-02-12T19:04:35Z
- **Tasks:** 1
- **Files modified:** 4

## Accomplishments
- Removed tiny_http dependency entirely, eliminating rustls 0.20 transitive dependency that conflicted with rustls 0.23
- Implemented parse_request() with BufReader for line-by-line header parsing and Content-Length body reading
- Implemented write_response() with proper HTTP/1.1 status line, Content-Type, Content-Length, and Connection: close headers
- Rewrote snow_http_serve to use std::net::TcpListener::bind directly instead of tiny_http::Server::http
- Extracted process_request() from handle_request() to cleanly separate routing logic from I/O (returns status+body tuple)
- All 7 HTTP E2E tests pass without modification (server, crash isolation, path params, middleware, compile-only)

## Task Commits

Each task was committed atomically:

1. **Task 1: Remove tiny_http and rewrite server.rs** - `dec76d7` (feat)

## Files Created/Modified
- `crates/snow-rt/Cargo.toml` - Removed tiny_http = "0.12" dependency
- `crates/snow-rt/src/http/server.rs` - Rewritten: parse_request, write_response, process_request, TcpListener server loop
- `crates/snow-rt/src/http/mod.rs` - Updated module doc comment to remove tiny_http references
- `Cargo.lock` - Updated lockfile (tiny_http and transitive deps removed)

## Decisions Made
- Used `BufReader<&mut TcpStream>` (borrowing reference) instead of `BufReader<TcpStream>` (consuming ownership) so the stream remains available for write_response after parsing completes
- Separated process_request from I/O: returns `(u16, Vec<u8>)` tuple rather than writing to stream directly, enabling Plan 02 to reuse the same function over TLS streams
- Set 30-second read timeout on accepted TCP connections to prevent slow/malicious clients from blocking actors indefinitely
- Added 8KB header section limit and 100 header max for basic DoS protection

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- process_request and write_response are stream-type agnostic, ready for Plan 02's TLS integration
- The HttpStream enum (Plan 02) only needs to implement Read + Write, then parse_request/write_response can be generalized to accept any stream type
- tiny_http is fully removed from the dependency tree, no rustls version conflicts remain

## Self-Check: PASSED

- All key files exist (server.rs, mod.rs, Cargo.toml)
- Commit dec76d7 exists in git log
- tiny_http removed from Cargo.toml
- parse_request, write_response, process_request functions present in server.rs
- TcpListener::bind used in server.rs
- All 7 HTTP E2E tests pass
- All 275 snow-rt unit tests pass

---
*Phase: 56-https-server*
*Completed: 2026-02-12*
