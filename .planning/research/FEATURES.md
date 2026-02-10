# Feature Landscape

**Domain:** Database & serialization features for compiled programming language (Snow v2.0)
**Researched:** 2026-02-10
**Confidence:** HIGH (all features are well-studied across Go, Rust, Elixir; existing codebase patterns thoroughly reviewed)

---

## Current State in Snow

Before defining features, here is what already exists and directly affects this milestone:

**Working (infrastructure these features build on):**
- Full compiler pipeline: lexer -> parser -> HM type inference -> MIR lowering -> LLVM codegen
- `deriving(Eq, Ord, Display, Debug, Hash)` auto-generates trait implementations from struct field metadata
- MIR lowering checks `derive_list` for trait names, then calls `self.generate_*_struct(name, fields)` to emit synthetic functions
- `SnowJson` tagged union runtime type (tag: Null/Bool/Number/Str/Array/Object) in `snow-rt/src/json.rs`
- `snow_json_from_int`, `snow_json_from_float`, `snow_json_from_bool`, `snow_json_from_string` -- build SnowJson values from Snow primitives
- `snow_json_encode(SnowJson) -> String` and `snow_json_parse(String) -> Result<SnowJson, String>` -- roundtrip serialization
- HTTP server with actor-per-connection model (`tiny_http` + M:N scheduler coroutines)
- `SnowRouter` with exact match and `/*` wildcard patterns in `snow-rt/src/http/router.rs`
- `SnowHttpRequest` struct: method, path, body, query_params (Map), headers (Map) in `snow-rt/src/http/server.rs`
- Module-qualified access: `HTTP.route(r, "/path", handler)` resolves via `STDLIB_MODULES` list and `map_builtin_name` function
- Known-function registration pattern: `self.known_functions.insert("snow_*", MirType::FnPtr(...))` in MIR lowering
- Intrinsic declaration pattern in `snow-codegen/src/codegen/intrinsics.rs`
- `Result<T, E>` with Ok/Err, `Option<T>` with Some/None, pattern matching, `?` operator
- Collections: List, Map, Set with 20+ operations each
- Actor runtime with typed `Pid<M>`, blocking I/O model (like BEAM NIFs)

**What this milestone adds:**
- `deriving(Json)` for struct-aware JSON encode/decode
- SQLite driver (new C FFI dependency: sqlite3)
- PostgreSQL driver (new C FFI dependency: libpq)
- Parameterized queries for both database drivers
- HTTP path parameters (`:param` segment matching)
- HTTP middleware (handler wrapping pattern)

---

## Table Stakes

Features users expect. Missing = product feels incomplete.

### 1. Struct-Aware JSON Serde via `deriving(Json)`

Every modern language provides automatic JSON serialization from struct definitions. Rust has `#[derive(Serialize, Deserialize)]` via serde. Go has `json.Marshal`/`json.Unmarshal` with struct tags. Elixir has `@derive Jason.Encoder`. Snow already has the deriving infrastructure for 5 traits and the SnowJson runtime type. This feature connects them.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `deriving(Json)` generates `json_encode` for structs | Universal: Rust serde, Go struct tags, Elixir `@derive Jason.Encoder`. Every language auto-generates serializers from struct metadata. | **High** | New synthetic function `Json__encode__StructName(self) -> String`. Pattern: iterate fields at codegen time, call `snow_json_from_*` per field, build SnowMap of field_name->SnowJson, wrap as SnowJson object, call `snow_json_encode`. |
| Struct encode produces JSON object with field names as keys | `Point { x: 1, y: 2 }` must produce `{"x":1,"y":2}`. Universal across Go, Rust, Elixir. | Med | Field names and types already available in `MirStructDef.fields`. Codegen emits `snow_json_from_int(field_val)` for Int fields, etc. |
| `deriving(Json)` generates `json_decode` from JSON string | Go `json.Unmarshal`, Rust `serde_json::from_str`, Elixir `Poison.decode!(s, as: %T{})`. Decoding is the harder half but expected. | **High** | New synthetic function `Json__decode__StructName(json_str) -> Result<StructName, String>`. Must parse JSON, extract SnowJson object, look up each field by name, cast to correct type, construct struct. |
| Nested struct encode/decode | If `User` has `address :: Address` and both derive Json, encoding must recurse. | Med | Generated `json_encode` calls the inner type's `json_encode` when the field type is a struct with Json derived. Decode calls the inner type's `json_decode`. |
| List field support in serde | A struct with `tags :: List<String>` must encode as a JSON array. | Med | Need new runtime helper `snow_json_from_list(list, element_encoder_fn)` that maps over elements. For decode: `snow_json_to_list(json_array, element_decoder_fn)`. |
| Option field handling | `email :: Option<String>` encodes as the value or `null`, decodes missing/null as `None`. | Med | Standard across all three reference languages. Encode: `Some(v)` -> encode v, `None` -> `snow_json_from_null()`. Decode: JSON null or missing key -> `None`. |
| Error messages on decode failure | When JSON doesn't match the struct shape, the error message must say which field failed and why. | Low | `"Expected field 'age' to be a number, got string"` -- quality error messages are table stakes for developer experience. |

