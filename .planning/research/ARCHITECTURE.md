# Architecture Patterns

**Domain:** ORM library integration into the Mesh programming language
**Researched:** 2026-02-16
**Overall confidence:** HIGH (architecture derived from direct codebase analysis of all compiler crates, runtime DB layer, and Mesher application code)

## Executive Summary

The Mesh ORM integrates into a compiler/runtime system with well-defined boundaries: a Rust compiler (mesh-lexer through mesh-codegen), a Rust runtime (mesh-rt with PG driver, connection pool, row parsing), and Mesh-level user code (types, queries, services). The ORM's architecture must span all three layers -- new compiler deriving infrastructure, new runtime functions for query execution and SQL generation, and a Mesh-level library providing the user-facing API (Schema, Query, Repo, Changeset, Migration).

The key architectural insight from studying the existing codebase is that Mesh already has all the building blocks for an ORM, but they exist as disconnected primitives. The `deriving(Row)` system maps database rows to structs. `Pool.query` and `Pool.execute` run SQL. Type annotations give struct field metadata at compile time. What the ORM adds is the connective tissue: schema metadata that links struct fields to database columns, a query builder that composes SQL from struct types, a Repo module that combines pool operations with row mapping, and a migration system that generates DDL from schema definitions.

The architecture follows Ecto's four-module pattern (Schema, Repo, Query, Changeset) because Mesh's language primitives map almost 1:1 to Elixir's: pipe-chain composition, functional data transformation, structs with metadata, and explicit separation of data and behavior. This is not coincidence -- Mesh was designed with Elixir's philosophy, so Ecto's patterns are the natural ORM architecture for this language.

## Existing Architecture (What We Are Integrating Into)

### Current Layer Map

```
LAYER 1: Mesh User Code (.mpl files)
  Types/User.mpl       struct User do ... end deriving(Json, Row)
  Storage/Queries.mpl   Pool.query(pool, "SELECT ...", [params])
  Services/User.mpl     service UserService do ... end

LAYER 2: Mesh Compiler (Rust, ~99K LOC)
  mesh-parser       Parses deriving() clauses, struct defs, fn defs
  mesh-typeck       Registers trait impls for deriving, validates trait names
  mesh-codegen      Generates MIR functions: FromRow__from_row__User, to_json, etc.

LAYER 3: Mesh Runtime (Rust, mesh-rt crate)
  db/pool.rs        mesh_pool_query, mesh_pool_execute (auto checkout/checkin)
  db/pg.rs          Wire protocol v3, SCRAM-SHA-256, TLS
  db/row.rs         mesh_row_from_row_get, mesh_row_parse_int/float/bool
  json.rs           mesh_json_encode, mesh_json_decode
```

### Current Data Flow (Raw SQL Pattern)

```
Mesh code:                Pool.query(pool, sql_string, params_list)
                               |
Compiler resolves to:     mesh_pool_query(pool_handle, sql_ptr, params_ptr)
                               |
Runtime (pool.rs):        checkout conn -> mesh_pg_query -> checkin conn
                               |
Runtime (pg.rs):          Send Query message -> Parse DataRow -> Build Map<String, String>
                               |
Returns to Mesh:          List<Map<String, String>>  (raw row maps)
                               |
Mesh code:                Manual struct construction from Map.get(row, "field")
```

### Current Struct-to-Row Mapping (deriving(Row))

```
Mesh code:                Pool.query_as(pool, sql, params, User.from_row)
                               |
Compiler generates:       FromRow__from_row__User function at compile time
                               |
Generated function:       For each field: mesh_row_from_row_get(row, "field_name")
                          Then: parse to correct type (String passthrough, Int/Float/Bool parse)
                          Then: Construct struct literal with all fields
                               |
Runtime (row.rs):         mesh_row_from_row_get extracts from Map, parse functions convert
                               |
Returns to Mesh:          Result<User, String> (typed struct or error)
```

### Current Pain Points (What the ORM Solves)

1. **Manual SQL strings everywhere**: 627 lines of queries.mpl with hand-written SQL
2. **Manual struct construction**: Every query function manually maps Map fields to struct fields
3. **No schema-query coupling**: Struct fields and SQL column lists are maintained separately
4. **No relationship support**: JOINs are hand-written, no eager loading
5. **No migration system**: Schema DDL is imperative `CREATE TABLE IF NOT EXISTS` in Mesh code
6. **No validation layer**: Input validation is scattered across handlers
7. **No query composition**: Each query is a monolithic SQL string, not composable

## Recommended Architecture

### ORM Layer Map

```
LAYER 1: Mesh User Code (.mpl files) -- ORM Library
  Orm/Schema.mpl        Schema definition functions, field/belongs_to/has_many metadata
  Orm/Query.mpl         Query builder: where, order_by, limit, select, join, preload
  Orm/Repo.mpl          Database operations: all, get, get_by, insert, update, delete
  Orm/Changeset.mpl     Validation, casting, constraint checking
  Orm/Migration.mpl     Migration runner, schema diffing, DDL generation

LAYER 2: Mesh Compiler (Rust additions)
  mesh-typeck           New deriving trait: deriving(Schema) -- registers table/column metadata
  mesh-codegen          Generate schema metadata functions: __schema_table__, __schema_fields__,
                        __schema_field_type__, __schema_associations__, etc.

LAYER 3: Mesh Runtime (Rust additions)
  db/orm.rs             mesh_orm_build_select, mesh_orm_build_insert, mesh_orm_build_update,
                        mesh_orm_build_delete, mesh_orm_build_where -- SQL generation from
                        metadata structs. Returns parameterized SQL + params list.
```

