# Project Research Summary

**Project:** Mesh ORM Library (v10.0)
**Domain:** ORM library for a statically-typed, LLVM-compiled functional language targeting PostgreSQL
**Researched:** 2026-02-16
**Confidence:** HIGH

## Executive Summary

Building an ORM for Mesh requires a hybrid approach: compiler additions to provide language primitives (atoms, keyword arguments, multi-line pipes, deriving(Schema), struct update syntax) combined with pure Mesh library code for the ORM API (Query builder, Repo, Changeset, Migration). The key architectural insight is that Mesh already has most building blocks needed for an ORM — deriving(Row) for struct-to-row mapping, Pool.query for SQL execution, traits for type-safe dispatch, pipe operator for query composition — but lacks the connective tissue to make these primitives feel like an integrated ORM. The missing pieces are language-level: atoms for field references, keyword arguments for ergonomic DSL syntax, and deriving(Schema) for compile-time metadata generation.

The recommended approach follows Ecto's four-module pattern (Schema, Query, Repo, Changeset) because Mesh's language design maps almost 1:1 to Elixir's functional paradigm. Schema definitions use deriving(Schema) to generate table metadata at compile time. Queries are immutable structs composed via pipe chains, with SQL generation delegated to runtime Rust functions. The Repo module provides stateless database operations that combine Pool.query with schema metadata. Changesets handle validation and type coercion as pure data transformations. Migrations use explicit up/down functions in Mesh files with a forward-only, expand-migrate-contract philosophy.

The critical risks are: (1) attempting to replicate Ecto's macro-based DSL without macros leads to runtime configuration hell — use deriving(Schema) instead; (2) single-line pipe chains make query building unusable — fix the parser first; (3) string-based column references enable SQL injection — generate column accessors at compile time; (4) N+1 queries without lazy loading — implement Ecto-style explicit preloading; (5) PostgreSQL text protocol returns all-strings — build a centralized type coercion layer in schema metadata. Address these via compiler additions in Phase 1 before building library code.

## Key Findings

### Recommended Stack

Mesh ORM is built as three layers: compiler additions (atoms, keyword args, deriving(Schema), struct update, multi-line pipes), runtime SQL generation functions (mesh-rt/db/orm.rs), and pure Mesh library code (Query, Repo, Changeset, Migration modules). No macro system is needed. No new parser grammar for schema blocks is needed. The ORM leverages existing features — struct definitions, deriving infrastructure, pipe operator, traits, Result/Option types, pattern matching — with targeted compiler additions to unlock ergonomic syntax.

**Core technologies:**
- **Atom literals (`:name`, `:email`)**: Compiler addition for field references in queries and schema DSL — enables compile-time validation of field names
- **Keyword arguments (`where(name: "Alice")`)**: Compiler addition desugaring to Map literals — unlocks Ecto-style ergonomic query builder API
- **deriving(Schema)**: Compiler addition generating `__table__()`, `__fields__()`, `__primary_key__()` metadata functions — connects struct definitions to database schema
- **Multi-line pipe chains**: Parser fix to treat `|>` at line start as continuation — makes query builder readable
- **Struct update syntax (`%{user | name: "Bob"}`)**: Parser/codegen addition for functional data updates — needed for changeset application
- **Runtime SQL generation (Rust)**: New mesh-rt/db/orm.rs module with mesh_orm_build_select/insert/update/delete — centralizes parameterized SQL generation
- **Pure Mesh ORM library**: Query, Repo, Changeset, Migration modules using existing language features — no runtime changes to Pool/Pg driver needed

**Critical ordering**: Atoms first (no dependencies), then multi-line pipes (parser fix), then keyword args (depends on atoms), then struct update (independent), then deriving(Schema) (depends on atoms), then relationship declarations (extends Schema). After compiler additions, all ORM library code is pure Mesh.

**Alternatives rejected**: Runtime schema registry (requires mutable globals), macro system (massive scope creep), SQL parser (diminishing returns), multi-database support (prevents PG-specific features), lazy loading (causes N+1 problems), Active Record pattern (incompatible with functional paradigm).

### Expected Features

Research identified table stakes (features users expect in any ORM), differentiators (what sets Mesh ORM apart), and anti-features (explicit non-goals based on ORM ecosystem learnings).

