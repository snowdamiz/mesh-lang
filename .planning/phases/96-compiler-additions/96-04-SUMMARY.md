---
phase: 96-compiler-additions
plan: 04
subsystem: compiler
tags: [schema, derive, orm, metadata, relationships, belongs_to, has_many, has_one]

# Dependency graph
requires:
  - phase: 96-03
    provides: "Struct update expression and struct field parsing"
provides:
  - "deriving(Schema) generates __table__, __fields__, __primary_key__, __relationships__ metadata functions"
  - "belongs_to, has_many, has_one relationship declarations parsed inside struct bodies"
  - "Schema metadata callable via StructName.__table__() static method syntax"
affects: [97-schema-and-changesets, 98-query-builder, 99-repo-layer, 100-relationships]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Schema derive generates synthetic MIR functions", "Relationship declarations as contextual identifiers", "Colon-separated string encoding for relationship metadata"]

key-files:
  created: []
  modified:
    - crates/mesh-parser/src/syntax_kind.rs
    - crates/mesh-parser/src/parser/items.rs
    - crates/mesh-parser/src/ast/item.rs
    - crates/mesh-typeck/src/infer.rs
    - crates/mesh-codegen/src/mir/lower.rs
    - crates/meshc/tests/e2e.rs

key-decisions:
  - "Naive pluralization (lowercase + s) for table names; Phase 97 handles configurable table names"
  - "Relationship metadata encoded as 'kind:name:target' strings in List<String> to avoid complex map MIR"
  - "Schema metadata functions use StructName.__method__() static syntax, same pattern as from_row/from_json"
  - "Default primary key is always 'id'; Phase 97 adds schema options for override"
  - "Schema derive rejected on sum types with UnsupportedDerive error"

patterns-established:
  - "Schema metadata: generate_schema_metadata() produces four MIR functions following existing derive pattern"
  - "Relationship declarations: contextual identifiers (not keywords) recognized in struct body parsing"
  - "RELATIONSHIP_DECL AST node with kind_text/assoc_name/target_type accessors"

# Metrics
duration: 11min
completed: 2026-02-16
---

# Phase 96 Plan 04: deriving(Schema) and Relationship Declarations Summary

**Schema derive generates __table__/__fields__/__primary_key__/__relationships__ metadata functions; belongs_to/has_many/has_one declarations parsed as relationship metadata in struct bodies**

## Performance

- **Duration:** 11 min
- **Started:** 2026-02-16T09:13:05Z
- **Completed:** 2026-02-16T09:24:55Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- `deriving(Schema)` on structs generates four callable metadata functions through the full compiler pipeline (parser -> typeck -> MIR -> codegen)
- `belongs_to :user, User`, `has_many :posts, Post`, `has_one :profile, Profile` parse as RELATIONSHIP_DECL nodes inside struct bodies
- Schema metadata functions callable via `User.__table__()` static method syntax, consistent with existing `from_row`/`from_json` patterns
- 5 e2e tests verify table name, primary key, fields list, relationship metadata, and mixed derives
- All 166 existing e2e tests pass unchanged (zero regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Parse relationship declarations and register deriving(Schema) in type checker** - `039dbb0a` (feat)
2. **Task 2: Generate Schema metadata functions in MIR and codegen, with e2e tests** - `f9c9274b` (feat)

## Files Created/Modified
- `crates/mesh-parser/src/syntax_kind.rs` - Added RELATIONSHIP_DECL composite node kind
- `crates/mesh-parser/src/parser/items.rs` - Parse belongs_to/has_many/has_one in struct bodies
- `crates/mesh-parser/src/ast/item.rs` - RelationshipDecl AST node, StructDef.relationships() accessor
- `crates/mesh-typeck/src/infer.rs` - Schema in valid_derives, metadata function registration, field access resolution
- `crates/mesh-codegen/src/mir/lower.rs` - generate_schema_metadata() MIR functions, field access lowering
- `crates/meshc/tests/e2e.rs` - 5 e2e tests for Schema derive functionality

## Decisions Made
- **Naive pluralization:** `User -> "users"`, `Post -> "posts"` via simple lowercase + "s". Proper/configurable table names deferred to Phase 97.
- **String-encoded relationships:** Relationships stored as `"kind:name:target"` strings in `List<String>` rather than complex map structures. Avoids MapLit MIR complexity while remaining parseable at runtime.
- **Default primary key:** Always `"id"` regardless of whether struct has an `id` field. Override via schema options in Phase 97.
- **Sum type rejection:** `deriving(Schema)` on sum types emits UnsupportedDerive error since Schema semantics (table name, fields) only apply to structs.
- **Mangled function names:** `{StructName}____table__` (double underscore separator between struct name and method name with leading underscores).

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Schema metadata infrastructure complete; `__table__()`, `__fields__()`, `__primary_key__()`, `__relationships__()` all callable
- Ready for Phase 96-05 (remaining compiler additions) and Phase 97 (schema layer with configurable options)
- Query builder (Phase 98) can use `StructName.__table__()` for table references and `StructName.__fields__()` for column lists

## Self-Check: PASSED

- All 7 modified/created files verified present
- Both task commits (039dbb0a, f9c9274b) verified in git log
- 166 e2e tests pass (5 new, 161 existing unchanged)
- Full workspace build clean (0 warnings)

---
*Phase: 96-compiler-additions*
*Completed: 2026-02-16*
