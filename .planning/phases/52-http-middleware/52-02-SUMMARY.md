---
phase: 52-http-middleware
plan: 02
subsystem: testing
tags: [http, middleware, e2e, closure-abi, arm64]

# Dependency graph
requires:
  - phase: 52-http-middleware
    plan: 01
    provides: HTTP.use runtime, middleware chain, typeck+codegen+intrinsics
  - phase: 51-http-path-params
    provides: E2E test infrastructure (compile_and_start_server, send_request, ServerGuard)
provides:
  - E2E test proving middleware chain executes correctly end-to-end
  - Fixed call_middleware ABI to decompose closure structs for LLVM calling convention
  - Snow middleware fixture with passthrough and short-circuit patterns
affects: [future-middleware-phases, http-testing]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Middleware requires :: Request type annotation on parameters (type inference limitation)"
    - "Passthrough middleware needs -> Response return type annotation"
    - "Snow boolean branching uses if/else, not case true/false"
    - "Closure struct {ptr, ptr} must be decomposed into separate args at FFI boundary"

key-files:
  created:
    - tests/e2e/stdlib_http_middleware.snow
  modified:
    - crates/snowc/tests/e2e_stdlib.rs
    - crates/snow-rt/src/http/server.rs

key-decisions:
  - "Used type annotations (:: Request, -> Response) to work around incomplete type inference for middleware function parameters"
  - "Fixed call_middleware to decompose {ptr, ptr} closure struct into separate register args matching LLVM arm64 ABI"
  - "Used if/else instead of case for boolean branching since Snow parser lacks boolean literal patterns"

patterns-established:
  - "Middleware E2E test pattern: compile fixture, start server, test passthrough + short-circuit + 404 paths"
  - "Closure ABI at FFI boundary: always decompose {fn_ptr, env_ptr} struct into separate args"

# Metrics
duration: 30min
completed: 2026-02-11
---

# Phase 52 Plan 02: HTTP Middleware E2E Test Summary

**End-to-end test proving middleware chain with passthrough, short-circuit auth, and 404 fallback; fixed closure struct ABI decomposition in call_middleware**

## Performance

- **Duration:** 30 min
- **Started:** 2026-02-11T17:51:16Z
- **Completed:** 2026-02-11T18:21:47Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Snow fixture with two middleware (logger passthrough + auth short-circuit) and two route handlers on port 18083
- E2E Rust test with three scenarios: passthrough (200), auth block (401), and 404 fallback
- Fixed critical ABI bug in `call_middleware` where closure struct was passed as raw pointer instead of decomposed fields
- All 85 E2E tests pass (7 HTTP-specific, including new middleware test)

## Task Commits

Each task was committed atomically:

1. **Task 1: Create Snow middleware fixture** - `fb6cda8` (test)
2. **Task 2: Add E2E test + fix runtime ABI** - `c01d910` (feat)

**Plan metadata:** (pending)

## Files Created/Modified
- `tests/e2e/stdlib_http_middleware.snow` - Snow middleware fixture with logger, auth_check, handler, secret_handler
- `crates/snowc/tests/e2e_stdlib.rs` - e2e_http_middleware test function (3 test scenarios)
- `crates/snow-rt/src/http/server.rs` - Fixed call_middleware to decompose closure struct into separate fn_ptr+env_ptr args

## Decisions Made
- **Type annotations required:** Snow's type inference doesn't fully propagate `Request`/`Response` types from `HTTP.use(router, middleware)` back into the middleware function definition. Added explicit `:: Request` parameter annotations and `-> Response` return type for logger.
- **if/else over case:** Snow's parser doesn't support `case bool_expr do true -> ... false -> ... end`. Used `if/else` for the auth check's boolean branching.
- **Closure struct decomposition:** LLVM's arm64 calling convention passes `{ptr, ptr}` struct parameters in two registers (x1, x2). The runtime must decompose the closure struct pointer into its field values before calling through the function pointer.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed closure struct ABI in call_middleware**
- **Found during:** Task 2 (E2E test execution)
- **Issue:** `call_middleware` passed the `next_closure` pointer as a single raw pointer argument, but LLVM-compiled Snow functions expect the `{fn_ptr, env_ptr}` closure struct decomposed into two separate register arguments on arm64.
- **Fix:** Dereference the closure struct pointer to extract fn_ptr (offset 0) and env_ptr (offset 8), then pass them as separate arguments matching the 3-arg (bare function) or 4-arg (closure middleware) calling convention.
- **Files modified:** `crates/snow-rt/src/http/server.rs`
- **Verification:** All 7 HTTP E2E tests pass, including new middleware test
- **Committed in:** c01d910 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed Snow fixture syntax (case -> if/else)**
- **Found during:** Task 2 (compilation verification)
- **Issue:** Initial fixture used `case String.starts_with(path, "/secret") do true -> ... false -> ... end` which Snow's parser doesn't support (boolean literal patterns not implemented).
- **Fix:** Rewrote to `if is_secret do ... else ... end` with a let binding for the boolean result.
- **Files modified:** `tests/e2e/stdlib_http_middleware.snow`
- **Verification:** Fixture compiles successfully
- **Committed in:** c01d910 (Task 2 commit)

**3. [Rule 3 - Blocking] Added type annotations for incomplete type inference**
- **Found during:** Task 2 (LLVM verification failure)
- **Issue:** Without `:: Request` annotations, Snow's type inference produced `MirType::Unit` (LLVM `{}`) for middleware parameters instead of `MirType::Ptr` (LLVM `ptr`), causing ABI mismatches and LLVM module verification failures.
- **Fix:** Added `:: Request` on all handler/middleware request parameters and `-> Response` on the logger function.
- **Files modified:** `tests/e2e/stdlib_http_middleware.snow`
- **Verification:** LLVM IR shows correct `ptr` types, compilation succeeds
- **Committed in:** c01d910 (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 blocking)
**Impact on plan:** All fixes necessary for correct middleware execution. The ABI decomposition fix is a critical runtime correction that was an oversight in Phase 52 Plan 01's implementation. No scope creep.

## Issues Encountered
- **libsnow_rt.a linking:** `cargo build -p snowc` does not produce `target/debug/libsnow_rt.a` (only deps/ hash-named copies). Manual copy was needed for standalone `snowc build` testing. The E2E test framework (`cargo test`) works correctly because it links via cargo's rlib mechanism, not the staticlib.
- **Actor coroutine I/O suppression:** stderr output (eprintln, file writes) from actor coroutine contexts doesn't appear in test output, making debugging middleware chain execution difficult. Required main-thread testing to isolate the issue.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- HTTP middleware stack is fully operational: registration, chain execution, passthrough, short-circuit, and 404 fallback all verified
- Type inference limitation documented: middleware functions need explicit type annotations
- Ready for future middleware features (request/response transformation, async middleware, etc.)

## Self-Check: PASSED

- All 3 created/modified files exist on disk
- Both task commits (fb6cda8, c01d910) found in git history
- e2e_http_middleware test passes (1 passed, 0 failed)

---
*Phase: 52-http-middleware*
*Completed: 2026-02-11*
