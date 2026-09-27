//! HTTP server runtime for the Mesh language.
//!
//! Uses a hand-rolled HTTP/1.1 request parser and response writer with the
//! Mesh actor system for per-connection handling. Each incoming connection is
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
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

use crate::actor;
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::callback::{call1, call3};
use crate::collections::map;
use crate::dist::telemetry::AdmissionController;
use crate::gc::mesh_gc_alloc_actor;
use crate::string::{mesh_str, MeshString};

use super::router::{ChainStep, MeshRouter, RouteEntry};

// ── Stream Abstraction ──────────────────────────────────────────────────

/// A connection stream that may be plain TCP or TLS-wrapped.
///
/// Mirrors the `PgStream` pattern from `crates/mesh-rt/src/db/pg.rs` (Phase 55).
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
pub(crate) fn build_server_config(
    cert_path: &str,
    key_path: &str,
) -> Result<Arc<ServerConfig>, String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| format!("open cert file: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse certs: {}", e))?;

    let key = PrivateKeyDer::from_pem_file(key_path).map_err(|e| format!("load key: {}", e))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("TLS config: {}", e))?;

    Ok(Arc::new(config))
}

/// A server session under `config`, one `build_server_config` made: a
/// session fails to start only on a maximum fragment size a config may not
/// have, and such a config keeps the default.
pub(crate) fn tls_session(config: &Arc<ServerConfig>) -> ServerConnection {
    ServerConnection::new(Arc::clone(config)).expect("a built TLS config starts sessions")
}

// ── Request/Response structs ────────────────────────────────────────────

/// HTTP request representation passed to Mesh handler functions.
///
/// All fields are opaque pointers at the LLVM level. The Mesh program
/// accesses them via accessor functions (request_method, request_path, etc.).
///
/// IMPORTANT: This struct is `#[repr(C)]` -- new fields MUST be appended
/// at the end to preserve existing field offsets.
#[repr(C)]
pub struct MeshHttpRequest {
    /// HTTP method as MeshString (e.g. "GET", "POST").
    pub method: *mut u8,
    /// Request path as MeshString (e.g. "/api/users").
    pub path: *mut u8,
    /// Request body as MeshString (empty string for GET).
    pub body: *mut u8,
    /// Query parameters as MeshMap (string keys -> string values).
    pub query_params: *mut u8,
    /// Headers as MeshMap (string keys -> string values).
    pub headers: *mut u8,
    /// Path parameters as MeshMap (string keys -> string values).
    /// Populated by the router when matching parameterized routes.
    pub path_params: *mut u8,
    /// Globally unique transport request identity, stable across remote dispatch.
    pub request_id: *mut u8,
    /// Validated caller idempotency key, or null when absent.
    pub idempotency_key: *mut u8,
    /// Request body as byte-exact MeshBytes.
    pub body_bytes: *mut u8,
}

/// HTTP response returned by Mesh handler functions.
///
/// IMPORTANT: This struct is `#[repr(C)]` -- new fields MUST be appended
/// at the end to preserve existing field offsets.
#[repr(C)]
pub struct MeshHttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status: i64,
    /// Response body as MeshString.
    pub body: *mut u8,
    /// Optional response headers as MeshMap (string keys -> string values).
    /// Null when no custom headers are set (backward compatible).
    pub headers: *mut u8,
    /// Byte-exact response body, or null for a text response.
    pub body_bytes: *mut u8,
}

// ── Response constructor ───────────────────────────────────────────────

/// Create a new HTTP response with the given status code and body.
/// Headers are set to null (no custom headers).
#[no_mangle]
pub extern "C" fn mesh_http_response_new(status: i64, body: *const MeshString) -> *mut u8 {
    unsafe {
        let ptr = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshHttpResponse>() as u64,
            std::mem::align_of::<MeshHttpResponse>() as u64,
        ) as *mut MeshHttpResponse;
        (*ptr).status = status;
        (*ptr).body = body as *mut u8;
        (*ptr).headers = std::ptr::null_mut();
        (*ptr).body_bytes = std::ptr::null_mut();
        ptr as *mut u8
    }
}

/// Create a new HTTP response with status, body, and custom headers.
///
/// The `headers` parameter is a MeshMap pointer (string keys -> string values).
/// These headers are emitted in the HTTP response alongside the standard headers.
#[no_mangle]
pub extern "C" fn mesh_http_response_with_headers(
    status: i64,
    body: *const MeshString,
    headers: *mut u8,
) -> *mut u8 {
    unsafe {
        let ptr = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshHttpResponse>() as u64,
            std::mem::align_of::<MeshHttpResponse>() as u64,
        ) as *mut MeshHttpResponse;
        (*ptr).status = status;
        (*ptr).body = body as *mut u8;
        (*ptr).headers = headers;
        (*ptr).body_bytes = std::ptr::null_mut();
        ptr as *mut u8
    }
}

/// Create a byte-exact HTTP response.
#[no_mangle]
pub extern "C" fn mesh_http_response_bytes_new(status: i64, body: *const MeshBytes) -> *mut u8 {
    mesh_http_response_bytes_with_headers(status, body, std::ptr::null_mut())
}

/// Create a byte-exact HTTP response with custom headers.
#[no_mangle]
pub extern "C" fn mesh_http_response_bytes_with_headers(
    status: i64,
    body: *const MeshBytes,
    headers: *mut u8,
) -> *mut u8 {
    unsafe {
        let ptr = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshHttpResponse>() as u64,
            std::mem::align_of::<MeshHttpResponse>() as u64,
        ) as *mut MeshHttpResponse;
        (*ptr).status = status;
        (*ptr).body = std::ptr::null_mut();
        (*ptr).headers = headers;
        (*ptr).body_bytes = body as *mut u8;
        ptr as *mut u8
    }
}

const CLUSTERED_ROUTE_FAILURE_STATUS: i64 = 503;
const CLUSTERED_ROUTE_REQUEST_KEY_HEADER: &str = "X-Mesh-Continuity-Request-Key";
const IDEMPOTENCY_REPLAY_HEADER: &str = "Idempotency-Replayed";
const CLUSTERED_ROUTE_INGRESS_HEADER: &str = "X-Mesh-Ingress-Node";
const CLUSTERED_ROUTE_EXECUTION_HEADER: &str = "X-Mesh-Execution-Node";
const CLUSTERED_ROUTE_REMOTE_HEADER: &str = "X-Mesh-Routed-Remotely";

#[derive(Clone, Debug, PartialEq, Eq)]
struct TransportHttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
    query_params: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    path_params: Vec<(String, String)>,
    request_id: String,
    idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TransportHttpResponse {
    status: i64,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

/// A request's or response's string field: a Mesh String, never null.
fn mesh_string_to_owned(ptr: *mut u8) -> String {
    unsafe { (*(ptr as *const MeshString)).as_str().to_string() }
}

/// A string-keyed map's entries, in order (a map may be a view of a
/// table); a response without custom headers has a null map, and none.
fn string_pairs(map_ptr: *mut u8) -> Vec<(String, String)> {
    if map_ptr.is_null() {
        return Vec::new();
    }
    let (_, entries) = unsafe { map::live_entries(map_ptr) };
    entries
        .iter()
        .map(|[key, value]| {
            (
                mesh_string_to_owned(*key as *mut u8),
                mesh_string_to_owned(*value as *mut u8),
            )
        })
        .collect()
}

fn pairs_to_mesh_map(pairs: &[(String, String)]) -> *mut u8 {
    let mut map_ptr = map::mesh_map_new_typed(1);
    for (key, value) in pairs {
        let key_ptr = mesh_str(key) as *mut u8;
        let value_ptr = mesh_str(value) as *mut u8;
        map_ptr = map::mesh_map_put(map_ptr, key_ptr as u64, value_ptr as u64);
    }
    map_ptr
}

/// A request as the runtime built it: every field set, the idempotency key
/// null when the caller gave none.
fn mesh_request_to_transport(request_ptr: *mut u8) -> TransportHttpRequest {
    let request = unsafe { &*(request_ptr as *const MeshHttpRequest) };
    TransportHttpRequest {
        method: mesh_string_to_owned(request.method),
        path: mesh_string_to_owned(request.path),
        body: unsafe {
            (*(request.body_bytes as *const MeshBytes))
                .as_slice()
                .to_vec()
        },
        query_params: string_pairs(request.query_params),
        headers: string_pairs(request.headers),
        path_params: string_pairs(request.path_params),
        request_id: mesh_string_to_owned(request.request_id),
        idempotency_key: (!request.idempotency_key.is_null())
            .then(|| mesh_string_to_owned(request.idempotency_key)),
    }
}

fn transport_request_to_mesh(request: &TransportHttpRequest) -> *mut u8 {
    unsafe {
        let req_ptr = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshHttpRequest>() as u64,
            std::mem::align_of::<MeshHttpRequest>() as u64,
        ) as *mut MeshHttpRequest;
        (*req_ptr).method = mesh_str(&request.method) as *mut u8;
        (*req_ptr).path = mesh_str(&request.path) as *mut u8;
        (*req_ptr).body = mesh_str(std::str::from_utf8(&request.body).unwrap_or("")) as *mut u8;
        (*req_ptr).query_params = pairs_to_mesh_map(&request.query_params);
        (*req_ptr).headers = pairs_to_mesh_map(&request.headers);
        (*req_ptr).path_params = pairs_to_mesh_map(&request.path_params);
        (*req_ptr).request_id = mesh_str(&request.request_id) as *mut u8;
        (*req_ptr).idempotency_key = request
            .idempotency_key
            .as_deref()
            .map_or(std::ptr::null_mut(), |key| mesh_str(key) as *mut u8);
        (*req_ptr).body_bytes =
            mesh_bytes_new(request.body.as_ptr(), request.body.len() as u64) as *mut u8;
        req_ptr as *mut u8
    }
}

/// A handler's response: its status, its body (the bytes of a byte-exact
/// response, else its text) and its headers, a byte-exact one's with an
/// octet-stream content type unless it names one.
fn mesh_response_to_transport(response_ptr: *mut u8) -> TransportHttpResponse {
    let response = unsafe { &*(response_ptr as *const MeshHttpResponse) };
    let mut headers = string_pairs(response.headers);
    let body = if response.body_bytes.is_null() {
        mesh_string_to_owned(response.body).into_bytes()
    } else {
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        {
            headers.push((
                "Content-Type".to_string(),
                "application/octet-stream".to_string(),
            ));
        }
        unsafe {
            (*(response.body_bytes as *const MeshBytes))
                .as_slice()
                .to_vec()
        }
    };
    TransportHttpResponse {
        status: response.status,
        body,
        headers,
    }
}

fn transport_response_to_mesh(response: &TransportHttpResponse) -> *mut u8 {
    if response.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value.eq_ignore_ascii_case("application/octet-stream")
    }) {
        let body = mesh_bytes_new(response.body.as_ptr(), response.body.len() as u64);
        let response_ptr = mesh_http_response_bytes_new(response.status, body);
        unsafe {
            (*(response_ptr as *mut MeshHttpResponse)).headers =
                pairs_to_mesh_map(&response.headers);
        }
        return response_ptr;
    }
    let body_text = std::str::from_utf8(&response.body).unwrap_or("");
    let body = mesh_str(body_text) as *const MeshString;
    if response.headers.is_empty() {
        mesh_http_response_new(response.status, body)
    } else {
        let headers = pairs_to_mesh_map(&response.headers);
        mesh_http_response_with_headers(response.status, body, headers)
    }
}