### System Diagram

```
                     USER CODE (Mesher services)
                     ==========================
                          |
           User |> Query.where("email", email)
                |> Query.limit(1)
                |> Repo.one(pool)
                          |
                     ORM LIBRARY (Mesh .mpl files)
                     ============================
                          |
          +---------+-----+------+-----------+
          |         |            |           |
      Schema    Query       Changeset    Repo
      Module    Builder     Module       Module
          |         |            |           |
          |    Builds query  Validates   Executes
          |    struct with   changes     query via
          |    clauses       before      Pool layer
          |         |        persist         |
          |         +--------+--+           |
          |                  |              |
          |           SQL Generation        |
          |           (Runtime Rust)        |
          |                  |              |
          +---> Schema       +----> Pool.query / Pool.execute
                Metadata              |
                (Compiler-            |
                 generated)    Existing PG Driver
                                      |
                                 PostgreSQL
```

### Component Boundaries

| Component | Responsibility | Language | Communicates With |
|-----------|---------------|----------|-------------------|
| `Orm.Schema` | Define table name, fields, types, associations, constraints | Mesh | Compiler (via deriving), Query, Repo, Migration |
| `Orm.Query` | Build composable query structs with where/order/limit/join/preload clauses | Mesh | Repo (passes query struct), Schema (reads metadata) |
| `Orm.Repo` | Execute queries against database, hydrate results to structs | Mesh | Pool (existing), Query (receives query structs), Schema (row mapping) |
| `Orm.Changeset` | Validate and cast data before persistence, track changes | Mesh | Repo (changesets passed to insert/update), Schema (field types) |
| `Orm.Migration` | Generate and execute DDL, track migration state | Mesh + Runtime | Pool (executes DDL), Schema (reads definitions) |
| `deriving(Schema)` | Generate compile-time metadata functions from struct definition | Rust (compiler) | Schema module (metadata consumed at runtime) |
| `db/orm.rs` | Build parameterized SQL from query metadata structs | Rust (runtime) | Repo module (called via FFI), PG driver (SQL execution) |

## Detailed Component Design

### 1. Schema Definition (deriving(Schema))

**Where it lives:** New compiler deriving trait + Mesh-level Schema module

**Design decision:** Schema metadata is generated at compile time via `deriving(Schema)` rather than defined via runtime DSL macros. This is because Mesh has no macro system, but has a proven deriving infrastructure that already generates `from_row`, `to_json`, and `from_json` functions from struct field metadata.

**What `deriving(Schema)` generates:**

```
# User code writes:
pub struct User do
  id :: String
  email :: String
  display_name :: String
  created_at :: String
end deriving(Json, Row, Schema)

# Compiler generates these functions (accessible as User.__schema_table__(), etc.):

fn __schema_table__() -> String = "users"

fn __schema_fields__() -> List<String> = ["id", "email", "display_name", "created_at"]

fn __schema_field_type__(field :: String) -> String
  # Returns "string", "int", "float", "bool", "option_string", etc.

fn __schema_primary_key__() -> String = "id"
```

**Table name convention:** Struct name lowercased + pluralized (User -> users, ApiKey -> api_keys). Overridable via a separate function or annotation approach if needed.

**Why compile-time metadata, not runtime DSL:**
- Mesh has no macro/metaprogramming system -- cannot generate code at compile time from DSL blocks
- The deriving system already processes struct fields and generates functions per field
- Compile-time metadata means the compiler can validate schema consistency (field types match DB types)
- Generated metadata functions are regular Mesh functions, callable from any module

**Compiler changes required:**
1. `mesh-typeck/src/infer.rs`: Add `"Schema"` to `valid_derives` array (currently `["Eq", "Ord", "Display", "Debug", "Hash", "Json", "Row"]`)
2. `mesh-typeck/src/infer.rs`: Register Schema trait impl for the type
3. `mesh-codegen/src/mir/lower.rs`: Add `generate_schema_metadata_struct()` method alongside existing `generate_from_row_struct()`, `generate_to_json_struct()`, etc.
4. Generated MIR functions return string constants and list literals -- no new MIR node types needed

**Association metadata:** Relationship information (belongs_to, has_many) needs a separate mechanism because struct fields alone do not encode relationship semantics. Two options:

**Option A -- Companion metadata functions (RECOMMENDED):**
```
# Written alongside the struct definition
pub struct Post do
  id :: String
  user_id :: String
  title :: String
  body :: String
end deriving(Json, Row, Schema)

# Separate metadata registration using module-level functions
pub fn __belongs_to__() -> List<Map<String, String>> = [
  %{"name" => "user", "module" => "User", "foreign_key" => "user_id"}
]

pub fn __has_many__() -> List<Map<String, String>> = [
  %{"name" => "comments", "module" => "Comment", "foreign_key" => "post_id"}
]
```

**Option B -- Convention functions in a Schema module:**
```
# In a dedicated schema module
module PostSchema do
  pub fn table() = "posts"
  pub fn belongs_to() = [("user", "User", "user_id")]
  pub fn has_many() = [("comments", "Comment", "post_id")]
end
```

Option A is recommended because it keeps metadata co-located with the struct and leverages the existing module system. The ORM library functions can look up these metadata functions by name convention.

### 2. Query Builder (Orm.Query)

**Where it lives:** Pure Mesh library code (Orm/Query.mpl)

