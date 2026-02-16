# Technology Stack

**Project:** Mesh ORM Library (v10.0)
**Researched:** 2026-02-16
**Confidence:** HIGH (compiler analysis), MEDIUM (DSL design patterns), HIGH (migration tooling)

## Executive Summary

Building an ORM for Mesh requires a combination of **compiler additions** and **pure Mesh library code**. The key insight from analyzing Ecto, Diesel, and the existing Mesh compiler is that Mesh already has most of the building blocks -- `deriving(Row)`, struct definitions, pipe operator, closures, traits, generics -- but lacks three critical features needed for an expressive ORM:

1. **Atom/symbol literals** (`:name`, `:email`) for field references in queries and schema DSL
2. **Keyword arguments** (`where(name: "Alice", age: 30)`) for ergonomic query builder API
3. **Extended deriving system** (`deriving(Schema)`) for compile-time generation of schema metadata, relationship accessors, and changeset functions

No macro system is needed. No new parser grammar for a "schema DSL block" is needed. The ORM can be built using the existing `struct` + `deriving` pattern, with atoms and keyword args as the two new language primitives.

## What Exists Today (DO NOT rebuild)

These Mesh features are validated and ready for ORM use:

| Capability | How ORM Uses It | Status |
|------------|----------------|--------|
| `struct` definitions with typed fields | Schema model definitions (struct User do name :: String end) | Shipped |
| `deriving(Row)` | Basis for `deriving(Schema)` -- same compiler-generated function pattern | Shipped |
| `deriving(Json)` | Changeset serialization, API integration | Shipped |
| Pipe operator `\|>` | Query builder chains: `User \|> where(...) \|> Repo.all()` | Shipped |
| Closures `fn(x) -> expr end` | Dynamic query fragments, custom validations | Shipped |
| Trailing closures `foo() do \|x\| ... end` | Transaction blocks: `Repo.transaction() do \|conn\| ... end` | Shipped |
| Trait system with associated types | Queryable, Changeable, Schema traits for type-safe dispatch | Shipped |
| `From/Into` conversion traits | Type coercion in changesets (string -> int parsing) | Shipped |
| Result/Option with `?` propagation | Error handling in Repo operations | Shipped |
| `Pg.execute`, `Pg.query`, parameterized queries | Underlying SQL execution layer | Shipped |
| Connection pooling with `Pool.open/query_as/execute` | Production database access | Shipped |
| Transactions `Pg.transaction(conn, fn)` | Transaction support for Repo operations | Shipped |
| Map type with string keys | Row data representation, changeset params | Shipped |
| Pattern matching, case expressions | Changeset validation branching | Shipped |
| Module system with pub visibility | ORM library organization (Repo, Query, Schema, Migration modules) | Shipped |
| `List.map`, `List.filter`, iterators | Collection operations on query results | Shipped |

## Recommended Stack Additions

### 1. Atom Literals (Compiler Addition -- REQUIRED)

| Property | Detail |
|----------|--------|
| Syntax | `:name`, `:email`, `:inserted_at`, `:asc`, `:desc` |
| Runtime representation | Interned string pointer (like Erlang atoms) or compile-time string constant |
| Type | New `Atom` type in the type system |
| Why required | Field references in queries (`where(:name, "Alice")`), ordering direction (`:asc`/`:desc`), association keys (`:user_id`), migration column specification |

**Why atoms and not plain strings:** Strings are runtime values that can be anything. Atoms are compile-time-known identifiers that the compiler can validate against struct field definitions. `where(:naem, "Alice")` can produce a compile error; `where("naem", "Alice")` cannot. Atoms also read better in DSL code -- they look like field references, not arbitrary data.

**Implementation approach:**

Lexer: Add `:` followed by identifier as a new token kind `AtomLiteral`. The `:` is already a `Colon` token, so the lexer needs a contextual rule: when `:` is followed immediately (no space) by an identifier character, lex as `AtomLiteral` instead of `Colon` + `Ident`. This avoids ambiguity with type annotations (`:: Type`) which use `ColonColon`.

Parser: Add `ATOM_LITERAL` to SyntaxKind. Parse as a primary expression (like string/int literals). The AST node carries the atom name as a string.

Type checker: Add `Ty::Atom` or use `Ty::Con(TyCon::new("Atom"))`. For the ORM, atoms don't need to be a full algebraic type -- they're just compile-time string constants with a distinct type. The type checker can optionally validate atoms against known field names when used in ORM contexts.

Codegen: Atoms compile to string constants. At the MIR/LLVM level, an atom is just a pointer to a string literal in the data section. No interning table needed for v10.0 -- that's an optimization for a future version.