**Expected API surface:**

```snow
struct User do
  name :: String
  age :: Int
  email :: Option<String>
end deriving(Eq, Json)

fn main() do
  let user = User { name: "Alice", age: 30, email: Some("alice@example.com") }

  # Encode: struct -> JSON string
  let json_str = JSON.encode(user)
  # => '{"name":"Alice","age":30,"email":"alice@example.com"}'

  # Decode: JSON string -> Result<User, String>
  let result = User.from_json(json_str)
  case result do
    Ok(u) -> println("Got user: ${u.name}")
    Err(e) -> println("Parse error: ${e}")
  end
end
```

**Implementation approach:**

The compiler generates two MIR functions per struct that derives Json:

1. `Json__encode__User(self: User) -> String` -- Extracts each field, converts to SnowJson via `snow_json_from_*`, builds a SnowMap, wraps as SnowJson Object, calls `snow_json_encode`.

2. `Json__decode__User(input: String) -> Result<User, String>` -- Calls `snow_json_parse(input)` to get SnowJson. Checks it's an Object (tag 5). Extracts field values by name from the SnowMap. Casts each to the expected Snow type. Constructs the struct. Returns `Ok(user)` or `Err(message)`.

The `JSON.encode(value)` call dispatches to the appropriate `Json__encode__T` based on the argument type. This requires the MIR lowerer to resolve the call based on the argument's type (similar to how Display trait dispatch works for `${expr}` interpolation).

**What NOT to include:**
- Field renaming attributes (`@json_name("user_name")`) -- requires annotation/attribute syntax Snow doesn't have yet
- Custom serializers per field -- requires plugin system
- JSON Schema validation -- separate concern
- Streaming JSON parsing -- overkill for this stage

**Confidence:** HIGH -- the deriving infrastructure is proven (5 traits), the SnowJson runtime type exists, the `snow_json_from_*` helpers exist. The main work is connecting them in MIR codegen.

**Dependencies:** Existing deriving system, existing SnowJson runtime, existing `snow_json_from_*` and `snow_json_encode`/`snow_json_parse`.

---

### 2. SQLite Driver

SQLite is the most common embedded database. Every language provides a way to interact with it. Go has `database/sql` + `go-sqlite3`. Rust has `rusqlite`. Elixir has `Ecto.Adapters.SQLite3`. The SQLite C API is remarkably minimal: `sqlite3_open`, `sqlite3_prepare_v2`, `sqlite3_step`, `sqlite3_column_*`, `sqlite3_finalize`, `sqlite3_close`.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Sqlite.open(path)` returns `Result<SqliteConn, String>` | Every SQLite wrapper starts with open. Universal first step. | Low | Wraps `sqlite3_open()`. Returns opaque handle as Ptr. Error via `sqlite3_errmsg`. |
| `Sqlite.execute(conn, sql, params)` for INSERT/UPDATE/DELETE | Non-SELECT execution returning affected row count. Go: `db.Exec()`, Rust: `conn.execute()`. | Med | Calls `sqlite3_prepare_v2`, binds params via `sqlite3_bind_text`, `sqlite3_step`, `sqlite3_finalize`. Returns `Result<Int, String>`. |
| `Sqlite.query(conn, sql, params)` for SELECT | Must return rows. Go: `db.Query()` + `rows.Scan()`, Rust: `conn.query_map()`. | **Med-High** | Prepare/step/column cycle. Returns `Result<List<Map<String, String>>, String>`. All values as strings initially -- matches Go's vanilla `database/sql` pattern. |
| Parameterized queries with `?` placeholders | SQL injection prevention is non-negotiable. Every database driver uses parameterized queries. SQLite native: `?`. | Med | Params as `List<String>`. Runtime iterates and calls `sqlite3_bind_text(stmt, idx+1, value, -1, SQLITE_TRANSIENT)` for each. |
| `Sqlite.close(conn)` | Resource cleanup. Universal. | Low | Calls `sqlite3_close()`. |
| Error handling returns Result | All operations that can fail must return `Result<T, String>`. | Low | Established pattern in Snow. Use `sqlite3_errmsg(db)` for error messages. |

**Expected API surface:**

```snow
fn main() do
  let conn = Sqlite.open("app.db")?
  Sqlite.execute(conn, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)", [])?
  Sqlite.execute(conn, "INSERT INTO users (name, age) VALUES (?, ?)", ["Alice", "30"])?
  let rows = Sqlite.query(conn, "SELECT name, age FROM users WHERE age > ?", ["25"])?
  # rows :: List<Map<String, String>>
  List.each(rows, fn(row) do
    let name = Map.get(row, "name")
    println("User: ${name}")
  end)
  Sqlite.close(conn)
