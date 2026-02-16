---
phase: 98-query-builder-repo
plan: 03
subsystem: database
tags: [repo, insert, update, delete, transaction, write-operations, orm-pipeline, returning-star]

# Dependency graph
requires:
  - phase: 98-query-builder-repo
    provides: "Repo read operations (all, one, get, get_by, count, exists), Query builder, Schema metadata"
  - phase: 97-schema-metadata-sql-generation
    provides: "Orm SQL builders (build_insert_sql, build_update_sql, build_delete_sql), Pool.checkout/checkin, Pg.begin/commit/rollback"
provides:
  - "Repo.insert: INSERT with RETURNING * from Map<String,String> fields"
  - "Repo.update: UPDATE with RETURNING * by primary key from Map<String,String> fields"
  - "Repo.delete: DELETE with RETURNING * by primary key"
  - "Repo.transaction: Pool.checkout + Pg.begin + callback + Pg.commit/rollback + Pool.checkin with panic safety"
  - "Complete Repo module: 10 operations (6 read + 4 write)"
  - "Full ORM pipeline: Schema metadata + Query builder + Repo operations compiles end-to-end"
affects: [99-changesets, 100-relationships, 102-mesher-rewrite]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Repo write ops: accept Map<String,String> for fields, use ORM SQL builders with RETURNING *"
    - "Repo.transaction: checkout/begin/callback/commit-or-rollback/checkin with catch_unwind"
    - "Direct map internal access: read map header + entries array from Rust for key/value extraction"
    - "pub(crate) SQL builder wrappers: expose pure Rust functions from orm.rs for cross-module reuse"

key-files:
  created: []
  modified:
    - "crates/mesh-rt/src/db/repo.rs"
    - "crates/mesh-rt/src/db/orm.rs"
    - "crates/mesh-rt/src/lib.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"
    - "crates/mesh-repl/src/jit.rs"
    - "crates/meshc/tests/e2e.rs"

key-decisions:
  - "Map fields extracted via direct internal structure access (header + entries array) rather than mesh_map_keys/mesh_map_values for efficiency"
  - "ORM SQL builders exposed as pub(crate) wrappers (build_insert_sql_pure etc.) preserving existing private API"
  - "Repo.transaction takes pool (not conn) and manages full checkout/begin/commit/checkin lifecycle internally"
  - "RETURNING * used for all write operations (insert returns inserted row, update returns updated row, delete returns deleted row)"

patterns-established:
  - "Repo write pattern: extract map fields -> build SQL via ORM helpers -> execute via Pool.query (RETURNING) -> extract first row"
  - "Transaction lifecycle: checkout -> begin -> catch_unwind(callback) -> commit/rollback -> checkin"

# Metrics
duration: 8min
completed: 2026-02-16
---

# Phase 98 Plan 03: Repo Write Operations and Transaction Summary

**Repo.insert/update/delete with Map<String,String> fields and RETURNING *, Repo.transaction with checkout/begin/callback/commit-or-rollback/checkin lifecycle and catch_unwind panic safety**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-16T19:28:15Z
- **Completed:** 2026-02-16T19:36:39Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments
- Complete Repo module with 10 operations (6 read + 4 write) across full compiler pipeline
- Write operations use ORM SQL builders with RETURNING * for immediate row return
- Transaction composes existing Pool and Pg primitives with catch_unwind panic safety
- Full ORM pipeline compiles end-to-end: Schema metadata + Query builder + composable scopes + Repo operations
- 5 new e2e tests, all 197 tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Add Repo write operations and transaction to compiler pipeline and runtime** - `381e5a0d` (feat)
2. **Task 2: Add e2e tests for Repo write operations and full ORM pipeline** - `f6b13ee5` (test)

## Files Created/Modified
- `crates/mesh-typeck/src/infer.rs` - Added insert, update, delete, transaction type signatures to Repo module
- `crates/mesh-codegen/src/mir/lower.rs` - Added known_functions and map_builtin_name entries for 4 new functions
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - Declared mesh_repo_insert/update/delete/transaction LLVM intrinsics
- `crates/mesh-rt/src/db/repo.rs` - Implemented 4 write operations with map extraction, SQL building, and transaction lifecycle
- `crates/mesh-rt/src/db/orm.rs` - Added pub(crate) wrappers for build_insert_sql, build_update_sql, build_delete_sql
- `crates/mesh-rt/src/lib.rs` - Re-exported 4 new Repo functions
- `crates/mesh-repl/src/jit.rs` - Registered 4 new JIT symbol mappings
- `crates/meshc/tests/e2e.rs` - Added 5 new e2e tests for write operations and full ORM pipeline

## Decisions Made
- Map fields extracted via direct internal structure access (reading header len + entries array) rather than calling mesh_map_keys/mesh_map_values -- more efficient, avoids creating intermediate lists
- ORM SQL builders exposed as pub(crate) wrapper functions (build_insert_sql_pure, build_update_sql_pure, build_delete_sql_pure) to preserve existing private API while enabling cross-module reuse from repo.rs
- Repo.transaction takes pool handle (not connection) and manages the full lifecycle internally (checkout, begin, callback, commit/rollback, checkin) -- simpler API for Mesh users
- All write operations use RETURNING * so the caller gets the full inserted/updated/deleted row back as Map<String,String>

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed IO.println -> println in e2e tests**
- **Found during:** Task 2 (e2e test execution)
- **Issue:** Plan examples used `IO.println("ok")` but Mesh uses `println()` as a top-level stdlib function; `IO` is not a valid module name
- **Fix:** Changed all 5 new tests to use `println("ok")` matching existing e2e test convention
- **Files modified:** crates/meshc/tests/e2e.rs
- **Committed in:** f6b13ee5 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Trivial naming fix, no scope change.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 98 (Query Builder + Repo) is fully complete
- Complete ORM CRUD stack available: Schema metadata -> Query builder -> Repo read/write operations
- Ready for Phase 99 (Changesets) which will add validation and type-safe field management
- Repo.insert/update accept Map<String,String> for Phase 98; changeset integration points prepared for Phase 99

---
*Phase: 98-query-builder-repo*
*Completed: 2026-02-16*

## Self-Check: PASSED
- All 8 modified files exist on disk
- Both task commits verified: 381e5a0d (feat), f6b13ee5 (test)
- 197 e2e tests pass, 455 runtime tests pass, zero build warnings
