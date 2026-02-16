# Feature Landscape: Mesh ORM

**Domain:** ORM library for a statically-typed, LLVM-compiled functional language (Mesh) targeting PostgreSQL
**Researched:** 2026-02-16
**Confidence:** HIGH for core ORM features (Ecto, ActiveRecord, Prisma, Diesel, SeaORM thoroughly documented). MEDIUM for Mesh-specific DSL design (depends on compiler additions). HIGH for anti-patterns (extensive post-mortems across ecosystems).

---

## Existing System Baseline

What Mesh already provides that the ORM builds upon:

- **PostgreSQL driver:** Pure wire protocol, SCRAM-SHA-256 auth, TLS, connection pooling, transactions with panic-safe rollback
- **`deriving(Row)`:** Generates `from_row` mapping `Map<String, String>` to typed structs (String, Int, Float, Bool, Option)
- **`deriving(Json)`:** Automatic JSON encode/decode for structs, sum types, nested types, Option, List, Map
- **`Pool.query` / `Pool.execute`:** Parameterized queries with `$1` placeholders, returns `List<Map<String, String>>`
- **`Pool.query_as`:** One-step query + struct hydration via `from_row`
- **`Pg.transaction`:** Panic-safe transactions with automatic commit/rollback via catch_unwind
- **Pipe operator:** `value |> fn(args)` with pipe-aware type inference
- **Traits with associated types:** Monomorphization-based static dispatch
- **Pattern matching:** Exhaustive, with sum types, structs, literals, wildcards, guards
- **Module system:** File-based with `pub` visibility, qualified imports, cross-module type checking
- **Iterators:** Lazy pipeline composition (map, filter, take, skip), Collect into List/Map/Set/String

### What Mesh Does NOT Have (Relevant Gaps for ORM)

| Gap | Impact on ORM | Mitigation |
|-----|--------------|------------|
| **No keyword arguments** | Cannot write `where(name: "Alice")` -- the most natural ORM syntax | PROJECT.md explicitly lists keyword args as potential compiler addition. **Highest-leverage compiler change.** |
| **Single-line pipe chains only** | `User |> where(...) |> limit(10) |> Repo.all()` must be on one line | Parser change to support multi-line `|>` continuation. Known limitation in STATE.md. |
| **No macros** | Schema DSL must be `deriving` variants or new parser syntax, not user-definable macros | Use `deriving(Schema)` or new dedicated syntax (like `schema` block) |
| **No runtime reflection** | Struct field names/types not queryable at runtime | Compile-time code generation must produce metadata functions |
| **No atom type** | Cannot use `:name` to reference fields symbolically | Add atom literals to the language, or use strings as field references |
| **No default function arguments** | Every argument must be provided explicitly | Multi-clause functions with pattern matching as workaround |
| **No method overloading** | Cannot have `where(field, value)` and `where(map)` at same arity | Multi-clause with pattern matching distinguishes cases |
| **No struct update syntax** | Cannot write `%{user | name: "Bob"}` to produce a modified copy | Needed for changeset `apply_changes`. Compiler addition. |

### What Mesher Currently Does (the code the ORM must replace)

Analysis of `/Users/sn0w/Documents/dev/snow/mesher/storage/queries.mpl` (627 lines) reveals the pain points:

1. **Manual struct construction from Map:** Every query function manually maps `Map.get(row, "column")` to struct fields. Example: `Organization { id: Map.get(row, "id"), name: Map.get(row, "name"), ... }` -- 6+ fields per struct, repeated for every query function.
2. **Raw SQL strings everywhere:** 40+ raw SQL query strings, each hand-written with `$1` placeholders. No reuse of WHERE clauses or common patterns.
3. **Manual type casting:** `parse_event_count(Map.get(row, "event_count"))` -- converting `String.to_int` manually because `Pool.query` returns all strings.
4. **No validation before persistence:** Data goes straight from API to `Pool.execute` with no changeset/validation layer.
5. **Schema DDL as imperative code:** `create_schema` function runs 25+ `Pool.execute` calls for CREATE TABLE / CREATE INDEX. No migration versioning.
6. **Duplicated query patterns:** `if List.length(rows) > 0 do Ok(List.head(rows)) else Err("not found") end` repeated in 15+ functions.

The ORM must eliminate all six of these pain points.

---

## Table Stakes

Features every ORM user expects. Missing any of these makes the ORM feel incomplete.

### 1. Schema DSL for Model Definition

