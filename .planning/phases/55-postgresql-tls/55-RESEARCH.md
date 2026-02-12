# Phase 55: PostgreSQL TLS - Research

**Researched:** 2026-02-12
**Domain:** PostgreSQL SSLRequest protocol upgrade, rustls TLS library, PgStream abstraction
**Confidence:** HIGH

## Summary

Phase 55 adds TLS support to the existing PostgreSQL wire protocol driver so Snow programs can connect to cloud databases (AWS RDS, Supabase, Neon) that require encrypted connections. The implementation involves three coordinated changes: (1) refactoring `PgConn` to use a `PgStream` enum (`Plain(TcpStream)` / `Tls(StreamOwned<ClientConnection, TcpStream>)`) with `Read+Write` delegation, (2) inserting the PostgreSQL SSLRequest handshake into the connection flow before the StartupMessage, and (3) installing the `ring` crypto provider at runtime startup for process-wide TLS compatibility.

The key insight is that **all TLS dependencies are already compiled** as transitive dependencies of `ureq 2.12`: `rustls 0.23.36`, `ring 0.17.14`, `webpki-roots 0.26.11` (re-exporting `1.0.6`), and `rustls-pki-types 1.14.0`. Adding these as direct dependencies in `snow-rt/Cargo.toml` costs zero additional compile time because Cargo deduplicates them. The only new code is ~150-200 lines in `pg.rs`: the PgStream enum, Read+Write impl, SSLRequest flow, and sslmode URL parameter parsing.

No compiler changes are needed. The `Pg.connect`, `Pg.close`, `Pg.execute`, `Pg.query` API surface is unchanged. The `sslmode` parameter is parsed from the connection URL query string (`?sslmode=require`). The `PgStream` enum is an internal detail -- all existing wire protocol functions (`read_message`, `write_*`) are refactored to accept `&mut PgStream` (or `&mut dyn Read`/`&mut dyn Write`) instead of `&mut TcpStream`.

**Primary recommendation:** Refactor PgConn.stream to use a PgStream enum, add SSLRequest negotiation before StartupMessage, install ring crypto provider in snow_rt_init, and parse sslmode from URL query parameters. No new crate API, no compiler changes.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `rustls` | 0.23.36 | TLS 1.2/1.3 client implementation | Pure Rust, no OpenSSL dependency. Already compiled as transitive dep of ureq 2.12. The `ring` feature is already enabled by ureq. |
| `ring` | 0.17.14 | Cryptographic provider for rustls | Already compiled as transitive dep. Easier cross-platform builds than aws-lc-rs (no CMake). |
| `webpki-roots` | 0.26.11 / 1.0.6 | Mozilla CA certificate bundle | Already compiled as transitive dep. Provides `TLS_SERVER_ROOTS` for building `RootCertStore`. |
| `rustls-pki-types` | 1.14.0 | Certificate and server name types | Already compiled as transitive dep. Provides `ServerName` type needed by `ClientConnection::new`. |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| (none) | - | - | All supporting deps are already present |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `rustls` + `ring` | `native-tls` (OpenSSL wrapper) | native-tls requires system OpenSSL installation, platform-specific behavior, C dependency. rustls is pure Rust and already compiled. |
| `webpki-roots` (compiled-in CA bundle) | `rustls-native-certs` (system CA store) | rustls-native-certs reads OS certificate store at runtime (macOS Keychain, Linux /etc/ssl). More flexible but adds platform-specific code and new dependency. webpki-roots is simpler, already compiled, sufficient for cloud databases. |
| `ring` | `aws-lc-rs` (rustls default) | aws-lc-rs requires CMake + C compiler for assembly code. ring is already enabled by ureq's rustls dependency. No build change needed. |

**Cargo.toml additions to `snow-rt`:**
```toml
# Phase 55: TLS for PostgreSQL connections
# All three crates are already compiled as transitive deps of ureq 2.12.
# Adding them as direct deps costs zero additional compile time.
rustls = { version = "0.23", default-features = false, features = ["ring", "tls12", "logging", "std"] }
webpki-roots = "0.26"
rustls-pki-types = "1"
```