**Estimated scope:** ~200 LOC across lexer, parser, type checker, codegen. Small, self-contained change.

### 2. Keyword Arguments (Compiler Addition -- REQUIRED)

| Property | Detail |
|----------|--------|
| Syntax | `where(name: "Alice", age: 30)` at call sites |
| Representation | Desugared to `Map<Atom, Any>` or a struct literal at the call site |
| Why required | Ergonomic query builder API, changeset `cast` field lists, migration column options |

**Why keyword args and not Map literals:** `where(%{name: "Alice"})` is verbose and ugly. `where(name: "Alice")` is the Ecto/Ruby ergonomic that makes ORM code readable. Keyword arguments at call sites are syntactic sugar that makes the pipe-chain query builder feel natural.

**Implementation approach -- two options:**

**Option A (Recommended): Keyword args as last-position Map sugar.**
When the parser sees `name: expr` pairs in a function argument list (identifiers followed by `:` then expression), collect them as a single `Map<String, T>` argument. This is exactly how Ruby and Elixir handle keyword arguments.

```
where(name: "Alice", age: 30)
# desugars to:
where(%{"name" => "Alice", "age" => 30})
```

This requires:
- Parser: Detect `ident COLON expr` pattern in arg lists, collect into a MAP_LITERAL node as the final argument
- Type checker: The function signature expects a Map (or a new KeywordList type) as the last parameter
- Codegen: No change -- it's just a Map

**Option B: Keyword args as ordered list of pairs (Elixir-style).**
`[name: "Alice", age: 30]` desugars to `[(:name, "Alice"), (:age, 30)]` -- a `List<(Atom, T)>`. This preserves ordering and allows duplicate keys.

**Recommendation: Option A** (Map sugar) because Mesh already has Map with full codegen support. The `List<(Atom, T)>` approach requires tuple-of-mixed-types which is harder to type-check with HM inference.

**Estimated scope:** ~300 LOC in parser (detect pattern in arg lists), ~50 LOC type checker, 0 LOC codegen.

### 3. Extended Deriving System: `deriving(Schema)` (Compiler Addition -- REQUIRED)

| Property | Detail |
|----------|--------|
| Syntax | `struct User do ... end deriving(Schema, table: "users")` |
| What it generates | `__schema__/1` metadata function, `changeset/2`, `from_row/1` (already exists from Row), field type map, relationship accessors |
| Why required | Schema metadata must be accessible at runtime for query building, validation, and relationship loading |

**How deriving(Schema) extends the existing pattern:**

The existing `deriving(Row)` generates a `from_row` function. `deriving(Json)` generates `to_json`/`from_json`. Following this exact pattern, `deriving(Schema)` generates:

1. **`__table__()` -> String**: Returns the table name (from the `table:` option, or pluralized struct name as default)
2. **`__fields__()` -> List<(String, String)>`: Returns field names and their SQL types
3. **`__primary_key__()` -> String**: Returns the primary key field name (default: `"id"`)
4. **`changeset(struct, params)` -> Changeset<T>**: Casts external params onto the struct with type checking
5. Relationship metadata functions (generated from relationship declarations -- see below)

**Estimated scope:** ~500 LOC in type checker (registration), ~800 LOC in codegen (MIR generation), following the exact pattern of `generate_from_row_struct` and `generate_to_json_struct`.

### 4. Relationship Declarations (New Struct Annotation Syntax -- Compiler Addition)

| Property | Detail |
|----------|--------|
| Syntax | Inside struct body: `belongs_to :user, User` / `has_many :posts, Post` / `has_one :profile, Profile` |
| Parser change | New node types inside STRUCT_DEF: BELONGS_TO_DECL, HAS_MANY_DECL, HAS_ONE_DECL |
| What they generate | Foreign key field (for belongs_to), association loader functions, preload metadata |

**Implementation approach:**

These are NOT new keywords. They are parsed as function-call-like declarations within a struct body when `deriving(Schema)` is present. The parser sees `belongs_to` as an identifier followed by atom and type arguments.

Actually, to keep it simpler and avoid keyword reservation: **use the `def` keyword pattern** already in the language, or simply make `belongs_to`, `has_many`, `has_one` new keywords. Given the language already has 48 keywords and the ORM is a first-class feature, adding 3 more keywords is justified.

**Recommended: Add `belongs_to`, `has_many`, `has_one` as parser-recognized declarations inside struct bodies.**

Lexer: Add 3 new keywords.
Parser: Inside struct body parsing, recognize these as relationship declarations.
Type checker: Validate relationship targets exist, generate appropriate types.
Codegen: Generate loader functions and metadata.

**Estimated scope:** ~100 LOC lexer/parser, ~300 LOC type checker, ~400 LOC codegen.

### 5. Multi-line Pipe Chains (Compiler Fix -- STRONGLY RECOMMENDED)

| Property | Detail |
|----------|--------|
| Current limitation | `\|>` must be on same line as previous expression |
| Needed | `User \|> where(name: "Alice") \|> limit(10) \|> Repo.all()` across multiple lines |
| Impact | Without this, every ORM query chain must be on one long line or use parenthesized workaround |

**This is listed as a known limitation in STATE.md.** Fix it for v10.0 because ORM query chains are the primary use case for multi-line pipes.

**Implementation:** In the parser's newline handling, when a line starts with `|>`, treat it as a continuation of the previous expression rather than a new statement. This is a ~50 LOC parser change.

### 6. Struct Update Syntax (Compiler Addition -- RECOMMENDED)

| Property | Detail |
|----------|--------|
| Syntax | `%{user \| name: "Bob", age: 31}` or `User { ...user, name: "Bob" }` |
| Why needed | Changeset application: `apply_changes(changeset)` returns a new struct with changed fields applied |
| Current workaround | Manually construct new struct with all fields, which is verbose and error-prone |

**Implementation:** Add spread/update syntax to struct literals. When the parser sees `...expr` inside a struct literal, it copies all fields from the source struct and overrides the specified ones.

**Estimated scope:** ~150 LOC parser, ~200 LOC type checker (verify field compatibility), ~300 LOC codegen.

## What Does NOT Need Compiler Changes

These ORM components are implementable in **pure Mesh** using existing features:

| Component | Implementation Approach | Why No Compiler Change |
|-----------|------------------------|----------------------|
| Query builder types | Mesh structs with builder methods | Just structs and functions |
| `Repo.all/insert/update/delete` | Module functions calling `Pool.query_as`/`Pool.execute` | Existing DB primitives |
| SQL generation from Query struct | String concatenation with parameterized placeholders | Pure string building |
| Migration file format | Mesh source files with `up()`/`down()` functions | Standard Mesh code |
| Migration runner | Module that reads migration files, tracks applied versions | Existing FS + DB operations |
| Changeset validation pipeline | Pipe-chain of validation functions on Changeset struct | Closures + pipes |
| Preloading/eager loading | Separate queries + Map-based association | Existing Map + List operations |
| Connection pool management | Already exists via `Pool.open` | Shipped |
| Transaction wrapping | Already exists via `Pg.transaction` | Shipped |

## Stack Decision Matrix

| Component | Compiler Change? | Mesh Library? | Complexity | Priority |
|-----------|-----------------|---------------|------------|----------|
| Atom literals | YES | - | Low | P0 (blocks everything) |
| Keyword arguments | YES | - | Medium | P0 (blocks query builder) |
| Multi-line pipes | YES (parser fix) | - | Low | P0 (blocks usability) |
| `deriving(Schema)` | YES | - | High | P1 (core feature) |
| Relationship declarations | YES | - | Medium | P1 (core feature) |
| Struct update syntax | YES | - | Medium | P2 (changeset convenience) |
| Query builder | - | YES | High | P1 |
| Repo module | - | YES | Medium | P1 |
| Migration tooling | - | YES | Medium | P2 |
| Changeset system | - | YES | Medium | P1 |
| Preloading | - | YES | High | P2 |

## Alternatives Considered

| Decision | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Field references | Atom literals (`:name`) | String literals (`"name"`) | No compile-time validation, looks like data not identifiers |
| Query builder API | Keyword args (`where(name: "x")`) | Map args (`where(%{name: "x"})`) | Verbose, ugly, doesn't match Ecto ergonomics |
| Schema DSL | Extended `deriving(Schema)` on structs | New `schema` keyword/block | Structs already work; adding a whole new declaration type adds parser complexity for no benefit |
| Relationship syntax | Keywords in struct body | Annotations/decorators | Mesh has no annotation system; keywords are simpler |
| Migration format | Mesh source files (.mpl) | SQL files | Mesh files can use the migration DSL for reversibility |
| Query approach | Query builder + raw SQL escape hatch | Full SQL parser | Parsing SQL is massive scope; query builder handles 95% of cases |
| Macro system | NOT adding one | Elixir-style compile-time macros | Massive compiler scope creep; `deriving` handles the code generation needs |
| Runtime reflection | NOT adding | Runtime type introspection | `deriving(Schema)` generates all needed metadata at compile time; no reflection needed |

## Version/Compatibility

| Technology | Current Version | Changes for v10.0 |
|------------|----------------|-------------------|
| Mesh compiler (meshc) | ~99K LOC Rust | +~2500 LOC for atom, kwargs, schema deriving, relationships, struct update, multi-line pipes |
| PostgreSQL driver | Shipped v2.0 | No changes -- ORM builds on top |
| Connection pooling | Shipped v3.0 | No changes |
| Type checker (HM) | Shipped v1.0-v7.0 | Extended for atom type, keyword arg desugaring, schema validation |
| Lexer | 96 token kinds | +1 (AtomLiteral), +3 keywords (belongs_to, has_many, has_one) |
| Parser | ~120 syntax kinds | +5-8 new node kinds (ATOM_LITERAL, BELONGS_TO_DECL, HAS_MANY_DECL, HAS_ONE_DECL, STRUCT_UPDATE) |

## Resulting ORM API (Target Syntax)

This is what the ORM code looks like with all stack additions in place:

```mesh
# Schema definition
struct User do
  id :: Int
  name :: String
  email :: String
  age :: Int
  inserted_at :: String
  updated_at :: String

  has_many :posts, Post
  belongs_to :team, Team
