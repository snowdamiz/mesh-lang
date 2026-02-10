# Architecture Patterns

**Domain:** Database drivers, JSON serde, and HTTP enhancements for the Snow compiled language
**Researched:** 2026-02-10
**Confidence:** HIGH (all analysis based on direct codebase inspection + official protocol documentation)

## Recommended Architecture

### High-Level Integration Map

```
                   COMPILER PIPELINE
                   ================
  snow-parser        snow-typeck             snow-codegen/mir/lower.rs        snow-codegen/codegen
  -----------        -----------             -------------------------        --------------------
  DERIVING_CLAUSE    valid_derives           generate_to_json_struct()        intrinsics.rs
  already parsed     add "Json" to list      generate_from_json_struct()      add snow_json_object_*
  (NO changes)       register ToJson/        generate_to_json_sum()           add snow_sqlite_*
                     FromJson trait impls    generate_from_json_sum()         add snow_pg_*
                     Add Sqlite,Pg modules   (follow Hash/Debug pattern)      add snow_http_param/use
                     Add Http.param/use                                       Method-specific routes

                   RUNTIME (snow-rt)
                   =================
  json.rs            NEW: db/sqlite.rs       NEW: db/postgres.rs           http/router.rs
  -------            ----------------        -------------------           --------------
  Add:               sqlite3 C FFI:          PG wire protocol v3:          Path param
  snow_json_         sqlite3_open            TCP connect via               extraction with
  object_new         sqlite3_prepare_v2      std::net::TcpStream           :param patterns
  snow_json_         sqlite3_bind_*          StartupMessage                SnowMap of params
  object_put         sqlite3_step            MD5/cleartext auth            Method filtering
  snow_json_         sqlite3_column_*        Parse/Bind/Execute/Sync
  array_new          sqlite3_finalize        DataRow -> SnowList<SnowMap>  http/server.rs
  snow_json_         sqlite3_close                                         ---------------
  array_push         Params via bind_*       Params via Bind message       Middleware chain
  snow_json_as_*                                                           before handler
  snow_json_         Parameterized queries   Parameterized queries         SnowHttpRequest
  object_get         via sqlite3_bind_*      via Extended Query Protocol   + path_params field
```

### Component Boundaries

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| `snow-typeck/infer.rs` | Validate `deriving(Json)`, register `ToJson`/`FromJson` trait impls with method signatures; add `Sqlite`, `Pg` module types; extend `Http` module | snow-codegen MIR lowering via trait registry |
| `snow-codegen/mir/lower.rs` | Generate `ToJson__to_json__Type` and `FromJson__from_json__Type` MIR functions from struct/sum field metadata; register all new `snow_*` functions; map module names to runtime functions | Runtime via MIR Call nodes to `snow_*` functions |
| `snow-codegen/codegen/intrinsics.rs` | Declare all new `extern "C"` function signatures for LLVM IR | Links against `libsnow_rt.a` |
| `snow-rt/json.rs` | New runtime functions for structured JSON construction/extraction: `snow_json_object_new/put/get`, `snow_json_array_new/push`, `snow_json_as_*` | GC allocator, SnowString, existing SnowJson tagged union |
| `snow-rt/db/sqlite.rs` (NEW) | SQLite3 C FFI wrapper: open, prepare, bind, step, column, finalize, close | Links `libsqlite3` system library; returns `SnowResult` |
| `snow-rt/db/postgres.rs` (NEW) | Pure Rust PostgreSQL wire protocol v3 client: TCP connect, startup, auth, extended query | `std::net::TcpStream`; returns `SnowResult`/`SnowList` |
| `snow-rt/http/router.rs` | Extended path matching with `:param` segments, parameter extraction into `SnowMap`; method filtering; middleware storage | HTTP server, GC allocator, SnowMap |
| `snow-rt/http/server.rs` | Middleware chain execution before route handler dispatch; path_params population in request struct | Router, middleware function pointers |

---

## Data Flow: deriving(Json) -- Encode Path

### Struct Encoding (Full Depth)

```
Snow source:
  type Address = { street :: String, city :: String } deriving(Json)
  type User = { name :: String, age :: Int, addr :: Address } deriving(Json)

Parser (UNCHANGED):
  DERIVING_CLAUSE -> tokens include "Json"
  Parser already handles deriving(Json) syntactically -- no new SyntaxKind needed

Typeck (infer.rs):
  1. valid_derives extended: ["Eq", "Ord", "Display", "Debug", "Hash", "Json"]
     (two locations: struct + sum type validation blocks ~line 1775 and ~2073)
  2. Register ToJson impl for User:
     ImplMethodSig { has_self: true, param_tys: [], return_ty: Ty::Con("Json") }
  3. Register FromJson impl for User:
     ImplMethodSig { has_self: false, param_tys: [Ty::Con("Json")],
                     return_ty: Ty::result(Ty::Con("User"), Ty::string()) }

MIR lowering (lower_struct_def, ~line 1578):
  if derive_list.iter().any(|t| t == "Json"):
    generate_to_json_struct("User", [("name", String), ("age", Int), ("addr", Struct("Address"))])

  Emitted MirFunction "ToJson__to_json__User":
    body =
      let obj = Call(snow_json_object_new, [])
      let obj = Call(snow_json_object_put, [obj, "name", Call(snow_json_from_string, [self.name])])
      let obj = Call(snow_json_object_put, [obj, "age", Call(snow_json_from_int, [self.age])])
      let obj = Call(snow_json_object_put, [obj, "addr", Call(ToJson__to_json__Address, [self.addr])])
      obj

  Key: For field type Struct("Address"), emit recursive call to ToJson__to_json__Address.
  For field type Int, emit Call(snow_json_from_int, [field_val]).
  For field type String, emit Call(snow_json_from_string, [field_val]).
  For field type Bool, emit Call(snow_json_from_bool, [field_val]).
  For field type Float, emit Call(snow_json_from_float, [field_val]).

Codegen (intrinsics.rs + codegen/expr.rs):
  snow_json_object_new declared as () -> ptr
  snow_json_object_put declared as (ptr, ptr, ptr) -> ptr
  Codegen emits LLVM IR calls -- no special handling needed beyond standard Call codegen
```