Note: `default-features = false` prevents pulling in `aws-lc-rs`. The `ring` feature enables the `ring`-based crypto provider. The `tls12` feature ensures compatibility with PostgreSQL servers that only support TLS 1.2. These exact feature flags match what ureq already enables, so Cargo resolves to the same compiled crate.

## Architecture Patterns

### Recommended Changes (All in snow-rt)
```
crates/snow-rt/src/
  db/
    pg.rs         # MODIFY: PgStream enum, SSLRequest flow, sslmode parsing, refactor read_message
  gc.rs           # MODIFY: Install ring CryptoProvider in snow_rt_init()
```

No changes to: `snow-codegen`, `snow-typeck`, `snow-parser`, `tests/`. The compiler pipeline is unchanged. No new intrinsics, no new type signatures, no new module functions.

### Pattern 1: PgStream Enum with Read+Write Delegation
**What:** Replace `TcpStream` in PgConn with an enum that supports both plain and TLS streams.
**When to use:** This is the core abstraction for TLS-02.
**Example:**
```rust
// Source: rustls docs (StreamOwned) + PostgreSQL driver pattern
use rustls::{ClientConnection, StreamOwned};
use std::io::{Read, Write, Result as IoResult};
use std::net::TcpStream;

enum PgStream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}

impl Read for PgStream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self {
            PgStream::Plain(s) => s.read(buf),
            PgStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for PgStream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        match self {
            PgStream::Plain(s) => s.write(buf),
            PgStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> IoResult<()> {
        match self {
            PgStream::Plain(s) => s.flush(),
            PgStream::Tls(s) => s.flush(),
        }
    }
}

struct PgConn {
    stream: PgStream,  // was: stream: TcpStream
}
```

### Pattern 2: SSLRequest Handshake Before StartupMessage
**What:** Send the 8-byte SSLRequest message on the raw TcpStream, read 1-byte response, then perform TLS handshake if server accepts.
**When to use:** Inside `snow_pg_connect`, between TCP connect and StartupMessage.
**Example:**
```rust
// Source: PostgreSQL docs, protocol-message-formats.html (SSLRequest)
// SSLRequest: Int32(8) Int32(80877103) -- no message type byte
fn write_ssl_request(stream: &mut TcpStream) -> Result<(), String> {
    let mut buf = Vec::with_capacity(8);
    buf.extend_from_slice(&8_i32.to_be_bytes());       // length = 8
    buf.extend_from_slice(&80877103_i32.to_be_bytes()); // SSL request code
    stream.write_all(&buf).map_err(|e| format!("send SSLRequest: {}", e))
}

fn read_ssl_response(stream: &mut TcpStream) -> Result<u8, String> {
    let mut byte = [0u8; 1];
    stream.read_exact(&mut byte).map_err(|e| format!("read SSL response: {}", e))?;
    Ok(byte[0])
}
```

### Pattern 3: rustls ClientConfig + StreamOwned Construction
**What:** Build a rustls ClientConfig and wrap the TcpStream for TLS.
**When to use:** After receiving 'S' from SSLRequest.
**Example:**
```rust
// Source: rustls docs (ClientConfig builder, StreamOwned)
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::ServerName;
use std::sync::Arc;

fn upgrade_to_tls(
    stream: TcpStream,
    hostname: &str,
) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
    // Build root cert store from Mozilla CA bundle
    let root_store = RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned()
    );

    // Build client config with certificate verification
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    // Create connection with server name for SNI
    let server_name = ServerName::try_from(hostname.to_string())
        .map_err(|_| format!("invalid server name: {}", hostname))?;
    let conn = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("TLS client connection: {}", e))?;

    // Wrap into StreamOwned -- owns both conn and socket
    Ok(StreamOwned::new(conn, stream))
}
```