The foundation. Users define their data model in code; the ORM uses it for queries, validation, and migrations.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Schema block defining table name + fields | Every ORM has this. Without it, there is no ORM. | **High** | Requires compiler work: new syntax or deriving macro. Ecto uses `schema "users" do field :name, :string end`. |
| Field types mapping to PG types | Each field has a Mesh type mapped to a PG type (String->TEXT, Int->INTEGER, Float->DOUBLE PRECISION, Bool->BOOLEAN). | **Med** | Start with types `deriving(Row)` already handles. Add DateTime/UUID later. |
| Primary key configuration | Default auto-increment `id :: Int` or configurable UUID. | **Low** | Ecto defaults to `{:id, :id, autogenerate: true}`. Match that convention. |
| `timestamps()` macro/function | Automatic `inserted_at` and `updated_at` fields. Universal across ORMs. | **Low** | Auto-set `inserted_at` on insert, auto-update `updated_at` on update. |
| Virtual fields (not persisted) | Fields for computed values, not stored in DB. Ecto: `field :full_name, :string, virtual: true`. | **Low** | Exclude from INSERT/UPDATE/SELECT generation. |
| Schema metadata generation | Query builder and Repo need table name, field names, field types, PK at compile time. Ecto generates `__schema__/1`. | **High** | Bridge between schema definition and query builder. Must be compile-time generated. |

**Cross-ORM comparison:**
- **Ecto:** `schema "users" do field :name, :string; has_many :posts, Post end` -- macro-based, generates struct + metadata
- **ActiveRecord:** Schema inferred from database at runtime (no code definition), or `t.string :name` in migrations
- **Prisma:** `model User { name String }` in `.prisma` schema file, generates typed client
- **Diesel:** `table!` macro auto-generated from DB schema by `diesel print-schema`
- **SeaORM:** `#[derive(DeriveEntityModel)]` on Rust struct with attribute annotations

**Recommendation for Mesh:** Follow Ecto. Define schema in Mesh code using a DSL block that generates the struct and metadata. Fits Mesh's `do/end` syntax. Avoids external schema files (Prisma) and database inference (ActiveRecord). Use `deriving(Schema)` or a new `schema` block.

### 2. Query Builder with Pipe Composition

The ORM's primary API. Users construct queries by chaining operations via pipe operator.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `where` clause filtering | Most fundamental query operation. Filter records by field conditions. | **High** | Needs keyword args OR positional: `where(:name, "Alice")`. |
| `select` field selection | Choose columns to return. Default: all schema fields. | **Med** | `User |> select([:name, :email])` using list of field references. |
| `order_by` sorting | Sort by fields with ASC/DESC. | **Low** | `User |> order_by(:name, :asc)`. |
| `limit` and `offset` | Pagination building blocks. | **Low** | `User |> limit(10) |> offset(20)`. |
| `join` for associations | Join related tables. Inner, left joins. | **High** | Complex. Simplified via schema associations: `join(:left, :posts)`. |
| `group_by` and `having` | Aggregation queries. | **Med** | Needed for dashboard/reporting. |
| `preload` for eager loading | Load associated records. Prevents N+1. Critical. | **High** | Generates separate query per association. |
| Query composition | Queries are data structures, composable before execution. | **Med** | `base = User |> where(:active, true); base |> limit(10) |> Repo.all()` |
| Parameterized (injection-safe) | All values parameterized, never interpolated. | **Low** | Already built into Pool.query. |
| Raw SQL escape hatch | Drop to raw SQL when the builder cannot express what you need. | **Low** | Pool.query/Pool.execute already exist. |

**How pipe-based query building works (Ecto model):**

Each query function takes a Query struct as first argument (or a Schema module that auto-converts) and returns a new Query struct. The pipe operator threads the query through:

```
# Pseudocode of how this works in Mesh
User                          # Schema module -> converts to Query<User>
  |> where(:name, "Alice")   # Query<User> -> Query<User> with WHERE clause
  |> where(:active, true)    # Query<User> -> Query<User> with additional WHERE
  |> order_by(:created_at, :desc)  # adds ORDER BY
  |> limit(10)               # adds LIMIT
  |> Repo.all()              # executes: returns List<User>
```

The Query struct accumulates clauses as data. SQL is generated only when a terminal operation (Repo.all, Repo.one, etc.) is called. This is Ecto's greatest design insight and maps perfectly to Mesh's pipe operator.