end
```

**Implementation approach:**

New Rust runtime module `snow-rt/src/db/sqlite.rs` that wraps the sqlite3 C API:
1. Link `sqlite3` as a C dependency (use the `rusqlite` crate's bundled sqlite3, or link system sqlite3)
2. Expose `snow_sqlite_open(path: *const SnowString) -> *mut SnowResult`
3. Expose `snow_sqlite_execute(conn: *mut u8, sql: *const SnowString, params: *mut u8) -> *mut SnowResult`
4. Expose `snow_sqlite_query(conn: *mut u8, sql: *const SnowString, params: *mut u8) -> *mut SnowResult`
5. Expose `snow_sqlite_close(conn: *mut u8)`
6. Register as `Sqlite` module in STDLIB_MODULES, map function names in `map_builtin_name`
7. Declare intrinsics in `intrinsics.rs`, register types in `known_functions`

**What NOT to include:**
- Connection pooling -- single connection is fine for v2.0
- Prepared statement caching -- user creates/discards connections
- Transaction API (`begin`/`commit`/`rollback` as typed API) -- users call `Sqlite.execute(conn, "BEGIN", [])` manually
- Automatic struct-to-row mapping -- return `List<Map<String, String>>`
- Async/non-blocking I/O -- blocking is fine in actor context (same as HTTP)

**Confidence:** HIGH -- SQLite C API is well-documented and stable. The Rust runtime already links C dependencies (via cargo). The pattern for adding new stdlib modules is established.

**Dependencies:** C FFI linkage for sqlite3 library. No dependencies on other v2.0 features.

---

### 3. PostgreSQL Driver

PostgreSQL is the most common production database. Go has `pgx`. Rust has `rust-postgres` and `sqlx`. Elixir has `Ecto.Adapters.Postgres` via `postgrex`. The C interface is `libpq`: `PQconnectdb`, `PQexecParams`, `PQgetvalue`, `PQclear`, `PQfinish`.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Pg.connect(conn_string)` returns `Result<PgConn, String>` | Standard connection string: `"host=localhost dbname=mydb user=me"`. Go pgx: `pgx.Connect(ctx, str)`, Rust: `Client::connect(str)`. | Med | Wraps `PQconnectdb(conninfo)`. Check `PQstatus(conn) == CONNECTION_OK`. Error via `PQerrorMessage`. |
| `Pg.execute(conn, sql, params)` for non-SELECT | Same pattern as SQLite but with PostgreSQL's `$1, $2` param syntax. | Med | Uses `PQexecParams(conn, sql, nParams, NULL, paramValues, NULL, NULL, 0)`. Returns `Result<Int, String>`. |
| `Pg.query(conn, sql, params)` for SELECT | Returns rows as `List<Map<String, String>>`. Same return type as SQLite for API consistency. | Med-High | Uses `PQexecParams`, then `PQntuples`/`PQnfields`/`PQfname`/`PQgetvalue` to extract rows. |
| Parameterized queries with `$1, $2, ...` placeholders | PostgreSQL's native parameter syntax. All PG drivers use this. | Med | libpq `PQexecParams` takes param values as `const char *const *paramValues`. |
| `Pg.close(conn)` | Calls `PQfinish`. Universal cleanup. | Low | |
| Connection string parsing | Users expect standard PG connection strings to work. | Low | libpq handles this internally via `PQconnectdb`. |

**Expected API surface:**

```snow
fn main() do
  let conn = Pg.connect("host=localhost dbname=myapp user=snow")?
  Pg.execute(conn, "INSERT INTO users (name, age) VALUES ($1, $2)", ["Alice", "30"])?
  let rows = Pg.query(conn, "SELECT name, age FROM users WHERE age > $1", ["25"])?
  List.each(rows, fn(row) do
    let name = Map.get(row, "name")
    println("User: ${name}")
  end)
  Pg.close(conn)
end
```

**Implementation approach:**