### Pattern 4: sslmode URL Query Parameter Parsing
**What:** Parse `?sslmode=require|prefer|disable` from the connection URL.
**When to use:** Extend existing `parse_pg_url` function.
**Example:**
```rust
// Extend PgUrl struct
struct PgUrl {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
    sslmode: SslMode,  // NEW
}

#[derive(Clone, Copy, PartialEq)]
enum SslMode {
    Disable,  // No TLS negotiation
    Prefer,   // Try TLS, fall back to plain (default)
    Require,  // TLS required, error if server declines
}

// Parse from URL: postgres://user:pass@host/db?sslmode=require
// Split on '?' to get query string, then split on '&' to get params
fn parse_sslmode(query_str: &str) -> SslMode {
    for param in query_str.split('&') {
        if let Some(value) = param.strip_prefix("sslmode=") {
            return match value {
                "disable" => SslMode::Disable,
                "require" => SslMode::Require,
                "prefer" => SslMode::Prefer,
                _ => SslMode::Prefer,
            };
        }
    }
    SslMode::Prefer  // default
}
```

### Pattern 5: Ring CryptoProvider Installation at Startup
**What:** Install the `ring` crypto provider as the process-wide default in `snow_rt_init`.
**When to use:** Must be called before any TLS operation (PG connect or ureq HTTP request).
**Example:**
```rust
// Source: rustls docs (CryptoProvider::install_default)
// In gc.rs snow_rt_init():
pub extern "C" fn snow_rt_init() {
    // ... existing arena init ...

    // Install ring crypto provider for TLS (idempotent: ignore if already installed)
    let _ = rustls::crypto::ring::default_provider().install_default();
}
```

### Pattern 6: Refactored Connection Flow
**What:** The complete two-phase connection: (1) TCP + optional TLS, (2) PG protocol.
**When to use:** Replaces the current `snow_pg_connect` flow.
**Flow:**
```
snow_pg_connect(url)
  |-- parse_pg_url(url) -> PgUrl { ..., sslmode }
  |-- TcpStream::connect_timeout(addr, 10s)
  |
  |-- IF sslmode == Disable:
  |     |-- stream = PgStream::Plain(tcp_stream)
  |
  |-- IF sslmode == Require:
  |     |-- write_ssl_request(&mut tcp_stream)
  |     |-- response = read_ssl_response(&mut tcp_stream)
  |     |-- IF response == 'S':
  |     |     |-- tls = upgrade_to_tls(tcp_stream, hostname)
  |     |     |-- stream = PgStream::Tls(tls)
  |     |-- IF response == 'N':
  |     |     |-- return Err("server does not support SSL")
  |     |-- IF response == 'E':
  |     |     |-- return Err("server rejected SSLRequest")
  |
  |-- IF sslmode == Prefer:
  |     |-- write_ssl_request(&mut tcp_stream)
  |     |-- response = read_ssl_response(&mut tcp_stream)
  |     |-- IF response == 'S':
  |     |     |-- tls = upgrade_to_tls(tcp_stream, hostname)
  |     |     |-- stream = PgStream::Tls(tls)
  |     |-- IF response == 'N':
  |     |     |-- stream = PgStream::Plain(tcp_stream)  // fall back
  |
  |-- PgConn { stream }
  |-- Send StartupMessage (over stream, works for both Plain and Tls)
  |-- Authentication handshake (unchanged logic, now via PgStream)
  |-- Read until ReadyForQuery
  |-- Return handle
```