**Keyword args are the key unlock.** Without them, conditions must use positional args: `where(:name, :eq, "Alice")` or `where("name", "Alice")`. With keyword args: `where(name: "Alice", active: true)`. The ergonomic difference is enormous.

### 3. Repo Pattern for Database Operations

All database operations go through a central Repo module. No model.save() scattered throughout the code.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Repo.all(query)` | Execute query, return all matching records as `List<Schema>`. | **Med** | Build SQL from Query, execute via Pool.query, hydrate via from_row. |
| `Repo.one(query)` | Execute query, return exactly one record or error. | **Low** | Repo.all + assert single result. Return `Option<T>` or `Result<T, String>`. |
| `Repo.get(Schema, id)` | Fetch by primary key. Most common single-record fetch. | **Low** | Sugar for `Schema |> where(:id, id) |> Repo.one()`. |
| `Repo.get_by(Schema, clauses)` | Fetch by arbitrary conditions. | **Low** | Sugar for `Schema |> where(clauses) |> Repo.one()`. |
| `Repo.insert(changeset)` | Insert from changeset. Return inserted record with generated fields. | **Med** | Generate INSERT from changeset changes. RETURNING for id/timestamps. |
| `Repo.update(changeset)` | Update from changeset. Only SET changed fields. | **Med** | Generate UPDATE SET for changed fields only. WHERE id = pk. |
| `Repo.delete(struct)` | Delete by primary key. | **Low** | `DELETE FROM table WHERE id = $1`. |
| `Repo.preload(struct, assocs)` | Load associations on already-fetched structs. | **High** | Post-fetch: given a User, query their posts, attach to struct. |
| `Repo.transaction(fn)` | Atomic multi-operation. Already exists as Pg.transaction. | **Low** | Wrap Repo operations in existing transaction support. |

### 4. Relationships (Associations)

Defining and querying relationships between schemas.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `belongs_to` (many-to-one) | Post belongs_to User. Post has `user_id` FK. | **Med** | Adds FK field to schema. Enables preloading parent. |
| `has_many` (one-to-many) | User has_many Posts. | **Med** | Reverse of belongs_to. `SELECT * FROM posts WHERE user_id = $1`. |
| `has_one` (one-to-one) | User has_one Profile. | **Low** | Variant of has_many returning `Option<Profile>` not list. |
| `many_to_many` (join table) | Users have many Roles through `user_roles`. | **High** | JOIN through bridge table. Two-step query or explicit join. |
| Nested preloading | `User |> preload(posts: :comments)` | **High** | Multiple queries, recursive result stitching. |

**Critical design decision: Explicit preloading only. No lazy loading.** This is the single most important architectural choice. Ecto chose this deliberately and it is universally praised. Lazy loading (ActiveRecord, SQLAlchemy) silently triggers N+1 queries and is the #1 source of ORM performance issues. In a functional language without mutable state, lazy loading is even more problematic -- it requires hidden side effects.

Unloaded associations return a `NotLoaded` marker value. Accessing it produces a clear error: "association :posts not loaded. Use Repo.preload or include in query." This forces developers to be explicit about what data they need.

### 5. Changesets for Validation and Casting

The pipeline between raw external data and the database.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Changeset.cast(struct, params, allowed)` | Filter + type-cast external params to schema types. Prevents mass assignment. | **High** | Takes struct + Map + list of allowed fields. Converts string values to schema types. |
| `validate_required(cs, fields)` | Ensure fields present and non-empty. Most common validation. | **Low** | Check each field has value in changes. Add error if missing. |
| `validate_length(cs, field, opts)` | String/list length bounds (min, max). | **Low** | `validate_length(:name, min: 2, max: 100)`. |
| `validate_format(cs, field, pattern)` | String pattern matching. Email, phone, etc. | **Low** | Mesh string operations or regex if added. |
| `validate_inclusion(cs, field, values)` | Value in allowed set. For enum-like fields. | **Low** | `validate_inclusion(:role, ["admin", "member"])`. Uses List.contains. |
| `validate_number(cs, field, opts)` | Numeric bounds. greater_than, less_than. | **Low** | `validate_number(:age, greater_than: 0)`. |
| Custom validation functions | User-defined validation logic. | **Low** | `fn(changeset) -> changeset` that calls add_error. Pipe-friendly. |
| `unique_constraint(cs, field)` | Map PG unique violation to changeset error. | **Med** | Catch PG 23505 error, convert to field error. |
| `foreign_key_constraint(cs, field)` | Map FK violation to changeset error. | **Med** | Catch PG 23503 error, convert to field error. |
| Change tracking (dirty fields) | Know which fields changed for UPDATE optimization. | **Med** | Changeset stores changes map. Repo.update only SETs changed fields. |

