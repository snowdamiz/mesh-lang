//! HTTP server runtime for the Snow language.
//!
//! Uses a hand-rolled HTTP/1.1 request parser and response writer with the
//! Snow actor system for per-connection handling. Each incoming connection is
//! dispatched to a lightweight actor (corosensei coroutine on the M:N
//! scheduler) rather than an OS thread, benefiting from 64 KiB stacks and
//! crash isolation via `catch_unwind`.
//!
//! ## History
//!
//! Phase 8 used `std::thread::spawn` for per-connection handling. Phase 15
//! replaced this with actor-per-connection using the existing lightweight
//! actor system, unifying the runtime model. Phase 56-01 replaced the tiny_http
//! library with a hand-rolled HTTP/1.1 parser to eliminate a rustls 0.20
//! transitive dependency conflict with rustls 0.23 used by the rest of the
//! runtime. Phase 56-02 added TLS support via `HttpStream` enum (mirrors the
//! `PgStream` pattern from Phase 55), enabling both HTTP and HTTPS serving
//! through the same actor infrastructure. Blocking I/O is accepted (similar
//! to BEAM NIFs) since each actor runs on a scheduler worker thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rustls::{ServerConfig, ServerConnection, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

use crate::actor;
use crate::collections::map;
use crate::gc::snow_gc_alloc_actor;
use crate::string::{snow_string_new, SnowString};

use super::router::{MiddlewareEntry, SnowRouter};

// ── Stream Abstraction ──────────────────────────────────────────────────

/// A connection stream that may be plain TCP or TLS-wrapped.
///
/// Mirrors the `PgStream` pattern from `crates/snow-rt/src/db/pg.rs` (Phase 55).
/// Both variants implement `Read` and `Write`, enabling `parse_request` and
/// `write_response` to operate on either stream type transparently.
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

// ── TLS Configuration ───────────────────────────────────────────────────

/// Build a rustls `ServerConfig` from PEM-encoded certificate and private key files.
///
/// The certificate file may contain a chain (multiple PEM blocks). The private
/// key file must contain exactly one PEM-encoded private key (RSA, ECDSA, or Ed25519).
pub(crate) fn build_server_config(cert_path: &str, key_path: &str) -> Result<Arc<ServerConfig>, String> {
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

// ── Request/Response structs ────────────────────────────────────────────

/// HTTP request representation passed to Snow handler functions.
///
/// All fields are opaque pointers at the LLVM level. The Snow program
/// accesses them via accessor functions (request_method, request_path, etc.).
///
/// IMPORTANT: This struct is `#[repr(C)]` -- new fields MUST be appended
/// at the end to preserve existing field offsets.
#[repr(C)]
pub struct SnowHttpRequest {
    /// HTTP method as SnowString (e.g. "GET", "POST").
    pub method: *mut u8,
    /// Request path as SnowString (e.g. "/api/users").
    pub path: *mut u8,
    /// Request body as SnowString (empty string for GET).
    pub body: *mut u8,
    /// Query parameters as SnowMap (string keys -> string values).
    pub query_params: *mut u8,
    /// Headers as SnowMap (string keys -> string values).
    pub headers: *mut u8,
    /// Path parameters as SnowMap (string keys -> string values).
    /// Populated by the router when matching parameterized routes.
    pub path_params: *mut u8,
}

/// HTTP response returned by Snow handler functions.
#[repr(C)]
pub struct SnowHttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status: i64,
    /// Response body as SnowString.
    pub body: *mut u8,
}

// ── Response constructor ───────────────────────────────────────────────

/// Create a new HTTP response with the given status code and body.
#[no_mangle]
pub extern "C" fn snow_http_response_new(status: i64, body: *const SnowString) -> *mut u8 {
    unsafe {
        let ptr = snow_gc_alloc_actor(
            std::mem::size_of::<SnowHttpResponse>() as u64,
            std::mem::align_of::<SnowHttpResponse>() as u64,
        ) as *mut SnowHttpResponse;
        (*ptr).status = status;
        (*ptr).body = body as *mut u8;
        ptr as *mut u8
    }
}

// ── Request accessors ──────────────────────────────────────────────────

/// Get the HTTP method from a request.
#[no_mangle]
pub extern "C" fn snow_http_request_method(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const SnowHttpRequest)).method }
}

/// Get the URL path from a request.
#[no_mangle]
pub extern "C" fn snow_http_request_path(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const SnowHttpRequest)).path }
}

