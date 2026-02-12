# Architecture Patterns

**Domain:** Production backend features for Snow programming language (connection pooling, TLS/SSL, transactions, struct-to-row mapping)
**Researched:** 2026-02-12
**Confidence:** HIGH (based on direct codebase inspection of all 11 crates + official PostgreSQL/Rustls documentation)

## Current Architecture Summary

The Snow runtime (`snow-rt`) is a static library linked into compiled Snow binaries. It exposes `extern "C"` functions that LLVM-generated code calls directly. New features integrate at up to four layers:

1. **typeck builtins** (`snow-typeck/src/builtins.rs`) -- Snow-level type signatures
2. **MIR lowering** (`snow-codegen/src/mir/lower.rs`) -- name mapping + known function registration
3. **LLVM intrinsics** (`snow-codegen/src/codegen/intrinsics.rs`) -- LLVM function declarations
4. **Runtime implementation** (`snow-rt/src/`) -- actual Rust code

The existing pattern for adding new runtime functionality is well-established: SQLite, PostgreSQL, HTTP, and JSON all follow the same four-layer integration path.

## Recommended Architecture

### High-Level Integration Map

```
FEATURE              RUNTIME (snow-rt)           COMPILER              SNOW API
=======              =============               ========              ========

Connection Pool      db/pool.rs (NEW)            intrinsics.rs         Pool.open(url, opts)
                     PoolConn wrapper            builtins.rs           Pool.acquire(pool)
                     Actor-based pool mgr        lower.rs              Pool.release(pool, conn)
                                                                       Pool.with(pool, fn)

TLS/PostgreSQL       db/pg.rs (MODIFY)           (none)                Pg.connect("...?sslmode=require")
                     rustls StreamOwned                                (transparent to user)
                     SSLRequest flow

TLS/HTTPS            http/server.rs (MODIFY)     (none)                HTTP.serve_tls(r, port, cert, key)
                     tiny_http ssl-rustls

Transactions         db/pg.rs (MODIFY)           intrinsics.rs         Pg.begin(conn)
                     db/sqlite.rs (MODIFY)       builtins.rs           Pg.commit(conn)
                     BEGIN/COMMIT/ROLLBACK        lower.rs             Pg.rollback(conn)
                                                                       Pg.transaction(conn, fn)

Struct-to-Row        mir/lower.rs (MODIFY)       lower.rs (MODIFY)    deriving(Row)
                     db/row.rs (NEW)             builtins.rs           Pg.query_as<T>(conn, sql, params)
                     Column name mapping          intrinsics.rs
```

### Component Boundaries

| Component | Responsibility | Communicates With | New/Modified |
|-----------|---------------|-------------------|--------------|
| `db/pool.rs` | Connection pool lifecycle, checkout/return, health checks | `db/pg.rs`, `db/sqlite.rs`, actor scheduler | **NEW** |
| `db/pg.rs` | PostgreSQL wire protocol, TLS upgrade, transactions | `db/pool.rs`, rustls | **MODIFY** |
| `db/sqlite.rs` | SQLite C FFI, transactions | `db/pool.rs` | **MODIFY** |
| `db/row.rs` | Row-to-struct decode helpers | `db/pg.rs`, `db/sqlite.rs` | **NEW** |
| `http/server.rs` | HTTPS via tiny_http ssl-rustls | rustls (via tiny_http) | **MODIFY** |
| `mir/lower.rs` | Generate `deriving(Row)` MIR functions | codegen pipeline | **MODIFY** |
| `builtins.rs` | Type signatures for new functions | typeck | **MODIFY** |
| `intrinsics.rs` | LLVM declarations for new runtime functions | LLVM IR module | **MODIFY** |

### Data Flow

#### Connection Pooling

```
Snow program                    Runtime (snow-rt)
-----------                     -----------------
let pool = Pool.open(url, 4)  --> snow_pool_open(url, size)
                                   |-- allocates Pool { connections: Vec<PgConn>, waiters: VecDeque }
                                   |-- pre-connects `size` connections
                                   |-- returns pool handle (u64, Box::into_raw pattern)

let conn = Pool.acquire(pool) --> snow_pool_acquire(pool_handle)
                                   |-- tries connections vec for idle conn
                                   |-- if none: block current actor (yield loop, like actor receive)
                                   |-- returns conn handle (u64)

Pool.release(pool, conn)      --> snow_pool_release(pool_handle, conn_handle)
                                   |-- returns conn to idle vec
                                   |-- wakes waiting actors if any

Pool.with(pool, fn)           --> snow_pool_with(pool_handle, fn_ptr, fn_env)
                                   |-- acquire
                                   |-- call fn(conn)
                                   |-- release (even on panic, via catch_unwind)
                                   |-- return fn result
```