### Struct Decoding (Full Depth)

```
MIR lowering:
  generate_from_json_struct("User", [("name", String), ("age", Int), ("addr", Struct("Address"))])

  Emitted MirFunction "FromJson__from_json__User":
    body =
      let name_json = Call(snow_json_object_get, [json, "name"])
      let name_result = Call(snow_json_as_string, [name_json])
      // name_result is SnowResult -- check for error
      Match { scrutinee: name_result,
        arms: [
          Ok(name_val) ->
            let age_json = Call(snow_json_object_get, [json, "age"])
            let age_result = Call(snow_json_as_int, [age_json])
            Match { scrutinee: age_result,
              arms: [
                Ok(age_val) ->
                  let addr_json = Call(snow_json_object_get, [json, "addr"])
                  let addr_result = Call(FromJson__from_json__Address, [addr_json])
                  Match { scrutinee: addr_result,
                    arms: [
                      Ok(addr_val) ->
                        ConstructVariant("Result", "Ok",
                          [StructLit("User", [("name", name_val), ("age", age_val), ("addr", addr_val)])])
                      Err(e) -> ConstructVariant("Result", "Err", [e])
                    ]
                  }
                Err(e) -> ConstructVariant("Result", "Err", [e])
              ]
            }
          Err(e) -> ConstructVariant("Result", "Err", [e])
        ]
      }

  This nested Match pattern propagates errors from any field extraction failure.
  Alternative: Use a flat sequence with early Return (like ? operator desugaring), but
  nested Match is safer because it does not require Return semantics in a non-function context.
```

### Sum Type Encoding

```
Snow source:
  type Shape =
    | Circle(Float)
    | Rectangle(Float, Float)
    | Point
  deriving(Json)

MIR lowering:
  generate_to_json_sum("Shape", variants)

  Emitted MirFunction "ToJson__to_json__Shape":
    body = Match { scrutinee: self, arms: [
      Constructor("Shape", "Circle", [Var("r", Float)]) ->
        let obj = Call(snow_json_object_new, [])
        let obj = Call(snow_json_object_put, [obj, "tag", Call(snow_json_from_string, ["Circle"])])
        let arr = Call(snow_json_array_new, [])
        let arr = Call(snow_json_array_push, [arr, Call(snow_json_from_float, [r])])
        Call(snow_json_object_put, [obj, "fields", arr])

      Constructor("Shape", "Rectangle", [Var("w", Float), Var("h", Float)]) ->
        let obj = Call(snow_json_object_new, [])
        let obj = Call(snow_json_object_put, [obj, "tag", Call(snow_json_from_string, ["Rectangle"])])
        let arr = Call(snow_json_array_new, [])
        let arr = Call(snow_json_array_push, [arr, Call(snow_json_from_float, [w])])
        let arr = Call(snow_json_array_push, [arr, Call(snow_json_from_float, [h])])
        Call(snow_json_object_put, [obj, "fields", arr])

      Constructor("Shape", "Point", []) ->
        let obj = Call(snow_json_object_new, [])
        Call(snow_json_object_put, [obj, "tag", Call(snow_json_from_string, ["Point"])])
    ] }

  JSON format: { "tag": "VariantName", "fields": [field1, field2, ...] }
  Fieldless variants omit the "fields" key.
```

### Generic Struct Support

```
For generic structs like Pair<A, B>:
  - Non-generic structs: generate_to_json_struct called immediately in lower_struct_def
  - Generic structs: deferred to ensure_monomorphized_struct_trait_fns
    (same pattern as Hash, Debug, Eq, Ord -- see line 1656-1681 in lower.rs)

  In ensure_monomorphized_struct_trait_fns, add:
    let has_json = self.trait_registry.has_impl("ToJson", typeck_ty);
    if has_json {
        self.generate_to_json_struct(&mangled, &fields);
        self.generate_from_json_struct(&mangled, &fields);
    }
```

---

## Data Flow: SQLite Query Path