**Design decision:** The query builder uses structs + pipe functions, not a service/actor pattern. Queries are pure data -- they should be immutable values that can be composed, stored, and passed around. Building a query does not perform I/O; only Repo functions execute queries.

**Query struct:**

```
pub struct Query do
  source :: String           # Table name ("users")
  select_fields :: List<String>  # ["id", "email"] or [] for all
  where_clauses :: List<String>  # Serialized where conditions
  where_params :: List<String>   # Parameter values
  order_fields :: List<String>   # ["created_at DESC", "name ASC"]
  limit_val :: Int               # 0 = no limit
  offset_val :: Int              # 0 = no offset
  join_clauses :: List<String>   # Serialized join specs
  preload_assocs :: List<String> # Association names to preload
  group_fields :: List<String>   # GROUP BY fields
end deriving(Json)
```

**Pipe-chain API:**

```
# Start from a schema type name
User
  |> Query.from()
  |> Query.where("email", "=", email)
  |> Query.where("status", "=", "active")
  |> Query.order_by("created_at", "desc")
  |> Query.limit(10)
  |> Query.select(["id", "email", "display_name"])
  |> Repo.all(pool)
```

**How Query.from() works:**

`Query.from()` does not need the actual struct type at runtime. It needs the table name. Two approaches:

**Approach A -- String-based (simpler, recommended for Phase 1):**
```
pub fn from(table :: String) -> Query do
  Query { source: table, select_fields: [], ... }
end

# Usage: Query.from("users") |> ...
```

**Approach B -- Schema-aware (requires metadata function lookup):**
```
# The query builder calls __schema_table__() on the module
# This requires the ORM to resolve module names to metadata functions
# Deferred to Phase 2 -- requires compiler support for function-as-value from module name
```

**SQL generation:** The Query struct is converted to parameterized SQL by a runtime Rust function. This is in the runtime (not the Mesh library) because:
1. String manipulation for SQL building is performance-sensitive
2. Proper parameter placeholder numbering ($1, $2, $3...) is mechanical
3. SQL injection prevention (escaping identifiers) is best done in Rust
4. The runtime already handles SQL string manipulation in pg.rs

**Runtime function signature:**
```rust
#[no_mangle]
pub extern "C" fn mesh_orm_build_select(query_ptr: *mut u8) -> *mut u8
// Takes serialized Query struct, returns MeshResult<(sql_string, params_list), error>
```

**Alternative considered -- pure Mesh SQL generation:**
Building SQL via string concatenation in Mesh is possible (the existing codebase does it extensively in queries.mpl). However, it leads to the exact code duplication the ORM is meant to eliminate. The runtime approach centralizes SQL generation logic in one place.

### 3. Repo Module (Orm.Repo)

**Where it lives:** Mesh library code (Orm/Repo.mpl), calling into runtime functions

**Design:** Stateless module with functions that take a pool handle and a query/changeset. Does not hold state. This matches Mesher's existing pattern where all query functions take `pool :: PoolHandle` as the first argument.

**Core operations:**

```
# Read operations
pub fn all(pool :: PoolHandle, query :: Query) -> List<Map<String, String>>!String
pub fn one(pool :: PoolHandle, query :: Query) -> Map<String, String>!String
pub fn get(pool :: PoolHandle, table :: String, id :: String) -> Map<String, String>!String
pub fn get_by(pool :: PoolHandle, table :: String, field :: String, value :: String) -> Map<String, String>!String

# Write operations (take changesets)
pub fn insert(pool :: PoolHandle, changeset :: Changeset) -> Map<String, String>!String
pub fn update(pool :: PoolHandle, changeset :: Changeset) -> Map<String, String>!String
pub fn delete(pool :: PoolHandle, table :: String, id :: String) -> Int!String

# Aggregate operations
pub fn count(pool :: PoolHandle, query :: Query) -> Int!String
pub fn exists(pool :: PoolHandle, query :: Query) -> Bool!String
```

**Return type decision:** Repo functions return `Map<String, String>` (raw row) rather than typed structs. This is because Mesh does not have dynamic dispatch or trait objects -- the Repo cannot call a generic `from_row` function for an arbitrary type T. The caller converts the Map to a struct using the existing deriving(Row) `from_row` function:

```
# Pattern for typed queries:
let row = Repo.get(pool, "users", user_id)?
let user = User.from_row(row)?

# Or the existing Pool.query_as pattern continues to work for custom queries
```

**Future optimization:** A `Repo.all_as(pool, query, User.from_row)` function could take the from_row callback directly, similar to the existing `Pool.query_as` pattern. This avoids the intermediate `List<Map<String, String>>` allocation.

**Data flow for Repo.all:**

```
1. Repo.all(pool, query)
2.   -> mesh_orm_build_select(query)          [Runtime: build SQL + params]
3.   -> Returns (sql_string, params_list)
4.   -> Pool.query(pool, sql_string, params)  [Existing pool infrastructure]
5.   -> Returns List<Map<String, String>>     [Existing PG driver]
6. Caller: List.map(rows, fn(row) do User.from_row(row) end)
```

### 4. Changeset Module (Orm.Changeset)

**Where it lives:** Pure Mesh library code (Orm/Changeset.mpl)

**Design:** A Changeset is a struct that wraps pending changes with validation state. It does not touch the database -- it is pure data transformation until passed to Repo.insert/Repo.update.

