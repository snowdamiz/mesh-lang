---
phase: 100-relationships-preloading
plan: 01
subsystem: compiler
tags: [schema, metadata, relationships, fk-inference, mir]

# Dependency graph
requires:
  - phase: 96-compiler-additions
    provides: "Schema derive with __relationships__() 3-field encoding"
  - phase: 97-schema-metadata-sql-generation
    provides: "Schema metadata pattern (__table__, __fields__, __field_types__)"
provides:
  - "__relationship_meta__() returning 5-field encoded relationship strings"
  - "FK inference convention: has_many/has_one -> {owner_lowercase}_id, belongs_to -> {assoc_name}_id"
  - "Target table inference: naive pluralization {target_lowercase}s"
affects: [100-02-preloading, repo-preload]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "5-field metadata encoding: kind:name:target:fk:target_table"
    - "FK inference by relationship kind (owner vs assoc name)"

key-files:
  created: []
  modified:
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/meshc/tests/e2e.rs"

key-decisions:
  - "FK convention: has_many/has_one uses {owner_lowercase}_id, belongs_to uses {assoc_name}_id"
  - "Target table: naive pluralization {target_lowercase}s (consistent with existing __table__ convention)"
  - "New __relationship_meta__ added alongside existing __relationships__ (no breaking changes)"

patterns-established:
  - "5-field relationship metadata encoding for preloader consumption"

# Metrics
duration: 3min
completed: 2026-02-16
---

# Phase 100 Plan 01: Relationship Metadata Summary

**__relationship_meta__() schema function with 5-field FK/table encoding for preloader batch queries**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-16T21:17:06Z
- **Completed:** 2026-02-16T21:20:04Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- `__relationship_meta__()` callable on any Schema-derived struct via `User.__relationship_meta__()`
- Returns List<String> with "kind:name:target:fk:target_table" 5-field encoding
- FK inference: has_many/has_one use {owner_lowercase}_id, belongs_to uses {assoc_name}_id
- Target table: naive pluralization {target_lowercase}s
- Existing `__relationships__()` output unchanged (backward compatible)
- 3 new e2e tests covering all relationship kinds and multiple relationships

## Task Commits

Each task was committed atomically:

1. **Task 1: Add __relationship_meta__() to compiler pipeline** - `45638e3d` (feat)
2. **Task 2: Add e2e tests for __relationship_meta__() output** - `52ff646f` (test)

## Files Created/Modified
- `crates/mesh-typeck/src/infer.rs` - Register __relationship_meta__ type signature + field access resolution
- `crates/mesh-codegen/src/mir/lower.rs` - Generate __relationship_meta__ MIR function with 5-field encoding + field access resolution
- `crates/meshc/tests/e2e.rs` - 3 e2e tests: has_many, has_one, multiple relationships

## Decisions Made
- FK convention: has_many/has_one uses {owner_lowercase}_id (e.g., User has_many :posts -> "user_id"), belongs_to uses {assoc_name}_id (e.g., belongs_to :user -> "user_id"). This is consistent with the Elixir/Ecto convention.
- Target table uses naive pluralization (lowercase + "s"), same convention as existing __table__() generation.
- New function added alongside existing __relationships__() -- no modifications to existing 3-field encoding.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- `__relationship_meta__()` provides all metadata needed for Repo.preload batch IN queries
- Plan 100-02 can use this to build preloader that extracts FK column and target table from metadata
- 216 e2e tests pass with zero regressions

## Self-Check: PASSED

All files verified present. All commit hashes verified in git log.

---
*Phase: 100-relationships-preloading*
*Completed: 2026-02-16*
