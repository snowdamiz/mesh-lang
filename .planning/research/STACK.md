# Technology Stack: v2.0 Database & Serialization

**Project:** Snow compiler -- JSON serde (deriving(Json)), SQLite driver (C FFI), PostgreSQL driver (pure wire protocol), parameterized queries, HTTP path parameters, HTTP middleware
**Researched:** 2026-02-10
**Confidence:** HIGH (based on direct codebase analysis of all 12 crates, PostgreSQL wire protocol official docs, SQLite C API official docs, and Rust crate ecosystem research)

## Executive Summary

This milestone requires **3-5 new Rust crate dependencies** in `snow-rt` (for SQLite C bindings, MD5 hashing, SHA-256/HMAC/PBKDF2 for PostgreSQL auth) and **zero changes** to the compiler toolchain (Inkwell, LLVM). The work divides into two categories:

1. **Compiler-side (codegen):** JSON serde via `deriving(Json)` is 100% compile-time code generation -- the compiler emits LLVM IR that calls existing `snow_json_from_*` / `snow_json_parse` runtime functions field-by-field. No reflection, no runtime type info. HTTP path parameters and middleware require router changes and new runtime functions. All follow the established pattern: new MIR lowering -> new intrinsic declarations -> LLVM IR emission -> `extern "C"` runtime functions in `snow-rt`.

2. **Runtime-side (snow-rt):** SQLite via `libsqlite3-sys` with `bundled` feature (compiles SQLite from source, zero system dependencies). PostgreSQL via pure Rust TCP implementation of the v3 wire protocol (no libpq dependency). Both expose `extern "C"` functions callable from Snow-compiled code.

The critical architectural decision: **SQLite uses C FFI through the Rust runtime (not LLVM-emitted C calls)** because the Rust runtime already manages GC, strings, and error handling. The Snow compiler emits calls to `snow_sqlite_*` functions in `snow-rt`, which internally call `libsqlite3-sys` functions. This follows the exact same pattern as `snow_json_parse` calling `serde_json`.

## Recommended Stack

### Core Framework (NO CHANGES)

| Technology | Version | Purpose | Status |
|------------|---------|---------|--------|
| Rust | stable 2021 edition | Compiler implementation | No change |
| Inkwell | 0.8.0 (`llvm21-1`) | LLVM IR generation | No change |
| LLVM | 21.1 | Backend codegen + optimization | No change |
| Rowan | 0.16 | CST for parser | No change |
| ena | 0.14 | Union-find for HM type inference | No change |
| ariadne | 0.6 | Diagnostic error reporting | No change |
| corosensei | 0.3 | Stackful coroutines for actors | No change |

### Runtime (snow-rt) -- NEW DEPENDENCIES

| Technology | Version | Purpose | Why This |
|------------|---------|---------|----------|
| libsqlite3-sys | 0.36.0 | SQLite C FFI bindings | Provides raw `sqlite3_open`, `sqlite3_prepare_v2`, etc. Used with `bundled` feature to compile SQLite from source -- zero system dependency, cross-platform. No need for `rusqlite` wrapper since we write our own thin C FFI layer in the runtime. |
| md5 | 0.8.0 | MD5 hashing for PostgreSQL legacy auth | PostgreSQL's MD5 auth requires `"md5" + MD5(MD5(password + username) + salt)`. Zero dependencies, 76M+ downloads, battle-tested. Required for compatibility with PostgreSQL servers using MD5 auth. |
| sha2 | 0.10 | SHA-256 for SCRAM-SHA-256 auth | PostgreSQL's modern SCRAM-SHA-256 auth requires SHA-256 hashing. Part of RustCrypto ecosystem. |
| hmac | 0.12 | HMAC-SHA256 for SCRAM-SHA-256 auth | SCRAM protocol requires HMAC-SHA256 for client/server proof computation. |
| pbkdf2 | 0.12 | PBKDF2 key derivation for SCRAM auth | SCRAM-SHA-256 uses PBKDF2 with SHA-256 for salted key derivation. |

### What NOT to Add