/// A length field: four little-endian bytes, which cannot carry 4 GiB or
/// more (the `label` field is too large then).
fn encode_len(payload: &mut Vec<u8>, len: usize, label: &str) -> Result<(), String> {
    let len =
        u32::try_from(len).map_err(|_| format!("mesh_http_transport_{label}_too_large:{len}"))?;
    payload.extend_from_slice(&len.to_le_bytes());
    Ok(())
}

fn encode_len_prefixed(payload: &mut Vec<u8>, value: &[u8], label: &str) -> Result<(), String> {
    encode_len(payload, value.len(), label)?;
    payload.extend_from_slice(value);
    Ok(())
}

fn encode_string_pairs(
    payload: &mut Vec<u8>,
    pairs: &[(String, String)],
    label: &str,
) -> Result<(), String> {
    encode_len(payload, pairs.len(), &format!("{label}_count"))?;
    for (index, (key, value)) in pairs.iter().enumerate() {
        encode_len_prefixed(payload, key.as_bytes(), &format!("{label}_key_{index}"))?;
        encode_len_prefixed(payload, value.as_bytes(), &format!("{label}_value_{index}"))?;
    }
    Ok(())
}

/// The length field at `pos`, if the payload holds one.
fn decode_len(payload: &[u8], pos: &mut usize) -> Option<usize> {
    let bytes = payload.get(*pos..*pos + 4)?;
    *pos += 4;
    Some(u32::from_le_bytes(bytes.try_into().expect("four length bytes")) as usize)
}

fn decode_len_prefixed_bytes(
    payload: &[u8],
    pos: &mut usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    let len = decode_len(payload, pos)
        .ok_or_else(|| format!("mesh_http_transport_{label}_len_missing"))?;
    let value = payload
        .get(*pos..*pos + len)
        .ok_or_else(|| format!("mesh_http_transport_{label}_truncated"))?
        .to_vec();
    *pos += len;
    Ok(value)
}

fn decode_len_prefixed_string(
    payload: &[u8],
    pos: &mut usize,
    label: &str,
) -> Result<String, String> {
    String::from_utf8(decode_len_prefixed_bytes(payload, pos, label)?)
        .map_err(|_| format!("mesh_http_transport_{label}_invalid_utf8"))
}

/// The pairs at `pos`. Their count is what the payload claims, not what it
/// holds: nothing is allocated for it up front.
fn decode_string_pairs(
    payload: &[u8],
    pos: &mut usize,
    label: &str,
) -> Result<Vec<(String, String)>, String> {
    let count = decode_len(payload, pos)
        .ok_or_else(|| format!("mesh_http_transport_{label}_count_missing"))?;
    (0..count)
        .map(|index| {
            Ok((
                decode_len_prefixed_string(payload, pos, &format!("{label}_key_{index}"))?,
                decode_len_prefixed_string(payload, pos, &format!("{label}_value_{index}"))?,
            ))
        })
        .collect()
}

fn encode_transport_request(request: &TransportHttpRequest) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    encode_len_prefixed(&mut payload, request.method.as_bytes(), "request_method")?;
    encode_len_prefixed(&mut payload, request.path.as_bytes(), "request_path")?;
    encode_len_prefixed(&mut payload, &request.body, "request_body")?;
    encode_string_pairs(&mut payload, &request.query_params, "request_query_params")?;
    encode_string_pairs(&mut payload, &request.headers, "request_headers")?;
    encode_string_pairs(&mut payload, &request.path_params, "request_path_params")?;
    encode_len_prefixed(&mut payload, request.request_id.as_bytes(), "request_id")?;
    match &request.idempotency_key {
        Some(key) => {
            payload.push(1);
            encode_len_prefixed(&mut payload, key.as_bytes(), "idempotency_key")?;
        }
        None => payload.push(0),
    }
    Ok(payload)
}

fn decode_transport_request(payload: &[u8]) -> Result<TransportHttpRequest, String> {
    if payload.is_empty() {
        return Err("mesh_http_transport_request_empty".to_string());
    }
    let mut pos = 0usize;
    let method = decode_len_prefixed_string(payload, &mut pos, "request_method")?;
    let path = decode_len_prefixed_string(payload, &mut pos, "request_path")?;
    let body = decode_len_prefixed_bytes(payload, &mut pos, "request_body")?;
    let query_params = decode_string_pairs(payload, &mut pos, "request_query_params")?;
    let headers = decode_string_pairs(payload, &mut pos, "request_headers")?;
    let path_params = decode_string_pairs(payload, &mut pos, "request_path_params")?;
    // A payload from before request IDs and idempotency keys ends here.
    let request_id = if pos == payload.len() {
        next_request_id()
    } else {
        decode_len_prefixed_string(payload, &mut pos, "request_id")?
    };
    let idempotency_key = match payload.get(pos) {
        None => None,
        Some(0) => {
            pos += 1;
            None
        }
        Some(1) => {
            pos += 1;
            let key = decode_len_prefixed_string(payload, &mut pos, "idempotency_key")?;
            crate::dist::identity::validate_idempotency_key(&key)?;
            Some(key)
        }
        Some(_) => return Err("mesh_http_transport_idempotency_key_flag_invalid".to_string()),
    };
    if pos != payload.len() {
        return Err("mesh_http_transport_request_trailing_bytes".to_string());
    }
    Ok(TransportHttpRequest {
        method,
        path,
        body,
        query_params,
        headers,
        path_params,
        request_id,
        idempotency_key,
    })
}

fn encode_transport_response(response: &TransportHttpResponse) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&response.status.to_le_bytes());
    encode_len_prefixed(&mut payload, &response.body, "response_body")?;
    encode_string_pairs(&mut payload, &response.headers, "response_headers")?;
    Ok(payload)
}

fn decode_transport_response(payload: &[u8]) -> Result<TransportHttpResponse, String> {
    let status = payload
        .get(..8)
        .ok_or_else(|| "mesh_http_transport_response_too_short".to_string())?;
    let mut pos = 8usize;
    let response = TransportHttpResponse {
        status: i64::from_le_bytes(status.try_into().expect("eight status bytes")),
        body: decode_len_prefixed_bytes(payload, &mut pos, "response_body")?,
        headers: decode_string_pairs(payload, &mut pos, "response_headers")?,
    };
    if pos != payload.len() {
        return Err("mesh_http_transport_response_trailing_bytes".to_string());
    }
    Ok(response)
}

/// A new request's identity. The generator's 64-bit counter does not run
/// out.
fn next_request_id() -> String {
    crate::dist::identity::request_id_generator()
        .next()
        .expect("the request ID counter is not exhausted")
        .to_string()
}

pub(crate) fn encode_http_request_payload(request_ptr: *mut u8) -> Result<Vec<u8>, String> {
    encode_transport_request(&mesh_request_to_transport(request_ptr))
}

pub(crate) fn decode_http_request_payload(payload: &[u8]) -> Result<*mut u8, String> {
    let request = decode_transport_request(payload)?;
    Ok(transport_request_to_mesh(&request))
}

/// Returns whether Mesh may safely start a replacement execution after an
/// indeterminate remote-dispatch failure.
///
/// A generated request ID is only a correlation identity. It must never turn
/// an unsafe mutation into a replayable operation. Caller-scoped idempotency
/// or an HTTP safe method is required before continuity recovery can execute
/// the retained request payload on a new owner.
pub(crate) fn http_request_payload_is_replay_safe(payload: &[u8]) -> Result<bool, String> {
    let request = decode_transport_request(payload)?;
    Ok(request.idempotency_key.is_some()
        || matches!(
            request.method.to_ascii_uppercase().as_str(),
            "GET" | "HEAD" | "OPTIONS" | "TRACE"
        ))
}

pub(crate) fn encode_http_response_payload(response_ptr: *mut u8) -> Result<Vec<u8>, String> {
    encode_transport_response(&mesh_response_to_transport(response_ptr))
}

pub(crate) fn decode_http_response_payload(payload: &[u8]) -> Result<*mut u8, String> {
    let response = decode_transport_response(payload)?;
    Ok(transport_response_to_mesh(&response))
}

pub(crate) fn invoke_route_handler_from_payload(
    fn_ptr: *mut u8,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    let request_ptr = decode_http_request_payload(request_payload)
        .map_err(|reason| format!("clustered_route_request_decode_failed:{reason}"))?;
    let response_ptr =
        unsafe { call1(fn_ptr, std::ptr::null_mut(), request_ptr as u64) as *mut u8 };
    encode_http_response_payload(response_ptr)
        .map_err(|reason| format!("clustered_route_response_encode_failed:{reason}"))
}

pub(crate) fn build_clustered_http_route_identity(
    runtime_name: &str,
    request_payload: &[u8],
) -> Result<(String, String), String> {
    let runtime_name = runtime_name.trim();
    if runtime_name.is_empty() {
        return Err("clustered_route_runtime_name_missing".to_string());
    }
    if request_payload.is_empty() {
        return Err("clustered_route_request_payload_missing".to_string());
    }

    let request = decode_transport_request(request_payload)?;
    let semantic_headers: Vec<(String, String)> = request
        .headers
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "content-type" | "if-match" | "if-none-match"
            )
        })
        .cloned()
        .collect();
    let tenant_scope = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-mesh-authenticated-tenant"))
        .map(|(_, value)| value.as_str());
    let payload_hash = crate::dist::identity::CanonicalHttpRequest {
        method: &request.method,
        route_id: runtime_name,
        path_parameters: &request.path_params,
        query_parameters: &request.query_params,
        semantic_headers: &semantic_headers,
        body: &request.body,
        tenant_scope,
    }
    .hash()?;

    let caller_key = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("idempotency-key"))
        .map(|(_, value)| value.as_str());
    let request_key = if let Some(caller_key) = caller_key {
        let application_id =
            std::env::var("MESH_APPLICATION_ID").unwrap_or_else(|_| "mesh-application".to_string());
        format!(
            "operation::{}",
            crate::dist::identity::OperationKey::derive(
                &application_id,
                runtime_name,
                tenant_scope,
                caller_key,
            )?
        )
    } else {
        if request.request_id.is_empty() {
            return Err("clustered_route_request_id_missing".to_string());
        }
        format!("request::{}", request.request_id)
    };

    Ok((request_key, payload_hash))
}

fn escape_json_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn set_response_header(response_ptr: *mut u8, name: &str, value: &str) -> *mut u8 {
    unsafe {
        let response = &mut *(response_ptr as *mut MeshHttpResponse);
        let headers = if response.headers.is_null() {
            map::mesh_map_new_typed(1)
        } else {
            response.headers
        };
        response.headers =
            map::mesh_map_put(headers, mesh_str(name) as u64, mesh_str(value) as u64);
    }
    response_ptr
}

fn clustered_route_failure_response(reason: &str, request_key: Option<&str>) -> *mut u8 {
    let body = format!("{{\"error\":\"{}\"}}", escape_json_string(reason));
    let response_ptr = mesh_http_response_new(
        CLUSTERED_ROUTE_FAILURE_STATUS,
        mesh_str(&body) as *const MeshString,
    );
    match request_key {
        Some(request_key) => set_response_header(
            response_ptr,
            CLUSTERED_ROUTE_REQUEST_KEY_HEADER,
            request_key,
        ),
        None => response_ptr,
    }
}