New Rust runtime module `snow-rt/src/db/pg.rs` that wraps libpq:
1. Link `libpq` as a C dependency (use the `pq-sys` crate or link system libpq)
2. Same module registration pattern as SQLite: `Pg` in STDLIB_MODULES
3. Same return types: `Result<Int, String>` for execute, `Result<List<Map<String, String>>, String>` for query
4. `PQexecParams` handles parameterized queries natively

**API consistency with SQLite:**
- Both return `Result<List<Map<String, String>>, String>` for queries
- Both take `List<String>` for parameters
- Difference: SQLite uses `?` placeholders, PostgreSQL uses `$1, $2, ...`
- Difference: SQLite uses file path to open, PostgreSQL uses connection string

**What NOT to include:**
- Connection pooling -- future enhancement
- LISTEN/NOTIFY -- PostgreSQL-specific feature, defer
- COPY command support -- niche, defer
- SSL/TLS connection options -- libpq handles basic TLS via connection string params
- Prepared statement caching -- users re-execute queries

**Confidence:** HIGH -- libpq is the standard C interface to PostgreSQL, well-documented. The pattern mirrors SQLite exactly.

**Dependencies:** C FFI linkage for libpq. No dependencies on other v2.0 features.

---

### 4. HTTP Path Parameters

Every modern web framework supports path parameters: Express `/users/:id`, Phoenix `/users/:id`, Go 1.22 `/users/{id}`, Axum `/users/{id}`. Snow's current router only supports exact match and `/*` wildcard. Without path parameters, building RESTful APIs requires manual path parsing.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Route patterns with `:param` segments | `/users/:id` matches `/users/42` and binds `id = "42"`. Universal convention. | Med | Extends `SnowRouter::match_route` to do segment-by-segment matching. When a pattern segment starts with `:`, it matches any single segment and captures the value. |
| Extracted params accessible via `Request.param(req, "id")` | Returns `Option<String>`. Go 1.22: `r.PathValue("id")`, Axum: `Path(id)`, Phoenix: `conn.params["id"]`. | Med | `SnowHttpRequest` gains a `path_params: *mut u8` field (SnowMap). Router populates it during matching. New runtime function `snow_http_request_param`. |
| Multiple params in one path | `/users/:user_id/posts/:post_id` captures both. Universal expectation. | Low | The segment matching loop captures all named segments. No special handling needed beyond the basic matching. |
| Static routes take precedence over parameterized | `/users/me` matches before `/users/:id` when both are registered. Standard in Go 1.22, Axum, Express. | Low | Already handled by "first match wins" ordering. Document: register specific routes before parameterized ones. |
| Mixed static and param segments | `/api/v1/users/:id` -- static `api`, static `v1`, static `users`, param `id`. | Low | Segment-by-segment: static segments must match exactly, param segments match anything. |

**Expected API surface:**

```snow
fn get_user(request) do
  let id = Request.param(request, "id")
  case id do
    Some(user_id) -> HTTP.response(200, "User: ${user_id}")
    None -> HTTP.response(400, "Missing user ID")
  end
end

fn get_user_post(request) do
  let user_id = Request.param(request, "user_id")
  let post_id = Request.param(request, "post_id")
  HTTP.response(200, "User ${user_id}, Post ${post_id}")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/users/me", get_me)       # static, matches first
  let r = HTTP.route(r, "/users/:id", get_user)     # param, matches second
  let r = HTTP.route(r, "/users/:user_id/posts/:post_id", get_user_post)
  HTTP.serve(r, 8080)
end
```

**Implementation approach:**

1. **Router matching (`router.rs`):** Modify `matches_pattern` to split pattern and path by `/`, compare segment by segment. If pattern segment starts with `:`, it matches any path segment and the captured pair `(param_name, segment_value)` is recorded.

2. **Request struct (`server.rs`):** Add `path_params: *mut u8` field to `SnowHttpRequest`. The `handle_request` function builds this map from the router match result.

3. **Accessor function:** New `snow_http_request_param(req: *mut u8, name: *const SnowString) -> *mut u8` returning SnowOption, same pattern as `snow_http_request_header` and `snow_http_request_query`.

4. **Module registration:** Add `Request.param` to the function name mapping. Already has `Request.method`, `Request.path`, etc.

**What NOT to include:**
- Regex-based route matching -- over-engineering
- Route groups/scoping -- future enhancement
- Typed path extraction (automatic Int parsing) -- return String, users parse manually
- Catch-all named wildcard (`/api/*rest` capturing "rest") -- current `/*` already works for catch-all

**Confidence:** HIGH -- the router code is simple and well-structured. The modification is straightforward segment-level matching. All reference implementations (Go, Axum, Phoenix) work identically.

**Dependencies:** Existing `SnowRouter` and `SnowHttpRequest`. Must be built before middleware (middleware should see path params).