```
Snow source:
  let db = Sqlite.open("app.db")?
  let rows = Sqlite.query(db, "SELECT name, age FROM users WHERE id = ?", [42])?
  Sqlite.close(db)

Runtime call chain:

  snow_sqlite_open("app.db")
    1. Convert SnowString to &str
    2. sqlite3_open_v2(path_cstr, &db, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE, null)
    3. Check return code: SQLITE_OK -> wrap db ptr in Box<SqliteConn>
    4. Return SnowResult { tag: 0, value: Box::into_raw(conn) as *mut u8 }

  snow_sqlite_query(conn_ptr, sql_string, params_list)
    1. Recover SqliteConn: &*(conn_ptr as *const SqliteConn)
    2. sqlite3_prepare_v2(conn.db, sql_cstr, sql_len, &stmt, null)
    3. Bind parameters from SnowList:
       for i in 0..list_length(params_list):
         let param = list_get(params_list, i)
         // Determine type by inspecting the SnowJson tag of the param:
         //   JSON_NUMBER (tag 2) -> sqlite3_bind_int64(stmt, i+1, val)
         //   JSON_STR (tag 3)    -> sqlite3_bind_text(stmt, i+1, str, len, TRANSIENT)
         //   JSON_BOOL (tag 1)   -> sqlite3_bind_int64(stmt, i+1, val)
         //   JSON_NULL (tag 0)   -> sqlite3_bind_null(stmt, i+1)
         //   Raw Int (not SnowJson) -> sqlite3_bind_int64 (detect by context)
    4. Get column names: sqlite3_column_count(stmt), sqlite3_column_name(stmt, col)
    5. Loop: while sqlite3_step(stmt) == SQLITE_ROW
         Build SnowMap: for each column:
           let col_type = sqlite3_column_type(stmt, col)
           match col_type:
             SQLITE_INTEGER -> snow_int_to_string(sqlite3_column_int64(stmt, col))
             SQLITE_FLOAT   -> snow_float_to_string(sqlite3_column_double(stmt, col))
             SQLITE_TEXT    -> snow_string_new(sqlite3_column_text(stmt, col), len)
             SQLITE_NULL    -> (skip or store empty string)
           snow_map_put(row_map, col_name_snow, col_val_snow)
         Append row_map to result_list
    6. sqlite3_finalize(stmt)
    7. Return SnowResult { tag: 0, value: result_list }

  snow_sqlite_close(conn_ptr)
    1. Recover Box<SqliteConn>: Box::from_raw(conn_ptr as *mut SqliteConn)
    2. sqlite3_close(conn.db)
    3. Box drops, freeing memory
```

### Parameter Type Strategy for SQLite

Parameters arrive as a `SnowList` of values. The question is how to determine the type of each parameter for binding.

**Recommended approach:** Parameters are passed as `SnowJson` values. The existing `snow_json_from_int`, `snow_json_from_string`, etc. functions create typed JSON values. The SQLite binder inspects the tag byte to dispatch to the correct `sqlite3_bind_*` call. This reuses existing infrastructure and avoids introducing a new tagged-value type.

```
Snow source:
  Sqlite.query(db, "SELECT * FROM users WHERE age > ? AND name = ?",
    [Json.from_int(21), Json.from_string("Alice")])

At runtime, the param list contains SnowJson ptrs:
  [SnowJson{tag:2, value:21}, SnowJson{tag:3, value:ptr_to_"Alice"}]

The sqlite binder reads each tag and calls the appropriate sqlite3_bind_* function.
```

### SQLite C FFI Declarations

```rust
// In snow-rt/src/db/sqlite.rs -- raw FFI declarations
extern "C" {
    fn sqlite3_open_v2(
        filename: *const c_char,
        ppDb: *mut *mut sqlite3,
        flags: c_int,
        zVfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close(db: *mut sqlite3) -> c_int;
    fn sqlite3_prepare_v2(
        db: *mut sqlite3,
        zSql: *const c_char,
        nByte: c_int,
        ppStmt: *mut *mut sqlite3_stmt,
        pzTail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_step(stmt: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_finalize(stmt: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut sqlite3_stmt, idx: c_int, val: i64) -> c_int;
    fn sqlite3_bind_double(stmt: *mut sqlite3_stmt, idx: c_int, val: f64) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut sqlite3_stmt,
        idx: c_int,
        val: *const c_char,
        n: c_int,
        destructor: isize,  // SQLITE_TRANSIENT = -1
    ) -> c_int;
    fn sqlite3_bind_null(stmt: *mut sqlite3_stmt, idx: c_int) -> c_int;
    fn sqlite3_column_count(stmt: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_column_type(stmt: *mut sqlite3_stmt, idx: c_int) -> c_int;
    fn sqlite3_column_int64(stmt: *mut sqlite3_stmt, idx: c_int) -> i64;
    fn sqlite3_column_double(stmt: *mut sqlite3_stmt, idx: c_int) -> f64;
    fn sqlite3_column_text(stmt: *mut sqlite3_stmt, idx: c_int) -> *const c_char;
    fn sqlite3_column_name(stmt: *mut sqlite3_stmt, idx: c_int) -> *const c_char;
    fn sqlite3_errmsg(db: *mut sqlite3) -> *const c_char;
}

// Opaque types
#[repr(C)] struct sqlite3 { _opaque: [u8; 0] }
#[repr(C)] struct sqlite3_stmt { _opaque: [u8; 0] }

// Constants
const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_OPEN_READWRITE: c_int = 0x00000002;
const SQLITE_OPEN_CREATE: c_int = 0x00000004;
const SQLITE_TRANSIENT: isize = -1;
const SQLITE_INTEGER: c_int = 1;
const SQLITE_FLOAT: c_int = 2;
const SQLITE_TEXT: c_int = 3;
const SQLITE_NULL: c_int = 5;
```

### SQLite Build System Integration

```
snow-rt/Cargo.toml changes:
  [build-dependencies]
  cc = "1"          # For finding/linking system sqlite3

snow-rt/build.rs (NEW):
  fn main() {
      // Link to system SQLite library
      println!("cargo:rustc-link-lib=sqlite3");
      // On macOS, sqlite3 is in the SDK
      // On Linux, requires libsqlite3-dev package
  }

snow-codegen/src/link.rs changes:
  Add "-lsqlite3" to the cc linker invocation
  (conditional on whether the program uses Sqlite module -- or always link it)
```

---

## Data Flow: PostgreSQL Wire Protocol

### Connection Sequence

