---
phase: 15-http-actor-model
plan: 01
subsystem: runtime
tags: [actor, http, coroutine, crash-isolation, catch-unwind, corosensei]

# Dependency graph
requires:
  - phase: 08-stdlib
    provides: "HTTP server with tiny_http, router, thread-per-connection model"
  - phase: 09-actors
    provides: "Lightweight actor system with M:N scheduler, corosensei coroutines"
provides:
  - "Actor-per-connection HTTP server replacing thread-per-connection"
  - "Crash isolation via catch_unwind in connection handler actors"
  - "snow_panic changed from abort() to panic!() for catch_unwind compatibility"
  - "pub(crate) global_scheduler() for cross-module actor access"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Actor-per-connection: HTTP connections dispatched as lightweight actors"
    - "Crash isolation: catch_unwind wraps actor entry to isolate panics"
    - "ConnectionArgs struct: raw pointer transfer of router+request to actor"

key-files:
  created:
    - "tests/e2e/stdlib_http_crash_isolation.snow"
  modified:
    - "crates/snow-rt/src/http/server.rs"
    - "crates/snow-rt/src/http/mod.rs"
    - "crates/snow-rt/src/actor/mod.rs"
    - "crates/snow-rt/src/panic.rs"
    - "crates/snowc/tests/e2e_stdlib.rs"

key-decisions:
  - "snow_panic changed from abort() to panic!() for catch_unwind crash isolation"
  - "Actor scheduler initialized idempotently in snow_http_serve via snow_rt_init_actor(0)"
  - "Multi-clause function do_crash(0) used for crash test (case expr non-exhaustive is compile error)"

patterns-established:
  - "Actor crash isolation: extern C entry wrapped in catch_unwind with AssertUnwindSafe"
  - "Args transfer: heap-allocated struct passed as *const u8, reclaimed via Box::from_raw"

# Metrics
duration: 6min
completed: 2026-02-08
---

# Phase 15 Plan 01: Actor-per-Connection HTTP Server Summary

**Actor-per-connection HTTP server with catch_unwind crash isolation, replacing thread-per-connection model**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-08T02:24:16Z
- **Completed:** 2026-02-08T02:30:46Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Replaced `std::thread::spawn` with `actor::global_scheduler().spawn()` for each HTTP connection
- Added `connection_handler_entry` with `catch_unwind` for per-connection crash isolation
- Changed `snow_panic` from `abort()` to `panic!()` to make runtime panics catchable
- New e2e test proves server survives a panicking handler and serves subsequent requests
- Full backward compatibility: existing HTTP server test passes unchanged

## Task Commits

Each task was committed atomically:

1. **Task 1: Replace thread-per-connection with actor-per-connection** - `71897af` (feat)
2. **Task 2: Add crash isolation e2e test** - `630df48` (feat)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/snow-rt/src/http/server.rs` - Actor-per-connection with ConnectionArgs, connection_handler_entry, catch_unwind
- `crates/snow-rt/src/http/mod.rs` - Updated module docs reflecting actor model
- `crates/snow-rt/src/actor/mod.rs` - Made global_scheduler() pub(crate)
- `crates/snow-rt/src/panic.rs` - Changed snow_panic from abort() to panic!()
- `tests/e2e/stdlib_http_crash_isolation.snow` - Snow test with crash/health routes on port 18081
- `crates/snowc/tests/e2e_stdlib.rs` - Added e2e_http_crash_isolation test, updated HTTP comments

## Decisions Made
- **snow_panic: abort() to panic!()**: The catch_unwind crash isolation model requires panics (not aborts) to be catchable. Changed snow_panic to use `panic!()` instead of `std::process::abort()`. This is safe because panics unwind and terminate the process by default outside catch_unwind contexts, matching the previous abort behavior.
- **Multi-clause function for crash test**: The Snow compiler rejects non-exhaustive `case` expressions as hard errors (E0012), but non-exhaustive multi-clause functions produce only warnings. Used `fn do_crash(0) = 0` called with `42` to trigger a runtime panic.
- **Idempotent scheduler init**: `snow_rt_init_actor(0)` called at top of `snow_http_serve` ensures the actor scheduler is ready, even if the Snow program doesn't use actors directly.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Changed snow_panic from abort() to panic!() for crash isolation**
- **Found during:** Task 2 (crash isolation test)
- **Issue:** `snow_panic` used `std::process::abort()` which terminates the entire process, not catchable by `catch_unwind`. Crash isolation requires catchable panics.
- **Fix:** Changed `snow_panic` to use `panic!()` instead of `abort()`. Updated docs to reflect the new behavior.
- **Files modified:** `crates/snow-rt/src/panic.rs`
- **Verification:** e2e_http_crash_isolation test passes: server survives handler panic
- **Committed in:** `630df48` (Task 2 commit)

**2. [Rule 1 - Bug] Used multi-clause function instead of case expression for crash trigger**
- **Found during:** Task 2 (crash isolation test)
- **Issue:** Plan specified `case 42 do 0 -> ... end` but Snow compiler rejects non-exhaustive case expressions as hard error (E0012). Cannot compile.
- **Fix:** Used `fn do_crash(0) = 0` (multi-clause, non-exhaustive is warning only) called with `42` to trigger runtime panic.
- **Files modified:** `tests/e2e/stdlib_http_crash_isolation.snow`
- **Verification:** Test compiles and runs, panic caught by catch_unwind
- **Committed in:** `630df48` (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 missing critical, 1 bug)
**Impact on plan:** Both fixes essential for crash isolation to work. No scope creep.

## Issues Encountered
None beyond the deviations documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 15 is the final v1.1 phase. All v1.1 features complete:
  - Phase 11: Multi-clause functions
  - Phase 12: Closures
  - Phase 13: String interpolation
  - Phase 14: Generic Map types
  - Phase 15: HTTP actor model with crash isolation
- v1.1 milestone ready for tagging
- No blockers or concerns

## Self-Check: PASSED

---
*Phase: 15-http-actor-model*
*Completed: 2026-02-08*
