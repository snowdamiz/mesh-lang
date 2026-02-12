# Phase 54: PostgreSQL Driver - Research

**Researched:** 2026-02-12
**Domain:** PostgreSQL wire protocol v3, SCRAM-SHA-256/MD5 authentication, compiler pipeline extension
**Confidence:** HIGH

## Summary

Phase 54 adds a PostgreSQL database driver to the Snow language. Users interact via a `Pg` module (`Pg.connect`, `Pg.query`, `Pg.execute`, `Pg.close`). The implementation follows the exact same compiler pipeline pattern established in Phase 53 (SQLite): runtime functions in `snow-rt` (extern "C"), intrinsic declarations in `codegen/intrinsics.rs`, known_functions in `mir/lower.rs`, module type signatures in `typeck/infer.rs` and `typeck/builtins.rs`, and builtin name mapping in `map_builtin_name`.

The critical difference from SQLite is the runtime layer. SQLite used `libsqlite3-sys` (a C FFI binding). PostgreSQL requires a **pure Rust TCP client** that implements the PostgreSQL wire protocol v3, including two authentication mechanisms: SCRAM-SHA-256 (production/cloud) and MD5 (local development). Requirement PG-06 mandates "pure wire protocol implementation (zero C dependencies beyond crypto crates)." This means the runtime code must open a TCP socket via `std::net::TcpStream`, send/receive binary protocol messages, and handle the SASL/MD5 authentication handshake entirely in Rust.

The recommended approach is to implement the wire protocol manually using low-level crypto crates (`sha2`, `hmac`, `md-5`, `base64`, `rand`, `pbkdf2`) rather than pulling in the full `postgres-protocol` crate. The `postgres-protocol` crate (0.6.10) is pure Rust and tokio-free, but it brings 10+ transitive dependencies and is designed as a building block for the full `rust-postgres` client. For the Snow runtime, which only needs 4 operations (connect, close, query, execute) with text-mode parameters, implementing the ~15 message types directly is more proportionate and avoids dependency bloat. The wire protocol is well-documented and stable.

**Primary recommendation:** Implement the PostgreSQL wire protocol v3 from scratch in `snow-rt/src/db/pg.rs` using `std::net::TcpStream` for networking and `sha2`/`hmac`/`md-5`/`pbkdf2`/`base64`/`rand` crates for authentication. Follow the SQLite compiler pipeline pattern exactly for the 4 new intrinsics.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `sha2` | 0.10 | SHA-256 hashing for SCRAM-SHA-256 auth | Standard pure-Rust SHA-2 from RustCrypto project. Used by `postgres-protocol` itself. |
| `hmac` | 0.12 | HMAC for SCRAM-SHA-256 auth | Standard pure-Rust HMAC from RustCrypto. Required for SCRAM client-proof computation. |
| `md-5` | 0.10 | MD5 hashing for MD5 auth | Standard pure-Rust MD5 from RustCrypto. Required for `md5(md5(password+user)+salt)` auth. |
| `pbkdf2` | 0.12 | PBKDF2 key derivation for SCRAM-SHA-256 | Standard pure-Rust PBKDF2 from RustCrypto. Required for SaltedPassword derivation in SCRAM. |
| `base64` | 0.22 | Base64 encode/decode for SCRAM messages | Standard pure-Rust base64. SCRAM messages are base64-encoded. |
| `rand` | 0.9 | Random nonce generation for SCRAM | Standard Rust RNG. SCRAM requires a random client nonce. |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `byteorder` | 1.0 | Big-endian integer read/write for wire protocol | PostgreSQL wire protocol uses network byte order (big-endian). All Int32/Int16 fields must be encoded/decoded in big-endian. |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Hand-rolled wire protocol | `postgres-protocol` crate (0.6.10) | `postgres-protocol` is pure Rust, no tokio, provides message encode/decode + SCRAM + MD5. But it pulls in 10+ dependencies (bytes, byteorder, fallible-iterator, memchr, stringprep, plus all crypto crates). For Snow's 4 operations with text-mode only, hand-rolling ~15 message types is ~500 lines and avoids the dependency tree. |
| Hand-rolled wire protocol | `postgres` crate (sync, 0.19.x) | Full-featured sync PG client, but internally wraps `tokio-postgres` with a hidden tokio runtime. Adds ~50+ transitive deps including the entire tokio ecosystem. Completely inappropriate for a language runtime. |
| Individual crypto crates | `scram-rs` crate | Full SCRAM library, but adds another dependency layer. The SCRAM-SHA-256 client-side flow is ~80 lines with raw `sha2`/`hmac`/`pbkdf2`. |
| `byteorder` | Manual byte manipulation | `byteorder` is a single-file crate with no deps. Manual `u32::to_be_bytes()` works too but `byteorder` is cleaner for Read/Write trait extensions. Actually, since Rust has `to_be_bytes()`/`from_be_bytes()` in std, `byteorder` may not be needed at all. |

