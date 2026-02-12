# Feature Landscape

**Domain:** Production backend capabilities for compiled programming language (Snow v3.0)
**Researched:** 2026-02-12
**Confidence:** HIGH (all four features are well-studied across Elixir/Go/Rust ecosystems; existing Snow codebase patterns thoroughly reviewed)

---

## Current State in Snow

Before defining features, here is what already exists and what this milestone builds on:

**Working infrastructure these features extend:**
- SQLite: `Sqlite.open/close/query/execute` with `?` parameterized queries, bundled C FFI via `libsqlite3-sys`
- PostgreSQL: `Pg.connect/close/query/execute` with `$1` parameterized queries, pure wire protocol (no libpq), SCRAM-SHA-256/MD5 auth
- Both return `Result<List<Map<String, String>>, String>` for queries -- all values as text strings
- HTTP server: `tiny_http` with actor-per-connection, `SnowRouter` with path params, method routing, middleware pipeline
- JSON serde: `deriving(Json)` generates `Json.encode(value)` and `StructName.from_json(str)` at compile time
- Actor runtime: M:N scheduler with corosensei coroutines, typed `Pid<M>`, `snow_service_call` for request/reply, `snow_actor_register/whereis` for name registry
- GC-safe opaque u64 handles for DB connections (`Box::into_raw as u64`)
- `PgConn` wraps a `std::net::TcpStream` directly -- pure Rust, no external C libraries for PostgreSQL
- `Result<T,E>` with `?` operator for error propagation, `Option<T>` for nullable values
- `deriving(Eq, Ord, Display, Debug, Hash, Json)` infrastructure with MIR-level synthetic function generation

**What this milestone adds:**
- Connection pooling (actor-based pool manager for SQLite and PostgreSQL)
- TLS/SSL (encrypted PostgreSQL connections + HTTPS server support)
- Database transactions (block-based `Db.transaction(conn, fn)` with auto-commit/rollback)
- Struct-to-row mapping (automatic query result to struct hydration via `deriving(Db)`)

---

## Table Stakes

Features users expect. Missing = product feels incomplete for production use.

### 1. Connection Pooling

Every production backend framework provides connection pooling. Opening a new database connection per request is prohibitively expensive: TCP handshake, TLS negotiation, and authentication add 10-150ms per connection. PostgreSQL allocates ~1.3MB per connection. Without pooling, a server handling 100 concurrent requests would need 100 simultaneous connections, each with full setup cost.

Elixir has `db_connection` (GenServer-based pool with checkout/checkin). Go has `database/sql` (built-in pool with `SetMaxOpenConns`). Rust has `r2d2` and `sqlx::Pool`. Every production database library pools connections.

Snow's actor runtime is a natural fit: a pool manager actor holds N connections, callers check out a connection, use it, and check it back in. This mirrors Elixir's `db_connection` pattern where each connection is a GenServer process.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Pool.start(config)` returns a pool handle | Every pool has a startup function. Elixir: `DBConnection.start_link(opts)`. Go: pool created implicitly by `sql.Open`. | Med | Config struct holds URL, min/max size, idle timeout. Pool actor spawns initial connections. |
| `Pool.checkout(pool)` returns `Result<Conn, String>` | Caller gets exclusive access to one connection. Elixir: `DBConnection.checkout`. Go: implicit on every query. | Med | Pool actor receives checkout request, hands out an idle connection or creates a new one (up to max). Blocks if pool exhausted. |
| `Pool.checkin(pool, conn)` returns connection | Explicit return. Elixir: `DBConnection.checkin`. Go: implicit via `rows.Close()`. | Low | Pool actor receives checkin, marks connection idle. |
| `Pool.query(pool, sql, params)` auto checkout/checkin | Users should not need manual checkout for simple queries. Elixir: `Repo.all(query)` auto-manages. Go: `db.Query` auto-manages. | Med | Convenience wrapper: checkout, query, checkin. Returns `Result<List<Map<String, String>>, String>`. |
| `Pool.execute(pool, sql, params)` auto checkout/checkin | Same pattern for write queries. | Low | Follows query pattern. Returns `Result<Int, String>`. |
| Configurable pool size (min, max) | Every pool has size limits. Too few = contention. Too many = DB overload. | Low | Config fields `min_size: Int`, `max_size: Int`. Default min=2, max=10. |
| Idle connection timeout | Connections sitting unused waste resources. Elixir: `idle_interval`. HikariCP: `idleTimeout`. | Med | Pool actor periodically checks idle connections, closes those exceeding timeout. |
| Connection health validation | Stale/broken connections must not be handed to callers. HikariCP: `connectionTestQuery`. Elixir: `ping` callback. | Med | Before checkout, issue a lightweight query (`SELECT 1` for PG, `SELECT 1` for SQLite) to verify liveness. Discard dead connections. |
| Graceful exhaustion handling | When max connections are in use, callers should wait (with timeout), not crash. | Med | Caller blocks with configurable acquisition timeout. Returns `Err("pool exhausted")` after timeout. |

**Expected API surface:**

```snow
fn main() do
  # Start a PostgreSQL pool
  let pool = Pool.start(%{
    url: "postgres://user:pass@localhost:5432/mydb",
    min_size: 2,
    max_size: 10,
    idle_timeout: 60000
  })?

  # Simple query (auto checkout/checkin)
  let rows = Pool.query(pool, "SELECT name, age FROM users WHERE age > $1", ["25"])?

  # Simple execute (auto checkout/checkin)
  let affected = Pool.execute(pool, "INSERT INTO users (name, age) VALUES ($1, $2)", ["Alice", "30"])?

  # Manual checkout for multiple operations on same connection
  let conn = Pool.checkout(pool)?
  Pg.execute(conn, "INSERT INTO logs (msg) VALUES ($1)", ["started"])?
  Pg.execute(conn, "INSERT INTO logs (msg) VALUES ($1)", ["finished"])?
  Pool.checkin(pool, conn)
