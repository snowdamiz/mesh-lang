# Requirements: ORM

**Defined:** 2026-02-16
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.

## v10.0 Requirements

Requirements for ORM library targeting PostgreSQL. Each maps to roadmap phases.

### Compiler Additions

- [ ] **COMP-01**: Atom literal syntax (`:name`, `:email`) as compile-time string constants with lexer/parser/typeck/codegen support
- [ ] **COMP-02**: Keyword argument syntax (`where(name: "Alice")`) desugaring to Map literals
- [ ] **COMP-03**: Multi-line pipe chain support (`|>` at line start treated as continuation)
- [ ] **COMP-04**: Struct update syntax (`%{user | name: "Bob"}`) for immutable field updates
- [ ] **COMP-05**: deriving(Schema) infrastructure generating `__table__()`, `__fields__()`, `__primary_key__()` metadata functions
- [ ] **COMP-06**: Relationship declaration syntax (belongs_to, has_many, has_one) as schema metadata
- [ ] **COMP-07**: Fix Map.collect string key propagation (currently assumes integer keys)
- [ ] **COMP-08**: Fix cross-module from_row/from_json resolution

### Schema Definition

- [ ] **SCHM-01**: Struct + deriving(Schema) generates table name from struct name (pluralized, lowercased)
- [ ] **SCHM-02**: Field metadata includes column name, Mesh type, and SQL type mapping
- [ ] **SCHM-03**: Primary key configuration (default: `id` UUID, configurable per schema)
- [ ] **SCHM-04**: Timestamps support (inserted_at, updated_at) via schema option
- [ ] **SCHM-05**: Column accessor functions generated per field for type-safe query building

### Query Builder

- [ ] **QBLD-01**: Immutable Query struct with pipe-composable builder functions
- [ ] **QBLD-02**: `where` clause with field-value conditions and operators (=, !=, <, >, <=, >=, IN, LIKE, IS NULL)
- [ ] **QBLD-03**: `select` to specify columns (default: all fields from schema)
- [ ] **QBLD-04**: `order_by` with ascending/descending direction
- [ ] **QBLD-05**: `limit` and `offset` for pagination
- [ ] **QBLD-06**: `join` for cross-table queries (inner, left, right)
- [ ] **QBLD-07**: `group_by` and `having` for aggregation queries
- [ ] **QBLD-08**: `fragment` for raw SQL escape hatch with parameterized values
- [ ] **QBLD-09**: Composable scopes as pure functions returning Query structs

### Repo Operations

- [ ] **REPO-01**: `Repo.all(pool, query)` returns `List<T>` of all matching rows
- [ ] **REPO-02**: `Repo.one(pool, query)` returns `Option<T>` of first matching row
- [ ] **REPO-03**: `Repo.get(pool, Schema, id)` returns `Option<T>` by primary key
- [ ] **REPO-04**: `Repo.get_by(pool, Schema, conditions)` returns `Option<T>` by field conditions
- [ ] **REPO-05**: `Repo.insert(pool, changeset)` inserts validated changeset and returns `Result<T, Changeset>`
- [ ] **REPO-06**: `Repo.update(pool, changeset)` updates validated changeset and returns `Result<T, Changeset>`
- [ ] **REPO-07**: `Repo.delete(pool, struct)` deletes record and returns `Result<T, String>`
- [ ] **REPO-08**: `Repo.count(pool, query)` returns integer count of matching rows
- [ ] **REPO-09**: `Repo.exists(pool, query)` returns boolean existence check
- [ ] **REPO-10**: `Repo.preload(pool, structs, associations)` loads associated records via separate queries
- [ ] **REPO-11**: `Repo.transaction(pool, fn)` wraps operations in database transaction with automatic rollback

### Changesets

- [ ] **CHST-01**: Changeset struct with data, changes, errors, and valid fields
- [ ] **CHST-02**: `Changeset.cast(struct, params, allowed_fields)` for type coercion from Map params
- [ ] **CHST-03**: `validate_required(changeset, fields)` ensures fields are present and non-empty
- [ ] **CHST-04**: `validate_length(changeset, field, opts)` validates string/list length with min/max/is
- [ ] **CHST-05**: `validate_format(changeset, field, pattern)` validates string against pattern
- [ ] **CHST-06**: `validate_inclusion(changeset, field, values)` validates field value in allowed list
- [ ] **CHST-07**: `validate_number(changeset, field, opts)` validates numeric bounds
- [ ] **CHST-08**: Constraint error mapping (PostgreSQL unique/FK violations -> changeset errors)
- [ ] **CHST-09**: Pipe-chain validation: `changeset |> validate_required([:name]) |> validate_length(:name, min: 1)`

### Migrations

- [ ] **MIGR-01**: Migration file format as Mesh functions with `up` and `down` definitions
- [ ] **MIGR-02**: Migration DSL: `create_table`, `alter_table`, `drop_table` with column definitions
- [ ] **MIGR-03**: Migration DSL: `create_index`, `drop_index` for index management
- [ ] **MIGR-04**: Migration tracking via `_mesh_migrations` table with version and timestamp
- [ ] **MIGR-05**: Migration runner: discover, sort, apply pending, rollback last
- [ ] **MIGR-06**: `meshc migrate` CLI subcommand (up, down, status)
- [ ] **MIGR-07**: `meshc migrate generate <name>` scaffold generation with timestamp prefix
- [ ] **MIGR-08**: Forward-only philosophy with expand-migrate-contract pattern documented