| Crate | Why NOT |
|-------|---------|
| `rusqlite` | Overkill -- Snow's runtime only needs ~10 SQLite C functions (`open`, `close`, `prepare_v2`, `step`, `finalize`, `bind_*`, `column_*`, `errmsg`). `rusqlite` adds 15K+ lines of safe wrapper code we do not need. Use `libsqlite3-sys` directly with `unsafe` in the runtime, matching our existing pattern (the runtime is already full of `unsafe extern "C"` functions). |
| `tokio-postgres` / `postgres` | These crates are async-first (require Tokio runtime) and synchronous wrapper respectively. Snow's runtime uses its own M:N actor scheduler with corosensei coroutines -- Tokio would conflict. Implement the PostgreSQL v3 wire protocol directly over `std::net::TcpStream`. The protocol is well-documented and straightforward (startup, extended query with Parse/Bind/Execute/Sync, read DataRow responses). |
| `sqlx` | Same problem as tokio-postgres -- async runtime dependency. Also brings compile-time query checking which requires a running database, inappropriate for a language compiler's runtime. |
| `diesel` | ORM abstraction layer -- completely wrong level of abstraction for a language runtime that needs raw protocol access. |
| `base64` (crate) | Only needed for SCRAM-SHA-256. Can use a minimal inline base64 encoder/decoder (~40 lines) or the `base64` crate from RustCrypto. Decision: use the `base64ct` crate (0.1, constant-time, RustCrypto ecosystem, already a transitive dep of `pbkdf2`). |
| `scram-rs` | Full SCRAM library is unnecessary -- SCRAM-SHA-256 implementation is ~100 lines using `sha2` + `hmac` + `pbkdf2`. Pulling in a full library for what is a simple 4-message exchange is excessive. |
| `serde` (for runtime JSON) | Already in workspace for compiler tooling. NOT needed for Snow's JSON serde -- the `deriving(Json)` approach generates LLVM code that calls `snow_json_from_*` and `snow_json_parse` directly. No Rust serde involved at runtime. |

## Feature-by-Feature Stack Analysis

### 1. JSON Serde via deriving(Json)

**Approach:** Compile-time code generation, not runtime reflection.

**How it works:** When a struct has `deriving(Json)`, the compiler generates two synthetic functions at MIR lowering time:

```
// For: struct User { name: String, age: Int } deriving(Json)

// Generated: User_to_json(self: User) -> *mut SnowJson
//   1. Allocate SnowMap
//   2. For each field:
//      - snow_json_from_string(self.name) -> json_val
//      - snow_map_put(map, "name", json_val)
//      - snow_json_from_int(self.age) -> json_val
//      - snow_map_put(map, "age", json_val)
//   3. Return alloc_json(JSON_OBJECT, map)

// Generated: User_from_json(json: *mut SnowJson) -> Result<User, String>
//   1. Verify json.tag == JSON_OBJECT
//   2. For each field:
//      - snow_map_get(json.value, "name") -> field_json
//      - Verify field_json.tag == JSON_STR -> extract string
//      - snow_map_get(json.value, "age") -> field_json
//      - Verify field_json.tag == JSON_NUMBER -> extract int
//   3. Return Ok(User { name, age })
```

**Stack requirements:** NONE new. Uses existing:
- `snow_json_from_int`, `snow_json_from_string`, `snow_json_from_bool`, `snow_json_from_float` (already in `snow-rt/src/json.rs`)
- `snow_json_parse`, `snow_json_encode` (already in `snow-rt/src/json.rs`)
- `snow_map_new`, `snow_map_put`, `snow_map_get` (already in `snow-rt/src/collections/map.rs`)