end
```

**Implementation approach:**

The pool is implemented as a Snow actor (using the existing actor runtime). This follows Elixir's `db_connection` pattern:
1. A pool manager actor is spawned by `Pool.start`. It holds a list of idle connections and tracks checked-out connections.
2. `Pool.checkout` sends a message to the pool actor requesting a connection. The pool either hands out an idle one or creates a new one (up to max_size).
3. `Pool.checkin` sends the connection back. The pool actor marks it idle.
4. `Pool.query/execute` are convenience wrappers: checkout -> operation -> checkin, with error handling to ensure checkin on failure.
5. A periodic health check runs via `Timer.send_after` to evict idle/dead connections.

The pool can work with both SQLite and PostgreSQL connections since both use opaque u64 handles and have the same query/execute API shape. The pool config includes a `driver` field (`:pg` or `:sqlite`) to know how to create new connections and validate health.

**What NOT to include:**
- Connection warming / pre-connect on startup beyond min_size -- simple lazy creation is sufficient
- Dynamic pool resizing at runtime -- static min/max is fine for v3.0
- Per-query timeout -- users handle this at the application level
- Read/write splitting -- single pool per database is sufficient
- Named pools / pool registry -- one pool per `Pool.start` call

**Confidence:** HIGH -- connection pooling is a solved problem. The actor-based design maps cleanly to Snow's runtime. Elixir's `db_connection` provides an exact reference implementation using the same actor model.

**Dependencies:** Existing actor runtime (`snow_actor_spawn`, `snow_service_call`, `snow_actor_register`), existing Pg/Sqlite connection functions, existing `Timer.send_after` for periodic health checks.

---

### 2. TLS/SSL for PostgreSQL Connections

Every cloud PostgreSQL provider (AWS RDS, GCP Cloud SQL, Azure Database, Supabase, Neon) requires or strongly recommends TLS. Without TLS support, Snow programs cannot connect to production databases. This is a hard blocker for real-world deployment.

PostgreSQL TLS works via an upgrade mechanism: the client sends an 8-byte `SSLRequest` message (`00 00 00 08 04 D2 16 2F`) before the StartupMessage. The server responds with `S` (willing) or `N` (unwilling). On `S`, the client performs a TLS handshake, then sends the normal StartupMessage over the encrypted channel.

Snow's PostgreSQL driver already speaks the wire protocol via `std::net::TcpStream`. The TLS upgrade wraps that `TcpStream` in a `rustls::StreamOwned<ClientConnection, TcpStream>`, which implements the same `Read + Write` traits. The rest of the wire protocol code remains unchanged.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| TLS-encrypted PostgreSQL connections | Cloud databases require TLS. AWS RDS, Supabase, Neon all enforce it. | **High** | Send SSLRequest before StartupMessage. On `S` response, perform TLS handshake with rustls. Wrap TcpStream in StreamOwned. |
| `sslmode` parameter in connection URL | PostgreSQL connection strings support `?sslmode=require` etc. Universal convention. | Med | Parse `sslmode` from URL query params. Modes: `disable` (no TLS), `require` (TLS required, no cert verify), `verify-ca` / `verify-full` (with certificate validation). |
| System root certificate trust | Connecting to cloud PG should work without manually providing CA certs. | Med | Use `webpki-roots` or `rustls-native-certs` to load system CA certificates. |
| Self-signed cert support for development | Local Docker PostgreSQL often uses self-signed certs. `sslmode=require` should work without CA verification. | Low | `require` mode skips certificate validation (accepts any cert). Only `verify-ca`/`verify-full` validate. |
| Fallback to plaintext if server declines | If server responds `N` to SSLRequest and sslmode is not `require`, proceed without TLS. | Low | Already handled: if server says `N` and sslmode allows, send StartupMessage unencrypted. |

**Expected API surface:**

```snow
fn main() do
  # Cloud database -- TLS required
  let conn = Pg.connect("postgres://user:pass@db.supabase.co:5432/mydb?sslmode=require")?

  # Local development -- no TLS
  let local = Pg.connect("postgres://dev:dev@localhost:5432/devdb")?

  # Strict verification
  let strict = Pg.connect("postgres://user:pass@prod.example.com:5432/prod?sslmode=verify-full")?