```
pub struct Changeset do
  table :: String                    # Target table
  fields :: Map<String, String>      # Field values to write
  changes :: Map<String, String>     # Only the changed fields (for updates)
  errors :: List<Map<String, String>>  # Validation errors: [{"field": "email", "message": "required"}]
  valid :: Bool                      # Quick check: are there errors?
  action :: String                   # "insert" or "update"
  primary_key_value :: String        # For updates: the ID of the row being updated
end deriving(Json)
```

**Changeset API:**

```
# Create a changeset for insert
pub fn cast(table :: String, params :: Map<String, String>, allowed :: List<String>) -> Changeset

# Validations (return new Changeset with errors appended if invalid)
pub fn validate_required(changeset :: Changeset, fields :: List<String>) -> Changeset
pub fn validate_length(changeset :: Changeset, field :: String, min :: Int, max :: Int) -> Changeset
pub fn validate_format(changeset :: Changeset, field :: String, pattern :: String) -> Changeset
pub fn validate_inclusion(changeset :: Changeset, field :: String, values :: List<String>) -> Changeset

# For updates: create changeset from existing data + new params
pub fn cast_update(table :: String, existing :: Map<String, String>, params :: Map<String, String>, allowed :: List<String>) -> Changeset
```

**Usage pattern:**

```
let changeset = Changeset.cast("users", params, ["email", "display_name", "password"])
  |> Changeset.validate_required(["email", "display_name", "password"])
  |> Changeset.validate_length("password", 8, 128)
  |> Changeset.validate_length("display_name", 1, 100)

if changeset.valid do
  Repo.insert(pool, changeset)
else
  Err(to_json(changeset.errors))
end
```

### 5. Migration System (Orm.Migration)

**Where it lives:** Mesh library code + CLI integration

**Design decision:** Migrations are Mesh functions (not separate SQL files) that call DDL helper functions. This keeps everything in the Mesh ecosystem and allows type checking of migration code.

**Migration structure:**

```
# migrations/20260216_create_users.mpl
pub fn up(pool :: PoolHandle) -> Int!String do
  Migration.create_table(pool, "users", [
    Migration.column("id", "uuid", ["primary_key", "default gen_random_uuid()"]),
    Migration.column("email", "text", ["not_null", "unique"]),
    Migration.column("display_name", "text", ["not_null"]),
    Migration.column("password_hash", "text", ["not_null"]),
    Migration.column("created_at", "timestamptz", ["not_null", "default now()"])
  ])?
  Migration.create_index(pool, "users", ["email"], ["unique"])?
  Ok(0)
end

pub fn down(pool :: PoolHandle) -> Int!String do
  Migration.drop_table(pool, "users")?
  Ok(0)
end
```

**Migration tracking:** A `_mesh_migrations` table stores applied migration names + timestamps. The migration runner:
1. Reads all migration modules from a `migrations/` directory
2. Queries `_mesh_migrations` for already-applied migrations
3. Runs unapplied migrations in filename order within a transaction
4. Records each successful migration in `_mesh_migrations`

**CLI integration:** `meshc migrate` (or `mesh migrate`) command that:
1. Discovers migration files in the project
2. Connects to the database
3. Runs pending migrations

This requires a new subcommand in the `meshc` binary. The existing `meshc` main.rs already handles `build` and `fmt` subcommands.

**Alternative considered -- SQL migration files:**
Plain .sql files are simpler but lose Mesh type checking and cannot use Mesh functions. Since migrations are run infrequently and debuggability matters more than raw performance, keeping them as Mesh code is worth the extra compilation step.

### 6. Relationship and Preloading

**Where it lives:** Orm.Query (preload specification) + Orm.Repo (preload execution)

**Design:** Relationships are metadata-driven. The ORM reads relationship metadata from companion functions (see Schema section) and generates appropriate JOINs or separate queries.

**Preload strategy -- Separate queries (not JOINs):**

```
# Load a user with their posts and each post's comments:
let user = Repo.get(pool, "users", user_id)?
let posts = Query.from("posts")
  |> Query.where("user_id", "=", user_id)
  |> Repo.all(pool)?
# For each post, load comments:
let posts_with_comments = List.map(posts, fn(post) do
  let comments = Query.from("comments")
    |> Query.where("post_id", "=", Map.get(post, "id"))
    |> Repo.all(pool)?
  Map.put(post, "_comments", to_json(comments))
end)
```

**Why separate queries, not JOINs for preloading:**
1. Mesh does not have dynamic struct extension -- cannot add a `posts` field to a User struct at runtime
2. Separate queries avoid the M*N row explosion from JOINing multiple has_many associations
3. Ecto uses the same strategy (separate queries per association by default)
4. Simpler to implement, easier to debug, more predictable performance

**Preload API (Phase 2):**

```
# Automatic preloading via Repo helper
let user = Repo.get(pool, "users", user_id)?
let user_with_posts = Repo.preload(pool, user, "User", ["posts"])?
# Returns a Map with "_posts" key containing the loaded association

# Nested preloading
let user_with_all = Repo.preload(pool, user, "User", ["posts.comments", "org_memberships"])?
```

**Data representation for loaded associations:**

Since Mesh structs are statically typed and cannot have dynamic fields, preloaded associations are stored as JSON string values in a wrapper Map or as separate variables. The recommended pattern:

```
# Instead of trying to attach posts to a User struct,
# return a tuple or map with both:
let user = Repo.get(pool, "users", user_id)?
let posts = Repo.preload_assoc(pool, "User", "posts", user_id)?
# user :: Map<String, String>, posts :: List<Map<String, String>>
```

### 7. Connection to Existing Pool/Pg Layer