#### TLS for PostgreSQL (SSLRequest Upgrade)

```
snow_pg_connect(url)
  |-- parse URL, extract sslmode from query params
  |-- TcpStream::connect(addr)
  |
  |-- IF sslmode != "disable":
  |     |-- Send SSLRequest message (8 bytes: length=8, code=80877103)
  |     |-- Read 1 byte response
  |     |-- IF 'S':
  |     |     |-- Create rustls::ClientConfig with webpki roots
  |     |     |-- Create rustls::ClientConnection
  |     |     |-- Wrap TcpStream in rustls::StreamOwned
  |     |     |-- Store TlsStream enum variant in PgConn
  |     |-- IF 'N' and sslmode == "require":
  |     |     |-- Return error
  |     |-- IF 'N' and sslmode == "prefer":
  |     |     |-- Continue with plain TCP
  |
  |-- Send StartupMessage (over plain or TLS stream)
  |-- Continue authentication (same as today, but Read/Write on enum stream)
```

#### TLS for HTTPS

```
snow_http_serve_tls(router, port, cert_ptr, key_ptr)
  |-- Extract cert/key bytes from SnowString pointers
  |-- Call tiny_http::Server::https(addr, SslConfig { certificate, private_key })
  |-- Rest identical to snow_http_serve (actor-per-connection dispatch)
```

#### Transactions

```
snow_pg_begin(conn_handle)    --> Send "BEGIN" via Extended Query Protocol
                                   |-- Parse("BEGIN") + Bind() + Execute() + Sync()
                                   |-- Wait for ReadyForQuery with 'T' (in transaction)
                                   |-- Return Result<Unit, String>

snow_pg_commit(conn_handle)   --> Send "COMMIT" via same flow
                                   |-- Wait for ReadyForQuery with 'I' (idle)

snow_pg_rollback(conn_handle) --> Send "ROLLBACK" via same flow

snow_pg_transaction(conn_handle, fn_ptr, fn_env)
  |-- begin
  |-- result = catch_unwind(fn(conn))
  |-- if Ok: commit
  |-- if Err: rollback
  |-- return result
```

#### Struct-to-Row Mapping (deriving(Row))

```
COMPILER TIME (mir/lower.rs):
  struct User deriving(Row)
    |-- Generate "FromRow__from_row__User" MIR function
    |-- For each field (name, id, email):
    |     |-- Emit: map_get(row_map, "name") -> string value
    |     |-- Emit: type conversion (String->Int, String->Bool, etc.)
    |     |-- Emit: field assignment
    |-- Emit: StructLit with all fields
    |-- Pattern follows generate_from_json_struct exactly

RUNTIME (snow-rt):
  snow_pg_query_as(conn, sql, params, from_row_fn)
    |-- Execute query (same as snow_pg_query)
    |-- For each row (Map<String, String>):
    |     |-- Call from_row_fn(row_map) -> struct pointer
    |-- Return List<T> instead of List<Map<String, String>>
```

## Patterns to Follow

### Pattern 1: Four-Layer Integration (Established)

Every new runtime function follows this exact path:

**What:** Add type signature in builtins, register in MIR lowering, declare LLVM intrinsic, implement in runtime.
**When:** Any new Snow-accessible function.

```
1. snow-typeck/src/builtins.rs     -- Ty::fun(params, return_type)
2. snow-codegen/src/mir/lower.rs   -- known_functions.insert + name mapping
3. snow-codegen/src/codegen/intrinsics.rs -- module.add_function
4. snow-rt/src/...                 -- extern "C" fn implementation
```

This pattern is proven by sqlite (4 functions), postgres (4 functions), http (10+ functions), json (11 functions). Total consistency across 30+ runtime functions.

### Pattern 2: Opaque Handle as u64 (Established)

**What:** Complex Rust objects (connections, pools) are `Box::into_raw` as u64 handles, GC-safe because GC never traces integers.
**When:** Any runtime-managed stateful resource.

```rust
// Create handle
let obj = Box::new(MyResource { ... });
let handle = Box::into_raw(obj) as u64;

// Use handle
let resource = unsafe { &mut *(handle as *mut MyResource) };

// Free handle
let resource = unsafe { Box::from_raw(handle as *mut MyResource) };
```