**Must have (table stakes):**
- **Schema DSL**: Struct + deriving(Schema) generates table name, field metadata, primary key, timestamps
- **Query builder**: Pipe-composable where/select/order_by/limit/offset/join/group_by/having — returns immutable Query struct
- **Repo pattern**: Stateless all/one/get/get_by/insert/update/delete operations through central Repo module
- **Relationships**: belongs_to, has_many, has_one declarations with preload support
- **Changesets**: cast/validate pipeline with type coercion, validation functions (required/length/format/inclusion/number), constraint error mapping
- **Migrations**: Timestamped files with up/down functions, schema_migrations tracking, create_table/alter_table/create_index helpers

**Should have (competitive):**
- **Pipe-native query builder**: Designed from ground up for `|>` composition, more natural than Ecto's binding syntax
- **Compile-time field validation**: Catch `:naem` typos at compile time via atom validation against schema fields
- **Actor-integrated transactions**: Crash isolation via catch_unwind (already exists in Pg.transaction)
- **Composable scopes**: Named query fragments as pure functions returning Query structs
- **Upsert support**: PostgreSQL ON CONFLICT clause (heavily used in Mesher)
- **Zero-cost metadata**: All schema metadata baked in at compilation, no runtime reflection overhead

**Defer (v2+):**
- **Compile-time query field validation**: Complex type system work beyond atom checking
- **Schema-diff migration auto-generation**: Requires DB introspection and schema comparison
- **Schemaless changesets**: Validate data without database-backed schema
- **Aggregate functions as first-class ops**: Use raw SQL escape hatch initially
- **Subqueries and CTEs**: Raw SQL escape hatch covers these
- **Streaming/cursor queries**: LIMIT/OFFSET pagination initially

**Anti-features (explicit non-goals):**
- **Lazy loading**: The #1 ORM anti-pattern, causes N+1 queries, requires mutable state
- **Active Record pattern (model.save)**: Incompatible with functional paradigm
- **Identity map/session cache**: Complex, causes stale data issues, nonsensical with immutable data
- **Automatic schema-diff migrations**: Dangerous for production, hides complexity
- **Multi-database support**: Makes every feature 3x harder, prevents PG-specific features
- **Callback hooks (before_save)**: Scatter side effects, hard to test, explicitly rejected by Ecto

### Architecture Approach

The ORM integrates into Mesh's existing three-layer architecture: Layer 1 (Mesh user code — ORM library modules), Layer 2 (Rust compiler — deriving(Schema) code generation), Layer 3 (Rust runtime — SQL generation functions). Zero changes to the existing Pool/Pg driver. The ORM builds SQL strings and parameter lists, then calls Pool.query/Pool.execute unchanged.

**Major components:**
1. **deriving(Schema)** (compiler) — Generates `__table__()`, `__fields__()`, `__primary_key__()` metadata functions from struct definitions following the deriving(Row)/deriving(Json) pattern
2. **Query builder** (Mesh library) — Immutable Query struct with pipe-composable where/order/limit functions, builds query data structure without executing
3. **SQL generator** (runtime Rust) — mesh_orm_build_select/insert/update/delete functions in db/orm.rs, converts Query struct to parameterized SQL + params list
4. **Repo module** (Mesh library) — Stateless functions (all/one/get/insert/update/delete) that combine Query builder + SQL generator + Pool.query
5. **Changeset module** (Mesh library) — Pure data transformations for casting (Map -> typed struct) and validation (pipe-chain of validation functions)
6. **Migration system** (Mesh library + CLI) — Migration files as Mesh functions (up/down), migration runner with schema_migrations tracking, meshc migrate subcommand

**Key patterns**: Struct-as-Query (immutable composition), pool-first-arg convention (all DB functions take PoolHandle first), Result-error propagation (T!String with ? operator), generated metadata functions (deriving pattern). Anti-patterns to avoid: runtime schema reflection, dynamic return types, SQL string building in Mesh, actor/service for query building.

**Data flow for query execution**: User writes `User |> where("email", v) |> Repo.one(pool)` -> Query builder creates Query struct -> Repo.one calls mesh_orm_build_select(query) -> Runtime Rust builds SQL "SELECT * FROM users WHERE email = $1" -> Pool.query executes -> Returns Map<String, String> -> Caller applies User.from_row(map) for typed result.

### Critical Pitfalls

Research identified 20 pitfalls across 3 severity levels. Top 5 critical (cause rewrites or fundamental breakage):