end deriving(Schema, table: "users")

# Query builder (pipe chains)
let users = User
  |> Query.where(name: "Alice")
  |> Query.order_by(:inserted_at, :desc)
  |> Query.limit(10)
  |> Repo.all(pool)

# Changeset
let changeset = User.changeset(user, %{name: "Bob", age: 31})
  |> Changeset.validate_required([:name, :email])
  |> Changeset.validate_length(:name, min: 2, max: 100)

case Repo.update(pool, changeset) do
  Ok(updated_user) -> println("Updated: ${updated_user.name}")
  Err(changeset) -> println("Errors: ${changeset.errors}")
end

# Migration
module CreateUsers do
  pub fn up(conn) do
    Migration.create_table(conn, "users") do |t|
      t.string(:name)
      t.string(:email)
      t.integer(:age)
      t.timestamps()
    end

    Migration.create_index(conn, "users", [:email], unique: true)
  end

  pub fn down(conn) do
    Migration.drop_table(conn, "users")
  end
end
```

## Implementation Order

The compiler additions must be built in dependency order:

1. **Atom literals** -- No dependencies. Enables field references everywhere.
2. **Multi-line pipe chains** -- No dependencies. Parser fix only.
3. **Keyword arguments** -- Depends on atoms (keywords desugar with atom keys).
4. **Struct update syntax** -- No dependencies. Standalone parser+codegen feature.
5. **`deriving(Schema)`** -- Depends on atoms (schema metadata uses atoms for field names).
6. **Relationship declarations** -- Depends on Schema deriving (extends it).

After compiler additions (phases 1-6), all ORM library code (Query builder, Repo, Changeset, Migration) is pure Mesh.

## Sources

- [Ecto.Schema documentation](https://hexdocs.pm/ecto/Ecto.Schema.html) -- Schema DSL design, macro-based field/relationship declarations (HIGH confidence)
- [Ecto.Changeset documentation](https://hexdocs.pm/ecto/Ecto.Changeset.html) -- Changeset pipeline design, validation vs constraints, cast/validate separation (HIGH confidence)
- [Ecto.Migration documentation](https://hexdocs.pm/ecto_sql/Ecto.Migration.html) -- Migration file format, up/down/change, transaction behavior, schema_migrations tracking (HIGH confidence)
- [Ecto.Query documentation](https://hexdocs.pm/ecto/Ecto.Query.html) -- Query builder API, binding system, type safety, composability (HIGH confidence)
- [Diesel ORM](https://diesel.rs/) -- Rust compile-time query validation via type system, table! macro code generation (HIGH confidence)
- [Anatomy of an Ecto migration](https://fly.io/phoenix-files/anatomy-of-an-ecto-migration/) -- Migration internals, timestamp versioning, deferred execution (MEDIUM confidence)
- [Elixir School: Ecto Associations](https://elixirschool.com/en/lessons/ecto/associations) -- belongs_to/has_many implementation patterns (MEDIUM confidence)
- [A Guide to Rust ORMs in 2025](https://www.shuttle.dev/blog/2024/01/16/best-orm-rust) -- Comparison of Diesel vs SeaORM vs SQLx approaches (MEDIUM confidence)
- Mesh compiler source analysis: mesh-lexer, mesh-parser, mesh-typeck, mesh-codegen (HIGH confidence -- direct code reading)
- Mesh PROJECT.md v10.0 requirements (HIGH confidence -- direct project specification)

---
*Stack research for: Mesh ORM Library (v10.0)*
*Researched: 2026-02-16*