### Anti-Patterns to Avoid
- **Attempting TLS handshake without SSLRequest:** PostgreSQL uses in-band TLS negotiation. The server must first agree to TLS via SSLRequest/S response. Sending TLS ClientHello directly causes the server to interpret it as a malformed PostgreSQL message and disconnect.
- **Reading more than 1 byte after SSLRequest:** The server sends exactly 1 byte ('S' or 'N'). Reading more risks consuming the first bytes of the TLS handshake or the next PostgreSQL message. CVE-2021-23222 (buffer-stuffing attack) -- read exactly 1 byte before handing socket to rustls.
- **Using trait objects (`Box<dyn Read + Write>`) instead of enum:** Trait objects add heap allocation and dynamic dispatch overhead on every I/O call. The enum dispatches statically (match on 2 variants) and stores inline. The PgStream enum is the standard pattern for this exact scenario.
- **Creating a new ClientConfig per connection:** `ClientConfig` should be built once and shared via `Arc<ClientConfig>`. For simplicity in the initial implementation, building per-connection is acceptable (the cost is minimal compared to a TCP+TLS handshake), but can be optimized later in Phase 57 (connection pooling).
- **Forgetting to set read/write timeouts on TLS stream:** The current code sets timeouts on the TcpStream. When wrapped in StreamOwned, the timeouts are inherited because StreamOwned delegates to the underlying TcpStream. No additional timeout configuration is needed.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| TLS 1.2/1.3 protocol | Custom TLS implementation | `rustls 0.23` | TLS is thousands of lines of crypto. rustls is audited, maintained, and already compiled. |
| CA certificate bundle | Custom cert loading | `webpki-roots` | Mozilla's CA bundle, maintained by the rustls team. Already compiled. |
| Crypto primitives for TLS | Custom AES/ChaCha20/etc | `ring` via rustls | Ring provides the crypto backend. Already compiled. |
| Server name verification | Custom hostname matching | rustls `ServerName` | SNI and hostname verification have complex edge cases (wildcards, IDN, etc). |
| Stream Read+Write delegation | Trait objects or manual buffering | Enum with match delegation | The PgStream enum is 6 lines of Read impl + 9 lines of Write impl. Simple, zero-cost. |

**Key insight:** All TLS libraries are already compiled. The only new code is the PgStream enum (~30 lines), SSLRequest protocol (~20 lines), sslmode parsing (~20 lines), and the connection flow refactor (~50 lines of rearrangement). Total new code: ~120-150 lines.

## Common Pitfalls

### Pitfall 1: SSLRequest Must Precede TLS Handshake
**What goes wrong:** Developer wraps TcpStream in TLS directly without sending SSLRequest first. The server interprets the TLS ClientHello as a malformed PostgreSQL message and disconnects.
**Why it happens:** PostgreSQL uses in-band TLS negotiation (unlike HTTPS on port 443). The SSLRequest is a PostgreSQL protocol message, not a TLS message.
**How to avoid:** Always send the 8-byte SSLRequest (`Int32(8) Int32(80877103)`) on the raw TcpStream and read the 1-byte response ('S'/'N') before constructing the rustls ClientConnection.
**Warning signs:** "unexpected message type" errors, connection timeout after TCP connect, "invalid message length" from server.

### Pitfall 2: read_message Still Typed to TcpStream
**What goes wrong:** The current `read_message(stream: &mut TcpStream)` function signature does not accept `&mut PgStream`. All callers in `snow_pg_connect`, `snow_pg_execute`, and `snow_pg_query` would fail to compile after changing PgConn.stream to PgStream.
**Why it happens:** Phase 54 typed the function to the concrete TcpStream type.
**How to avoid:** Change `read_message` signature to accept `&mut PgStream` (or `impl Read`). Similarly, change all write helpers to accept `&mut PgStream` (or `impl Write`). Since PgStream implements Read+Write, all existing callers work with minimal changes (just pass `&mut conn.stream` where the type is now PgStream instead of TcpStream).
**Warning signs:** Compilation errors on "expected TcpStream, found PgStream".

### Pitfall 3: CryptoProvider Not Installed Before First TLS Use
**What goes wrong:** If `CryptoProvider::install_default()` is not called before `ClientConfig::builder()`, rustls panics with "no process-level CryptoProvider available."
**Why it happens:** rustls 0.23 requires an explicit crypto provider. When both `aws-lc-rs` and `ring` features are enabled (which happens because ureq enables `ring`), rustls cannot auto-detect which to use.
**How to avoid:** Call `rustls::crypto::ring::default_provider().install_default()` in `snow_rt_init()` (gc.rs). This runs before any Snow code executes. Use `let _ =` to ignore the error if already installed (idempotent pattern).
**Warning signs:** Runtime panic: "no process-level CryptoProvider available" or "could not automatically determine the process-level CryptoProvider."