**New runtime functions needed:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_json_to_string` | `(json: ptr) -> ptr` | Serialize SnowJson to JSON string (alias for `snow_json_encode`, but may need to handle struct-produced JSON objects) |
| `snow_json_from_null` | `() -> ptr` | Create a JSON null value (for Option::None serialization) |
| `snow_json_from_array` | `(list: ptr) -> ptr` | Create a JSON array from a SnowList of SnowJson values |
| `snow_json_from_object` | `(map: ptr) -> ptr` | Create a JSON object from a SnowMap of (String -> SnowJson) |
| `snow_json_get_tag` | `(json: ptr) -> i8` | Read the tag byte (for type checking during deserialization) |
| `snow_json_get_value` | `(json: ptr) -> u64` | Read the value field (for extraction during deserialization) |
| `snow_json_object_get` | `(json: ptr, key: ptr) -> ptr` | Get a field from a JSON object by string key (returns SnowOption) |

**Compiler changes:**
- **Parser:** Already parses `deriving(...)` (verified: `has_deriving_clause()`, `deriving_traits()` on struct/sum type AST nodes)
- **Typeck:** Add "Json" to the recognized deriving trait names. Validate that all fields of the struct have types that are JSON-serializable (primitives, String, Option, List, Map, other structs with deriving(Json), sum types with deriving(Json))
- **MIR lowering:** In `lower_struct_def` and `lower_sum_type_def`, when `derive_list` contains "Json", generate `to_json` and `from_json` MIR functions. This follows the existing pattern for `generate_debug_inspect_struct`, `generate_eq_struct`, etc.
- **Codegen:** No new codegen nodes -- the generated MIR functions use existing `Call`, `FieldAccess`, `StructLit`, `ConstructVariant` nodes

**Confidence:** HIGH -- follows identical pattern to existing `deriving(Debug)`, `deriving(Eq)`, `deriving(Hash)`. The MIR lowering in `lower.rs` (lines 1579-1601) already has the dispatch structure for deriving traits.

---

### 2. SQLite Driver (C FFI via Runtime)

**Approach:** `libsqlite3-sys` with `bundled` feature in `snow-rt`, exposed via `extern "C"` functions.

**Architecture decision: Why runtime FFI, not LLVM-emitted C calls**

The Snow compiler could theoretically emit LLVM IR that calls `sqlite3_open` directly (declaring it as an external function in the LLVM module). However, this is the WRONG approach because:

1. **Error handling:** SQLite returns integer error codes. Converting these to Snow's `Result<T, String>` requires calling `sqlite3_errmsg()` and constructing a `SnowString`. This logic belongs in Rust, not in generated LLVM IR.
2. **Memory management:** SQLite allocates memory internally (statement handles, result strings). The runtime needs to track these for cleanup. Rust's RAII pattern handles this naturally.
3. **GC integration:** Result values (rows, columns) must be allocated on the actor's GC heap via `snow_gc_alloc_actor`. This is trivial from Rust, complex from LLVM IR.
4. **String conversion:** SQLite uses C strings (`*const c_char`), Snow uses `SnowString` (pointer + length). Conversion happens in the runtime.
5. **Linker simplicity:** `libsqlite3-sys` with `bundled` compiles SQLite into `libsnow_rt.a`. The existing link step (`cc obj.o -L dir -lsnow_rt`) works unchanged.

**Cargo.toml change (snow-rt):**

```toml
[dependencies]
libsqlite3-sys = { version = "0.36", features = ["bundled"] }
```

The `bundled` feature uses the `cc` crate to compile SQLite 3.51.1 from source during `cargo build`. This produces a static archive that gets linked into `libsnow_rt.a`. No system SQLite installation needed. Cross-compilation works out of the box.

**New runtime functions (snow-rt/src/sqlite.rs):**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_sqlite_open` | `(path: ptr) -> ptr` | Open database, returns `Result<DbHandle, String>` |
| `snow_sqlite_close` | `(db: ptr) -> void` | Close database connection |
| `snow_sqlite_execute` | `(db: ptr, sql: ptr) -> ptr` | Execute SQL without results, returns `Result<Unit, String>` |
| `snow_sqlite_query` | `(db: ptr, sql: ptr, params: ptr) -> ptr` | Execute query with params, returns `Result<List<Row>, String>` |
| `snow_sqlite_prepare` | `(db: ptr, sql: ptr) -> ptr` | Prepare statement, returns `Result<StmtHandle, String>` |
| `snow_sqlite_bind_int` | `(stmt: ptr, idx: i64, val: i64) -> ptr` | Bind int param, returns `Result<Unit, String>` |
| `snow_sqlite_bind_string` | `(stmt: ptr, idx: i64, val: ptr) -> ptr` | Bind string param |
| `snow_sqlite_bind_float` | `(stmt: ptr, idx: i64, val: f64) -> ptr` | Bind float param |
| `snow_sqlite_bind_null` | `(stmt: ptr, idx: i64) -> ptr` | Bind null param |
| `snow_sqlite_step` | `(stmt: ptr) -> ptr` | Step statement, returns `Result<Option<Row>, String>` |
| `snow_sqlite_finalize` | `(stmt: ptr) -> void` | Destroy prepared statement |
| `snow_sqlite_row_get_int` | `(row: ptr, col: i64) -> i64` | Get int from column |
| `snow_sqlite_row_get_string` | `(row: ptr, col: i64) -> ptr` | Get string from column |
| `snow_sqlite_row_get_float` | `(row: ptr, col: i64) -> f64` | Get float from column |
| `snow_sqlite_row_is_null` | `(row: ptr, col: i64) -> i8` | Check if column is null |
| `snow_sqlite_column_count` | `(stmt: ptr) -> i64` | Number of result columns |

