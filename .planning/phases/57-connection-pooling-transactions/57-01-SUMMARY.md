---
phase: 57-connection-pooling-transactions
plan: 01
subsystem: database
tags: [postgresql, sqlite, transactions, wire-protocol, catch_unwind, panic-safety]

# Dependency graph
requires:
  - phase: 54-postgresql-driver
    provides: "PgConn struct, PG wire protocol, Box::into_raw handle pattern"
  - phase: 53-sqlite-driver
    provides: "SqliteConn struct, sqlite3 FFI, SnowResult pattern"
provides:
  - "PgConn.txn_status field tracking ReadyForQuery transaction status byte (I/T/E)"
  - "snow_pg_begin, snow_pg_commit, snow_pg_rollback intrinsics"
  - "snow_pg_transaction with catch_unwind panic-safe rollback"
  - "snow_sqlite_begin, snow_sqlite_commit, snow_sqlite_rollback intrinsics"
  - "pg_simple_command helper for Simple Query protocol"
  - "sqlite_simple_exec helper using sqlite3_exec FFI"
affects: [57-02-connection-pool, 57-03-compiler-pipeline]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Simple Query protocol (type Q) for transaction commands"
    - "txn_status tracking on every ReadyForQuery for pool cleanup"
    - "sqlite3_exec FFI for bare SQL commands"

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/db/pg.rs"
    - "crates/snow-rt/src/db/sqlite.rs"

key-decisions:
  - "Used Simple Query protocol (not Extended Query) for BEGIN/COMMIT/ROLLBACK - simpler, no params needed"
  - "Read SnowResult tag via struct cast (not raw u64 read) matching established SnowResult layout"
  - "Used sqlite3_exec FFI for bare SQL instead of prepare/step/finalize - simpler for parameterless commands"

patterns-established:
  - "pg_simple_command: reusable helper for sending SQL via Simple Query protocol with txn_status tracking"
  - "sqlite_simple_exec: reusable helper for bare SQLite SQL execution via sqlite3_exec"

# Metrics
duration: 3min
completed: 2026-02-12
---

# Phase 57 Plan 01: Transaction Management Summary

**PgConn txn_status tracking on all ReadyForQuery paths, 4 PG transaction intrinsics with catch_unwind panic-safe rollback, 3 SQLite transaction intrinsics using sqlite3_exec**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-12T19:43:46Z
- **Completed:** 2026-02-12T19:47:10Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- PgConn now tracks transaction status byte (I/T/E) from every ReadyForQuery message across all 3 existing code paths plus the new transaction helper
- Manual PG transaction control via snow_pg_begin, snow_pg_commit, snow_pg_rollback using Simple Query protocol
- Block-based PG transaction via snow_pg_transaction with automatic commit on Ok, rollback on Err, and catch_unwind rollback on panic
- Manual SQLite transaction control via snow_sqlite_begin, snow_sqlite_commit, snow_sqlite_rollback using sqlite3_exec FFI

## Task Commits

Each task was committed atomically:

1. **Task 1: Add txn_status to PgConn and PG transaction intrinsics** - `906bc08` (feat)
2. **Task 2: Add SQLite transaction intrinsics** - `c5b26f2` (feat)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/snow-rt/src/db/pg.rs` - Added txn_status field to PgConn, updated 3 ReadyForQuery arms, added pg_simple_command helper, added snow_pg_begin/commit/rollback/transaction
- `crates/snow-rt/src/db/sqlite.rs` - Added sqlite_simple_exec helper, added snow_sqlite_begin/commit/rollback

## Decisions Made
- Used Simple Query protocol (type 'Q') for BEGIN/COMMIT/ROLLBACK instead of Extended Query (Parse/Bind/Execute/Sync) -- simpler, no parameters needed, fewer round-trip messages
- Read SnowResult tag via proper struct cast (`*const SnowResult` with `.tag` field) instead of raw pointer read (`*const u64`) as suggested in plan -- the tag is u8, not u64, so raw read would be incorrect
- Used sqlite3_exec FFI for bare SQL instead of prepare/step/finalize pattern -- sqlite3_exec is simpler for parameterless commands and already available in libsqlite3_sys

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed SnowResult tag reading in snow_pg_transaction**
- **Found during:** Task 1 (snow_pg_transaction implementation)
- **Issue:** Plan specified reading result tag as `let tag = *(result_ptr as *const u64)` but SnowResult.tag is u8, not u64. Raw u64 read would include garbage bytes from the value pointer field.
- **Fix:** Used proper struct cast: `let r = &*(result_ptr as *const crate::io::SnowResult); if r.tag == 0 { ... }` -- matches the established pattern used throughout snow-rt (io.rs, sqlite.rs tests, job.rs).
- **Files modified:** crates/snow-rt/src/db/pg.rs
- **Verification:** Compiles correctly, matches all other SnowResult reads in the codebase.
- **Committed in:** 906bc08 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug fix)
**Impact on plan:** Essential correctness fix. The plan's suggested u64 tag read would have caused incorrect behavior. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- txn_status field is ready for Plan 02 (connection pool) to use for clean connection checkin (POOL-05: rollback if txn_status != 'I')
- All 7 transaction intrinsics are ready for Plan 03 (compiler pipeline) to add LLVM declarations, MIR lowering, and type checker entries
- snow-rt builds clean with no new warnings

## Self-Check: PASSED

All files exist, all commits verified, all functions present, build succeeds.

---
*Phase: 57-connection-pooling-transactions*
*Completed: 2026-02-12*
