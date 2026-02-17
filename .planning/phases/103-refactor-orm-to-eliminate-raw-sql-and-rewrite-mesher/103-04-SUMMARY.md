---
phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher
plan: 04
subsystem: database
tags: [orm, repo, query-raw, execute-raw, pool-elimination, postgres, mesher]

# Dependency graph
requires:
  - phase: 103-02
    provides: "Query.select_raw and Query.where_raw raw SQL extensions"
  - phase: 103-03
    provides: "Repo.query_raw, Repo.execute_raw, Repo.update_where, Repo.delete_where"
provides:
  - "Zero Pool.query/Pool.execute calls in queries.mpl -- all database access through Repo namespace"
  - "Concrete typeck signatures for Repo.query_raw and Repo.execute_raw matching Pool.query/Pool.execute"
affects: [103-05]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Repo.query_raw/execute_raw as 1:1 replacements for Pool.query/Pool.execute with concrete type signatures"]

key-files:
  created: []
  modified:
    - "mesher/storage/queries.mpl"
    - "crates/mesh-typeck/src/infer.rs"

key-decisions:
  - "Repo.query_raw/execute_raw typeck signatures changed from Ptr to concrete types matching Pool.query/Pool.execute (Result<List<Map<String,String>>,String> and Result<Int,String>)"
  - "ORM Query builder conversions (Repo.exists) deferred: all Repo.all/one/exists return Ptr in typeck, incompatible with concrete function return types"
  - "Repo.query_raw used for all query functions regardless of complexity -- consistent namespace is the win, not forced Query builder usage"

patterns-established:
  - "Repo.query_raw is the standard for all SQL queries in Mesh source files; Pool.query reserved only for runtime internals"
  - "Repo.execute_raw is the standard for all SQL mutations in Mesh source files; Pool.execute reserved only for runtime internals"

# Metrics
duration: 9min
completed: 2026-02-17
---

# Phase 103 Plan 04: Pool.query/Pool.execute Elimination Summary

**All 50+ database functions in queries.mpl migrated from Pool.query/Pool.execute to Repo.query_raw/Repo.execute_raw with concrete typeck signatures for type-safe Mesh compilation**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-17T00:18:56Z
- **Completed:** 2026-02-17T00:28:35Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Eliminated all 62 Pool.query/Pool.execute call occurrences from queries.mpl (34 query_raw + 16 execute_raw conversions)
- Fixed Repo.query_raw/execute_raw typeck signatures from opaque Ptr to concrete types matching Pool.query/Pool.execute
- All function signatures preserved -- zero caller changes needed across mesher codebase
- No new compilation errors introduced (47 pre-existing errors remain unchanged)

## Task Commits

Each task was committed atomically:

1. **Task 1: Replace all Pool.query/Pool.execute with Repo namespace** - `c0e12f74` (feat)
2. **Task 2: Selective Query builder upgrades** - No commit (pragmatic decision: all candidates blocked by Ptr return type constraint)

## Files Created/Modified
- `mesher/storage/queries.mpl` - All Pool.query -> Repo.query_raw, all Pool.execute -> Repo.execute_raw, updated module header comment
- `crates/mesh-typeck/src/infer.rs` - Repo.query_raw signature: (PoolHandle, String, List<String>) -> Result<List<Map<String,String>>, String>; Repo.execute_raw signature: (PoolHandle, String, List<String>) -> Result<Int, String>

## Decisions Made
- Repo.query_raw/execute_raw typeck signatures changed from `Ptr` return to concrete types. The original `Ptr` signatures caused type unification failures since queries.mpl functions declare concrete return types like `List<Map<String, String>>!String`. The concrete signatures match `Pool.query`/`Pool.execute` exactly, enabling seamless replacement.
- ORM Query builder conversions (is_issue_discarded -> Repo.exists, check_new_issue -> Repo.exists) were attempted but reverted. All Repo.all/one/count/exists functions return `Ptr` in the type checker, which cannot unify with concrete types like `Bool!String`. Used Repo.query_raw instead, which preserves identical behavior.
- Task 2 (selective Query builder upgrades) resulted in no changes. Every candidate function returns concrete types incompatible with Repo.all's `Ptr` return type. The Repo.query_raw versions are already clean and readable -- forcing Query builder usage would provide no benefit and would require fixing all Repo return types in typeck (a much larger scope change).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Repo.query_raw/execute_raw typeck signatures**
- **Found during:** Task 1 (mesher build verification)
- **Issue:** Repo.query_raw and Repo.execute_raw were typed as returning `Ptr` and accepting `Ptr` params, but Pool.query uses `List<String>` params and returns `Result<List<Map<String,String>>, String>`. This caused type unification errors at every call site in queries.mpl.
- **Fix:** Changed typeck signatures to use concrete types matching Pool.query/Pool.execute
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** mesher build produces 47 errors (all pre-existing, none from queries.mpl)
- **Committed in:** c0e12f74 (Task 1 commit)

**2. [Rule 1 - Bug] Reverted Repo.exists ORM conversions**
- **Found during:** Task 1 (mesher build verification after ORM conversion attempt)
- **Issue:** is_issue_discarded and check_new_issue were converted to use Query + Repo.exists, but Repo.exists returns `Ptr` in typeck while these functions return `Bool!String`. This caused 11 new compilation errors including cascading failures in callers.
- **Fix:** Reverted both functions to use Repo.query_raw with identical SQL as original Pool.query calls
- **Files modified:** mesher/storage/queries.mpl
- **Verification:** Error count returned to 47 (matching pre-existing baseline)
- **Committed in:** c0e12f74 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs)
**Impact on plan:** Both fixes necessary for compilation correctness. No scope creep. The plan explicitly anticipated this with "PRAGMATIC APPROACH" and "Be pragmatic" guidance.

## Issues Encountered
- The Ptr-based opaque type system for Repo functions prevents using Repo.all/one/exists/count in Mesh source files that declare concrete return types. This is a known design limitation -- the Repo typeck uses `Ptr` for internal type erasure, but this prevents the type checker from verifying return type compatibility. This does not affect Plan 103-05 (which focuses on different concerns) but would need to be addressed if full ORM Query builder adoption is desired in the future.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All database access in queries.mpl now uses the Repo namespace
- Plan 103-05 (final verification and cleanup) can proceed
- Future work: fixing Repo function typeck signatures to use concrete types (like was done for query_raw/execute_raw) would enable full Query builder adoption

## Self-Check: PASSED
- All 2 modified source files verified on disk (mesher/storage/queries.mpl, crates/mesh-typeck/src/infer.rs)
- Task 1 commit verified in git log (c0e12f74)
- Zero Pool.query/Pool.execute calls in queries.mpl (verified via grep)
- 47 mesher build errors (all pre-existing, zero new)
- 90 Rust tests passing (2 pre-existing HTTP integration failures)

---
*Phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher*
*Completed: 2026-02-17*
