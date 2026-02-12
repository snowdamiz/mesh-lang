---
phase: 54-postgresql-driver
plan: 02
subsystem: database
tags: [postgresql, e2e-test, scram-sha-256, wire-protocol, crud]

# Dependency graph
requires:
  - phase: 54-postgresql-driver
    provides: "PostgreSQL runtime functions (snow_pg_connect/close/execute/query) and compiler pipeline registration"
  - phase: 53-sqlite-driver
    provides: "SQLite E2E test pattern (fixture + harness) to follow for PostgreSQL"
provides:
  - "E2E test fixture exercising full PostgreSQL CRUD lifecycle in Snow"
  - "E2E test harness entry e2e_pg (ignored, requires running PostgreSQL)"
  - "Verified SCRAM-SHA-256 authentication works end-to-end"
  - "Verified full pipeline: Snow source -> typeck -> MIR -> LLVM -> native binary -> TCP -> PostgreSQL"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns: [postgresql-e2e-fixture-pattern, ignored-test-for-external-service]

key-files:
  created:
    - "tests/e2e/stdlib_pg.snow"
  modified:
    - "crates/snowc/tests/e2e_stdlib.rs"
    - "crates/snow-rt/src/db/pg.rs"

key-decisions:
  - "Follow exact SQLite E2E pattern: helper fn with Result return for ? chaining, case dispatch in main()"
  - "Test marked #[ignore] since it requires external PostgreSQL instance"
  - "Connection string hardcoded in fixture (matching SQLite :memory: pattern)"
  - "Fixed SCRAM-SHA-256 client-first-bare to use empty n= (matching libpq behavior)"

patterns-established:
  - "PostgreSQL E2E test fixture with DROP TABLE IF EXISTS for idempotent runs"
  - "Ignored test pattern for external service dependencies with setup instructions in comments"

# Metrics
duration: 11min
completed: 2026-02-12
---

# Phase 54 Plan 02: PostgreSQL E2E Test Summary

**E2E test fixture proving full Snow->PostgreSQL pipeline (compile, connect, SCRAM-SHA-256 auth, DDL/DML with $1 params, query with column access, close) against real PostgreSQL 16**

## Performance

- **Duration:** 11 min
- **Started:** 2026-02-12T16:16:05Z
- **Completed:** 2026-02-12T16:27:11Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Snow fixture exercises all PostgreSQL driver functions: connect, execute (DDL+DML), query, parameterized queries ($1/$2), close
- E2E test verified against real PostgreSQL 16 with SCRAM-SHA-256 authentication
- Fixed SCRAM-SHA-256 bug in runtime (client-first-bare username mismatch)
- Full pipeline proven: Snow source -> type check -> MIR -> LLVM IR -> native binary -> TCP -> wire protocol -> correct results

## Task Commits

Each task was committed atomically:

1. **Task 1: Snow fixture and E2E test for PostgreSQL CRUD lifecycle** - `30672b1` (feat)
2. **SCRAM-SHA-256 auth fix (deviation Rule 1)** - `5df17a8` (fix)

## Files Created/Modified
- `tests/e2e/stdlib_pg.snow` - Snow fixture testing full PostgreSQL CRUD lifecycle (connect, DDL, insert with params, query with column access, filtered query, close)
- `crates/snowc/tests/e2e_stdlib.rs` - Added `e2e_pg` test function marked `#[ignore]` with setup instructions in comments
- `crates/snow-rt/src/db/pg.rs` - Fixed SCRAM-SHA-256 client-first-bare to use empty username (matching libpq behavior)

## Decisions Made
- Followed exact SQLite E2E pattern: `run_db()` helper with `Int!String` return type for `?` chaining, `case` dispatch in `main()`
- Test marked `#[ignore]` since it requires a running PostgreSQL instance -- user runs with `cargo test e2e_pg -- --ignored`
- Connection string hardcoded as `postgres://snow_test:snow_test@localhost:5432/snow_test` (matching SQLite `:memory:` simplicity pattern)
- Uses `DROP TABLE IF EXISTS` at start for idempotent test runs

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed SCRAM-SHA-256 authentication failure**
- **Found during:** Task 1 verification (compiling and running against real PostgreSQL)
- **Issue:** `scram_client_first()` generated `n=<username>` in client-first-bare but `scram_client_final()` computed AuthMessage using `n=` (empty username). The mismatch caused the server to reject the client proof.
- **Fix:** Changed `scram_client_first()` to use empty `n=` in client-first-bare, matching libpq behavior. PostgreSQL already knows the username from the StartupMessage.
- **Files modified:** `crates/snow-rt/src/db/pg.rs`
- **Verification:** E2E test passes against PostgreSQL 16 with SCRAM-SHA-256 auth
- **Committed in:** `5df17a8`

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Critical fix -- SCRAM-SHA-256 auth was broken without it. No scope creep.

## Issues Encountered
- Snow parser requires `let r = run_db()` followed by `case r do` rather than `case run_db() do` -- matched existing SQLite fixture pattern
- Rebuilding snow-rt required explicit `cargo build -p snow-rt` before recompiling Snow fixtures for runtime changes to take effect

## User Setup Required
None - verification was performed during execution using a Docker PostgreSQL 16 instance.

## Next Phase Readiness
- Full PostgreSQL driver verified end-to-end
- Phase 54 (PostgreSQL Driver) is complete
- All 8 PG requirements proven: connect, close, execute (DDL/DML), query, parameterized queries, SCRAM-SHA-256 auth, text format results, column name access

## Self-Check: PASSED

- All 3 key files exist on disk
- Both task commits verified (30672b1, 5df17a8)
- E2E test `e2e_pg` passes against real PostgreSQL 16 (`test e2e_pg ... ok`)
- Full test suite passes (0 failures, 2 ignored)

---
*Phase: 54-postgresql-driver*
*Completed: 2026-02-12*