end
```

**Implementation approach:**

1. **Abstract the stream type:** Change `PgConn` from holding `stream: TcpStream` to holding an enum or trait object that supports both plain TCP and TLS. Use `enum PgStream { Plain(TcpStream), Tls(StreamOwned<ClientConnection, TcpStream>) }` and implement `Read + Write` for it.
2. **SSLRequest handshake:** After TCP connect but before StartupMessage, send the SSLRequest 8-byte message. Read the 1-byte response.
3. **TLS upgrade:** On `S`, create a `rustls::ClientConfig` with appropriate certificate validation based on `sslmode`, construct a `ClientConnection`, wrap the `TcpStream` in `StreamOwned`.
4. **Continue protocol:** The rest of `snow_pg_connect` (StartupMessage, authentication, ReadyForQuery) works unchanged because `PgStream` implements `Read + Write`.

**New Rust dependencies:** `rustls` (pure Rust TLS), `webpki-roots` (Mozilla CA bundle), `rustls-pki-types` (certificate types).

**What NOT to include:**
- Client certificate authentication -- niche, defer
- Custom CA certificate file path -- use system certs or `webpki-roots`
- TLS session resumption -- optimization, not needed for correctness
- ALPN-based direct TLS (PostgreSQL 17+) -- use standard SSLRequest flow

**Confidence:** HIGH -- the SSLRequest protocol is well-documented (PostgreSQL docs 54.2, 54.7). `rustls::StreamOwned` wrapping `TcpStream` is the standard pattern for TLS upgrade on a synchronous stream. The existing PG wire protocol code uses `Read + Write` traits throughout, making the abstraction clean.

**Dependencies:** Existing PgConn/TcpStream infrastructure, `rustls` crate, URL parsing already handles query params.

---

### 3. TLS/SSL for HTTPS Server

Production HTTP servers must support HTTPS. Snow's HTTP server uses `tiny_http`, which already has built-in TLS support via the `ssl-rustls` feature flag. Enabling HTTPS is a matter of feature-flagging `tiny_http` and adding a new `HTTP.serve_tls` function that accepts certificate/key paths.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `HTTP.serve_tls(router, port, cert_path, key_path)` | Production servers must serve HTTPS. | **Med** | `tiny_http` supports `Server::https(addr, SslConfig)` with the `ssl-rustls` feature. Read cert/key PEM files, pass to SslConfig. |
| PEM certificate and key file loading | Standard format for TLS certificates. Let's Encrypt, self-signed, all use PEM. | Low | `std::fs::read(cert_path)` and `std::fs::read(key_path)` to load PEM bytes. |
| Plaintext `HTTP.serve` continues to work | Not all deployments need TLS (reverse proxy handles it). Development uses plaintext. | Low | Existing function unchanged. New function is additive. |

**Expected API surface:**

```snow
fn main() do
  let r = HTTP.router()
  let r = HTTP.on_get(r, "/health", health_handler)

  # HTTPS with certificate
  HTTP.serve_tls(r, 443, "/etc/ssl/cert.pem", "/etc/ssl/key.pem")

  # OR plaintext (existing, unchanged)
  # HTTP.serve(r, 8080)
