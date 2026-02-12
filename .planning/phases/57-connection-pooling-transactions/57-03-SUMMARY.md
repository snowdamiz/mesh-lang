---
phase: 57-connection-pooling-transactions
plan: 03
subsystem: database
tags: [connection-pool, transactions, postgres, sqlite, llvm, mir, typeck, compiler-pipeline]

# Dependency graph
requires:
  - phase: 57-01
    provides: "PG/SQLite transaction runtime functions (snow_pg_begin/commit/rollback/transaction, snow_sqlite_begin/commit/rollback)"
  - phase: 57-02
    provides: "Connection pool runtime functions (snow_pool_open/close/checkout/checkin/query/execute)"
  - phase: 54
    provides: "PG connect/close/execute/query compiler pipeline pattern"
  - phase: 53
    provides: "SQLite compiler pipeline pattern"
provides:
  - "Pool.*, Pg.begin/commit/rollback/transaction, Sqlite.begin/commit/rollback callable from Snow source code"
  - "PoolHandle type recognized throughout compiler pipeline (typeck -> MIR -> LLVM)"
  - "Pool module in STDLIB_MODULE_NAMES for qualified access (Pool.open, Pool.query, etc.)"
affects: [phase-58, future-database-phases]

# Tech tracking
tech-stack:
  added: []
  patterns: ["opaque-handle-to-MirType::Int for pool handles (same as PgConn/SqliteConn)"]

key-files:
  created: []
  modified:
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/mir/types.rs"
    - "crates/snow-typeck/src/builtins.rs"
    - "crates/snow-typeck/src/infer.rs"

key-decisions:
  - "Pg.transaction uses mono type (Result<Unit, String>) for callback -- sufficient for most use cases, runtime handles ptr-level forwarding"
  - "PoolHandle follows opaque u64 handle pattern (MirType::Int) established by PgConn/SqliteConn"
  - "Pool.checkout returns PgConn (not generic conn) since pool is PG-focused in v3.0"

patterns-established:
  - "Module extension pattern: adding methods to existing Pg/Sqlite modules without restructuring"

# Metrics
duration: 5min
completed: 2026-02-12
---

# Phase 57 Plan 03: Compiler Pipeline Wiring Summary

**13 new runtime intrinsics (pool + transactions) wired through full compiler pipeline: LLVM declarations, MIR lowering, type mapping, and type checker with Pool module**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-12T19:55:09Z
- **Completed:** 2026-02-12T19:59:47Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- All 13 new runtime functions declared in LLVM module with correct signatures (4 PG transaction, 3 SQLite transaction, 6 pool)
- PoolHandle type maps to MirType::Int through the full pipeline (typeck -> MIR -> LLVM)
- Pool module added to stdlib_modules with full type signatures for qualified access (Pool.open, Pool.query, etc.)
- Pg module extended with begin/commit/rollback/transaction methods
- Sqlite module extended with begin/commit/rollback methods
- All existing tests continue to pass; full cargo build and cargo test clean

## Task Commits

Each task was committed atomically:

1. **Task 1: Add LLVM intrinsic declarations and MIR type mapping** - `4465247` (feat)
2. **Task 2: Add MIR lowering and type checker entries** - `8dddbfd` (feat)

## Files Created/Modified
- `crates/snow-codegen/src/codegen/intrinsics.rs` - 13 new LLVM function declarations + test assertions
- `crates/snow-codegen/src/mir/lower.rs` - known_functions entries + map_builtin_name mappings for all 13 intrinsics
- `crates/snow-codegen/src/mir/types.rs` - PoolHandle => MirType::Int type mapping
- `crates/snow-typeck/src/builtins.rs` - PoolHandle type + all function signatures in builtins env
- `crates/snow-typeck/src/infer.rs` - Pool module in stdlib_modules + Pg/Sqlite transaction methods + "Pool" in STDLIB_MODULE_NAMES

## Decisions Made
- Pg.transaction uses mono Result<Unit, String> for callback type rather than polymorphic -- sufficient for most transaction use cases, and the runtime handles ptr-level forwarding regardless of inner type
- PoolHandle follows the opaque u64 handle pattern established by PgConn/SqliteConn (MirType::Int) for GC safety
- Pool.checkout returns PgConn specifically (not a generic connection type) since the pool is PG-focused in v3.0

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 57 complete: all three plans (runtime transactions, connection pool, compiler pipeline) are done
- Snow programs can now call Pool.open/close/checkout/checkin/query/execute and Pg.begin/commit/rollback/transaction and Sqlite.begin/commit/rollback
- Ready for Phase 58 or any future database-dependent features

---
*Phase: 57-connection-pooling-transactions*
*Completed: 2026-02-12*