/// A clustered route's response, from wherever in the cluster it runs,
/// once `admission` admits it.
fn clustered_route_response_from_request(
    admission: &Arc<AdmissionController>,
    runtime_name: &str,
    request_ptr: *mut u8,
) -> *mut u8 {
    let _admission = match admission.reserve_application() {
        Ok(permit) => permit,
        Err(rejection) => {
            return clustered_route_failure_response(
                &format!("admission_rejected:{rejection:?}"),
                None,
            );
        }
    };
    let result = encode_http_request_payload(request_ptr).and_then(|request_payload| {
        let (request_key, payload_hash) =
            build_clustered_http_route_identity(runtime_name, &request_payload)?;
        let response_result = crate::dist::node::execute_clustered_http_route(
            runtime_name,
            &request_key,
            &payload_hash,
            &request_payload,
        )
        .and_then(|execution| {
            let response_ptr = decode_http_response_payload(&execution.response_payload)?;
            for (name, value) in [
                (CLUSTERED_ROUTE_INGRESS_HEADER, &*execution.ingress_node),
                (CLUSTERED_ROUTE_EXECUTION_HEADER, &*execution.execution_node),
                (
                    CLUSTERED_ROUTE_REMOTE_HEADER,
                    &execution.routed_remotely.to_string(),
                ),
            ] {
                set_response_header(response_ptr, name, value);
            }
            Ok(if execution.replayed {
                set_response_header(response_ptr, IDEMPOTENCY_REPLAY_HEADER, "true")
            } else {
                response_ptr
            })
        });
        Ok((request_key, response_result))
    });

    match result {
        Ok((request_key, Ok(response_ptr))) => set_response_header(
            response_ptr,
            CLUSTERED_ROUTE_REQUEST_KEY_HEADER,
            &request_key,
        ),
        Ok((request_key, Err(reason))) => {
            clustered_route_failure_response(&reason, Some(&request_key))
        }
        Err(reason) => clustered_route_failure_response(&reason, None),
    }
}

// ── Request accessors ──────────────────────────────────────────────────

/// Get the HTTP method from a request.
#[no_mangle]
pub extern "C" fn mesh_http_request_method(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const MeshHttpRequest)).method }
}

/// Get the URL path from a request.
#[no_mangle]
pub extern "C" fn mesh_http_request_path(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const MeshHttpRequest)).path }
}

/// Get the request body.
#[no_mangle]
pub extern "C" fn mesh_http_request_body(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const MeshHttpRequest)).body }
}

/// Get the byte-exact request body.
#[no_mangle]
pub extern "C" fn mesh_http_request_body_bytes(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const MeshHttpRequest)).body_bytes }
}

/// The value `name` has in `map`, one of a request's string maps, as a
/// MeshOption (tag 0 = Some with MeshString, tag 1 = None): the entry
/// whose key `matches` the name.
fn request_map_value(
    map: *mut u8,
    name: *const MeshString,
    matches: fn(&str, &str) -> bool,
) -> *mut u8 {
    unsafe {
        let name = (*name).as_str();
        let (_, entries) = map::live_entries(map);
        match entries
            .iter()
            .find(|[key, _]| matches((*(*key as *const MeshString)).as_str(), name))
        {
            Some([_, value]) => alloc_option(0, *value as *mut u8),
            None => alloc_option(1, std::ptr::null_mut()),
        }
    }
}

/// Get the value of a request header by name, whose case does not matter:
/// `x-agent` finds `X-Agent`.
#[no_mangle]
pub extern "C" fn mesh_http_request_header(req: *mut u8, name: *const MeshString) -> *mut u8 {
    let request = unsafe { &*(req as *const MeshHttpRequest) };
    request_map_value(request.headers, name, str::eq_ignore_ascii_case)
}

/// Get the value of a query parameter by name.
#[no_mangle]
pub extern "C" fn mesh_http_request_query(req: *mut u8, name: *const MeshString) -> *mut u8 {
    let request = unsafe { &*(req as *const MeshHttpRequest) };
    request_map_value(request.query_params, name, <str as PartialEq>::eq)
}

/// Get the value of a path parameter by name.
///
/// Path parameters are extracted from parameterized route patterns
/// like `/users/:id`. For a request matching this pattern with path
/// `/users/42`, `Request.param(req, "id")` returns `Some("42")`.
#[no_mangle]
pub extern "C" fn mesh_http_request_param(req: *mut u8, name: *const MeshString) -> *mut u8 {
    let request = unsafe { &*(req as *const MeshHttpRequest) };
    request_map_value(request.path_params, name, <str as PartialEq>::eq)
}

/// Return the globally unique request identity assigned at ingress.
#[no_mangle]
pub extern "C" fn mesh_http_request_id(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const MeshHttpRequest)).request_id }
}

/// Return the validated caller idempotency key, when supplied.
#[no_mangle]
pub extern "C" fn mesh_http_idempotency_key(req: *mut u8) -> *mut u8 {
    let key = unsafe { (*(req as *const MeshHttpRequest)).idempotency_key };
    if key.is_null() {
        alloc_option(1, std::ptr::null_mut())
    } else {
        alloc_option(0, key)
    }
}

// ── Option allocation helper (shared from crate::option) ────────────────

fn alloc_option(tag: u8, value: *mut u8) -> *mut u8 {
    crate::option::alloc_option(tag, value) as *mut u8
}

// ── HTTP/1.1 Request Parser ─────────────────────────────────────────────

/// Parsed HTTP/1.1 request with method, path, headers, and body.
#[derive(Debug)]
struct ParsedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

const MAX_HTTP_HEADER_BYTES: usize = 8 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;

fn configure_accepted_stream(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    maximum: usize,
    limit_error: &str,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader
        .take((maximum + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|error| format!("read HTTP line: {error}"))?;
    if bytes.len() > maximum {
        return Err(limit_error.to_string());
    }
    if !bytes.ends_with(b"\n") {
        return Err("unterminated HTTP line".to_string());
    }
    String::from_utf8(bytes).map_err(|_| "HTTP headers must be UTF-8".to_string())
}

/// Parse an HTTP/1.1 request from an `HttpStream` (plain TCP or TLS).
///
/// Uses `BufReader<&mut HttpStream>` so the stream can be reused for
/// writing the response after parsing completes (the BufReader borrows
/// the stream mutably, and the borrow ends when this function returns).
///
/// Limits: max 100 headers, max 8KB total header data, max 1MB body.
fn parse_buffered_request<R: BufRead>(reader: &mut R) -> Result<ParsedRequest, String> {
    let mut total_header_bytes: usize = 0;

    // 1. Read request line: "GET /path HTTP/1.1\r\n"
    let request_line = read_bounded_line(
        reader,
        MAX_HTTP_HEADER_BYTES,
        "request line exceeds 8KB header limit",
    )?;
    total_header_bytes += request_line.len();

    let request_line_trimmed = request_line.trim_end();
    let parts: Vec<&str> = request_line_trimmed.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("malformed request line: {}", request_line_trimmed));
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    // 2. Read headers until blank line (\r\n alone), each line within what
    // the 8KB header section has left.
    let mut headers = Vec::new();
    let mut content_length = None;
    loop {
        let line = read_bounded_line(
            reader,
            MAX_HTTP_HEADER_BYTES - total_header_bytes,
            "header section exceeds 8KB limit",
        )?;
        total_header_bytes += line.len();

        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break; // blank line = end of headers
        }
        if headers.len() >= 100 {
            return Err("too many headers (max 100)".to_string());
        }
        let (name, value) = trimmed
            .split_once(':')
            .ok_or_else(|| "malformed HTTP header".to_string())?;
        let name = name.trim().to_string();
        let value = value.trim().to_string();
        if name.is_empty() {
            return Err("malformed HTTP header".to_string());
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("transfer-encoding is not supported".to_string());
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("duplicate content-length header".to_string());
            }
            let length = value
                .parse::<usize>()
                .map_err(|_| "invalid content-length header".to_string())?;
            if length > MAX_HTTP_BODY_BYTES {
                return Err("request body exceeds 1MB limit".to_string());
            }
            content_length = Some(length);
        }
        headers.push((name, value));
    }

    // 3. Read body based on Content-Length.
    let content_length = content_length.unwrap_or(0);
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

fn parse_request(stream: &mut HttpStream) -> Result<ParsedRequest, String> {
    parse_buffered_request(&mut BufReader::new(stream))
}

// ── HTTP/1.1 Response Writer ────────────────────────────────────────────

/// Headers `write_response` always emits itself so the framing it advertises
/// matches the body it actually writes. Handler-supplied copies are rejected
/// rather than emitted alongside the writer's values.
const WRITER_OWNED_HEADERS: [&str; 3] = ["content-length", "transfer-encoding", "connection"];

/// Reject handler-supplied headers that could split the response or override
/// writer-owned framing. Handlers may copy request data into headers, so this
/// is the shared sink where CR/LF and other control bytes are refused.
///
/// Names must be RFC 9110 tokens. Values may contain HTAB, SP, visible ASCII,
/// and obs-text bytes (non-ASCII UTF-8), but no CR, LF, NUL, or other controls.
fn validate_response_headers(headers: &[(String, String)]) -> Result<(), String> {
    for (name, value) in headers {
        if name.is_empty() || !name.bytes().all(is_header_token_byte) {
            return Err(format!("invalid response header name {name:?}"));
        }
        if WRITER_OWNED_HEADERS
            .iter()
            .any(|owned| name.eq_ignore_ascii_case(owned))
        {
            return Err(format!(
                "response header {name:?} is owned by the response writer"
            ));
        }
        if !value.bytes().all(is_header_value_byte) {
            return Err(format!(
                "response header {name:?} value contains a control byte"
            ));
        }
    }
    Ok(())
}

fn is_header_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn is_header_value_byte(byte: u8) -> bool {
    byte == b'\t' || (byte >= 0x20 && byte != 0x7F)
}

/// A handler's status as a status line's: three digits, or an error.
fn response_status(status: i64) -> Result<u16, String> {
    u16::try_from(status)
        .ok()
        .filter(|status| (100..=999).contains(status))
        .ok_or_else(|| format!("invalid response status {status}"))
}

