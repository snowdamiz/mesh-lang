---
phase: 08-standard-library
plan: 05
subsystem: api
tags: [http, tiny-http, ureq, router, server, client, request, response]

# Dependency graph
requires:
  - phase: 08-03
    provides: File I/O and SnowResult/SnowOption types for HTTP client returns
  - phase: 08-04
    provides: JSON encode/decode for API responses
provides:
  - HTTP server with routing (exact + wildcard patterns)
  - HTTP request/response types with accessor functions
  - HTTP client (GET/POST) returning SnowResult
  - Full compiler pipeline for HTTP module (typeck, MIR, codegen)
affects: [09-production-hardening, 10-ecosystem]

# Tech tracking
tech-stack:
  added: [tiny_http 0.12, ureq 2]
  patterns: [thread-per-connection HTTP server, opaque Ptr types for HTTP, module-qualified access for HTTP/Request]

key-files:
  created:
    - crates/snow-rt/src/http/mod.rs
    - crates/snow-rt/src/http/server.rs
    - crates/snow-rt/src/http/router.rs
    - crates/snow-rt/src/http/client.rs
    - tests/e2e/stdlib_http_response.snow
    - tests/e2e/stdlib_http_client.snow
  modified:
    - crates/snow-rt/Cargo.toml
    - crates/snow-rt/src/lib.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/mir/types.rs
    - crates/snowc/tests/e2e_stdlib.rs

key-decisions:
  - "Thread-per-connection instead of actor-per-connection for HTTP server (pragmatic; actor runtime uses coroutines not suited for blocking I/O)"
  - "3-arg snow_http_route (router, pattern, handler_fn) with null env -- bare function handlers only in Phase 8"
  - "No bare name mappings for HTTP/Request functions (router, route, get, post, method, path, body) to avoid collision with common variable names"
  - "Router, Request, Response resolve to MirType::Ptr (opaque pointers at LLVM level)"

patterns-established:
  - "HTTP module pattern: HTTP.router(), HTTP.route(r, path, handler), HTTP.serve(r, port)"
  - "Request accessor pattern: Request.method(req), Request.path(req), Request.body(req)"
  - "Handler calling convention: fn_ptr(request) -> response for bare functions, fn_ptr(env, request) -> response for closures"

# Metrics
duration: 13min
completed: 2026-02-07
---

# Phase 8 Plan 5: HTTP Server/Client Summary

**HTTP server with tiny_http routing, request/response types, ureq client, and full compiler pipeline from Snow source to native binary**

## Performance

- **Duration:** 13 min
- **Started:** 2026-02-07T06:17:00Z
- **Completed:** 2026-02-07T06:29:35Z
- **Tasks:** 2
- **Files modified:** 16

## Accomplishments

- HTTP server runtime with SnowRouter (exact + wildcard URL pattern matching), SnowHttpRequest/Response structs, and thread-per-connection model
- HTTP client with GET/POST using ureq, returning SnowResult (Ok/Err) with response body or error message
- Full compiler pipeline: type checker (Router/Request/Response types + 11 functions), LLVM intrinsics (12 function declarations), MIR lowering (known_functions + name mappings)
- 3 E2E tests verifying HTTP server compilation, client compilation+run, and full server with request accessors
- All 25 E2E stdlib tests pass, full workspace (758+ tests) green

## Task Commits

Each task was committed atomically:

1. **Task 1: HTTP runtime -- server, router, request/response, and client** - `57d99e6` (feat)
2. **Task 2: Compiler pipeline and E2E tests for HTTP** - `393904f` (feat)

## Files Created/Modified

- `crates/snow-rt/src/http/mod.rs` - HTTP module entrypoint with re-exports
- `crates/snow-rt/src/http/router.rs` - SnowRouter with exact/wildcard URL pattern matching
- `crates/snow-rt/src/http/server.rs` - HTTP server (tiny_http), request/response structs, accessors
- `crates/snow-rt/src/http/client.rs` - HTTP GET/POST client (ureq) returning SnowResult
- `crates/snow-rt/Cargo.toml` - Added tiny_http 0.12 and ureq 2 dependencies
- `crates/snow-rt/src/lib.rs` - Added http module and re-exports
- `crates/snow-typeck/src/builtins.rs` - Registered HTTP types and 11 functions
- `crates/snow-typeck/src/infer.rs` - Added HTTP and Request modules to stdlib_modules()
- `crates/snow-codegen/src/codegen/intrinsics.rs` - Added 12 LLVM function declarations
- `crates/snow-codegen/src/mir/lower.rs` - Added known_functions and name mappings for HTTP
- `crates/snow-codegen/src/mir/types.rs` - Router/Request/Response resolve to MirType::Ptr
- `crates/snowc/tests/e2e_stdlib.rs` - Added compile_only helper and 3 HTTP E2E tests
- `tests/e2e/stdlib_http_response.snow` - Server compilation test fixture
- `tests/e2e/stdlib_http_client.snow` - Client compilation+run test fixture

