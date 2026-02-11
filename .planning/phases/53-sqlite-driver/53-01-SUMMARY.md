---
phase: 53-sqlite-driver
plan: 01
subsystem: database
tags: [sqlite, ffi, libsqlite3-sys, bundled, runtime, compiler-pipeline]

requires:
  - phase: 52-http-middleware
    provides: existing compiler pipeline patterns (intrinsics, typeck, MIR)
provides:
  - 4 extern C SQLite runtime functions (open, close, execute, query)
  - Sqlite module registered in typeck with type signatures
  - LLVM intrinsic declarations for all 4 functions
  - MIR known_functions and map_builtin_name entries
  - SqliteConn opaque type lowered to MirType::Int for GC safety
affects: [53-02-E2E-test, 54-postgresql-driver]

tech-stack:
  added: [libsqlite3-sys 0.36 bundled]
  patterns: [opaque u64 handle for GC safety, StmtGuard RAII for statement finalization]

key-files:
  created:
    - crates/snow-rt/src/db/sqlite.rs
    - crates/snow-rt/src/db/mod.rs
  modified:
    - crates/snow-rt/Cargo.toml
    - crates/snow-rt/src/lib.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/mir/types.rs

key-decisions:
  - "SqliteConn handle is u64 (MirType::Int) for GC safety — GC cannot trace through opaque handles"
  - "libsqlite3-sys bundled compiles SQLite from C amalgamation — zero system dependencies"
  - "All params bound as text via sqlite3_bind_text with SQLITE_TRANSIENT"
  - "StmtGuard RAII wrapper ensures sqlite3_finalize always called on error paths"
  - "SnowMap with string keys (typed map) for query result rows"

patterns-established:
  - "Database driver pattern: opaque u64 handle + extern C functions + Result returns"
  - "StmtGuard RAII for SQLite statement lifecycle management"

duration: ~11min
completed: 2026-02-11
---

# Plan 53-01: SQLite Runtime FFI + Compiler Pipeline Summary

**4 extern C SQLite functions via libsqlite3-sys bundled + full compiler pipeline registration (intrinsics, typeck, MIR) with SqliteConn as opaque i64 handle**

## Performance

- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- Implemented `snow_sqlite_open`, `snow_sqlite_close`, `snow_sqlite_execute`, `snow_sqlite_query` as extern C functions
- Bundled SQLite via `libsqlite3-sys` with zero system dependencies
- Registered Sqlite module in typechecker with proper function signatures
- Declared LLVM intrinsics for all 4 functions
- Added MIR known_functions and map_builtin_name entries
- SqliteConn type lowers to MirType::Int (i64) for GC safety

## Task Commits

Each task was committed atomically:

1. **Task 1: Runtime -- SQLite C FFI wrapper functions and dependency setup** - `01438eb` (feat)
2. **Task 2: Compiler pipeline -- intrinsics, type checker, and MIR lowering** - `6b37cbd` (feat)

## Files Created/Modified
- `crates/snow-rt/src/db/sqlite.rs` - 4 extern C functions wrapping SQLite C API
- `crates/snow-rt/src/db/mod.rs` - db module declaration
- `crates/snow-rt/Cargo.toml` - Added libsqlite3-sys bundled dependency
- `crates/snow-rt/src/lib.rs` - Re-exports for sqlite functions
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM declarations for 4 sqlite functions
- `crates/snow-typeck/src/infer.rs` - Sqlite module in stdlib_modules
- `crates/snow-typeck/src/builtins.rs` - SqliteConn type and function signatures
- `crates/snow-codegen/src/mir/lower.rs` - known_functions and name mapping
- `crates/snow-codegen/src/mir/types.rs` - SqliteConn type lowering to MirType::Int

## Decisions Made
- SqliteConn handle is u64/i64 (not pointer) for GC safety
- libsqlite3-sys bundled for zero system dependencies
- All params bound as text via sqlite3_bind_text with SQLITE_TRANSIENT
- StmtGuard RAII wrapper ensures sqlite3_finalize on all error paths

## Deviations from Plan

None - plan executed as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Runtime functions and compiler pipeline complete
- Ready for E2E test in Plan 53-02

---
*Phase: 53-sqlite-driver*
*Completed: 2026-02-11*