---

### 5. HTTP Middleware

Every web framework provides middleware for cross-cutting concerns: logging, authentication, CORS, rate limiting. Go uses handler wrapping `func(http.Handler) http.Handler`. Elixir Plug uses a pipeline of `call(conn, opts)` functions. Rust Tower uses `Layer`/`Service` traits. The core pattern is always the same: a function that wraps a handler.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Middleware wraps handlers, runs before/after | The fundamental middleware contract. Go: `func(http.Handler) http.Handler`. Elixir Plug: `call(conn, opts)`. | Med | In Snow, middleware is a function that takes a handler and returns a new handler: `fn(fn(Request) -> Response) -> fn(Request) -> Response`. Natural fit with Snow's closure support. |
| Middleware composition via `HTTP.use(router, middleware)` | Multiple middleware compose in order. Go: nested wrapping. Elixir: `pipeline`. | Med | Router gains a `middlewares: Vec<(fn_ptr, env_ptr)>` list. `HTTP.use` appends. Before calling matched handler, server wraps it through middleware chain. |
| Middleware can short-circuit (return early without calling next) | Auth middleware returns 401 without calling the inner handler. Universal pattern. | Low | Natural: if middleware returns a response without calling the wrapped handler, the chain stops. |
| Middleware execution order is intuitive | First middleware added runs first (outermost). Matches Go and Express conventions. | Low | Middleware applied in registration order. First registered = outermost wrapper. |

**Expected API surface:**

```snow
fn logging_middleware(handler) do
  fn(request) do
    let method = Request.method(request)
    let path = Request.path(request)
    println("[${method}] ${path}")
    handler(request)
  end
end

fn auth_middleware(handler) do
  fn(request) do
    let token = Request.header(request, "Authorization")
    case token do
      Some(_) -> handler(request)
      None -> HTTP.response(401, "Unauthorized")
    end
  end
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/api/users", list_users)
  let r = HTTP.use(r, logging_middleware)
  let r = HTTP.use(r, auth_middleware)
  HTTP.serve(r, 8080)
end
```

**Implementation approach:**

Snow's closure system already supports the middleware pattern naturally. A middleware is a function that takes a handler function and returns a new handler function. The key implementation work is:

1. **Router middleware list:** `SnowRouter` gains a `middlewares: Vec<(*mut u8, *mut u8)>` field (fn_ptr, env_ptr pairs).

2. **`HTTP.use(router, middleware_fn)` runtime function:** `snow_http_use(router, middleware_fn)` returns a new router with the middleware appended.

3. **Server handler wrapping:** In `handle_request`, before calling the matched route handler, wrap it through the middleware chain. Each middleware receives the next handler (inner) and returns a new handler (outer). The outermost wrapper receives the actual request.

4. **Calling convention:** The middleware function is called with the inner handler as argument. It returns a closure. That closure is called with the request. This matches Snow's existing closure calling convention (fn_ptr + env_ptr).

**What NOT to include:**
- Per-route middleware -- global only for MVP. Per-route is a differentiator.
- Middleware ordering DSL (like Phoenix pipelines) -- `HTTP.use` in order is sufficient
- Response modification after handler (post-processing) -- the middleware pattern naturally supports this (middleware calls handler, then wraps the response), but we don't need special API for it
- Async middleware -- blocking model is fine in actor context

**Confidence:** HIGH -- Snow's closures already support the wrapping pattern. The main work is plumbing middleware through the router and server. The calling convention for closures (fn_ptr + env_ptr) is well-established.

**Dependencies:** HTTP server infrastructure. Should be built after path params (middleware should have access to path params on the request).

---

## Differentiators