1. **Schema DSL without macros (the Ecto trap)** — Attempting to replicate Ecto's `schema "users" do field :name, :string end` macro-based DSL without macros leads to runtime configuration hell with no compile-time safety. **Prevention**: Use deriving(Schema) as the single entry point, following the established deriving(Json)/deriving(Row) pattern. The struct definition IS the schema. Compiler generates metadata at MIR lowering time.

2. **Query builder type safety (string concatenation trap)** — Implementing the query builder as string concatenation with unchecked column names (`where(query, column, value)` where `column` is arbitrary string) provides zero type safety and enables SQL injection through column name injection. **Prevention**: Generate column accessor functions per schema field. When deriving(Schema) processes `name :: String`, generate `User.name_col() -> String`. Query builder accepts these accessor return values, not arbitrary strings.

3. **N+1 problem without lazy loading (preload design failure)** — The natural code pattern (load users, iterate and query each user's posts) executes N+1 queries. Mesh has no lazy loading (correct decision — prevents invisible N+1), but must provide ergonomic explicit preloading. **Prevention**: Implement Ecto-style preloading with separate queries. `Repo.preload(users, ["posts"])` collects all user IDs, executes single `SELECT * FROM posts WHERE user_id IN (...)`, maps results to parents in memory. Turns N+1 into 2 queries.

4. **deriving(Row) all-strings problem infects the entire ORM** — PostgreSQL text protocol returns all values as strings. The ORM must maintain consistent type coercion layer between Mesh types and PostgreSQL text representations at every boundary: insert (Mesh -> SQL params), select (SQL result -> Mesh struct), where clause values, join conditions. **Prevention**: Centralize type coercion in schema metadata. Each field's metadata includes to_param (Mesh value -> SQL string) and from_column (SQL string -> Mesh value) functions generated by deriving(Schema). Changeset casting validates and coerces in one step before persistence.

5. **Single-line pipe chains make query builder unusable** — The ORM's showcase feature (pipe-chain query building) produces 100+ character single lines because Mesh parser treats newline after `User` as statement terminator and `|>` at line start as syntax error. **Prevention**: Add multi-line pipe continuation to parser (if line ends with `|>` or starts with `|>`, treat as continuation). If parser not extended, verify parenthesized workaround works and document as standard pattern.

**Other critical pitfalls**: Migration rollback data loss (use forward-only + expand-migrate-contract pattern), single-expression case arms break ORM Result handling (add multi-expression case arms or use ? operator aggressively), Map.collect integer key assumption breaks preload grouping (fix collect codegen key type propagation).

## Implications for Roadmap

Based on research, the implementation must follow strict dependency order: compiler additions first (enable ergonomic syntax), then schema metadata (foundation for everything), then query builder + Repo (core API), then changesets (validation layer), then relationships (most complex feature), then migrations (operational tooling), finally Mesher rewrite (validation).

### Phase 1: Compiler Additions (Language Primitives)
**Rationale:** Every subsequent phase depends on these language features. Attempting to build the ORM without atoms, keyword args, multi-line pipes, and deriving(Schema) leads to verbose, unsafe, unergonomic code. These are prerequisites, not nice-to-haves.

**Delivers:**
- Atom literal syntax (`:name`) with lexer/parser/typeck/codegen support
- Keyword argument syntax desugaring to Map literals
- Multi-line pipe chain support (parser fix)
- Struct update syntax (`%{user | name: "Bob"}`)
- deriving(Schema) infrastructure (typeck registration, MIR generation)
- Relationship declaration syntax (belongs_to, has_many, has_one keywords)

**Addresses:**
- Features: Ergonomic schema DSL, pipe-composable query builder syntax
- Pitfalls: #1 (schema DSL without macros), #2 (string column names), #7 (single-expression case arms), #8 (single-line pipes), #12 (struct update), #15 (relationship declaration syntax)

**Estimated scope:** ~2500 LOC across compiler crates (lexer, parser, typeck, codegen)

### Phase 2: Schema Metadata + SQL Generation
**Rationale:** Schema metadata is the foundation. Query builder, Repo, Changeset, and Migrations all depend on knowing table names, field names, field types, and primary keys at compile time.

**Delivers:**
- deriving(Schema) generates `__table__()`, `__fields__()`, `__primary_key__()` functions
- Runtime SQL generation module (mesh-rt/db/orm.rs)
- mesh_orm_build_select/insert/update/delete functions
- Parameterized SQL generation with proper $1, $2 placeholders
- Identifier quoting and type casting

**Uses:**
- Stack: Atom literals for field references, deriving infrastructure from Phase 1
- Architecture: Generated metadata functions pattern, runtime SQL generation layer

**Implements:** Schema component, SQL generator component

**Avoids:** Pitfall #1 (runtime schema registration), #4 (type coercion gaps)

### Phase 3: Query Builder + Repo (Core API)
**Rationale:** These are the user-facing ORM API. Developers write queries and execute them through Repo. Must work before adding complexity like changesets or relationships.

**Delivers:**
- Query struct with where/select/order_by/limit/offset/join/group_by
- Pipe-composable query builder functions
- Repo.all/one/get/get_by/insert_raw/update_raw/delete/count/exists
- Integration with Pool.query via SQL generator
- Raw SQL escape hatch (fragment function)

**Addresses:**
- Features: Query builder (table stakes), Repo pattern (table stakes), composable scopes (differentiator)
- Pitfalls: #8 (multi-line pipes now available), #13 (expression problem — fragment escape hatch)

**Implements:** Query builder component, Repo module component

### Phase 4: Changesets (Validation Layer)
**Rationale:** Changesets enhance Repo operations with type-safe validation. Not needed for basic querying, so comes after core Repo is working. Builds on schema metadata for type coercion.

**Delivers:**
- Changeset struct (data, changes, errors, valid)
- Changeset.cast for type coercion from Map params
- Validation functions: required/length/format/inclusion/number
- Constraint error mapping (PG unique/FK violations -> changeset errors)
- Repo.insert/update accept Changeset structs

**Addresses:**
- Features: Changesets (table stakes), type-safe casting (differentiator)
- Pitfalls: #4 (centralized type coercion), #12 (struct update for error accumulation)

**Implements:** Changeset module component

### Phase 5: Relationships + Preloading
**Rationale:** Most complex ORM feature. Requires working queries, schema metadata, and Repo operations. Relationships are cross-schema (User -> Post requires both schemas known). Preloading requires correct string-keyed map grouping.

**Delivers:**
- belongs_to/has_many/has_one metadata declarations
- Relationship metadata generation in deriving(Schema)
- Repo.preload_assoc for single association loading
- Repo.preload for multiple associations (separate queries, not JOINs)
- Nested preloading support (posts.comments)
- many_to_many through join table support

**Addresses:**
- Features: Relationships (table stakes), preloading (table stakes)
- Pitfalls: #3 (N+1 problem — explicit preloading prevents it), #11 (relationship metadata without reflection), #14 (Map.collect string keys for grouping)

**Implements:** Relationship definitions, preload component

### Phase 6: Migration System
**Rationale:** Migrations are operationally important but not required for query/data operations. Can be developed in parallel with Phase 5 if resources allow. Uses schema metadata for future auto-generation but starts with manual migration files.

**Delivers:**
- Migration file format (Mesh functions with up/down)
- Migration DSL (create_table, alter_table, drop_table, create_index)
- Migration tracking (_mesh_migrations table)
- Migration runner (discover, sort, apply pending, rollback)
- meshc migrate CLI subcommand
- Migration generation scaffold (meshc migrate generate name)

**Addresses:**
- Features: Migrations (table stakes)
- Pitfalls: #5 (multi-line SQL strings — use plain SQL files or multi-line string literals), #6 (rollback data loss — forward-only + expand-migrate-contract)

**Implements:** Migration module component, CLI integration

### Phase 7: Mesher Rewrite (Validation)
**Rationale:** Rewriting Mesher's entire storage layer validates every ORM feature against a real application. This is the dogfooding phase that exposes API usability issues and missing features.

**Delivers:**
- 11 type structs converted to deriving(Schema)
- 627 lines of storage/queries.mpl replaced with ORM calls
- 82 lines of storage/schema.mpl replaced with migration files
- All service modules using Repo instead of raw Pool.query
- All API handlers using typed structs instead of raw Maps
- Verification that all existing functionality works identically

**Estimated impact:** 627 lines -> ~100-150 lines of ORM calls. More maintainable, type-safe, less SQL duplication.

**Validates:** All features work together in production-like environment. Real-world query patterns (filtering, pagination, aggregation, preloading) are supported.

### Phase Ordering Rationale

- **Phase 1 first (compiler)**: Language primitives are prerequisites. Building ORM without them produces verbose, unsafe code that needs rewriting when primitives are added later.
- **Phase 2 second (schema)**: Everything depends on schema metadata. Query builder needs table/field names. Repo needs field types. Changesets need type coercion rules.
- **Phase 3 third (query + repo)**: Core user-facing API. Must work before adding complexity. Changesets and relationships are enhancements.
- **Phase 4 fourth (changesets)**: Enhances Repo operations. Independent of relationships. Can be developed in parallel with Phase 5.
- **Phase 5 fifth (relationships)**: Most complex feature. Depends on everything else working. Preloading requires correct query builder, schema metadata, and Map grouping.
- **Phase 6 parallel (migrations)**: Partially independent. Can start after Phase 2 (schema metadata) and develop in parallel with Phases 3-5.
- **Phase 7 final (validation)**: Requires all features complete. Real-world testing phase.

**Dependency chain**: Phase 1 -> Phase 2 -> (Phase 3, Phase 6) -> (Phase 4, Phase 5) -> Phase 7. Phases in parentheses can run in parallel.

### Research Flags

**Phases needing deeper research during planning:**
- **Phase 1 (compiler additions)**: Known parser/typeck patterns but need careful design for keyword args desugaring, atom type representation, deriving(Schema) metadata generation strategy
- **Phase 5 (relationships + preloading)**: Complex cross-schema metadata, nested preloading with recursive planning, many-to-many through tables

**Phases with standard patterns (skip research-phase):**
- **Phase 2 (schema metadata)**: Follows existing deriving(Row)/deriving(Json) pattern exactly
- **Phase 3 (query builder + repo)**: Well-documented Ecto patterns, straightforward Rust SQL generation
- **Phase 4 (changesets)**: Pure data transformation, established validation patterns
- **Phase 6 (migrations)**: Standard migration runner pattern (Rails, Ecto, Diesel all use same approach)
- **Phase 7 (Mesher rewrite)**: No research needed, direct code translation

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Compiler source directly analyzed, existing deriving infrastructure validated, runtime Rust patterns proven in current codebase |
| Features | HIGH | Ecto/Diesel/ActiveRecord/Prisma thoroughly documented, Mesher pain points clearly identified (627 LOC queries.mpl) |
| Architecture | HIGH | Architecture derived from direct codebase analysis of all compiler crates, runtime DB layer, Mesher application code |
| Pitfalls | HIGH | 20 pitfalls identified from Mesh compiler limitations (single-line pipes, cross-module from_row), ORM ecosystem learnings (N+1, lazy loading, migration rollback), and Mesher development experience |

**Overall confidence:** HIGH

Research is based on: (1) Direct Mesh compiler source analysis (~99K LOC across crates), (2) Mesher application analysis (627 LOC queries demonstrating pain points), (3) Established ORM patterns (Ecto, Diesel, ActiveRecord with extensive documentation), (4) Known Mesh language limitations documented in STATE.md and PROJECT.md.

### Gaps to Address

**Compiler additions feasibility**: All proposed additions (atoms, keyword args, multi-line pipes, deriving(Schema), struct update) follow existing patterns in the compiler, but implementation complexity needs validation during Phase 1. Keyword arguments have most uncertainty — desugaring to Map vs special calling convention.

**Cross-module from_row resolution**: Known issue from Mesher Phase 88 that MUST be fixed in Phase 1. If not fixed, ORM is non-functional across module boundaries (which is the default case).

**Map.collect string key assumption**: Known issue from PROJECT.md that breaks preload grouping. Must be fixed in Phase 1 or worked around with manual Map.put loops.

**Relationship metadata propagation**: No runtime reflection means relationship metadata must flow through function calls or be looked up via generated functions. Design approach needs finalization in Phase 2.

**Mesher UUID vs Integer PKs**: Mesher uses UUID primary keys (`gen_random_uuid()`). ORM must support both UUID and Integer PKs based on schema field type. Default to UUID to match Mesher pattern.

**PostgreSQL text protocol coercion edge cases**: Boolean (`t`/`f`), NULL (missing Map key), JSONB (JSON string requiring from_json), timestamps (string without DateTime type). Coercion layer must handle all cases. Test coverage critical.

## Sources

### Primary (HIGH confidence)

**Mesh compiler/runtime source analysis:**
- Direct codebase analysis of crates/mesh-lexer, mesh-parser, mesh-typeck, mesh-codegen, mesh-rt
- crates/mesh-typeck/src/infer.rs:2276 — valid_derives array, deriving infrastructure entry point
- crates/mesh-codegen/src/mir/lower.rs:1689-1746 — lower_struct_def, deriving code generation patterns
- crates/mesh-rt/src/db/pool.rs, pg.rs, row.rs — existing database layer
- mesher/storage/queries.mpl (627 lines) — pain points the ORM eliminates
- mesher/types/*.mpl — all structs use deriving(Row), deriving(Json)

**Mesh language limitations:**
- .planning/PROJECT.md:235-236 — single-line pipes, Map.collect integer key assumption
- .planning/STATE.md:44-48 — known blockers for ORM development
- .planning/phases/88-02-SUMMARY.md:142-143 — cross-module from_json resolution failure
- .planning/phases/87.1-RESEARCH.md:246-248 — Row all-String fields limitation

**Ecto (primary reference for architecture):**
- [Ecto.Schema v3.13.5](https://hexdocs.pm/ecto/Ecto.Schema.html) — macro-based schema DSL, metadata generation
- [Ecto.Query v3.13.5](https://hexdocs.pm/ecto/Ecto.Query.html) — query builder API, composability, type safety
- [Ecto.Changeset v3.13.5](https://hexdocs.pm/ecto/Ecto.Changeset.html) — validation pipeline, casting, constraint error mapping
- [Ecto.Repo v3.13.5](https://hexdocs.pm/ecto/Ecto.Repo.html) — repository pattern, preloading strategy
- [Ecto Associations](https://hexdocs.pm/ecto/associations.html) — relationship metadata, belongs_to/has_many

### Secondary (MEDIUM confidence)

**Rust ORMs:**
- [Diesel ORM](https://diesel.rs/) — compile-time query validation, type-safe query builder
- [SeaORM](https://www.sea-ql.org/SeaORM/) — derive-based schema, async support
- [Rust ORMs Guide (Shuttle)](https://www.shuttle.dev/blog/2024/01/16/best-orm-rust) — Diesel vs SeaORM vs SQLx comparison

**ORM patterns and anti-patterns:**
- [The Basic Mistake All ORMs Make (Vogten)](https://martijnvogten.github.io/2025/04/16/the-basic-mistake-all-orms-make-and-how-to-fix-it.html) — lazy loading critique
- [ORM Lazy Loading Anti-Pattern](https://www.mehdi-khalili.com/orm-anti-patterns-part-3-lazy-loading) — N+1 problem
- [ORM Framework Anti-Patterns (Lindbakk)](https://lindbakk.com/blog/orm-frameworks-anti-patterns) — identity map, callbacks
- [SQL injection in ORMs](https://snyk.io/blog/sql-injection-orm-vulnerabilities/) — column name injection risks

**Migration patterns:**
- [Ecto.Migration documentation](https://hexdocs.pm/ecto_sql/Ecto.Migration.html) — migration file format, schema_migrations tracking
- [Atlas: Database rollback hard truths](https://atlasgo.io/blog/2024/11/14/the-hard-truth-about-gitops-and-db-rollbacks) — forward-only migration philosophy
- [Database migrations: safe strategies](https://vadimkravcenko.com/shorts/database-migrations/) — expand-migrate-contract pattern

**Other ORMs:**
- [ActiveRecord Query Interface](https://guides.rubyonrails.org/active_record_querying.html) — established patterns, callbacks
- [Prisma ORM](https://www.prisma.io/orm) — schema-first approach, type generation
- [SQLAlchemy Session Basics](https://docs.sqlalchemy.org/en/20/orm/session_basics.html) — unit of work, identity map

### Tertiary (LOW confidence, needs validation)

- [Elixir School: Ecto Associations](https://elixirschool.com/en/lessons/ecto/associations) — relationship implementation patterns
- [Composing Ecto Queries (AmberBit)](https://www.amberbit.com/blog/2019/4/16/composing-ecto-queries-filters-and-preloads/) — query composition patterns
- [Drizzle vs Prisma 2026](https://makerkit.dev/blog/tutorials/drizzle-vs-prisma) — TypeScript ORM comparison

---
*Research completed: 2026-02-16*
*Ready for roadmap: yes*