/// Write an HTTP/1.1 response to an `HttpStream` (plain TCP or TLS).
///
/// Format: status line (the status's standard reason, or none for a status
/// without one), Content-Type, Content-Length, Connection: close, the extra
/// headers, blank line, body bytes. A custom Content-Type replaces the JSON
/// default. The headers are valid ones: `validate_response_headers` passed
/// them, or the runtime wrote them.
fn write_response(
    stream: &mut impl Write,
    status: u16,
    body: &[u8],
    extra_headers: &[(String, String)],
) -> Result<(), String> {
    let reason = ureq::http::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("");
    let content_type = extra_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map_or("application/json; charset=utf-8", |(_, value)| {
            value.as_str()
        });
    let mut header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        if !name.eq_ignore_ascii_case("content-type") {
            header.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    header.push_str("\r\n");

    stream
        .write_all(header.as_bytes())
        .map_err(|e| format!("write response header: {}", e))?;
    stream
        .write_all(body)
        .map_err(|e| format!("write response body: {}", e))?;
    stream.flush().map_err(|e| format!("flush response: {}", e))
}

// ── Actor-per-connection infrastructure ────────────────────────────────

/// What a connection's actor takes over: the router, the connection, and
/// its admission and telemetry permits.
struct ConnectionArgs {
    /// Router address as usize (for Send safety across thread boundaries).
    router_addr: usize,
    stream: HttpStream,
    /// Queue permit released when the actor starts running.
    queue_permit: crate::dist::telemetry::QueuePermit,
    /// Tracks accepted connections and request end-to-end/service latency.
    connection_permit: crate::dist::telemetry::HttpConnectionPermit,
}

/// Actor entry function for handling a single HTTP connection.
///
/// Receives a raw pointer to a boxed `ConnectionArgs`. Wraps the handler
/// call in `catch_unwind` for crash isolation -- a panic in one handler
/// does not affect other connections.
///
/// The read timeout is already set on the underlying TcpStream before
/// wrapping in `HttpStream` (both Plain and Tls variants). For TLS
/// connections, the actual TLS handshake happens lazily on the first
/// `read` call (via `StreamOwned`), which occurs inside this actor --
/// not in the accept loop.
extern "C" fn connection_handler_entry(args: *const u8) {
    let ConnectionArgs {
        router_addr,
        mut stream,
        queue_permit,
        mut connection_permit,
    } = *unsafe { Box::from_raw(args as *mut ConnectionArgs) };
    queue_permit.begin();
    connection_permit.begin_service();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match parse_request(&mut stream) {
            Ok(parsed) => {
                let (status, body, headers) = process_request(router_addr as *mut u8, parsed);
                let checked = response_status(status)
                    .and_then(|status| validate_response_headers(&headers).map(|()| status));
                let _ = match checked {
                    Ok(status) => write_response(&mut stream, status, &body, &headers),
                    Err(error) => {
                        eprintln!("[mesh-rt] HTTP handler response rejected: {}", error);
                        write_response(&mut stream, 500, b"Internal Server Error", &[])
                    }
                };
            }
            Err(e) => {
                eprintln!("[mesh-rt] HTTP parse error: {}", e);
            }
        }
    }));

    if let Err(panic_info) = result {
        eprintln!("[mesh-rt] HTTP handler panicked: {:?}", panic_info);
        let _ = write_response(&mut stream, 500, b"Internal Server Error", &[]);
    }
    drop(connection_permit);
}

// ── Server ─────────────────────────────────────────────────────────────

fn drain_accepted_connections() {
    crate::dist::telemetry::global_admission_controller().set_draining(true);
    let mut connections = crate::dist::telemetry::runtime_telemetry()
        .snapshot()
        .http_connections;
    eprintln!("[mesh-rt] draining {connections} accepted HTTP connections");
    while connections != 0 {
        std::thread::sleep(Duration::from_millis(10));
        connections = crate::dist::telemetry::runtime_telemetry()
            .snapshot()
            .http_connections;
    }
}

/// Start an HTTP server on the given port, blocking the calling thread.
///
/// The server listens for incoming connections and dispatches each
/// request to a lightweight actor via the Mesh actor scheduler. Each
/// connection handler runs as a coroutine (64 KiB stack) with crash
/// isolation via `catch_unwind` in `connection_handler_entry`.
///
/// Handler calling convention (same as closures in collections):
/// - If handler_env is null: `fn(request_ptr) -> response_ptr`
/// - If handler_env is non-null: `fn(handler_env, request_ptr) -> response_ptr`
#[no_mangle]
pub extern "C" fn mesh_http_serve(router: *mut u8, port: i64) {
    serve(router, port, None);
}

// ── HTTPS Server ────────────────────────────────────────────────────────

/// Start an HTTPS server on the given port with TLS, blocking the calling thread.
///
/// Loads PEM-encoded certificate and private key files, builds a rustls
/// `ServerConfig`, and enters the same accept loop as `mesh_http_serve`.
/// Each accepted connection is wrapped in `HttpStream::Tls` and dispatched
/// to a lightweight actor.
///
/// The TLS handshake is lazy: `StreamOwned::new()` does NO I/O. The actual
/// handshake occurs on the first `read` call inside the actor's coroutine,
/// ensuring the accept loop is never blocked by slow TLS clients.
#[no_mangle]
pub extern "C" fn mesh_http_serve_tls(
    router: *mut u8,
    port: i64,
    cert_path: *const MeshString,
    key_path: *const MeshString,
) {
    let (cert_path, key_path) = unsafe { ((*cert_path).as_str(), (*key_path).as_str()) };
    match build_server_config(cert_path, key_path) {
        Ok(tls) => serve(router, port, Some(tls)),
        Err(error) => eprintln!("[mesh-rt] Failed to load TLS certificates: {error}"),
    }
}

/// How long an accept loop with nothing to take waits before it looks
/// again; one that failed waits as long, rather than failing again at once
/// (out of descriptors, say) and again, a thread spinning on its log.
pub(crate) const ACCEPT_PAUSE: Duration = Duration::from_millis(25);

/// The accept loop HTTP.serve and HTTP.serve_tls share, until a shutdown is
/// requested: each accepted connection (in TLS when `tls` is given) is
/// admitted and handled on an actor of its own, or refused.
fn serve(router: *mut u8, port: i64, tls: Option<Arc<ServerConfig>>) {
    // Ensure the actor scheduler is initialized (idempotent).
    crate::actor::mesh_rt_init_actor(0);
    let scheme = if tls.is_some() { "HTTPS" } else { "HTTP" };

    let addr = format!("[::]:{}", port);
    let listener = match std::net::TcpListener::bind(&addr)
        .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
    {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("[mesh-rt] Failed to start {scheme} server on {addr}: {e}");
            return;
        }
    };

    eprintln!("[mesh-rt] {scheme} server listening on {addr}");
    crate::dist::node::mesh_trigger_startup_work();

    let admission = crate::dist::telemetry::global_admission_controller();
    while !crate::process_signal::shutdown_requested() {
        match listener.accept() {
            Ok((stream, _peer)) => admit(stream, router as usize, tls.as_ref(), admission),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    eprintln!("[mesh-rt] accept error: {}", e);
                }
                std::thread::sleep(ACCEPT_PAUSE);
            }
        }
    }
    drain_accepted_connections();
    eprintln!("[mesh-rt] {scheme} server stopped");
}

/// Hand an accepted connection to an actor of its own, as `admission`
/// allows: a refused HTTP connection is answered 503, a refused HTTPS one
/// just closed (answering would take a handshake here, in the accept
/// loop). No I/O happens here for TLS: the handshake happens on the
/// connection's actor, at its first read.
fn admit(
    stream: TcpStream,
    router_addr: usize,
    tls: Option<&Arc<ServerConfig>>,
    admission: &Arc<AdmissionController>,
) {
    if let Err(error) = configure_accepted_stream(&stream) {
        eprintln!("[mesh-rt] failed to configure accepted connection: {error}");
        return;
    }
    let connection_permit = crate::dist::telemetry::runtime_telemetry().begin_http_connection();
    let mut stream = match tls {
        None => HttpStream::Plain(stream),
        Some(config) => HttpStream::Tls(StreamOwned::new(tls_session(config), stream)),
    };
    let queue_permit = match admission.enqueue(1) {
        Ok(permit) => permit,
        Err(_) if tls.is_some() => return,
        Err(rejection) => {
            let _ = write_response(
                &mut stream,
                503,
                format!("admission_rejected:{rejection:?}").as_bytes(),
                &[("Retry-After".to_string(), "1".to_string())],
            );
            return;
        }
    };
    let args = Box::new(ConnectionArgs {
        router_addr,
        stream,
        queue_permit,
        connection_permit,
    });
    actor::global_scheduler().spawn(
        connection_handler_entry as *const u8,
        Box::into_raw(args) as *const u8,
        std::mem::size_of::<ConnectionArgs>() as u64,
        1, // Normal priority
    );
}

// ── Middleware chain infrastructure ──────────────────────────────────

/// Trampoline for the middleware `next` function.
///
/// This is what Mesh calls when middleware invokes `next(request)`, with
/// the request's step through its router's middleware: the next
/// middleware, given the step beside this one as its `next`, or the
/// route's handler once past them all.
///
/// Mesh compiles middleware with signature `fn(request: ptr, next: {ptr, ptr}) -> ptr`;
/// the `next` closure struct `{fn_ptr, env_ptr}` goes as two arguments.
extern "C" fn chain_next(env_ptr: *mut u8, request_ptr: *mut u8) -> *mut u8 {
    let step = unsafe { &*(env_ptr as *const ChainStep) };
    let router = unsafe { &*step.router };
    let Some(middleware) = router.middlewares.get(step.index) else {
        return route_response(step.route.map(|index| &router.routes[index]), request_ptr);
    };
    let next = unsafe { (step as *const ChainStep).add(1) };
    unsafe {
        call3(
            middleware.fn_ptr,
            middleware.env_ptr,
            request_ptr as u64,
            chain_next as *mut u8 as u64,
            next as u64,
        ) as *mut u8
    }
}

/// A request's response from its route's handler: in the cluster for a
/// clustered route, and a 404 for a request no route matched.
fn route_response(route: Option<&RouteEntry>, request_ptr: *mut u8) -> *mut u8 {
    match route {
        Some(RouteEntry {
            declared_handler_runtime_name: Some(runtime_name),
            ..
        }) => clustered_route_response_from_request(
            crate::dist::telemetry::global_admission_controller(),
            runtime_name,
            request_ptr,
        ),
        Some(route) => unsafe {
            call1(route.handler_fn, route.handler_env, request_ptr as u64) as *mut u8
        },
        None => mesh_http_response_new(404, mesh_str("Not Found")),
    }
}