```
snow_pg_connect("host=localhost port=5432 dbname=mydb user=snow password=secret")

  1. Parse connection string into key-value pairs
  2. TcpStream::connect(format!("{}:{}", host, port))
  3. Send StartupMessage:
     [int32 length][int32 protocol_version=196608 (3.0)]
     [str "user"][str username][str "database"][str dbname][byte 0]
  4. Read server response:
     Match first byte:
       'R' (Authentication):
         Read int32 subtype:
           0 -> AuthenticationOk (trust mode, done)
           3 -> AuthenticationCleartextPassword
                Send PasswordMessage: ['p'][int32 len][str password][0]
           5 -> AuthenticationMD5Password
                Read 4-byte salt
                md5_hex = md5(md5(password + username) + salt)
                Send PasswordMessage: ['p'][int32 len]["md5" + md5_hex][0]
           10 -> AuthenticationSASL (SCRAM) -- NOT IMPLEMENTED initially
                 Return error: "SCRAM-SHA-256 auth not yet supported"
       'E' (ErrorResponse):
         Parse error fields, return Err
  5. After AuthenticationOk, read parameter messages:
     Loop until ReadyForQuery:
       'S' (ParameterStatus) -> store server params (e.g., server_encoding)
       'K' (BackendKeyData) -> store process_id + secret_key (for cancel)
       'Z' (ReadyForQuery) -> done, connection ready
       'E' (ErrorResponse) -> connection failed
  6. Return SnowResult { tag: 0, value: Box::into_raw(PgConn) }
```

### Parameterized Query via Extended Protocol

```
snow_pg_query(conn_ptr, "SELECT name, age FROM users WHERE id = $1", [42])

  1. Recover PgConn: &mut *(conn_ptr as *mut PgConn)
  2. Send Parse message:
     ['P'][int32 length]["" (unnamed stmt)][sql_string][int16 0 (no param type hints)][0]
  3. Send Bind message:
     ['B'][int32 length]
     ["" (unnamed portal)]
     ["" (unnamed stmt)]
     [int16 0 (all params text format)]
     [int16 num_params]
     For each param:
       Convert SnowJson param to text representation:
         JSON_NUMBER -> int_to_string or float_to_string
         JSON_STR -> raw string bytes
         JSON_BOOL -> "t" or "f"
         JSON_NULL -> [int32 -1] (null indicator)
       [int32 param_len][param_bytes]
     [int16 0 (all result columns text format)]
  4. Send Describe message:
     ['D'][int32 length]['P' (portal)]["" (unnamed)][0]
  5. Send Execute message:
     ['E'][int32 length]["" (unnamed portal)][int32 0 (fetch all)]
  6. Send Sync message:
     ['S'][int32 4]

  7. Read responses:
     '1' (ParseComplete) -> ok
     '2' (BindComplete) -> ok
     'T' (RowDescription) ->
       [int16 num_fields]
       For each field:
         [str field_name][0]
         [int32 table_oid][int16 column_attr]
         [int32 type_oid][int16 type_len]
         [int32 type_modifier][int16 format_code]
       -> Store field_names and type_oids for result parsing
     'D' (DataRow) ->
       [int16 num_columns]
       For each column:
         [int32 col_len]  // -1 = NULL
         [col_len bytes]  // text-encoded value
       -> Build SnowMap: field_names[i] -> text value as SnowString
       -> Append to result_list
     'C' (CommandComplete) -> done with rows
     'E' (ErrorResponse) -> parse error, return Err
     'Z' (ReadyForQuery) -> query complete

  8. Return SnowResult { tag: 0, value: result_list (SnowList of SnowMap) }
```

### PostgreSQL Internal Structure

```rust
struct PgConn {
    stream: TcpStream,
    process_id: i32,
    secret_key: i32,
    server_params: HashMap<String, String>,
    // Read buffer for partial message assembly
    read_buf: Vec<u8>,
}

// Message reading helper
fn read_message(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), String> {
    let mut tag = [0u8; 1];
    stream.read_exact(&mut tag)?;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = i32::from_be_bytes(len_buf) as usize - 4;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok((tag[0], body))
}

// Message writing helper
fn write_message(stream: &mut TcpStream, tag: u8, body: &[u8]) -> Result<(), String> {
    stream.write_all(&[tag])?;
    let len = (body.len() + 4) as i32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(body)?;
    Ok(())
}
```

### MD5 Authentication

```rust
fn md5_auth(user: &str, password: &str, salt: &[u8; 4]) -> String {
    // Step 1: md5(password + username)
    let inner = md5::compute(format!("{}{}", password, user));
    let inner_hex = format!("{:x}", inner);
    // Step 2: md5(inner_hex + salt)
    let mut outer_input = inner_hex.into_bytes();
    outer_input.extend_from_slice(salt);
    let outer = md5::compute(&outer_input);
    format!("md5{:x}", outer)
}
```

---

## Data Flow: HTTP Path Parameters

```
Snow source:
  let router = Http.router()
    |> Http.get("/users/:id", handle_user)
    |> Http.get("/users/:id/posts/:post_id", handle_post)

  fn handle_user(req :: Request) -> Response =
    let id = Http.param(req, "id")   // returns Option<String>
    ...

Router matching (modified router.rs):

  RouteEntry gains:
    pub method: Option<String>,     // None = any method, Some("GET") = GET only
    pub has_params: bool,           // Quick check: does pattern contain ':'?

  fn matches_pattern(pattern: &str, path: &str) -> Option<Vec<(String, String)>>:
    // Split both by '/'
    let pat_segs: Vec<&str> = pattern.split('/').collect();
    let path_segs: Vec<&str> = path.split('/').collect();
    if pat_segs.len() != path_segs.len() { return None; }
    let mut params = Vec::new();
    for (pat, val) in pat_segs.iter().zip(path_segs.iter()) {
        if pat.starts_with(':') {
            params.push((pat[1..].to_string(), val.to_string()));
        } else if pat != val {
            return None;
        }
    }
    Some(params)

  match_route returns: Option<(handler_fn, handler_env, params_map)>
    where params_map is a freshly-allocated SnowMap from extracted params

SnowHttpRequest change (APPEND to end, not insert in middle):
  #[repr(C)]
  pub struct SnowHttpRequest {
      pub method: *mut u8,
      pub path: *mut u8,
      pub body: *mut u8,
      pub query_params: *mut u8,
      pub headers: *mut u8,
      pub path_params: *mut u8,     // NEW: appended at end for ABI compat
  }

Server wiring (handle_request):
  let (handler_fn, handler_env, params) = router.match_route(path_str, method_str)?;
  (*snow_req).path_params = params_map;  // populate before calling handler

Accessor:
  #[no_mangle]
  pub extern "C" fn snow_http_request_param(req: *mut u8, name: *const SnowString) -> *mut u8 {
      // Same pattern as snow_http_request_query -- lookup in path_params SnowMap
      // Returns SnowOption (tag 0 = Some, tag 1 = None)
  }
```

