---
phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher
plan: 03
subsystem: database
tags: [repo, orm, update-where, delete-where, query-raw, execute-raw, postgres]

# Dependency graph
requires:
  - phase: 98-query-builder-repo
    provides: "Repo module (insert/update/delete/all/one/get/count/exists), Query builder with WHERE slots"
  - phase: 103-02
    provides: "RAW: prefix WHERE clause handling, Query.where_raw"
provides:
  - "Repo.update_where: UPDATE SET + WHERE from Map fields + Query conditions"
  - "Repo.delete_where: DELETE WHERE from Query conditions returning affected count"
  - "Repo.query_raw: explicit raw SQL query escape hatch (Repo-namespaced)"
  - "Repo.execute_raw: explicit raw SQL execute escape hatch (Repo-namespaced)"
  - "build_where_from_query_parts shared helper for WHERE clause construction"
affects: [103-04, 103-05]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Shared WHERE builder helper with parameterized offset for reuse across update_where/delete_where", "Safety guards rejecting empty WHERE on destructive operations"]

key-files:
  created: []
  modified:
    - "crates/mesh-rt/src/db/repo.rs"
    - "crates/mesh-rt/src/lib.rs"
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-repl/src/jit.rs"

key-decisions:
  - "build_where_from_query_parts extracted as shared helper with start_idx offset for reuse"
  - "update_where and delete_where reject empty WHERE clauses as safety guard against accidental table-wide mutations"
  - "query_raw and execute_raw are thin wrappers over Pool.query/Pool.execute namespaced under Repo for explicit intent"
  - "update_where returns first row via RETURNING *; delete_where returns affected count via Pool.execute"

patterns-established:
  - "WHERE clause builder with parameterized start index: build_where_from_query_parts(clauses, params, start_idx) -> (sql, params, next_idx)"
  - "Safety guards: destructive operations with Query WHERE slots require non-empty WHERE clauses"

# Metrics
duration: 6min
completed: 2026-02-16
---

# Phase 103 Plan 03: Extended Repo Write Operations Summary

**Repo.update_where/delete_where for Query-based WHERE conditions, plus Repo.query_raw/execute_raw explicit raw SQL escape hatches with full compiler pipeline integration**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-17T00:09:37Z
- **Completed:** 2026-02-17T00:15:57Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Four new Repo functions with runtime implementations, safety guards, and shared WHERE builder
- Full compiler pipeline integration: LLVM intrinsics, MIR known_functions, map_builtin_name, typeck signatures, JIT symbols
- 6 new unit tests for build_where_from_query_parts covering equality, offset, IS NULL, RAW prefix, IN clause, and default eq
- All 508 mesh-rt tests pass, full workspace builds cleanly

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement update_where, delete_where, query_raw, execute_raw in repo.rs** - `0394508a` (feat)
2. **Task 2: Register all four new Repo functions in compiler pipeline** - `08e9b2c1` (feat)

## Files Created/Modified
- `crates/mesh-rt/src/db/repo.rs` - Four new extern C functions + shared WHERE builder + 6 unit tests
- `crates/mesh-rt/src/lib.rs` - Re-export four new functions
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - LLVM function type declarations (already present from Plan 02)
- `crates/mesh-codegen/src/mir/lower.rs` - known_functions + map_builtin_name entries
- `crates/mesh-typeck/src/infer.rs` - Repo module type signatures for typeck
- `crates/mesh-repl/src/jit.rs` - JIT symbol registration for REPL support

## Decisions Made
- Extracted build_where_from_query_parts as shared helper with configurable start_idx for parameter numbering offset (SET columns use $1..$N, WHERE starts at $N+1)
- update_where uses RETURNING * and returns first updated row (Map), matching Repo.update pattern
- delete_where uses Pool.execute and returns affected count (Int), matching bulk operation semantics
- query_raw/execute_raw are intentionally thin wrappers -- the Repo namespace signals "this is intentional raw SQL" vs accidental Pool.query usage
- Safety guards: both update_where and delete_where return err_result if WHERE clauses are empty, preventing accidental table-wide mutations

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed unnecessary unsafe blocks in query_raw/execute_raw**
- **Found during:** Task 1
- **Issue:** query_raw and execute_raw used `unsafe {}` blocks around safe pointer casts and extern C function calls
- **Fix:** Removed unnecessary unsafe blocks (the casts are safe and the called functions are extern C)
- **Files modified:** crates/mesh-rt/src/db/repo.rs
- **Committed in:** 0394508a (part of Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Minor compiler warning fix. No scope creep.

## Issues Encountered
- intrinsics.rs already contained the four new function declarations from Plan 02 execution, so the edit in Task 2 was effectively a no-op for that file. All other pipeline files were updated correctly.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All four new Repo functions ready for use in Mesher rewrite (Plan 04/05)
- Repo.update_where enables converting conditional UPDATE queries from raw SQL to ORM
- Repo.delete_where enables converting conditional DELETE queries from raw SQL to ORM
- Repo.query_raw/execute_raw provide explicit escape hatch for Tier 3 complex queries

## Self-Check: PASSED

All source files verified present. Both task commits (0394508a, 08e9b2c1) verified in git log. All four functions present in repo.rs, re-exported in lib.rs, registered in intrinsics.rs, lower.rs, infer.rs, and jit.rs.

---
*Phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher*
*Completed: 2026-02-16*