**How the changeset pipeline works:**

```
# Pseudocode for Mesh ORM changeset workflow
user
  |> Changeset.cast(params, [:name, :email, :age])   # filter + type-cast
  |> Changeset.validate_required([:name, :email])     # check presence
  |> Changeset.validate_format(:email, "@")           # check format
  |> Changeset.validate_number(:age, greater_than: 0) # check bounds
  |> Repo.insert()                                     # persist or return errors
```

The Changeset struct shape:
- `data :: T` -- the original struct
- `changes :: Map<String, String>` -- approved modifications (field name -> new value)
- `errors :: List<{String, String}>` -- validation failures (field, message)
- `valid :: Bool` -- overall validity flag

Repo.insert/update check `valid` before executing SQL. If invalid, return `Err(changeset)` with errors attached.

### 6. Migration Tooling

Schema evolution over time. Every production application needs this.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Migration file generation | Create timestamped files with up/down. | **Med** | `mesh orm.gen.migration CreateUsers` -> `20260216120000_create_users.mpl` |
| `create_table` DDL helper | DSL for creating tables. | **Med** | `create_table("users") do add(:name, :string, null: false) end` |
| `alter_table` DDL helper | Add/remove/rename columns. | **Med** | `alter_table("users") do add(:email, :string) end` |
| `drop_table` helper | Drop tables in down migrations. | **Low** | `drop_table("users")` |
| `create_index` helper | Create indexes. | **Low** | `create_index("users", [:email], unique: true)` |
| Migration runner (up/down) | Apply pending, rollback last. | **Med** | `schema_migrations` table tracks applied by version timestamp. |
| Rollback support | Undo last N migrations. | **Med** | Run down function of last applied migration. |

**Recommendation:** Use explicit `up`/`down` functions (Ecto/Diesel pattern). NOT auto-diff (Prisma pattern) -- schema diffing is extremely complex and dangerous for production. Users write migration logic explicitly. Provide `mesh orm.gen.migration` to scaffold empty files.

---

## Differentiators

Features that set the Mesh ORM apart. Not expected, but valuable.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **Pipe-native query builder** | Designed from the ground up for `|>` composition. More natural than Ecto's binding syntax. | **Low** | Each function takes Query as first arg, returns Query. Natural pipe threading. |
| **Compile-time field validation** | Catch `:naem` typos at compile time, not runtime. | **Med** | If atoms are compile-time values, check against schema field list during type checking. |
| **Actor-integrated transactions** | Transaction blocks run with crash isolation via catch_unwind. Already exists. | **Low** | Pg.transaction already does this. ORM wraps it. |
| **Composable scopes as functions** | Named query fragments: `pub fn active(q) do q |> where(:active, true) end`. | **Low** | Free with pipe composition -- just functions returning Query structs. |
| **Upsert support (insert_or_update)** | PostgreSQL `ON CONFLICT` clause. Heavily used in Mesher's `upsert_issue`. | **Med** | `Repo.insert(changeset, on_conflict: :replace_all, conflict_target: [:email])`. |
| **Schemaless changesets** | Validate data without a database-backed schema. Useful for API input validation. | **Med** | Changeset works with any `{data, types}` pair, not just schema structs. |
| **Schema metadata at compile time** | No runtime reflection overhead. All metadata baked in at compilation. Zero-cost. | **Med** | `deriving(Schema)` generates static functions/constants. |
| **PostgreSQL-native features** | JSONB queries, array types, CTEs, window functions via raw SQL escape hatch. No multi-DB abstraction tax. | **Low** | PG-only means we can use PG features freely. |

---

## Anti-Features

