---
phase: 51-http-path-parameters
plan: 01
subsystem: http
tags: [http, router, path-params, rest, method-routing]

# Dependency graph
requires:
  - phase: 08-stdlib
    provides: "HTTP router, server, client, request accessors, SnowMap, SnowOption"
provides:
  - "Segment-based path parameter matching with :param capture"
  - "Method-specific routing (GET/POST/PUT/DELETE) via RouteEntry.method field"
  - "Request.param accessor for extracting path parameters"
  - "HTTP.on_get/on_post/on_put/on_delete Snow-level API"
  - "Full compiler pipeline: intrinsics, typeck, MIR lowering for 5 new runtime functions"
affects: [51-02-e2e-tests, http-middleware]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Two-pass route matching (exact > parameterized) for priority ordering"
    - "HTTP.on_* naming convention for method-specific routing (avoids HTTP.get/post client collision)"

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/http/router.rs"
    - "crates/snow-rt/src/http/server.rs"
    - "crates/snow-rt/src/http/mod.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-typeck/src/builtins.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"

key-decisions:
  - "Use HTTP.on_get/on_post/on_put/on_delete naming instead of arity-based dispatch for HTTP.get/post to avoid typeck collision with existing client functions"
  - "Two-pass matching (exact/wildcard first, then parameterized) for automatic priority without requiring registration order"
  - "Path params stored as SnowMap on SnowHttpRequest (appended field for repr(C) safety)"

patterns-established:
  - "Method-specific routing uses on_* prefix: HTTP.on_get, HTTP.on_post, HTTP.on_put, HTTP.on_delete"
  - "Route matching returns 3-tuple (handler_fn, handler_env, params Vec) instead of 2-tuple"

# Metrics
duration: 5min
completed: 2026-02-11
---

# Phase 51 Plan 01: HTTP Path Parameters & Method Routing Summary

**Segment-based path parameter matching with :param capture, method-specific routing via RouteEntry.method field, and Request.param accessor across runtime and full compiler pipeline**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-11T16:59:35Z
- **Completed:** 2026-02-11T17:05:10Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Router supports segment-based matching: `/users/:id` matches `/users/42` and captures `id=42`
- Two-pass priority matching ensures exact routes beat parameterized regardless of registration order
- Method-specific routing via 4 new extern functions (`snow_http_route_get/post/put/delete`)
- `Request.param(req, "id")` accessor returns `Option<String>` following existing `Request.query` pattern
- Full compiler pipeline wired: LLVM intrinsics, typeck builtins + module system, MIR known_functions + map_builtin_name
- Existing `HTTP.route` (method-agnostic) and `HTTP.get`/`HTTP.post` (client) fully backward compatible

## Task Commits

Each task was committed atomically:

1. **Task 1: Runtime -- path parameter matching, method-specific routing, and request param accessor** - `b4b3fcd` (feat)
2. **Task 2: Compiler pipeline -- intrinsics, type checker, and MIR lowering** - `e98b36f` (feat)

## Files Created/Modified
- `crates/snow-rt/src/http/router.rs` - Segment-based matching, method field on RouteEntry, two-pass priority, 4 method-specific extern functions
- `crates/snow-rt/src/http/server.rs` - path_params field on SnowHttpRequest, snow_http_request_param accessor, match_route updated to pass method and handle 3-tuple return
- `crates/snow-rt/src/http/mod.rs` - Re-exports for 5 new public functions
- `crates/snow-codegen/src/codegen/intrinsics.rs` - 5 LLVM function declarations for route_get/post/put/delete and request_param
- `crates/snow-typeck/src/builtins.rs` - Type registrations for http_on_get/post/put/delete and request_param
- `crates/snow-typeck/src/infer.rs` - HTTP module entries for on_get/on_post/on_put/on_delete, Request module entry for param
- `crates/snow-codegen/src/mir/lower.rs` - known_functions for 5 new runtime functions, map_builtin_name for http_on_get -> snow_http_route_get etc.

## Decisions Made
- **HTTP.on_* naming over arity-based dispatch:** The plan identified a naming collision between `HTTP.get(url)` (client, 1 arg) and `HTTP.get(router, path, handler)` (routing, 3 args). Arity-based dispatch would require surgery across both the type checker (which uses a single HashMap per module name) and MIR lowering. Using `HTTP.on_get`/`HTTP.on_post`/`HTTP.on_put`/`HTTP.on_delete` avoids all collision cleanly with no loss of functionality. The plan explicitly sanctioned this as the fallback approach.
- **Two-pass matching for priority:** Routes are partitioned into exact/wildcard (first pass) and parameterized (second pass), giving automatic exact > parameterized priority regardless of registration order.
- **path_params as SnowMap appended to SnowHttpRequest:** Follows the exact pattern of query_params and headers, appended at the end for repr(C) layout safety.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed borrow conflict in handle_request**
- **Found during:** Task 1 (server.rs changes)
- **Issue:** `method_str` borrowed `request` immutably via `request.method().as_str()`, but later `request.as_reader()` needed a mutable borrow, and `method_str` was still used when calling `router.match_route(path_str, method_str)`
- **Fix:** Clone method_str to owned String with `.to_string()` to break the borrow
- **Files modified:** crates/snow-rt/src/http/server.rs
- **Verification:** Compilation succeeds, all tests pass
- **Committed in:** b4b3fcd (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Trivial borrow-checker fix necessary for compilation. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Runtime and compiler pipeline fully wired for path parameters and method-specific routing
- Ready for Plan 02 (E2E integration tests) to verify the full pipeline from Snow source to running HTTP server
- Snow surface syntax: `HTTP.on_get(r, "/users/:id", handler)` and `Request.param(req, "id")`

## Self-Check: PASSED

All 7 modified files verified present. Both task commits (b4b3fcd, e98b36f) verified in git log.

---
*Phase: 51-http-path-parameters*
*Completed: 2026-02-11*