### Mesher Rewrite

- [ ] **MSHR-01**: All 11 Mesher type structs converted to deriving(Schema)
- [ ] **MSHR-02**: storage/queries.mpl (627 lines) replaced with ORM Repo calls
- [ ] **MSHR-03**: storage/schema.mpl (82 lines) replaced with migration files
- [ ] **MSHR-04**: All service modules using Repo instead of raw Pool.query
- [ ] **MSHR-05**: All existing Mesher functionality verified working identically after rewrite

## v2+ Requirements

Deferred to future milestone. Tracked but not in current roadmap.

### Compile-Time Query Validation

- **DEFER-01**: Validate query field references against schema at compile time (beyond atom checking)

### Schema-Diff Migrations

- **DEFER-02**: Auto-generate migration files by comparing current schema to database state

### Schemaless Changesets

- **DEFER-03**: Validate arbitrary Map data without database-backed schema

### Advanced Query Features

- **DEFER-04**: Aggregate functions as first-class operations (count, sum, avg, min, max in select)
- **DEFER-05**: Subquery and CTE support in query builder
- **DEFER-06**: Streaming/cursor queries for large result sets

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Lazy loading | #1 ORM anti-pattern. Causes N+1 queries, requires mutable state, incompatible with functional paradigm. Use explicit preloading. |
| Active Record pattern (model.save) | Incompatible with functional paradigm. Repo pattern provides clear separation of data and operations. |
| Identity map / session cache | Complex, causes stale data issues, nonsensical with immutable data. |
| Automatic schema-diff migrations | Dangerous for production, hides complexity. Manual migrations with expand-migrate-contract. |
| Multi-database support | Makes every feature 3x harder, prevents PostgreSQL-specific optimizations. PG only. |
| Callback hooks (before_save, after_create) | Scatter side effects, hard to test. Explicitly rejected by Ecto for good reasons. |
| MySQL / SQLite ORM support | PostgreSQL-only target. SQLite driver exists but ORM targets PG exclusively. |
| GraphQL integration | REST + raw SQL escape hatch covers all query patterns. |
| Connection-per-request model | Pool with checkout/checkin already works. No need for request-scoped connections. |
| Runtime schema reflection | All metadata baked in at compile time via deriving(Schema). No runtime reflection. |
| Macro system for DSL | deriving(Schema) provides compile-time codegen. Full macro system is massive scope creep. |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| COMP-01 | Phase 96 | Pending |
| COMP-02 | Phase 96 | Pending |
| COMP-03 | Phase 96 | Pending |
| COMP-04 | Phase 96 | Pending |
| COMP-05 | Phase 96 | Pending |
| COMP-06 | Phase 96, Phase 100 | Pending |
| COMP-07 | Phase 96 | Pending |
| COMP-08 | Phase 96 | Pending |
| SCHM-01 | Phase 97 | Pending |
| SCHM-02 | Phase 97 | Pending |
| SCHM-03 | Phase 97 | Pending |
| SCHM-04 | Phase 97 | Pending |
| SCHM-05 | Phase 97 | Pending |
| QBLD-01 | Phase 98 | Pending |
| QBLD-02 | Phase 98 | Pending |
| QBLD-03 | Phase 98 | Pending |
| QBLD-04 | Phase 98 | Pending |
| QBLD-05 | Phase 98 | Pending |
| QBLD-06 | Phase 98 | Pending |
| QBLD-07 | Phase 98 | Pending |
| QBLD-08 | Phase 98 | Pending |
| QBLD-09 | Phase 98 | Pending |
| REPO-01 | Phase 98 | Pending |
| REPO-02 | Phase 98 | Pending |
| REPO-03 | Phase 98 | Pending |
| REPO-04 | Phase 98 | Pending |
| REPO-05 | Phase 98 | Pending |
| REPO-06 | Phase 98 | Pending |
| REPO-07 | Phase 98 | Pending |
| REPO-08 | Phase 98 | Pending |
| REPO-09 | Phase 98 | Pending |
| REPO-10 | Phase 100 | Pending |
| REPO-11 | Phase 98 | Pending |
| CHST-01 | Phase 99 | Pending |
| CHST-02 | Phase 99 | Pending |
| CHST-03 | Phase 99 | Pending |
| CHST-04 | Phase 99 | Pending |
| CHST-05 | Phase 99 | Pending |
| CHST-06 | Phase 99 | Pending |
| CHST-07 | Phase 99 | Pending |
| CHST-08 | Phase 99 | Pending |
| CHST-09 | Phase 99 | Pending |
| MIGR-01 | Phase 101 | Pending |
| MIGR-02 | Phase 101 | Pending |
| MIGR-03 | Phase 101 | Pending |
| MIGR-04 | Phase 101 | Pending |
| MIGR-05 | Phase 101 | Pending |
| MIGR-06 | Phase 101 | Pending |
| MIGR-07 | Phase 101 | Pending |
| MIGR-08 | Phase 101 | Pending |
| MSHR-01 | Phase 102 | Pending |
| MSHR-02 | Phase 102 | Pending |
| MSHR-03 | Phase 102 | Pending |
| MSHR-04 | Phase 102 | Pending |
| MSHR-05 | Phase 102 | Pending |

**Coverage:**
- v10.0 requirements: 50 total
- Mapped to phases: 50
- Unmapped: 0

---
*Requirements defined: 2026-02-16*
*Last updated: 2026-02-16 -- Traceability populated after roadmap creation*
