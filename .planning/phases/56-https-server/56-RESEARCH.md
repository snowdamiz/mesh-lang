# Phase 56: HTTPS Server - Research

**Researched:** 2026-02-12
**Domain:** HTTP/1.1 parsing, rustls ServerConnection, TLS server-side, tiny_http replacement
**Confidence:** HIGH

## Summary

Phase 56 has two tightly coupled requirements: (1) replace tiny_http with a hand-rolled HTTP/1.1 parser (TLS-05), and (2) add `Http.serve_tls(router, port, cert_path, key_path)` for HTTPS serving (TLS-04). These must be done together because tiny_http's built-in TLS uses rustls 0.20, which is incompatible with the rustls 0.23 already used by Phase 55's PostgreSQL TLS and by ureq's HTTP client.

The replacement is tractable because Snow's HTTP server uses a narrow subset of HTTP/1.1: it needs to parse the request line (method, path, HTTP version), parse headers (key-value pairs terminated by `\r\n\r\n`), read a body using Content-Length, and write a simple response. It does NOT need: chunked transfer encoding (input), HTTP/2, keep-alive connection reuse, or trailer headers. The existing `handle_request` function in `server.rs` already does all the routing, middleware, and response construction -- it currently receives a `tiny_http::Request` and calls `request.respond()`. The replacement only needs to produce the same data (method string, URL string, headers map, body bytes) and write a response back.

The HTTPS layer mirrors Phase 55's client-side pattern: `StreamOwned<ServerConnection, TcpStream>` wraps the TCP socket just as `StreamOwned<ClientConnection, TcpStream>` does for PostgreSQL. The `ServerConfig` is built once with the user's cert/key pair and shared via `Arc`. Each incoming TLS connection creates a new `ServerConnection` and wraps it in `StreamOwned`. The connection handler actor then reads/writes through the `StreamOwned` stream, which implements `Read + Write`, making it transparent to the HTTP parser.