**Zero changes to the existing pool and PG driver.** The ORM builds SQL strings and parameter lists, then calls the same `Pool.query` and `Pool.execute` functions that Mesher already uses.

```
ORM:          mesh_orm_build_select(query_struct) -> (sql, params)
Existing:     Pool.query(pool, sql, params)       -> List<Map<String, String>>
Existing:     Pool.execute(pool, sql, params)      -> Int
Existing:     Pool.query_as(pool, sql, params, from_row_fn) -> List<Struct>
```

The only runtime addition is `db/orm.rs` for SQL generation. It does not touch `db/pool.rs` or `db/pg.rs`.

## Data Flow Diagrams

### Query Execution Flow (Read)

```
Mesh User Code                    ORM Library                  Runtime (Rust)              PostgreSQL
=============                    ===========                  ==============              ==========

User
  |> Query.from("users")         Creates Query struct
  |> Query.where("email", v)     Appends to where_clauses
  |> Query.limit(1)              Sets limit_val
  |> Repo.one(pool)              --->
                                 mesh_orm_build_select(query) -->
                                                                  Builds SQL:
                                                                  "SELECT * FROM users
                                                                   WHERE email = $1
                                                                   LIMIT 1"
                                                                  Returns (sql, ["alice@..."])
                                 <---
                                 Pool.query(pool, sql, params) -->
                                                                  mesh_pool_query -->
                                                                  checkout -> pg_query -> checkin
                                                                                          Execute SQL
                                                                                          <-- DataRows
                                                                  <-- List<Map<String,String>>
                                 <---
                                 Return first row or Err
<--- Map<String, String>
User.from_row(row)?
<--- User struct
```

### Insert Flow (Write)

```
Mesh User Code                    ORM Library                  Runtime (Rust)              PostgreSQL
=============                    ===========                  ==============              ==========

Changeset.cast("users",
  params, allowed)               Creates Changeset struct
  |> validate_required(...)      Validates fields
  |> validate_length(...)        Appends errors if invalid

Repo.insert(pool, changeset) --> Check changeset.valid
                                 If invalid: return Err(errors)
                                 mesh_orm_build_insert(cs) ------>
                                                                  Builds SQL:
                                                                  "INSERT INTO users
                                                                   (email, display_name, ...)
                                                                   VALUES ($1, $2, ...)
                                                                   RETURNING *"
                                                                  Returns (sql, params)
                                 <---
                                 Pool.query(pool, sql, params) -->
                                                                  Execute INSERT
                                                                  <-- RETURNING row
                                 <---
                                 Return inserted row Map
<--- Map<String, String>
```

## Patterns to Follow

### Pattern 1: Struct-as-Query (Immutable Query Composition)

**What:** Queries are immutable struct values. Each pipe operation returns a new Query with the modification applied. This is the established Mesh pattern (see HTTP.router() pipe chain in main.mpl).

**When:** All query building.

**Example:**
```
# Each operation returns a new Query, not mutating the original
let base_query = Query.from("issues")
  |> Query.where("project_id", "=", project_id)

# Reuse base for different views
let unresolved = base_query |> Query.where("status", "=", "unresolved")
let resolved = base_query |> Query.where("status", "=", "resolved")
```

### Pattern 2: Pool-First-Arg Convention

**What:** All functions that touch the database take `pool :: PoolHandle` as the first argument. This is the universal convention in all 627 lines of Mesher's queries.mpl and all service modules.

**When:** Any Repo function.

**Example:**
```
# Consistent with existing codebase
pub fn all(pool :: PoolHandle, query :: Query) -> List<Map<String, String>>!String
pub fn get(pool :: PoolHandle, table :: String, id :: String) -> Map<String, String>!String
```

### Pattern 3: Result-Error Propagation

**What:** All fallible operations return `T!String` (Result<T, String>) and are composable with the `?` operator. This is the established pattern throughout Mesher.

**When:** Any operation that can fail.

**Example:**
```
pub fn get_user_with_posts(pool :: PoolHandle, user_id :: String) -> Map<String, String>!String do
  let user = Repo.get(pool, "users", user_id)?
  let posts = Query.from("posts")
    |> Query.where("user_id", "=", user_id)
    |> Repo.all(pool)?
  # Combine...
  Ok(user)
end
```

### Pattern 4: Generated Metadata Functions (deriving Pattern)

**What:** Compile-time code generation produces named functions with predictable mangled names. The ORM follows the exact same pattern as `deriving(Json)` (generates `to_json__User`, `from_json__User`) and `deriving(Row)` (generates `FromRow__from_row__User`).

**When:** Schema metadata generation.

**Example:**
```
# deriving(Schema) on struct User generates:
# Schema__table__User() -> "users"
# Schema__fields__User() -> ["id", "email", "display_name", "created_at"]
# Schema__primary_key__User() -> "id"
```

## Anti-Patterns to Avoid

### Anti-Pattern 1: Runtime Schema Reflection

**What:** Discovering struct field names at runtime by introspecting memory layout or using dynamic lookups.

**Why bad:** Mesh has no runtime reflection. All type information is erased during compilation. The GC does not store type metadata. Attempting runtime reflection would require a parallel type metadata system.

**Instead:** Generate all metadata at compile time via deriving(Schema).

### Anti-Pattern 2: Dynamic Return Types

**What:** A generic `Repo.all<T>()` that returns `List<T>` for any schema type T.