---

## Data Flow: HTTP Middleware Chain

```
Snow source:
  let router = Http.router()
    |> Http.use(log_middleware)
    |> Http.use(auth_middleware)
    |> Http.get("/api/users", list_users)

  fn log_middleware(req :: Request, next :: fn(Request) -> Response) -> Response =
    IO.eprintln("=> " ++ Http.method(req) ++ " " ++ Http.path(req))
    let resp = next(req)
    resp

  fn auth_middleware(req :: Request, next :: fn(Request) -> Response) -> Response =
    case Http.header(req, "Authorization") do
      Some(token) -> next(req)
      None -> Http.response(401, "Unauthorized")
    end

Middleware type:
  fn(Request, fn(Request) -> Response) -> Response
  At C ABI level: fn(req: *mut u8, next_fn: *mut u8, next_env: *mut u8) -> *mut u8

Router storage:
  struct SnowRouter {
      routes: Vec<RouteEntry>,
      middlewares: Vec<MiddlewareEntry>,   // NEW
  }

  struct MiddlewareEntry {
      fn_ptr: *mut u8,
      env_ptr: *mut u8,   // null for bare functions
  }

Registration:
  #[no_mangle]
  pub extern "C" fn snow_http_use(router: *mut u8, mw_fn: *mut u8) -> *mut u8 {
      // Clone routes + middlewares, append new middleware, return new router
  }

Dispatch (in handle_request):
  1. Match route to get handler
  2. Build chain from inside out:

     // Start with the actual handler
     let mut current_fn = handler_fn;
     let mut current_env = handler_env;

     // Wrap with each middleware, innermost first (reverse order)
     for mw in router.middlewares.iter().rev() {
         // Create a closure that captures current_fn/current_env
         // The "next" function is: |req| call(current_fn, current_env, req)
         // The middleware call is: mw.fn_ptr(req, next_fn, next_env)
         // Implementation: allocate a NextClosure struct on stack/GC
         // containing the captured fn_ptr and env_ptr
     }

  3. Call outermost chain(request) -> response

Implementation detail:
  The "next" function passed to middleware is a closure-like pair (fn_ptr, env_ptr).
  We allocate a small struct on the actor GC heap:

  #[repr(C)]
  struct NextChain {
      handler_fn: *mut u8,
      handler_env: *mut u8,
  }

  extern "C" fn next_chain_trampoline(env: *mut u8, req: *mut u8) -> *mut u8 {
      let chain = &*(env as *const NextChain);
      if chain.handler_env.is_null() {
          let f: fn(*mut u8) -> *mut u8 = transmute(chain.handler_fn);
          f(req)
      } else {
          let f: fn(*mut u8, *mut u8) -> *mut u8 = transmute(chain.handler_fn);
          f(chain.handler_env, req)
      }
  }

  // For each middleware, wrap:
  let next_env = alloc NextChain { handler_fn: current_fn, handler_env: current_env };
  let next_fn = next_chain_trampoline as *mut u8;
  // Now the middleware receives (req, next_fn, next_env)
```

---

## Patterns to Follow

### Pattern 1: Deriving Trait Generation (established pattern)

**What:** The existing `generate_hash_struct` / `generate_debug_inspect_struct` pattern in `mir/lower.rs` is the exact template for `generate_to_json_struct` and `generate_from_json_struct`.

**When:** Implementing `deriving(Json)` for structs and sum types.

**How it works (from codebase analysis):**
1. Typeck adds "Json" to `valid_derives` list and registers trait impls with method signatures
2. MIR lowering checks `derive_list.iter().any(|t| t == "Json")`
3. Generator method builds MIR function body by iterating struct fields
4. Uses `MirExpr::FieldAccess` to read fields, `MirExpr::Call` to invoke runtime helpers
5. Function named `ToJson__to_json__StructName` per double-underscore convention
6. Function + type inserted into `self.functions` and `self.known_functions`
7. For generics: `ensure_monomorphized_struct_trait_fns` generates at instantiation (line 1619-1688)

**Concrete example pattern from Hash derive (line 2658-2713):**
```rust
fn generate_hash_struct(&mut self, name: &str, fields: &[(String, MirType)]) {
    let mangled = format!("Hash__hash__{}", name);
    let struct_ty = MirType::Struct(name.to_string());
    let self_var = MirExpr::Var("self".to_string(), struct_ty.clone());
    // ... iterate fields, build MirExpr tree with Call + FieldAccess ...
    let func = MirFunction {
        name: mangled.clone(),
        params: vec![("self".to_string(), struct_ty.clone())],
        return_type: MirType::Int,
        body,
        is_closure_fn: false,
        captures: vec![],
        has_tail_calls: false,
    };
    self.functions.push(func);
    self.known_functions.insert(mangled, MirType::FnPtr(vec![struct_ty], Box::new(MirType::Int)));
}
```