**Primary recommendation:** Split into two plans: (1) Replace tiny_http with a hand-rolled HTTP/1.1 request parser and response writer, keeping `Http.serve` working identically on plaintext TCP; (2) Add `Http.serve_tls` using rustls `ServerConfig` + `ServerConnection` + `StreamOwned`, reusing the same parser over TLS streams. Use a `HttpStream` enum (mirroring Phase 55's `PgStream`) for zero-cost dispatch between plain and TLS connections.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `rustls` | 0.23.36 | TLS 1.2/1.3 server implementation | Already a direct dependency (Phase 55). `ServerConfig::builder().with_no_client_auth().with_single_cert()` for server-side TLS. |
| `rustls-pki-types` | 1.14.0 | PEM certificate/key loading | Already a direct dependency (Phase 55). `CertificateDer::pem_file_iter()` and `PrivateKeyDer::from_pem_file()` for loading cert/key from disk. |
| `std::net::TcpListener` | stdlib | TCP accept loop | Standard Rust. Replaces tiny_http's listener. |
| `std::io::{BufReader, Read, Write}` | stdlib | Buffered HTTP parsing | Standard Rust. BufReader for efficient line-by-line header parsing. |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| (none) | - | - | No new dependencies needed. All cert loading is via rustls-pki-types PemObject trait. |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Hand-rolled HTTP parser | `httparse` crate | httparse is fast (SIMD-optimized) and safe, but adds a new dependency. Snow's HTTP needs are minimal (no pipelining, no chunked input, no keep-alive). A ~150-line parser is sufficient and avoids dependency churn. |
| Hand-rolled HTTP parser | `hyper` | Massively over-engineered for this use case. Pulls in tokio, requires async runtime. Snow uses blocking I/O on coroutines. |
| Removing tiny_http | Keep tiny_http for plaintext, hand-roll only TLS | Creates two code paths. Unifying on one parser is simpler, removes the rustls 0.20 transitive dependency conflict, and reduces total dependencies. |

### Dependency Changes
```toml
# REMOVE from snow-rt/Cargo.toml:
# tiny_http = "0.12"

# KEEP (already present from Phase 55):
rustls = { version = "0.23", default-features = false, features = ["ring", "tls12", "logging", "std"] }
rustls-pki-types = "1"
```

Removing `tiny_http` also removes its transitive deps: `ascii`, `chunked_transfer`, `httpdate`, `log`. Net dependency reduction.

## Architecture Patterns

### Recommended File Structure
```
crates/snow-rt/src/
  http/
    mod.rs           # MODIFY: add snow_http_serve_tls export
    server.rs        # REWRITE: replace tiny_http with hand-rolled parser
    router.rs        # UNCHANGED
    client.rs        # UNCHANGED
```

### Pattern 1: HttpStream Enum (Mirroring PgStream)
**What:** An enum dispatching between plain TCP and TLS-wrapped TCP streams, exactly like Phase 55's PgStream.
**When to use:** Every connection handler receives an HttpStream. The HTTP parser operates on it via `Read + Write`.
**Example:**
```rust
// Source: Phase 55 PgStream pattern + rustls docs (StreamOwned)
use rustls::{ServerConnection, StreamOwned};
use std::io::{Read, Write, BufReader};
use std::net::TcpStream;

enum HttpStream {
    Plain(TcpStream),
    Tls(StreamOwned<ServerConnection, TcpStream>),
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            HttpStream::Plain(s) => s.read(buf),
            HttpStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for HttpStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            HttpStream::Plain(s) => s.write(buf),
            HttpStream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            HttpStream::Plain(s) => s.flush(),
            HttpStream::Tls(s) => s.flush(),
        }
    }
}
```

### Pattern 2: Minimal HTTP/1.1 Request Parser
**What:** Parse the request line, headers, and body from a `BufReader<&mut HttpStream>`.
**When to use:** Called for every incoming connection, before routing.
**Example:**
```rust
// Source: RFC 9112 (HTTP/1.1 Message Syntax) minimal implementation
use std::io::{BufRead, BufReader, Read};

struct ParsedRequest {
    method: String,
    path: String,       // includes query string: "/api/users?page=1"
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn parse_request(stream: &mut HttpStream) -> Result<ParsedRequest, String> {
    let mut reader = BufReader::new(stream);

    // 1. Read request line: "GET /path HTTP/1.1\r\n"
    let mut request_line = String::new();
    reader.read_line(&mut request_line)
        .map_err(|e| format!("read request line: {}", e))?;
    let request_line = request_line.trim_end();

    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("malformed request line: {}", request_line));
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    // 2. Read headers until blank line
    let mut headers = Vec::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)
            .map_err(|e| format!("read header: {}", e))?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;  // blank line = end of headers
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }

    // 3. Read body based on Content-Length
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)
            .map_err(|e| format!("read body: {}", e))?;
    }

    Ok(ParsedRequest { method, path, headers, body })
}
```

### Pattern 3: HTTP/1.1 Response Writer
**What:** Write a properly formatted HTTP/1.1 response.
**When to use:** After the Snow handler returns a SnowHttpResponse.
**Example:**
```rust
// Source: RFC 9112 response format
fn write_response(
    stream: &mut HttpStream,
    status: u16,
    body: &[u8],
) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };

    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, status_text, body.len()
    );

    stream.write_all(header.as_bytes())
        .map_err(|e| format!("write response header: {}", e))?;
    stream.write_all(body)
        .map_err(|e| format!("write response body: {}", e))?;
    stream.flush()
        .map_err(|e| format!("flush response: {}", e))?;
    Ok(())
}
```

### Pattern 4: TLS Accept with ServerConfig
**What:** Build a `ServerConfig` once, wrap each accepted `TcpStream` in `StreamOwned<ServerConnection, TcpStream>`.
**When to use:** In `snow_http_serve_tls`, called once at server startup. The config is shared across all connections.
**Example:**
```rust
// Source: rustls docs (ServerConfig, ServerConnection, StreamOwned)
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::sync::Arc;

fn build_server_config(
    cert_path: &str,
    key_path: &str,
) -> Result<Arc<ServerConfig>, String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| format!("open cert file: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse certs: {}", e))?;

    let key = PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| format!("load key: {}", e))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("TLS config: {}", e))?;

    Ok(Arc::new(config))
}

fn accept_tls(
    tcp_stream: TcpStream,
    config: &Arc<ServerConfig>,
) -> Result<StreamOwned<ServerConnection, TcpStream>, String> {
    let conn = ServerConnection::new(Arc::clone(config))
        .map_err(|e| format!("TLS connection: {}", e))?;
    let mut tls_stream = StreamOwned::new(conn, tcp_stream);
    // The TLS handshake happens lazily on first read/write.
    // StreamOwned::new does no I/O.
    Ok(tls_stream)
}
```

### Pattern 5: Unified Accept Loop (Plaintext and TLS)
**What:** Both `snow_http_serve` and `snow_http_serve_tls` use the same accept loop structure, differing only in whether they wrap the TcpStream in TLS.
**When to use:** Core server loop pattern.
**Example:**
```rust
// Shared accept loop -- the only difference is how the stream is created
fn serve_loop(
    listener: TcpListener,
    tls_config: Option<Arc<ServerConfig>>,
    router_addr: usize,
) {
    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[snow-rt] accept error: {}", e);
                continue;
            }
        };

        let stream = if let Some(ref config) = tls_config {
            match accept_tls(tcp_stream, config) {
                Ok(tls) => HttpStream::Tls(tls),
                Err(e) => {
                    eprintln!("[snow-rt] TLS handshake failed: {}", e);
                    continue;
                }
            }
        } else {
            HttpStream::Plain(tcp_stream)
        };

        // Pack stream into ConnectionArgs and spawn actor
        // (same pattern as current server.rs)
    }
}
```

### Pattern 6: Codegen Integration (serve_tls intrinsic)
**What:** Add `snow_http_serve_tls` as a new runtime function callable from Snow as `HTTP.serve_tls(router, port, cert_path, key_path)`.
**When to use:** Compiler changes needed in mir/lower.rs and codegen/intrinsics.rs.
**Example:**
```rust
// In snow-codegen/src/codegen/intrinsics.rs:
// snow_http_serve_tls(router: ptr, port: i64, cert_path: ptr, key_path: ptr) -> void
module.add_function("snow_http_serve_tls",
    void_type.fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into(), ptr_type.into()], false),
    Some(inkwell::module::Linkage::External));

// In snow-codegen/src/mir/lower.rs (known_functions):
self.known_functions.insert("snow_http_serve_tls".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Int, MirType::String, MirType::String],
    Box::new(MirType::Unit)));

// In snow-codegen/src/mir/lower.rs (map_builtin_name):
"http_serve_tls" => "snow_http_serve_tls".to_string(),
```

The mapping works automatically: `HTTP.serve_tls(...)` -> STDLIB_MODULES detects "HTTP" -> prefixes as `http_serve_tls` -> map_builtin_name -> `snow_http_serve_tls`. The module "HTTP" is already in `STDLIB_MODULES`.

### Anti-Patterns to Avoid
- **Keeping tiny_http alongside hand-rolled parser:** Two HTTP parsing paths means double the testing surface and the rustls 0.20 conflict remains. Remove tiny_http entirely.
- **Doing TLS handshake in the accept loop thread:** The TLS handshake involves network I/O (multiple round trips). If done in the accept loop, it blocks new connection acceptance. Instead, spawn the actor FIRST, then do the TLS handshake inside the actor. This matches success criterion 4: "TLS handshakes do not block the actor scheduler -- unrelated actors continue executing during handshake processing." The handshake happens inside the actor on a scheduler worker thread, which is what the M:N scheduler is designed for.
- **Using `BufReader` that outlives the stream:** `BufReader` buffers data internally. If you create a `BufReader` for header parsing then drop it to read the body directly from the stream, buffered data is lost. Instead, read headers AND body through the same `BufReader`, or use `BufReader::into_inner()` + `BufReader::buffer()` carefully.
- **Building ServerConfig per connection:** `ServerConfig` loads cert/key from disk and validates them. Build it ONCE before the accept loop and share via `Arc<ServerConfig>`.
- **Parsing HTTP/1.1 too strictly:** Real HTTP clients send headers with varying capitalization, optional whitespace, etc. Use case-insensitive header name comparison for Content-Length. Do not reject requests with unexpected headers.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| TLS protocol | Custom TLS | `rustls 0.23` ServerConnection + StreamOwned | TLS is thousands of lines of audited crypto. Already compiled. |
| PEM file parsing | Custom PEM parser | `rustls-pki-types` PemObject trait | PEM has edge cases (multiple certs in chain, PKCS#1/PKCS#8/SEC1 key formats). PemObject handles all. |
| Certificate chain validation | Custom cert validation | rustls built-in verifier | Chain validation, revocation, hostname matching are complex. Not needed for server-side anyway (server presents certs, doesn't validate them). |
| Crypto provider | Custom AES/etc | ring via rustls | Already installed at startup (Phase 55). |

**Key insight:** The only thing we hand-roll is HTTP/1.1 parsing (~150 lines) and response writing (~30 lines). Everything else (TLS, certs, crypto) uses existing libraries already in the dependency tree. The HTTP parser is hand-rolled specifically because: (a) Snow's needs are minimal (no chunked, no keep-alive, no HTTP/2), (b) adding httparse is a new dependency for ~150 lines of equivalent code, and (c) it eliminates tiny_http and its outdated rustls 0.20 dep.

## Common Pitfalls

### Pitfall 1: TLS Handshake Blocking the Accept Loop
**What goes wrong:** If the TLS handshake (ServerConnection + complete_io) runs in the accept loop thread, a slow/malicious client can block all new connections during the handshake.
**Why it happens:** The TLS handshake involves multiple network round trips (ClientHello, ServerHello, key exchange). A slow client can stall this for seconds.
**How to avoid:** Spawn the actor FIRST with the raw `TcpStream` + `Arc<ServerConfig>`. The actor performs the TLS handshake inside its coroutine on a scheduler worker thread. Other actors (and the accept loop) continue running. This is the same pattern as Phase 55's PostgreSQL TLS -- the handshake runs inside the connection context, not the listener.
**Warning signs:** Server becomes unresponsive to new connections when a client connects slowly. The accept loop is stuck waiting for one handshake to complete.

### Pitfall 2: BufReader Data Loss
**What goes wrong:** Creating a `BufReader`, reading headers, then dropping it and reading the body directly from the underlying stream. The BufReader may have buffered some body bytes that are now lost.
**Why it happens:** `BufReader` reads in chunks (default 8KB). If the client sends headers + body in one TCP segment, the BufReader will buffer some body bytes while reading headers.
**How to avoid:** Read BOTH headers and body through the same `BufReader`. Use `read_line()` for headers, then `read_exact()` for the body, all through the same `BufReader` instance. Alternatively, read all available data into a contiguous buffer first, then parse.
**Warning signs:** POST request bodies are empty or truncated. GET requests work fine (no body).

### Pitfall 3: Missing Content-Length Sends Empty Body
**What goes wrong:** The hand-rolled response writer forgets the `Content-Length` header. HTTP/1.1 clients may not read the body or may hang waiting for more data.
**Why it happens:** tiny_http handled this automatically. When hand-rolling, it's easy to forget.
**How to avoid:** Always include `Content-Length: {body.len()}` in the response. Also include `Connection: close` since Snow does not implement keep-alive (each connection = one request-response cycle).
**Warning signs:** curl shows empty response body. Browser hangs waiting for response.

### Pitfall 4: Incorrect Request Line Parsing
**What goes wrong:** Splitting the request line by space fails when the URL contains encoded spaces or when the HTTP version is missing (HTTP/0.9).
**Why it happens:** HTTP/1.1 request line is `METHOD SP Request-Target SP HTTP-Version CRLF`. Some clients may send only method and path (HTTP/0.9 legacy).
**How to avoid:** Use `splitn(3, ' ')` to split into at most 3 parts. Require at least 2 parts (method + path). The HTTP version (third part) can be ignored since Snow only needs to respond with HTTP/1.1 regardless.
**Warning signs:** Parsing errors on legitimate requests. Random 400 errors.

### Pitfall 5: ConnectionArgs Must Transfer Stream Ownership
**What goes wrong:** The current `ConnectionArgs` transfers a `Box<tiny_http::Request>` as a `usize`. The new code must transfer the `HttpStream` (which may contain a `StreamOwned`) the same way.
**Why it happens:** The actor spawn pattern requires passing data as raw pointers through the scheduler. `HttpStream::Tls(StreamOwned<ServerConnection, TcpStream>)` is not `Send` by default (ServerConnection contains `!Send` internals).
**How to avoid:** Box the `HttpStream` and transfer as `usize`, same as the existing `tiny_http::Request` pattern. The Send safety is maintained by the same reasoning as the current code: the boxed value is constructed on one thread, consumed on another, and never shared.
**Warning signs:** Compiler errors about `Send` bounds. Solution: transfer ownership via raw pointer (usize), not via Send channel.

### Pitfall 6: Existing E2E Tests Break
**What goes wrong:** The HTTP E2E tests in `crates/snowc/tests/e2e_stdlib.rs` make raw TCP connections and send/parse HTTP requests manually. The hand-rolled parser must produce responses with the same format as tiny_http's responses (status line, headers, body).
**Why it happens:** The tests use `TcpStream` to send raw HTTP requests and assert on the raw response string (e.g., checking for "200" and JSON body content).
**How to avoid:** Ensure the response format matches: `HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: N\r\n\r\n{body}`. Run existing E2E tests after replacing the parser to verify compatibility.
**Warning signs:** E2E tests fail on response parsing. The assertions check for `"200"` in the response string, so the status line format matters.

### Pitfall 7: Read Timeout on TLS Stream
**What goes wrong:** Without a read timeout, a malicious client that opens a TLS connection but never sends data will cause the handler actor to block forever on `read_line()`.
**Why it happens:** The TLS stream inherits the underlying TcpStream's timeout settings. If no timeout is set before wrapping in StreamOwned, reads are unbounded.
**How to avoid:** Set `tcp_stream.set_read_timeout(Some(Duration::from_secs(30)))` BEFORE wrapping in StreamOwned. The timeout is preserved because StreamOwned delegates reads to the underlying TcpStream. This matches the existing tiny_http behavior.
**Warning signs:** Actors accumulate over time, consuming scheduler worker threads. The server eventually stops accepting new connections.

## Code Examples

Verified patterns from official sources:

### Server Certificate Loading (rustls-pki-types PemObject)
```rust
// Source: https://docs.rs/rustls-pki-types/latest/rustls_pki_types/struct.CertificateDer.html
// Source: https://docs.rs/rustls-pki-types/latest/rustls_pki_types/enum.PrivateKeyDer.html
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

// Load certificate chain from PEM file (may contain multiple certs)
let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
    .map_err(|e| format!("open cert file: {}", e))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("parse cert: {}", e))?;

// Load private key from PEM file (PKCS#1, PKCS#8, or SEC1 auto-detected)
let key = PrivateKeyDer::from_pem_file(key_path)
    .map_err(|e| format!("load key: {}", e))?;
```

### ServerConfig Builder (rustls 0.23)
```rust
// Source: https://docs.rs/rustls/latest/rustls/struct.ConfigBuilder.html
// Typestate pattern: WantsVerifier -> with_no_client_auth() -> WantsServerCert -> with_single_cert() -> ServerConfig
use rustls::ServerConfig;
use std::sync::Arc;

let config = ServerConfig::builder()
    .with_no_client_auth()       // No client certificate required
    .with_single_cert(certs, key) // Server's cert chain + private key
    .map_err(|e| format!("TLS config: {}", e))?;

let config = Arc::new(config); // Share across all connections
```

### ServerConnection + StreamOwned
```rust
// Source: https://docs.rs/rustls/latest/rustls/server/struct.ServerConnection.html
// Source: https://docs.rs/rustls/latest/rustls/struct.StreamOwned.html
use rustls::{ServerConnection, StreamOwned};

// Per-connection: create ServerConnection, wrap with StreamOwned
let conn = ServerConnection::new(Arc::clone(&config))
    .map_err(|e| format!("TLS connection: {}", e))?;
// StreamOwned::new does NO I/O. TLS handshake happens on first read/write.
let tls_stream = StreamOwned::new(conn, tcp_stream);
```

### Complete snow_http_serve_tls Runtime Function
```rust
// New runtime entry point for HTTPS serving
#[no_mangle]
pub extern "C" fn snow_http_serve_tls(
    router: *mut u8,
    port: i64,
    cert_path: *const SnowString,
    key_path: *const SnowString,
) {
    crate::actor::snow_rt_init_actor(0);

    let cert_str = unsafe { (*cert_path).as_str() };
    let key_str = unsafe { (*key_path).as_str() };

    let tls_config = match build_server_config(cert_str, key_str) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[snow-rt] Failed to load TLS certificates: {}", e);
            return;
        }
    };

    let addr = format!("0.0.0.0:{}", port);
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[snow-rt] Failed to bind {}: {}", addr, e);
            return;
        }
    };

    eprintln!("[snow-rt] HTTPS server listening on {}", addr);

    let router_addr = router as usize;
    serve_loop(listener, Some(tls_config), router_addr);
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| tiny_http with ssl-rustls (rustls 0.20) | Hand-rolled parser with rustls 0.23 | Phase 56 (now) | Eliminates rustls version conflict. Single TLS implementation for PG + HTTP. |
| `rustls-pemfile` crate for PEM loading | `rustls-pki-types` PemObject trait | Aug 2025 (rustls-pemfile archived) | PEM loading is now built into pki-types. No extra dependency needed. |
| `ServerConfig::new(NoClientAuth)` | `ServerConfig::builder().with_no_client_auth().with_single_cert()` | rustls 0.21 (2023) | Typestate builder ensures compile-time correctness. |
| Implicit crypto provider | Explicit `CryptoProvider::install_default()` | rustls 0.22 (Dec 2023) | Must install ring provider at startup. Already done in Phase 55. |

**Deprecated/outdated:**
- `tiny_http`'s `ssl-rustls` feature: Uses rustls 0.20 (from 2022). Incompatible with rustls 0.23. Being removed in this phase.
- `rustls-pemfile` crate: Archived Aug 2025. Functionality moved to `rustls-pki-types`. Use `PemObject` trait instead.
- `ServerConfig::new(NoClientAuth::new())`: Pre-0.21 API. Use builder pattern.

## Open Questions

1. **BufReader vs. single-buffer approach for HTTP parsing**
   - What we know: `BufReader` provides `read_line()` which is convenient for header-per-line parsing. However, it buffers data that must not be lost before body reading.
   - What's unclear: Whether to use BufReader throughout (headers + body) or read everything into a Vec<u8> first then parse.
   - Recommendation: Use BufReader for the entire request lifecycle (headers via `read_line()`, body via `read_exact()`). Keep the BufReader alive until both headers and body are read. This avoids the buffered-data-loss pitfall.

2. **Connection: close vs keep-alive**
   - What we know: Snow's current tiny_http setup handles one request per connection (the accept loop yields one request per `incoming_requests()` call, which is one per TCP connection for practical purposes). The actor-per-connection model assumes one request = one actor.
   - What's unclear: Whether to support HTTP keep-alive in the future.
   - Recommendation: Always send `Connection: close` in responses. One request per connection is the correct model for Snow's actor system. If keep-alive is ever wanted, it would be a separate phase.

3. **Error responses for TLS failures**
   - What we know: If the TLS handshake fails (bad client cert, protocol mismatch), no HTTP response can be sent (TLS is below HTTP).
   - What's unclear: Whether to log TLS failures to stderr or silently drop them.
   - Recommendation: Log to stderr with `[snow-rt] TLS handshake failed: {error}` and continue accepting new connections. This matches the existing error logging pattern for accept failures.

4. **Maximum request size**
   - What we know: tiny_http has built-in limits. The hand-rolled parser needs limits to prevent denial-of-service.
   - What's unclear: What limits are appropriate.
   - Recommendation: Limit header section to 8KB (RFC 9112 recommends at least 8000 octets for request-line). Limit body to Content-Length value (no limit on body size -- Snow programs must handle their own input validation). Limit number of headers to 100 (generous, prevents abuse).

## Sources

### Primary (HIGH confidence)
- **Snow codebase** (direct reading):
  - `crates/snow-rt/src/http/server.rs` -- Current tiny_http-based server, ConnectionArgs pattern, handle_request function, actor spawn via global_scheduler
  - `crates/snow-rt/src/http/router.rs` -- SnowRouter, route matching, middleware chain (UNCHANGED by this phase)
  - `crates/snow-rt/src/http/mod.rs` -- Module exports (needs snow_http_serve_tls added)
  - `crates/snow-rt/src/db/pg.rs` -- PgStream enum pattern (Phase 55 prior art for HttpStream)
  - `crates/snow-rt/src/gc.rs` -- snow_rt_init() with CryptoProvider installation (Phase 55)
  - `crates/snow-rt/Cargo.toml` -- Current deps (tiny_http 0.12, rustls 0.23 already present)
  - `crates/snow-codegen/src/codegen/intrinsics.rs` -- snow_http_serve intrinsic declaration (pattern for serve_tls)
  - `crates/snow-codegen/src/mir/lower.rs` -- STDLIB_MODULES, map_builtin_name, known_functions (pattern for serve_tls)
  - `crates/snowc/tests/e2e_stdlib.rs` -- HTTP E2E tests (must pass after tiny_http replacement)
  - `Cargo.lock` -- Confirmed: tiny_http 0.12.0 deps are ascii, chunked_transfer, httpdate, log

- **rustls Official Documentation** (v0.23.36):
  - [ServerConfig](https://docs.rs/rustls/latest/rustls/server/struct.ServerConfig.html) -- Builder pattern, with_single_cert
  - [ServerConnection](https://docs.rs/rustls/latest/rustls/server/struct.ServerConnection.html) -- new(Arc<ServerConfig>), implements Deref<Target=ConnectionCommon>
  - [StreamOwned](https://docs.rs/rustls/latest/rustls/struct.StreamOwned.html) -- Works with ServerConnection, implements Read+Write+BufRead
  - [ConfigBuilder](https://docs.rs/rustls/latest/rustls/struct.ConfigBuilder.html) -- Typestate: WantsVerifier -> with_no_client_auth -> WantsServerCert -> with_single_cert -> ServerConfig

- **rustls-pki-types Official Documentation** (v1.14.0):
  - [CertificateDer](https://docs.rs/rustls-pki-types/latest/rustls_pki_types/struct.CertificateDer.html) -- pem_file_iter for loading cert chain
  - [PrivateKeyDer](https://docs.rs/rustls-pki-types/latest/rustls_pki_types/enum.PrivateKeyDer.html) -- from_pem_file for key loading, auto-detects PKCS#1/PKCS#8/SEC1

- **RFC 9112** (HTTP/1.1 Message Syntax):
  - [Request line parsing](https://www.rfc-editor.org/rfc/rfc9112.html) -- METHOD SP Request-Target SP HTTP-Version CRLF
  - Headers: field-name ":" OWS field-value OWS CRLF, terminated by blank CRLF
  - Body: Content-Length determines body length

### Secondary (MEDIUM confidence)
- **Snow project research** (`.planning/research/`):
  - `STACK.md` -- Pre-planned tiny_http replacement, ServerConfig code examples, estimated ~350 lines for server.rs rewrite
  - `FEATURES.md` -- Original HTTPS feature analysis (noted that tiny_http's ssl-rustls is incompatible with rustls 0.23)
  - `PITFALLS.md` -- Pitfall notes on TLS version conflicts

- **Phase 55 Research** (`.planning/phases/55-postgresql-tls/55-RESEARCH.md`):
  - PgStream enum pattern (proven in Phase 55, directly applicable)
  - CryptoProvider installation pattern (already done)
  - StreamOwned usage pattern (client-side, same applies server-side)

### Tertiary (LOW confidence)
- None -- all critical claims verified against official docs, codebase, or RFC specifications.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- All deps already present (rustls 0.23, rustls-pki-types 1.14.0). No new crate additions. PEM loading API verified on docs.rs.
- Architecture: HIGH -- HttpStream enum mirrors proven PgStream pattern. ServerConfig/ServerConnection API verified against docs.rs. Request parsing follows RFC 9112 minimal subset.
- Pitfalls: HIGH -- BufReader data loss, TLS handshake blocking, and E2E test compatibility are verified concerns from codebase analysis. Connection transfer pattern proven by existing server.rs code.

**Research date:** 2026-02-12
**Valid until:** 2026-03-12 (stable domain -- HTTP/1.1 and rustls 0.23 APIs are stable)