Pool handles, connection handles, and transaction contexts all use this pattern. It is the **only** safe way to expose Rust heap objects to Snow's GC.

### Pattern 3: SnowResult Return Convention (Established)

**What:** Functions that can fail return `*mut u8` pointing to `SnowResult { tag: u8, value: *mut u8 }`. Tag 0 = Ok, tag 1 = Err.
**When:** Any fallible operation.

All new pool/transaction/TLS functions follow this pattern for error reporting.

### Pattern 4: Deriving Code Generation (Established for Json, extend for Row)

**What:** The compiler generates MIR functions at struct definition time based on `deriving(Trait)` clauses.
**When:** `deriving(Row)` on a struct.

The `generate_from_json_struct` function (line 3651 of lower.rs) is the direct template:
- Iterate struct fields
- For each field: extract value from container (JSON object / Map row), convert type, assign
- Construct StructLit with all field values

`deriving(Row)` is simpler than `deriving(Json)` because:
- Input is always `Map<String, String>` (no nested JSON types)
- Type conversion is string parsing only (no recursive decode)
- No need for error-checked object_get (map_get returns empty string for missing)

### Pattern 5: Actor-Compatible Blocking (Established)

**What:** Blocking I/O operations run within the actor system, where each actor has its own coroutine stack. Blocking one actor does not block others.
**When:** Connection pool `acquire` with no available connections.

```rust
// Pool acquire blocks like actor receive:
loop {
    if let Some(conn) = pool.try_acquire() {
        return conn;
    }
    // Yield to scheduler, will be resumed
    stack::yield_current();
}
```

This is identical to how `snow_actor_receive` handles blocking waits. The pool manager wakes waiting actors when connections are returned.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Global Mutex Pool

**What:** Wrapping the entire connection pool in a `Mutex<Pool>`.
**Why bad:** Every acquire/release locks the entire pool, creating a bottleneck. With actor-per-connection HTTP, many actors contend simultaneously.
**Instead:** Use a lock-free or fine-grained approach. The pool's idle connection list can use `parking_lot::Mutex` (already a dependency) with very short critical sections: lock, pop/push, unlock. The pool itself is accessed via handle (no global lock needed).

### Anti-Pattern 2: Async Runtime for TLS

**What:** Pulling in tokio or async-std for TLS support.
**Why bad:** Snow's runtime is synchronous (blocking I/O on coroutine stacks). Adding an async runtime creates two execution models that conflict.
**Instead:** Use rustls directly with `StreamOwned<ClientConnection, TcpStream>` for PostgreSQL TLS. Use tiny_http's `ssl-rustls` feature for HTTPS. Both are synchronous.

### Anti-Pattern 3: Connection Pool as Separate Actor

**What:** Implementing the pool as a Snow actor that receives acquire/release messages.
**Why bad:** Message passing overhead for every DB operation. Serialization bottleneck (single actor processes requests sequentially).
**Instead:** Pool is a Rust-side data structure (behind handle), accessed directly via runtime functions. The pool uses `parking_lot::Mutex` for internal synchronization. Pool waiting uses the scheduler yield mechanism (not actor messaging).

### Anti-Pattern 4: Generating Row Mapping at Runtime

**What:** Using reflection or runtime field inspection to map rows to structs.
**Why bad:** Snow is a compiled language with no runtime reflection. All type information is available at compile time.
**Instead:** Generate mapping code at MIR level during compilation (same as `deriving(Json)`). Zero runtime overhead beyond the string-to-type conversions.

### Anti-Pattern 5: TLS State in PgConn as Option

**What:** `stream: Option<TcpStream>, tls_stream: Option<StreamOwned<...>>`.
**Why bad:** Every read/write requires checking which Option is Some. Error-prone, verbose.
**Instead:** Use an enum:

```rust
enum PgStream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}
```

Both variants implement `Read + Write`. All wire protocol code operates on `&mut impl Read + Write` or through a helper method on the enum.

## Detailed Component Designs

### Connection Pool (`db/pool.rs`)

```rust
struct Pool {
    /// URL for creating new connections
    url: String,
    /// Backend type (Pg or Sqlite)
    backend: PoolBackend,
    /// Maximum pool size
    max_size: usize,
    /// Idle connections available for checkout
    idle: parking_lot::Mutex<Vec<u64>>,    // connection handles
    /// Number of connections currently checked out
    active: AtomicUsize,
    /// Total connections created (idle + active)
    total: AtomicUsize,
}

enum PoolBackend {
    Pg,
    Sqlite,
}
```