**Why bad:** Mesh uses monomorphization, not dynamic dispatch. There are no trait objects. The generic version would need to be monomorphized for every schema type, which requires the compiler to see the concrete type at every call site. This works for simple generics but breaks down for a Repo module that needs to work with any schema.

**Instead:** Return `List<Map<String, String>>` from Repo functions and let callers apply their specific `from_row` function. This matches how the existing `Pool.query` works.

### Anti-Pattern 3: SQL String Building in Mesh

**What:** Building SQL by concatenating strings in Mesh code (e.g., `"SELECT " <> fields <> " FROM " <> table <> " WHERE " <> conditions`).

**Why bad:** This is exactly what queries.mpl does today, producing 627 lines of brittle, repetitive code. It is error-prone (missing spaces, wrong parameter numbering), hard to compose, and impossible to validate.

**Instead:** Build SQL in the Rust runtime from structured Query data. One place to handle SQL generation correctly.

### Anti-Pattern 4: Actor/Service for Query Building

**What:** Making the query builder a stateful service (like OrgService, ProjectService) that accumulates query state via message passing.

**Why bad:** Query building is a pure computation. It does not need concurrency, state management, or fault tolerance. Making it a service adds latency (message passing), complexity (service lifecycle), and breaks composition (cannot pass query values across pipe chains).

**Instead:** Use plain structs and pure functions. Queries are values, not processes.

## New vs. Modified Components

### New Components

| Component | Type | Location | Purpose |
|-----------|------|----------|---------|
| `Orm/Schema.mpl` | Mesh library | Mesher or separate ORM package | Schema helper functions (from, field metadata lookup) |
| `Orm/Query.mpl` | Mesh library | Mesher or separate ORM package | Query builder struct + pipe functions |
| `Orm/Repo.mpl` | Mesh library | Mesher or separate ORM package | Database operation wrappers |
| `Orm/Changeset.mpl` | Mesh library | Mesher or separate ORM package | Validation and casting |
| `Orm/Migration.mpl` | Mesh library | Mesher or separate ORM package | Migration DDL helpers and runner |
| `db/orm.rs` | Rust runtime | `crates/mesh-rt/src/db/` | SQL generation from query structs |
| `generate_schema_metadata_struct()` | Rust compiler | `crates/mesh-codegen/src/mir/lower.rs` | MIR generation for schema metadata |

### Modified Components

| Component | Change | Risk |
|-----------|--------|------|
| `mesh-typeck/src/infer.rs` | Add "Schema" to valid_derives, register trait impl | LOW -- follows exact pattern of "Json" and "Row" |
| `mesh-codegen/src/mir/lower.rs` | Add schema metadata generation alongside existing deriving code | LOW -- isolated addition to existing deriving switch |
| `mesh-codegen/src/mir/lower.rs` | Register new runtime functions (mesh_orm_*) in known_functions | LOW -- additive only |
| `mesh-rt/src/db/mod.rs` | Add `pub mod orm;` | LOW -- new module, no existing code changed |
| `meshc/src/main.rs` | Add `migrate` subcommand | LOW -- additive CLI path |

### Unchanged Components

| Component | Why Unchanged |
|-----------|---------------|
| `mesh-lexer` | No new tokens needed |
| `mesh-parser` | No new grammar -- deriving(Schema) uses existing deriving clause syntax |
| `mesh-rt/src/db/pg.rs` | ORM builds SQL, PG driver executes it -- no changes needed |
| `mesh-rt/src/db/pool.rs` | ORM calls Pool.query/Pool.execute as-is |
| `mesh-rt/src/db/row.rs` | Row parsing works unchanged -- deriving(Row) continues to work |

## Build Order (Implementation Phases)

The build order is driven by dependencies: each phase must have its prerequisites complete before starting.

### Phase 1: Schema Metadata (Foundation)

**Prerequisites:** None (builds on existing deriving infrastructure)

**What:** Add `deriving(Schema)` to the compiler. When a struct has `deriving(Schema)`, generate metadata functions that return the table name, field list, field types, and primary key.

**Deliverables:**
1. Add "Schema" to valid_derives in mesh-typeck
2. Implement `generate_schema_metadata_struct()` in mesh-codegen
3. Register generated functions in known_functions
4. E2E test: struct with deriving(Schema) produces callable metadata functions

**Why first:** Everything else in the ORM depends on schema metadata. The Query builder needs table names. The Repo needs field lists. The Migration system needs field types. Without this, all ORM library code would hardcode strings.

### Phase 2: SQL Generation Runtime

**Prerequisites:** Phase 1 (needs to know what Query struct looks like)

**What:** Implement `db/orm.rs` in the Mesh runtime with functions to build parameterized SQL from structured input. Functions: `mesh_orm_build_select`, `mesh_orm_build_insert`, `mesh_orm_build_update`, `mesh_orm_build_delete`.

**Deliverables:**
1. New `crates/mesh-rt/src/db/orm.rs` module
2. SQL generation for SELECT with WHERE, ORDER BY, LIMIT, OFFSET, GROUP BY
3. SQL generation for INSERT with RETURNING
4. SQL generation for UPDATE with WHERE and RETURNING
5. SQL generation for DELETE with WHERE
6. Parameter numbering ($1, $2, ...) and identifier quoting
7. Rust unit tests for all SQL generation paths

**Why second:** The Repo module needs to call these functions. Building them first allows Repo development to proceed with a working SQL backend.

### Phase 3: Query Builder + Repo (Core API)

**Prerequisites:** Phase 1 (schema metadata), Phase 2 (SQL generation)