### Pattern 2: Three-Point Runtime Function Registration

**What:** Every new runtime function must be registered in exactly three places.

**Points:**
1. **`snow-rt/`**: `#[no_mangle] pub extern "C" fn snow_foo(...)` implementation
2. **`snow-codegen/codegen/intrinsics.rs`**: `module.add_function("snow_foo", ..., Linkage::External)`
3. **`snow-codegen/mir/lower.rs`**: `self.known_functions.insert("snow_foo", MirType::FnPtr(...))`
   Plus module name mapping in the resolution function (around line 7728)

### Pattern 3: Opaque Handle for Non-GC Resources

**What:** Database connections are heap-allocated with `Box::into_raw()`, not GC-allocated.

**Why:** The GC has no finalizer mechanism. A GC-collected SqliteConn would leak the file descriptor (sqlite3_close never called) and the TCP socket (PgConn stream never shutdown).

**Implementation:** `Box::into_raw(Box::new(SqliteConn { ... })) as *mut u8` on creation, `Box::from_raw(ptr as *mut SqliteConn)` on close. This matches the router pattern (line 56-59 in router.rs).

### Pattern 4: SnowResult Return Convention

**What:** All fallible operations return `*mut SnowResult` (tag 0 = Ok, tag 1 = Err). Err payload is always `*mut SnowString`.

**Already used by:** `snow_json_parse`, `snow_file_read`, `snow_http_get`, `snow_http_post`.

### Pattern 5: Module Namespace Mapping

**What:** `Json.parse(s)` maps to `snow_json_parse` via the module resolution in `lower.rs`.

**Where to add new modules:**
1. `typeck/infer.rs` around line 435 (module initialization): add `Sqlite`, `Pg` module entries with function types
2. `lower.rs` function resolution (around line 7728): add `"sqlite_open" => "snow_sqlite_open"` etc.
3. `lower.rs` `known_functions` init: register all function signatures

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: GC-Allocating Database Handles

**What:** Using `snow_gc_alloc_actor` for SQLite or PostgreSQL connection objects.

**Why bad:** The Snow GC is a mark-sweep collector with no destructor/finalization support. If a connection handle is collected, `sqlite3_close()` / TCP `shutdown()` never runs. This leaks file descriptors, database locks, and TCP connections.

**Instead:** Use `Box::into_raw()` for all resource handles. Require explicit `Sqlite.close(db)` / `Pg.disconnect(conn)` calls in Snow programs.

### Anti-Pattern 2: Simple Query Protocol for PostgreSQL Parameterized Queries

**What:** Using PostgreSQL's Simple Query protocol (`'Q'` message) and interpolating parameters into SQL strings.

**Why bad:** SQL injection. No type safety. No prepared statement reuse. Defeats the purpose of parameterized queries.

**Instead:** Always use Extended Query protocol (Parse/Bind/Execute/Sync) for parameterized queries. Use Simple Query only for non-parameterized utility commands if ever needed.

### Anti-Pattern 3: Implementing SCRAM-SHA-256 in First Iteration

**What:** Full SCRAM-SHA-256 authentication for PostgreSQL.

**Why bad:** SCRAM requires HMAC-SHA-256, PBKDF2 key derivation, channel binding, and a multi-round message exchange. This is substantial cryptographic code. Most local development uses `trust`, `password`, or `md5` auth.

**Instead:** Start with `trust` (no auth), cleartext password, and MD5. MD5 auth is a single hash computation. Return a clear error message if the server requests SCRAM: "SCRAM-SHA-256 authentication not yet supported; configure pg_hba.conf for md5 or trust."

### Anti-Pattern 4: Inserting path_params in the Middle of SnowHttpRequest

**What:** Adding the `path_params` field between existing fields in the `#[repr(C)]` struct.

**Why bad:** Codegen accesses struct fields by byte offset via GEP. Inserting a field shifts all subsequent offsets, breaking any compiled Snow programs that access `headers` or `query_params`.

**Instead:** Append `path_params` as the LAST field. Programs that do not use path params never read that field, so appending is ABI-compatible.

### Anti-Pattern 5: Regex-Based Path Parameter Matching

**What:** Compiling route patterns like `/users/:id` into regexes for matching.

**Why bad:** Regex is heavy for simple segment matching. The route table is typically small (< 50 routes). Adds a regex dependency. Over-engineered for the use case.

**Instead:** Simple segment-by-segment comparison. Split both pattern and path by `/`. Compare segments. If a pattern segment starts with `:`, it is a named parameter -- extract the value from the corresponding path segment. O(segments) per match, which is fast for typical URL depths (3-5 segments).

### Anti-Pattern 6: Naming Conflicts with Existing HTTP Functions

**What:** Using `snow_http_get` and `snow_http_post` for both the HTTP client (existing) and route registration (new).

**Why bad:** Symbol collision at link time. The functions have different signatures.

**Instead:** Keep existing client functions as `snow_http_get` / `snow_http_post`. Use `snow_http_route_get` / `snow_http_route_post` for method-specific route registration. Or rename the route registration to use the existing `snow_http_route` function with an additional method parameter.

---

## New Components Required

### New Runtime Modules

| Module | File | Purpose | External Dependencies |
|--------|------|---------|----------------------|
| `db` | `snow-rt/src/db/mod.rs` | Database module root | None |
| `db::sqlite` | `snow-rt/src/db/sqlite.rs` | SQLite C FFI wrapper | `libsqlite3` (system, linked via `build.rs`) |
| `db::postgres` | `snow-rt/src/db/postgres.rs` | PG wire protocol client | `std::net::TcpStream`, `md5` crate |