**Implementation pattern (snow_sqlite_open):**

```rust
use libsqlite3_sys::*;
use std::ffi::CString;

#[no_mangle]
pub extern "C" fn snow_sqlite_open(path: *const SnowString) -> *mut SnowResult {
    unsafe {
        let path_str = (*path).as_str();
        let c_path = match CString::new(path_str) {
            Ok(c) => c,
            Err(_) => return err_result("Invalid path: contains null byte"),
        };
        let mut db: *mut sqlite3 = std::ptr::null_mut();
        let rc = sqlite3_open(c_path.as_ptr(), &mut db);
        if rc != SQLITE_OK {
            let err = if db.is_null() {
                "Failed to open database".to_string()
            } else {
                let msg = std::ffi::CStr::from_ptr(sqlite3_errmsg(db));
                let s = msg.to_string_lossy().to_string();
                sqlite3_close(db);
                s
            };
            return err_result(&err);
        }
        // Wrap db pointer as opaque handle
        ok_result(db as *mut u8)
    }
}
```

**Linker impact:** NONE. The `bundled` feature compiles SQLite into `libsnow_rt.a` via the `cc` crate during `cargo build -p snow-rt`. The existing link step already links `libsnow_rt.a`. No additional `-lsqlite3` flag needed.

**Confidence:** HIGH -- `libsqlite3-sys` with `bundled` is the standard approach used by rusqlite (0.38.0, 15K+ GitHub stars). The `extern "C"` function pattern is identical to 50+ existing functions in `snow-rt`.

---

### 3. PostgreSQL Driver (Pure Wire Protocol)

**Approach:** Implement PostgreSQL v3 wire protocol directly over `std::net::TcpStream` in `snow-rt`. No external PostgreSQL library.

**Why pure implementation instead of a crate:**

1. **No async runtime dependency.** `tokio-postgres` requires Tokio. `postgres` (sync) embeds a Tokio runtime internally. Snow's M:N actor scheduler uses corosensei coroutines -- embedding Tokio would create a second scheduler, doubling thread count and creating deadlock risk.
2. **The protocol is simple.** PostgreSQL v3 is a message-based protocol. Each message is: `[1-byte type][4-byte length][payload]`. The Extended Query flow (Parse -> Bind -> Execute -> Sync) requires handling ~12 message types. This is ~500-800 lines of Rust.
3. **Full control.** Connection pooling, error handling, and timeout behavior can integrate with Snow's actor model naturally. Each database connection lives in an actor.

**Protocol implementation (snow-rt/src/postgres/):**

```
snow-rt/src/postgres/
  mod.rs        -- module root, re-exports
  protocol.rs   -- message encoding/decoding (~300 lines)
  connection.rs -- TcpStream management, startup, auth (~200 lines)
  auth.rs       -- MD5 and SCRAM-SHA-256 authentication (~150 lines)
  query.rs      -- parameterized query execution (~200 lines)
  types.rs      -- PostgreSQL type OID mapping (~50 lines)
```

**Key message types to implement:**