/// Process a single HTTP request by matching it against the router
/// and calling the appropriate handler function.
///
/// Returns the response's status, body and headers (none for a response
/// without).
fn process_request(
    router_ptr: *mut u8,
    parsed: ParsedRequest,
) -> (i64, Vec<u8>, Vec<(String, String)>) {
    let router = unsafe { &*(router_ptr as *const MeshRouter) };
    let (path, query) = parsed.path.split_once('?').unwrap_or((&parsed.path, ""));
    let idempotency_key = parsed
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("idempotency-key"))
        .map(|(_, value)| value.clone());
    if let Some(key) = &idempotency_key {
        if let Err(error) = crate::dist::identity::validate_idempotency_key(key) {
            return (400, error.into_bytes(), Vec::new());
        }
    }
    let matched = router.match_route(path, &parsed.method);
    let has_middleware = !router.middlewares.is_empty();
    if matched.is_none() && !has_middleware {
        return (404, b"Not Found".to_vec(), Vec::new());
    }

    let request_ptr = transport_request_to_mesh(&TransportHttpRequest {
        method: parsed.method.clone(),
        path: path.to_string(),
        body: parsed.body,
        query_params: query
            .split('&')
            .filter_map(|param| param.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        headers: parsed.headers,
        path_params: matched
            .as_ref()
            .map_or_else(Vec::new, |(_, params)| params.clone()),
        request_id: next_request_id(),
        idempotency_key,
    });
    let route = matched.as_ref().map(|(route, _)| *route);
    let response_ptr = if has_middleware {
        let index = route.map(|route| unsafe {
            (route as *const RouteEntry).offset_from(router.routes.as_ptr()) as usize
        });
        chain_next(router.first_step(index) as *mut u8, request_ptr)
    } else {
        route_response(route, request_ptr)
    };
    let response = mesh_response_to_transport(response_ptr);
    (response.status, response.body, response.headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dist::continuity::{continuity_registry, ContinuityPhase, ContinuityResult};
    use crate::dist::node::{
        clear_declared_handler_registry_for_test, declared_handler_registry_test_lock,
        mesh_register_declared_handler,
    };
    use crate::gc::mesh_rt_init;
    use crate::http::router::{mesh_http_route_get, mesh_http_router};
    use crate::string::mesh_string_new;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn bytes_of(ptr: *mut u8) -> Vec<u8> {
        unsafe { (*(ptr as *const MeshBytes)).as_slice().to_vec() }
    }

    fn reset_clustered_runtime_state() {
        clear_declared_handler_registry_for_test();
        continuity_registry().clear_for_test();
    }

    fn owned_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn build_test_request(
        method: &str,
        path: &str,
        body: &str,
        query_params: &[(&str, &str)],
        headers: &[(&str, &str)],
        path_params: &[(&str, &str)],
    ) -> *mut u8 {
        unsafe {
            let req_ptr = mesh_gc_alloc_actor(
                std::mem::size_of::<MeshHttpRequest>() as u64,
                std::mem::align_of::<MeshHttpRequest>() as u64,
            ) as *mut MeshHttpRequest;
            (*req_ptr).method = mesh_str(method) as *mut u8;
            (*req_ptr).path = mesh_str(path) as *mut u8;
            (*req_ptr).body = mesh_str(body) as *mut u8;
            (*req_ptr).query_params = pairs_to_mesh_map(&owned_pairs(query_params));
            (*req_ptr).headers = pairs_to_mesh_map(&owned_pairs(headers));
            (*req_ptr).path_params = pairs_to_mesh_map(&owned_pairs(path_params));
            (*req_ptr).request_id = mesh_str(
                &crate::dist::identity::request_id_generator()
                    .next()
                    .expect("test request id")
                    .to_string(),
            ) as *mut u8;
            (*req_ptr).idempotency_key = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("idempotency-key"))
                .map(|(_, value)| mesh_str(value) as *mut u8)
                .unwrap_or(std::ptr::null_mut());
            (*req_ptr).body_bytes = mesh_bytes_new(body.as_ptr(), body.len() as u64) as *mut u8;
            req_ptr as *mut u8
        }
    }

    fn build_test_response(status: i64, body: &str, headers: &[(&str, &str)]) -> *mut u8 {
        let body_ptr = mesh_str(body) as *const MeshString;
        if headers.is_empty() {
            mesh_http_response_new(status, body_ptr)
        } else {
            let headers_ptr = pairs_to_mesh_map(&owned_pairs(headers));
            mesh_http_response_with_headers(status, body_ptr, headers_ptr)
        }
    }

    fn required_response_header(headers: &[(String, String)], name: &str) -> String {
        let mut matches = headers
            .iter()
            .filter(|(header_name, _)| header_name.eq_ignore_ascii_case(name));
        let value = matches
            .next()
            .unwrap_or_else(|| panic!("missing response header {name} in {headers:?}"))
            .1
            .clone();
        assert!(
            matches.next().is_none(),
            "duplicate response header {name} in {headers:?}"
        );
        assert!(
            !value.is_empty(),
            "response header {name} should not be empty"
        );
        value
    }

    static CLUSTERED_ROUTE_HANDLER_CALLS: AtomicU64 = AtomicU64::new(0);
    static IDEMPOTENCY_HANDLER_CALLS: AtomicU64 = AtomicU64::new(0);

    extern "C" fn clustered_route_test_handler(request: *mut u8) -> *mut u8 {
        CLUSTERED_ROUTE_HANDLER_CALLS.fetch_add(1, Ordering::Relaxed);
        let request = unsafe { &*(request as *const MeshHttpRequest) };
        let body = mesh_string_to_owned(request.body);
        build_test_response(
            200,
            &format!("{{\"echo\":\"{}\"}}", body),
            &[("X-Clustered", "true")],
        )
    }

    extern "C" fn idempotency_test_handler(request: *mut u8) -> *mut u8 {
        IDEMPOTENCY_HANDLER_CALLS.fetch_add(1, Ordering::Relaxed);
        let request = unsafe { &*(request as *const MeshHttpRequest) };
        let body = mesh_string_to_owned(request.body);
        build_test_response(
            200,
            &format!("{{\"echo\":\"{}\"}}", body),
            &[("X-Clustered", "true")],
        )
    }

    #[test]
    fn request_parser_rejects_unbounded_or_ambiguous_input() {
        let parse = |request: String| {
            let mut reader = BufReader::new(std::io::Cursor::new(request.into_bytes()));
            parse_buffered_request(&mut reader)
        };

        let oversized_body = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_HTTP_BODY_BYTES + 1
        );
        assert_eq!(
            parse(oversized_body).unwrap_err(),
            "request body exceeds 1MB limit"
        );

        let oversized_line = format!(
            "GET /{} HTTP/1.1\r\n\r\n",
            "x".repeat(MAX_HTTP_HEADER_BYTES)
        );
        assert_eq!(
            parse(oversized_line).unwrap_err(),
            "request line exceeds 8KB header limit"
        );

        assert_eq!(
            parse("POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx".to_string())
                .unwrap_err(),
            "duplicate content-length header"
        );
        assert_eq!(
            parse("POST / HTTP/1.1\r\nContent-Length: invalid\r\n\r\n".to_string()).unwrap_err(),
            "invalid content-length header"
        );
    }

    #[test]
    fn accepted_connections_wait_for_request_bytes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
        listener
            .set_nonblocking(true)
            .expect("make listener nonblocking");
        let mut client = TcpStream::connect(listener.local_addr().expect("listener address"))
            .expect("connect client");
        let accepted = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::yield_now();
                }
                Err(error) => panic!("accept connection: {error}"),
            }
        };
        accepted
            .set_nonblocking(true)
            .expect("reproduce inherited listener mode");
        configure_accepted_stream(&accepted).expect("configure accepted stream");
        assert_eq!(
            accepted.write_timeout().unwrap(),
            Some(Duration::from_secs(30))
        );

        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            client
                .write_all(b"GET /health HTTP/1.1\r\n\r\n")
                .expect("write delayed request");
        });
        let parsed = parse_request(&mut HttpStream::Plain(accepted)).expect("parse request");
        sender.join().expect("join request writer");

        assert_eq!(parsed.method, "GET");
        assert_eq!(parsed.path, "/health");
    }

    /// Shrink both socket buffers so a peer that stops reading stalls the
    /// writer after a few KiB instead of after the kernel's multi-MiB defaults.
    #[cfg(unix)]
    fn shrink_socket_buffers(stream: &TcpStream) {
        use std::os::unix::io::AsRawFd;
        let size: libc::c_int = 4 * 1024;
        for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
            let rc = unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    option,
                    (&size as *const libc::c_int).cast(),
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            };
            assert_eq!(rc, 0, "setsockopt: {}", std::io::Error::last_os_error());
        }
    }

    /// A client that requests a large response and then never reads must not
    /// pin the connection actor (and its scheduler worker) forever: the write
    /// timeout configured on the accepted socket turns the stalled write into
    /// an error that `write_response` reports to the handler.
    #[cfg(unix)]
    #[test]
    fn stalled_client_cannot_block_response_writes_indefinitely() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let client = TcpStream::connect(listener.local_addr().expect("listener address"))
            .expect("connect client");
        let (accepted, _) = listener.accept().expect("accept connection");
        shrink_socket_buffers(&client);
        shrink_socket_buffers(&accepted);

        configure_accepted_stream(&accepted).expect("configure accepted stream");
        assert_eq!(
            accepted.write_timeout().unwrap(),
            Some(Duration::from_secs(30)),
            "accepted sockets must carry the production write timeout"
        );
        // Keep the test fast: the production value is asserted above, the
        // behaviour under a stalled peer is exercised with a short deadline.
        accepted
            .set_write_timeout(Some(Duration::from_millis(250)))
            .expect("shorten write timeout for the test");

        let body = vec![b'x'; 8 * 1024 * 1024];
        let started = std::time::Instant::now();
        let error = write_response(&mut HttpStream::Plain(accepted), 200, &body, &[])
            .expect_err("a write to a peer that never reads must time out");
        assert!(
            error.starts_with("write response"),
            "timeout must surface through write_response: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "stalled write must fail on the write timeout, took {:?}",
            started.elapsed()
        );
        drop(client);
    }

    #[test]
    fn test_response_creation() {
        mesh_rt_init();
        let body = mesh_string_new(b"Hello".as_ptr(), 5);
        let resp_ptr = mesh_http_response_new(200, body);
        assert!(!resp_ptr.is_null());
        unsafe {
            let resp = &*(resp_ptr as *const MeshHttpResponse);
            assert_eq!(resp.status, 200);
            let body_str = &*(resp.body as *const MeshString);
            assert_eq!(body_str.as_str(), "Hello");
            assert!(resp.headers.is_null());
        }
    }

    #[test]
    fn test_response_with_headers() {
        mesh_rt_init();
        let resp_ptr = build_test_response(429, "{\"retry_after\":60}", &[("Retry-After", "60")]);
        assert!(!resp_ptr.is_null());
        unsafe {
            let resp = &*(resp_ptr as *const MeshHttpResponse);
            assert_eq!(resp.status, 429);
            let body_str = &*(resp.body as *const MeshString);
            assert_eq!(body_str.as_str(), "{\"retry_after\":60}");
            assert!(!resp.headers.is_null());
            assert_eq!(map::mesh_map_size(resp.headers), 1);
        }
    }

    #[test]
    fn response_rejects_header_injection_before_writing() {
        for (name, value) in [
            ("Location", "/ok\r\nSet-Cookie: admin=1"),
            ("X-Test\r\nSet-Cookie", "admin=1"),
            ("Content-Type", "text/plain\r\n\r\ninjected"),
            ("X-Test", "bad\0value"),
            ("Bad Name", "value"),
            ("", "value"),
        ] {
            assert!(
                validate_response_headers(&owned_pairs(&[(name, value)])).is_err(),
                "accepted invalid header {name:?}: {value:?}"
            );
        }
    }

    #[test]
    fn response_accepts_valid_custom_headers() {
        let mut response = Vec::new();
        write_response(
            &mut response,
            429,
            b"{}",
            &owned_pairs(&[
                ("Retry-After", "60"),
                ("X-Reason", "rate\tlimited"),
                ("X-Unicode", "café"),
            ]),
        )
        .unwrap();

        let response = String::from_utf8(response).unwrap();
        assert_eq!(
            validate_response_headers(&owned_pairs(&[
                ("Retry-After", "60"),
                ("X-Reason", "rate\tlimited"),
                ("X-Unicode", "café"),
            ])),
            Ok(())
        );
        assert!(response.starts_with("HTTP/1.1 429 Too Many Requests\r\n"));
        assert!(response.contains("\r\nRetry-After: 60\r\n"));
        assert!(response.contains("\r\nX-Reason: rate\tlimited\r\n"));
        assert!(response.contains("\r\nX-Unicode: café\r\n"));
        assert!(response.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn response_rejects_conflicting_framing_headers() {
        for (name, value) in [
            ("content-length", "0"),
            ("Transfer-Encoding", "chunked"),
            ("Connection", "keep-alive"),
        ] {
            assert!(validate_response_headers(&owned_pairs(&[(name, value)])).is_err());
        }
    }

    #[test]
    fn response_content_type_header_overrides_json_default() {
        let mut response = Vec::new();
        write_response(
            &mut response,
            200,
            b"metric 1\n",
            &owned_pairs(&[("Content-Type", "text/plain; version=0.0.4; charset=utf-8")]),
        )
        .unwrap();

        let response = String::from_utf8(response).unwrap();
        assert_eq!(
            response
                .lines()
                .filter(|line| line.to_ascii_lowercase().starts_with("content-type:"))
                .collect::<Vec<_>>(),
            ["Content-Type: text/plain; version=0.0.4; charset=utf-8"]
        );
    }

    #[test]
    fn test_request_accessors() {
        mesh_rt_init();
        let req = build_test_request(
            "GET",
            "/test",
            "",
            &[],
            &[("Idempotency-Key", "accessor-key")],
            &[],
        );

        unsafe {
            let m = mesh_http_request_method(req);
            let m_str = &*(m as *const MeshString);
            assert_eq!(m_str.as_str(), "GET");

            let p = mesh_http_request_path(req);
            let p_str = &*(p as *const MeshString);
            assert_eq!(p_str.as_str(), "/test");

            let b = mesh_http_request_body(req);
            let b_str = &*(b as *const MeshString);
            assert_eq!(b_str.as_str(), "");

            let request_id = mesh_http_request_id(req);
            let request_id = &*(request_id as *const MeshString);
            assert_eq!(request_id.as_str().len(), 64);

            let key = mesh_http_idempotency_key(req) as *const crate::option::MeshOption;
            assert_eq!((*key).tag, 0);
            assert_eq!(
                (*((*key).value as *const MeshString)).as_str(),
                "accessor-key"
            );
        }
    }

    #[test]
    fn http_request_transport_roundtrip_preserves_method_body_headers_and_params() {
        mesh_rt_init();
        let request_ptr = build_test_request(
            "POST",
            "/todos/42",
            "{\"title\":\"mesh\"}",
            &[("limit", "10")],
            &[("Content-Type", "application/json")],
            &[("id", "42")],
        );

        let encoded = encode_http_request_payload(request_ptr).expect("encode request payload");
        let decoded_ptr = decode_http_request_payload(&encoded).expect("decode request payload");
        let decoded = mesh_request_to_transport(decoded_ptr);

        assert_eq!(decoded.method, "POST");
        assert_eq!(decoded.path, "/todos/42");
        assert_eq!(decoded.body, b"{\"title\":\"mesh\"}");
        assert_eq!(decoded.query_params, owned_pairs(&[("limit", "10")]));
        assert_eq!(
            decoded.headers,
            owned_pairs(&[("Content-Type", "application/json")])
        );
        assert_eq!(decoded.path_params, owned_pairs(&[("id", "42")]));
    }

    #[test]
    fn continuity_replay_requires_a_safe_method_or_caller_idempotency_key() {
        mesh_rt_init();

        let get =
            encode_http_request_payload(build_test_request("GET", "/todos", "", &[], &[], &[]))
                .expect("encode safe request");
        assert!(http_request_payload_is_replay_safe(&get).unwrap());

        let unsafe_post = encode_http_request_payload(build_test_request(
            "POST",
            "/todos",
            "{\"title\":\"one\"}",
            &[],
            &[],
            &[],
        ))
        .expect("encode unsafe request");
        assert!(!http_request_payload_is_replay_safe(&unsafe_post).unwrap());

        let idempotent_post = encode_http_request_payload(build_test_request(
            "POST",
            "/todos",
            "{\"title\":\"one\"}",
            &[],
            &[("Idempotency-Key", "create-todo-1")],
            &[],
        ))
        .expect("encode idempotent request");
        assert!(http_request_payload_is_replay_safe(&idempotent_post).unwrap());
    }

    #[test]
    fn http_response_transport_roundtrip_preserves_status_body_and_headers() {
        mesh_rt_init();
        let response_ptr = build_test_response(201, "{\"created\":true}", &[("X-Test", "yes")]);

        let encoded = encode_http_response_payload(response_ptr).expect("encode response payload");
        let decoded_ptr = decode_http_response_payload(&encoded).expect("decode response payload");
        let decoded = mesh_response_to_transport(decoded_ptr);

        assert_eq!(decoded.status, 201);
        assert_eq!(decoded.body, b"{\"created\":true}");
        assert_eq!(decoded.headers, owned_pairs(&[("X-Test", "yes")]));
    }

    #[test]
    fn http_transport_roundtrip_preserves_non_utf8_bodies() {
        mesh_rt_init();
        let binary = [0, 0xff, 0x80];
        let request_ptr = build_test_request("POST", "/binary", "", &[], &[], &[]);
        unsafe {
            (*(request_ptr as *mut MeshHttpRequest)).body_bytes =
                mesh_bytes_new(binary.as_ptr(), binary.len() as u64) as *mut u8;
        }

        let request_payload = encode_http_request_payload(request_ptr).unwrap();
        let decoded_request = decode_http_request_payload(&request_payload).unwrap();
        assert_eq!(
            bytes_of(mesh_http_request_body_bytes(decoded_request)),
            binary
        );

        let response_ptr =
            mesh_http_response_bytes_new(200, mesh_bytes_new(binary.as_ptr(), binary.len() as u64));
        let response_payload = encode_http_response_payload(response_ptr).unwrap();
        let decoded_response = decode_http_response_payload(&response_payload).unwrap();
        let response = unsafe { &*(decoded_response as *const MeshHttpResponse) };
        assert_eq!(bytes_of(response.body_bytes), binary);
    }

    #[test]
    fn http_transport_rejects_malformed_request_and_response_payloads() {
        assert!(decode_http_request_payload(&[]).is_err());
        assert!(decode_http_response_payload(&[1, 2, 3]).is_err());
    }

    /// A pair count is what the payload claims, not what it holds: a count
    /// of four billion with no pairs after it is refused, and nothing is
    /// allocated for it.
    #[test]
    fn http_transport_refuses_a_pair_count_the_payload_does_not_hold() {
        let mut payload = Vec::new();
        for field in [b"GET".as_slice(), b"/", b""] {
            payload.extend_from_slice(&(field.len() as u32).to_le_bytes());
            payload.extend_from_slice(field);
        }
        payload.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decode_transport_request(&payload).unwrap_err(),
            "mesh_http_transport_request_query_params_key_0_len_missing"
        );
    }

    #[test]
    fn clustered_route_identity_rejects_empty_runtime_name_and_payload() {
        assert!(build_clustered_http_route_identity("", b"payload").is_err());
        assert!(build_clustered_http_route_identity("Api.Todos.handle", b"").is_err());
    }

    #[test]
    fn process_request_attaches_correlation_header_on_clustered_success_and_preserves_handler_headers(
    ) {
        let _guard = declared_handler_registry_test_lock();
        mesh_rt_init();
        reset_clustered_runtime_state();
        CLUSTERED_ROUTE_HANDLER_CALLS.store(0, Ordering::Relaxed);

        let runtime_name = "Api.Todos.handle_list_todos";
        let executable_name = "__declared_route_api_todos_handle_list_todos";
        mesh_register_declared_handler(
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            executable_name.as_ptr(),
            executable_name.len() as u64,
            1,
            clustered_route_test_handler as *const u8,
        );

        let router = mesh_http_router();
        let pattern = mesh_string_new(b"/todos".as_ptr(), 6);
        let router = mesh_http_route_get(
            router,
            pattern,
            clustered_route_test_handler as *mut u8,
            std::ptr::null_mut(),
        );

        let first_request = ParsedRequest {
            method: "GET".to_string(),
            path: "/todos".to_string(),
            headers: vec![("X-Request-Id".to_string(), "first".to_string())],
            body: b"first".to_vec(),
        };
        let second_request = ParsedRequest {
            method: "GET".to_string(),
            path: "/todos".to_string(),
            headers: vec![("X-Request-Id".to_string(), "second".to_string())],
            body: b"second".to_vec(),
        };

        let (first_status, first_body, first_headers) = process_request(router, first_request);
        let (second_status, second_body, second_headers) = process_request(router, second_request);

        assert_eq!(first_status, 200);
        assert_eq!(
            String::from_utf8(first_body).unwrap(),
            "{\"echo\":\"first\"}"
        );
        assert_eq!(second_status, 200);
        assert_eq!(
            String::from_utf8(second_body).unwrap(),
            "{\"echo\":\"second\"}"
        );
        assert_eq!(
            required_response_header(&first_headers, "X-Clustered"),
            "true"
        );
        assert_eq!(
            required_response_header(&second_headers, "X-Clustered"),
            "true"
        );

        let first_request_key =
            required_response_header(&first_headers, CLUSTERED_ROUTE_REQUEST_KEY_HEADER);
        let second_request_key =
            required_response_header(&second_headers, CLUSTERED_ROUTE_REQUEST_KEY_HEADER);
        assert!(
            first_request_key.starts_with("request::"),
            "unexpected first request key: {first_request_key}"
        );
        assert!(
            second_request_key.starts_with("request::"),
            "unexpected second request key: {second_request_key}"
        );
        assert_ne!(first_request_key, second_request_key);
        assert_eq!(CLUSTERED_ROUTE_HANDLER_CALLS.load(Ordering::Relaxed), 2);

        let snapshot = continuity_registry().snapshot();
        assert_eq!(snapshot.records.len(), 2);

        let first_record = snapshot
            .records
            .iter()
            .find(|record| record.request_key == first_request_key)
            .expect("first continuity record should exist");
        assert_eq!(first_record.phase, ContinuityPhase::Completed);
        assert_eq!(first_record.result, ContinuityResult::Succeeded);
        assert_eq!(first_record.declared_handler_runtime_name(), runtime_name);
        assert_eq!(first_record.replication_count, 1);

        let second_record = snapshot
            .records
            .iter()
            .find(|record| record.request_key == second_request_key)
            .expect("second continuity record should exist");
        assert_eq!(second_record.phase, ContinuityPhase::Completed);
        assert_eq!(second_record.result, ContinuityResult::Succeeded);
        assert_eq!(second_record.declared_handler_runtime_name(), runtime_name);
        assert_eq!(second_record.replication_count, 1);

        reset_clustered_runtime_state();
    }

    #[test]
    fn process_request_returns_503_when_requested_replica_capacity_is_unavailable() {
        let _guard = declared_handler_registry_test_lock();
        mesh_rt_init();
        reset_clustered_runtime_state();
        CLUSTERED_ROUTE_HANDLER_CALLS.store(0, Ordering::Relaxed);

        let runtime_name = "Api.Todos.handle_list_todos";
        let executable_name = "__declared_route_api_todos_handle_list_todos";
        mesh_register_declared_handler(
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            executable_name.as_ptr(),
            executable_name.len() as u64,
            3,
            clustered_route_test_handler as *const u8,
        );

        let router = mesh_http_router();
        let pattern = mesh_string_new(b"/todos".as_ptr(), 6);
        let router = mesh_http_route_get(
            router,
            pattern,
            clustered_route_test_handler as *mut u8,
            std::ptr::null_mut(),
        );

        let (status, body, headers) = process_request(
            router,
            ParsedRequest {
                method: "GET".to_string(),
                path: "/todos".to_string(),
                headers: vec![],
                body: Vec::new(),
            },
        );

        assert_eq!(status, 503);
        let request_key = required_response_header(&headers, CLUSTERED_ROUTE_REQUEST_KEY_HEADER);
        assert!(
            request_key.starts_with("request::"),
            "unexpected rejected request key: {request_key}"
        );
        let body = String::from_utf8(body).expect("response body utf8");
        assert!(body.contains("replica_required_unavailable"), "{body}");
        assert_eq!(CLUSTERED_ROUTE_HANDLER_CALLS.load(Ordering::Relaxed), 0);

        let snapshot = continuity_registry().snapshot();
        assert_eq!(snapshot.records.len(), 1);
        let record = &snapshot.records[0];
        assert_eq!(record.request_key, request_key);
        assert_eq!(record.phase, ContinuityPhase::Rejected);
        assert_eq!(record.result, ContinuityResult::Rejected);
        assert!(record.error.starts_with("replica_required_unavailable"));
        assert_eq!(record.declared_handler_runtime_name(), runtime_name);
        assert_eq!(record.replication_count, 3);

        reset_clustered_runtime_state();
    }

    #[test]
    fn idempotency_key_replays_retained_success_without_reexecuting_handler() {
        let _guard = declared_handler_registry_test_lock();
        mesh_rt_init();
        reset_clustered_runtime_state();
        IDEMPOTENCY_HANDLER_CALLS.store(0, Ordering::Relaxed);

        let runtime_name = "Api.Todos.handle_list_todos";
        let executable_name = "__declared_route_api_todos_handle_list_todos";
        mesh_register_declared_handler(
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            executable_name.as_ptr(),
            executable_name.len() as u64,
            1,
            idempotency_test_handler as *const u8,
        );
        let router = mesh_http_router();
        let pattern = mesh_string_new(b"/todos".as_ptr(), 6);
        let router = mesh_http_route_get(
            router,
            pattern,
            idempotency_test_handler as *mut u8,
            std::ptr::null_mut(),
        );
        let request = || ParsedRequest {
            method: "GET".to_string(),
            path: "/todos".to_string(),
            headers: vec![
                ("Idempotency-Key".to_string(), "replay-key-001".to_string()),
                ("X-Request-Id".to_string(), "same".to_string()),
            ],
            body: Vec::new(),
        };

        let (first_status, first_body, first_headers) = process_request(router, request());
        let (second_status, second_body, second_headers) = process_request(router, request());

        assert_eq!(first_status, 200);
        assert_eq!(second_status, 200);
        assert_eq!(first_body, second_body);
        assert_eq!(IDEMPOTENCY_HANDLER_CALLS.load(Ordering::Relaxed), 1);
        assert!(!first_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(IDEMPOTENCY_REPLAY_HEADER)));
        assert_eq!(
            required_response_header(&second_headers, IDEMPOTENCY_REPLAY_HEADER),
            "true"
        );

        reset_clustered_runtime_state();
    }

    #[test]
    fn invoke_route_handler_from_payload_executes_real_handler_boundary() {
        let _guard = declared_handler_registry_test_lock();
        mesh_rt_init();
        CLUSTERED_ROUTE_HANDLER_CALLS.store(0, Ordering::Relaxed);

        let request_ptr = build_test_request(
            "POST",
            "/clustered",
            "payload",
            &[],
            &[("Content-Type", "text/plain")],
            &[],
        );
        let request_payload = encode_http_request_payload(request_ptr).expect("encode request");
        let response_payload = invoke_route_handler_from_payload(
            clustered_route_test_handler as *mut u8,
            &request_payload,
        )
        .expect("invoke clustered handler from payload");
        let response_ptr =
            decode_http_response_payload(&response_payload).expect("decode response");
        let response = mesh_response_to_transport(response_ptr);

        assert_eq!(CLUSTERED_ROUTE_HANDLER_CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{\"echo\":\"payload\"}");
        assert_eq!(response.headers, owned_pairs(&[("X-Clustered", "true")]));
    }

    // ── Transport payloads ───────────────────────────────────────────────

    /// A payload's leading fields, as the encoder writes them.
    fn fields(values: &[&[u8]]) -> Vec<u8> {
        let mut payload = Vec::new();
        for value in values {
            encode_len_prefixed(&mut payload, value, "test").unwrap();
        }
        payload
    }

    /// Every way a request or response payload can be malformed is refused
    /// by name, and payloads from before request IDs and idempotency keys
    /// still decode.
    #[test]
    fn transport_payloads_are_refused_by_what_is_wrong() {
        mesh_rt_init();
        let head = fields(&[b"GET", b"/", b""]);
        let no_pairs = [head.clone(), vec![0; 12]].concat();
        for (payload, error) in [
            (vec![1, 0], "mesh_http_transport_request_method_len_missing"),
            (
                vec![5, 0, 0, 0, b'G'],
                "mesh_http_transport_request_method_truncated",
            ),
            (
                fields(&[&[0xff]]),
                "mesh_http_transport_request_method_invalid_utf8",
            ),
            (
                head.clone(),
                "mesh_http_transport_request_query_params_count_missing",
            ),
            (
                [no_pairs.clone(), fields(&[b"id"]), vec![2]].concat(),
                "mesh_http_transport_idempotency_key_flag_invalid",
            ),
            (
                [
                    no_pairs.clone(),
                    fields(&[b"id"]),
                    vec![1],
                    fields(&[b"bad key"]),
                ]
                .concat(),
                "idempotency_key_invalid_characters",
            ),
            (
                [no_pairs.clone(), fields(&[b"id"]), vec![0, 9]].concat(),
                "mesh_http_transport_request_trailing_bytes",
            ),
        ] {
            assert_eq!(decode_transport_request(&payload).unwrap_err(), error);
        }

        let legacy = decode_transport_request(&no_pairs).unwrap();
        assert!(!legacy.request_id.is_empty());
        assert_eq!(legacy.idempotency_key, None);
        let keyless =
            decode_transport_request(&[no_pairs.clone(), fields(&[b"id"])].concat()).unwrap();
        assert_eq!(
            (keyless.request_id.as_str(), keyless.idempotency_key),
            ("id", None)
        );

        let mut response = encode_transport_response(&TransportHttpResponse {
            status: 200,
            body: Vec::new(),
            headers: Vec::new(),
        })
        .unwrap();
        response.push(0);
        assert_eq!(
            decode_transport_response(&response).unwrap_err(),
            "mesh_http_transport_response_trailing_bytes"
        );
    }

    /// A length field cannot carry 4 GiB or more: the field is too large.
    #[test]
    fn transport_lengths_past_four_gigabytes_are_refused() {
        let mut payload = Vec::new();
        assert_eq!(
            encode_len(&mut payload, u32::MAX as usize + 1, "response_body"),
            Err("mesh_http_transport_response_body_too_large:4294967296".to_string())
        );
        assert!(payload.is_empty());
    }

    /// A clustered route's identity needs a request ID, and a caller key
    /// that is a valid idempotency key; the content type is part of the
    /// payload hash.
    #[test]
    fn clustered_route_identity_refuses_what_it_cannot_key() {
        let request = |headers: &[(&str, &str)], request_id: &str| TransportHttpRequest {
            method: "POST".to_string(),
            path: "/todos".to_string(),
            body: b"{}".to_vec(),
            query_params: Vec::new(),
            headers: owned_pairs(headers),
            path_params: Vec::new(),
            request_id: request_id.to_string(),
            idempotency_key: None,
        };
        let payload = |request: &TransportHttpRequest| encode_transport_request(request).unwrap();
        assert_eq!(
            build_clustered_http_route_identity("Api.Todos.create", &payload(&request(&[], ""))),
            Err("clustered_route_request_id_missing".to_string())
        );
        assert!(build_clustered_http_route_identity(
            "Api.Todos.create",
            &payload(&request(&[("Idempotency-Key", "bad key")], "id"))
        )
        .is_err());
        let (_, json) = build_clustered_http_route_identity(
            "Api.Todos.create",
            &payload(&request(&[("Content-Type", "application/json")], "id")),
        )
        .unwrap();
        let (_, text) = build_clustered_http_route_identity(
            "Api.Todos.create",
            &payload(&request(&[("Content-Type", "text/plain")], "id")),
        )
        .unwrap();
        assert_ne!(json, text);
    }

    /// A clustered route an admission controller refuses, or whose request
    /// cannot be keyed, answers 503 without a request key: no request was
    /// keyed.
    #[test]
    fn a_refused_clustered_route_answers_503() {
        mesh_rt_init();
        let admission = Arc::new(AdmissionController::new(Default::default()));
        let unkeyable = build_test_request(
            "POST",
            "/todos",
            "",
            &[],
            &[("Idempotency-Key", "bad key")],
            &[],
        );
        let response = mesh_response_to_transport(clustered_route_response_from_request(
            &admission,
            "Api.Todos.create",
            unkeyable,
        ));
        assert_eq!(response.status, 503);
        assert_eq!(
            response.body,
            br#"{"error":"idempotency_key_invalid_characters"}"#
        );
        assert!(response.headers.is_empty());

        admission.set_draining(true);
        let request = build_test_request("GET", "/todos", "", &[], &[], &[]);
        let response = mesh_response_to_transport(clustered_route_response_from_request(
            &admission,
            "Api.Todos.list",
            request,
        ));
        assert_eq!(response.status, 503);
        assert_eq!(response.body, br#"{"error":"admission_rejected:Draining"}"#);
        assert!(response.headers.is_empty());
    }

    /// A text response without headers comes back from its payload as one.
    #[test]
    fn a_bare_text_response_survives_its_payload() {
        mesh_rt_init();
        let payload = encode_http_response_payload(build_test_response(204, "", &[])).unwrap();
        let response = decode_http_response_payload(&payload).unwrap();
        assert_eq!(
            mesh_response_to_transport(response),
            TransportHttpResponse {
                status: 204,
                body: Vec::new(),
                headers: Vec::new(),
            }
        );
        assert!(unsafe { (*(response as *const MeshHttpResponse)).headers.is_null() });
    }

    /// A request with an idempotency key that is none is refused, 400.
    #[test]
    fn an_invalid_idempotency_key_is_refused() {
        mesh_rt_init();
        let (status, body, headers) = process_request(
            mesh_http_router(),
            ParsedRequest {
                method: "POST".to_string(),
                path: "/".to_string(),
                headers: owned_pairs(&[("Idempotency-Key", "")]),
                body: Vec::new(),
            },
        );
        assert_eq!(
            (status, body, headers),
            (400, b"idempotency_key_missing".to_vec(), Vec::new())
        );
    }

    extern "C" fn passthrough_middleware(
        request: *mut u8,
        next_fn: *mut u8,
        next_env: *mut u8,
    ) -> *mut u8 {
        unsafe { call1(next_fn, next_env, request as u64) as *mut u8 }
    }

    /// A clustered route behind middleware runs through the chain to the
    /// cluster; a request no route matches goes through the middleware to
    /// a 404, and without middleware straight to one.
    #[test]
    fn middleware_reaches_clustered_routes_and_the_404() {
        let _guard = declared_handler_registry_test_lock();
        mesh_rt_init();
        reset_clustered_runtime_state();
        let runtime_name = "Api.Todos.handle_list_todos";
        let executable_name = "__declared_route_api_todos_handle_list_todos";
        mesh_register_declared_handler(
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            executable_name.as_ptr(),
            executable_name.len() as u64,
            1,
            clustered_route_test_handler as *const u8,
        );
        let bare = mesh_http_route_get(
            mesh_http_router(),
            mesh_string_new(b"/todos".as_ptr(), 6),
            clustered_route_test_handler as *mut u8,
            std::ptr::null_mut(),
        );
        let router = crate::http::router::mesh_http_use_middleware(
            bare,
            passthrough_middleware as *mut u8,
            std::ptr::null_mut(),
        );
        let get = |router, path: &str| {
            process_request(
                router,
                ParsedRequest {
                    method: "GET".to_string(),
                    path: path.to_string(),
                    headers: Vec::new(),
                    body: b"hi".to_vec(),
                },
            )
        };
        let (status, body, headers) = get(router, "/todos");
        assert_eq!((status, body), (200, br#"{"echo":"hi"}"#.to_vec()));
        assert!(
            required_response_header(&headers, CLUSTERED_ROUTE_REQUEST_KEY_HEADER)
                .starts_with("request::")
        );
        assert_eq!(
            get(router, "/missing"),
            (404, b"Not Found".to_vec(), Vec::new())
        );
        assert_eq!(
            get(bare, "/missing"),
            (404, b"Not Found".to_vec(), Vec::new())
        );
        reset_clustered_runtime_state();
    }

    static NEXT_STEPS: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

    extern "C" fn recording_middleware(
        request: *mut u8,
        next_fn: *mut u8,
        next_env: *mut u8,
    ) -> *mut u8 {
        NEXT_STEPS.lock().unwrap().push(next_env as usize);
        unsafe { call1(next_fn, next_env, request as u64) as *mut u8 }
    }

    /// A request's way through middleware takes nothing that outlives it:
    /// every request's `next` is the same step, the router's, where each
    /// request used to box a new one, never freed.
    #[test]
    fn requests_through_middleware_share_their_routers_steps() {
        mesh_rt_init();
        let router = crate::http::router::mesh_http_use_middleware(
            router_of("/plain", empty_headers_handler),
            recording_middleware as *mut u8,
            std::ptr::null_mut(),
        );
        for _ in 0..2 {
            let (status, body, _) = process_request(
                router,
                ParsedRequest {
                    method: "GET".to_string(),
                    path: "/plain".to_string(),
                    headers: Vec::new(),
                    body: Vec::new(),
                },
            );
            assert_eq!((status, body), (200, b"plain".to_vec()));
        }
        let steps = NEXT_STEPS.lock().unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0], steps[1]);
    }

    // ── Requests ─────────────────────────────────────────────────────────

    /// A query or path parameter is found by its exact name, a header by
    /// its name in any case; one not there is None, as is an idempotency
    /// key the request has none of.
    #[test]
    fn request_values_are_options() {
        mesh_rt_init();
        let request = build_test_request(
            "GET",
            "/users/7",
            "",
            &[("page", "2")],
            &[("X-Agent", "mesh")],
            &[("id", "7")],
        );
        let value = |option: *mut u8| unsafe {
            let option = &*(option as *const crate::option::MeshOption);
            (option.tag == 0).then(|| (*(option.value as *const MeshString)).as_str().to_string())
        };
        let name = |name: &str| mesh_str(name) as *const MeshString;
        assert_eq!(
            value(mesh_http_request_query(request, name("page"))).as_deref(),
            Some("2")
        );
        assert_eq!(value(mesh_http_request_query(request, name("Page"))), None);
        assert_eq!(
            value(mesh_http_request_param(request, name("id"))).as_deref(),
            Some("7")
        );
        assert_eq!(value(mesh_http_request_param(request, name("name"))), None);
        assert_eq!(
            value(mesh_http_request_header(request, name("x-agent"))).as_deref(),
            Some("mesh")
        );
        assert_eq!(
            value(mesh_http_request_header(request, name("x-other"))),
            None
        );
        assert_eq!(value(mesh_http_idempotency_key(request)), None);
    }

    /// Each way a request can be malformed or too large is refused by
    /// name, before a handler sees it.
    #[test]
    fn request_parser_refuses_malformed_requests() {
        let parse = |request: Vec<u8>| {
            parse_buffered_request(&mut BufReader::new(std::io::Cursor::new(request))).unwrap_err()
        };
        let many_headers = format!("GET / HTTP/1.1\r\n{}\r\n", "A: b\r\n".repeat(101));
        let wide_headers = format!(
            "GET / HTTP/1.1\r\nA: {}\r\nB: {}\r\n\r\n",
            "x".repeat(4096),
            "y".repeat(4096)
        );
        for (request, error) in [
            (
                b"GET / HTTP/1.1\r\nHost: x".to_vec(),
                "unterminated HTTP line",
            ),
            (b"GET\r\n\r\n".to_vec(), "malformed request line: GET"),
            (many_headers.into_bytes(), "too many headers (max 100)"),
            (
                wide_headers.into_bytes(),
                "header section exceeds 8KB limit",
            ),
            (
                b"GET / HTTP/1.1\r\nno colon\r\n\r\n".to_vec(),
                "malformed HTTP header",
            ),
            (
                b"GET / HTTP/1.1\r\n: empty\r\n\r\n".to_vec(),
                "malformed HTTP header",
            ),
            (
                b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
                "transfer-encoding is not supported",
            ),
            (
                b"GET / HTTP/1.1\r\nA: \xff\r\n\r\n".to_vec(),
                "HTTP headers must be UTF-8",
            ),
        ] {
            assert_eq!(parse(request), error);
        }
        assert!(
            parse(b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\nab".to_vec())
                .starts_with("read body: ")
        );
    }

    /// A status line carries its status's standard reason, or none for a
    /// status without one; a status of more or fewer than three digits is
    /// refused.
    #[test]
    fn statuses_are_written_with_their_reasons() {
        for (status, line) in [
            (202, "HTTP/1.1 202 Accepted\r\n"),
            (403, "HTTP/1.1 403 Forbidden\r\n"),
            (405, "HTTP/1.1 405 Method Not Allowed\r\n"),
            (409, "HTTP/1.1 409 Conflict\r\n"),
            (299, "HTTP/1.1 299 \r\n"),
        ] {
            let mut response = Vec::new();
            write_response(&mut response, status, b"", &[]).unwrap();
            assert!(
                String::from_utf8(response).unwrap().starts_with(line),
                "{status}"
            );
        }
        for status in [99, 1000, -1, 70_000] {
            assert_eq!(
                response_status(status),
                Err(format!("invalid response status {status}"))
            );
        }
        assert_eq!(response_status(100), Ok(100));
    }

    // ── Connections ──────────────────────────────────────────────────────

    /// Read one HTTP response: its head, and its body by Content-Length.
    fn read_response(stream: &mut impl Read) -> (String, Vec<u8>) {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).unwrap();
        (head, body)
    }

    /// A connection admitted to `router` under `admission`: the client end.
    fn connect_admitted(
        router: *mut u8,
        tls: Option<&Arc<ServerConfig>>,
        admission: &Arc<AdmissionController>,
    ) -> TcpStream {
        crate::actor::mesh_rt_init_actor(0);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let (accepted, _) = listener.accept().unwrap();
        admit(accepted, router as usize, tls, admission);
        client
    }

    fn router_of(path: &str, handler: extern "C" fn(*mut u8) -> *mut u8) -> *mut u8 {
        mesh_http_route_get(
            mesh_http_router(),
            mesh_str(path) as *const MeshString,
            handler as *mut u8,
            std::ptr::null_mut(),
        )
    }

    extern "C" fn invalid_header_handler(_request: *mut u8) -> *mut u8 {
        build_test_response(200, "ok", &[("Content-Length", "0")])
    }

    extern "C" fn invalid_status_handler(_request: *mut u8) -> *mut u8 {
        build_test_response(70_000, "ok", &[])
    }

    extern "C-unwind" fn panicking_handler(_request: *mut u8) -> *mut u8 {
        panic!("handler failed");
    }

    extern "C" fn empty_headers_handler(_request: *mut u8) -> *mut u8 {
        mesh_http_response_with_headers(
            200,
            mesh_str("plain") as *const MeshString,
            map::mesh_map_new_typed(1),
        )
    }

    /// A handler's response with a header the writer owns, or a status that
    /// is no status, or a handler that panics, is answered 500 instead; a
    /// response with an empty headers map is written without extra headers.
    #[test]
    fn connections_answer_500_for_responses_that_cannot_be_written() {
        let admission = Arc::new(AdmissionController::new(Default::default()));
        let panicking = mesh_http_route_get(
            mesh_http_router(),
            mesh_str("/bad") as *const MeshString,
            panicking_handler as *mut u8,
            std::ptr::null_mut(),
        );
        for router in [
            router_of("/bad", invalid_header_handler),
            router_of("/bad", invalid_status_handler),
            panicking,
        ] {
            let mut client = connect_admitted(router, None, &admission);
            client.write_all(b"GET /bad HTTP/1.1\r\n\r\n").unwrap();
            let (head, body) = read_response(&mut client);
            assert!(
                head.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
                "{head}"
            );
            assert_eq!(body, b"Internal Server Error");
        }
        let mut client =
            connect_admitted(router_of("/plain", empty_headers_handler), None, &admission);
        client.write_all(b"GET /plain HTTP/1.1\r\n\r\n").unwrap();
        let (head, body) = read_response(&mut client);
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert_eq!(body, b"plain");
    }

    /// A request that does not parse gets no answer: the connection closes.
    #[test]
    fn a_malformed_request_closes_the_connection() {
        let admission = Arc::new(AdmissionController::new(Default::default()));
        let mut client = connect_admitted(router_of("/", empty_headers_handler), None, &admission);
        client.write_all(b"GET\r\n\r\n").unwrap();
        assert_eq!(client.read(&mut [0u8; 1]).unwrap(), 0);
    }

    /// A connection the admission controller refuses is answered 503 over
    /// HTTP, and closed unanswered over HTTPS.
    #[test]
    fn refused_connections_are_answered_503_or_closed() {
        let admission = Arc::new(AdmissionController::new(Default::default()));
        admission.set_draining(true);
        let router = router_of("/", empty_headers_handler);
        let mut client = connect_admitted(router, None, &admission);
        let (head, body) = read_response(&mut client);
        assert!(
            head.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
            "{head}"
        );
        assert!(head.contains("\r\nRetry-After: 1\r\n"), "{head}");
        assert_eq!(body, b"admission_rejected:Draining");

        let _ = rustls::crypto::ring::default_provider().install_default();
        let (server_config, _) = crate::dist::node::ws_test_tls_configs();
        let mut client = connect_admitted(router, Some(&server_config), &admission);
        assert_eq!(client.read(&mut [0u8; 1]).unwrap(), 0);
    }

    /// An HTTPS connection's request and response go through TLS.
    #[test]
    fn https_connections_are_served_over_tls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (server_config, client_config) = crate::dist::node::ws_test_tls_configs();
        let admission = Arc::new(AdmissionController::new(Default::default()));
        let tcp = connect_admitted(
            router_of("/plain", empty_headers_handler),
            Some(&server_config),
            &admission,
        );
        let session = rustls::ClientConnection::new(
            client_config,
            rustls_pki_types::ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();
        let mut client = StreamOwned::new(session, tcp);
        client.write_all(b"GET /plain HTTP/1.1\r\n\r\n").unwrap();
        let (head, body) = read_response(&mut client);
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert_eq!(body, b"plain");
    }

    /// HTTP.serve on a port another socket holds says so and returns.
    #[test]
    fn serving_on_a_taken_port_returns() {
        let taken = std::net::TcpListener::bind("[::]:0").unwrap();
        mesh_http_serve(
            mesh_http_router(),
            i64::from(taken.local_addr().unwrap().port()),
        );
    }
}