### New Runtime Functions: JSON Serde Support

| Function | Signature (C ABI) | Purpose |
|----------|-------------------|---------|
| `snow_json_object_new()` | `() -> *mut u8` | Create empty SnowJson Object (tag=5, value=empty SnowMap) |
| `snow_json_object_put(obj, key, val)` | `(ptr, ptr, ptr) -> ptr` | Add key(SnowString)-value(SnowJson) to object |
| `snow_json_object_get(obj, key)` | `(ptr, ptr) -> ptr` | Get SnowJson value by key (null if missing) |
| `snow_json_array_new()` | `() -> *mut u8` | Create empty SnowJson Array (tag=4, value=empty SnowList) |
| `snow_json_array_push(arr, val)` | `(ptr, ptr) -> ptr` | Append SnowJson value to array |
| `snow_json_as_int(json)` | `(ptr) -> *mut SnowResult` | Extract i64 from Number, Err if wrong tag |
| `snow_json_as_float(json)` | `(ptr) -> *mut SnowResult` | Extract f64 from Number, Err if wrong tag |
| `snow_json_as_string(json)` | `(ptr) -> *mut SnowResult` | Extract SnowString from Str, Err if wrong tag |
| `snow_json_as_bool(json)` | `(ptr) -> *mut SnowResult` | Extract i8 from Bool, Err if wrong tag |

### New Runtime Functions: SQLite

| Function | Signature (C ABI) | Purpose |
|----------|-------------------|---------|
| `snow_sqlite_open(path)` | `(ptr) -> *mut SnowResult` | Open SQLite database, return opaque handle |
| `snow_sqlite_close(conn)` | `(ptr) -> ()` | Close database, free handle |
| `snow_sqlite_execute(conn, sql, params)` | `(ptr, ptr, ptr) -> *mut SnowResult` | Execute INSERT/UPDATE/DELETE, return affected rows |
| `snow_sqlite_query(conn, sql, params)` | `(ptr, ptr, ptr) -> *mut SnowResult` | Execute SELECT, return List<Map<String,String>> |

### New Runtime Functions: PostgreSQL

| Function | Signature (C ABI) | Purpose |
|----------|-------------------|---------|
| `snow_pg_connect(conn_str)` | `(ptr) -> *mut SnowResult` | Connect to PG, return opaque handle |
| `snow_pg_disconnect(conn)` | `(ptr) -> ()` | Close TCP connection, free handle |
| `snow_pg_execute(conn, sql, params)` | `(ptr, ptr, ptr) -> *mut SnowResult` | Execute non-SELECT, return affected rows |
| `snow_pg_query(conn, sql, params)` | `(ptr, ptr, ptr) -> *mut SnowResult` | Execute SELECT, return List<Map<String,String>> |

### New Runtime Functions: HTTP Enhancements

| Function | Signature (C ABI) | Purpose |
|----------|-------------------|---------|
| `snow_http_request_param(req, name)` | `(ptr, ptr) -> ptr` | Get path param by name (SnowOption) |
| `snow_http_use(router, mw_fn)` | `(ptr, ptr) -> ptr` | Add middleware, return new router |
| `snow_http_route_get(router, pattern, handler)` | `(ptr, ptr, ptr) -> ptr` | Add GET-only route |
| `snow_http_route_post(router, pattern, handler)` | `(ptr, ptr, ptr) -> ptr` | Add POST-only route |
| `snow_http_route_put(router, pattern, handler)` | `(ptr, ptr, ptr) -> ptr` | Add PUT-only route |
| `snow_http_route_delete(router, pattern, handler)` | `(ptr, ptr, ptr) -> ptr` | Add DELETE-only route |

### Existing Components That Need Modification

| Component | File | Change | Complexity |
|-----------|------|--------|------------|
| **Typeck** | `snow-typeck/src/infer.rs` | Add "Json" to `valid_derives` (2 locations, ~line 1775 and ~2073). Register ToJson/FromJson trait impls. Add Sqlite, Pg module types. Extend Http module. | Medium |
| **Typeck** | `snow-typeck/src/error.rs` | Update UnsupportedDerive help to include "Json" | Trivial |
| **Typeck** | `snow-typeck/src/diagnostics.rs` | Update help text (line 1379) | Trivial |
| **MIR Lowering** | `snow-codegen/src/mir/lower.rs` | Add generate_to_json_struct/sum, generate_from_json_struct/sum. Add emit_to_json_for_type/emit_from_json_for_type helpers. Add Json to derive checks in lower_struct_def (~line 1585) and lower_sum_type_def (~line 1732). Add Json to ensure_monomorphized_struct_trait_fns (~line 1658). Register all new snow_* in known_functions (~line 622). Add module mappings (~line 7728). | High |
| **Codegen** | `snow-codegen/src/codegen/intrinsics.rs` | Declare ~25 new extern functions with LLVM types | Low (mechanical) |
| **Linker** | `snow-codegen/src/link.rs` | Add `-lsqlite3` to linker flags | Low |
| **Runtime JSON** | `snow-rt/src/json.rs` | Add snow_json_object_new/put/get, snow_json_array_new/push, snow_json_as_* | Medium |
| **Runtime Router** | `snow-rt/src/http/router.rs` | Add :param segment matching, method filtering, middleware storage | Medium |
| **Runtime Server** | `snow-rt/src/http/server.rs` | Append path_params to SnowHttpRequest. Middleware chain in handle_request. | Medium |
| **Runtime Cargo** | `snow-rt/Cargo.toml` | Add `md5` crate dependency | Trivial |
| **Runtime lib** | `snow-rt/src/lib.rs` | Add `pub mod db;`, re-export new functions | Trivial |

