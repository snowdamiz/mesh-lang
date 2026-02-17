---
phase: 104-fix-mesher-compilation-errors
plan: 01
subsystem: compiler
tags: [typeck, codegen, mir, schema-metadata, cross-module-imports]

# Dependency graph
requires:
  - phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher
    provides: "Mesher rewrite with Repo.* ORM calls and deriving(Schema) structs"
provides:
  - "Concrete Result types for Repo.insert/get/get_by/all/delete in typechecker"
  - "Cross-module Schema metadata resolution in MIR lowerer"
  - "Zero-error meshc build mesher producing working binary"
affects: [105-runtime-verification, mesher, compiler]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Cross-module Schema metadata registration pattern (same as FromJson/ToJson/FromRow)"
    - "MIR lowerer known_functions pre-seeding for imported Schema-derived structs"

key-files:
  created: []
  modified:
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"

key-decisions:
  - "Repo typeck signatures use concrete Result<Map/List<Map>, String> types matching runtime behavior"
  - "Schema metadata functions registered in MIR known_functions for cross-module imports (same pattern as FromJson/ToJson)"

patterns-established:
  - "Cross-module trait function registration: when adding new trait-derived functions (like Schema metadata), register them in known_functions during lower_to_mir for imported structs"

# Metrics
duration: 12min
completed: 2026-02-17
---

# Phase 104 Plan 01: Fix Mesher Compilation Errors Summary

**Concrete Repo typeck signatures and cross-module Schema metadata resolution eliminating all Mesher compilation errors**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-17T01:23:00Z
- **Completed:** 2026-02-17T01:35:11Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Updated 5 Repo function typeck signatures from opaque Ptr to concrete Result types (insert, get, get_by, all, delete)
- Fixed cross-module Schema metadata resolution bug where imported structs could not call `__table__()` etc.
- meshc build mesher completes with zero errors, producing a working 8.8MB binary

## Task Commits

Each task was committed atomically:

1. **Task 1: Update Repo typeck signatures from Ptr to concrete Result types** - `1ba2db20` (feat)
2. **Task 2: Verify Mesher builds with zero errors and fix remaining issues** - `23c4cdfd` (fix)

## Files Created/Modified
- `crates/mesh-typeck/src/infer.rs` - Updated Repo.insert/get/get_by/all/delete return types from Ptr to concrete Result<Map/List<Map>, String>; also changed Repo.insert third parameter from Ptr to Map<String,String>
- `crates/mesh-codegen/src/mir/lower.rs` - Added Schema metadata known_functions registration for cross-module imported structs (parallel to existing FromJson/ToJson/FromRow registration)
- `mesher/mesher` - Freshly compiled binary (8.8MB)

## Decisions Made
- Repo typeck signatures match runtime behavior exactly: single-row operations return `Result<Map<String,String>, String>`, multi-row operations return `Result<List<Map<String,String>>, String>`
- Schema metadata cross-module registration follows the same pattern established by FromJson/ToJson/FromRow trait function registration in the MIR lowerer

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed cross-module Schema metadata resolution in MIR lowerer**
- **Found during:** Task 2 (Verify Mesher builds)
- **Issue:** After Task 1's typeck fix, the original 46 errors were already resolved (apparently by prior work), but a remaining "Undefined variable 'Organization'" codegen error in `insert_org` function prevented compilation. Root cause: when `Storage.Queries` imports `Organization` from `Types.Project`, the MIR lowerer's `known_functions` did not contain `Organization____table__` because Schema metadata functions were only generated for locally-defined structs, not imported ones.
- **Fix:** Added Schema metadata known_functions registration in `lower_to_mir` (parallel to existing FromJson/ToJson/FromRow cross-module registration). Registers `__table__`, `__fields__`, `__primary_key__`, `__relationships__`, `__field_types__`, `__relationship_meta__`, and per-field `__*_col__` for all imported structs with `deriving(Schema)`.
- **Files modified:** `crates/mesh-codegen/src/mir/lower.rs`
- **Verification:** `cargo run --release -- build mesher` produces zero errors and a working binary
- **Committed in:** `23c4cdfd` (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug fix)
**Impact on plan:** Bug fix was essential for correct cross-module compilation. No scope creep -- this is the same pattern used for other cross-module trait functions.

## Issues Encountered
- The plan anticipated 46 compilation errors, but the codebase on main only had 1 error ("Undefined variable 'Organization'"). The 46 errors referenced in STATE.md were apparently from an earlier analysis. The typeck signature changes in Task 1 were still necessary and correct (they prevent future errors when the typechecker encounters Repo calls), but the actual blocking error was a MIR lowerer bug in cross-module Schema metadata resolution.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Mesher binary compiles cleanly and is ready for Phase 105 runtime verification
- All 6 affected Mesher files (queries.mpl, org.mpl, project.mpl, user.mpl, team.mpl, main.mpl) compile without errors
- No blockers remaining

## Self-Check: PASSED

All artifacts verified:
- crates/mesh-typeck/src/infer.rs: FOUND
- crates/mesh-codegen/src/mir/lower.rs: FOUND
- mesher/mesher: FOUND
- 104-01-SUMMARY.md: FOUND
- Commit 1ba2db20: FOUND
- Commit 23c4cdfd: FOUND

---
*Phase: 104-fix-mesher-compilation-errors*
*Completed: 2026-02-17*