**Dependency additions to `snow-rt/Cargo.toml`:**
```toml
[dependencies]
# Phase 54: PostgreSQL wire protocol
sha2 = "0.10"
hmac = "0.12"
md-5 = "0.10"
pbkdf2 = { version = "0.12", default-features = false, features = ["hmac"] }
base64 = "0.22"
rand = "0.9"
```

Note: `byteorder` is likely unnecessary since Rust std provides `i32::to_be_bytes()` etc. Use std primitives.

## Architecture Patterns

### Recommended File Structure
```
crates/snow-rt/src/
  db/
    mod.rs              # pub mod sqlite; pub mod pg;
    sqlite.rs           # (existing) SQLite C FFI wrapper functions
    pg.rs               # PostgreSQL pure wire protocol client
  lib.rs                # Add: re-exports for snow_pg_* functions

crates/snow-codegen/src/
  codegen/intrinsics.rs  # Add: snow_pg_* LLVM declarations
  mir/lower.rs           # Add: known_functions + map_builtin_name + STDLIB_MODULES entries

crates/snow-typeck/src/
  infer.rs               # Add: Pg module in stdlib_modules() + STDLIB_MODULE_NAMES
  builtins.rs            # Add: PgConn opaque type + pg_* function signatures

crates/snow-codegen/src/
  mir/types.rs           # Add: "PgConn" => MirType::Int in resolve_con

tests/e2e/
  stdlib_pg.snow         # E2E test (requires running PostgreSQL)
```

### Pattern 1: Exact Copy of SQLite Compiler Pipeline
**What:** The PostgreSQL module follows the identical 7-step compiler registration pattern as SQLite.
**Why:** Consistency. The pattern is proven across 53+ phases.
**Steps (copy from SQLite, rename):**

1. **`snow-typeck/src/builtins.rs`** -- Add `PgConn` opaque type and `pg_*` function signatures:
```rust
// PgConn opaque type -- lowered to Int (i64) at MIR level for GC safety.
let pg_conn_t = Ty::Con(TyCon::new("PgConn"));
env.insert("PgConn".into(), Scheme::mono(pg_conn_t.clone()));

// Pg.connect(String) -> Result<PgConn, String>
env.insert("pg_connect".into(), Scheme::mono(Ty::fun(
    vec![Ty::string()],
    Ty::result(pg_conn_t.clone(), Ty::string()),
)));
// Pg.close(PgConn) -> Unit
env.insert("pg_close".into(), Scheme::mono(Ty::fun(
    vec![pg_conn_t.clone()],
    Ty::Tuple(vec![]),
)));
// Pg.execute(PgConn, String, List<String>) -> Result<Int, String>
env.insert("pg_execute".into(), Scheme::mono(Ty::fun(
    vec![pg_conn_t.clone(), Ty::string(), Ty::list(Ty::string())],
    Ty::result(Ty::int(), Ty::string()),
)));
// Pg.query(PgConn, String, List<String>) -> Result<List<Map<String, String>>, String>
env.insert("pg_query".into(), Scheme::mono(Ty::fun(
    vec![pg_conn_t.clone(), Ty::string(), Ty::list(Ty::string())],
    Ty::result(Ty::list(Ty::map(Ty::string(), Ty::string())), Ty::string()),
)));
```

2. **`snow-typeck/src/infer.rs` -- `stdlib_modules()`** -- Add `Pg` module:
```rust
let pg_conn_t = Ty::Con(TyCon::new("PgConn"));
let mut pg_mod = HashMap::new();
pg_mod.insert("connect".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::string()], Ty::result(pg_conn_t.clone(), Ty::string()),
)));
pg_mod.insert("close".to_string(), Scheme::mono(Ty::fun(
    vec![pg_conn_t.clone()], Ty::Tuple(vec![]),
)));
pg_mod.insert("execute".to_string(), Scheme::mono(Ty::fun(
    vec![pg_conn_t.clone(), Ty::string(), Ty::list(Ty::string())],
    Ty::result(Ty::int(), Ty::string()),
)));
pg_mod.insert("query".to_string(), Scheme::mono(Ty::fun(
    vec![pg_conn_t.clone(), Ty::string(), Ty::list(Ty::string())],
    Ty::result(Ty::list(Ty::map(Ty::string(), Ty::string())), Ty::string()),
)));
modules.insert("Pg".to_string(), pg_mod);
```

3. **`snow-typeck/src/infer.rs` -- `STDLIB_MODULE_NAMES`** -- Add `"Pg"`:
```rust
const STDLIB_MODULE_NAMES: &[&str] = &[
    "String", "IO", "Env", "File", "List", "Map", "Set", "Tuple", "Range", "Queue",
    "HTTP", "JSON", "Json", "Request", "Job", "Math", "Int", "Float", "Timer",
    "Sqlite", "Pg",
];
```

4. **`snow-codegen/src/mir/types.rs` -- `resolve_con()`** -- Map `PgConn` to `MirType::Int`:
```rust
"PgConn" => MirType::Int,  // Opaque u64 handle, GC-safe (same as SqliteConn)
```