**Key design decisions:**
- Pool stores connection handles (u64), not PgConn/SqliteConn directly. This means pool code is backend-agnostic.
- `idle` uses `parking_lot::Mutex` (already a dependency) for the idle connection vector. Lock is held only for push/pop (microseconds).
- Health checking: on acquire, send a simple query ("SELECT 1" for Pg, "SELECT 1" for SQLite) to verify the connection is alive. If dead, discard and create new.
- Pool does NOT own connections' lifetimes. Connections are created/destroyed via `snow_pg_connect`/`snow_pg_close` (reusing existing code).

**Snow-level API:**

```snow
let pool = Pool.open("postgres://user:pass@localhost/db", 4)  // Result<PoolHandle, String>
let conn = Pool.acquire(pool)                                  // Result<PgConn, String>
Pool.release(pool, conn)                                       // Unit
let result = Pool.with(pool, fn(conn) { ... })                // Result<T, String>
Pool.close(pool)                                               // Unit
```

### PostgreSQL TLS (`db/pg.rs` modification)

```rust
enum PgStream {
    Plain(TcpStream),
    Tls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl PgStream {
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String> { ... }
    fn write_all(&mut self, buf: &[u8]) -> Result<(), String> { ... }
}

struct PgConn {
    stream: PgStream,  // was: TcpStream
}
```

**TLS flow integrated into `snow_pg_connect`:**

1. Parse URL query params for `sslmode` (disable/prefer/require, default: prefer)
2. After TCP connect, before StartupMessage:
   - Send SSLRequest (Int32(8) + Int32(80877103))
   - Read 1-byte response ('S' or 'N')
   - If 'S': upgrade to TLS using rustls
   - If 'N': continue plaintext or error based on sslmode
3. All subsequent protocol messages go through `PgStream` enum

**No changes to the Snow API needed.** The URL controls TLS:
- `postgres://host/db` -- prefer TLS, fallback to plain
- `postgres://host/db?sslmode=require` -- require TLS
- `postgres://host/db?sslmode=disable` -- no TLS

### HTTPS (`http/server.rs` modification)

**New runtime function:**

```rust
#[no_mangle]
pub extern "C" fn snow_http_serve_tls(
    router: *mut u8,
    port: i64,
    cert: *const SnowString,
    key: *const SnowString,
) {
    let cert_bytes = unsafe { (*cert).as_str().as_bytes().to_vec() };
    let key_bytes = unsafe { (*key).as_str().as_bytes().to_vec() };

    let server = tiny_http::Server::https(
        &format!("0.0.0.0:{}", port),
        tiny_http::SslConfig {
            certificate: cert_bytes,
            private_key: key_bytes,
        },
    ).unwrap();

    // Rest identical to snow_http_serve
}
```

**Snow-level API:**

```snow
let cert = File.read("cert.pem") |> Result.unwrap
let key = File.read("key.pem") |> Result.unwrap
HTTP.serve_tls(router, 8443, cert, key)
```

`tiny_http` already supports `ssl-rustls` as a feature flag. This is a minimal integration.

### Transactions (`db/pg.rs` and `db/sqlite.rs` modifications)

**PostgreSQL:**

Three new runtime functions using the existing Extended Query Protocol:

```rust
#[no_mangle]
pub extern "C" fn snow_pg_begin(conn_handle: u64) -> *mut u8 {
    // Execute "BEGIN" via Parse/Bind/Execute/Sync
    // Check ReadyForQuery status byte == 'T' (in transaction)
}

#[no_mangle]
pub extern "C" fn snow_pg_commit(conn_handle: u64) -> *mut u8 {
    // Execute "COMMIT" via Parse/Bind/Execute/Sync
    // Check ReadyForQuery status byte == 'I' (idle)
}

#[no_mangle]
pub extern "C" fn snow_pg_rollback(conn_handle: u64) -> *mut u8 {
    // Execute "ROLLBACK" via Parse/Bind/Execute/Sync
}
```

These reuse the existing `write_parse`, `write_bind`, `write_execute`, `write_sync` helpers. The only difference from `snow_pg_execute` is that the SQL is hardcoded and there are no params.

**SQLite transactions are identical** but use `sqlite3_exec` for BEGIN/COMMIT/ROLLBACK (no parameter binding needed).

**Higher-level `transaction` wrapper:**