---

## Suggested Build Order

Build order is driven by dependency chains, testability, and risk management:

### Phase A: JSON Serde Runtime Helpers + deriving(Json) for Structs

**Rationale:** Self-contained. Touches all compiler layers but uses established patterns. JSON serde is a prerequisite for database result handling (query results use JSON-like typed values for parameter binding). No external dependencies.

1. Runtime JSON helpers in `snow-rt/json.rs` (pure additions, unit-testable)
2. Intrinsics + known_functions registration
3. Typeck: valid_derives + trait impls
4. MIR: generate_to_json_struct + generate_from_json_struct
5. E2E test: struct roundtrip through JSON

### Phase B: deriving(Json) for Sum Types + Generics

**Rationale:** Extends Phase A to full language coverage. Sum types use Constructor patterns in MIR which are already well-established.

1. MIR: generate_to_json_sum + generate_from_json_sum
2. Generic support: ensure_monomorphized_struct_trait_fns
3. E2E test: sum type + nested generic struct roundtrip

### Phase C: HTTP Path Parameters

**Rationale:** Small, contained runtime-only change. No compiler changes needed. Enables building REST APIs. Needed before middleware.

1. Router path param extraction
2. SnowHttpRequest.path_params field (appended)
3. Server wiring + accessor function
4. Registration in intrinsics/lower/typeck

### Phase D: HTTP Middleware

**Rationale:** Builds on closures (working) and path params (Phase C). The middleware chain is a runtime-only feature using the existing closure calling convention.

1. Router middleware storage + snow_http_use
2. Middleware chain execution in handle_request
3. Method-specific routes (snow_http_route_get etc.)
4. Registration in intrinsics/lower/typeck

### Phase E: SQLite Driver

**Rationale:** C FFI is well-understood. Single-file database with no network complexity. Establishes parameterized query patterns reused by PG.

1. Build system (Cargo.toml, build.rs, link.rs)
2. FFI declarations
3. Open/Close
4. Query with parameter binding
5. Execute (INSERT/UPDATE/DELETE)
6. Registration in intrinsics/lower/typeck

### Phase F: PostgreSQL Driver

**Rationale:** Most complex -- pure Rust TCP + binary wire protocol. Depends on patterns established by SQLite. Risk contained to runtime only.

1. Wire protocol message encoding/decoding primitives
2. TCP connect + StartupMessage + auth (trust, cleartext, MD5)
3. Extended query: Parse/Bind/Execute/Sync
4. Result parsing: RowDescription + DataRow -> List<Map>
5. Registration in intrinsics/lower/typeck

```
Dependency graph:

Phase A: JSON Struct Serde -----> Phase B: JSON Sum + Generics
                                         |
Phase C: HTTP Path Params ------> Phase D: HTTP Middleware
                                         |
Phase E: SQLite Driver ----------> Phase F: PostgreSQL Driver
   (uses JSON for param types)      (uses same param pattern)
```

---

## Scalability Considerations

| Concern | At 100 users | At 10K users | At 1M users |
|---------|-------------|--------------|-------------|
| JSON serde | Negligible -- small SnowJson allocations | Allocation pressure from deep struct trees | Consider streaming encoder for large payloads |
| SQLite connections | Single-process, single-writer is fine | WAL mode helps concurrent readers | Not designed for this; use PostgreSQL |
| PostgreSQL connections | One conn per actor is fine | Connection pool needed (future milestone) | Pool + connection limits essential |
| HTTP middleware chain | Negligible per-request overhead | Fine -- O(middlewares) function calls | Consider compiled/inlined middleware |
| Path param extraction | O(segments) string split per request | Fine | Fine |
| Prepared statement cache | N/A (statements prepared per query) | Named prepared statements for hot queries (future) | Statement cache pool |

---

## Sources

- **Snow codebase:** Direct reading of `snow-typeck/src/infer.rs` (deriving validation, lines 1770-1888, 2065-2181), `snow-codegen/src/mir/lower.rs` (MIR generation patterns, lines 1574-1688, 2658-2713), `snow-codegen/src/codegen/intrinsics.rs` (function declaration patterns, lines 339-424), `snow-rt/src/json.rs` (existing JSON runtime), `snow-rt/src/http/router.rs` (current router), `snow-rt/src/http/server.rs` (request handling), `snow-rt/src/lib.rs` (module organization) -- HIGH confidence
- **PostgreSQL Wire Protocol v3:** [Chapter 54, Official Docs](https://www.postgresql.org/docs/current/protocol.html) -- HIGH confidence
- **PostgreSQL Message Formats:** [Section 54.7](https://www.postgresql.org/docs/current/protocol-message-formats.html) -- HIGH confidence
- **PostgreSQL Extended Query Protocol:** [Section 54.2 Message Flow](https://www.postgresql.org/docs/current/protocol-flow.html) -- HIGH confidence
- **PostgreSQL Authentication:** [Section 54.3 SASL Authentication](https://www.postgresql.org/docs/current/sasl-authentication.html), [Section 20.5 Password Authentication](https://www.postgresql.org/docs/current/auth-password.html) -- HIGH confidence
- **SQLite C/C++ Interface:** [sqlite.org/cintro.html](https://sqlite.org/cintro.html) -- HIGH confidence
- **HTTP Middleware Patterns:** [Leapcell: Middleware as Chain of Responsibility](https://leapcell.io/blog/unpacking-middleware-in-web-frameworks-a-chain-of-responsibility-deep-dive) -- MEDIUM confidence (well-established pattern)