Features to explicitly NOT build. Each has strong reasons.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| **Lazy loading** | #1 ORM anti-pattern. Silently triggers N+1 queries. In a functional language without mutable state, it requires hidden side effects. Ecto deliberately omitted this; universally praised. ActiveRecord/SQLAlchemy lazy loading causes more production perf issues than any other ORM feature. | Explicit preloading only. Unloaded associations return `NotLoaded` marker that errors on access, forcing explicit preload. |
| **Active Record pattern (model.save)** | Blurs data/DB boundary. Leads to `User.save()` scattered everywhere. Violates separation of concerns. Incompatible with Mesh's functional paradigm (no mutable objects with methods). | Repo pattern: all DB ops through `Repo.insert/update/delete/all`. Models are pure data structs. |
| **Identity map / session cache** | SQLAlchemy's Session tracks every loaded object. Adds enormous complexity (stale data, cache invalidation). In a functional language with immutable data, an identity map is nonsensical. | Every query returns fresh data. No caching in ORM. Application builds its own cache if needed. |
| **Unit of Work (batch flush)** | SQLAlchemy auto-tracks changes, flushes in batch. Complex implicit behavior. "Why did my UPDATE run here?" Explicit is better in functional languages. | Explicit operations: Repo.insert inserts immediately. Use Repo.transaction to batch atomically. |
| **Automatic schema-diff migrations** | Prisma-style auto-generation hides complexity. Column rename vs drop+add ambiguity. Data loss detection is extremely hard. Dangerous for production. | Explicit migration files with up/down. Add `mesh orm.gen.migration` scaffold. Schema-diff as future differentiator. |
| **Multi-database support** | Supporting MySQL + PG + SQLite makes every feature 3x harder. Prevents PG-specific features (JSONB, arrays, upserts, CTEs, window functions). | PostgreSQL only. Use PG features freely. Matches existing Mesh driver. |
| **Callback hooks (before_save, after_create)** | ActiveRecord callbacks scatter side effects throughout model lifecycle. Order-dependent, hard to test, surprising. "Why did sending email happen in my unit test?" | No lifecycle callbacks. Explicit function calls in application code. Ecto deliberately omits callbacks. |
| **Dynamic finders (find_by_name)** | Magic methods generated at runtime. No compile-time safety. Nonsensical in a statically-typed language. | Use `Repo.get_by(User, name: "Alice")` or query builder. |
| **Polymorphic associations** | Breaks referential integrity. Complex to implement. Different meaning across ORMs. | Use explicit join tables or sum types. |
| **Full SQL parser** | SQL grammar is enormous. Diminishing returns. | Query builder for 95% of cases. Raw SQL escape hatch for the rest. |
| **Embedded schemas** | Ecto has these for non-persisted data. Adds complexity for marginal value. | Use regular structs with `deriving(Json)` for non-persisted data. |
| **Query caching / prepared statements** | Premature optimization. PG already caches query plans. Adds complexity. | Use PG's built-in plan caching. Add prepared statements later if needed. |
| **Cyclic association loading** | `User -> Posts -> User -> Posts -> ...` infinite traversal. Some ORMs handle with session cache. Creates infinite object graphs. | Preloading is always one-directional and explicitly bounded. Nest explicitly: `preload(posts: :user)`. |

---

## Feature Dependencies

```
Compiler Additions (parallel track -- enables ergonomic DSL)
  |
  +-> Keyword Arguments (enables where(name: "Alice") syntax)
  |     |
  |     +-> Query Builder conditions
  |     +-> Changeset cast field lists
  |     +-> Migration DSL options
  |
  +-> Multi-line Pipe Chains (enables readable query pipelines)
  |     |
  |     +-> Query Builder usability
  |
  +-> Atom Literals (enables :field_name references)
  |     |
  |     +-> Schema field references in queries
  |     +-> Validation field references
  |
  +-> Struct Update Syntax (enables %{user | name: "Bob"})
        |
        +-> Changeset apply_changes

Schema DSL (foundation -- everything depends on this)
  |
  +-> Schema Metadata Generation (field names, types, table, PK)
  |     |
  |     +-> Query Builder (needs metadata for SQL generation)
  |     |     |
  |     |     +-> where, select, order_by, limit, offset
  |     |     +-> join (needs relationship metadata)
  |     |     +-> group_by, having
  |     |
  |     +-> Changeset System (needs metadata for casting)
  |     |     |
  |     |     +-> cast (type conversion)
  |     |     +-> validate_* functions
  |     |     +-> constraint error mapping (PG error -> changeset error)
  |     |
  |     +-> Repo Operations (needs metadata for SQL generation)
  |           |
  |           +-> Repo.insert (changeset + metadata -> INSERT SQL)
  |           +-> Repo.update (changeset + PK + changes -> UPDATE SQL)
  |           +-> Repo.delete (PK -> DELETE SQL)
  |           +-> Repo.all / one / get / get_by (query builder -> SELECT SQL)
  |
  +-> Relationship Definitions (belongs_to, has_many, has_one, many_to_many)
  |     |
  |     +-> FK Field Generation (belongs_to adds user_id)
  |     +-> Preload Queries (has_many generates SELECT WHERE fk = $1)
  |     +-> Join Queries (builder uses relationship metadata)
  |     +-> Nested Preloading (recursive preload + stitch)
  |
  +-> Migration Tooling (parallel, uses schema metadata for future auto-gen)
        |
        +-> Migration DSL (create_table, alter_table, create_index)
        +-> Migration Runner (up/down, schema_migrations tracking)
        +-> Migration Generation CLI (mesh orm.gen.migration)
```