end
```

**Implementation approach:**

1. Add `ssl-rustls` feature to `tiny_http` dependency in `Cargo.toml`: `tiny_http = { version = "0.12", features = ["ssl-rustls"] }`
2. New runtime function `snow_http_serve_tls(router, port, cert_path, key_path)` that:
   - Reads the PEM files from disk
   - Constructs `tiny_http::SslConfig { certificate, private_key }`
   - Calls `Server::https(addr, config)` instead of `Server::http(addr)`
   - Uses the same `incoming_requests()` loop and actor dispatch as the plaintext server

**What NOT to include:**
- Automatic Let's Encrypt / ACME -- out of scope for a language runtime
- SNI-based virtual hosting -- single cert per server
- HTTP to HTTPS redirect -- application-level concern
- Hot-reloading certificates -- restart server to update certs

**Confidence:** HIGH -- `tiny_http` has an official `ssl-rustls` example. The `Server::https` API is a drop-in replacement for `Server::http`. Pure additive change.

**Dependencies:** `tiny_http` with `ssl-rustls` feature, `rustls` (shared dependency with PG TLS). File I/O for cert loading (existing `std::fs`).

---

### 4. Database Transactions

Every production application needs transactions for data consistency. Without a transaction API, Snow users must manually issue `Pg.execute(conn, "BEGIN", [])`, handle errors, and remember to COMMIT or ROLLBACK -- error-prone and ugly. Every framework provides a block-based transaction API: Elixir's `Repo.transaction(fn -> ... end)`, Go's `tx.Begin/Commit/Rollback` pattern, Rust's `sqlx::Transaction`.

The block-based pattern is the right fit for Snow: pass a closure to `Db.transaction`, the runtime issues BEGIN before calling it and COMMIT after, or ROLLBACK if the closure returns Err or raises. This mirrors Elixir's `Repo.transaction` and Django's `atomic()`.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Db.transaction(conn, fn(conn) -> ... end)` | Block-based transaction with automatic BEGIN/COMMIT/ROLLBACK. Elixir: `Repo.transaction`. Django: `atomic()`. | **Med** | Send `BEGIN` before calling the closure, `COMMIT` on Ok, `ROLLBACK` on Err. Return the closure's result. |
| Automatic ROLLBACK on error | If the closure returns `Err(e)`, the transaction must ROLLBACK and propagate the error. Non-negotiable for data safety. | Med | Wrap the closure call. If it returns tag 1 (Err), send ROLLBACK and forward the Err. |
| Automatic ROLLBACK on panic/crash | If the closure crashes (actor panic), ROLLBACK must still execute. | Med | Use `catch_unwind` or ensure cleanup via the actor's terminate callback. |
| Works with both PostgreSQL and SQLite | Transactions are universal SQL. `BEGIN`/`COMMIT`/`ROLLBACK` work on both. | Low | The transaction function just issues SQL commands. Same for both drivers. |
| Nested transaction via savepoints | Inner `Db.transaction` within an outer one should use SAVEPOINT, not a real nested BEGIN. Elixir, Django, Rails all handle this. | **High** | Track nesting depth. Depth 0: real BEGIN/COMMIT. Depth 1+: SAVEPOINT/RELEASE SAVEPOINT. ROLLBACK TO SAVEPOINT on inner error. |

**Expected API surface:**

```snow
fn transfer(pool, from_id, to_id, amount) -> Int!String do
  let conn = Pool.checkout(pool)?

  Db.transaction(conn, fn(tx) do
    Pg.execute(tx, "UPDATE accounts SET balance = balance - $1 WHERE id = $2", [amount, from_id])?
    Pg.execute(tx, "UPDATE accounts SET balance = balance + $1 WHERE id = $2", [amount, to_id])?

    let rows = Pg.query(tx, "SELECT balance FROM accounts WHERE id = $1", [from_id])?
    let balance = Map.get(List.head(rows), "balance")
    case String.to_int(balance) do
      Ok(b) when b >= 0 -> Ok(b)
      _ -> Err("insufficient funds")
    end
  end)?

  Pool.checkin(pool, conn)
  Ok(0)
end
```

**Implementation approach:**

1. **Runtime function:** `snow_db_transaction(conn_handle, driver_tag, closure_fn, closure_env)` that:
   - Sends `BEGIN` via the appropriate driver
   - Calls the closure with the connection handle
   - On Ok result: sends `COMMIT`, returns Ok
   - On Err result: sends `ROLLBACK`, returns Err
2. **Savepoint support (v3.0 stretch):** Track transaction depth on the connection. Depth 0 uses `BEGIN`/`COMMIT`/`ROLLBACK`. Depth 1+ uses `SAVEPOINT sp_N`/`RELEASE SAVEPOINT sp_N`/`ROLLBACK TO SAVEPOINT sp_N`.

**What NOT to include:**
- `Ecto.Multi`-style composable transaction builder -- too complex for v3.0
- Distributed transactions / two-phase commit -- out of scope
- Transaction isolation level configuration -- users can issue `SET TRANSACTION ISOLATION LEVEL` manually
- Read-only transactions -- users issue `BEGIN READ ONLY` manually

**Confidence:** HIGH -- transaction semantics are well-defined by SQL. The block-based pattern (closure + automatic BEGIN/COMMIT/ROLLBACK) is universal across Elixir, Django, Rails, and Spring. Snow's closure system supports this naturally.

**Dependencies:** Existing Pg/Sqlite execute functions. Closure calling convention (fn_ptr + env_ptr). For pool integration, transaction operates on a checked-out connection.

---

### 5. Struct-to-Row Mapping via `deriving(Db)`

Currently, Snow database queries return `List<Map<String, String>>` -- all values as text. Users must manually extract fields and parse types:

```snow
let rows = Pg.query(conn, "SELECT name, age FROM users", [])?
List.map(rows, fn(row) do
  let name = Map.get(row, "name")
  let age = String.to_int(Map.get(row, "age"))
  # ... tedious, error-prone
end)
```

Every modern framework automates this. Rust sqlx: `#[derive(FromRow)]`. Go: `db.StructScan`. Elixir Ecto: schema macros map columns to struct fields. Snow already has `deriving(Json)` that generates encode/decode from struct field metadata. `deriving(Db)` follows the exact same pattern: generate a `from_row` function that takes a `Map<String, String>` and returns a typed struct.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `deriving(Db)` generates `StructName.from_row(row)` | Automates `Map<String, String>` to struct conversion. Rust sqlx: `#[derive(FromRow)]`. | **Med-High** | Generate MIR function that extracts each field by name from the map, parses to correct type (String.to_int, String.to_float, etc.), constructs struct. Returns `Result<T, String>`. |
| Automatic type coercion (String -> Int, String -> Float, etc.) | Database rows are text. Struct fields are typed. The mapping must bridge this. | Med | For each field: String field = direct use. Int field = `String.to_int`. Float field = `String.to_float`. Bool field = `"true"/"false"` comparison. |
| Option field handling for NULL columns | Database columns can be NULL (represented as empty string `""` in current API). `Option<String>` field maps NULL to `None`. | Med | Empty string in map -> `None`. Non-empty -> `Some(parsed_value)`. Matches the existing NULL-as-empty-string convention. |
| Error on missing or unparseable columns | If a column is missing or the text cannot parse to the expected type, return `Err` with a descriptive message. | Low | `"column 'age' not found"` or `"column 'age': expected integer, got 'abc'"`. |
| Works with query results directly | `Db.query_as<User>(conn, sql, params)` that combines query + mapping. | Med | Convenience wrapper: query, then map each row through `User.from_row`. Returns `Result<List<User>, String>`. |

**Expected API surface:**

```snow
struct User do
  name :: String
  age :: Int
  email :: Option<String>
end deriving(Db)

fn main() do
  let conn = Pg.connect("postgres://...")?

  # Option A: Manual row mapping
  let rows = Pg.query(conn, "SELECT name, age, email FROM users", [])?
  let users = List.map(rows, fn(row) do
    User.from_row(row)
  end)

  # Option B: Query + map in one step (stretch goal)
  # let users = Db.query_as(conn, User, "SELECT name, age, email FROM users", [])?

  Pg.close(conn)
end
```

**Implementation approach:**

Follows the `deriving(Json)` pattern exactly:

1. **MIR lowering:** When `deriving(Db)` is seen on a struct, generate a synthetic function `Db__from_row__StructName(row: Map<String, String>) -> Result<StructName, String>`.
2. **Field extraction:** For each field, call `Map.get(row, "field_name")` to get the text value.
3. **Type coercion:** Based on the field's type:
   - `String` -> use directly
   - `Int` -> call `String.to_int`, propagate error
   - `Float` -> call `String.to_float`, propagate error
   - `Bool` -> compare with `"true"`/`"t"`/`"1"`
   - `Option<T>` -> empty string becomes `None`, otherwise parse inner type and wrap in `Some`
4. **Struct construction:** Build the struct from parsed fields.
5. **Error handling:** Return `Err("column 'X': expected int, got 'Y'")` on parse failure.

The `deriving(Db)` infrastructure reuses the field metadata extraction from `deriving(Json)`. The main difference is the source format (Map<String,String> instead of SnowJson) and the coercion logic (string parsing instead of JSON type checking).

**What NOT to include:**
- Column renaming attributes (`@column("user_name")`) -- requires annotation syntax
- Nested struct hydration from JOINs -- complex, defer
- Custom type converters -- fixed mapping for v3.0
- Automatic query generation from struct fields -- users write SQL
- Row-to-struct for INSERT (reverse mapping) -- write operations use manual params

**Confidence:** HIGH -- this follows the proven `deriving(Json)` pattern. The MIR lowering infrastructure is established. The main complexity is type coercion from strings, which is straightforward with existing `String.to_int`/`String.to_float`.

**Dependencies:** Existing deriving infrastructure, existing Map operations (`Map.get`), existing `String.to_int`/`String.to_float`, existing struct constructor codegen.

---

## Differentiators

