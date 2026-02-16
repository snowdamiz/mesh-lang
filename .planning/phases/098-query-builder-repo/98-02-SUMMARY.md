---
phase: 98-query-builder-repo
plan: 02
subsystem: database
tags: [repo, query-execution, sql-generation, read-operations, pool-query]

# Dependency graph
requires:
  - phase: 98-query-builder-repo
    provides: "Query module with 14 builder functions, 13-slot opaque Query struct"
  - phase: 97-schema-metadata-sql-generation
    provides: "Orm.build_select, Pool.query, Schema metadata (__table__, __fields__)"
provides:
  - "Repo module with 6 read operations: all, one, get, get_by, count, exists"
  - "Comprehensive SQL builder reading all 13 Query slots (WHERE, JOIN, GROUP BY, HAVING, ORDER BY, LIMIT, OFFSET, fragment)"
  - "query_to_select_sql, query_to_count_sql, query_to_exists_sql internal functions"
  - "Full compiler pipeline registration: typeck, MIR, LLVM intrinsics, JIT"
affects: [98-03, 99-changesets, 100-relationships]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Repo: stateless read operations consuming opaque Query structs via Pool.query"
    - "SQL builder: reads Query struct slots by offset, generates parameterized SQL with WHERE/JOIN/GROUP/HAVING/fragment"
    - "PoolHandle typed as Int at MIR level (u64 handle), Ptr at typeck level"

key-files:
  created:
    - "crates/mesh-rt/src/db/repo.rs"
  modified:
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"
    - "crates/mesh-rt/src/db/mod.rs"
    - "crates/mesh-rt/src/lib.rs"
    - "crates/mesh-repl/src/jit.rs"
    - "crates/meshc/tests/e2e.rs"

key-decisions:
  - "98-02: Repo functions use PoolHandle (MirType::Int / i64) for pool parameter, matching existing Pool.query pattern"
  - "98-02: SQL builder separated into pure Rust functions (build_select_sql_from_parts) for unit testability without GC"
  - "98-02: Repo.count uses SELECT COUNT(*) with 'count' column key extraction from result map"
  - "98-02: Repo.exists wraps query in SELECT EXISTS(SELECT 1 FROM ... LIMIT 1)"

patterns-established:
  - "Repo read pattern: read Query slots -> build SQL -> call mesh_pool_query -> extract/transform result"
  - "Pure Rust SQL builder separation: FFI functions delegate to testable pure functions"

# Metrics
duration: 8min
completed: 2026-02-16
---

# Phase 98 Plan 02: Repo Read Operations Summary

**Stateless Repo module with 6 read operations (all, one, get, get_by, count, exists) consuming Query structs via comprehensive SQL builder across full compiler pipeline**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-16T19:15:56Z
- **Completed:** 2026-02-16T19:24:37Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments
- Repo module registered across typeck (STDLIB_MODULE_NAMES + 6 function types), MIR (STDLIB_MODULES + known_functions + map_builtin_name), intrinsics (6 LLVM declarations), JIT (6 symbol mappings), and runtime (6 extern C functions in repo.rs)
- Comprehensive query_to_select_sql builder reads all 13 Query struct slots and generates complete parameterized SQL with WHERE (=, >, LIKE, IN, IS NULL), JOIN, GROUP BY, HAVING, ORDER BY, LIMIT, OFFSET, and fragment clauses
- 12 unit tests for pure Rust SQL builder functions (select, where, is_null, in_clause, join, group_by/having, order/limit/offset, full_query, fragment, count, exists)
- 5 e2e tests verify Repo module compiles and type-checks correctly in the full compiler pipeline
- 192 total e2e tests pass (187 existing + 5 new), zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Register Repo module and implement read operations with SQL generation** - `6319e7d5` (feat)
2. **Task 2: Add e2e tests for Repo read operations and query pipeline** - `32ad3c80` (test)

## Files Created/Modified
- `crates/mesh-rt/src/db/repo.rs` - Repo runtime: 6 read operations with comprehensive SQL builder from Query struct
- `crates/mesh-typeck/src/infer.rs` - Repo module type signatures (all, one, get, get_by, count, exists)
- `crates/mesh-codegen/src/mir/lower.rs` - Repo known_functions + map_builtin_name mappings
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - 6 LLVM intrinsic declarations for Repo functions
- `crates/mesh-rt/src/db/mod.rs` - Added pub mod repo
- `crates/mesh-rt/src/lib.rs` - Re-export all mesh_repo_* functions
- `crates/mesh-repl/src/jit.rs` - JIT symbol registrations for 6 Repo functions
- `crates/meshc/tests/e2e.rs` - 5 new e2e tests for Repo module

## Decisions Made
- Used PoolHandle (MirType::Int) for pool parameter at MIR/LLVM level, matching the established pattern from Pool.query (pool handles are u64 values, not pointers)
- Separated SQL generation into pure Rust functions (build_select_sql_from_parts, build_count_sql_from_parts, build_exists_sql_from_parts) for unit testability without GC involvement
- Repo.count extracts the "count" column from Pool.query result maps and parses as integer
- Repo.exists uses SELECT EXISTS(SELECT 1 FROM ... LIMIT 1) pattern for boolean result

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Repo read operations complete, ready for Repo writes + transactions (98-03)
- All 6 read functions (all, one, get, get_by, count, exists) registered and available
- SQL builder comprehensively handles all Query struct clause types
- Composable pipeline verified: Query.from |> Query.where |> ... compiles with Repo module available

---
*Phase: 98-query-builder-repo*
*Completed: 2026-02-16*