## Decisions Made

1. **Thread-per-connection server model** - Used `std::thread::spawn` instead of `snow_actor_spawn` for HTTP connections. The actor runtime uses corosensei coroutines with cooperative scheduling, and integrating tiny-http's blocking I/O model with it introduces unnecessary complexity. Thread-per-connection is simple and correct.

2. **3-argument snow_http_route** - Changed from 4 args (router, pattern, fn_ptr, env_ptr) to 3 args (router, pattern, fn_ptr) with env always null internally. The codegen passes function references as pointer arguments directly, and closure support for route handlers is deferred beyond Phase 8.

3. **No bare name mappings for HTTP/Request** - Removed generic bare names like `"path"`, `"body"`, `"method"`, `"get"`, `"post"` from map_builtin_name to avoid catastrophic collisions with user variable names (e.g., `let path = "/tmp/..."` in File tests was being rewritten to `snow_http_request_path`). Users must use module-qualified access: `HTTP.router()`, `Request.method(req)`.

4. **Opaque Ptr types** - Router, Request, Response added to the `resolve_con` list alongside List, Map, Set, etc. as MirType::Ptr. These are all opaque heap-allocated structs at the LLVM level.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed bare name collision in map_builtin_name**
- **Found during:** Task 2 (compiler pipeline)
- **Issue:** Bare names "path", "body", "method", "get", "post", "query" collided with common variable names in existing test fixtures, causing `let path = "..."` to be rewritten as `snow_http_request_path` -- breaking all File I/O E2E tests
- **Fix:** Removed all generic bare name mappings for HTTP/Request. Only prefixed names (`http_router`, `request_method`, etc.) are mapped. Module-qualified access handles the rest.
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Verification:** All 25 E2E tests pass including File I/O tests
- **Committed in:** 393904f (Task 2 commit)

**2. [Rule 1 - Bug] Fixed *mut u8 Send trait requirement for thread::spawn**
- **Found during:** Task 1 (HTTP runtime)
- **Issue:** `std::thread::spawn` requires closure to be Send, but raw `*mut u8` router pointer is not Send
- **Fix:** Cast router pointer to usize (which is Send) before spawning thread, cast back inside thread
- **Files modified:** crates/snow-rt/src/http/server.rs
- **Verification:** cargo test -p snow-rt passes (199 tests)
- **Committed in:** 57d99e6 (Task 1 commit)

**3. [Rule 2 - Missing Critical] Added Router/Request/Response to MirType::Ptr opaque type list**
- **Found during:** Task 2 (compiler pipeline)
- **Issue:** LLVM verification failed with "Cannot allocate unsized type %Router" -- these types were falling through to MirType::Struct instead of MirType::Ptr
- **Fix:** Added "Router" | "Request" | "Response" to the resolve_con opaque types list in types.rs
- **Files modified:** crates/snow-codegen/src/mir/types.rs
- **Verification:** HTTP server program compiles successfully
- **Committed in:** 393904f (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 missing critical)
**Impact on plan:** All auto-fixes necessary for correct compilation. No scope creep.

## Issues Encountered

- Multiline pipe operator (`|>` at start of continuation line) fails to parse -- the Snow parser requires pipe expressions on a single line. Changed E2E test to use explicit let bindings instead of multiline pipe chains. This is a pre-existing parser limitation, not introduced by this plan.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- All 4 Phase 8 success criteria met:
  1. File read + process + write (tested in 08-03)
  2. HTTP server with JSON responses (this plan -- verified compilation)
  3. List map/filter/reduce with pipe chains (tested in 08-02)
  4. print/IO.read_line available (tested in 08-01)
- Phase 8 standard library is complete
- Ready for Phase 9 (production hardening) or Phase 10 (ecosystem)
- HTTP module provides foundation for web application development

## Self-Check: PASSED

---
*Phase: 08-standard-library*
*Completed: 2026-02-07*
