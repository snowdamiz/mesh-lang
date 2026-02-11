---
phase: 51-http-path-parameters
plan: 02
subsystem: http
tags: [http, e2e-test, path-params, method-routing, route-priority]

# Dependency graph
requires:
  - phase: 51-http-path-parameters
    provides: "Runtime path parameter matching, method-specific routing, Request.param accessor, full compiler pipeline"
  - phase: 08-stdlib
    provides: "HTTP router, server, compile_and_start_server E2E test infrastructure"
provides:
  - "E2E test proving full Phase 51 stack: Snow source -> typeck -> MIR -> LLVM -> working HTTP server"
  - "Snow test fixture demonstrating path params, method routing, exact priority, and fallback"
  - "Three-pass route matching: exact > parameterized > wildcard"
  - "String-keyed SnowMaps for HTTP request params, query, and headers"
affects: [http-middleware, future-http-tests]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Three-pass route matching priority: exact routes > parameterized routes > wildcard catch-all"
    - "Use None (not None(_)) for nullary Option variant matching in Snow"
    - "Use snow_map_new_typed(1) for string-keyed SnowMaps in HTTP request construction"

key-files:
  created:
    - "tests/e2e/stdlib_http_path_params.snow"
  modified:
    - "crates/snowc/tests/e2e_stdlib.rs"
    - "crates/snow-rt/src/http/router.rs"
    - "crates/snow-rt/src/http/server.rs"

key-decisions:
  - "Use port 18082 for path params E2E test (18080=server_runtime, 18081=crash_isolation)"
  - "Three-pass route matching to prevent wildcards from stealing parameterized route matches"
  - "Fix string-keyed SnowMaps for all HTTP request maps (path_params, query_params, headers)"

patterns-established:
  - "Snow Option matching uses None (nullary) not None(_) for the empty case"
  - "HTTP E2E test ports: 18080, 18081, 18082 (increment for each new test)"

# Metrics
duration: 10min
completed: 2026-02-11
---

# Phase 51 Plan 02: HTTP Path Parameters E2E Test Summary

**E2E test proving full path params + method routing + route priority stack from Snow source through LLVM to running HTTP server, with three-pass routing and string-keyed map fixes**

## Performance

- **Duration:** 10 min
- **Started:** 2026-02-11T17:07:55Z
- **Completed:** 2026-02-11T17:17:52Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- E2E test with 5 HTTP request cases verifying all Phase 51 success criteria
- Snow fixture demonstrating path parameter extraction, exact-before-param priority, method-specific routing, and backward-compatible fallback
- Fixed three-pass route matching to correctly prioritize exact > parameterized > wildcard
- Fixed string-keyed SnowMap creation for HTTP request path_params, query_params, and headers
- All 6 HTTP E2E tests pass (no regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create Snow test fixture** - `2597efb` (test)
2. **Task 2: Add Rust E2E test + runtime fixes** - `d757065` (feat)

## Files Created/Modified
- `tests/e2e/stdlib_http_path_params.snow` - Snow fixture with 4 routes: exact /users/me, parameterized /users/:id, POST /data, wildcard /* fallback
- `crates/snowc/tests/e2e_stdlib.rs` - E2E test function `e2e_http_path_params` with 5 test cases and `send_request` helper
- `crates/snow-rt/src/http/router.rs` - Three-pass route matching: exact > parameterized > wildcard (was two-pass: exact+wildcard > parameterized)
- `crates/snow-rt/src/http/server.rs` - String-keyed SnowMaps (snow_map_new_typed(1)) for path_params, query_params, and headers

## Decisions Made
- **Port 18082:** Existing HTTP E2E tests use 18080 (server_runtime) and 18081 (crash_isolation), so path params test uses 18082
- **None not None(_):** Snow parser requires nullary `None` (no arguments) for Option matching, not `None(_)` as originally planned
- **Three-pass matching:** Two-pass matching (exact+wildcard first, parameterized second) caused wildcard `/*` to match before parameterized `/users/:id`. Split into three passes: exact only, then parameterized, then wildcard catch-all
- **String-keyed SnowMaps:** All HTTP request maps (path_params, query_params, headers) use `snow_map_new_typed(1)` instead of `snow_map_new()` for content-based string comparison instead of pointer equality

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Snow fixture: None(_) parse error**
- **Found during:** Task 2 (compilation of Snow fixture)
- **Issue:** Snow parser rejects `None(_)` as nullary constructor with argument; expects bare `None`
- **Fix:** Changed `None(_)` to `None` in case expression
- **Files modified:** tests/e2e/stdlib_http_path_params.snow
- **Verification:** Compilation succeeds
- **Committed in:** d757065 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed three-pass route matching (wildcard priority)**
- **Found during:** Task 2 (Test A: GET /users/42 hit fallback instead of parameterized route)
- **Issue:** Two-pass matching checked wildcard `/*` in the first pass alongside exact routes, so `/*` matched before parameterized `/users/:id` got a chance
- **Fix:** Split into three-pass matching: 1) exact routes only, 2) parameterized routes, 3) wildcard catch-all routes
- **Files modified:** crates/snow-rt/src/http/router.rs
- **Verification:** All 11 router unit tests pass, E2E test passes
- **Committed in:** d757065 (Task 2 commit)

**3. [Rule 1 - Bug] Fixed string-keyed SnowMap for HTTP request maps**
- **Found during:** Task 2 (Request.param returned None despite correct path params in router)
- **Issue:** `snow_map_new()` defaults to KEY_TYPE_INT (pointer equality comparison). Path params, query params, and headers store SnowString pointers as keys, so lookup with a freshly allocated SnowString fails because different pointer != different pointer, even with same content
- **Fix:** Changed all three map allocations in `handle_request` to `snow_map_new_typed(1)` (KEY_TYPE_STR) for content-based string comparison
- **Files modified:** crates/snow-rt/src/http/server.rs
- **Verification:** Request.param correctly returns Some("42") for /users/42
- **Committed in:** d757065 (Task 2 commit)

**4. [Rule 3 - Blocking] Rebuilt release libsnow_rt.a with Plan 01 symbols**
- **Found during:** Task 2 (linker error: undefined symbols for snow_http_route_get, snow_http_request_param)
- **Issue:** Plan 01 added new runtime functions but only the debug build was updated; E2E tests link against release build
- **Fix:** `cargo build -p snow-rt --release` to rebuild release static library
- **Files modified:** target/release/libsnow_rt.a (build artifact)
- **Verification:** Linker succeeds, E2E test compiles and runs
- **Committed in:** d757065 (Task 2 commit)

---

**Total deviations:** 4 auto-fixed (3 bugs, 1 blocking)
**Impact on plan:** All fixes necessary for correctness. Bug #2 (wildcard priority) and #3 (string-keyed maps) are genuine runtime bugs from Plan 01 that only surface under E2E testing. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviations above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 51 (HTTP Path Parameters) is complete: all 4 roadmap success criteria verified
- Path parameter extraction, method-specific routing, exact-before-param priority, and backward-compatible fallback all proven E2E
- Runtime bugs fixed (three-pass routing, string-keyed maps) strengthen HTTP server for future phases

## Self-Check: PASSED

All files verified present. Both task commits verified in git log.

---
*Phase: 51-http-path-parameters*
*Completed: 2026-02-11*