| Direction | Message | Byte ID | Purpose |
|-----------|---------|---------|---------|
| F -> B | StartupMessage | (none) | Protocol version 3.0, user, database |
| F -> B | PasswordMessage | 'p' | MD5 or cleartext password |
| F -> B | SASLInitialResponse | 'p' | SCRAM-SHA-256 first message |
| F -> B | SASLResponse | 'p' | SCRAM-SHA-256 final message |
| F -> B | Parse | 'P' | Prepare statement with $1, $2 params |
| F -> B | Bind | 'B' | Bind parameter values to statement |
| F -> B | Describe | 'D' | Get column metadata |
| F -> B | Execute | 'E' | Execute prepared statement |
| F -> B | Sync | 'S' | Transaction sync point |
| F -> B | Terminate | 'X' | Close connection |
| B -> F | AuthenticationOk | 'R' | Auth successful |
| B -> F | AuthenticationMD5Password | 'R' | MD5 auth challenge (with salt) |
| B -> F | AuthenticationSASL | 'R' | SCRAM auth start |
| B -> F | ReadyForQuery | 'Z' | Backend ready (idle/txn/error) |
| B -> F | RowDescription | 'T' | Column names and types |
| B -> F | DataRow | 'D' | Row data (text or binary) |
| B -> F | CommandComplete | 'C' | Query finished |
| B -> F | ErrorResponse | 'E' | Error with severity, code, message |
| B -> F | ParseComplete | '1' | Parse succeeded |
| B -> F | BindComplete | '2' | Bind succeeded |
| B -> F | ParameterStatus | 'S' | Server config param notification |

**Authentication support:**

| Method | Implementation | Dependencies |
|--------|---------------|-------------|
| `trust` | No password needed | None |
| `password` (cleartext) | Send password in PasswordMessage | None |
| `md5` | `"md5" + MD5(MD5(password + username) + salt)` | `md5` crate |
| `scram-sha-256` | Full SCRAM exchange (4 messages) | `sha2`, `hmac`, `pbkdf2` |

**SCRAM-SHA-256 is essential** -- it is the default auth method since PostgreSQL 10 and mandatory on many cloud providers (AWS RDS, Google Cloud SQL, Azure). Omitting it would make Snow unusable with most production PostgreSQL instances.

**New runtime functions (snow-rt/src/postgres/):**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_pg_connect` | `(conn_str: ptr) -> ptr` | Connect to PostgreSQL, returns `Result<ConnHandle, String>` |
| `snow_pg_close` | `(conn: ptr) -> void` | Close connection |
| `snow_pg_execute` | `(conn: ptr, sql: ptr, params: ptr) -> ptr` | Execute with params, returns `Result<Unit, String>` |
| `snow_pg_query` | `(conn: ptr, sql: ptr, params: ptr) -> ptr` | Query with params, returns `Result<List<Row>, String>` |
| `snow_pg_row_get_int` | `(row: ptr, col: i64) -> i64` | Get int from column |
| `snow_pg_row_get_string` | `(row: ptr, col: i64) -> ptr` | Get string from column |
| `snow_pg_row_get_float` | `(row: ptr, col: i64) -> f64` | Get float from column |
| `snow_pg_row_is_null` | `(row: ptr, col: i64) -> i8` | Check if column is null |
| `snow_pg_row_column_count` | `(row: ptr) -> i64` | Number of columns |

**Connection string format:** `"host=localhost port=5432 user=snow password=secret dbname=mydb"` (key=value pairs, matching libpq convention for user familiarity).

**Parameterized queries use `$1`, `$2` syntax** (PostgreSQL native), not `?` (which would conflict with Snow's error propagation operator and require translation):

```snow
let rows = Pg.query(conn, "SELECT * FROM users WHERE age > $1 AND name = $2", [42, "Alice"])?
```

**Confidence:** MEDIUM-HIGH -- PostgreSQL wire protocol is well-documented and stable (v3 since PostgreSQL 7.4, 2003). The implementation is straightforward but the SCRAM-SHA-256 auth adds complexity. The `rust-postgres` crate's `postgres_protocol` sub-crate demonstrates this is ~800 lines of Rust for a complete implementation.

---

### 4. Parameterized Queries (Shared Infrastructure)

**Approach:** Both SQLite and PostgreSQL share a common query parameter interface at the Snow language level.

**Snow-level API:**

```snow
// SQLite uses ? placeholders
let rows = Sqlite.query(db, "SELECT * FROM users WHERE age > ?", [42])?

// PostgreSQL uses $1, $2 placeholders
let rows = Pg.query(conn, "SELECT * FROM users WHERE age > $1", [42])?
```

**Parameter passing:** Parameters are passed as a `List<DbValue>` where `DbValue` is a sum type:

```snow
type DbValue =
  | DbInt(Int)
  | DbFloat(Float)
  | DbString(String)
  | DbBool(Bool)
  | DbNull