```rust
#[no_mangle]
pub extern "C" fn snow_pg_transaction(
    conn_handle: u64,
    fn_ptr: *mut u8,
    fn_env: *mut u8,
) -> *mut u8 {
    // 1. BEGIN
    // 2. Call fn(conn) -- same calling convention as HTTP handlers
    // 3. If Ok: COMMIT, return result
    // 4. If Err/panic: ROLLBACK, return error
}
```

### Struct-to-Row Mapping (`mir/lower.rs` modification + `db/row.rs` new)

**Compiler side -- `generate_from_row_struct` in lower.rs:**

This is a simplified version of `generate_from_json_struct` (which starts at line 3651). The key differences:

1. Input: `Map<String, String>` (a row from `Pg.query`)
2. Field extraction: `snow_map_get(row, "field_name")` returns SnowString pointer (or 0)
3. Type conversion: `snow_string_to_int`, `snow_string_to_float`, `snow_string_to_bool` (all already exist as runtime functions)
4. Output: heap-allocated struct

```
// Generated MIR for: struct User { name: String, age: Int } deriving(Row)
//
// FromRow__from_row__User(row: Ptr) -> Ptr (SnowResult<User, String>)
//   let name_val = snow_map_get(row, "name")    // returns u64
//   let name = name_val as *mut SnowString       // String field, no conversion
//   let age_str_val = snow_map_get(row, "age")
//   let age_str = age_str_val as *mut SnowString
//   let age_result = snow_string_to_int(age_str) // returns SnowResult<Int, String>
//   if snow_result_is_ok(age_result):
//     let age = snow_result_unwrap(age_result)
//     let user = User { name, age }
//     alloc_result(0, user)
//   else:
//     alloc_result(1, "failed to parse field 'age'")
```

**Runtime side -- `db/row.rs`:**

Two new helper functions for query_as:

```rust
#[no_mangle]
pub extern "C" fn snow_pg_query_as(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
    from_row_fn: *mut u8,     // fn(row_map) -> struct_ptr
    from_row_env: *mut u8,    // closure env (null for bare fn)
) -> *mut u8 {
    // 1. Execute query (reuse snow_pg_query internals)
    // 2. For each row map: call from_row_fn(row_map)
    // 3. Collect into List<T>
    // 4. Return Result<List<T>, String>
}
```

**Snow-level API:**

```snow
struct User deriving(Row)
  name: String
  age: Int

let users = Pg.query_as<User>(conn, "SELECT name, age FROM users", [])
// users : Result<List<User>, String>
```

## Build Order (Dependency-Driven)

```
Phase 1: Transactions          (no dependencies on other features)
   |
   +-- pg: snow_pg_begin, snow_pg_commit, snow_pg_rollback
   +-- sqlite: snow_sqlite_begin, snow_sqlite_commit, snow_sqlite_rollback
   +-- Higher-level: snow_pg_transaction, snow_sqlite_transaction
   +-- All four-layer integration (builtins, lower, intrinsics, runtime)

Phase 2: TLS/SSL               (no dependencies on other features)
   |
   +-- PostgreSQL TLS: PgStream enum, SSLRequest flow, rustls StreamOwned
   +-- HTTPS: snow_http_serve_tls via tiny_http ssl-rustls
   +-- Cargo.toml: add rustls + webpki-roots deps, enable tiny_http ssl-rustls

Phase 3: Connection Pooling    (benefits from transactions for pool health checks)
   |
   +-- db/pool.rs: Pool struct, acquire/release/with
   +-- Actor yield integration for blocking acquire
   +-- Health checking on acquire
   +-- Pool.close drains all connections

Phase 4: Struct-to-Row         (benefits from pool for realistic usage patterns)
   |
   +-- deriving(Row) in parser (trivial: reuse deriving clause infrastructure)
   +-- generate_from_row_struct in lower.rs (template: generate_from_json_struct)
   +-- snow_pg_query_as / snow_sqlite_query_as runtime functions
   +-- Four-layer integration
```

**Phase ordering rationale:**

1. **Transactions first** because they are the simplest feature (3 wire protocol commands reusing existing helpers) and are a prerequisite for the pool's internal health-check queries. They also establish the pattern for subsequent DB features.

2. **TLS second** because it modifies `PgConn` fundamentally (TcpStream -> PgStream enum). All subsequent features that touch PgConn need to work with the enum. Doing TLS before pooling means pool code is written against the final PgConn shape.