Features that set Snow apart. Not expected but valued -- these make Snow feel polished and intentional.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Typed parameter binding for database queries | `Sqlite.query(conn, "SELECT * WHERE age > ?", [42])` where `42` is bound as Int via `sqlite3_bind_int64`. Most basic drivers bind everything as text. | Med | Runtime inspects Snow value type tag and calls `sqlite3_bind_int64` for Int, `sqlite3_bind_double` for Float, `sqlite3_bind_text` for String. Params become `List<Any>` or separate typed bind functions. |
| `JSON.encode(value)` works on any type that derives Json | Dispatch based on argument type at compile time, not just on structs. Can also work on primitives and collections directly. | Med | The existing `JSON.encode_int`, `JSON.encode_string` etc. already handle primitives. The new dispatch unifies them: `JSON.encode(42)` calls `snow_json_encode_int`, `JSON.encode(user)` calls `Json__encode__User`. |
| Wildcard route with named capture | `/api/*rest` captures the remainder as `Request.param(req, "rest")`. Go: `{rest...}`, Axum: `{*path}`. | Low | Extend the existing `/*` wildcard to optionally capture the suffix. Minimal change to router matching. |
| JSON pretty-print option | `JSON.encode_pretty(value)` for human-readable output. Most JSON libs offer this. | Low | New runtime function wrapping `serde_json::to_string_pretty`. Low effort, nice developer UX. |
| Consistent database API surface | SQLite and PostgreSQL share the same return types and pattern: `open`/`connect` -> `execute`/`query` -> `close`. Makes switching databases straightforward. | Low | Design choice, not implementation work. Both modules return `Result<List<Map<String, String>>, String>` for queries. |
| `deriving(Json)` for sum types / enums | `type Color = Red \| Green \| Blue deriving(Json)` encodes as `"Red"`, `"Green"`, `"Blue"`. Tagged variants with fields encode as `{"tag":"Variant","field1":val}`. | High | Extension of the struct Json derive pattern. Sum types with no fields encode as strings. Sum types with fields encode as tagged objects. Defer to post-MVP. |
| Per-route middleware | Apply middleware to specific routes: `HTTP.route(r, "/admin/*", handler, [auth_middleware])`. Phoenix: `pipe_through`. Express: `router.use("/admin", auth)`. | Med | Extend `RouteEntry` to carry per-route middleware. Not needed for MVP -- global is sufficient. |
| Float column support in database queries | Return Float values from database columns as actual Float instead of String. | Low | Check `sqlite3_column_type` and use `sqlite3_column_double` for REAL columns. Enhancement over all-string return. |

---

## Anti-Features

Features to explicitly NOT build in this milestone.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| ORM / Query builder DSL | Massive scope. Ecto took years. ORMs are controversial in Go/Rust (many prefer raw SQL). Snow's type system isn't ready for a type-safe query builder. | Raw SQL with parameterized queries. Users write SQL strings. This is what Go's `database/sql` does. |
| Automatic struct-to-row mapping | Auto-mapping rows to structs (like Go sqlx `StructScan` or Rust sqlx `FromRow`) requires reflection or compile-time introspection Snow doesn't have. | Return rows as `List<Map<String, String>>`. Users extract fields manually. |
| Async/non-blocking database I/O | The actor runtime uses blocking I/O. Adding async DB ops would require a different I/O model. The HTTP server already uses blocking I/O successfully. | Database ops block the calling actor's coroutine. The M:N scheduler runs other actors on other threads. Same model as HTTP. |
| Connection pooling | Requires managing a pool of connections, handling concurrent access, idle timeouts, health checks. Significant complexity. | Single connection per `open`/`connect` call. Users manage connection lifecycle manually. Pool in a future milestone. |
| Database transactions as typed API | `BEGIN`/`COMMIT`/`ROLLBACK` with proper error recovery semantics requires careful design around partial failure, nested transactions, savepoints. | Users call `Sqlite.execute(conn, "BEGIN", [])` and `Sqlite.execute(conn, "COMMIT", [])` manually. Functional but not type-safe. |
| Database migrations framework | Schema versioning, up/down migrations, migration tracking table. Large feature surface. | Users call `Sqlite.execute(conn, "CREATE TABLE IF NOT EXISTS ...", [])`. Manual schema management. |
| JSON Schema validation | Validating JSON against a schema is a separate concern from encoding/decoding. | Users validate by decoding into a typed struct -- decode failure means the data didn't match. |
| Custom JSON serializers per field | Rust serde's `#[serde(serialize_with)]` requires a plugin/attribute system Snow doesn't have. | Default serialization for all field types. No customization. |
| Field renaming in JSON serde | `@json_name("user_name")` requires annotation/attribute syntax not yet in the parser. | Use struct field names as-is for JSON keys. Snake_case in Snow = snake_case in JSON. |
| WebSocket support | Different protocol. Not related to database/serialization. | Keep HTTP as request/response only. |
| Regex-based route matching | Over-engineering. Modern frameworks moved to simpler segment-based patterns. | `:param` named segments and `*` wildcards cover 99% of use cases. |
| Compile-time query validation | Like Rust sqlx's compile-time checked queries. Requires connecting to a database during compilation. | Runtime query validation only. SQL errors return `Err`. |
| `JSON.decode` with type inference | `let user: User = JSON.decode(str)?` where the compiler infers the target type. Requires a more sophisticated type inference system than Snow currently has. | Use explicit `User.from_json(str)` generated by `deriving(Json)`. |

---

## Feature Dependencies