5. **`snow-codegen/src/mir/lower.rs` -- `known_functions`** -- Register function MIR types:
```rust
// Phase 54: PostgreSQL functions -- handle is MirType::Int (i64) for GC safety
self.known_functions.insert("snow_pg_connect".to_string(),
    MirType::FnPtr(vec![MirType::Ptr], Box::new(MirType::Ptr)));
self.known_functions.insert("snow_pg_close".to_string(),
    MirType::FnPtr(vec![MirType::Int], Box::new(MirType::Unit)));
self.known_functions.insert("snow_pg_execute".to_string(),
    MirType::FnPtr(vec![MirType::Int, MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)));
self.known_functions.insert("snow_pg_query".to_string(),
    MirType::FnPtr(vec![MirType::Int, MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)));
```

6. **`snow-codegen/src/mir/lower.rs` -- `map_builtin_name()`** -- Name mapping:
```rust
"pg_connect" => "snow_pg_connect".to_string(),
"pg_close" => "snow_pg_close".to_string(),
"pg_execute" => "snow_pg_execute".to_string(),
"pg_query" => "snow_pg_query".to_string(),
```

7. **`snow-codegen/src/mir/lower.rs` -- `STDLIB_MODULES`** -- Add `"Pg"`:
```rust
const STDLIB_MODULES: &[&str] = &[
    "String", "IO", "Env", "File", "List", "Map", "Set", "Tuple", "Range", "Queue",
    "HTTP", "JSON", "Json", "Request", "Job", "Math", "Int", "Float", "Timer",
    "Sqlite", "Pg",
];
```

8. **`snow-codegen/src/codegen/intrinsics.rs` -- `declare_intrinsics()`** -- LLVM declarations:
```rust
// Phase 54: PostgreSQL
// snow_pg_connect(url: ptr) -> ptr (SnowResult)
module.add_function("snow_pg_connect",
    ptr_type.fn_type(&[ptr_type.into()], false),
    Some(inkwell::module::Linkage::External));
// snow_pg_close(conn: i64) -> void
module.add_function("snow_pg_close",
    void_type.fn_type(&[i64_type.into()], false),
    Some(inkwell::module::Linkage::External));
// snow_pg_execute(conn: i64, sql: ptr, params: ptr) -> ptr (SnowResult)
module.add_function("snow_pg_execute",
    ptr_type.fn_type(&[i64_type.into(), ptr_type.into(), ptr_type.into()], false),
    Some(inkwell::module::Linkage::External));
// snow_pg_query(conn: i64, sql: ptr, params: ptr) -> ptr (SnowResult)
module.add_function("snow_pg_query",
    ptr_type.fn_type(&[i64_type.into(), ptr_type.into(), ptr_type.into()], false),
    Some(inkwell::module::Linkage::External));
```

### Pattern 2: PostgreSQL Wire Protocol Connection Flow
**What:** The connect function must implement the full startup + authentication handshake.
**When to use:** Inside `snow_pg_connect()`.
**Flow:**
```
Client                          Server
  |                               |
  |--- StartupMessage ----------->|  (version 3.0, user, database)
  |                               |
  |<-- AuthenticationXXX ---------|  (Ok, MD5Password, or SASL)
  |                               |
  |    [if MD5]:                  |
  |--- PasswordMessage ---------->|  (md5(md5(pass+user)+salt))
  |<-- AuthenticationOk ----------|
  |                               |
  |    [if SCRAM-SHA-256]:        |
  |--- SASLInitialResponse ------>|  (client-first-message)
  |<-- AuthSASLContinue ----------|  (server-first-message)
  |--- SASLResponse ------------->|  (client-final-message)
  |<-- AuthSASLFinal -------------|  (server-final: verification)
  |<-- AuthenticationOk ----------|
  |                               |
  |<-- ParameterStatus * N -------|  (server_version, encoding, etc.)
  |<-- BackendKeyData ------------|  (pid, secret_key)
  |<-- ReadyForQuery -------------|  (transaction status 'I')
  |                               |
  [Connection ready for queries]
```

### Pattern 3: Extended Query for Parameterized Queries
**What:** Use the Extended Query sub-protocol (Parse/Bind/Execute/Sync) for parameterized queries.
**When to use:** Inside `snow_pg_query()` and `snow_pg_execute()`.
**Why not Simple Query:** Simple Query sends raw SQL text with no parameter binding, which is vulnerable to SQL injection. Extended Query uses `$1, $2` placeholders with separate parameter values.
**Flow:**
```
Client                          Server
  |                               |
  |--- Parse ------------------->|  (SQL with $1, $2; unnamed stmt)
  |--- Bind -------------------->|  (param values as text; unnamed portal)
  |--- Describe (Portal) ------->|  (optional, needed for query to get column info)
  |--- Execute ----------------->|  (unnamed portal, 0 = all rows)
  |--- Sync -------------------->|  (close implicit transaction)
  |                               |
  |<-- ParseComplete ------------|
  |<-- BindComplete -------------|
  |<-- RowDescription -----------|  (column names, types)  [query only]
  |<-- DataRow * N --------------|  (row data)              [query only]
  |<-- CommandComplete ----------|  (e.g., "SELECT 5" or "INSERT 0 1")
  |<-- ReadyForQuery ------------|  (status 'I')
```