3. **Connection pooling third** because it builds on both transactions (for health checks within a transaction-safe context) and TLS (pool creates connections that may be TLS-enabled). The pool is also the most architecturally complex feature.

4. **Struct-to-row last** because it requires the most compiler-side work (new deriving trait) and benefits from having pool + transactions available for realistic E2E testing. It is also the most independent feature -- it does not affect the others.

## Scalability Considerations

| Concern | At 10 connections | At 100 connections | At 1000 connections |
|---------|------------------|-------------------|---------------------|
| Pool contention | Negligible (parking_lot Mutex) | Fine (microsecond lock hold) | Consider sharded idle lists |
| TLS handshake cost | ~2ms per connection | Use pool to amortize | Pool is essential |
| Memory per connection | ~50KB (TCP buffers + TLS state) | ~5MB total | ~50MB, may need limits |
| Actor pool waiting | Rarely blocks | Yield loop adequate | Add timeout to prevent starvation |

## New Dependencies

| Crate | Version | Purpose | Size Impact |
|-------|---------|---------|-------------|
| `rustls` | 0.23+ | TLS 1.2/1.3 for PostgreSQL connections | ~500KB compiled |
| `webpki-roots` | 0.26+ | Mozilla root CA certificates | ~200KB |
| `rustls-pki-types` | 1.0+ | PKI type definitions (dep of rustls) | Minimal |
| tiny_http `ssl-rustls` feature | -- | HTTPS support via rustls | Already included in rustls |

**Note:** `ring` or `aws-lc-rs` is required as a crypto provider for rustls 0.23+. Use `ring` for simpler cross-platform builds (rustls's `ring` feature). `aws-lc-rs` has better performance but harder to build on some platforms.

## Modification Summary by File

### Runtime (`snow-rt`)

| File | Change | Lines Est. |
|------|--------|-----------|
| `db/pg.rs` | PgStream enum, SSLRequest, TLS upgrade, begin/commit/rollback, refactor read/write to use enum | +200 |
| `db/sqlite.rs` | begin/commit/rollback functions | +60 |
| `db/pool.rs` | **NEW**: Pool struct, acquire/release/with/close | +250 |
| `db/row.rs` | **NEW**: query_as helpers | +80 |
| `db/mod.rs` | Add pool and row modules | +2 |
| `http/server.rs` | serve_tls function | +30 |
| `http/mod.rs` | Re-export serve_tls | +1 |
| `lib.rs` | Re-export new functions | +10 |
| `Cargo.toml` | rustls, webpki-roots deps; tiny_http ssl-rustls feature | +4 |

### Compiler (`snow-codegen`)

| File | Change | Lines Est. |
|------|--------|-----------|
| `codegen/intrinsics.rs` | Declare ~12 new LLVM functions | +60 |
| `mir/lower.rs` | Register known functions, name mappings, generate_from_row_struct | +200 |

### Type Checker (`snow-typeck`)

| File | Change | Lines Est. |
|------|--------|-----------|
| `builtins.rs` | Type signatures for ~12 new functions, PoolHandle type | +60 |

### Parser (`snow-parser`)

| File | Change | Lines Est. |
|------|--------|-----------|
| (none) | "Row" is just a string in deriving clause -- already supported | 0 |

**Total estimated new/modified code: ~960 lines across 12 files.**

## Sources

- PostgreSQL Wire Protocol: [PostgreSQL 18 Documentation - Message Flow](https://www.postgresql.org/docs/current/protocol-flow.html)
- PostgreSQL SSLRequest: [PostgreSQL 18 Security Improvements](https://neon.com/postgresql/postgresql-18/security-improvements)
- Rustls documentation: [docs.rs/rustls](https://docs.rs/rustls/latest/rustls/)
- Rustls StreamOwned: [docs.rs/rustls StreamOwned](https://docs.rs/rustls/latest/rustls/struct.StreamOwned.html)
- Rustls ClientConnection: [docs.rs/rustls ClientConnection](https://docs.rs/rustls/latest/rustls/client/struct.ClientConnection.html)
- tiny_http HTTPS: [tiny_http SslConfig](https://tiny-http.github.io/tiny-http/tiny_http/struct.SslConfig.html)
- tiny_http GitHub: [github.com/tiny-http/tiny-http](https://github.com/tiny-http/tiny-http)
- Connection pooling patterns: [r2d2](https://github.com/sfackler/r2d2), [Deadpool + SQLx guide (Jan 2026)](https://oneuptime.com/blog/post/2026-01-07-rust-database-connection-pooling/view)