/// Get the request body.
#[no_mangle]
pub extern "C" fn snow_http_request_body(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const SnowHttpRequest)).body }
}

/// Get the value of a request header by name. Returns SnowOption
/// (tag 0 = Some with SnowString, tag 1 = None).
#[no_mangle]
pub extern "C" fn snow_http_request_header(req: *mut u8, name: *const SnowString) -> *mut u8 {
    unsafe {
        let request = &*(req as *const SnowHttpRequest);
        let key_str = (*name).as_str();
        // Look up in the headers map. Keys are SnowString pointers stored as u64.
        let key_snow = snow_string_new(key_str.as_ptr(), key_str.len() as u64);
        let val = map::snow_map_get(request.headers, key_snow as u64);
        if val == 0 {
            // None
            alloc_option(1, std::ptr::null_mut())
        } else {
            // Some -- val is the SnowString pointer stored as u64
            alloc_option(0, val as *mut u8)
        }
    }
}

/// Get the value of a query parameter by name. Returns SnowOption
/// (tag 0 = Some with SnowString, tag 1 = None).
#[no_mangle]
pub extern "C" fn snow_http_request_query(req: *mut u8, name: *const SnowString) -> *mut u8 {
    unsafe {
        let request = &*(req as *const SnowHttpRequest);
        let key_str = (*name).as_str();
        let key_snow = snow_string_new(key_str.as_ptr(), key_str.len() as u64);
        let val = map::snow_map_get(request.query_params, key_snow as u64);
        if val == 0 {
            alloc_option(1, std::ptr::null_mut())
        } else {
            alloc_option(0, val as *mut u8)
        }
    }
}

/// Get the value of a path parameter by name. Returns SnowOption
/// (tag 0 = Some with SnowString, tag 1 = None).
///
/// Path parameters are extracted from parameterized route patterns
/// like `/users/:id`. For a request matching this pattern with path
/// `/users/42`, `Request.param(req, "id")` returns `Some("42")`.
#[no_mangle]
pub extern "C" fn snow_http_request_param(req: *mut u8, name: *const SnowString) -> *mut u8 {
    unsafe {
        let request = &*(req as *const SnowHttpRequest);
        let key_str = (*name).as_str();
        let key_snow = snow_string_new(key_str.as_ptr(), key_str.len() as u64);
        let val = map::snow_map_get(request.path_params, key_snow as u64);
        if val == 0 {
            alloc_option(1, std::ptr::null_mut())
        } else {
            alloc_option(0, val as *mut u8)
        }
    }
}

// ── Option allocation helper (shared from crate::option) ────────────────

fn alloc_option(tag: u8, value: *mut u8) -> *mut u8 {
    crate::option::alloc_option(tag, value) as *mut u8
}

// ── HTTP/1.1 Request Parser ─────────────────────────────────────────────

/// Parsed HTTP/1.1 request with method, path, headers, and body.
struct ParsedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Parse an HTTP/1.1 request from an `HttpStream` (plain TCP or TLS).
///
/// Uses `BufReader<&mut HttpStream>` so the stream can be reused for
/// writing the response after parsing completes (the BufReader borrows
/// the stream mutably, and the borrow ends when this function returns).
///
/// Limits: max 100 headers, max 8KB total header data.
fn parse_request(stream: &mut HttpStream) -> Result<ParsedRequest, String> {
    let mut reader = BufReader::new(stream);
    let mut total_header_bytes: usize = 0;

    // 1. Read request line: "GET /path HTTP/1.1\r\n"
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| format!("read request line: {}", e))?;
    total_header_bytes += request_line.len();

    let request_line_trimmed = request_line.trim_end();
    let parts: Vec<&str> = request_line_trimmed.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("malformed request line: {}", request_line_trimmed));
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    // 2. Read headers until blank line (\r\n alone).
    let mut headers = Vec::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("read header: {}", e))?;
        total_header_bytes += line.len();
        if total_header_bytes > 8192 {
            return Err("header section exceeds 8KB limit".to_string());
        }

        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break; // blank line = end of headers
        }
        if headers.len() >= 100 {
            return Err("too many headers (max 100)".to_string());
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

    // 3. Read body based on Content-Length.
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|e| format!("read body: {}", e))?;
    }

    Ok(ParsedRequest {
        method,
        path,
        headers,
        body,
    })
}

