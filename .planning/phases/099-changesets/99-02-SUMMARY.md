---
phase: 099-changesets
plan: 02
subsystem: database
tags: [changeset, repo, constraint-mapping, orm, validation, persistence]

# Dependency graph
requires:
  - phase: 099-changesets
    plan: 01
    provides: Changeset module with cast/validators/accessors, 8-slot struct layout
  - phase: 098-query-builder-repo
    provides: Repo module, Pool.query, ORM SQL builders, Map/List runtime collections
provides:
  - Repo.insert_changeset validating before SQL and mapping PG constraint errors
  - Repo.update_changeset validating before SQL and mapping PG constraint errors
  - PgError struct with full ErrorResponse field extraction (SQLSTATE, constraint, table, column)
  - Constraint-to-changeset error mapping (unique, foreign key, not null violations)
  - Structured tab-separated error string format for PG error propagation
affects: [100-relationships, 101-migrations, 102-mesher-rewrite]

# Tech tracking
tech-stack:
  added: []
  patterns: [structured-error-string, constraint-to-field-mapping, validate-before-sql]

key-files:
  created: []
  modified:
    - crates/mesh-rt/src/db/pg.rs
    - crates/mesh-rt/src/db/changeset.rs
    - crates/mesh-rt/src/db/repo.rs
    - crates/mesh-rt/src/lib.rs
    - crates/mesh-typeck/src/infer.rs
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/mesh-codegen/src/codegen/intrinsics.rs
    - crates/mesh-repl/src/jit.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "Tab-separated structured error string format for PG errors: sqlstate\\tconstraint\\ttable\\tcolumn\\tmessage"
  - "Constraint name parsing follows PostgreSQL conventions: {table}_{column}_{key|fkey|pkey|check}"
  - "Invalid changesets return immediately without SQL execution (Err(changeset))"
  - "Unmapped PG errors add generic _base error to changeset rather than losing the error"

patterns-established:
  - "Validate-before-SQL: changeset functions check valid flag before executing database queries"
  - "Structured error propagation: PG errors carry SQLSTATE and constraint info through tab-separated strings"
  - "Constraint-to-field mapping: database constraint names automatically resolve to user-facing field names"

# Metrics
duration: 8min
completed: 2026-02-16
---

# Phase 99 Plan 02: Repo Changeset Integration Summary

**PG constraint error mapping and Repo.insert_changeset/update_changeset with validate-before-SQL pattern across the full compiler pipeline**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-16T20:38:03Z
- **Completed:** 2026-02-16T20:46:00Z
- **Tasks:** 3
- **Files modified:** 9

## Accomplishments
- Enhanced PG wire protocol parser with PgError struct extracting SQLSTATE, constraint, table, column from ErrorResponse
- Implemented Repo.insert_changeset and Repo.update_changeset with validate-before-SQL pattern and PG constraint-to-changeset error mapping
- Registered both new Repo functions across all 5 compiler layers (typeck, MIR, LLVM, JIT, lib.rs)
- Added 6 e2e tests and 6 unit tests covering constraint mapping, PG error parsing, and full changeset pipeline -- all 213 e2e tests pass

## Task Commits

Each task was committed atomically:

1. **Task 1a: Enhanced PG error parsing and constraint mapping** - `edb9516f` (feat)
2. **Task 1b: Repo changeset functions and compiler pipeline integration** - `78080dba` (feat)
3. **Task 2: Add e2e tests for Repo changeset integration** - `6604de59` (test)

**Plan metadata:** TBD (docs: complete plan)

## Files Created/Modified
- `crates/mesh-rt/src/db/pg.rs` - PgError struct, parse_error_response_full, structured error string format
- `crates/mesh-rt/src/db/changeset.rs` - map_constraint_error, extract_field_from_constraint, add_constraint_error_to_changeset
- `crates/mesh-rt/src/db/repo.rs` - mesh_repo_insert_changeset, mesh_repo_update_changeset, parse_pg_error_string, 6 unit tests
- `crates/mesh-rt/src/lib.rs` - Re-exported mesh_repo_insert_changeset, mesh_repo_update_changeset
- `crates/mesh-typeck/src/infer.rs` - Repo.insert_changeset and Repo.update_changeset type signatures
- `crates/mesh-codegen/src/mir/lower.rs` - known_functions and map_builtin_name entries for both functions
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - LLVM intrinsic declarations for both functions
- `crates/mesh-repl/src/jit.rs` - JIT symbol registrations for both functions
- `crates/meshc/tests/e2e.rs` - 6 new e2e tests for Repo changeset integration

## Decisions Made
- Used tab-separated structured error string format (`sqlstate\tconstraint\ttable\tcolumn\tmessage`) for PG error propagation between pg.rs and repo.rs
- PostgreSQL constraint names parsed using `{table}_{column}_{suffix}` convention for automatic field name extraction
- Invalid changesets short-circuit immediately without touching the database, returning the changeset with existing errors
- Unmapped PG errors (unknown SQLSTATE) add a `_base` key error with "database error" message to the changeset

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Mesh struct field syntax uses `::` not `:`**
- **Found during:** Task 2 (e2e test execution)
- **Issue:** Test 14 used `id: String` syntax for struct fields, but Mesh uses `id :: String`
- **Fix:** Changed struct definition to use `::` separator in the e2e test
- **Files modified:** crates/meshc/tests/e2e.rs
- **Verification:** Test 14 passes, all 213 e2e tests pass
- **Committed in:** 6604de59 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Test-only fix for incorrect Mesh syntax in test code. No impact on runtime implementation.

## Issues Encountered
None -- all implementation code worked as specified.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 99 (Changesets) is now complete with both plans executed
- Full changeset pipeline works: Changeset.cast -> validators -> Repo.insert_changeset/update_changeset
- Ready for Phase 100 (Relationships) or Phase 101 (Migrations)
- Constraint error mapping provides foundation for Mesher rewrite (Phase 102)

## Self-Check: PASSED

All 9 modified files verified present. All 3 task commits (edb9516f, 78080dba, 6604de59) verified in git log. All 7 key artifacts (PgError, parse_error_response_full, map_constraint_error, extract_field_from_constraint, add_constraint_error_to_changeset, mesh_repo_insert_changeset, mesh_repo_update_changeset) verified present.

---
*Phase: 099-changesets*
*Completed: 2026-02-16*