```

**Runtime representation:** The parameter list is a `SnowList` where each element is a tagged union (8-byte tag + 8-byte value, matching Snow's sum type layout). The runtime functions iterate the list and call the appropriate `sqlite3_bind_*` or PostgreSQL Bind message encoding.

**Stack requirement:** NONE new -- uses existing `SnowList` and sum type infrastructure.

**Confidence:** HIGH -- follows existing patterns for passing heterogeneous data through the runtime.

---

### 5. HTTP Path Parameters

**Approach:** Extend the router in `snow-rt/src/http/router.rs` to support named path segments.

**Current state:** The router supports exact match (`/api/health`) and wildcard (`/api/*`). It does NOT support named parameters like `/users/:id`.

**New pattern matching:**

```
/users/:id        -> captures { "id": "123" }
/users/:id/posts  -> captures { "id": "456" }
/api/:version/*   -> captures { "version": "v2" }  (mixed named + wildcard)
```

**Implementation:** Extend `matches_pattern` in `router.rs` to detect `:name` segments, extract them during matching, and store them in a captures map. The captured parameters are added to the `SnowHttpRequest` struct as a new `path_params: *mut u8` field (SnowMap).

**New runtime functions:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_http_request_param` | `(req: ptr, name: ptr) -> ptr` | Get path parameter by name, returns `Option<String>` |
| `snow_http_route_with_method` | `(router: ptr, method: ptr, pattern: ptr, handler: ptr) -> ptr` | Route with specific HTTP method |

**Stack requirement:** NONE new -- uses existing `SnowMap`, `SnowString` infrastructure.

**Confidence:** HIGH -- simple extension to existing router. The `matches_pattern` function is 10 lines; extending it for `:param` captures adds ~30 lines.

---

### 6. HTTP Middleware

**Approach:** Middleware as a function chain in the router, applied before the handler.

**Design:** Middleware is a function `fn(Request) -> Result<Request, Response>`. If it returns `Ok(request)`, the chain continues. If it returns `Err(response)`, the response is sent immediately (short-circuit).

**Runtime representation:** The router stores an ordered list of middleware functions. During request handling, before calling the route handler, the runtime iterates the middleware chain:

```rust
// In handle_request():
let mut current_req = snow_req;
for middleware in &router.middleware {
    let result = call_middleware(middleware, current_req);
    match result_tag(result) {
        0 => current_req = result_value(result) as *mut SnowHttpRequest, // Ok(modified_req)
        1 => {
            // Err(response) -- short-circuit
            let resp = result_value(result) as *const SnowHttpResponse;
            send_response(request, resp);
            return;
        }
    }
}
// Continue to route handler with (potentially modified) current_req
```

**New runtime functions:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_http_use` | `(router: ptr, middleware_fn: ptr) -> ptr` | Add middleware to router, returns new router |

**Stack requirement:** NONE new -- middleware functions use the same calling convention as route handlers (fn pointer + optional env pointer for closures).

**Confidence:** HIGH -- follows the same function pointer calling convention already used for route handlers.

---

## Integration Points with Existing Crates

### snow-parser (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| No parser changes for JSON serde | `deriving(Json)` uses existing `deriving(...)` syntax | 0 |
| No parser changes for databases | Database operations are stdlib function calls | 0 |
| No parser changes for HTTP | Middleware/params are stdlib function calls | 0 |

### snow-typeck (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| Recognize "Json" in deriving list | Allow `deriving(Json)` on structs/sum types | ~5 |
| Validate Json-serializable fields | Check field types are JSON-compatible | ~40 |
| `Sqlite` stdlib module types | Type signatures for open, query, execute, etc. | ~30 |
| `Pg` stdlib module types | Type signatures for connect, query, execute, etc. | ~30 |
| `DbValue` sum type | Built-in type for database parameters | ~15 |
| `DbRow` struct type | Built-in type for database result rows | ~15 |
| Extended HTTP module types | path_param, middleware, route_with_method | ~15 |

### snow-codegen / MIR (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| `generate_to_json_struct` | MIR function for struct -> JSON | ~80 |
| `generate_from_json_struct` | MIR function for JSON -> struct | ~100 |
| `generate_to_json_sum_type` | MIR function for sum type -> JSON | ~100 |
| `generate_from_json_sum_type` | MIR function for JSON -> sum type | ~120 |
| Sqlite/Pg call routing | Route `Sqlite.query(...)` to `snow_sqlite_query` | ~40 |
| HTTP middleware/params routing | Route `Http.use(...)`, `request.param(...)` | ~20 |

### snow-codegen / codegen (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| SQLite intrinsic declarations | `snow_sqlite_open`, etc. in intrinsics.rs | ~40 |
| PostgreSQL intrinsic declarations | `snow_pg_connect`, etc. in intrinsics.rs | ~30 |
| JSON serde intrinsic declarations | `snow_json_from_null`, `snow_json_from_array`, etc. | ~20 |
| HTTP middleware/params declarations | `snow_http_use`, `snow_http_request_param`, etc. | ~10 |

### snow-codegen / link (NO CHANGES)

The `bundled` feature in `libsqlite3-sys` compiles SQLite into `libsnow_rt.a`. No linker flag changes needed. PostgreSQL is pure Rust over TCP, no external libraries.

### snow-rt (additions)

| Addition | Purpose | Estimated Lines |
|----------|---------|----------------|
| `sqlite.rs` (new module) | SQLite driver: open, prepare, bind, step, close | ~300 |
| `postgres/mod.rs` (new dir) | PostgreSQL module root | ~20 |
| `postgres/protocol.rs` | Wire protocol message encode/decode | ~300 |
| `postgres/connection.rs` | TCP connection, startup, shutdown | ~200 |
| `postgres/auth.rs` | MD5 + SCRAM-SHA-256 authentication | ~150 |
| `postgres/query.rs` | Parameterized query execution | ~200 |
| `postgres/types.rs` | PostgreSQL OID -> Snow type mapping | ~50 |
| `json.rs` additions | `snow_json_from_null`, `snow_json_from_array`, etc. | ~40 |
| `http/router.rs` additions | Path parameter matching, method routing | ~60 |
| `http/server.rs` additions | Middleware chain execution | ~50 |

### Total Estimated New Lines: ~2,130

## Cargo.toml Changes

### snow-rt/Cargo.toml

```toml
[dependencies]
# Existing (unchanged)
crossbeam-deque = "0.8"
crossbeam-utils = "0.8"
crossbeam-channel = "0.5"
corosensei = "0.3"
parking_lot = "0.12"
rustc-hash = { workspace = true }
serde_json = "1"
tiny_http = "0.12"
ureq = "2"

# NEW: SQLite C FFI bindings (compiles SQLite from source)
libsqlite3-sys = { version = "0.36", features = ["bundled"] }

# NEW: PostgreSQL authentication
md5 = "0.8"
sha2 = "0.10"
hmac = "0.12"
pbkdf2 = "0.12"
```

### Workspace Cargo.toml (no changes needed)

The new crates are only dependencies of `snow-rt`, not workspace-wide.

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| SQLite binding | `libsqlite3-sys` (direct) | `rusqlite` (safe wrapper) | 15K lines of safe wrapper we do not need. Our runtime is already `unsafe extern "C"`. Direct FFI is simpler, smaller, no abstraction mismatch. |
| SQLite binding | `bundled` feature | System SQLite (`pkg-config`) | Requires users to install `libsqlite3-dev`. Bundled compiles from source -- zero external dependencies, reproducible builds, consistent SQLite version. |
| PostgreSQL driver | Pure wire protocol | `tokio-postgres` | Tokio runtime conflicts with corosensei M:N scheduler. Would double thread count and create potential deadlocks. |
| PostgreSQL driver | Pure wire protocol | `postgres` (sync wrapper) | Still embeds internal Tokio runtime. Also 10K+ lines of code for features we do not need (COPY, notifications, TLS negotiation). |
| PostgreSQL driver | Pure wire protocol | `postgres_protocol` (low-level) | This is the wire protocol crate from rust-postgres. Could use it, but it still pulls in tokio-related dependencies transitively and is designed for async I/O. Rolling our own ~800 lines of sync TCP protocol code is cleaner. |
| PostgreSQL auth | `md5` + `sha2` + `hmac` + `pbkdf2` | `scram-rs` crate | Full SCRAM library for what is ~100 lines of code. Adds unnecessary dependency for a simple 4-message exchange. |
| JSON serde | Compile-time codegen | Runtime reflection (type info tables) | Reflection requires storing type metadata in compiled binaries, increasing binary size and adding runtime overhead. Codegen is zero-cost at runtime. |
| JSON serde | Compile-time codegen | Runtime `serde`-style visitor | Visitor pattern requires trait objects and dynamic dispatch. Snow uses static dispatch via monomorphization -- codegen fits naturally. |
| HTTP path params | Router-level extraction | Regex-based routing | Regex is a massive dependency (~10K lines). Path parameter matching is simple string splitting -- `:id` segments are just `split('/')` + name lookup. |
| HTTP middleware | Function chain | Tower-style middleware | Tower is async-first and generic over service types. Snow's middleware is simpler: `fn(Request) -> Result<Request, Response>`. |
| Parameterized queries | `List<DbValue>` sum type | Type-level query params | Type-level params would require generic database functions with dependent types. Sum type approach is simple, safe, and sufficient. |

## Installation

```bash
# Build runtime (now compiles SQLite from source via cc crate)
cargo build -p snow-rt

# Build compiler (unchanged)
cargo build -p snowc

# The cc crate needs a C compiler for SQLite compilation.
# On macOS: Xcode command line tools (pre-installed or `xcode-select --install`)
# On Linux: `apt install build-essential` or equivalent
# This is the same requirement as the existing LLVM toolchain.
```

## Sources

### Primary (HIGH confidence -- direct codebase analysis)
- `snow-rt/src/json.rs`: Existing SnowJson tagged union, `snow_json_from_*` functions, `snow_json_parse`, `snow_json_encode`
- `snow-rt/src/http/router.rs`: Existing router with exact match + wildcard patterns
- `snow-rt/src/http/server.rs`: Existing HTTP server with actor-per-connection, request/response structs
- `snow-codegen/src/mir/lower.rs` lines 1579-1601: Existing deriving dispatch (Debug, Eq, Ord, Hash, Display)
- `snow-codegen/src/codegen/intrinsics.rs`: Full intrinsic declaration pattern (512 lines, 80+ functions)
- `snow-codegen/src/link.rs`: Linker invocation via `cc` -- links `libsnow_rt.a`
- `snow-codegen/src/mir/mod.rs`: MIR type system, struct/sum type definitions

### Secondary (HIGH confidence -- official documentation)
- [PostgreSQL Wire Protocol v3](https://www.postgresql.org/docs/current/protocol.html) -- message formats, startup flow, extended query protocol
- [PostgreSQL Message Formats](https://www.postgresql.org/docs/current/protocol-message-formats.html) -- byte-level message structures
- [PostgreSQL Password Authentication](https://www.postgresql.org/docs/current/auth-password.html) -- MD5 and SCRAM-SHA-256 methods
- [SQLite C/C++ Interface](https://sqlite.org/cintro.html) -- sqlite3_open, sqlite3_prepare_v2, sqlite3_bind_*, sqlite3_step
- [SQLite Binding Values](https://sqlite.org/c3ref/bind_blob.html) -- bind function signatures and semantics
- [libsqlite3-sys crate](https://crates.io/crates/libsqlite3-sys) -- v0.36.0, bundled SQLite 3.51.1
- [libsqlite3-sys docs](https://docs.rs/libsqlite3-sys/latest/libsqlite3_sys/) -- Rust FFI function signatures

### Tertiary (MEDIUM confidence -- ecosystem research)
- [rust-postgres](https://github.com/sfackler/rust-postgres) -- reference implementation for PostgreSQL wire protocol in Rust
- [md5 crate](https://crates.io/crates/md5) -- v0.8.0, zero dependencies, 76M+ downloads
- [RustCrypto hashes](https://github.com/RustCrypto/hashes) -- sha2, hmac, pbkdf2 crate ecosystem
- [PgDog blog: Hacking the Postgres wire protocol](https://pgdog.dev/blog/hacking-postgres-wire-protocol) -- practical implementation guide

---
*Stack research for: Snow v2.0 Database & Serialization features*
*Researched: 2026-02-10*