// ── HTTP/1.1 Response Writer ────────────────────────────────────────────

/// Write an HTTP/1.1 response to an `HttpStream` (plain TCP or TLS).
///
/// Format: status line, Content-Type, Content-Length, Connection: close,
/// blank line, body bytes.
fn write_response(stream: &mut HttpStream, status: u16, body: &[u8]) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    };

    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, status_text, body.len()
    );

    stream
        .write_all(header.as_bytes())
        .map_err(|e| format!("write response header: {}", e))?;
    stream
        .write_all(body)
        .map_err(|e| format!("write response body: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("flush response: {}", e))?;
    Ok(())
}

// ── Actor-per-connection infrastructure ────────────────────────────────

/// Arguments passed to the connection handler actor via raw pointer.
#[repr(C)]
struct ConnectionArgs {
    /// Router address as usize (for Send safety across thread boundaries).
    router_addr: usize,
    /// Raw pointer to a boxed `HttpStream`, transferred as usize.
    request_ptr: usize,
}

/// Actor entry function for handling a single HTTP connection.
///
/// Receives a raw pointer to `ConnectionArgs` containing the router
/// address and a boxed `HttpStream`. Wraps the handler call in
/// `catch_unwind` for crash isolation -- a panic in one handler does
/// not affect other connections.
///
/// The read timeout is already set on the underlying TcpStream before
/// wrapping in `HttpStream` (both Plain and Tls variants). For TLS
/// connections, the actual TLS handshake happens lazily on the first
/// `read` call (via `StreamOwned`), which occurs inside this actor --
/// not in the accept loop.
extern "C" fn connection_handler_entry(args: *const u8) {
    if args.is_null() {
        return;
    }

    let args = unsafe { Box::from_raw(args as *mut ConnectionArgs) };
    let router_ptr = args.router_addr as *mut u8;
    let mut stream = unsafe { *Box::from_raw(args.request_ptr as *mut HttpStream) };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match parse_request(&mut stream) {
            Ok(parsed) => {
                let (status, body) = process_request(router_ptr, parsed);
                let _ = write_response(&mut stream, status, &body);
            }
            Err(e) => {
                eprintln!("[snow-rt] HTTP parse error: {}", e);
            }
        }
    }));

    if let Err(panic_info) = result {
        eprintln!("[snow-rt] HTTP handler panicked: {:?}", panic_info);
    }
}

// ── Server ─────────────────────────────────────────────────────────────

/// Start an HTTP server on the given port, blocking the calling thread.
///
/// The server listens for incoming connections and dispatches each
/// request to a lightweight actor via the Snow actor scheduler. Each
/// connection handler runs as a coroutine (64 KiB stack) with crash
/// isolation via `catch_unwind` in `connection_handler_entry`.
///
/// Handler calling convention (same as closures in collections):
/// - If handler_env is null: `fn(request_ptr) -> response_ptr`
/// - If handler_env is non-null: `fn(handler_env, request_ptr) -> response_ptr`
#[no_mangle]
pub extern "C" fn snow_http_serve(router: *mut u8, port: i64) {
    // Ensure the actor scheduler is initialized (idempotent).
    crate::actor::snow_rt_init_actor(0);

    let addr = format!("0.0.0.0:{}", port);
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[snow-rt] Failed to start HTTP server on {}: {}", addr, e);
            return;
        }
    };

    eprintln!("[snow-rt] HTTP server listening on {}", addr);

    let router_addr = router as usize;

    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[snow-rt] accept error: {}", e);
                continue;
            }
        };

        // Set read timeout BEFORE wrapping in HttpStream.
        tcp_stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        let http_stream = HttpStream::Plain(tcp_stream);
        let stream_ptr = Box::into_raw(Box::new(http_stream)) as usize;
        let args = ConnectionArgs {
            router_addr,
            request_ptr: stream_ptr,
        };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;
        let args_size = std::mem::size_of::<ConnectionArgs>() as u64;

        let sched = actor::global_scheduler();
        sched.spawn(
            connection_handler_entry as *const u8,
            args_ptr,
            args_size,
            1, // Normal priority
        );
    }
}

// ── HTTPS Server ────────────────────────────────────────────────────────

