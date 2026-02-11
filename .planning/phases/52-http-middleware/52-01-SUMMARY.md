---
phase: 52-http-middleware
plan: 01
subsystem: http
tags: [middleware, http, router, chain, closure, trampoline, llvm, codegen]

# Dependency graph
requires:
  - phase: 51-http-path-parameters
    provides: "Method-specific routing, path parameter extraction, SnowRouter with RouteEntry"
provides:
  - "MiddlewareEntry struct and middlewares Vec on SnowRouter"
  - "snow_http_use_middleware runtime function (immutable router copy with middleware appended)"
  - "Middleware chain execution via chain_next trampoline with Snow closure construction"
  - "Full compiler pipeline: HTTP.use(router, fn) compiles end-to-end"
  - "Middleware runs on both matched routes and 404 responses"
affects: [52-02-http-middleware-e2e-test, http-server, router]

# Tech tracking
tech-stack:
  added: []
  patterns: [middleware-chain-trampoline, snow-closure-construction, onion-model-middleware]

key-files:
  created: []
  modified:
    - crates/snow-rt/src/http/router.rs
    - crates/snow-rt/src/http/server.rs
    - crates/snow-rt/src/http/mod.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs

key-decisions:
  - "Middleware fn_ptr passed as single MirType::Ptr (no closure splitting) matching existing handler pattern"
  - "chain_next trampoline builds Snow closure via GC-allocated {fn_ptr, env_ptr} struct for next function"
  - "Synthetic 404 handler wrapped in middleware chain when middleware is registered but no route matches"

patterns-established:
  - "Middleware chain trampoline: chain_next recursively wraps each middleware with a new Snow closure for next"
  - "build_snow_closure: GC-allocate 16-byte closure struct matching Snow codegen layout"

# Metrics
duration: 4min
completed: 2026-02-11
---

# Phase 52 Plan 01: HTTP Middleware Runtime and Compiler Pipeline Summary

**Onion-model middleware chain with trampoline-based next function, Snow closure construction, and full compiler pipeline wiring for HTTP.use(router, fn)**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-11T17:44:11Z
- **Completed:** 2026-02-11T17:49:09Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- SnowRouter extended with MiddlewareEntry storage and immutable-copy registration via snow_http_use_middleware
- Middleware chain execution via chain_next trampoline that constructs Snow-compatible closure structs for the next function
- Middleware wraps both matched route handlers and synthetic 404 handlers (runs on every request)
- Fast path preserved: routers without middleware skip chain construction entirely
- Full compiler pipeline wired: intrinsics, known_functions, map_builtin_name, builtins, and infer module all register HTTP.use

## Task Commits

Each task was committed atomically:

1. **Task 1: Runtime -- middleware storage, registration, and chain execution** - `4bf8b2c` (feat)
2. **Task 2: Compiler pipeline -- intrinsics, type checker, and MIR lowering** - `4976135` (feat)

## Files Created/Modified
- `crates/snow-rt/src/http/router.rs` - MiddlewareEntry struct, middlewares Vec on SnowRouter, snow_http_use_middleware function
- `crates/snow-rt/src/http/server.rs` - ChainState, chain_next trampoline, build_snow_closure, call_handler, call_middleware, middleware-aware handle_request
- `crates/snow-rt/src/http/mod.rs` - Re-export snow_http_use_middleware
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM declaration of snow_http_use_middleware (ptr, ptr) -> ptr
- `crates/snow-codegen/src/mir/lower.rs` - known_functions entry and map_builtin_name for http_use -> snow_http_use_middleware
- `crates/snow-typeck/src/builtins.rs` - Type registration for http_use: (Router, Fn(Request, Fn(Request) -> Response) -> Response) -> Router
- `crates/snow-typeck/src/infer.rs` - HTTP module "use" entry with matching type signature

## Decisions Made
- Middleware function passed as single MirType::Ptr (no closure splitting at codegen) -- same pattern as HTTP.route handler
- chain_next trampoline uses Box::into_raw for ChainState and snow_gc_alloc_actor for Snow closure construction
- Synthetic 404 handler wraps middleware chain for unmatched routes, ensuring middleware runs on every request

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- HTTP.use compiles from Snow source through the full pipeline to LLVM calls
- Middleware chain executes at runtime with correct onion-model composition
- Ready for E2E testing in plan 52-02

---
*Phase: 52-http-middleware*
*Completed: 2026-02-11*