**Key ordering insight:** Compiler additions should come first or in parallel with Schema DSL because they unlock ergonomic syntax for everything downstream. Schema DSL is the next critical foundation -- query builder, changesets, Repo, and relationships all depend on schema metadata. Query builder and changesets are independent of each other but both feed into Repo operations. Relationships and preloading layer on top. Migrations are partially independent.

---

## MVP Recommendation

Build in dependency order. Each step unlocks the next.

### Phase 1: Compiler Additions + Schema DSL
1. **Keyword arguments** -- Highest-leverage compiler change. Unlocks `where(name: "Alice")`.
2. **Atom literals** -- Enable `:field_name` references for field identification.
3. **Multi-line pipe chains** -- Already a known limitation. Unlock readable query pipelines.
4. **Struct update syntax** -- `%{user | name: "Bob"}`. Needed for changeset apply.
5. **Schema DSL** -- `schema Users, "users" do field :name, String end`. Generates struct + metadata.

### Phase 2: Query Builder + Repo Basics
6. **Query struct + basic clauses** -- where, select, order_by, limit, offset. Pipe-composable.
7. **Repo.all / Repo.one / Repo.get / Repo.get_by** -- Execute queries against DB.
8. **Repo.insert / Repo.update / Repo.delete** -- Basic CRUD through Repo.

### Phase 3: Changesets
9. **Changeset struct + cast** -- Type-safe casting from external params.
10. **Validation functions** -- validate_required, validate_length, validate_format, validate_inclusion, validate_number.
11. **Constraint error mapping** -- PG unique/FK violations -> changeset errors.

### Phase 4: Relationships + Preloading
12. **belongs_to / has_many / has_one** -- Define associations in schema.
13. **Preloading** -- `Repo.preload(user, :posts)` and `User |> preload(:posts) |> Repo.all()`.
14. **many_to_many** -- Join table relationships.
15. **Nested preloading** -- `preload(posts: :comments)`.

### Phase 5: Migration Tooling
16. **Migration DSL** -- create_table, alter_table, drop_table, create_index.
17. **Migration runner** -- Apply/rollback, schema_migrations tracking.
18. **Migration generation CLI** -- `mesh orm.gen.migration CreateUsers`.

### Phase 6: Validation (Rewrite Mesher)
19. **Rewrite Mesher's DB layer** using the ORM. Replace all 627 lines of raw SQL queries + 82 lines of schema DDL.