```
deriving(Json) encode
  +-- Existing deriving infrastructure (MIR lowering for Eq, Display, etc.)
  +-- Existing snow_json_from_int, snow_json_from_float, snow_json_from_bool, snow_json_from_string
  +-- Existing snow_json_encode (SnowJson -> String)
  +-- NEW: snow_json_from_null (for Option<T> None values)
  +-- NEW: snow_json_from_map (for struct -> SnowJson Object conversion)
  +-- NEW: snow_json_from_list_mapped (for List<T> field encoding)
  |
  v
deriving(Json) decode
  +-- Existing snow_json_parse (String -> SnowJson)
  +-- Existing struct constructor codegen
  +-- NEW: snow_json_object_get_field (extract field from SnowJson Object by name)
  +-- NEW: snow_json_to_int, snow_json_to_string, etc. (SnowJson -> Snow type)
  +-- DEPENDS ON: encode being designed first (shared field metadata codegen)

HTTP path parameters
  +-- Existing SnowRouter (router.rs)
  +-- Existing SnowHttpRequest (server.rs)
  +-- NEW: path_params field on SnowHttpRequest
  +-- NEW: segment-level matching in router
  |
  v
HTTP middleware
  +-- Existing HTTP server (handle_request in server.rs)
  +-- Existing closure calling convention (fn_ptr + env_ptr)
  +-- DEPENDS ON: path params (middleware should see path params)
  +-- NEW: middleware list on SnowRouter
  +-- NEW: snow_http_use runtime function

SQLite driver (INDEPENDENT)
  +-- C FFI: sqlite3 library linkage
  +-- NEW: snow-rt/src/db/sqlite.rs
  +-- NEW: Sqlite module in STDLIB_MODULES

PostgreSQL driver (INDEPENDENT, follows SQLite pattern)
  +-- C FFI: libpq library linkage
  +-- NEW: snow-rt/src/db/pg.rs
  +-- NEW: Pg module in STDLIB_MODULES
```

**Critical dependency chain:** deriving(Json) encode -> decode (shared infrastructure)
**Critical dependency chain:** HTTP path params -> HTTP middleware (middleware needs params)
**Fully independent features:** SQLite driver, PostgreSQL driver (independent of each other and of JSON/HTTP)

---

## MVP Recommendation

### Build order rationale:

**1. `deriving(Json)` encode** -- Highest user-visible impact. Existing deriving infrastructure proven. Encode is simpler than decode. Unblocks practical HTTP handler patterns: `JSON.encode(user)` instead of manual map building.

**2. `deriving(Json)` decode** -- Completes the serde story. Harder than encode (error handling, type coercion). Returns `Result<T, String>`. Must share field metadata codegen with encode.

**3. HTTP path parameters** -- Small, well-scoped change to existing router. Required before middleware makes sense. Extends SnowRouter with segment matching and SnowHttpRequest with params map.

**4. HTTP middleware** -- Depends on path params being available. Changes handler calling convention in the server. Global middleware only for MVP.

**5. SQLite driver** -- New C FFI dependency. Independent of JSON/HTTP features. Simpler than PostgreSQL (embedded, no network). Establishes the database API pattern.

**6. PostgreSQL driver** -- Follows SQLite's API pattern exactly. Adds libpq C FFI dependency. Network-based, slightly more complex error handling.

### Prioritize:
1. `deriving(Json)` encode -- makes HTTP handlers practical
2. `deriving(Json)` decode -- completes the serde loop
3. HTTP path parameters -- makes the router useful for real APIs
4. HTTP middleware -- enables cross-cutting concerns (logging, auth)
5. SQLite driver with parameterized queries -- local data persistence
6. PostgreSQL driver with parameterized queries -- production database access

### Defer:
- **Unified Db module**: Ship Sqlite and Pg as separate modules, unify later
- **Connection pooling**: Single connection is fine for initial release
- **Field renaming in JSON**: Requires attribute/annotation syntax
- **Per-route middleware**: Global middleware is sufficient MVP
- **Transaction API**: Manual BEGIN/COMMIT via execute is acceptable
- **Typed row mapping**: `List<Map<String, String>>` is the MVP return type
- **Sum type JSON serde**: Ship struct serde first, extend to sum types later
- **JSON type-inferred decode**: Use explicit `User.from_json(str)` instead

---

## Complexity Assessment