### Pitfall 4: sslmode=require Does NOT Verify Certificates
**What goes wrong:** Developer assumes `sslmode=require` validates the server certificate. In libpq, `require` only ensures the connection IS encrypted -- it does not verify the server's identity. Only `verify-ca` and `verify-full` do certificate validation.
**Why it happens:** The name "require" suggests security, but it only requires encryption, not authentication.
**How to avoid:** For this phase (TLS-01 scope), `sslmode=require` uses the full webpki-roots CA verification (matching what cloud databases provide -- valid CA-signed certificates). This is actually more secure than libpq's default `require` behavior. The `verify-ca` and `verify-full` modes are deferred (TLS-08 in future requirements).
**Warning signs:** None -- using webpki-roots verification is the safe default. Self-signed cert connections will fail with `require` mode, which is the correct behavior for cloud databases.

### Pitfall 5: URL Query Parameter Parsing Edge Cases
**What goes wrong:** The existing `parse_pg_url` function splits on `@`, `/`, and `:` but does not handle `?` for query parameters. Adding query string parsing must not break existing URLs that have no `?`.
**Why it happens:** Phase 54's URL parser was designed for the simple `postgres://user:pass@host:port/database` format.
**How to avoid:** Before parsing the host/database portion, split on `?` first. Parse the part before `?` using existing logic, parse the part after `?` for sslmode and other params. Default sslmode to `Prefer` when not specified, matching libpq's default behavior and ensuring backward compatibility (existing URLs work unchanged).
**Warning signs:** Existing Phase 54 tests fail, URLs without `?sslmode=` break.

### Pitfall 6: StreamOwned Does Not Support set_read_timeout
**What goes wrong:** The current code calls `stream.set_read_timeout()` and `stream.set_write_timeout()` on the TcpStream. After wrapping in StreamOwned, these methods are not directly available on the wrapper.
**Why it happens:** StreamOwned implements Read+Write but not TcpStream-specific methods like timeout setters.
**How to avoid:** Set timeouts on the TcpStream BEFORE wrapping it in StreamOwned. StreamOwned delegates I/O to the underlying TcpStream, so timeouts set before wrapping are preserved. Access the inner stream via `stream_owned.sock` (public field) or `stream_owned.get_mut()` if needed later.
**Warning signs:** Connections hang indefinitely on read if timeout was not set before TLS wrapping.

## Code Examples

Verified patterns from official sources:

### Complete SSLRequest + TLS Upgrade Flow
```rust
// Source: PostgreSQL protocol-message-formats.html + rustls docs
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::ServerName;

/// Send SSLRequest and perform TLS handshake if server accepts.
fn negotiate_tls(
    mut stream: TcpStream,
    hostname: &str,
    sslmode: SslMode,
) -> Result<PgStream, String> {
    if sslmode == SslMode::Disable {
        return Ok(PgStream::Plain(stream));
    }

    // Send SSLRequest: Int32(8) Int32(80877103)
    let ssl_request: [u8; 8] = {
        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&8_i32.to_be_bytes());
        buf[4..8].copy_from_slice(&80877103_i32.to_be_bytes());
        buf
    };
    stream.write_all(&ssl_request)
        .map_err(|e| format!("send SSLRequest: {}", e))?;

    // Read exactly 1 byte response
    let mut response = [0u8; 1];
    stream.read_exact(&mut response)
        .map_err(|e| format!("read SSL response: {}", e))?;

    match response[0] {
        b'S' => {
            // Server accepts SSL -- perform TLS handshake
            let root_store = RootCertStore::from_iter(
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned()
            );
            let config = ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();

            let server_name = ServerName::try_from(hostname.to_string())
                .map_err(|_| format!("invalid hostname for TLS: {}", hostname))?;
            let tls_conn = ClientConnection::new(Arc::new(config), server_name)
                .map_err(|e| format!("TLS connection: {}", e))?;

            Ok(PgStream::Tls(StreamOwned::new(tls_conn, stream)))
        }
        b'N' => {
            // Server declines SSL
            match sslmode {
                SslMode::Require => Err("server does not support SSL".to_string()),
                SslMode::Prefer => Ok(PgStream::Plain(stream)),
                SslMode::Disable => unreachable!(),
            }
        }
        other => Err(format!("unexpected SSL response: {}", other as char)),
    }
}
```