**What:** Implement the Mesh-level ORM library: Query struct, pipe-chain builder functions, and Repo module with all CRUD operations.

**Deliverables:**
1. `Orm/Query.mpl` -- Query struct + from/where/order_by/limit/offset/select/group_by
2. `Orm/Repo.mpl` -- all/one/get/get_by/insert_raw/update_raw/delete/count/exists
3. Integration test: build query, execute via Repo, verify results
4. Mesher smoke test: rewrite one simple query (e.g., get_org) to use ORM

**Why third:** This is the core user-facing API. It requires both schema metadata (Phase 1) and SQL generation (Phase 2) to function.

### Phase 4: Changesets (Validation Layer)

**Prerequisites:** Phase 3 (Repo for insert/update)

**What:** Implement the Changeset module for validating and casting data before persistence. Connect Repo.insert/Repo.update to accept changesets.

**Deliverables:**
1. `Orm/Changeset.mpl` -- cast, validate_required, validate_length, validate_format, validate_inclusion
2. Repo.insert and Repo.update accept Changeset structs
3. Validation error propagation (changeset.valid check)
4. Integration test: invalid changeset returns errors, valid changeset inserts

**Why fourth:** Changesets enhance Repo operations. They are not needed for basic querying, so they can come after the core Repo is working.

### Phase 5: Relationships and Preloading

**Prerequisites:** Phase 3 (Query + Repo), Phase 1 (Schema metadata for associations)

**What:** Implement relationship metadata, association queries, and preloading.

**Deliverables:**
1. Convention for declaring belongs_to/has_many/has_one metadata
2. `Repo.preload_assoc` for loading a single association
3. `Repo.preload` for loading multiple associations on a result
4. Nested preloading support (posts.comments)
5. Many-to-many through join table support

**Why fifth:** Relationships are the most complex ORM feature and depend on everything else working correctly. They require working queries, schema metadata, and Repo operations.

### Phase 6: Migration System

**Prerequisites:** Phase 2 (SQL generation for DDL), Phase 1 (Schema metadata)

**What:** Implement migration infrastructure: DDL helper functions, migration tracking table, migration runner, and CLI integration.

**Deliverables:**
1. `Orm/Migration.mpl` -- create_table, drop_table, add_column, remove_column, create_index, etc.
2. Migration tracking (_mesh_migrations table)
3. Migration runner (discover, sort, run pending)
4. `meshc migrate` CLI subcommand
5. `meshc migrate rollback` for down migrations
6. Migration generation: `meshc migrate generate create_users`

**Why sixth:** Migrations are operationally important but not required for query/data operations. They can be developed in parallel with Phase 5 if resources allow.

### Phase 7: Mesher Rewrite (Validation)

**Prerequisites:** All of Phases 1-6

**What:** Rewrite Mesher's entire storage layer to use the ORM. This validates every ORM feature against a real application.

**Deliverables:**
1. Convert all 11 type structs to use deriving(Schema)
2. Replace storage/queries.mpl (627 lines) with ORM query calls
3. Replace storage/schema.mpl (82 lines) with migration files
4. Replace storage/writer.mpl with ORM insert
5. Update all service modules to use Repo instead of raw queries
6. Update all API handlers that use raw Map results
7. Verify all existing functionality works identically

**Estimated reduction:** 627 lines of queries.mpl -> ~100-150 lines of ORM calls. 82 lines of schema.mpl -> ~150 lines of migration files (more structured, but declarative).

## Mesher DB Layer Refactoring Plan

### Current Structure (Before ORM)

```
mesher/
  types/
    user.mpl          pub struct User ... end deriving(Json, Row)
    project.mpl       pub struct Organization, Project, ApiKey ... end deriving(Json, Row)
    event.mpl         pub struct Event, EventPayload, StackFrame ... end deriving(Json, Row)
    issue.mpl         pub struct Issue ... end deriving(Json, Row)
    alert.mpl         pub struct AlertRule, Alert ... end deriving(Json, Row)
  storage/
    schema.mpl        82 lines: create_schema(), create_partition(), create_partitions_ahead()
    queries.mpl       627 lines: 50+ functions with raw SQL strings
    writer.mpl        21 lines: insert_event() with raw SQL
  services/
    org.mpl           OrgService delegates to Storage.Queries
    project.mpl       ProjectService delegates to Storage.Queries
    user.mpl          UserService delegates to Storage.Queries
    event_processor   EventProcessor uses Storage.Queries
    writer.mpl        StorageWriter batches and calls Storage.Writer
    retention.mpl     RetentionCleaner calls Storage.Queries
```

### Target Structure (After ORM)

```
mesher/
  types/
    user.mpl          pub struct User ... end deriving(Json, Row, Schema)   # ADD Schema
    project.mpl       pub struct Organization, Project, ApiKey ... end deriving(Json, Row, Schema)
    event.mpl         pub struct Event ... end deriving(Json, Row, Schema)
    issue.mpl         pub struct Issue ... end deriving(Json, Row, Schema)
    alert.mpl         pub struct AlertRule, Alert ... end deriving(Json, Row, Schema)
  orm/                # NEW: ORM library
    schema.mpl        Schema helper functions
    query.mpl         Query struct + builder functions
    repo.mpl          Database operations
    changeset.mpl     Validation
    migration.mpl     Migration helpers
  migrations/         # NEW: Migration files
    001_create_organizations.mpl
    002_create_users.mpl
    003_create_org_memberships.mpl
    004_create_sessions.mpl
    005_create_projects.mpl
    006_create_api_keys.mpl
    007_create_issues.mpl
    008_create_events.mpl
    009_create_alert_rules.mpl
    010_create_alerts.mpl
    011_add_retention_settings.mpl
  storage/
    queries.mpl       REMOVED (replaced by ORM calls in services)
    schema.mpl        REMOVED (replaced by migrations)
    writer.mpl        SIMPLIFIED (uses Repo.insert instead of raw SQL)
  services/
    org.mpl           Uses Repo.get/Repo.all instead of Storage.Queries
    project.mpl       Uses Repo.get/Repo.all instead of Storage.Queries
    user.mpl          Uses Repo.get/Changeset/Repo.insert
    ...
```