/// Start an HTTPS server on the given port with TLS, blocking the calling thread.
///
/// Loads PEM-encoded certificate and private key files, builds a rustls
/// `ServerConfig`, and enters the same accept loop as `snow_http_serve`.
/// Each accepted connection is wrapped in `HttpStream::Tls` and dispatched
/// to a lightweight actor.
///
/// The TLS handshake is lazy: `StreamOwned::new()` does NO I/O. The actual
/// handshake occurs on the first `read` call inside the actor's coroutine,
/// ensuring the accept loop is never blocked by slow TLS clients.
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
    // Leak the Arc<ServerConfig> as a raw pointer for transfer into the loop.
    // The server runs forever, so this is intentional (no cleanup needed).
    let config_ptr = Arc::into_raw(tls_config) as usize;

    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[snow-rt] accept error: {}", e);
                continue;
            }
        };

        // Set read timeout BEFORE wrapping in TLS (Pitfall 7 from research).
        tcp_stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        // Reconstruct the Arc without dropping it (we leaked it intentionally).
        let tls_config = unsafe { Arc::from_raw(config_ptr as *const ServerConfig) };
        let conn = match ServerConnection::new(Arc::clone(&tls_config)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[snow-rt] TLS connection setup failed: {}", e);
                // Re-leak the Arc so it's available for the next connection.
                std::mem::forget(tls_config);
                continue;
            }
        };
        // Re-leak the Arc so it's available for the next connection.
        std::mem::forget(tls_config);

        // StreamOwned::new does NO I/O -- handshake is lazy on first read/write.
        // The actual handshake happens inside the actor when parse_request calls
        // BufReader::read_line -> HttpStream::Tls::read -> StreamOwned::read.
        let tls_stream = StreamOwned::new(conn, tcp_stream);
        let http_stream = HttpStream::Tls(tls_stream);

        let stream_ptr = Box::into_raw(Box::new(http_stream)) as usize;
        let args = ConnectionArgs {
            router_addr,
            request_ptr: stream_ptr,
        };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;
        let args_size = std::mem::size_of::<ConnectionArgs>() as u64;

        let sched = actor::global_scheduler();
        sched.spawn(
            connection_handler_entry as *const u8,
            args_ptr,
            args_size,
            1,
        );
    }
}

// ── Middleware chain infrastructure ──────────────────────────────────

/// State for the middleware chain trampoline.
///
/// Each step in the chain creates a new ChainState with `index + 1`,
/// builds a Snow closure wrapping `chain_next`, and calls the current
/// middleware with (request, next_closure).
struct ChainState {
    middlewares: Vec<MiddlewareEntry>,
    index: usize,
    handler_fn: *mut u8,
    handler_env: *mut u8,
}

/// Trampoline for the middleware `next` function.
///
/// This is what Snow calls when middleware invokes `next(request)`.
/// If all middleware has been traversed, calls the route handler.
/// Otherwise, calls the next middleware with a new `next` closure.
extern "C" fn chain_next(env_ptr: *mut u8, request_ptr: *mut u8) -> *mut u8 {
    unsafe {
        let state = &*(env_ptr as *const ChainState);
        if state.index >= state.middlewares.len() {
            // End of chain: call the route handler.
            call_handler(state.handler_fn, state.handler_env, request_ptr)
        } else {
            // Call the current middleware with a new next closure.
            let mw = &state.middlewares[state.index];
            let next_state = Box::new(ChainState {
                middlewares: state.middlewares.clone(),
                index: state.index + 1,
                handler_fn: state.handler_fn,
                handler_env: state.handler_env,
            });
            let next_env = Box::into_raw(next_state) as *mut u8;
            let next_closure = build_snow_closure(chain_next as *mut u8, next_env);
            call_middleware(mw.fn_ptr, mw.env_ptr, request_ptr, next_closure)
        }
    }
}

/// Build a Snow-compatible closure struct (GC-allocated).
///
/// Layout: `{ fn_ptr: *mut u8, env_ptr: *mut u8 }` -- 16 bytes, 8-byte aligned.
/// This matches Snow's closure representation used by the codegen.
fn build_snow_closure(fn_ptr: *mut u8, env_ptr: *mut u8) -> *mut u8 {
    unsafe {
        let closure = snow_gc_alloc_actor(16, 8) as *mut *mut u8;
        *closure = fn_ptr;
        *closure.add(1) = env_ptr;
        closure as *mut u8
    }
}

