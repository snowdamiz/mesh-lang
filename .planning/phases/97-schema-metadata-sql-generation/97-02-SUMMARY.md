---
phase: 97-schema-metadata-sql-generation
plan: 02
subsystem: database
tags: [orm, sql-generation, parameterized-queries, postgresql, llvm-intrinsics]

# Dependency graph
requires:
  - phase: 97-01
    provides: "Schema metadata codegen (deriving(Schema), __table__, __fields__, etc.)"
  - phase: 58
    provides: "Row parsing runtime (mesh_row_from_row_get, from_row pattern)"
provides:
  - "Four parameterized SQL builder runtime functions (SELECT, INSERT, UPDATE, DELETE)"
  - "Orm module callable from Mesh code (Orm.build_select, etc.)"
  - "LLVM intrinsic declarations for ORM functions"
  - "JIT symbol mappings for REPL support"
affects: [98-query-builder, 99-repo-pattern, 100-changesets, 102-migrations]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Pure Rust helper + extern C wrapper pattern for testable SQL generation", "Sequential $N parameter numbering across SET+WHERE clauses"]

key-files:
  created:
    - crates/mesh-rt/src/db/orm.rs
  modified:
    - crates/mesh-rt/src/db/mod.rs
    - crates/mesh-rt/src/lib.rs
    - crates/mesh-codegen/src/codegen/intrinsics.rs
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/mesh-typeck/src/infer.rs
    - crates/mesh-repl/src/jit.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "Pure Rust helpers (build_select_sql, etc.) separated from extern C wrappers for unit testability without GC"
  - "WHERE clause format: 'column op' space-separated (e.g. 'name =', 'age >', 'status IS NULL')"
  - "IS NULL/IS NOT NULL in WHERE clauses do not consume parameter slots"

patterns-established:
  - "Orm module: Orm.build_select/insert/update/delete callable from Mesh code via module-qualified access"
  - "SQL parameter numbering: UPDATE SET columns get $1..$N, WHERE continues from $N+1"

# Metrics
duration: 6min
completed: 2026-02-16
---

# Phase 97 Plan 02: Runtime SQL Generation Summary

**Four parameterized SQL builder functions (SELECT/INSERT/UPDATE/DELETE) with $N placeholders, double-quoted identifiers, and Orm module integration for Mesh code**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-16T18:11:37Z
- **Completed:** 2026-02-16T18:17:45Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments
- Implemented 4 extern C SQL builder functions in mesh-rt with full PostgreSQL SQL generation
- Pure Rust helper layer enables 18 unit tests without GC initialization
- Registered Orm module across full compiler pipeline: typechecker, MIR lowerer, LLVM intrinsics, JIT
- 5 e2e tests verify correct SQL generation from Mesh code (Orm.build_select/insert/update/delete)
- All 180 e2e tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement runtime SQL builder functions in mesh-rt** - `e510ad8b` (feat)
2. **Task 2: Declare intrinsics, register known functions, and add e2e tests** - `d7c4fca1` (feat)

## Files Created/Modified
- `crates/mesh-rt/src/db/orm.rs` - Four SQL builder functions with pure Rust helpers and 18 unit tests
- `crates/mesh-rt/src/db/mod.rs` - orm module registration
- `crates/mesh-rt/src/lib.rs` - Re-export ORM functions
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - LLVM intrinsic declarations for 4 ORM functions
- `crates/mesh-codegen/src/mir/lower.rs` - known_functions, map_builtin_name, STDLIB_MODULES registration
- `crates/mesh-typeck/src/infer.rs` - Orm module type signatures, STDLIB_MODULE_NAMES
- `crates/mesh-repl/src/jit.rs` - JIT symbol mappings for REPL support
- `crates/meshc/tests/e2e.rs` - 5 e2e tests for ORM SQL generation from Mesh code

## Decisions Made
- Pure Rust helpers separated from extern C wrappers: enables unit testing without Mesh runtime GC initialization
- WHERE clause format uses space-separated "column op" entries (e.g. "name =", "age >", "status IS NULL")
- IS NULL/IS NOT NULL clauses do not consume parameter slots (no $N placeholder needed)
- UPDATE parameter numbering: SET columns get $1..$N, WHERE clauses continue from $N+1

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Phase 97 complete: schema metadata (97-01) and SQL generation (97-02) both done
- Phase 98 (Query Builder) can use Orm.build_select/insert/update/delete from Mesh code
- All ORM functions return MeshString pointers suitable for passing to Pg.execute/Pg.query

---
*Phase: 97-schema-metadata-sql-generation*
*Completed: 2026-02-16*
