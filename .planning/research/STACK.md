# Technology Stack: v3.0 Production Backend Features

**Project:** Snow compiler -- connection pooling, TLS/SSL (PostgreSQL + HTTPS), database transactions, struct-to-row mapping
**Researched:** 2026-02-12
**Confidence:** HIGH (codebase analysis of snow-rt, verified crate versions via `cargo tree` and crates.io/docs.rs, PostgreSQL wire protocol docs, rustls official docs)

## Executive Summary

This milestone adds **production-grade backend capabilities** to Snow's existing database and HTTP infrastructure. The work requires **3 new direct Rust crate dependencies** in `snow-rt` (for TLS), but critically, **all of their transitive dependencies are already in the build** via `ureq 2`. This means zero new crate compilations for TLS support. Connection pooling, transactions, and struct-to-row mapping require **zero new dependencies** -- they build on existing `crossbeam-channel`, `parking_lot`, and the codegen deriving infrastructure.

The most significant decision is TLS strategy. Snow currently uses `tiny_http 0.12` (HTTP server) and `ureq 2` (HTTP client), with a hand-rolled PostgreSQL wire protocol over raw `std::net::TcpStream`. TLS must be added to all three communication paths. The recommended approach is to use **rustls 0.23** directly (not through `tiny_http`'s `ssl-rustls` feature, which depends on the obsolete `rustls 0.20`). This means wrapping `TcpStream` in `rustls::StreamOwned` at the runtime level, giving Snow a single TLS implementation shared across PostgreSQL, HTTP server, and HTTP client.

**Key discovery:** `ureq 2` already pulls in `rustls 0.23.36`, `ring 0.17.14`, `rustls-pki-types 1.14.0`, and `webpki-roots 1.0.6` as transitive dependencies (verified via `cargo tree`). Adding `rustls` as a direct dependency simply promotes an existing transitive dep to a direct one -- no new code to compile, no build time increase.

## Recommended Stack

### Core Framework (NO CHANGES)

| Technology | Version | Purpose | Status |
|------------|---------|---------|--------|
| Rust | stable 2021 edition | Compiler implementation | No change |
| Inkwell | 0.8.0 (`llvm21-1`) | LLVM IR generation | No change |
| LLVM | 21.1 | Backend codegen + optimization | No change |
| corosensei | 0.3 | Stackful coroutines for actors | No change |
| crossbeam-channel | 0.5 | MPMC channels for actor mailboxes | No change (reused for pool) |
| parking_lot | 0.12 | Fast mutexes/condvars | No change (reused for pool) |

### Runtime (snow-rt) -- NEW DIRECT DEPENDENCIES

These are all already present as transitive dependencies of `ureq 2`. Adding them as direct deps enables Snow to use their APIs directly without adding any new crate compilations.

| Technology | Version | Purpose | Why This |
|------------|---------|---------|----------|
| rustls | 0.23 | TLS 1.2/1.3 for PostgreSQL and HTTPS | Pure Rust, no OpenSSL dependency, blocking I/O via `StreamOwned`, single-binary friendly. Use with `ring` crypto provider for simpler cross-platform builds. Already resolved to 0.23.36 via ureq. |
| webpki-roots | 1.0 | Mozilla root CA certificates | Compiled-in root certificates -- no system cert store dependency, deterministic across platforms. Required for validating server certificates (PostgreSQL cloud providers, HTTPS endpoints). Already resolved to 1.0.6 via ureq. |
| rustls-pki-types | 1 | PEM parsing for custom certificates | Replaces the unmaintained `rustls-pemfile`. Provides `PemObject` trait for loading custom CA certs and server key/cert pairs (needed for HTTPS server). Already resolved to 1.14.0 via rustls. |

### Runtime (snow-rt) -- EXISTING DEPENDENCIES REUSED

| Technology | Version | New Use | Why No New Dep |
|------------|---------|---------|----------------|
| crossbeam-channel | 0.5 | Connection pool checkout/return channel | Already used for actor mailboxes. Bounded MPMC channel is the ideal primitive for a sync connection pool. |
| parking_lot | 0.12 | Pool metadata mutex, health-check condvar | Already used for GC locks and scheduler state. |
| serde_json | 1 | (unchanged) | Still used for JSON parse/encode in runtime. |
| sha2/hmac/pbkdf2/base64/rand | (existing) | (unchanged) | Still used for PostgreSQL SCRAM-SHA-256 auth. |

### What NOT to Add

| Crate | Why NOT |
|-------|---------|
| `native-tls` | Wraps platform TLS (OpenSSL on Linux, SChannel on Windows, Security.framework on macOS). Defeats Snow's single-binary philosophy -- requires system libraries at runtime. `rustls` compiles everything into the binary. |
| `openssl` / `openssl-sys` | C dependency, requires `libssl-dev` installed. Cross-compilation nightmare. Conflicts with Snow's zero-system-dependency approach (bundled SQLite, pure Rust PG protocol). |
| `tiny_http` with `ssl-rustls` feature | Depends on `rustls 0.20` (3 major versions behind, from 2022). Would create a version conflict with `rustls 0.23` used for PostgreSQL TLS. Instead, handle TLS at the TCP level before passing to `tiny_http`. |
| `tokio-rustls` | Async TLS wrapper for Tokio. Snow uses blocking I/O on corosensei coroutines -- Tokio is not in the picture. Use `rustls::StreamOwned` directly for blocking TLS. |
| `r2d2` / `deadpool` / `bb8` | Generic connection pool crates. `r2d2` uses `std::sync::Mutex` (slower than parking_lot). `deadpool`/`bb8` are async-first. A custom pool using `crossbeam-channel` (already a dependency) is ~80 lines, perfectly fits the actor model, and avoids a new dependency. |
| `aws-lc-rs` (rustls default provider) | The default crypto provider for rustls 0.23. Requires a C/assembly build step via `cmake`. Cross-compilation from macOS to Linux is fragile. Use `ring` instead -- easier to build, sufficient feature set (no post-quantum needed for a language runtime). |
| `ureq 3` | Major rewrite with breaking API changes. Snow's HTTP client usage is minimal (2 functions: `snow_http_get`, `snow_http_post`). Upgrading from ureq 2 to ureq 3 gains nothing for this milestone and risks churn. Consider upgrading separately. |

## Feature-by-Feature Stack Analysis

### 1. TLS/SSL -- PostgreSQL (SSLRequest Wire Protocol)

**Approach:** Send SSLRequest before StartupMessage on existing `TcpStream`, upgrade to `rustls::StreamOwned<ClientConnection, TcpStream>`, then continue normal wire protocol over the encrypted stream.

**PostgreSQL SSLRequest flow:**
```
Client                                Server
  |  SSLRequest (8 bytes)              |
  | ---------------------------------> |
  |  'S' (1 byte = SSL accepted)       |
  | <--------------------------------- |
  |  TLS ClientHello                   |
  | ---------------------------------> |
  |  TLS ServerHello + handshake       |
  | <--------------------------------- |
  |  ... TLS established ...           |
  |  StartupMessage (over TLS)         |
  | ---------------------------------> |
  |  AuthenticationRequest (over TLS)  |
  | <--------------------------------- |
  |  ... normal PG protocol ...        |
```

The SSLRequest message is exactly 8 bytes: `Int32(8) Int32(80877103)`. The server responds with a single byte: `S` (accept) or `N` (reject). If `S`, the client performs a TLS handshake, then sends StartupMessage over the encrypted channel.

**Key integration point:** The existing `PgConn` struct holds `stream: TcpStream`. After TLS, it must hold either a `TcpStream` or a `StreamOwned<ClientConnection, TcpStream>`. Use an enum:

```rust
enum PgStream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}
```

Both variants implement `Read + Write`, so `read_message()` and all write functions work unchanged by operating on `&mut dyn Read + Write` instead of `&mut TcpStream`.

**Connection URL extension:** Add `sslmode` parameter to the existing URL parser:
- `postgres://user:pass@host/db` -- plain (current behavior)
- `postgres://user:pass@host/db?sslmode=require` -- TLS required
- `postgres://user:pass@host/db?sslmode=prefer` -- try TLS, fall back to plain
- `postgres://user:pass@host/db?sslmode=disable` -- no TLS (explicit)

**Cargo.toml additions (snow-rt):**

```toml
# TLS for PostgreSQL and HTTPS
rustls = { version = "0.23", default-features = false, features = ["ring", "tls12", "logging", "std"] }
webpki-roots = "1.0"
rustls-pki-types = { version = "1", features = ["std"] }
```

Note: `default-features = false` disables `aws-lc-rs` (the default crypto provider). The `ring` feature enables the `ring`-based provider instead, which is easier to build cross-platform. The `tls12` feature ensures compatibility with PostgreSQL servers that only support TLS 1.2.

**Confidence:** HIGH -- SSLRequest is documented in the [PostgreSQL protocol docs](https://www.postgresql.org/docs/current/protocol-flow.html). `rustls::StreamOwned` is explicitly designed for wrapping blocking `TcpStream` -- it implements `Read` + `Write` and does not require an async runtime.

---

### 2. TLS/SSL -- HTTPS Server

**Approach:** Replace `tiny_http` with a minimal hand-rolled HTTP/1.1 parser, enabling native `rustls::ServerConnection` integration for HTTPS. Do NOT use `tiny_http`'s `ssl-rustls` feature (it uses `rustls 0.20`, three major versions behind).

**Why replace tiny_http:**
- `tiny_http 0.12`'s `ssl-rustls` feature depends on `rustls 0.20` (from 2022). This is incompatible with `rustls 0.23` needed for PostgreSQL TLS.
- `tiny_http::Server` does not support receiving pre-established TLS streams.
- Snow's HTTP server already does minimal work -- `tiny_http` provides HTTP parsing, but the router, middleware, and actor dispatch are all in `snow-rt`.
- The HTTP parsing that tiny_http does is straightforward: read request line, read headers, read body, write response (~200 lines of Rust).

**Recommendation:** Replace `tiny_http` with a minimal hand-rolled HTTP/1.1 parser + `rustls` for TLS. This:
- Eliminates the `rustls 0.20` version conflict
- Creates a single TLS implementation for PG + HTTP
- Gives full control over connection lifecycle (important for actor-per-connection model)
- Removes a dependency with an outdated TLS stack

**Server certificate loading:**

```rust
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

// Load from PEM files (user provides cert + key paths)
let certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(cert_path)
    .expect("cert file")
    .collect::<Result<Vec<_>, _>>()
    .expect("valid certs");
let key = PrivateKeyDer::from_pem_file(key_path)
    .expect("key file");
```

**New API in Snow:**

```snow
// HTTP (plain, existing)
Http.serve(router, 8080)

// HTTPS (new)
Http.serve_tls(router, 8443, "cert.pem", "key.pem")
```

**Confidence:** MEDIUM-HIGH -- replacing `tiny_http` is more work than enabling a feature flag, but it eliminates the `rustls 0.20` version conflict and gives full control. The HTTP parsing is straightforward (Snow only needs HTTP/1.1, no HTTP/2, no chunked transfer encoding for the server side).

---

### 3. TLS/SSL -- HTTP Client

**Approach:** `ureq 2` already uses `rustls` for HTTPS. No changes needed -- HTTPS client works out of the box.

**Verification (via `cargo tree`):** `ureq 2` already resolves to `rustls 0.23.36` with `ring 0.17.14`. Since we use `rustls 0.23` as a direct dependency (which satisfies the same semver range), Cargo's version resolution will unify to a single `rustls` version. No conflict.

**Crypto provider coordination:** Since we use the `ring` feature for rustls (instead of default `aws-lc-rs`), we need to install the `ring` provider at startup so that both our direct rustls usage and ureq's internal TLS use the same provider:

```rust
// In snow-rt initialization (called once at program start)
rustls::crypto::ring::default_provider()
    .install_default()
    .expect("ring crypto provider");
```

This must happen before any TLS connection (PostgreSQL or HTTP client).

**Confidence:** HIGH -- verified via `cargo tree` that `ureq 2` and our direct `rustls 0.23` dependency share the same resolved version.

---

### 4. Connection Pooling

**Approach:** Build a synchronous connection pool using `crossbeam-channel` (bounded MPMC) and `parking_lot::Mutex`. No new crate dependencies.

**Architecture:**

```
                  crossbeam-channel (bounded)
Pool::checkout() ----recv()----> [conn1, conn2, conn3, ...]
Pool::checkin()  ----send()----> [conn returned to channel]

Pool {
    available: crossbeam_channel::Receiver<ConnHandle>,
    return_tx: crossbeam_channel::Sender<ConnHandle>,
    config: PoolConfig,
    state: parking_lot::Mutex<PoolState>,
}
```

The bounded channel acts as the pool itself. `checkout()` calls `recv_timeout()` (blocking wait with timeout). `checkin()` calls `send()` to return a connection. The channel capacity IS the pool size.

**Why crossbeam-channel not a Vec+Mutex:**
- `crossbeam-channel` provides blocking wait with timeout (`recv_timeout`) -- no spin-loop needed
- MPMC: multiple actors can checkout/checkin concurrently without contention on a single mutex
- Already a dependency of `snow-rt` (used for actor mailboxes)

**Pool configuration:**

```snow
let pool = Pg.pool("postgres://user:pass@host/db", {
    min_connections: 2,
    max_connections: 10,
    checkout_timeout: 5000,   // ms
    idle_timeout: 300000,     // ms (5 min)
    max_lifetime: 3600000,    // ms (1 hour)
})
```

**Runtime representation:** The pool is an opaque u64 handle (same pattern as `PgConn` -- `Box::into_raw` as u64, GC-safe).

**New runtime functions:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_pg_pool_new` | `(url: ptr, config: ptr) -> ptr` | Create pool, returns `Result<PoolHandle, String>` |
| `snow_pg_pool_checkout` | `(pool: u64) -> ptr` | Get connection, returns `Result<ConnHandle, String>` |
| `snow_pg_pool_checkin` | `(pool: u64, conn: u64) -> void` | Return connection to pool |
| `snow_pg_pool_close` | `(pool: u64) -> void` | Drain and close all connections |
| `snow_pg_pool_query` | `(pool: u64, sql: ptr, params: ptr) -> ptr` | Auto checkout+query+checkin |
| `snow_pg_pool_execute` | `(pool: u64, sql: ptr, params: ptr) -> ptr` | Auto checkout+execute+checkin |

The `snow_pg_pool_query`/`snow_pg_pool_execute` convenience functions handle the checkout-use-checkin lifecycle automatically, which is the common case. Manual checkout/checkin is available for transactions.

**Health checking:** A background thread (spawned at pool creation) periodically sends `SELECT 1` on idle connections and removes stale ones. Uses `parking_lot::Condvar` for timed wake-ups.

**SQLite pooling:** SQLite connections are typically single-writer, so pooling is less useful. However, a read-only pool (WAL mode) could share the same infrastructure. Defer to a later milestone.

**Confidence:** HIGH -- synchronous connection pooling with bounded channels is a well-established pattern. The existing `crossbeam-channel` and `parking_lot` dependencies provide all needed primitives.

---

### 5. Database Transactions

**Approach:** Implement transactions as a state machine on the PostgreSQL connection, using the existing `snow_pg_execute` for `BEGIN`/`COMMIT`/`ROLLBACK` and tracking transaction state in `PgConn`.

**PostgreSQL transaction wire protocol:** PostgreSQL transactions are simply SQL commands (`BEGIN`, `COMMIT`, `ROLLBACK`) sent over the existing Extended Query protocol. The ReadyForQuery message includes a transaction status byte:
- `I` = idle (not in transaction)
- `T` = in transaction block
- `E` = in failed transaction block (only ROLLBACK allowed)

**PgConn state tracking:**

```rust
struct PgConn {
    stream: PgStream,  // Plain or TLS
    txn_status: u8,    // 'I', 'T', or 'E' from last ReadyForQuery
}
```

The runtime already reads ReadyForQuery messages (the `b'Z'` match arm in the message loop at `snow-rt/src/db/pg.rs`). Currently it ignores the status byte. Adding `txn_status` tracking requires saving `body[0]` into the connection struct.

**New runtime functions:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_pg_begin` | `(conn: u64) -> ptr` | Send `BEGIN`, returns `Result<(), String>` |
| `snow_pg_commit` | `(conn: u64) -> ptr` | Send `COMMIT`, returns `Result<(), String>` |
| `snow_pg_rollback` | `(conn: u64) -> ptr` | Send `ROLLBACK`, returns `Result<(), String>` |
| `snow_pg_in_transaction` | `(conn: u64) -> i8` | Returns 1 if in transaction, 0 otherwise |

**Transaction + Pool integration:**

```snow
let conn = Pg.pool_checkout(pool)?
let result = Pg.begin(conn)
    |> fn(_) { Pg.execute(conn, "INSERT INTO users ...", []) }
    |> fn(_) { Pg.execute(conn, "INSERT INTO logs ...", []) }
    |> fn(_) { Pg.commit(conn) }

match result {
    Ok(_) -> Pg.pool_checkin(pool, conn)
    Err(e) -> {
        Pg.rollback(conn)
        Pg.pool_checkin(pool, conn)
        Err(e)
    }
}
```

**Savepoints:** PostgreSQL supports `SAVEPOINT name` / `RELEASE SAVEPOINT name` / `ROLLBACK TO SAVEPOINT name` for nested transactions. These are just SQL commands -- no special wire protocol support needed. Expose as optional API in a later milestone.

**SQLite transactions:** Same approach -- `BEGIN`, `COMMIT`, `ROLLBACK` are SQL commands executed via `snow_sqlite_execute`. Add corresponding `snow_sqlite_begin`/`snow_sqlite_commit`/`snow_sqlite_rollback` convenience functions.

**Stack requirement:** NONE new. Transactions are pure protocol-level operations using existing infrastructure.

**Confidence:** HIGH -- PostgreSQL transactions are standard SQL commands with well-defined wire protocol behavior. The status byte in ReadyForQuery is already received but currently ignored.

---

### 6. Struct-to-Row Mapping (deriving(Row))

**Approach:** Compile-time code generation via `deriving(Row)`, following the exact same pattern as the existing `deriving(Json)`. The compiler generates `to_row` and `from_row` functions that map struct fields to/from database column values.

**How it works:**

```snow
struct User {
    id: Int,
    name: String,
    email: String,
    active: Bool
} deriving(Row)

// Generated by compiler:
// User_from_row(row: Map<String, String>) -> Result<User, String>
//   1. Map.get(row, "id")    -> parse as Int
//   2. Map.get(row, "name")  -> use as String
//   3. Map.get(row, "email") -> use as String
//   4. Map.get(row, "active") -> parse as Bool
//   5. Return Ok(User { id, name, email, active })

// User_to_params(self: User) -> List<String>
//   1. [Int.to_string(self.id), self.name, self.email, Bool.to_string(self.active)]
```

**Key design decisions:**

1. **Column name mapping:** By default, field names map directly to column names (snake_case). The existing query returns `Map<String, String>` where keys are column names from `RowDescription`. This matches perfectly.

2. **Type conversion:** All PostgreSQL column values arrive as text (the current implementation uses text format in the Bind message). Conversion from text to Snow types:
   - `String` -> no conversion needed
   - `Int` -> parse text as i64
   - `Float` -> parse text as f64
   - `Bool` -> parse "t"/"true"/"1" as true, "f"/"false"/"0" as false
   - `Option<T>` -> empty string (NULL) maps to `None`, otherwise `Some(parse(value))`

3. **Runtime support functions needed:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_db_row_get` | `(row: ptr, col_name: ptr) -> ptr` | Get column value by name from row map, returns `Option<String>` |
| `snow_db_parse_int` | `(text: ptr) -> ptr` | Parse text to Int, returns `Result<Int, String>` |
| `snow_db_parse_float` | `(text: ptr) -> ptr` | Parse text to Float, returns `Result<Float, String>` |
| `snow_db_parse_bool` | `(text: ptr) -> ptr` | Parse text to Bool, returns `Result<Bool, String>` |

**Compiler changes:**

- **Typeck:** Add "Row" to recognized deriving trait names. Validate all fields are Row-mappable types (Int, Float, String, Bool, Option of those).
- **MIR lowering:** In `lower_struct_def`, when `derive_list` contains "Row", generate `from_row` and `to_params` MIR functions. Pattern follows existing `generate_debug_inspect_struct`, `generate_eq_struct`, etc.
- **Codegen:** No new codegen nodes -- generated MIR uses existing `Call`, `FieldAccess`, `StructLit` nodes.

**Usage at the Snow level:**

```snow
// Automatic row mapping
let users: List<User> = Pg.query_as(pool, "SELECT id, name, email, active FROM users WHERE active = $1", ["t"])?

// Under the hood, query_as calls:
// 1. Pg.query(pool, sql, params) -> List<Map<String, String>>
// 2. List.map(rows, User_from_row) -> List<Result<User, String>>
// 3. Collect results, fail on first error
```

**New runtime function for query_as:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_pg_query_as` | `(pool: u64, sql: ptr, params: ptr, from_row_fn: ptr) -> ptr` | Query + map rows using generated from_row function |

The `from_row_fn` parameter is a function pointer to the compiler-generated `User_from_row` function. The runtime calls it for each row.

**Stack requirement:** NONE new. Uses existing string operations, map lookups, and the deriving infrastructure in the compiler.

**Confidence:** HIGH -- follows the identical pattern as `deriving(Json)` which is already implemented. The main difference is the source data format (Map<String, String> from database rows vs SnowJson tagged union from JSON).

---

## Integration Points with Existing Crates

### snow-rt/Cargo.toml (CHANGES)

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
ureq = "2"
libsqlite3-sys = { version = "0.36", features = ["bundled"] }
sha2 = "0.10"
hmac = "0.12"
md-5 = "0.10"
pbkdf2 = { version = "0.12", default-features = false, features = ["hmac"] }
base64 = "0.22"
rand = "0.9"

# REMOVED (replaced by hand-rolled HTTP parser + rustls)
# tiny_http = "0.12"

# NEW: TLS for PostgreSQL + HTTPS server
# Note: rustls, ring, webpki-roots, and rustls-pki-types are already
# transitive deps of ureq 2 -- adding them as direct deps enables
# our code to use their APIs without adding any new crate compilations.
rustls = { version = "0.23", default-features = false, features = ["ring", "tls12", "logging", "std"] }
webpki-roots = "1.0"
rustls-pki-types = { version = "1", features = ["std"] }
```

### snow-rt Source File Changes

| File | Change Type | Purpose | Est. Lines |
|------|-------------|---------|------------|
| `db/pg.rs` | Modify | Add TLS upgrade (SSLRequest, StreamOwned), PgStream enum, txn_status tracking | +120 |
| `db/pg_pool.rs` | New | Connection pool (crossbeam-channel based) | ~150 |
| `db/pg_tls.rs` | New | rustls ClientConfig creation, certificate loading | ~80 |
| `db/pg_txn.rs` | New | Transaction begin/commit/rollback convenience functions | ~60 |
| `db/row.rs` | New | Row parsing helpers (parse_int, parse_float, parse_bool from text) | ~80 |
| `http/server.rs` | Rewrite | Replace tiny_http with hand-rolled HTTP/1.1 parser + rustls | ~350 |
| `http/tls.rs` | New | HTTPS ServerConfig, certificate loading, TLS accept | ~80 |
| `tls.rs` | New | Shared TLS initialization (ring crypto provider install) | ~30 |

### snow-codegen Changes

| File | Change Type | Purpose | Est. Lines |
|------|-------------|---------|------------|
| `mir/lower.rs` | Modify | Add `deriving(Row)` dispatch in struct lowering | +10 |
| `mir/lower.rs` | Modify | Generate `from_row` and `to_params` MIR functions | +120 |
| `codegen/intrinsics.rs` | Modify | Declare pool, txn, TLS, and row-mapping intrinsics | +40 |
| `codegen/db.rs` | New or modify | Route pool/txn/query_as calls to runtime functions | +30 |

### snow-typeck Changes

| File | Change Type | Purpose | Est. Lines |
|------|-------------|---------|------------|
| Stdlib module types | Modify | Add pool, transaction, TLS function signatures | +40 |
| Deriving validation | Modify | Add "Row" to recognized deriving traits | +5 |

### Linker (snow-codegen/link.rs) -- NO CHANGES

`rustls` with `ring` compiles to pure Rust + assembly (ring's crypto primitives). Everything links into `libsnow_rt.a`. No additional `-l` flags needed.

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| TLS library | `rustls 0.23` (ring provider) | `native-tls` | System dependency, non-deterministic behavior across platforms, defeats single-binary philosophy |
| TLS library | `rustls 0.23` (ring provider) | `rustls 0.23` (aws-lc-rs provider) | aws-lc-rs requires cmake for C build step, fragile cross-compilation from macOS to Linux. ring is pure Rust + asm, easier to build. |
| HTTPS server | Replace tiny_http + use rustls directly | `tiny_http` with `ssl-rustls` feature | Feature depends on `rustls 0.20` (3 major versions behind, 2022 era). Version conflict with our `rustls 0.23` for PostgreSQL TLS. |
| HTTPS server | Replace tiny_http + use rustls directly | Fork tiny_http, update rustls | Maintenance burden of maintaining a fork. Better to own the HTTP parsing (~200 lines) than maintain a fork of a low-activity project. |
| Connection pool | Custom (crossbeam-channel) | `r2d2` | Uses `std::sync::Mutex` (slower than parking_lot). Adds a dependency for ~80 lines of pool logic. Designed for generic resource pooling -- we only need database connections. |
| Connection pool | Custom (crossbeam-channel) | `deadpool` | Async-first (requires tokio). Incompatible with Snow's sync actor model. |
| Transactions | SQL commands over existing protocol | Dedicated transaction protocol layer | PostgreSQL transactions are just `BEGIN`/`COMMIT`/`ROLLBACK` SQL commands. No special protocol support needed. A complex abstraction layer would over-engineer a simple feature. |
| Struct-to-row | Compile-time codegen (deriving(Row)) | Runtime reflection | Same reasoning as deriving(Json) -- codegen is zero-cost, reflection requires type metadata in binaries. |
| Struct-to-row | Field name = column name | Annotation-based column mapping | Keep it simple for MVP. Column name aliasing can use SQL aliases (`SELECT user_id AS userId`). Annotations add parser complexity for marginal benefit. |

## Dependency Graph Impact

Verified via `cargo tree -p snow-rt` on 2026-02-12:

```
snow-rt (current)
  |-- ureq 2 (existing)
  |     |-- rustls 0.23.36          <-- ALREADY COMPILED
  |     |     |-- ring 0.17.14      <-- ALREADY COMPILED
  |     |     |-- rustls-pki-types 1.14.0  <-- ALREADY COMPILED
  |     |     |-- rustls-webpki 0.103.9    <-- ALREADY COMPILED
  |     |     |-- zeroize 1.8.2     <-- ALREADY COMPILED
  |     |     |-- subtle 2.6.1      <-- ALREADY COMPILED
  |     |     |-- once_cell 1.21.3  <-- ALREADY COMPILED
  |     |-- webpki-roots 0.26.11    <-- ALREADY COMPILED
  |     |     |-- webpki-roots 1.0.6  <-- ALREADY COMPILED
  |-- tiny_http 0.12 (REMOVED)
  |-- crossbeam-channel 0.5 (existing, reused for pool)
  |-- parking_lot 0.12 (existing, reused for pool)
  |-- libsqlite3-sys 0.36 (existing)
  |-- sha2/hmac/md-5/pbkdf2/base64/rand (existing)
  |-- corosensei 0.3 (existing)
  |-- serde_json 1 (existing)

After changes:
  |-- rustls 0.23 (NEW direct dep, resolves to existing 0.23.36)
  |-- webpki-roots 1.0 (NEW direct dep, resolves to existing 1.0.6)
  |-- rustls-pki-types 1 (NEW direct dep, resolves to existing 1.14.0)
```

**Net dependency change:** +3 direct deps (rustls, webpki-roots, rustls-pki-types), -1 direct dep (tiny_http). **Zero new transitive crates** -- all TLS crates are already compiled as transitive deps of `ureq 2`. This means no build time increase for TLS support.

## Installation

```bash
# Build runtime (TLS crates already compiled via ureq -- no build time increase)
cargo build -p snow-rt

# ring requires a C compiler for its assembly primitives
# On macOS: Xcode command line tools (already required for LLVM/SQLite)
# On Linux: build-essential (already required for LLVM/SQLite)
# No new system requirements beyond what LLVM already needs.
```

## Version Verification Matrix

All versions verified via `cargo tree -p snow-rt` on 2026-02-12:

| Crate | Spec in Cargo.toml | Resolved Version | Verified Via | Notes |
|-------|-------------------|------------------|--------------|-------|
| rustls | `0.23` (default-features=false, ring+tls12+logging+std) | 0.23.36 | `cargo tree` | Already transitive dep of ureq 2. MSRV Rust 1.71. |
| webpki-roots | `1.0` | 1.0.6 | `cargo tree` | Already transitive dep of ureq 2 (via 0.26.11 re-export). |
| rustls-pki-types | `1` (std feature) | 1.14.0 | `cargo tree` | Already transitive dep of rustls. Contains PEM parser (replaces unmaintained rustls-pemfile). |
| ring | (transitive via rustls ring feature) | 0.17.14 | `cargo tree` | Already transitive dep of ureq 2. |
| crossbeam-channel | `0.5` (existing) | 0.5.15 | `cargo tree` | Reused for connection pool. No change. |
| parking_lot | `0.12` (existing) | 0.12.5 | `cargo tree` | Reused for pool state. No change. |
| tiny_http | REMOVED | 0.12.0 | `cargo tree` | Uses rustls 0.20, incompatible with 0.23. |
| ureq | `2` (unchanged) | 2 (resolves current) | `cargo tree` | Uses rustls 0.23.36, compatible. |

## Sources

### PRIMARY (HIGH confidence -- official documentation + direct codebase analysis)
- [PostgreSQL Protocol Flow: SSL Session Encryption](https://www.postgresql.org/docs/current/protocol-flow.html) -- SSLRequest message format, S/N response byte
- [PostgreSQL Message Formats](https://www.postgresql.org/docs/current/protocol-message-formats.html) -- SSLRequest: Int32(8) Int32(80877103)
- [rustls docs: StreamOwned](https://docs.rs/rustls/latest/rustls/struct.StreamOwned.html) -- blocking TLS stream wrapper, implements Read+Write
- [rustls GitHub](https://github.com/rustls/rustls) -- 0.23.36 release, ring/aws-lc-rs provider system
- [rustls-pki-types: PemObject trait](https://docs.rs/rustls-pki-types/latest/rustls_pki_types/pem/trait.PemObject.html) -- PEM loading API (replaces unmaintained rustls-pemfile)
- [webpki-roots GitHub](https://github.com/rustls/webpki-roots) -- v1.0.6, Mozilla root CA bundle
- Snow codebase: `snow-rt/src/db/pg.rs` (PgConn struct, wire protocol, auth), `snow-rt/src/http/server.rs` (tiny_http usage), `snow-rt/src/http/client.rs` (ureq usage)
- `cargo tree -p snow-rt` output (2026-02-12) -- verified all resolved versions and transitive dep relationships

### SECONDARY (MEDIUM confidence -- ecosystem research)
- [tiny_http Cargo.toml](https://github.com/tiny-http/tiny-http/blob/master/Cargo.toml) -- confirms rustls 0.20 dependency in ssl-rustls feature
- [ureq crate](https://crates.io/crates/ureq) -- v2 uses rustls ^0.23.19
- [RUSTSEC-2025-0134](https://rustsec.org/advisories/RUSTSEC-2025-0134.html) -- rustls-pemfile unmaintained advisory (use rustls-pki-types instead)
- [rustls crypto providers](https://docs.rs/rustls/latest/rustls/#cryptography-providers) -- ring vs aws-lc-rs comparison

---
*Stack research for: Snow v3.0 Production Backend Features*
*Researched: 2026-02-12*