**Defer to post-MVP:**
- Compile-time query field validation (complex type system work)
- Schema-diff migration auto-generation (complex DB introspection)
- Schemaless changesets (nice-to-have)
- Aggregate functions as first-class query builder ops (use raw SQL)
- Upsert support (raw SQL for Mesher's upsert_issue initially)
- Subqueries and CTEs (raw SQL escape hatch)
- Streaming/cursor queries (LIMIT/OFFSET pagination initially)

---

## Complexity Summary

| Feature Area | Complexity | Why | Primary Dependencies |
|--------------|------------|-----|---------------------|
| Compiler: Keyword Args | **High** | Touches every compiler stage: lexer, parser, typeck, MIR, codegen | All compiler crates |
| Compiler: Multi-line Pipes | **Med** | Parser change only. Well-understood from Elixir | mesh-parser |
| Compiler: Atom Literals | **Med** | Lexer + parser + type system. New type `Atom` | mesh-lexer, mesh-parser, mesh-typeck |
| Compiler: Struct Update | **Med** | Parser + codegen. Must generate correct field copying | mesh-parser, mesh-codegen |
| Schema DSL | **High** | New syntax or deriving. Must generate struct + metadata at compile time | Parser or deriving, code generation |
| Query Builder (basic) | **Med** | Struct with SQL generation. Well-understood from Ecto | Schema metadata |
| Query Builder (joins) | **High** | Relationship metadata, binding management, SQL joins | Schema relationships |
| Repo Operations | **Med** | SQL generation + Pool.query. Builds on existing infra | Query builder, Pool.query |
| Changesets | **Med** | Pure data structure + validation functions | Schema metadata for casting |
| Relationships | **High** | Schema additions, FK conventions, preload generation | Schema DSL |
| Preloading (basic) | **Med** | Separate queries, stitch results | Relationships |
| Preloading (nested) | **High** | Recursive planning, multiple queries, nested stitching | Basic preloading |
| Migration DSL | **Med** | SQL DDL generation from function calls | None (standalone) |
| Migration Runner | **Med** | Track applied, execute in order, rollback | PostgreSQL |
| Mesher Rewrite | **High** | 627 lines raw SQL + 82 lines DDL to replace. Real-world validation | All ORM features |

---

## Sources

### Ecto (Elixir) -- Primary Reference
- [Ecto.Schema v3.13.5](https://hexdocs.pm/ecto/Ecto.Schema.html) -- HIGH confidence
- [Ecto.Query v3.13.5](https://hexdocs.pm/ecto/Ecto.Query.html) -- HIGH confidence
- [Ecto.Changeset v3.13.5](https://hexdocs.pm/ecto/Ecto.Changeset.html) -- HIGH confidence
- [Ecto.Repo v3.13.5](https://hexdocs.pm/ecto/Ecto.Repo.html) -- HIGH confidence
- [Ecto Associations](https://hexdocs.pm/ecto/associations.html) -- HIGH confidence
- [Preloading Nested Associations (Thoughtbot)](https://thoughtbot.com/blog/preloading-nested-associations-with-ecto) -- MEDIUM confidence
- [Ecto.Query.preload vs Ecto.Repo.preload](https://appunite.com/blog/ecto-query-preload-vs-ecto-repo-preload) -- MEDIUM confidence
- [Composing Ecto Queries (AmberBit)](https://www.amberbit.com/blog/2019/4/16/composing-ecto-queries-filters-and-preloads/) -- MEDIUM confidence

### ActiveRecord (Ruby/Rails)
- [Active Record Query Interface](https://guides.rubyonrails.org/active_record_querying.html) -- HIGH confidence
- [ActiveRecord::QueryMethods](https://api.rubyonrails.org/classes/ActiveRecord/QueryMethods.html) -- HIGH confidence

### Prisma (TypeScript)
- [Prisma ORM](https://www.prisma.io/orm) -- HIGH confidence
- [Drizzle vs Prisma 2026](https://makerkit.dev/blog/tutorials/drizzle-vs-prisma) -- MEDIUM confidence

### Diesel / SeaORM (Rust)
- [Diesel ORM](https://diesel.rs/) -- HIGH confidence
- [SeaORM](https://www.sea-ql.org/SeaORM/) -- HIGH confidence
- [SeaORM 2.0](https://www.sea-ql.org/blog/2025-12-12-sea-orm-2.0/) -- HIGH confidence
- [Rust ORMs Guide (Shuttle)](https://www.shuttle.dev/blog/2024/01/16/best-orm-rust) -- MEDIUM confidence

### SQLAlchemy (Python)
- [SQLAlchemy Session Basics](https://docs.sqlalchemy.org/en/20/orm/session_basics.html) -- HIGH confidence

### ORM Anti-Patterns
- [The Basic Mistake All ORMs Make (Vogten)](https://martijnvogten.github.io/2025/04/16/the-basic-mistake-all-orms-make-and-how-to-fix-it.html) -- MEDIUM confidence
- [ORM Lazy Loading Anti-Pattern](https://www.mehdi-khalili.com/orm-anti-patterns-part-3-lazy-loading) -- MEDIUM confidence
- [ORM Framework Anti-Patterns (Lindbakk)](https://lindbakk.com/blog/orm-frameworks-anti-patterns) -- MEDIUM confidence

### Migration Tooling
- [MikroORM Migrations](https://mikro-orm.io/docs/migrations) -- MEDIUM confidence
- [Schema Migration Tools 2025](https://www.getgalaxy.io/learn/data-tools/best-database-schema-migration-version-control-tools-2025) -- MEDIUM confidence

---
*Feature research for: Mesh ORM Library (v10.0)*
*Researched: 2026-02-16*
