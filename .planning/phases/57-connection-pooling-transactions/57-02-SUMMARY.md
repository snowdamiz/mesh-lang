---
phase: 57-connection-pooling-transactions
plan: 02
subsystem: database
tags: [postgresql, connection-pool, mutex, condvar, parking_lot, health-check, checkout-timeout]

# Dependency graph
requires:
  - phase: 54-postgresql-driver
    provides: "PgConn struct, snow_pg_connect/close/query/execute, wire protocol"
  - phase: 57-01
    provides: "PgConn.txn_status field, pg_simple_command helper"
provides:
  - "PgPool struct with parking_lot::Mutex<PoolInner> + Condvar"
  - "snow_pool_open with configurable min/max connections and checkout timeout"
  - "snow_pool_checkout with health check (SELECT 1) and Condvar blocking timeout"
  - "snow_pool_checkin with automatic ROLLBACK for dirty connections (txn_status != I)"
  - "snow_pool_query and snow_pool_execute with auto checkout-use-checkin"
  - "snow_pool_close that drains idle connections and wakes blocked checkouts"
affects: [57-03-compiler-pipeline]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Mutex+Condvar pool: bounded connection sharing with blocking checkout and timeout"
    - "Health check on checkout: SELECT 1 via pg_simple_command validates idle connections"
    - "Transaction cleanup on checkin: ROLLBACK if txn_status != b'I'"
    - "Optimistic slot reservation: increment total_created before dropping lock for I/O"

key-files:
  created:
    - "crates/snow-rt/src/db/pool.rs"
  modified:
    - "crates/snow-rt/src/db/mod.rs"
    - "crates/snow-rt/src/db/pg.rs"
    - "crates/snow-rt/src/lib.rs"

key-decisions:
  - "Used parking_lot::Mutex + Condvar (not std::sync) for consistency with scheduler"
  - "Health check uses pg_simple_command(SELECT 1) reusing Plan 01 helper"
  - "Optimistic slot reservation: increment total_created before dropping lock to create connection"
  - "Pool handle leaks on close (same pattern as server configs -- runs forever in practice)"

patterns-established:
  - "Pool checkout pattern: loop over idle->health_check->create_new->condvar_wait with timeout"
  - "Auto checkout-use-checkin: pool_query/pool_execute handle full lifecycle transparently"

# Metrics
duration: 3min
completed: 2026-02-12
---

# Phase 57 Plan 02: Connection Pool Summary

**PostgreSQL connection pool with Mutex+Condvar synchronization, health check on checkout, transaction cleanup on checkin, and auto checkout-use-checkin convenience methods**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-12T19:50:13Z
- **Completed:** 2026-02-12T19:53:02Z
- **Tasks:** 1
- **Files modified:** 4

## Accomplishments
- Created pool.rs with complete PgPool implementation using parking_lot::Mutex + Condvar for thread-safe connection sharing across Snow scheduler worker threads
- Implemented 6 extern "C" functions: snow_pool_open, snow_pool_checkout, snow_pool_checkin, snow_pool_query, snow_pool_execute, snow_pool_close
- Checkout validates idle connections with SELECT 1 health check, replacing dead connections automatically
- Checkin sends ROLLBACK if connection has active/failed transaction (txn_status != 'I')
- Checkout blocks with configurable timeout via Condvar::wait_for when pool is exhausted
- Made PgConn and pg_simple_command pub(super) for cross-module access within db/

## Task Commits

Each task was committed atomically:

1. **Task 1: Create pool.rs with full connection pool implementation** - `d940aff` (feat)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/snow-rt/src/db/pool.rs` - New file: PgPool struct with all 6 snow_pool_* extern C functions
- `crates/snow-rt/src/db/mod.rs` - Added `pub mod pool;` export
- `crates/snow-rt/src/db/pg.rs` - Changed PgConn and pg_simple_command to pub(super) visibility
- `crates/snow-rt/src/lib.rs` - Added pool function re-exports

## Decisions Made
- Used parking_lot::Mutex + Condvar (not std::sync) for consistency with the actor scheduler which already uses parking_lot
- Health check reuses pg_simple_command from Plan 01 with "SELECT 1" rather than a custom ping mechanism
- Optimistic slot reservation pattern: increment total_created and active_count before dropping the lock to create a new connection; decrement on failure. This avoids holding the lock during TCP+TLS+auth which takes 10-150ms
- Pool handle intentionally leaks on close (do NOT Box::from_raw) because active connections may still be checked out; same leak pattern as server configs

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Made PgConn and pg_simple_command pub(super)**
- **Found during:** Task 1 (pool.rs implementation)
- **Issue:** PgConn struct and pg_simple_command function were private to pg.rs, but pool.rs needs to access them for health checks and transaction cleanup
- **Fix:** Changed `struct PgConn` to `pub(super) struct PgConn`, `txn_status` field to `pub(super) txn_status`, and `fn pg_simple_command` to `pub(super) fn pg_simple_command`
- **Files modified:** crates/snow-rt/src/db/pg.rs
- **Verification:** cargo build -p snow-rt succeeds, pool.rs can access PgConn.txn_status and pg_simple_command
- **Committed in:** d940aff (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking issue)
**Impact on plan:** Plan anticipated this need ("may need to refactor... to expose an internal version"). Minimal visibility change, no structural impact.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All 6 pool runtime functions are ready for Plan 03 (compiler pipeline) to add LLVM declarations, MIR lowering, and type checker entries
- Pool handle is opaque u64, consistent with PgConn/SqliteConn handle pattern
- snow-rt builds clean with no new warnings

## Self-Check: PASSED

All files exist, all commits verified, build succeeds.

---
*Phase: 57-connection-pooling-transactions*
*Completed: 2026-02-12*