### Specific Refactoring Examples

**Before (raw SQL):**
```
pub fn get_org(pool :: PoolHandle, id :: String) -> Organization!String do
  let rows = Pool.query(pool, "SELECT id::text, name, slug, created_at::text FROM organizations WHERE id = $1::uuid", [id])?
  if List.length(rows) > 0 do
    let row = List.head(rows)
    Ok(Organization { id: Map.get(row, "id"), name: Map.get(row, "name"), slug: Map.get(row, "slug"), created_at: Map.get(row, "created_at") })
  else
    Err("not found")
  end
end
```

**After (ORM):**
```
pub fn get_org(pool :: PoolHandle, id :: String) -> Organization!String do
  let row = Repo.get(pool, "organizations", id)?
  Organization.from_row(row)
end
```

**Before (complex filtered query):**
```
pub fn list_issues_filtered(pool :: PoolHandle, project_id :: String, status :: String, level :: String, assigned_to :: String, cursor :: String, cursor_id :: String, limit_str :: String) -> List<Map<String, String>>!String do
  if String.length(cursor) > 0 do
    let sql = "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) AND ($3 = '' OR level = $3) AND ($4 = '' OR assigned_to = $4::uuid) AND (last_seen, id) < ($5::timestamptz, $6::uuid) ORDER BY last_seen DESC, id DESC LIMIT $7::int"
    Pool.query(pool, sql, [project_id, status, level, assigned_to, cursor, cursor_id, limit_str])
  else
    # ... 6 more lines
  end
end
```

**After (ORM):**
```
pub fn list_issues_filtered(pool :: PoolHandle, project_id :: String, status :: String, level :: String, assigned_to :: String, cursor :: String, cursor_id :: String, limit_str :: String) -> List<Map<String, String>>!String do
  let query = Query.from("issues")
    |> Query.where("project_id", "=", project_id)
    |> Query.where_if(status != "", "status", "=", status)
    |> Query.where_if(level != "", "level", "=", level)
    |> Query.where_if(assigned_to != "", "assigned_to", "=", assigned_to)
    |> Query.where_if(cursor != "", "(last_seen, id) <", "", cursor <> "," <> cursor_id)
    |> Query.order_by("last_seen", "desc")
    |> Query.order_by("id", "desc")
    |> Query.limit_str(limit_str)
  Repo.all(pool, query)
end
```

### Queries That May Remain as Raw SQL

Some Mesher queries are too complex or PostgreSQL-specific for a general ORM:

1. **Event partitioning** (`create_partition`, `create_partitions_ahead`): Uses dynamic DDL with date arithmetic. Keep as raw SQL.
2. **Spike detection** (`check_volume_spikes`): Complex correlated subquery with interval arithmetic. Keep as raw SQL.
3. **Extract event fields** (`extract_event_fields`): Server-side JSONB extraction with CASE expressions. Keep as raw SQL.
4. **Insert event** (`insert_event`): Uses `SELECT ... FROM (SELECT $4::jsonb) AS sub` for JSONB extraction. Consider ORM insert for simple version, keep raw for JSONB.

The ORM provides a `Repo.raw_query` escape hatch for these:
```
pub fn raw_query(pool :: PoolHandle, sql :: String, params :: List<String>) -> List<Map<String, String>>!String do
  Pool.query(pool, sql, params)
end
```

## Scalability Considerations

| Concern | At Mesher Scale (~10 tables) | At 50 Tables | At 200+ Tables |
|---------|-------------------------------|--------------|----------------|
| Schema metadata | Negligible compile time | Negligible | May add ~1s to compile |
| Query building | Struct allocation per query | Same -- structs are cheap | Same |
| SQL generation | <1ms per query in runtime | Same | Same |
| Migration management | Linear scan of files | Same (migrations run once) | Index on tracking table |
| Preloading N+1 | Separate query per assoc | Batched IN queries | Batched IN queries |

## Sources

- Direct codebase analysis of all files in `/Users/sn0w/Documents/dev/snow/crates/` and `/Users/sn0w/Documents/dev/snow/mesher/`
- [Ecto documentation (v3.13.5)](https://hexdocs.pm/ecto/Ecto.html) -- four-module architecture reference
- [Ecto.Schema documentation](https://hexdocs.pm/ecto/Ecto.Schema.html) -- schema design patterns
- [Ecto/Elixir database operations guide](https://oneuptime.com/blog/post/2026-01-26-elixir-ecto-database/view) -- practical Ecto patterns
- [ORMs vs Query Builders comparison](https://neon.com/blog/orms-vs-query-builders-for-your-typescript-application) -- architectural tradeoffs
- [Prisma Data Guide: SQL vs ORMs vs Query Builders](https://www.prisma.io/dataguide/types/relational/comparing-sql-query-builders-and-orms) -- approach comparison