Features that set Snow apart. Not expected but valued -- these make Snow feel polished and intentional for production backends.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Actor-based pool with supervision | Pool manager actor is supervised. If it crashes, supervisor restarts it and connections are re-established. Leverages Snow's existing supervision trees. No other non-BEAM language gets this for free. | Med | Pool actor is a child spec under a supervisor. Crash -> restart -> re-create connections. |
| `Pool.transaction(pool, fn(conn) -> ... end)` | Combines checkout + transaction + checkin in one call. The most ergonomic pattern. Elixir: `Repo.transaction`. | Low | Wraps: checkout -> begin -> call closure -> commit/rollback -> checkin. |
| `deriving(Db, Json)` on same struct | One struct definition, two auto-generated mapping layers. Parse from DB row AND serialize to JSON response. Eliminates boilerplate for the most common web API pattern. | Low | Both derives are independent. `deriving(Db, Json)` just generates both sets of functions. |
| Health-checked pool connections | Pool validates connections before handing them out. Dead connections are silently replaced. Users never see stale connection errors. | Med | `SELECT 1` ping before checkout. On failure, discard and create new. |
| Connection URL parsing for both drivers | `Pool.start(%{url: "postgres://..."})` and `Pool.start(%{url: "sqlite:///path/to/db.sqlite"})`. Unified config format. | Low | Parse URL scheme to determine driver. Already parsing PG URLs. |
| Transaction + Pool integration | `Pool.transaction` ensures the connection is returned to the pool even if the transaction fails. No connection leak possible. | Med | Finally-style cleanup: checkin happens in all paths (success, error, panic). |
| `sslmode=prefer` for PostgreSQL | Try TLS first, fall back to plaintext if server declines. Best of both worlds for flexible deployments. | Low | Send SSLRequest. On `N`, proceed with plaintext StartupMessage. On `S`, upgrade to TLS. |

---

## Anti-Features

Features to explicitly NOT build in this milestone.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| ORM / Query builder DSL | Massive scope, controversial (Go/Rust prefer raw SQL), Snow's type system not ready for type-safe query builder. | Raw SQL with parameterized queries. `deriving(Db)` maps results to structs. Users write SQL. |
| Automatic migration framework | Schema versioning, up/down migrations, tracking table. Large feature. | Users call `execute("CREATE TABLE IF NOT EXISTS ...")` or use external tools. |
| Connection pool per-query timeout | Complex (requires canceling in-flight queries on the wire). | Users handle timeouts at the application level or set PostgreSQL `statement_timeout`. |
| Read/write splitting | Multiple pools with routing logic. | Single pool per database. Users create separate pools if needed. |
| Async/non-blocking database I/O | Would require changing the blocking I/O model that works correctly. | Blocking I/O in actor context. M:N scheduler runs other actors. Same model as HTTP. |
| Compile-time SQL validation | Requires connecting to a database during compilation (like Rust sqlx). | Runtime query validation only. SQL errors return `Err`. |
| PgBouncer-style external pooler | External process management, configuration files, deployment complexity. | Built-in application-level pool is sufficient. |
| WebSocket / HTTP/2 support | Different protocols, not related to production backend data layer. | Keep HTTP/1.1 request/response. |
| Distributed transactions / 2PC | Cross-database consistency is a specialized concern. | Single-database transactions only. |
| Custom serializer per DB column | Requires annotation/attribute system Snow does not have. | Fixed type mappings: String, Int, Float, Bool, Option. |
| Row-to-struct for INSERT (reverse mapping) | Auto-generating INSERT SQL from struct fields requires schema awareness the runtime does not have. | Users write INSERT SQL with manual parameter lists. |
| Connection pool rebalancing / warm-up | Dynamic resizing and connection pre-warming add complexity. | Static min/max pool size. Lazy connection creation. |
| Nested struct hydration from JOINs | Mapping JOIN results to nested struct hierarchies (User with Address) requires understanding SQL result shape at compile time. | Flat struct mapping only. Users query and construct nested structs manually. |

---

## Feature Dependencies

```
Connection Pooling
  +-- Existing actor runtime (spawn, send, receive, register)
  +-- Existing Pg.connect / Sqlite.open (connection creation)
  +-- Existing Timer.send_after (idle timeout checks)
  +-- NEW: Pool.start, Pool.checkout, Pool.checkin, Pool.query, Pool.execute
  |
  v
TLS for PostgreSQL
  +-- Existing PgConn with TcpStream
  +-- Existing URL parsing in pg.rs
  +-- NEW: rustls dependency
  +-- NEW: PgStream enum (Plain | Tls)
  +-- NEW: SSLRequest handshake in snow_pg_connect
  +-- NEW: sslmode URL parameter parsing
  |   (INDEPENDENT of pooling -- works on raw connections)
  |
  v
TLS for HTTPS
  +-- Existing tiny_http server infrastructure
  +-- NEW: tiny_http ssl-rustls feature flag
  +-- NEW: snow_http_serve_tls runtime function
  +-- NEW: PEM file loading
  |   (INDEPENDENT of DB features)
  |
  v
Database Transactions
  +-- Existing Pg.execute / Sqlite.execute (for BEGIN/COMMIT/ROLLBACK)
  +-- Existing closure calling convention (fn_ptr + env_ptr)
  +-- DEPENDS ON: Connection pooling (transactions operate on checked-out connections)
  +-- NEW: Db.transaction runtime function
  +-- NEW: Transaction depth tracking for savepoints (stretch)
  |
  v
Struct-to-Row Mapping (deriving(Db))
  +-- Existing deriving infrastructure (MIR lowering)
  +-- Existing Map.get, String.to_int, String.to_float
  +-- Existing struct constructor codegen
  +-- INDEPENDENT of pooling and TLS (works on raw query results)
  +-- NEW: Db__from_row__StructName synthetic MIR function
```

