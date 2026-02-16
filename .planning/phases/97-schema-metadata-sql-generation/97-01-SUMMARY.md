---
phase: 97-schema-metadata-sql-generation
plan: 01
subsystem: compiler
tags: [schema, derive, metadata, field-types, column-accessors, timestamps, sql-types, parser, codegen]

# Dependency graph
requires:
  - phase: 96-04
    provides: "deriving(Schema) generates __table__, __fields__, __primary_key__, __relationships__ metadata functions"
provides:
  - "__field_types__() returns field-to-SQL-type mappings as 'field:SQL_TYPE' encoded strings"
  - "Per-field column accessors: __name_col__(), __id_col__(), etc."
  - "Schema options: table/primary_key/timestamps parsed inside struct bodies"
  - "Custom table names via `table \"custom_name\"`"
  - "Custom primary keys via `primary_key :custom_pk`"
  - "Timestamps injection via `timestamps true` adds inserted_at/updated_at to struct"
  - "mir_type_to_sql_type() maps MirType to PostgreSQL types (BIGINT, TEXT, BOOLEAN, DOUBLE PRECISION)"
affects: [97-02-runtime-sql-generation, 98-query-builder, 99-repo-layer, 100-relationships]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Schema options as contextual identifiers in struct bodies", "SCHEMA_OPTION AST node with option_name/string_value/atom_value/bool_value accessors", "mir_type_to_sql_type() for MIR-to-PostgreSQL type mapping", "Timestamp field injection at MirStructDef level"]

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
  - "Schema options use contextual identifiers (table/primary_key/timestamps) not @ annotations -- follows relationship declaration pattern, no lexer changes"
  - "Column accessors use __field_col__ double-underscore pattern matching existing __table__/__fields__ convention"
  - "Timestamps inject inserted_at/updated_at as String fields into MirStructDef layout, not just metadata"
  - "MirType -> SQL type mapping: Int->BIGINT, Float->DOUBLE PRECISION, Bool->BOOLEAN, String->TEXT, fallback TEXT"
  - "Field type metadata uses colon-separated encoding ('field:SQL_TYPE') consistent with relationship encoding ('kind:name:target')"

patterns-established:
  - "Schema options: contextual identifier + value (string/atom/bool) parsed as SCHEMA_OPTION nodes in struct body"
  - "Field type metadata: generate_schema_metadata accepts custom_table, custom_pk, has_timestamps parameters"
  - "Column accessors: {StructName}____{field}_col__ mangled function names"
  - "Timestamp injection: fields appended to schema_fields before any metadata generation or MirStructDef push"

# Metrics
duration: 34min
completed: 2026-02-16
---

# Phase 97 Plan 01: Enhanced Schema Metadata Summary

**Schema derives produce field-to-SQL-type mappings, per-field column accessors, configurable table/PK via schema options, and timestamps auto-injection with inserted_at/updated_at fields**

## Performance

- **Duration:** 34 min
- **Started:** 2026-02-16T17:34:00Z
- **Completed:** 2026-02-16T18:08:00Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- `__field_types__()` returns List<String> of "field:SQL_TYPE" entries mapping each struct field to its PostgreSQL column type
- Per-field column accessors (`User.__name_col__()`, `User.__id_col__()`) for type-safe column references in query building
- Schema options (`table "people"`, `primary_key :uuid`, `timestamps true`) parsed as contextual identifiers in struct bodies
- `timestamps true` injects `inserted_at` and `updated_at` String fields into struct layout and all metadata functions
- Custom table names and primary keys override Phase 96 defaults while maintaining backward compatibility
- 6 new e2e tests with all 175 tests passing (zero regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Parse schema options and register new Schema metadata types** - `1e7ee815` (feat)
2. **Task 2: Extend MIR codegen for field types, column accessors, schema options, timestamps + e2e tests** - `e8303131` (feat)

## Files Created/Modified
- `crates/mesh-parser/src/syntax_kind.rs` - Added SCHEMA_OPTION composite node kind
- `crates/mesh-parser/src/parser/items.rs` - parse_schema_option() for table/primary_key/timestamps in struct bodies
- `crates/mesh-parser/src/ast/item.rs` - SchemaOption AST wrapper, StructDef.schema_options() accessor
- `crates/mesh-typeck/src/infer.rs` - Register __field_types__ and __*_col__ function types, extend field access resolution
- `crates/mesh-codegen/src/mir/lower.rs` - mir_type_to_sql_type(), extended generate_schema_metadata with 3 new params, column accessor generation
- `crates/meshc/tests/e2e.rs` - 6 e2e tests for field types, column accessors, custom table, custom PK, timestamps, defaults

## Decisions Made
- **Contextual identifiers for schema options:** Used same parsing pattern as relationship declarations (belongs_to/has_many/has_one) rather than `@table` annotation syntax. Zero lexer changes required.
- **Double-underscore column accessor naming:** `User.__name_col__()` matches existing `__table__()`, `__fields__()` pattern and avoids collisions with user methods.
- **Timestamp fields in struct layout:** `timestamps true` adds physical fields to MirStructDef (not just metadata) so from_row can populate them when querying.
- **SQL type mapping defaults to TEXT:** Unknown/unmapped MirType variants fall back to TEXT, which is safe for PostgreSQL.
- **Colon-separated string encoding:** `__field_types__()` returns "field:SQL_TYPE" strings, consistent with `__relationships__()` returning "kind:name:target" strings. Avoids complex map MIR.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All enhanced Schema metadata functions callable via StructName.__method__() syntax
- Field type mappings ready for SQL generation in Plan 97-02 (runtime SQL builders)
- Column accessors ready for Phase 98 Query Builder type-safe column references
- Schema options (custom table name, primary key) available for runtime SQL generation
- Timestamp injection works end-to-end through full compiler pipeline

## Self-Check: PASSED

- All 6 modified files verified present
- Both task commits (1e7ee815, e8303131) verified in git log
- 175 e2e tests pass (6 new, 169 existing unchanged)
- Full workspace build clean (0 warnings)

---
*Phase: 97-schema-metadata-sql-generation*
*Completed: 2026-02-16*
