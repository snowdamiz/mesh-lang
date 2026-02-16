---
phase: 100-relationships-preloading
plan: 02
subsystem: runtime
tags: [preloading, batch-queries, n-plus-one, relationships, repo, orm]

# Dependency graph
requires:
  - phase: 100-relationships-preloading
    plan: 01
    provides: "__relationship_meta__() returning 5-field encoded relationship strings"
  - phase: 98-query-builder-repo
    provides: "Repo module pattern, mesh_pool_query, result types"
provides:
  - "Repo.preload(pool, rows, associations, meta) batch preloader"
  - "Direct preloading: has_many (List), has_one/belongs_to (single Map)"
  - "Nested preloading via dot-separated paths with positional re-stitching"
  - "Empty parent list short-circuit, ID deduplication"
affects: [101-migrations, 102-mesher-rewrite]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Batch IN queries: collect parent IDs, single SELECT WHERE fk IN (...) per association"
    - "Positional re-stitching: track (parent_idx, pos_in_list) for nested preload rebuild"
    - "Depth-sorted association processing: direct first, then nested"

key-files:
  created: []
  modified:
    - "crates/mesh-rt/src/db/repo.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"
    - "crates/mesh-rt/src/lib.rs"
    - "crates/mesh-repl/src/jit.rs"
    - "crates/meshc/tests/e2e.rs"

key-decisions:
  - "Preload uses separate WHERE fk IN (...) query per association level (not JOINs)"
  - "Nested preloading uses positional tracking for re-stitching rather than ID-based matching"

patterns-established:
  - "Batch preloader pattern: collect IDs -> deduplicate -> IN query -> group by FK -> attach"

# Metrics
duration: 8min
completed: 2026-02-16
---

# Phase 100 Plan 02: Repo.preload Summary

**Repo.preload batch preloader with WHERE fk IN (...) queries, FK grouping, nested dot-path support, and depth-sorted association processing**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-16T21:22:35Z
- **Completed:** 2026-02-16T21:31:13Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- `Repo.preload(pool, rows, associations, meta)` available across full compiler pipeline (typeck, MIR, LLVM, JIT, runtime)
- Runtime batch loads associated records using single `WHERE fk IN (...)` query per association
- has_many attaches List of rows, belongs_to/has_one attaches single Map row
- Nested preloading processes levels in depth order with positional re-stitching
- Empty parent list short-circuits without executing queries
- Duplicate parent IDs deduplicated in IN clause
- 4 unit tests + 2 e2e tests, 218 total e2e tests with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Register Repo.preload across compiler pipeline and implement runtime preloader** - `8adbed5c` (feat)
2. **Task 2: Add e2e tests for Repo.preload compilation and unit tests for preload SQL builder** - `38192750` (test)

## Files Created/Modified
- `crates/mesh-typeck/src/infer.rs` - Repo.preload type signature (4 params: pool, rows, assocs, meta)
- `crates/mesh-codegen/src/mir/lower.rs` - known_function + map_builtin_name for mesh_repo_preload
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - LLVM intrinsic declaration
- `crates/mesh-rt/src/db/repo.rs` - Runtime: preload_direct, preload_nested, attach_empty_association, mesh_repo_preload + 4 unit tests
- `crates/mesh-rt/src/lib.rs` - Re-export mesh_repo_preload
- `crates/mesh-repl/src/jit.rs` - JIT symbol for mesh_repo_preload
- `crates/meshc/tests/e2e.rs` - 2 e2e tests: type check + merged metadata

## Decisions Made
- Preload uses separate WHERE fk IN (...) query per association level rather than JOINs. This matches Ecto's preloader design and avoids cartesian product issues with multiple has_many associations.
- Nested preloading uses positional tracking (parent_idx, pos_in_list) for re-stitching enriched intermediate rows back into parent association lists, which preserves ordering without requiring ID-based matching.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed i64/u64 type mismatches in preload implementation**
- **Found during:** Task 1 (runtime implementation)
- **Issue:** Plan code used u64 for list length/index parameters, but mesh_list_length returns i64 and mesh_list_get takes i64
- **Fix:** Changed attach_empty_association row_count parameter from u64 to i64, position_map from Vec<(u64,u64)> to Vec<(i64,i64)>, parent_groups HashMap key from u64 to i64, idx cast from u64 to i64
- **Files modified:** crates/mesh-rt/src/db/repo.rs
- **Verification:** cargo build --workspace compiles cleanly
- **Committed in:** 8adbed5c (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Type mismatch fix necessary for compilation. No scope change.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 100 (Relationships + Preloading) is now complete
- Repo.preload provides N+1 query elimination for all relationship kinds
- Ready for Phase 101 (Migrations) or Phase 102 (Mesher Rewrite Validation)
- 218 e2e tests + 465 unit tests pass with zero regressions

## Self-Check: PASSED

All files verified present. All commit hashes verified in git log. All key patterns verified in source files.

---
*Phase: 100-relationships-preloading*
*Completed: 2026-02-16*