**Critical dependency chain:** Pooling -> Transactions (transactions use checked-out connections from pool)
**Can parallelize:** TLS (PG) + TLS (HTTPS) + deriving(Db) are all independent of each other
**Foundation:** Connection pooling is foundational -- transactions and higher-level APIs build on it

---

## MVP Recommendation

### Build order rationale:

**1. TLS for PostgreSQL connections** -- Hard blocker for production. Cannot connect to cloud databases without it. Scoped change to `pg.rs` (add SSLRequest + rustls wrapping). Does not affect other features. Unblocks all real-world PostgreSQL usage.

**2. TLS for HTTPS server** -- Second production blocker. Enable `ssl-rustls` on `tiny_http`, add `serve_tls` function. Small, self-contained change. Shares `rustls` dependency with PG TLS.

**3. Connection pooling** -- Foundational for production workloads. Enables sharing connections across concurrent request handlers. Actor-based design leverages Snow's strengths. Required before transactions make ergonomic sense (transactions need a connection to operate on, and pooling is how connections are managed in production).

**4. Database transactions** -- Builds on pooling. Block-based `Db.transaction(conn, fn)` with automatic BEGIN/COMMIT/ROLLBACK. Essential for data consistency. Nested savepoints as stretch goal.

**5. Struct-to-row mapping (deriving(Db))** -- Quality-of-life improvement. Follows proven `deriving(Json)` pattern. Can be built independently at any point but is most valuable after the data pipeline (pool + transactions) is solid.

### Prioritize:
1. PostgreSQL TLS -- production database connectivity
2. HTTPS server -- production HTTP serving
3. Connection pooling -- production concurrency
4. Database transactions -- production data integrity
5. Struct-to-row mapping -- developer ergonomics

### Defer:
- **Savepoint-based nested transactions**: Ship flat transactions first, add nesting later
- **Custom CA certificate paths**: Use system certs and `webpki-roots` initially
- **Dynamic pool resizing**: Static min/max is fine
- **Column renaming in deriving(Db)**: Requires attribute syntax
- **Per-route pool affinity**: Single pool per database is sufficient
- **Ecto.Multi-style composable transactions**: Too complex for v3.0

---

## Complexity Assessment

| Feature | Estimated Effort | Risk | Notes |
|---------|-----------------|------|-------|
| PostgreSQL TLS | 3-5 days | **MEDIUM** | Core challenge: abstracting PgConn's stream type to support both plain and TLS. SSLRequest protocol is simple. Rustls integration is well-documented. Risk: ensuring Read/Write trait works identically for both stream types. |
| HTTPS server | 1-2 days | LOW | `tiny_http` has built-in support. Enable feature flag, add `serve_tls` function, load PEM files. Minimal risk. |
| Connection pooling | 5-8 days | **HIGH** | Most complex feature. Actor-based pool with checkout/checkin, health validation, idle timeout, exhaustion handling. Risk: subtle concurrency bugs in pool state management, connection leak paths, cleanup on actor crash. |
| Database transactions | 3-4 days | MEDIUM | Straightforward BEGIN/COMMIT/ROLLBACK wrapping. Risk: ensuring ROLLBACK on all error/panic paths. Savepoint nesting adds complexity if included. |
| Struct-to-row mapping | 3-5 days | MEDIUM | Follows `deriving(Json)` pattern but with string-to-type coercion. Risk: handling all type combinations correctly (Int, Float, Bool, String, Option variants). |

**Total estimated effort:** 15-24 days

**Key risks:**
1. **Connection pool concurrency.** Multiple actors checking out/in simultaneously. The pool actor must handle messages atomically. Risk of deadlock if a transaction holds a connection and the same actor tries to check out another.
2. **TLS stream abstraction.** The `PgStream` enum must implement `Read + Write` identically to `TcpStream`. All existing wire protocol code (read_message, write_* functions) uses these traits. Must verify no code path assumes direct TcpStream access.
3. **Transaction cleanup on panic.** If a Snow actor panics inside a `Db.transaction` closure, the ROLLBACK must still execute. The existing `catch_unwind` in actor-per-connection HTTP handlers provides a pattern, but transactions add a nested cleanup requirement.
4. **Pool connection leak.** If a checked-out connection is never returned (actor crashes without checkin), the pool must reclaim it. Elixir handles this via ETS heir mechanism. Snow needs an equivalent -- possibly monitoring the caller actor's lifecycle.