### Ring CryptoProvider Installation
```rust
// Source: rustls docs (CryptoProvider::install_default)
// Place in snow_rt_init() in gc.rs

// Install ring as the process-wide TLS crypto provider.
// Must be called before any rustls ClientConfig/ServerConfig is built.
// Ignore the error if already installed (idempotent).
let _ = rustls::crypto::ring::default_provider().install_default();
```

### URL Query String Parsing Extension
```rust
// Extend parse_pg_url to handle ?sslmode=...
fn parse_pg_url(url: &str) -> Result<PgUrl, String> {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or_else(|| "URL must start with postgres:// or postgresql://".to_string())?;

    // Split off query string
    let (rest, query_str) = if let Some((r, q)) = rest.split_once('?') {
        (r, q)
    } else {
        (rest, "")
    };

    // Parse sslmode from query string
    let sslmode = parse_sslmode(query_str);

    // ... existing URL parsing logic on `rest` ...

    Ok(PgUrl { host, port, user, password, database, sslmode })
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| OpenSSL/native-tls for Rust TLS | rustls (pure Rust) | rustls stable since 2019, dominant since 2023 | No C dependency, no OpenSSL version conflicts |
| rustls with aws-lc-rs default provider | Explicit CryptoProvider selection (ring or aws-lc-rs) | rustls 0.22 (Dec 2023) | Must call `install_default()` or select per-config |
| webpki-roots 0.26 | webpki-roots 1.0 (stable, 0.26 re-exports) | Feb 2025 | API identical, version number is the only change |
| `RootCertStore::add_server_trust_anchors` | `RootCertStore::from_iter` | rustls 0.22 | Builder pattern, not mutable store |

**Deprecated/outdated:**
- `rustls::dangerous_configuration` feature flag: No longer needed in 0.23. The `dangerous()` method is always available on ConfigBuilder.
- `webpki_roots::TLS_SERVER_ROOTS` as `&[TrustAnchor]` passed to `add_server_trust_anchors`: Removed in rustls 0.22. Now use `from_iter(TLS_SERVER_ROOTS.iter().cloned())`.
- `tiny_http`'s `ssl-rustls` feature (uses rustls 0.20): Incompatible with rustls 0.23. Being replaced in Phase 56.

## Open Questions

1. **Default sslmode value**
   - What we know: libpq defaults to `prefer`. The phase requirements say "existing v2.0 code continues to work without modification."
   - What's unclear: Should the default be `prefer` (try TLS, fall back) or `disable` (preserve exact v2.0 behavior)?
   - Recommendation: Default to `prefer`. This maintains backward compatibility (plain connections still work when server declines SSL) while automatically upgrading to TLS when the server supports it. The success criteria explicitly list `sslmode=prefer` behavior, and existing v2.0 code (no `?sslmode=` in URL) will work because `prefer` falls back to plain when the server responds 'N'.

2. **ClientConfig caching**
   - What we know: Building a `ClientConfig` involves loading CA certs from `webpki_roots`. This is not expensive (~microseconds) but happens on every connection.
   - What's unclear: Whether to cache the Arc<ClientConfig> in a global or build per-connection.
   - Recommendation: Build per-connection for Phase 55. This is simpler and correct. Phase 57 (connection pooling) can add a cached config if needed. The cost of building ClientConfig is dwarfed by TCP connect + TLS handshake.

3. **Server name for IP address connections**
   - What we know: `ServerName::try_from("127.0.0.1")` will create an IP address server name, which works for TLS but some certificate verification may differ.
   - What's unclear: How cloud databases expose their hostname (FQDN vs IP).
   - Recommendation: Pass the hostname from the URL directly to `ServerName::try_from`. Cloud databases always provide FQDN hostnames (e.g., `db.supabase.co`). Local connections with IP addresses and `sslmode=prefer` will fall back to plain TCP if the server declines SSL, which is the expected local dev behavior.

4. **ErrorResponse handling from SSLRequest**
   - What we know: The PostgreSQL docs (CVE-2024-10977) say the frontend should be prepared to handle an ErrorResponse to SSLRequest, and should NOT display it to the user since the server has not been authenticated.
   - What's unclear: Whether to silently ignore the error or return a generic error message.
   - Recommendation: If SSLRequest gets an ErrorResponse, return a generic error "SSL negotiation failed" without including the server's error message (per CVE-2024-10977 guidance). For `sslmode=prefer`, fall back to plain TCP.

## Sources

### Primary (HIGH confidence)
- **Snow codebase** (direct reading):
  - `crates/snow-rt/src/db/pg.rs` -- Complete Phase 54 PgConn implementation, 933 lines, all wire protocol functions, URL parsing, authentication
  - `crates/snow-rt/Cargo.toml` -- Current dependency list, confirms no TLS deps yet
  - `crates/snow-rt/src/gc.rs` -- snow_rt_init() location for CryptoProvider installation
  - `crates/snow-codegen/src/codegen/mod.rs` -- generate_main_wrapper confirms snow_rt_init() called at startup
  - `Cargo.lock` -- Confirmed exact versions: rustls 0.23.36, ring 0.17.14, webpki-roots 0.26.11/1.0.6, rustls-pki-types 1.14.0
  - `cargo tree -p snow-rt` -- Confirmed all TLS deps are transitive deps of ureq 2.12.1

- **PostgreSQL Official Documentation** (v18):
  - [Section 54.2.10: SSL Session Encryption](https://www.postgresql.org/docs/current/protocol-flow.html) -- SSLRequest flow, 'S'/'N' response, CVE-2021-23222 buffer-stuffing warning
  - [Section 54.7: Message Formats - SSLRequest](https://www.postgresql.org/docs/current/protocol-message-formats.html) -- SSLRequest: Int32(8) Int32(80877103), no message type byte
  - [Section 32.19: SSL Support (libpq)](https://www.postgresql.org/docs/current/libpq-ssl.html) -- sslmode parameter semantics (disable/prefer/require/verify-ca/verify-full)

- **rustls Official Documentation** (v0.23.36):
  - [StreamOwned](https://docs.rs/rustls/latest/rustls/struct.StreamOwned.html) -- Owns Connection + TcpStream, implements Read+Write+BufRead
  - [ClientConnection](https://docs.rs/rustls/latest/rustls/client/struct.ClientConnection.html) -- Created via ClientConnection::new(Arc<ClientConfig>, ServerName)
  - [CryptoProvider::install_default](https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html) -- Process-wide, must be called once before any TLS use
  - [ConfigBuilder](https://docs.rs/rustls/latest/rustls/struct.ConfigBuilder.html) -- .with_root_certificates() for safe config, .dangerous() for custom verifier

### Secondary (MEDIUM confidence)
- **Snow project research** (`.planning/research/`):
  - `STACK.md` -- Pre-existing TLS architecture analysis, confirmed ring choice over aws-lc-rs
  - `FEATURES.md` -- TLS feature analysis, SSLRequest flow description
  - `PITFALLS.md` -- Pitfall 11 (SSLRequest ordering), Pitfall 12 (aws-lc-rs build complexity)
  - `ARCHITECTURE.md` -- Connection flow diagram for TLS upgrade

- [webpki-roots crate](https://docs.rs/webpki-roots) -- TLS_SERVER_ROOTS constant, version 1.0 = 0.26 (re-export via semver trick)
- [ureq issue #751](https://github.com/algesten/ureq/issues/751) -- ureq 2.x uses ring feature of rustls, confirmed feature flag propagation

### Tertiary (LOW confidence)
- None -- all critical claims verified against official docs, codebase, or Cargo.lock.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- All deps verified in Cargo.lock as already-compiled transitive deps. Zero new crate compilation.
- Architecture: HIGH -- PgStream enum pattern verified against rustls StreamOwned docs. SSLRequest bytes verified against PostgreSQL protocol docs. Connection flow matches prior project research.
- Pitfalls: HIGH -- SSLRequest ordering (Pitfall 11 from project research), CryptoProvider installation (confirmed via rustls docs and ureq interaction), read_message refactor (verified in current pg.rs source code).

**Research date:** 2026-02-12
**Valid until:** 2026-03-12 (stable domain -- PostgreSQL SSLRequest protocol stable since v7.2 (2001), rustls 0.23 API stable)