### Pattern 4: PgConn Struct Holds TCP Stream
**What:** Unlike SqliteConn (which holds a raw C pointer), PgConn holds a `TcpStream`.
**Implementation:**
```rust
use std::net::TcpStream;

struct PgConn {
    stream: TcpStream,
    // Optionally cache pid + secret_key from BackendKeyData for cancel requests
}
```
The handle pattern is identical to SQLite: `Box::into_raw(Box::new(PgConn { ... })) as u64` on connect, `Box::from_raw(handle as *mut PgConn)` on close.

### Anti-Patterns to Avoid
- **Using Simple Query protocol for parameterized queries:** Simple Query sends raw SQL text. Use Extended Query (Parse/Bind/Execute/Sync) for `$1, $2` parameter binding.
- **Forgetting to read all server messages after Sync:** After sending Sync, the server sends ParseComplete, BindComplete, possibly RowDescription, DataRow(s), CommandComplete, and finally ReadyForQuery. Must read ALL messages until ReadyForQuery to keep the protocol in sync.
- **Not handling ErrorResponse during message stream:** The server can send ErrorResponse at any point. After an error in extended query, the server discards all messages until Sync, then sends ReadyForQuery. Must handle this gracefully.
- **Blocking indefinitely on read:** If the server closes the connection, reads will fail. Set a reasonable timeout on the TcpStream or handle connection reset errors.
- **GC-allocating PgConn:** Same as SQLite -- use `Box::into_raw()` as `u64`, never `snow_gc_alloc_actor`. The GC has no finalizers.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| SCRAM-SHA-256 crypto primitives | Custom SHA-256/HMAC/PBKDF2 | `sha2`, `hmac`, `pbkdf2` crates | Cryptographic code must be correct. These are audited, standard RustCrypto crates. |
| MD5 hashing | Custom MD5 implementation | `md-5` crate | Same reason -- use audited crypto. |
| Base64 encode/decode | Custom base64 | `base64` crate | SCRAM messages use base64. The `base64` crate handles all the edge cases. |
| Random nonce generation | Custom randomness | `rand` crate | Cryptographic randomness is critical for SCRAM security. |
| SnowResult/SnowString/SnowList/SnowMap allocation | Reimplementing these types | Existing `crate::io::alloc_result`, `crate::string::snow_string_new`, `crate::collections::list::*`, `crate::collections::map::*` | These are the existing Snow runtime types already used by SQLite. Reuse them exactly. |

**Key insight:** The wire protocol itself IS appropriate to hand-roll (~500 lines for the 15 message types needed). The crypto primitives are NOT -- use crates. The Snow runtime types are already built -- reuse them.

## Common Pitfalls

### Pitfall 1: Protocol Desync After Error
**What goes wrong:** After a server ErrorResponse during extended query, the client sends another query without waiting for ReadyForQuery. The server is still waiting for Sync, leading to garbled communication.
**Why it happens:** ErrorResponse can arrive instead of ParseComplete, BindComplete, or DataRow. After error, the server discards remaining messages until it receives Sync, then sends ErrorResponse + ReadyForQuery.
**How to avoid:** Always send Parse+Bind+Execute+Sync as a batch (pipeline). After sending Sync, read messages in a loop until ReadyForQuery ('Z'). Handle ErrorResponse by collecting the error message and continuing to read until ReadyForQuery.
**Warning signs:** "unexpected message type" errors on second query, connection hangs.

### Pitfall 2: SCRAM-SHA-256 Nonce Handling
**What goes wrong:** The client nonce must be included in the server nonce. If the server returns a nonce that doesn't start with the client nonce, the server is not authentic (potential MITM).
**Why it happens:** SCRAM protocol requires the server nonce to be `client_nonce + server_extension`. Skipping this check allows a malicious server to impersonate.
**How to avoid:** After receiving server-first-message, verify that the server nonce starts with the client nonce. Abort authentication if not.
**Warning signs:** Authentication succeeds with a malicious server.

### Pitfall 3: CommandComplete Tag Parsing for Row Count
**What goes wrong:** `Pg.execute()` must return the number of rows affected. The CommandComplete message contains a tag string like `"INSERT 0 5"` (5 rows), `"UPDATE 3"`, `"DELETE 1"`, `"SELECT 10"`. The row count is the LAST number in the tag string.
**Why it happens:** Different commands have different tag formats. INSERT has an OID field before the count.
**How to avoid:** Split the tag string by whitespace and parse the last element as an integer. Works for INSERT, UPDATE, DELETE, SELECT, CREATE, DROP, etc.
**Warning signs:** Wrong row count returned, parse errors on INSERT commands.