| Feature | Estimated Effort | Risk | Notes |
|---------|-----------------|------|-------|
| `deriving(Json)` encode | 3-4 days | MEDIUM | New MIR codegen pattern, but follows established deriving infrastructure. Must handle all field types (Int, Float, String, Bool, Option, List, nested struct). |
| `deriving(Json)` decode | 4-6 days | **HIGH** | Error handling is complex. Must handle missing fields, wrong types, nested structs, Option fields. The SnowJson-to-Snow-type conversion needs careful codegen. |
| HTTP path parameters | 2-3 days | LOW | Straightforward segment matching extension to existing router. Well-understood pattern. |
| HTTP middleware | 2-3 days | MEDIUM | Must correctly wrap closures through the middleware chain. Calling convention must match existing closure handling. |
| SQLite driver | 3-5 days | MEDIUM | New C FFI dependency. Build system integration. Multiple runtime functions. Testing requires actual SQLite database operations. |
| PostgreSQL driver | 3-5 days | MEDIUM | Same pattern as SQLite. Requires libpq to be installed on build system. Connection string handling delegated to libpq. |

**Total estimated effort:** 17-26 days

**Key risks:**
1. **`deriving(Json)` decode correctness.** Decoding must handle all type combinations correctly. A SnowJson Number to Snow Int, SnowJson Str to Snow String, SnowJson Null to Snow None, SnowJson Array to Snow List, SnowJson Object to nested struct. Missing any case produces runtime crashes.
2. **C FFI build dependencies.** SQLite and PostgreSQL require C libraries at build time. SQLite can be bundled (rusqlite does this). PostgreSQL requires libpq installed on the system. Build instructions must be clear.
3. **Middleware closure calling convention.** The middleware wraps a handler (fn_ptr) and returns a new closure (fn_ptr + env_ptr). The server must call through this chain correctly. Getting the env_ptr handling wrong causes segfaults.
4. **SnowJson Number representation.** Current runtime stores numbers as i64 (or f64 bits). Decoding must distinguish between integers and floats when mapping to Snow Int vs Float fields. The current `JSON_NUMBER` tag doesn't distinguish -- may need a `JSON_FLOAT` tag.

---

## Sources

### JSON Serde (struct-aware)
- [Serde derive documentation](https://serde.rs/derive.html) -- HIGH confidence
- [Go encoding/json package](https://pkg.go.dev/encoding/json) -- HIGH confidence
- [Jason.Encoder (Elixir)](https://hexdocs.pm/jason/Jason.Encoder.html) -- HIGH confidence
- [Poison (Elixir) decode with as:](https://hexdocs.pm/poison/Poison.html) -- HIGH confidence

### Database Drivers
- [SQLite C/C++ Interface](https://sqlite.org/cintro.html) -- HIGH confidence
- [rusqlite documentation](https://docs.rs/rusqlite/) -- HIGH confidence
- [PostgreSQL libpq C Library](https://www.postgresql.org/docs/current/libpq.html) -- HIGH confidence
- [Go pgx v5 driver](https://pkg.go.dev/github.com/jackc/pgx/v5) -- HIGH confidence
- [Go database/sql querying](https://go.dev/doc/database/querying) -- HIGH confidence
- [Elixir Ecto query API](https://hexdocs.pm/ecto/Ecto.Query.html) -- HIGH confidence

### HTTP Path Parameters
- [Go 1.22 routing enhancements](https://go.dev/blog/routing-enhancements) -- HIGH confidence
- [Axum Path extractor](https://docs.rs/axum/latest/axum/extract/struct.Path.html) -- HIGH confidence
- [Phoenix.Router routing docs](https://hexdocs.pm/phoenix/routing.html) -- HIGH confidence

### HTTP Middleware
- [Elixir Plug library](https://hexdocs.pm/plug/readme.html) -- HIGH confidence
- [Go HTTP middleware patterns](https://www.alexedwards.net/blog/making-and-using-middleware) -- MEDIUM confidence
- [Tower middleware (Rust)](https://docs.rs/tower) -- HIGH confidence

### Snow Codebase (direct inspection)
- `crates/snow-rt/src/json.rs` -- SnowJson runtime type, `snow_json_from_*` helpers, `snow_json_encode`/`snow_json_parse`
- `crates/snow-rt/src/http/router.rs` -- SnowRouter with exact match and `/*` wildcard
- `crates/snow-rt/src/http/server.rs` -- SnowHttpRequest struct, handle_request, actor-per-connection
- `crates/snow-rt/src/http/client.rs` -- HTTP client pattern (ureq)
- `crates/snow-codegen/src/mir/lower.rs` lines 1578-1604 -- deriving infrastructure for structs
- `crates/snow-codegen/src/mir/lower.rs` lines 7583-7787 -- STDLIB_MODULES list and map_builtin_name function
- `crates/snow-codegen/src/codegen/intrinsics.rs` lines 345-370 -- JSON intrinsic declarations

---
*Feature research for: Snow Language v2.0 Database & Serialization*
*Researched: 2026-02-10*
