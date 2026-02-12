---
phase: 56-https-server
plan: 02
subsystem: http
tags: [https, tls, rustls, server-config, http-stream]

# Dependency graph
requires:
  - phase: 56-https-server
    plan: 01
    provides: "Hand-rolled HTTP/1.1 parser, process_request I/O separation, tiny_http removed"
  - phase: 55-postgresql-tls
    provides: "rustls 0.23 as direct dependency, PgStream enum pattern, CryptoProvider init"
provides:
  - "HttpStream enum (Plain/Tls) for zero-cost HTTP/HTTPS dispatch"
  - "snow_http_serve_tls runtime function with rustls ServerConfig"
  - "build_server_config for PEM cert/key loading"
  - "snow_http_serve_tls codegen intrinsic and MIR mapping"
  - "HTTP.serve_tls(router, port, cert, key) callable from Snow source"
affects: [production-deployment, https-testing]

# Tech tracking
tech-stack:
  added: []
  patterns: ["HttpStream enum mirrors PgStream from Phase 55", "Lazy TLS handshake via StreamOwned::new (no I/O in accept loop)", "Arc::into_raw/from_raw leak pattern for eternal server config"]

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/http/server.rs"
    - "crates/snow-rt/src/http/mod.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/builtins.rs"
    - "crates/snowc/tests/e2e_stdlib.rs"

key-decisions:
  - "HttpStream enum with Plain/Tls variants (same pattern as PgStream from Phase 55)"
  - "Lazy TLS handshake: StreamOwned::new does no I/O, handshake occurs on first read inside actor"
  - "Arc::into_raw leak for ServerConfig since server runs forever (no cleanup needed)"
  - "Read timeout set on raw TcpStream before TLS wrapping (Pitfall 7 from research)"

patterns-established:
  - "HttpStream enum: zero-cost dispatch between plain TCP and TLS connections"
  - "parse_request/write_response generalized to &mut HttpStream (works for both HTTP and HTTPS)"

# Metrics
duration: 7min
completed: 2026-02-12
---

# Phase 56 Plan 02: HTTPS TLS Layer Summary

**HttpStream enum with rustls ServerConfig for HTTPS serving, lazy TLS handshake inside per-connection actors, full codegen pipeline from HTTP.serve_tls to snow_http_serve_tls**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-12T19:07:53Z
- **Completed:** 2026-02-12T19:15:00Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Added HttpStream enum (Plain/Tls) with Read+Write delegation, mirroring PgStream pattern from Phase 55
- Added build_server_config for loading PEM cert/key into rustls ServerConfig
- Refactored parse_request and write_response to accept &mut HttpStream, making them work transparently for both HTTP and HTTPS
- Added snow_http_serve_tls runtime function with lazy TLS handshake (StreamOwned::new does no I/O, handshake runs inside actor)
- Added complete codegen pipeline: intrinsic declaration, known_functions entry, map_builtin_name mapping
- Added type checker entries in infer.rs and builtins.rs so HTTP.serve_tls passes type checking
- Added e2e_http_serve_tls_compile_only test confirming Snow source compiles successfully
- All 8 HTTP E2E tests pass, all 275 snow-rt unit tests pass, all 176 codegen tests pass

## Task Commits

Each task was committed atomically:

1. **Task 1: Add HttpStream enum and refactor server.rs for TLS support** - `3863791` (feat)
2. **Task 2: Add serve_tls codegen intrinsic and MIR mapping** - `70f24ad` (feat)

## Files Created/Modified
- `crates/snow-rt/src/http/server.rs` - HttpStream enum, build_server_config, snow_http_serve_tls, refactored parse_request/write_response/connection_handler_entry
- `crates/snow-rt/src/http/mod.rs` - Updated exports and module doc for HTTPS
- `crates/snow-codegen/src/codegen/intrinsics.rs` - snow_http_serve_tls intrinsic declaration (ptr, i64, ptr, ptr -> void)
- `crates/snow-codegen/src/mir/lower.rs` - known_functions entry and map_builtin_name mapping
- `crates/snow-typeck/src/infer.rs` - HTTP.serve_tls type in stdlib_modules
- `crates/snow-typeck/src/builtins.rs` - http_serve_tls builtin type entry
- `crates/snowc/tests/e2e_stdlib.rs` - e2e_http_serve_tls_compile_only test

## Decisions Made
- Used HttpStream enum with Plain/Tls variants (same pattern as PgStream from Phase 55) for zero-cost dispatch instead of Box<dyn Read+Write>
- TLS handshake is lazy: StreamOwned::new() does no I/O, the actual handshake occurs when parse_request calls BufReader::read_line inside the actor's coroutine -- this satisfies the requirement that handshakes run inside per-connection actors, not in the accept loop
- Used Arc::into_raw/from_raw leak pattern for ServerConfig since the server runs forever (intentional leak, no cleanup needed)
- Set read timeout on raw TcpStream BEFORE wrapping in StreamOwned (Pitfall 7 from research: timeout must be set on underlying socket)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added type checker entries for HTTP.serve_tls**
- **Found during:** Task 2 (compile-only E2E test)
- **Issue:** Plan only specified codegen intrinsic and MIR mapping changes but the type checker (snow-typeck) also needs to know about HTTP.serve_tls for type resolution. Without this, the compiler emits "undefined variable: HTTP" before reaching codegen.
- **Fix:** Added serve_tls entry in snow-typeck/src/infer.rs (stdlib_modules HTTP module) and snow-typeck/src/builtins.rs (builtin environment)
- **Files modified:** crates/snow-typeck/src/infer.rs, crates/snow-typeck/src/builtins.rs
- **Verification:** e2e_http_serve_tls_compile_only test passes
- **Committed in:** 70f24ad (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential for correctness -- without type checker entries, HTTP.serve_tls would fail to compile regardless of codegen changes. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- HTTPS serving is fully wired: Snow source -> type check -> MIR lower -> LLVM intrinsic -> runtime function
- Production use requires valid PEM certificate and key files (e.g., from Let's Encrypt or mkcert for development)
- Phase 56 is complete -- both HTTP parser (Plan 01) and HTTPS TLS layer (Plan 02) are implemented
- The server shares all infrastructure (actor system, router, middleware, request/response) between HTTP and HTTPS

## Self-Check: PASSED

- All key files exist (server.rs, mod.rs, intrinsics.rs, lower.rs, infer.rs, builtins.rs)
- Commit 3863791 exists in git log
- Commit 70f24ad exists in git log
- HttpStream enum present in server.rs with Plain and Tls variants
- build_server_config present in server.rs
- snow_http_serve_tls present in server.rs
- snow_http_serve_tls intrinsic declared in intrinsics.rs
- http_serve_tls -> snow_http_serve_tls mapping in lower.rs
- All 8 HTTP E2E tests pass
- All 275 snow-rt unit tests pass
- All 176 snow-codegen tests pass

---
*Phase: 56-https-server*
*Completed: 2026-02-12*