/// Call a route handler function.
///
/// If env_ptr is null: bare function `fn(request) -> response`.
/// If non-null: closure `fn(env, request) -> response`.
fn call_handler(fn_ptr: *mut u8, env_ptr: *mut u8, request: *mut u8) -> *mut u8 {
    unsafe {
        if env_ptr.is_null() {
            let f: fn(*mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(request)
        } else {
            let f: fn(*mut u8, *mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(env_ptr, request)
        }
    }
}

/// Call a middleware function.
///
/// Snow compiles middleware with signature `fn(request: ptr, next: {ptr, ptr}) -> ptr`.
/// The `next` parameter is a closure struct `{fn_ptr, env_ptr}` which LLVM's calling
/// convention decomposes into two separate register-passed arguments. So the actual
/// ABI signature is `fn(request, next_fn_ptr, next_env_ptr) -> response`.
///
/// If env_ptr (middleware's own env) is non-null, it's a closure middleware:
/// `fn(env, request, next_fn_ptr, next_env_ptr) -> response`.
fn call_middleware(fn_ptr: *mut u8, env_ptr: *mut u8, request: *mut u8, next_closure: *mut u8) -> *mut u8 {
    unsafe {
        // Dereference the next_closure pointer to extract fn_ptr and env_ptr fields.
        // The closure struct layout is { fn_ptr: *mut u8, env_ptr: *mut u8 } -- 16 bytes.
        let next_fn_ptr = *(next_closure as *const *mut u8);
        let next_env_ptr = *(next_closure as *const *mut u8).add(1);

        if env_ptr.is_null() {
            let f: fn(*mut u8, *mut u8, *mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(request, next_fn_ptr, next_env_ptr)
        } else {
            let f: fn(*mut u8, *mut u8, *mut u8, *mut u8) -> *mut u8 = std::mem::transmute(fn_ptr);
            f(env_ptr, request, next_fn_ptr, next_env_ptr)
        }
    }
}

/// Process a single HTTP request by matching it against the router
/// and calling the appropriate handler function.
///
/// Returns `(status_code, body_bytes)` for the response.
fn process_request(router_ptr: *mut u8, parsed: ParsedRequest) -> (u16, Vec<u8>) {
    unsafe {
        let router = &*(router_ptr as *const SnowRouter);

        // Build the SnowHttpRequest.
        let method_str = parsed.method;
        let method = snow_string_new(method_str.as_ptr(), method_str.len() as u64) as *mut u8;

        let url = parsed.path;
        // Split URL into path and query string.
        let (path_str, query_str) = match url.find('?') {
            Some(idx) => (&url[..idx], &url[idx + 1..]),
            None => (url.as_str(), ""),
        };
        let path = snow_string_new(path_str.as_ptr(), path_str.len() as u64) as *mut u8;

        // Body from parsed request.
        let body_bytes = parsed.body;
        let body = snow_string_new(body_bytes.as_ptr(), body_bytes.len() as u64) as *mut u8;

        // Parse query params into a SnowMap (string keys for content-based lookup).
        let mut query_map = map::snow_map_new_typed(1);
        if !query_str.is_empty() {
            for param in query_str.split('&') {
                if let Some((k, v)) = param.split_once('=') {
                    let key = snow_string_new(k.as_ptr(), k.len() as u64);
                    let val = snow_string_new(v.as_ptr(), v.len() as u64);
                    query_map = map::snow_map_put(query_map, key as u64, val as u64);
                }
            }
        }

        // Parse headers into a SnowMap (string keys for content-based lookup).
        let mut headers_map = map::snow_map_new_typed(1);
        for (name, value_str) in &parsed.headers {
            let key = snow_string_new(name.as_ptr(), name.len() as u64);
            let val = snow_string_new(value_str.as_ptr(), value_str.len() as u64);
            headers_map = map::snow_map_put(headers_map, key as u64, val as u64);
        }

        // Build the request struct (needed for both matched and 404 paths when middleware is present).
        let build_snow_request = |path_params_map: *mut u8| -> *mut u8 {
            let snow_req = snow_gc_alloc_actor(
                std::mem::size_of::<SnowHttpRequest>() as u64,
                std::mem::align_of::<SnowHttpRequest>() as u64,
            ) as *mut SnowHttpRequest;
            (*snow_req).method = method;
            (*snow_req).path = path;
            (*snow_req).body = body;
            (*snow_req).query_params = query_map;
            (*snow_req).headers = headers_map;
            (*snow_req).path_params = path_params_map;
            snow_req as *mut u8
        };

        // Match against router (now with method and path params).
        let matched = router.match_route(path_str, &method_str);
        let has_middleware = !router.middlewares.is_empty();

        let response_ptr = if let Some((handler_fn, handler_env, params)) = matched {
            // Convert captured path params into a SnowMap.
            let mut path_params_map = map::snow_map_new_typed(1);
            for (k, v) in &params {
                let key = snow_string_new(k.as_ptr(), k.len() as u64);
                let val = snow_string_new(v.as_ptr(), v.len() as u64);
                path_params_map = map::snow_map_put(path_params_map, key as u64, val as u64);
            }

            let req_ptr = build_snow_request(path_params_map);

            if has_middleware {
                // Execute middleware chain wrapping the matched handler.
                let state = Box::new(ChainState {
                    middlewares: router.middlewares.clone(),
                    index: 0,
                    handler_fn,
                    handler_env,
                });
                chain_next(Box::into_raw(state) as *mut u8, req_ptr)
            } else {
                // Fast path: no middleware, call handler directly.
                call_handler(handler_fn, handler_env, req_ptr)
            }
        } else if has_middleware {
            // 404 with middleware: wrap a synthetic 404 handler in the middleware chain.
            let path_params_map = map::snow_map_new_typed(1);
            let req_ptr = build_snow_request(path_params_map);

            // Synthetic 404 handler: returns a 404 response.
            extern "C" fn not_found_handler(_request: *mut u8) -> *mut u8 {
                let body_text = b"Not Found";
                let body = snow_string_new(body_text.as_ptr(), body_text.len() as u64);
                snow_http_response_new(404, body)
            }

            let state = Box::new(ChainState {
                middlewares: router.middlewares.clone(),
                index: 0,
                handler_fn: not_found_handler as *mut u8,
                handler_env: std::ptr::null_mut(),
            });
            chain_next(Box::into_raw(state) as *mut u8, req_ptr)
        } else {
            // 404 without middleware: respond directly.
            return (404, b"Not Found".to_vec());
        };

        // Extract response from the Snow response pointer.
        let resp = &*(response_ptr as *const SnowHttpResponse);
        let status_code = resp.status as u16;
        let body_str = if resp.body.is_null() {
            ""
        } else {
            let body_snow = &*(resp.body as *const SnowString);
            body_snow.as_str()
        };

        (status_code, body_str.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_response_creation() {
        snow_rt_init();
        let body = snow_string_new(b"Hello".as_ptr(), 5);
        let resp_ptr = snow_http_response_new(200, body);
        assert!(!resp_ptr.is_null());
        unsafe {
            let resp = &*(resp_ptr as *const SnowHttpResponse);
            assert_eq!(resp.status, 200);
            let body_str = &*(resp.body as *const SnowString);
            assert_eq!(body_str.as_str(), "Hello");
        }
    }

    #[test]
    fn test_request_accessors() {
        snow_rt_init();

        // Build a request manually.
        let method = snow_string_new(b"GET".as_ptr(), 3) as *mut u8;
        let path = snow_string_new(b"/test".as_ptr(), 5) as *mut u8;
        let body = snow_string_new(b"".as_ptr(), 0) as *mut u8;
        let query_params = map::snow_map_new();
        let headers = map::snow_map_new();
        let path_params = map::snow_map_new();

        unsafe {
            let req_ptr = snow_gc_alloc_actor(
                std::mem::size_of::<SnowHttpRequest>() as u64,
                std::mem::align_of::<SnowHttpRequest>() as u64,
            ) as *mut SnowHttpRequest;
            (*req_ptr).method = method;
            (*req_ptr).path = path;
            (*req_ptr).body = body;
            (*req_ptr).query_params = query_params;
            (*req_ptr).headers = headers;
            (*req_ptr).path_params = path_params;

            let req = req_ptr as *mut u8;

            // Test method accessor.
            let m = snow_http_request_method(req);
            let m_str = &*(m as *const SnowString);
            assert_eq!(m_str.as_str(), "GET");

            // Test path accessor.
            let p = snow_http_request_path(req);
            let p_str = &*(p as *const SnowString);
            assert_eq!(p_str.as_str(), "/test");

            // Test body accessor.
            let b = snow_http_request_body(req);
            let b_str = &*(b as *const SnowString);
            assert_eq!(b_str.as_str(), "");
        }
    }
}