---

## Sources

### Connection Pooling
- [Elixir db_connection source (connection_pool.ex)](https://github.com/elixir-ecto/db_connection/blob/master/lib/db_connection/connection_pool.ex) -- HIGH confidence
- [Elixir db_connection Ownership model](https://hexdocs.pm/db_connection/DBConnection.Ownership.html) -- HIGH confidence
- [Poolboy (Erlang worker pool)](https://github.com/devinus/poolboy) -- HIGH confidence
- [HikariCP best practices](https://www.baeldung.com/spring-boot-hikari) -- MEDIUM confidence
- [SQLAlchemy connection pooling docs](https://docs.sqlalchemy.org/en/20/core/pooling.html) -- HIGH confidence
- [Stack Overflow: connection pooling overview](https://stackoverflow.blog/2020/10/14/improve-database-performance-with-connection-pooling/) -- MEDIUM confidence

### TLS/SSL
- [PostgreSQL wire protocol - Message Flow (SSLRequest)](https://www.postgresql.org/docs/current/protocol-flow.html) -- HIGH confidence
- [PostgreSQL wire protocol - Message Formats (SSLRequest bytes)](https://www.postgresql.org/docs/current/protocol-message-formats.html) -- HIGH confidence
- [jackc/pgproto3 SSLRequest Go implementation](https://github.com/jackc/pgproto3/blob/master/ssl_request.go) -- HIGH confidence
- [tiny_http SSL example](https://github.com/tiny-http/tiny-http/blob/master/examples/ssl.rs) -- HIGH confidence
- [tiny_http SslConfig docs](https://docs.rs/tiny_http/latest/tiny_http/struct.SslConfig.html) -- HIGH confidence
- [rustls GitHub (StreamOwned pattern)](https://github.com/rustls/rustls) -- HIGH confidence

### Database Transactions
- [Elixir Ecto transaction guide (Curiosum)](https://www.curiosum.com/blog/elixir-ecto-database-transactions) -- MEDIUM confidence
- [Ecto.Repo.transaction nesting behavior](https://medium.com/@takanori.ishikawa/ecto-repo-transaction-3-can-be-nested-82b83545dfb0) -- MEDIUM confidence
- [Django atomic() transactions](https://docs.djangoproject.com/en/5.2/topics/db/transactions/) -- HIGH confidence
- [EF Core transactions (savepoints)](https://learn.microsoft.com/en-us/ef/core/saving/transactions) -- HIGH confidence
- [CockroachDB SAVEPOINT docs](https://www.cockroachlabs.com/docs/stable/savepoint) -- HIGH confidence

### Struct-to-Row Mapping
- [sqlx FromRow derive macro](https://docs.rs/sqlx/latest/sqlx/trait.FromRow.html) -- HIGH confidence
- [SQLBoiler (Go code generation ORM)](https://github.com/aarondl/sqlboiler) -- MEDIUM confidence
- [Rust ORM comparison: SQLx vs Diesel](https://infobytes.guru/articles/rust-orm-comparison-sqlx-diesel.html) -- MEDIUM confidence
- [Raw+DC pattern (2026)](https://mkennedy.codes/posts/raw-dc-the-orm-pattern-of-2026/) -- LOW confidence

### Snow Codebase (direct inspection)
- `crates/snow-rt/src/db/pg.rs` -- PgConn with TcpStream, wire protocol, SCRAM-SHA-256 auth
- `crates/snow-rt/src/db/sqlite.rs` -- SqliteConn with sqlite3 FFI, parameterized queries
- `crates/snow-rt/src/http/server.rs` -- actor-per-connection HTTP with tiny_http, middleware chain
- `crates/snow-rt/src/actor/service.rs` -- snow_service_call for synchronous request/reply
- `crates/snow-rt/src/actor/mod.rs` -- scheduler, spawn, registry infrastructure
- `crates/snow-rt/src/json.rs` -- SnowJson runtime type, deriving(Json) runtime support
- `crates/snow-rt/Cargo.toml` -- current dependencies (tiny_http 0.12, rustls not yet present)
- `tests/e2e/stdlib_pg.snow` -- existing PostgreSQL E2E test showing current API
- `tests/e2e/stdlib_sqlite.snow` -- existing SQLite E2E test showing current API
- `tests/e2e/deriving_json_basic.snow` -- existing deriving(Json) pattern to follow

---
*Feature research for: Snow Language v3.0 Production Backend*
*Researched: 2026-02-12*
