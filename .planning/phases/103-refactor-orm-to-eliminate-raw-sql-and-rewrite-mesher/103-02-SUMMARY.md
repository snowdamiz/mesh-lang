---
phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher
plan: 02
subsystem: database
tags: [query-builder, raw-sql, orm, postgres, compiler-pipeline]

# Dependency graph
requires:
  - phase: 98-query-builder-repo
    provides: Query builder with select/where/fragment and SQL generation
provides:
  - Query.select_raw for raw SQL expressions in SELECT clauses (e.g., count(*), casts)
  - Query.where_raw for raw SQL WHERE clauses with parameter binding
  - RAW: prefix convention in SQL builder for verbatim emission
affects: [103-03, 103-04, 103-05, mesher-queries-conversion]

# Tech tracking
tech-stack:
  added: []
  patterns: [RAW-prefix convention for bypassing quote_ident in SQL generation, ?-to-$N parameter renumbering]

key-files:
  created: []
  modified:
    - crates/mesh-rt/src/db/query.rs
    - crates/mesh-rt/src/db/repo.rs
    - crates/mesh-rt/src/lib.rs
    - crates/mesh-codegen/src/codegen/intrinsics.rs
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/mesh-typeck/src/infer.rs
    - crates/mesh-repl/src/jit.rs

key-decisions:
  - "RAW: prefix convention reuses existing SLOT_SELECT and SLOT_WHERE_CLAUSES (no new slots, no ABI change)"
  - "? placeholder in where_raw clauses gets renumbered to $N by SQL builder, matching fragment convention"
  - "select_raw appends RAW:-prefixed entries to existing select list, allowing mixed raw+quoted SELECT fields"

patterns-established:
  - "RAW: prefix on slot entries: SQL builder checks strip_prefix('RAW:') before quote_ident, emits verbatim if prefixed"
  - "Parameter renumbering: ? placeholders in raw clauses replaced with sequential $N continuing from current param_idx"

# Metrics
duration: 6min
completed: 2026-02-16
---

# Phase 103 Plan 02: Query Builder Raw Extensions Summary

**Query.select_raw and Query.where_raw with RAW: prefix convention for verbatim SQL emission in SELECT and WHERE clauses**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-17T00:09:19Z
- **Completed:** 2026-02-17T00:15:49Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Added mesh_query_select_raw and mesh_query_where_raw runtime functions following clone-and-modify Query pattern
- Updated SQL builder in all three query types (select, count, exists) to handle RAW: prefixed entries
- Full compiler pipeline integration: intrinsics, known_functions, map_builtin_name, typeck, JIT
- 8 new unit tests verifying RAW: handling with proper $N parameter numbering

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement select_raw and where_raw in query.rs runtime** - `c0bae5e3` (feat)
2. **Task 2: Update repo.rs SQL builder to handle RAW: prefixed entries + register compiler pipeline** - `81d1ac07` (feat)

## Files Created/Modified
- `crates/mesh-rt/src/db/query.rs` - Added mesh_query_select_raw and mesh_query_where_raw extern C functions
- `crates/mesh-rt/src/db/repo.rs` - Updated build_select_sql_from_parts, build_count_sql_from_parts, build_exists_sql_from_parts with RAW: prefix handling; added 8 unit tests
- `crates/mesh-rt/src/lib.rs` - Re-exported new functions
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - LLVM declarations for mesh_query_select_raw and mesh_query_where_raw
- `crates/mesh-codegen/src/mir/lower.rs` - known_functions signatures and map_builtin_name mappings
- `crates/mesh-typeck/src/infer.rs` - Query.select_raw and Query.where_raw type signatures
- `crates/mesh-repl/src/jit.rs` - JIT symbol registrations for REPL

## Decisions Made
- RAW: prefix convention reuses existing SLOT_SELECT and SLOT_WHERE_CLAUSES (no new slots, no ABI change to 104-byte Query layout)
- ? placeholder in where_raw clauses gets renumbered to $N by SQL builder, matching the existing fragment convention
- select_raw appends RAW:-prefixed entries to existing select list, allowing mixed raw+quoted SELECT fields in same query

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Missing re-export in mesh-rt lib.rs**
- **Found during:** Task 2 (compiler pipeline registration)
- **Issue:** mesh_query_select_raw and mesh_query_where_raw not re-exported from mesh-rt crate, causing mesh-repl JIT registration to fail
- **Fix:** Added both functions to the pub use db::query re-export block in lib.rs
- **Files modified:** crates/mesh-rt/src/lib.rs
- **Verification:** cargo build -p mesh-repl passes
- **Committed in:** 81d1ac07 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential for compilation. No scope creep.

## Issues Encountered
None - the plan file mentioned the re-export was needed in lib.rs but did not list it as a file to modify; this was caught by the build error.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Query.select_raw and Query.where_raw are ready for use in Mesh source files
- Plan 103-03 (extended repo operations) can now use these raw query extensions
- Plans 103-04/103-05 can convert remaining raw SQL calls in mesher to ORM with select_raw/where_raw

---
*Phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher*
*Completed: 2026-02-16*

## Self-Check: PASSED
- All 7 modified files verified on disk
- All 2 task commits verified in git log (c0bae5e3, 81d1ac07)
- 508 mesh-rt tests passing (0 failures)
- Full workspace build passing