### Pitfall 4: URL Parsing for Connection String
**What goes wrong:** `Pg.connect("postgres://user:pass@host:5432/dbname")` must be parsed into host, port, username, password, database. Edge cases: password with special characters (`@`, `/`), IPv6 hosts, default port (5432), missing database name.
**Why it happens:** URL parsing is deceptively complex.
**How to avoid:** Use Rust's `url::Url` parser or implement a simple hand-rolled parser that handles the `postgres://` scheme. For MVP, a simple split-based parser is sufficient since the URL format is well-defined. Handle percent-encoding for passwords.
**Warning signs:** Connection failures with passwords containing `@` or `:`.

### Pitfall 5: Text vs Binary Format Confusion
**What goes wrong:** PostgreSQL supports text and binary formats for parameters and results. If the client accidentally requests binary format, the returned data will be raw bytes, not human-readable strings.
**Why it happens:** The Bind message includes format codes (0=text, 1=binary). If format codes are wrong, results come back in binary.
**How to avoid:** Always use format code 0 (text) for both parameters and results. In the Bind message, set format code count to 1 and the single code to 0 (text). In the result format, similarly use 0 (text). This matches the Snow API where all values are `String`.
**Warning signs:** Garbled binary data in query results.

### Pitfall 6: Linker Changes May Not Be Needed
**What goes wrong:** Unlike SQLite (which needed `libsqlite3-sys` bundled), PostgreSQL uses only pure Rust crates. The crypto crates compile to Rust code, which gets linked into `libsnow_rt.a`.
**Why it happens:** No C dependencies means no extra linker flags.
**How to avoid:** Verify that `cargo build -p snow-rt` succeeds and the resulting `libsnow_rt.a` contains all PG symbols. The existing linker setup in `link.rs` should work without changes.
**Warning signs:** Undefined symbol errors for `snow_pg_*` during linking (would indicate the functions aren't being compiled into the static lib).

### Pitfall 7: Not Naming the Function `connect` (Not `open`)
**What goes wrong:** SQLite uses `Sqlite.open()` but the requirements specify `Pg.connect()` (matching PostgreSQL's network-oriented semantics). If you name it `open` by accident, the API won't match requirements.
**Why it happens:** Copy-pasting from SQLite pattern.
**How to avoid:** The function is `Pg.connect(url)` -> maps to `pg_connect` -> maps to `snow_pg_connect`. Not `open`.
**Warning signs:** Type checker errors when testing with `Pg.connect(...)`.

## Code Examples

Verified patterns from the existing Snow codebase and PostgreSQL protocol specification:

### PostgreSQL Wire Protocol Message Encoding (from official spec)
```rust
// Source: https://www.postgresql.org/docs/current/protocol-message-formats.html

/// Write a StartupMessage to a buffer.
/// Format: Int32(length) Int32(196608=v3.0) String("user") String(username) String("database") String(dbname) Byte1(0)
fn write_startup_message(buf: &mut Vec<u8>, user: &str, database: &str) {
    let mut body = Vec::new();
    // Protocol version 3.0 = 196608 = 0x00030000
    body.extend_from_slice(&196608_i32.to_be_bytes());
    // "user" parameter
    body.extend_from_slice(b"user\0");
    body.extend_from_slice(user.as_bytes());
    body.push(0);
    // "database" parameter
    body.extend_from_slice(b"database\0");
    body.extend_from_slice(database.as_bytes());
    body.push(0);
    // Terminator
    body.push(0);

    // Length includes itself (4 bytes)
    let len = (body.len() + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&body);
}

/// Write a Parse message: Byte1('P') Int32(len) String(stmt_name) String(query) Int16(0)
fn write_parse(buf: &mut Vec<u8>, query: &str) {
    let mut body = Vec::new();
    body.push(0); // unnamed statement (empty string = single null byte)
    body.extend_from_slice(query.as_bytes());
    body.push(0); // null-terminate query
    body.extend_from_slice(&0_i16.to_be_bytes()); // 0 parameter types

    buf.push(b'P');
    let len = (body.len() + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&body);
}

/// Write a Bind message with text parameters.
fn write_bind(buf: &mut Vec<u8>, params: &[&str]) {
    let mut body = Vec::new();
    body.push(0); // unnamed portal
    body.push(0); // unnamed statement

    // Parameter format codes: 1 code, value 0 (text for all)
    body.extend_from_slice(&1_i16.to_be_bytes());
    body.extend_from_slice(&0_i16.to_be_bytes()); // text format

    // Number of parameters
    body.extend_from_slice(&(params.len() as i16).to_be_bytes());
    for param in params {
        let bytes = param.as_bytes();
        body.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
        body.extend_from_slice(bytes);
    }

    // Result format codes: 1 code, value 0 (text for all)
    body.extend_from_slice(&1_i16.to_be_bytes());
    body.extend_from_slice(&0_i16.to_be_bytes()); // text format

    buf.push(b'B');
    let len = (body.len() + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&body);
}

/// Write Execute message: Byte1('E') Int32(len) String("") Int32(0)
fn write_execute(buf: &mut Vec<u8>) {
    buf.push(b'E');
    let body_len = 1 + 4; // empty string (1 null byte) + max_rows (4 bytes)
    let len = (body_len + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.push(0); // unnamed portal
    buf.extend_from_slice(&0_i32.to_be_bytes()); // 0 = no limit
}

/// Write Sync message: Byte1('S') Int32(4)
fn write_sync(buf: &mut Vec<u8>) {
    buf.push(b'S');
    buf.extend_from_slice(&4_i32.to_be_bytes());
}

/// Write Describe (Portal) message: Byte1('D') Int32(len) Byte1('P') String("")
fn write_describe_portal(buf: &mut Vec<u8>) {
    buf.push(b'D');
    let len = (4 + 1 + 1) as i32; // length + 'P' + null byte
    buf.extend_from_slice(&len.to_be_bytes());
    buf.push(b'P'); // Portal variant
    buf.push(0);    // unnamed portal
}
```

### MD5 Authentication (from PostgreSQL docs)
```rust
// Source: https://www.postgresql.org/docs/current/auth-password.html
// Formula: "md5" + md5(md5(password + username) + salt)
use md5::{Md5, Digest};

fn compute_md5_password(user: &str, password: &str, salt: &[u8; 4]) -> String {
    // Step 1: md5(password + username)
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    hasher.update(user.as_bytes());
    let inner = format!("{:x}", hasher.finalize());

    // Step 2: md5(inner_hex + salt)
    let mut hasher = Md5::new();
    hasher.update(inner.as_bytes());
    hasher.update(salt);
    let outer = format!("{:x}", hasher.finalize());

    format!("md5{}", outer)
}
```

### SCRAM-SHA-256 Client Flow (from RFC 5802 + PostgreSQL docs)
```rust
// Source: RFC 5802 + https://www.postgresql.org/docs/current/sasl-authentication.html
// This is the client-side SCRAM-SHA-256 implementation.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use pbkdf2::pbkdf2_hmac;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rand::Rng;

type HmacSha256 = Hmac<Sha256>;

fn scram_client_first(username: &str) -> (String, String) {
    // Generate random nonce
    let nonce: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(24)
        .map(char::from)
        .collect();

    let bare = format!("n={},r={}", username, nonce);
    let message = format!("n,,{}", bare);
    (message, nonce)
}

fn scram_client_final(
    password: &str,
    client_nonce: &str,
    server_first: &str,
) -> Result<(String, Vec<u8>), String> {
    // Parse server-first-message: r=<nonce>,s=<salt>,i=<iterations>
    let mut server_nonce = "";
    let mut salt_b64 = "";
    let mut iterations = 0u32;
    for part in server_first.split(',') {
        if let Some(v) = part.strip_prefix("r=") { server_nonce = v; }
        if let Some(v) = part.strip_prefix("s=") { salt_b64 = v; }
        if let Some(v) = part.strip_prefix("i=") { iterations = v.parse().map_err(|_| "bad iteration count")?; }
    }

    // Verify server nonce starts with client nonce
    if !server_nonce.starts_with(client_nonce) {
        return Err("server nonce mismatch".into());
    }

    let salt = BASE64.decode(salt_b64).map_err(|_| "bad salt")?;

    // SaltedPassword = PBKDF2(password, salt, iterations, SHA-256)
    let mut salted_password = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations, &mut salted_password);

    // ClientKey = HMAC(SaltedPassword, "Client Key")
    let client_key = hmac_sha256(&salted_password, b"Client Key");
    // StoredKey = SHA-256(ClientKey)
    let stored_key = sha256(&client_key);

    // AuthMessage = client-first-bare + "," + server-first + "," + client-final-without-proof
    let client_final_without_proof = format!("c=biws,r={}", server_nonce);
    // "biws" = base64("n,,") for no channel binding
    let client_first_bare = format!("n=,r={}", client_nonce);
    // Note: PostgreSQL ignores the username in client-first-bare
    let auth_message = format!("{},{},{}", client_first_bare, server_first, client_final_without_proof);

    // ClientSignature = HMAC(StoredKey, AuthMessage)
    let client_signature = hmac_sha256(&stored_key, auth_message.as_bytes());
    // ClientProof = ClientKey XOR ClientSignature
    let proof: Vec<u8> = client_key.iter().zip(client_signature.iter()).map(|(a, b)| a ^ b).collect();

    // ServerKey = HMAC(SaltedPassword, "Server Key")
    let server_key = hmac_sha256(&salted_password, b"Server Key");
    // ServerSignature = HMAC(ServerKey, AuthMessage)
    let server_signature = hmac_sha256(&server_key, auth_message.as_bytes());

    let client_final = format!("{},p={}", client_final_without_proof, BASE64.encode(&proof));
    Ok((client_final, server_signature))
}
```

### Expected Snow User Code
```snow
fn run_db() -> Int!String do
  # PG-01: Connect with URL
  let conn = Pg.connect("postgres://user:pass@localhost:5432/mydb")?

  # PG-04: Execute DDL
  let _ = Pg.execute(conn, "CREATE TABLE IF NOT EXISTS users (id SERIAL PRIMARY KEY, name TEXT NOT NULL)", [])?

  # PG-04 + PG-05: Insert with $1, $2 parameters
  let inserted = Pg.execute(conn, "INSERT INTO users (name) VALUES ($1)", ["Alice"])?
  println("Inserted: ${inserted}")

  # PG-03 + PG-05: Query with parameters
  let rows = Pg.query(conn, "SELECT id, name FROM users WHERE name = $1", ["Alice"])?
  List.map(rows, fn(row) do
    let id = Map.get(row, "id")
    let name = Map.get(row, "name")
    println(id <> ": " <> name)
  end)

  # PG-02: Close connection
  Pg.close(conn)

  Ok(0)
end

fn main() do
  case run_db() do
    Ok(_) -> println("done")
    Err(msg) -> println("error: " <> msg)
  end
end
```

### Runtime Function Structure (from SQLite pattern)
```rust
// Source: crates/snow-rt/src/db/sqlite.rs (Phase 53 established pattern)

use std::net::TcpStream;
use std::io::{Read, Write};
use crate::collections::list::{snow_list_append, snow_list_new};
use crate::collections::map::{snow_map_new_typed, snow_map_put};
use crate::io::alloc_result;
use crate::string::{snow_string_new, SnowString};

struct PgConn {
    stream: TcpStream,
}

#[no_mangle]
pub extern "C" fn snow_pg_connect(url: *const SnowString) -> *mut u8 {
    // 1. Parse URL: postgres://user:pass@host:port/database
    // 2. TcpStream::connect(host:port)
    // 3. Send StartupMessage(user, database)
    // 4. Read auth challenge (MD5 or SCRAM-SHA-256)
    // 5. Complete authentication handshake
    // 6. Read ParameterStatus + BackendKeyData + ReadyForQuery
    // 7. Box::into_raw(Box::new(PgConn { stream })) as u64
    // 8. Return SnowResult { tag: 0, value: handle }
}

#[no_mangle]
pub extern "C" fn snow_pg_close(conn_handle: u64) {
    // 1. Box::from_raw(conn_handle as *mut PgConn)
    // 2. Send Terminate message: Byte1('X') Int32(4)
    // 3. Drop closes TcpStream automatically
}

#[no_mangle]
pub extern "C" fn snow_pg_execute(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
) -> *mut u8 {
    // 1. Recover PgConn from handle
    // 2. Send Parse + Bind + Execute + Sync (pipelined)
    // 3. Read ParseComplete, BindComplete, CommandComplete, ReadyForQuery
    // 4. Parse CommandComplete tag for row count
    // 5. Return SnowResult { tag: 0, value: rows_affected as i64 }
}

#[no_mangle]
pub extern "C" fn snow_pg_query(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
) -> *mut u8 {
    // 1. Recover PgConn from handle
    // 2. Send Parse + Bind + Describe(Portal) + Execute + Sync
    // 3. Read ParseComplete, BindComplete
    // 4. Read RowDescription for column names
    // 5. Read DataRow messages, build Map<String, String> per row
    // 6. Read CommandComplete, ReadyForQuery
    // 7. Return SnowResult { tag: 0, value: list_of_maps }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `libpq` C library binding | Pure Rust wire protocol (`rust-postgres`, `tokio-postgres`) | Established since rust-postgres 0.1 (2015) | No C dependency needed; pure Rust PG clients are the standard in the Rust ecosystem |
| MD5-only auth | SCRAM-SHA-256 as default (PostgreSQL 14+) | PostgreSQL 10 added SCRAM (2017), PG 14 made it default (2021) | Cloud providers (AWS RDS, Azure, GCP CloudSQL) all use SCRAM-SHA-256 by default. MD5 is deprecated but still needed for local dev. |
| Simple Query protocol | Extended Query with parameterized queries | Always available in v3 protocol | Extended Query prevents SQL injection and enables prepared statement reuse. |
| Password auth in clear text | SCRAM-SHA-256 with channel binding | PostgreSQL 10+ | SCRAM never sends the password over the wire, even encrypted. Uses challenge-response with PBKDF2-derived keys. |

**Deprecated/outdated:**
- MD5 authentication: Deprecated in PostgreSQL, will be removed in a future release. But still needed for PG-08 (local development servers often default to MD5).
- Simple Query protocol for parameterized queries: Not suitable because it doesn't support parameter binding.

## Open Questions

1. **URL parsing approach**
   - What we know: The connection URL format is `postgres://user:pass@host:port/database`. Rust's `url` crate can parse this, but adds a dependency.
   - What's unclear: Whether to add the `url` crate or hand-roll a simple parser.
   - Recommendation: Hand-roll a simple parser. The `postgres://` URL format is well-defined and simple enough (split on `://`, `@`, `:`, `/`). Handle percent-decoding for passwords with special characters using a small helper. This avoids adding another dependency for a ~30-line function.

2. **Connection timeout**
   - What we know: `TcpStream::connect()` can block indefinitely if the server is unreachable.
   - What's unclear: What timeout value to use.
   - Recommendation: Use `TcpStream::connect_timeout()` with a 10-second default. This prevents Snow programs from hanging forever on a bad connection URL.

3. **Testing strategy**
   - What we know: E2E tests require a running PostgreSQL server. CI may not have one.
   - What's unclear: How to handle test infrastructure.
   - Recommendation: Write Rust unit tests that mock the wire protocol (test message encoding/decoding without a real server). Write one E2E `.snow` test that assumes a local PostgreSQL at `localhost:5432` with a test user. The E2E test can be gated by an environment variable or simply documented as requiring a running PostgreSQL.

4. **Parameter type specification in Parse message**
   - What we know: The Parse message allows specifying parameter types by OID (e.g., OID 25 = text, OID 23 = int4). Specifying 0 means "let the server infer."
   - What's unclear: Whether to specify types or let the server infer.
   - Recommendation: Specify 0 for all parameter types (let server infer). Since all parameters are passed as text format in Bind, the server will do text-to-type coercion based on the query context. This is the simplest approach and works for MVP.

5. **Describe message for execute vs query**
   - What we know: The Describe (Portal) message returns RowDescription, which tells us column names. For `execute()` (INSERT/UPDATE/DELETE), there are no result columns.
   - What's unclear: Whether to send Describe for execute too.
   - Recommendation: Only send Describe(Portal) for `query()`, not for `execute()`. For `execute()`, the CommandComplete tag provides the row count. For `query()`, RowDescription provides column names needed to build the Map keys.

## Sources

### Primary (HIGH confidence)
- **Snow codebase** -- Direct reading of:
  - `crates/snow-rt/src/db/sqlite.rs` (complete SQLite runtime implementation, exact pattern to follow)
  - `crates/snow-rt/src/db/mod.rs` (module organization)
  - `crates/snow-rt/src/io.rs` (SnowResult struct, alloc_result helper)
  - `crates/snow-rt/src/string.rs` (SnowString type, snow_string_new)
  - `crates/snow-rt/src/lib.rs` (re-exports pattern)
  - `crates/snow-rt/Cargo.toml` (current dependencies)
  - `crates/snow-codegen/src/codegen/intrinsics.rs` (LLVM function declarations, SQLite pattern)
  - `crates/snow-codegen/src/mir/lower.rs` (known_functions, map_builtin_name, STDLIB_MODULES)
  - `crates/snow-codegen/src/mir/types.rs` (SqliteConn -> MirType::Int pattern)
  - `crates/snow-typeck/src/infer.rs` (stdlib_modules(), STDLIB_MODULE_NAMES)
  - `crates/snow-typeck/src/builtins.rs` (SqliteConn registration pattern)
  - `crates/snow-codegen/src/link.rs` (linker setup -- no changes expected)
  - `tests/e2e/stdlib_sqlite.snow` (E2E test pattern)
- **PostgreSQL Official Documentation** (v18):
  - [Chapter 54: Frontend/Backend Protocol](https://www.postgresql.org/docs/current/protocol.html) -- Protocol overview
  - [54.2: Message Flow](https://www.postgresql.org/docs/current/protocol-flow.html) -- Startup, auth, query flows
  - [54.7: Message Formats](https://www.postgresql.org/docs/current/protocol-message-formats.html) -- Byte-level message definitions
  - [20.5: Password Authentication](https://www.postgresql.org/docs/current/auth-password.html) -- MD5 and SCRAM-SHA-256 details
  - [SASL Authentication](https://www.postgresql.org/docs/current/sasl-authentication.html) -- SCRAM protocol flow

### Secondary (MEDIUM confidence)
- [rust-postgres repository](https://github.com/sfackler/rust-postgres) -- Reference implementation of pure-Rust PG client; confirmed `postgres-protocol` is tokio-free
- [postgres-protocol crate](https://crates.io/crates/postgres-protocol) -- v0.6.10, confirmed dependencies: sha2, hmac, md-5, pbkdf2, base64, rand, bytes, byteorder, memchr, stringprep, fallible-iterator
- [RFC 5802: SCRAM](https://tools.ietf.org/html/rfc5802) -- SCRAM protocol specification
- [RFC 7677: SCRAM-SHA-256](https://tools.ietf.org/html/rfc7677) -- SCRAM-SHA-256 specifics

### Tertiary (LOW confidence)
- None -- all critical claims verified with official PostgreSQL docs or the Snow codebase.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- Crypto crates (`sha2`, `hmac`, `md-5`, `pbkdf2`) are the standard RustCrypto crates, same ones used by `postgres-protocol` itself. Dependencies are minimal and well-understood.
- Architecture: HIGH -- The compiler pipeline pattern is a direct copy of the proven SQLite pattern (Phase 53). File locations, function signatures, and registration steps are verified against the current codebase. The wire protocol message formats are from the official PostgreSQL documentation.
- Pitfalls: HIGH -- Protocol desync, SCRAM nonce verification, and CommandComplete parsing are well-documented issues in PostgreSQL client implementations. The GC safety pitfall is established from Phase 53.

**Research date:** 2026-02-12
**Valid until:** 2026-03-12 (stable domain -- PostgreSQL wire protocol v3 is stable since 2003, Snow compiler patterns are established)
