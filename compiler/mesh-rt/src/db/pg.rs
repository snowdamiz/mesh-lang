//! PostgreSQL wire protocol v3 client for the Mesh runtime.
//!
//! Provides four extern "C" functions that Mesh programs call to interact
//! with PostgreSQL databases:
//! - `mesh_pg_connect`: Connect to a PostgreSQL server via URL
//! - `mesh_pg_close`: Close a connection
//! - `mesh_pg_execute`: Execute a write query (INSERT/UPDATE/DELETE/CREATE)
//! - `mesh_pg_query`: Execute a read query (SELECT), returns rows
//!
//! Connection handles are opaque u64 values (Box::into_raw as u64) for GC
//! safety. The GC never traces integer values, so the connection won't be
//! corrupted by garbage collection.
//!
//! Authentication supports both SCRAM-SHA-256 (production/cloud) and MD5
//! (local development). The wire protocol is implemented from scratch using
//! `std::net::TcpStream` and crypto crates from the RustCrypto project.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use pbkdf2::pbkdf2_hmac;
use rand::Rng;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::ServerName;
use sha2::Sha256;

use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::collections::list::{
    mesh_list_append, mesh_list_from_array, mesh_list_get, mesh_list_length, mesh_list_new,
};
use crate::collections::map::{mesh_map_from_string_entries, mesh_map_new_typed, mesh_map_put};
use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};

type HmacSha256 = Hmac<Sha256>;

// ponytail: fixed safety caps; make these pool options if legitimate workloads need larger cells.
const MAX_DB_VALUE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PG_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PG_RESULT_BYTES: usize = 64 * 1024 * 1024;
const MAX_PG_VALUES: usize = i16::MAX as usize;
const MAX_PG_ROWS: usize = 100_000;
pub(crate) const DB_VALUE_TEXT: u8 = 0;
pub(crate) const DB_VALUE_BINARY: u8 = 1;
pub(crate) const DB_VALUE_NULL: u8 = 2;

/// ABI mirror of the compiler's `{ Text(String), Binary(Bytes), Null }` sum.
#[repr(C)]
pub struct MeshDbValue {
    pub tag: u8,
    pub payload: *mut u8,
}

// ── Stream Abstraction ─────────────────────────────────────────────────

/// A PostgreSQL connection stream that may be plain TCP or TLS-wrapped.
enum PgStream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
}

impl Read for PgStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            PgStream::Plain(s) => s.read(buf),
            PgStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for PgStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            PgStream::Plain(s) => s.write(buf),
            PgStream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            PgStream::Plain(s) => s.flush(),
            PgStream::Tls(s) => s.flush(),
        }
    }
}

/// Wrapper around a (possibly TLS-wrapped) stream to a PostgreSQL server.
pub(super) struct PgConn {
    stream: PgStream,
    /// Transaction status byte from the most recent ReadyForQuery message.
    /// b'I' = idle (not in transaction), b'T' = in transaction block,
    /// b'E' = in a failed transaction block. Updated on every ReadyForQuery.
    pub(super) txn_status: u8,
    broken: bool,
}

impl PgConn {
    fn write_wire_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if self.broken {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "PostgreSQL connection is unusable",
            ));
        }
        let result = self.stream.write_all(bytes);
        if result.is_err() {
            self.broken = true;
        }
        result
    }

    pub(super) fn read_wire_message(&mut self) -> Result<(u8, Vec<u8>), String> {
        if self.broken {
            return Err("PostgreSQL connection is unusable".to_string());
        }
        let result = read_message(&mut self.stream);
        if result.is_err() {
            self.broken = true;
        }
        result
    }

    pub(super) fn is_broken(&self) -> bool {
        self.broken
    }

    #[cfg(test)]
    pub(super) fn from_test_stream(stream: TcpStream) -> Self {
        Self {
            stream: PgStream::Plain(stream),
            txn_status: b'I',
            broken: false,
        }
    }
}

// ── SSL Mode ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum SslMode {
    Disable,
    Prefer,
    Require,
}

// ── URL Parsing ────────────────────────────────────────────────────────

/// Parsed PostgreSQL connection URL components.
struct PgUrl {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
    sslmode: SslMode,
}

/// Percent-decode a URL component (handles %XX sequences).
fn percent_decode(s: &str) -> String {
    let mut result = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

/// Parse the sslmode query parameter from a URL query string.
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
    SslMode::Prefer
}

/// Parse a `postgres://user:pass@host:port/database?sslmode=prefer` URL.
fn parse_pg_url(url: &str) -> Result<PgUrl, String> {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or_else(|| "URL must start with postgres:// or postgresql://".to_string())?;

    // Split off query string before parsing host/credentials
    let (rest, query_str) = if let Some((r, q)) = rest.split_once('?') {
        (r, q)
    } else {
        (rest, "")
    };
    let sslmode = parse_sslmode(query_str);

    // Split on '@' to separate credentials from host
    let (creds, host_part) = rest
        .split_once('@')
        .ok_or_else(|| "URL missing '@' separator".to_string())?;

    // Parse credentials: user:password
    let (user, password) = if let Some((u, p)) = creds.split_once(':') {
        (percent_decode(u), percent_decode(p))
    } else {
        (percent_decode(creds), String::new())
    };

    // Parse host:port/database
    let (host_port, database) = if let Some((hp, db)) = host_part.split_once('/') {
        (hp, percent_decode(db))
    } else {
        (host_part, user.clone()) // default database = username
    };

    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        let port = p
            .parse::<u16>()
            .map_err(|_| format!("invalid port: {}", p))?;
        (h.to_string(), port)
    } else {
        (host_port.to_string(), 5432)
    };

    Ok(PgUrl {
        host,
        port,
        user,
        password,
        database,
        sslmode,
    })
}

// ── Wire Protocol Helpers ──────────────────────────────────────────────

/// Write a StartupMessage to a buffer.
/// Format: Int32(length) Int32(196608=v3.0) String("user") String(username)
///         String("database") String(dbname) Byte1(0)
fn write_startup_message(buf: &mut Vec<u8>, user: &str, database: &str) {
    let mut body = Vec::new();
    // Protocol version 3.0 = 196608 = 0x00030000
    body.extend_from_slice(&196608_i32.to_be_bytes());
    body.extend_from_slice(b"user\0");
    body.extend_from_slice(user.as_bytes());
    body.push(0);
    body.extend_from_slice(b"database\0");
    body.extend_from_slice(database.as_bytes());
    body.push(0);
    // Terminator
    body.push(0);

    let len = (body.len() + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&body);
}

/// Write a Parse message: Byte1('P') Int32(len) String("") String(query) Int16(0)
fn write_parse(buf: &mut Vec<u8>, query: &str) {
    let mut body = Vec::new();
    body.push(0); // unnamed statement
    body.extend_from_slice(query.as_bytes());
    body.push(0); // null-terminate query
    body.extend_from_slice(&0_i16.to_be_bytes()); // 0 parameter type OIDs

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

#[derive(Clone, Copy)]
enum BindValue<'a> {
    Text(&'a [u8]),
    Binary(&'a [u8]),
    Null,
}

/// Write a Bind message with per-value parameter and result formats.
fn write_bind_values(
    buf: &mut Vec<u8>,
    params: &[BindValue<'_>],
    result_formats: &[i16],
) -> Result<(), String> {
    if params.len() > MAX_PG_VALUES {
        return Err(format!(
            "too many PostgreSQL parameters: {} (maximum {})",
            params.len(),
            MAX_PG_VALUES
        ));
    }
    if result_formats.len() > MAX_PG_VALUES {
        return Err(format!(
            "too many PostgreSQL result columns: {} (maximum {})",
            result_formats.len(),
            MAX_PG_VALUES
        ));
    }
    for format in result_formats {
        if !matches!(format, 0 | 1) {
            return Err(format!("invalid PostgreSQL format code: {format}"));
        }
    }

    // Preflight the complete frame before copying any parameter bytes. This keeps
    // a valid set of individually bounded values from growing an oversized buffer.
    let mut body_len = 8_usize
        .checked_add(
            params
                .len()
                .checked_mul(2)
                .ok_or_else(|| "PostgreSQL Bind message size calculation overflowed".to_string())?,
        )
        .and_then(|len| len.checked_add(result_formats.len().checked_mul(2)?))
        .ok_or_else(|| "PostgreSQL Bind message size calculation overflowed".to_string())?;
    for (index, param) in params.iter().enumerate() {
        let value_len = match param {
            BindValue::Text(bytes) | BindValue::Binary(bytes) => {
                if bytes.len() > MAX_DB_VALUE_BYTES {
                    return Err(format!(
                        "PostgreSQL parameter at index {index} exceeds {MAX_DB_VALUE_BYTES} byte limit"
                    ));
                }
                bytes.len()
            }
            BindValue::Null => 0,
        };
        body_len = body_len
            .checked_add(4)
            .and_then(|len| len.checked_add(value_len))
            .ok_or_else(|| "PostgreSQL Bind message size calculation overflowed".to_string())?;
    }
    let message_len = body_len
        .checked_add(4)
        .filter(|len| *len <= MAX_PG_MESSAGE_BYTES)
        .ok_or_else(|| {
            format!(
                "PostgreSQL Bind message exceeds {} byte limit",
                MAX_PG_MESSAGE_BYTES
            )
        })?;

    let mut body = Vec::with_capacity(body_len);
    body.push(0); // unnamed portal
    body.push(0); // unnamed statement
    body.extend_from_slice(&(params.len() as i16).to_be_bytes());
    for param in params {
        body.extend_from_slice(
            &match param {
                BindValue::Binary(_) => 1_i16,
                BindValue::Text(_) | BindValue::Null => 0_i16,
            }
            .to_be_bytes(),
        );
    }

    body.extend_from_slice(&(params.len() as i16).to_be_bytes());
    for param in params {
        match param {
            BindValue::Text(bytes) | BindValue::Binary(bytes) => {
                body.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
                body.extend_from_slice(bytes);
            }
            BindValue::Null => body.extend_from_slice(&(-1_i32).to_be_bytes()),
        }
    }

    body.extend_from_slice(&(result_formats.len() as i16).to_be_bytes());
    for format in result_formats {
        body.extend_from_slice(&format.to_be_bytes());
    }
    debug_assert_eq!(body.len(), body_len);

    buf.push(b'B');
    buf.extend_from_slice(&(message_len as i32).to_be_bytes());
    buf.extend_from_slice(&body);
    Ok(())
}

/// Write Describe (Portal) message: Byte1('D') Int32(len) Byte1('P') String("")
fn write_describe_portal(buf: &mut Vec<u8>) {
    buf.push(b'D');
    let len = (4 + 1 + 1) as i32; // length_field + 'P' + null byte
    buf.extend_from_slice(&len.to_be_bytes());
    buf.push(b'P'); // Portal variant
    buf.push(0); // unnamed portal
}

/// Write Describe (Statement) for the unnamed prepared statement.
fn write_describe_statement(buf: &mut Vec<u8>) {
    buf.push(b'D');
    buf.extend_from_slice(&6_i32.to_be_bytes());
    buf.push(b'S');
    buf.push(0);
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

/// Write a PasswordMessage: Byte1('p') Int32(len) String(password)
fn write_password_message(buf: &mut Vec<u8>, password: &str) {
    buf.push(b'p');
    let len = (password.len() + 1 + 4) as i32; // string + null + length field
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(password.as_bytes());
    buf.push(0);
}

/// Write SASLInitialResponse: Byte1('p') Int32(len) String(mechanism) Int32(data_len) Bytes(data)
fn write_sasl_initial_response(buf: &mut Vec<u8>, mechanism: &str, data: &[u8]) {
    let mut body = Vec::new();
    body.extend_from_slice(mechanism.as_bytes());
    body.push(0); // null-terminate mechanism
    body.extend_from_slice(&(data.len() as i32).to_be_bytes());
    body.extend_from_slice(data);

    buf.push(b'p');
    let len = (body.len() + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&body);
}

/// Write SASLResponse: Byte1('p') Int32(len) Bytes(data)
fn write_sasl_response(buf: &mut Vec<u8>, data: &[u8]) {
    buf.push(b'p');
    let len = (data.len() + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(data);
}

/// Write Terminate message: Byte1('X') Int32(4)
fn write_terminate(buf: &mut Vec<u8>) {
    buf.push(b'X');
    buf.extend_from_slice(&4_i32.to_be_bytes());
}

// ── TLS Negotiation ────────────────────────────────────────────────────

/// Upgrade a TCP stream to a TLS-wrapped stream using rustls.
fn upgrade_to_tls(
    stream: TcpStream,
    hostname: &str,
) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
    let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let server_name = ServerName::try_from(hostname.to_string())
        .map_err(|_| format!("invalid hostname for TLS: {}", hostname))?;
    let conn = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("TLS connection: {}", e))?;
    Ok(StreamOwned::new(conn, stream))
}

/// Perform PostgreSQL SSLRequest handshake and return a PgStream.
///
/// For sslmode=disable, returns Plain immediately.
/// For prefer/require, sends the SSLRequest message and reads the 1-byte response.
/// 'S' = server accepts SSL -> upgrade to TLS.
/// 'N' = server declines -> error on require, fallback on prefer.
fn negotiate_tls(
    mut stream: TcpStream,
    hostname: &str,
    sslmode: SslMode,
) -> Result<PgStream, String> {
    if sslmode == SslMode::Disable {
        return Ok(PgStream::Plain(stream));
    }

    // SSLRequest: Int32(8) Int32(80877103) -- no message type byte
    let mut ssl_request = [0u8; 8];
    ssl_request[0..4].copy_from_slice(&8_i32.to_be_bytes());
    ssl_request[4..8].copy_from_slice(&80877103_i32.to_be_bytes());
    stream
        .write_all(&ssl_request)
        .map_err(|e| format!("send SSLRequest: {}", e))?;

    // Read exactly 1 byte response (CVE-2021-23222: do NOT read more)
    let mut response = [0u8; 1];
    stream
        .read_exact(&mut response)
        .map_err(|e| format!("read SSL response: {}", e))?;

    match response[0] {
        b'S' => {
            let tls = upgrade_to_tls(stream, hostname)?;
            Ok(PgStream::Tls(tls))
        }
        b'N' => match sslmode {
            SslMode::Require => Err("server does not support SSL".to_string()),
            SslMode::Prefer => Ok(PgStream::Plain(stream)),
            SslMode::Disable => unreachable!(),
        },
        other => Err(format!("unexpected SSL response: 0x{:02x}", other)),
    }
}

// ── Wire Protocol Helpers (Message Reading) ────────────────────────────

/// Read a single message from the server: returns (tag_byte, body_bytes).
fn read_message(stream: &mut PgStream) -> Result<(u8, Vec<u8>), String> {
    let mut tag = [0u8; 1];
    stream
        .read_exact(&mut tag)
        .map_err(|e| format!("read tag: {}", e))?;

    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|e| format!("read length: {}", e))?;
    let len = i32::from_be_bytes(len_buf);

    if len < 4 || len as usize > MAX_PG_MESSAGE_BYTES {
        return Err(format!(
            "invalid PostgreSQL message length: {len} (maximum {MAX_PG_MESSAGE_BYTES})"
        ));
    }

    let body_len = len as usize - 4;
    let mut body = vec![0u8; body_len];
    if body_len > 0 {
        stream
            .read_exact(&mut body)
            .map_err(|e| format!("read body: {}", e))?;
    }

    Ok((tag[0], body))
}

// ── Authentication ─────────────────────────────────────────────────────

/// Compute MD5 password hash.
/// Formula: "md5" + hex(md5(hex(md5(password + username)) + salt_4_bytes))
fn compute_md5_password(user: &str, password: &str, salt: &[u8]) -> String {
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

/// Generate SCRAM-SHA-256 client-first-message and return (message, nonce).
///
/// Uses empty `n=` (no username in SASL) because PostgreSQL already knows
/// the username from the StartupMessage. This matches libpq behavior and
/// ensures the client-first-bare used in the AuthMessage computation is
/// consistent with what the server sees.
fn scram_client_first(_username: &str) -> (String, String) {
    let nonce: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(24)
        .map(char::from)
        .collect();

    let bare = format!("n=,r={}", nonce);
    let message = format!("n,,{}", bare);
    (message, nonce)
}

/// Compute HMAC-SHA-256.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Compute SHA-256 hash.
fn sha256(data: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Process SCRAM-SHA-256 client-final-message.
/// Returns (client_final_message, expected_server_signature).
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
        if let Some(v) = part.strip_prefix("r=") {
            server_nonce = v;
        }
        if let Some(v) = part.strip_prefix("s=") {
            salt_b64 = v;
        }
        if let Some(v) = part.strip_prefix("i=") {
            iterations = v.parse().map_err(|_| "bad iteration count".to_string())?;
        }
    }

    // Verify server nonce starts with client nonce
    if !server_nonce.starts_with(client_nonce) {
        return Err("server nonce mismatch".to_string());
    }

    let salt = BASE64
        .decode(salt_b64)
        .map_err(|_| "bad salt encoding".to_string())?;

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
    let auth_message = format!(
        "{},{},{}",
        client_first_bare, server_first, client_final_without_proof
    );

    // ClientSignature = HMAC(StoredKey, AuthMessage)
    let client_signature = hmac_sha256(&stored_key, auth_message.as_bytes());
    // ClientProof = ClientKey XOR ClientSignature
    let proof: Vec<u8> = client_key
        .iter()
        .zip(client_signature.iter())
        .map(|(a, b)| a ^ b)
        .collect();

    // ServerKey = HMAC(SaltedPassword, "Server Key")
    let server_key = hmac_sha256(&salted_password, b"Server Key");
    // ServerSignature = HMAC(ServerKey, AuthMessage)
    let server_signature = hmac_sha256(&server_key, auth_message.as_bytes());

    let client_final = format!("{},p={}", client_final_without_proof, BASE64.encode(&proof));
    Ok((client_final, server_signature))
}

// ── Error Response Parsing ─────────────────────────────────────────────

/// Structured PostgreSQL error extracted from an ErrorResponse message.
///
/// Contains SQLSTATE code, human-readable message, and optional constraint/table/column info
/// for mapping database constraint violations to user-friendly changeset errors.
pub(crate) struct PgError {
    pub sqlstate: String, // 'C' field (e.g., "23505")
    pub message: String,  // 'M' field
    #[allow(dead_code)]
    pub detail: Option<String>, // 'D' field
    pub constraint: Option<String>, // 'n' field
    pub table: Option<String>, // 't' field
    pub column: Option<String>, // 'c' field
}

/// Parse all tagged fields from an ErrorResponse body.
///
/// The body format is: `[field_type_byte][null_terminated_string]...[0]`
/// This is the same format the existing `parse_error_response()` uses but extracts
/// additional fields beyond just the message ('M').
pub(crate) fn parse_error_response_full(body: &[u8]) -> PgError {
    let mut sqlstate = String::new();
    let mut message = String::new();
    let mut detail: Option<String> = None;
    let mut constraint: Option<String> = None;
    let mut table: Option<String> = None;
    let mut column: Option<String> = None;

    let mut i = 0;
    while i < body.len() {
        let field_type = body[i];
        i += 1;
        if field_type == 0 {
            break;
        }
        // Find the null terminator for the value string
        let start = i;
        while i < body.len() && body[i] != 0 {
            i += 1;
        }
        let value = String::from_utf8_lossy(&body[start..i]).into_owned();
        match field_type {
            b'C' => sqlstate = value,
            b'M' => message = value,
            b'D' => detail = Some(value),
            b'n' => constraint = Some(value),
            b't' => table = Some(value),
            b'c' => column = Some(value),
            _ => {} // skip other fields (S, V, P, q, W, etc.)
        }
        i += 1; // skip null terminator
    }

    if message.is_empty() {
        message = "unknown PostgreSQL error".to_string();
    }

    PgError {
        sqlstate,
        message,
        detail,
        constraint,
        table,
        column,
    }
}

/// Extract the human-readable message from an ErrorResponse body.
/// Format: sequence of (Byte1 field_type, String value) pairs, terminated by Byte1(0).
/// Field 'M' = human-readable message.
fn parse_error_response(body: &[u8]) -> String {
    parse_error_response_full(body).message
}

/// Format a PgError into a tab-separated structured error string.
///
/// Format: `{sqlstate}\t{constraint}\t{table}\t{column}\t{message}`
///
/// This structured string is used by Repo changeset functions to extract
/// SQLSTATE and constraint info for mapping to changeset errors.
fn format_pg_error_string(pg_err: &PgError) -> String {
    let constraint_str = pg_err.constraint.as_deref().unwrap_or("");
    let table_str = pg_err.table.as_deref().unwrap_or("");
    let column_str = pg_err.column.as_deref().unwrap_or("");
    format!(
        "{}\t{}\t{}\t{}\t{}",
        pg_err.sqlstate, constraint_str, table_str, column_str, pg_err.message
    )
}

#[derive(Debug, PartialEq, Eq)]
struct PgColumn {
    name: String,
    oid: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum RowValue<'a> {
    Text(&'a str),
    Binary(&'a [u8]),
    Null,
}

fn parse_row_description(body: &[u8]) -> Result<Vec<PgColumn>, String> {
    if body.len() < 2 {
        return Err("invalid PostgreSQL RowDescription".to_string());
    }
    let count = i16::from_be_bytes([body[0], body[1]]);
    if count < 0 || count as usize > MAX_PG_VALUES {
        return Err(format!("invalid PostgreSQL column count: {count}"));
    }

    let mut offset = 2;
    let mut columns = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name_len = body
            .get(offset..)
            .and_then(|remaining| remaining.iter().position(|byte| *byte == 0))
            .ok_or_else(|| "unterminated PostgreSQL column name".to_string())?;
        let name_end = offset + name_len;
        let name = std::str::from_utf8(&body[offset..name_end])
            .map_err(|_| "PostgreSQL column name is not UTF-8".to_string())?
            .to_string();
        offset = name_end + 1;
        let fields = body
            .get(offset..offset + 18)
            .ok_or_else(|| "truncated PostgreSQL RowDescription".to_string())?;
        let oid = u32::from_be_bytes([fields[6], fields[7], fields[8], fields[9]]);
        columns.push(PgColumn { name, oid });
        offset += 18;
    }
    if offset != body.len() {
        return Err("trailing bytes in PostgreSQL RowDescription".to_string());
    }
    Ok(columns)
}

fn parse_typed_row<'a>(body: &'a [u8], columns: &[PgColumn]) -> Result<Vec<RowValue<'a>>, String> {
    if body.len() < 2 {
        return Err("invalid PostgreSQL DataRow".to_string());
    }
    let count = i16::from_be_bytes([body[0], body[1]]);
    if count < 0 || count as usize != columns.len() {
        return Err(format!(
            "PostgreSQL row has {count} columns; expected {}",
            columns.len()
        ));
    }

    let mut offset = 2;
    let mut values = Vec::with_capacity(columns.len());
    for column in columns {
        let length_bytes = body
            .get(offset..offset + 4)
            .ok_or_else(|| "truncated PostgreSQL DataRow length".to_string())?;
        let length = i32::from_be_bytes([
            length_bytes[0],
            length_bytes[1],
            length_bytes[2],
            length_bytes[3],
        ]);
        offset += 4;
        if length == -1 {
            values.push(RowValue::Null);
            continue;
        }
        if length < 0 || length as usize > MAX_DB_VALUE_BYTES {
            return Err(format!(
                "PostgreSQL column `{}` exceeds {MAX_DB_VALUE_BYTES} byte limit",
                column.name
            ));
        }
        let end = offset
            .checked_add(length as usize)
            .filter(|end| *end <= body.len())
            .ok_or_else(|| format!("truncated PostgreSQL column `{}`", column.name))?;
        let bytes = &body[offset..end];
        offset = end;
        if column.oid == 17 {
            values.push(RowValue::Binary(bytes));
        } else {
            values.push(RowValue::Text(std::str::from_utf8(bytes).map_err(
                |_| format!("PostgreSQL text column `{}` is not UTF-8", column.name),
            )?));
        }
    }
    if offset != body.len() {
        return Err("trailing bytes in PostgreSQL DataRow".to_string());
    }
    Ok(values)
}

fn validate_typed_sql(sql: &str) -> Result<(), String> {
    if sql.as_bytes().contains(&0) {
        return Err("PostgreSQL query contains a NUL byte".to_string());
    }
    if sql
        .len()
        .checked_add(9)
        .filter(|len| *len <= MAX_PG_MESSAGE_BYTES)
        .is_none()
    {
        return Err(format!(
            "PostgreSQL query exceeds {} byte message limit",
            MAX_PG_MESSAGE_BYTES
        ));
    }
    Ok(())
}

fn add_result_bytes(total: usize, row_bytes: usize) -> Result<usize, String> {
    total
        .checked_add(row_bytes)
        .filter(|total| *total <= MAX_PG_RESULT_BYTES)
        .ok_or_else(|| format!("PostgreSQL result exceeds {MAX_PG_RESULT_BYTES} byte limit"))
}

fn typed_row_result_cost(body: &[u8], columns: &[PgColumn]) -> Result<usize, String> {
    // Includes the result-list slot, final map, GC headers, keys, tagged values,
    // and a payload allocation for every cell. The wire body already includes
    // each non-null payload, so nulls are deliberately over-counted.
    let column_names = columns.iter().try_fold(0_usize, |total, column| {
        total.checked_add(column.name.len())
    });
    body.len()
        .checked_add(40)
        .and_then(|cost| columns.len().checked_mul(96)?.checked_add(cost))
        .and_then(|cost| cost.checked_add(column_names?))
        .ok_or_else(|| "PostgreSQL decoded result size overflow".to_string())
}

pub(crate) unsafe fn alloc_db_value(tag: u8, payload: *mut u8) -> *mut MeshDbValue {
    let value = mesh_gc_alloc_actor(
        std::mem::size_of::<MeshDbValue>() as u64,
        std::mem::align_of::<MeshDbValue>() as u64,
    ) as *mut MeshDbValue;
    if value.is_null() {
        return std::ptr::null_mut();
    }
    value.write(MeshDbValue { tag, payload });
    value
}

unsafe fn typed_row_to_map(body: &[u8], columns: &[PgColumn]) -> Result<*mut u8, String> {
    let values = parse_typed_row(body, columns)?;
    let mut entries = Vec::<[u64; 2]>::with_capacity(columns.len());
    let mut indexes = HashMap::<&str, usize>::with_capacity(columns.len());
    for (column, value) in columns.iter().zip(values) {
        let value = match value {
            RowValue::Text(text) => alloc_db_value(DB_VALUE_TEXT, rust_str_to_mesh(text)),
            RowValue::Binary(bytes) => alloc_db_value(
                DB_VALUE_BINARY,
                mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64) as *mut u8,
            ),
            RowValue::Null => alloc_db_value(DB_VALUE_NULL, std::ptr::null_mut()),
        };
        if let Some(index) = indexes.get(column.name.as_str()).copied() {
            entries[index][1] = value as u64;
        } else {
            indexes.insert(column.name.as_str(), entries.len());
            entries.push([rust_str_to_mesh(&column.name) as u64, value as u64]);
        }
    }
    Ok(mesh_map_from_string_entries(&entries))
}

fn prepare_typed_statement(conn: &mut PgConn, sql: &str) -> Result<Vec<PgColumn>, String> {
    validate_typed_sql(sql)?;
    let mut request = Vec::new();
    write_parse(&mut request, sql);
    write_describe_statement(&mut request);
    write_sync(&mut request);
    conn.write_wire_all(&request)
        .map_err(|error| format!("send prepare: {error}"))?;

    let mut columns = None;
    let mut error = None;
    loop {
        let (tag, body) = conn
            .read_wire_message()
            .map_err(|read_error| format!("read prepare: {read_error}"))?;
        match tag {
            b'1' | b't' | b'N' => {} // ParseComplete, ParameterDescription, NoticeResponse
            b'T' if error.is_none() => match parse_row_description(&body) {
                Ok(description) => columns = Some(description),
                Err(description_error) => {
                    conn.broken = true;
                    return Err(description_error);
                }
            },
            b'n' => columns = Some(Vec::new()), // NoData
            b'E' if error.is_none() => {
                error = Some(format_pg_error_string(&parse_error_response_full(&body)))
            }
            b'Z' => {
                conn.txn_status = body.first().copied().unwrap_or(b'I');
                break;
            }
            _ => {}
        }
    }
    if let Some(error) = error {
        Err(error)
    } else {
        columns.ok_or_else(|| "PostgreSQL prepare returned no row description".to_string())
    }
}

// ── MeshString / MeshResult Helpers ────────────────────────────────────

/// Extract a Rust &str from a raw MeshString pointer.
///
/// # Safety
///
/// The pointer must reference a valid MeshString allocation.
unsafe fn mesh_str_to_rust(s: *const MeshString) -> &'static str {
    (*s).as_str()
}

/// Create a MeshString from a Rust &str and return as *mut u8.
fn rust_str_to_mesh(s: &str) -> *mut u8 {
    mesh_string_new(s.as_ptr(), s.len() as u64) as *mut u8
}

/// Create an error MeshResult from a Rust string.
fn err_result(msg: &str) -> *mut u8 {
    let s = rust_str_to_mesh(msg);
    alloc_result(1, s) as *mut u8
}

/// Extract param strings from a Mesh List<String>.
///
/// MeshList layout: `{ len: u64, cap: u64, data: [u64; cap] }`
/// Each element is a u64 that is actually a pointer to a MeshString.
unsafe fn extract_params(params: *mut u8) -> Vec<String> {
    let len = *(params as *const u64);
    let data_ptr = (params as *const u64).add(2); // skip len + cap
    let mut result = Vec::with_capacity(len as usize);
    for i in 0..len as usize {
        let param_ptr = *data_ptr.add(i) as *const MeshString;
        let param_str = mesh_str_to_rust(param_ptr);
        result.push(param_str.to_string());
    }
    result
}

unsafe fn extract_db_values<'a>(params: *mut u8) -> Result<Vec<BindValue<'a>>, String> {
    if params.is_null() {
        return Err("invalid PostgreSQL parameter list".to_string());
    }
    let len = mesh_list_length(params) as usize;
    if len > MAX_PG_VALUES {
        return Err(format!(
            "too many PostgreSQL parameters: {len} (maximum {MAX_PG_VALUES})"
        ));
    }

    let mut result = Vec::with_capacity(len);
    for index in 0..len {
        let value = mesh_list_get(params, index as i64) as *const MeshDbValue;
        if value.is_null() {
            return Err(format!("invalid PostgreSQL parameter at index {index}"));
        }
        match (*value).tag {
            DB_VALUE_TEXT => {
                let text = (*value).payload as *const MeshString;
                if text.is_null() {
                    return Err(format!("invalid text parameter at index {index}"));
                }
                let bytes = (*text).as_str().as_bytes();
                if bytes.len() > MAX_DB_VALUE_BYTES {
                    return Err(format!(
                        "PostgreSQL parameter at index {index} exceeds {MAX_DB_VALUE_BYTES} byte limit"
                    ));
                }
                result.push(BindValue::Text(bytes));
            }
            DB_VALUE_BINARY => {
                let bytes = (*value).payload as *const MeshBytes;
                if bytes.is_null() {
                    return Err(format!("invalid binary parameter at index {index}"));
                }
                let len = usize::try_from((*bytes).len)
                    .map_err(|_| format!("invalid binary parameter length at index {index}"))?;
                if len > MAX_DB_VALUE_BYTES {
                    return Err(format!(
                        "PostgreSQL parameter at index {index} exceeds {MAX_DB_VALUE_BYTES} byte limit"
                    ));
                }
                result.push(BindValue::Binary((*bytes).as_slice()));
            }
            DB_VALUE_NULL => result.push(BindValue::Null),
            tag => {
                return Err(format!(
                    "invalid DbValue tag {tag} at PostgreSQL parameter index {index}"
                ))
            }
        }
    }
    Ok(result)
}

// ── Parse CommandComplete tag for row count ────────────────────────────

/// Parse the row count from a CommandComplete tag string.
/// Examples: "INSERT 0 5" -> 5, "UPDATE 3" -> 3, "DELETE 1" -> 1,
///           "SELECT 10" -> 10, "CREATE TABLE" -> 0
fn parse_command_tag(tag: &str) -> i64 {
    tag.split_whitespace()
        .last()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

// ── Public API ─────────────────────────────────────────────────────────

/// Connect to a PostgreSQL server.
///
/// # Signature
///
/// `mesh_pg_connect(url: *const MeshString) -> *mut u8 (MeshResult<u64, String>)`
///
/// Returns MeshResult with tag 0 (Ok) containing the connection handle as
/// a u64, or tag 1 (Err) containing an error message string.
#[no_mangle]
pub extern "C" fn mesh_pg_connect(url: *const MeshString) -> *mut u8 {
    unsafe {
        let url_str = mesh_str_to_rust(url);
        let pg_url = match parse_pg_url(url_str) {
            Ok(u) => u,
            Err(e) => return err_result(&e),
        };

        // Resolve address and connect with timeout
        let addr_str = format!("{}:{}", pg_url.host, pg_url.port);
        let addr: SocketAddr = match addr_str.to_socket_addrs() {
            Ok(mut addrs) => match addrs.next() {
                Some(a) => a,
                None => return err_result("could not resolve host"),
            },
            Err(e) => return err_result(&format!("DNS resolution failed: {}", e)),
        };

        let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
            Ok(s) => s,
            Err(e) => return err_result(&format!("connection failed: {}", e)),
        };

        // Set read/write timeouts BEFORE TLS wrapping (StreamOwned inherits them)
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

        // Negotiate TLS based on sslmode
        let mut stream = match negotiate_tls(stream, &pg_url.host, pg_url.sslmode) {
            Ok(s) => s,
            Err(e) => return err_result(&format!("TLS: {}", e)),
        };

        // Send StartupMessage
        let mut buf = Vec::new();
        write_startup_message(&mut buf, &pg_url.user, &pg_url.database);
        if let Err(e) = stream.write_all(&buf) {
            return err_result(&format!("send startup: {}", e));
        }

        // Read authentication response
        let (tag, body) = match read_message(&mut stream) {
            Ok(m) => m,
            Err(e) => return err_result(&format!("read auth: {}", e)),
        };

        if tag != b'R' {
            if tag == b'E' {
                return err_result(&parse_error_response(&body));
            }
            return err_result(&format!("expected auth message, got '{}'", tag as char));
        }

        if body.len() < 4 {
            return err_result("auth message too short");
        }

        let auth_type = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);

        match auth_type {
            0 => {
                // AuthenticationOk -- no auth needed
            }
            5 => {
                // MD5Password -- body[4..8] is the 4-byte salt
                if body.len() < 8 {
                    return err_result("MD5 auth: missing salt");
                }
                let salt = &body[4..8];
                let md5_pass = compute_md5_password(&pg_url.user, &pg_url.password, salt);

                let mut buf = Vec::new();
                write_password_message(&mut buf, &md5_pass);
                if let Err(e) = stream.write_all(&buf) {
                    return err_result(&format!("send MD5 password: {}", e));
                }

                // Read AuthenticationOk
                let (tag, body) = match read_message(&mut stream) {
                    Ok(m) => m,
                    Err(e) => return err_result(&format!("read MD5 auth result: {}", e)),
                };
                if tag == b'E' {
                    return err_result(&parse_error_response(&body));
                }
                if tag != b'R'
                    || body.len() < 4
                    || i32::from_be_bytes([body[0], body[1], body[2], body[3]]) != 0
                {
                    return err_result("MD5 authentication failed");
                }
            }
            10 => {
                // SASL -- read mechanism list from body[4..]
                let mech_data = &body[4..];
                let mech_str = String::from_utf8_lossy(mech_data);
                if !mech_str.contains("SCRAM-SHA-256") {
                    return err_result("server does not support SCRAM-SHA-256");
                }

                // Step 1: Send SASLInitialResponse with client-first-message
                let (client_first, client_nonce) = scram_client_first(&pg_url.user);

                let mut buf = Vec::new();
                write_sasl_initial_response(&mut buf, "SCRAM-SHA-256", client_first.as_bytes());
                if let Err(e) = stream.write_all(&buf) {
                    return err_result(&format!("send SASL initial: {}", e));
                }

                // Step 2: Read AuthenticationSASLContinue (tag 'R', auth_type 11)
                let (tag, body) = match read_message(&mut stream) {
                    Ok(m) => m,
                    Err(e) => return err_result(&format!("read SASL continue: {}", e)),
                };
                if tag == b'E' {
                    return err_result(&parse_error_response(&body));
                }
                if tag != b'R' || body.len() < 4 {
                    return err_result("expected SASL continue");
                }
                let sasl_type = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                if sasl_type != 11 {
                    return err_result(&format!("expected SASL continue (11), got {}", sasl_type));
                }
                let server_first = std::str::from_utf8(&body[4..])
                    .map_err(|_| "invalid UTF-8 in server-first")
                    .unwrap_or("invalid");

                // Step 3: Compute client-final-message
                let (client_final, expected_server_sig) =
                    match scram_client_final(&pg_url.password, &client_nonce, server_first) {
                        Ok(r) => r,
                        Err(e) => return err_result(&format!("SCRAM: {}", e)),
                    };

                let mut buf = Vec::new();
                write_sasl_response(&mut buf, client_final.as_bytes());
                if let Err(e) = stream.write_all(&buf) {
                    return err_result(&format!("send SASL response: {}", e));
                }

                // Step 4: Read AuthenticationSASLFinal (tag 'R', auth_type 12)
                let (tag, body) = match read_message(&mut stream) {
                    Ok(m) => m,
                    Err(e) => return err_result(&format!("read SASL final: {}", e)),
                };
                if tag == b'E' {
                    return err_result(&parse_error_response(&body));
                }
                if tag != b'R' || body.len() < 4 {
                    return err_result("expected SASL final");
                }
                let sasl_final_type = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                if sasl_final_type != 12 {
                    return err_result(&format!(
                        "expected SASL final (12), got {}",
                        sasl_final_type
                    ));
                }

                // Verify server signature
                let server_final = std::str::from_utf8(&body[4..]).unwrap_or("");
                if let Some(v_str) = server_final.strip_prefix("v=") {
                    if let Ok(sig) = BASE64.decode(v_str) {
                        if sig != expected_server_sig {
                            return err_result("SCRAM: server signature mismatch");
                        }
                    }
                }

                // Read AuthenticationOk
                let (tag, body) = match read_message(&mut stream) {
                    Ok(m) => m,
                    Err(e) => return err_result(&format!("read SCRAM auth ok: {}", e)),
                };
                if tag == b'E' {
                    return err_result(&parse_error_response(&body));
                }
                if tag != b'R'
                    || body.len() < 4
                    || i32::from_be_bytes([body[0], body[1], body[2], body[3]]) != 0
                {
                    return err_result("SCRAM authentication failed");
                }
            }
            3 => {
                // Cleartext password (rarely used, but handle it)
                let mut buf = Vec::new();
                write_password_message(&mut buf, &pg_url.password);
                if let Err(e) = stream.write_all(&buf) {
                    return err_result(&format!("send password: {}", e));
                }
                let (tag, body) = match read_message(&mut stream) {
                    Ok(m) => m,
                    Err(e) => return err_result(&format!("read auth result: {}", e)),
                };
                if tag == b'E' {
                    return err_result(&parse_error_response(&body));
                }
                if tag != b'R'
                    || body.len() < 4
                    || i32::from_be_bytes([body[0], body[1], body[2], body[3]]) != 0
                {
                    return err_result("cleartext authentication failed");
                }
            }
            _ => {
                return err_result(&format!("unsupported auth type: {}", auth_type));
            }
        }

        // Read messages until ReadyForQuery ('Z')
        #[allow(unused_assignments)]
        let mut last_txn_status: u8 = b'I';
        loop {
            let (tag, body) = match read_message(&mut stream) {
                Ok(m) => m,
                Err(e) => return err_result(&format!("post-auth read: {}", e)),
            };
            match tag {
                b'Z' => {
                    last_txn_status = if !body.is_empty() { body[0] } else { b'I' };
                    break;
                }
                b'S' => {} // ParameterStatus -- skip
                b'K' => {} // BackendKeyData -- skip
                b'N' => {} // NoticeResponse -- skip
                b'E' => {
                    return err_result(&parse_error_response(&body));
                }
                _ => {} // skip unknown
            }
        }

        // Create the PgConn handle
        let conn = Box::new(PgConn {
            stream,
            txn_status: last_txn_status,
            broken: false,
        });
        let handle = Box::into_raw(conn) as u64;
        // Result payloads with integer semantics are represented by pointers to
        // boxed integers. This keeps direct `Pg.connect(...) ?` unwrapping
        // consistent with SQLite and the other integer-returning DB APIs.
        let payload = Box::into_raw(Box::new(handle)) as *mut u8;
        alloc_result(0, payload) as *mut u8
    }
}

/// Close a PostgreSQL connection.
///
/// # Signature
///
/// `mesh_pg_close(conn_handle: u64)`
///
/// Recovers the Box<PgConn> from the handle, sends Terminate message,
/// and lets Box::drop free the Rust memory and close the TcpStream.
#[no_mangle]
pub extern "C" fn mesh_pg_close(conn_handle: u64) {
    unsafe {
        let mut conn = Box::from_raw(conn_handle as *mut PgConn);
        let mut buf = Vec::new();
        write_terminate(&mut buf);
        let _ = conn.stream.write_all(&buf);
        // Box drops, TcpStream closes
    }
}

/// Execute a write SQL statement (INSERT, UPDATE, DELETE, CREATE TABLE, etc.).
///
/// # Signature
///
/// `mesh_pg_execute(conn_handle: u64, sql: *const MeshString, params: *mut u8)
///     -> *mut u8 (MeshResult<Int, String>)`
///
/// Parameters are bound via the Extended Query protocol using $1, $2, etc.
/// Returns the number of rows affected from the CommandComplete tag.
#[no_mangle]
pub extern "C" fn mesh_pg_execute(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);
        let sql_str = mesh_str_to_rust(sql);
        let param_strs = extract_params(params);
        let param_refs: Vec<&str> = param_strs.iter().map(|s| s.as_str()).collect();

        // Build pipelined message: Parse + Bind + Execute + Sync
        let mut buf = Vec::new();
        write_parse(&mut buf, sql_str);
        write_bind(&mut buf, &param_refs);
        write_execute(&mut buf);
        write_sync(&mut buf);

        if let Err(e) = conn.write_wire_all(&buf) {
            return err_result(&format!("send execute: {}", e));
        }

        let mut rows_affected: i64 = 0;
        let mut error_msg: Option<String> = None;

        // Read messages until ReadyForQuery
        loop {
            let (tag, body) = match conn.read_wire_message() {
                Ok(m) => m,
                Err(e) => return err_result(&format!("read execute: {}", e)),
            };
            match tag {
                b'1' => {} // ParseComplete
                b'2' => {} // BindComplete
                b'C' => {
                    // CommandComplete
                    let tag_str = String::from_utf8_lossy(&body);
                    let tag_str = tag_str.trim_end_matches('\0');
                    rows_affected = parse_command_tag(tag_str);
                }
                b'E' => {
                    // ErrorResponse -- use structured format for constraint mapping
                    let pg_err = parse_error_response_full(&body);
                    error_msg = Some(format_pg_error_string(&pg_err));
                }
                b'Z' => {
                    conn.txn_status = if !body.is_empty() { body[0] } else { b'I' };
                    break;
                }
                b'N' => {} // NoticeResponse -- skip
                _ => {}
            }
        }

        if let Some(msg) = error_msg {
            err_result(&msg)
        } else {
            let boxed = Box::into_raw(Box::new(rows_affected)) as *mut u8;
            alloc_result(0, boxed) as *mut u8
        }
    }
}

/// Execute a read SQL statement (SELECT) and return rows.
///
/// # Signature
///
/// `mesh_pg_query(conn_handle: u64, sql: *const MeshString, params: *mut u8)
///     -> *mut u8 (MeshResult<List<Map<String, String>>, String>)`
///
/// Each row is a Map<String, String> where keys are column names and values
/// are the text representation of column values. NULL columns become empty
/// strings.
#[no_mangle]
pub extern "C" fn mesh_pg_query(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);
        let sql_str = mesh_str_to_rust(sql);
        let param_strs = extract_params(params);
        let param_refs: Vec<&str> = param_strs.iter().map(|s| s.as_str()).collect();

        // Build pipelined message: Parse + Bind + Describe(Portal) + Execute + Sync
        let mut buf = Vec::new();
        write_parse(&mut buf, sql_str);
        write_bind(&mut buf, &param_refs);
        write_describe_portal(&mut buf);
        write_execute(&mut buf);
        write_sync(&mut buf);

        if let Err(e) = conn.write_wire_all(&buf) {
            return err_result(&format!("send query: {}", e));
        }

        let mut col_names: Vec<String> = Vec::new();
        let mut result_list = mesh_list_new();
        let mut error_msg: Option<String> = None;

        // Read messages until ReadyForQuery
        loop {
            let (tag, body) = match conn.read_wire_message() {
                Ok(m) => m,
                Err(e) => return err_result(&format!("read query: {}", e)),
            };
            match tag {
                b'1' => {} // ParseComplete
                b'2' => {} // BindComplete
                b'T' => {
                    // RowDescription
                    if body.len() < 2 {
                        continue;
                    }
                    let num_fields = i16::from_be_bytes([body[0], body[1]]) as usize;
                    let mut offset = 2;
                    col_names.clear();
                    for _ in 0..num_fields {
                        // Read null-terminated column name
                        let name_start = offset;
                        while offset < body.len() && body[offset] != 0 {
                            offset += 1;
                        }
                        let name = String::from_utf8_lossy(&body[name_start..offset]).into_owned();
                        col_names.push(name);
                        offset += 1; // skip null terminator
                                     // Skip 18 bytes: table OID (4) + column number (2) + type OID (4)
                                     //   + type size (2) + type modifier (4) + format code (2)
                        offset += 18;
                    }
                }
                b'D' => {
                    // DataRow
                    if body.len() < 2 {
                        continue;
                    }
                    let num_cols = i16::from_be_bytes([body[0], body[1]]) as usize;
                    let mut offset = 2;

                    // Create a string-keyed map for this row (key_type = 1 = string)
                    let mut row_map = mesh_map_new_typed(1);

                    for col in 0..num_cols {
                        if offset + 4 > body.len() {
                            break;
                        }
                        let col_len = i32::from_be_bytes([
                            body[offset],
                            body[offset + 1],
                            body[offset + 2],
                            body[offset + 3],
                        ]);
                        offset += 4;

                        let value_str = if col_len == -1 {
                            // NULL
                            String::new()
                        } else {
                            let end = offset + col_len as usize;
                            let s = String::from_utf8_lossy(&body[offset..end]).into_owned();
                            offset = end;
                            s
                        };

                        let col_name = if col < col_names.len() {
                            &col_names[col]
                        } else {
                            "?"
                        };

                        let key_mesh = rust_str_to_mesh(col_name);
                        let val_mesh = rust_str_to_mesh(&value_str);
                        row_map = mesh_map_put(row_map, key_mesh as u64, val_mesh as u64);
                    }

                    result_list = mesh_list_append(result_list, row_map as u64);
                }
                b'C' => {} // CommandComplete -- skip for query
                b'E' => {
                    // ErrorResponse -- use structured format for constraint mapping
                    let pg_err = parse_error_response_full(&body);
                    error_msg = Some(format_pg_error_string(&pg_err));
                }
                b'Z' => {
                    conn.txn_status = if !body.is_empty() { body[0] } else { b'I' };
                    break;
                }
                b'N' => {} // NoticeResponse -- skip
                _ => {}
            }
        }

        if let Some(msg) = error_msg {
            err_result(&msg)
        } else {
            alloc_result(0, result_list) as *mut u8
        }
    }
}

// ── Transaction Management ─────────────────────────────────────────────

/// Execute an unnamed prepared statement with typed `DbValue` parameters.
#[no_mangle]
pub extern "C" fn mesh_pg_execute_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        if conn_handle == 0 || sql.is_null() {
            return err_result("invalid PostgreSQL execute_values arguments");
        }
        let conn = &mut *(conn_handle as *mut PgConn);
        let sql = mesh_str_to_rust(sql);
        if let Err(error) = validate_typed_sql(sql) {
            return err_result(&error);
        }
        let params = match extract_db_values(params) {
            Ok(params) => params,
            Err(error) => return err_result(&error),
        };

        let mut request = Vec::new();
        write_parse(&mut request, sql);
        if let Err(error) = write_bind_values(&mut request, &params, &[]) {
            return err_result(&error);
        }
        write_execute(&mut request);
        write_sync(&mut request);
        if let Err(error) = conn.write_wire_all(&request) {
            return err_result(&format!("send execute_values: {error}"));
        }

        let mut rows_affected = 0_i64;
        let mut error = None;
        loop {
            let (tag, body) = match conn.read_wire_message() {
                Ok(message) => message,
                Err(read_error) => {
                    return err_result(&format!("read execute_values: {read_error}"))
                }
            };
            match tag {
                b'1' | b'2' | b'D' | b'N' => {}
                b'C' => {
                    let command = String::from_utf8_lossy(&body);
                    rows_affected = parse_command_tag(command.trim_end_matches('\0'));
                }
                b'E' if error.is_none() => {
                    error = Some(format_pg_error_string(&parse_error_response_full(&body)))
                }
                b'Z' => {
                    conn.txn_status = body.first().copied().unwrap_or(b'I');
                    break;
                }
                _ => {}
            }
        }

        if let Some(error) = error {
            err_result(&error)
        } else {
            alloc_result(0, Box::into_raw(Box::new(rows_affected)) as *mut u8) as *mut u8
        }
    }
}

/// Query typed values while keeping non-BYTEA columns in PostgreSQL text format.
#[no_mangle]
pub extern "C" fn mesh_pg_query_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        if conn_handle == 0 || sql.is_null() {
            return err_result("invalid PostgreSQL query_values arguments");
        }
        let conn = &mut *(conn_handle as *mut PgConn);
        let sql = mesh_str_to_rust(sql);
        let params = match extract_db_values(params) {
            Ok(params) => params,
            Err(error) => return err_result(&error),
        };
        let columns = match prepare_typed_statement(conn, sql) {
            Ok(columns) => columns,
            Err(error) => return err_result(&error),
        };
        let formats: Vec<i16> = columns
            .iter()
            .map(|column| if column.oid == 17 { 1 } else { 0 })
            .collect();

        let mut request = Vec::new();
        if let Err(error) = write_bind_values(&mut request, &params, &formats) {
            return err_result(&error);
        }
        write_execute(&mut request);
        write_sync(&mut request);
        if let Err(error) = conn.write_wire_all(&request) {
            return err_result(&format!("send query_values: {error}"));
        }

        let mut rows = Vec::new();
        let mut row_count = 0_usize;
        // Final Result and list allocations, including actor-GC headers.
        let mut result_bytes = 64_usize;
        let mut error = None;
        loop {
            let (tag, body) = match conn.read_wire_message() {
                Ok(message) => message,
                Err(read_error) => return err_result(&format!("read query_values: {read_error}")),
            };
            match tag {
                b'2' | b'C' | b'N' => {}
                b'D' if error.is_none() => match typed_row_result_cost(&body, &columns)
                    .and_then(|cost| add_result_bytes(result_bytes, cost))
                {
                    Err(size_error) => error = Some(size_error),
                    Ok(_) if row_count == MAX_PG_ROWS => {
                        error = Some(format!("PostgreSQL result exceeds {MAX_PG_ROWS} row limit"))
                    }
                    Ok(next_result_bytes) => match typed_row_to_map(&body, &columns) {
                        Ok(row) => {
                            rows.push(row as u64);
                            row_count += 1;
                            result_bytes = next_result_bytes;
                        }
                        Err(row_error) => {
                            conn.broken = true;
                            return err_result(&row_error);
                        }
                    },
                },
                b'E' if error.is_none() => {
                    error = Some(format_pg_error_string(&parse_error_response_full(&body)))
                }
                b'Z' => {
                    conn.txn_status = body.first().copied().unwrap_or(b'I');
                    break;
                }
                _ => {}
            }
        }

        if let Some(error) = error {
            err_result(&error)
        } else {
            alloc_result(0, mesh_list_from_array(rows.as_ptr(), rows.len() as i64)) as *mut u8
        }
    }
}

/// Send a simple SQL command (BEGIN/COMMIT/ROLLBACK) using the Simple Query protocol.
/// Returns Ok(()) or Err(error_message). Updates conn.txn_status from ReadyForQuery.
pub(super) fn pg_simple_command(conn: &mut PgConn, sql: &str) -> Result<(), String> {
    // Simple Query protocol: Byte1('Q') Int32(len) String(query\0)
    let mut buf = Vec::new();
    buf.push(b'Q');
    let body = format!("{}\0", sql);
    let len = (body.len() + 4) as i32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(body.as_bytes());
    conn.write_wire_all(&buf)
        .map_err(|e| format!("send {}: {}", sql, e))?;

    let mut error_msg: Option<String> = None;
    loop {
        let (tag, body) = conn
            .read_wire_message()
            .map_err(|e| format!("read {}: {}", sql, e))?;
        match tag {
            b'C' => {} // CommandComplete
            b'E' => {
                error_msg = Some(parse_error_response(&body));
            }
            b'Z' => {
                conn.txn_status = if !body.is_empty() { body[0] } else { b'I' };
                break;
            }
            _ => {}
        }
    }
    match error_msg {
        Some(msg) => Err(msg),
        None => Ok(()),
    }
}

/// Begin a PostgreSQL transaction.
///
/// # Signature
///
/// `mesh_pg_begin(conn_handle: u64) -> *mut u8 (MeshResult<Unit, String>)`
///
/// Sends `BEGIN` and returns Ok(()) or Err(error_message).
#[no_mangle]
pub extern "C" fn mesh_pg_begin(conn_handle: u64) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);
        match pg_simple_command(conn, "BEGIN") {
            Ok(()) => alloc_result(0, std::ptr::null_mut()) as *mut u8,
            Err(e) => err_result(&e),
        }
    }
}

/// Commit a PostgreSQL transaction.
///
/// # Signature
///
/// `mesh_pg_commit(conn_handle: u64) -> *mut u8 (MeshResult<Unit, String>)`
///
/// Sends `COMMIT` and returns Ok(()) or Err(error_message).
#[no_mangle]
pub extern "C" fn mesh_pg_commit(conn_handle: u64) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);
        match pg_simple_command(conn, "COMMIT") {
            Ok(()) => alloc_result(0, std::ptr::null_mut()) as *mut u8,
            Err(e) => err_result(&e),
        }
    }
}

/// Rollback a PostgreSQL transaction.
///
/// # Signature
///
/// `mesh_pg_rollback(conn_handle: u64) -> *mut u8 (MeshResult<Unit, String>)`
///
/// Sends `ROLLBACK` and returns Ok(()) or Err(error_message).
#[no_mangle]
pub extern "C" fn mesh_pg_rollback(conn_handle: u64) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);
        match pg_simple_command(conn, "ROLLBACK") {
            Ok(()) => alloc_result(0, std::ptr::null_mut()) as *mut u8,
            Err(e) => err_result(&e),
        }
    }
}

pub(crate) unsafe fn invoke_transaction_callback(
    fn_ptr: *const u8,
    env_ptr: *const u8,
    conn_handle: u64,
) -> *mut u8 {
    let result = if env_ptr.is_null() {
        let callback: extern "C" fn(u64) -> MeshResult = std::mem::transmute(fn_ptr);
        callback(conn_handle)
    } else {
        let callback: extern "C" fn(*const u8, u64) -> MeshResult = std::mem::transmute(fn_ptr);
        callback(env_ptr, conn_handle)
    };
    alloc_result(result.tag, result.value) as *mut u8
}

/// Execute a Mesh closure inside a PostgreSQL transaction with automatic
/// commit on success and rollback on error or panic.
///
/// # Signature
///
/// `mesh_pg_transaction(conn_handle: u64, fn_ptr: *const u8, env_ptr: *const u8)
///     -> *mut u8 (MeshResult<T, String>)`
///
/// Protocol:
/// 1. Send BEGIN. On failure, return Err immediately.
/// 2. Call the Mesh closure via catch_unwind for panic safety.
/// 3. On Ok result from closure: COMMIT. If COMMIT fails, ROLLBACK and return Err.
/// 4. On Err result from closure: ROLLBACK and propagate the Err.
/// 5. On panic: ROLLBACK and return Err("transaction aborted: panic in callback").
#[no_mangle]
pub extern "C" fn mesh_pg_transaction(
    conn_handle: u64,
    fn_ptr: *const u8,
    env_ptr: *const u8,
) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);

        // 1. BEGIN
        if let Err(e) = pg_simple_command(conn, "BEGIN") {
            return err_result(&format!("BEGIN: {}", e));
        }

        // 2. Call the closure with catch_unwind for panic safety
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            invoke_transaction_callback(fn_ptr, env_ptr, conn_handle)
        }));

        match result {
            Ok(result_ptr) => {
                // Check if closure returned Ok or Err via MeshResult tag
                let r = &*(result_ptr as *const crate::io::MeshResult);
                if r.tag == 0 {
                    // Success -> COMMIT
                    if let Err(e) = pg_simple_command(conn, "COMMIT") {
                        let _ = pg_simple_command(conn, "ROLLBACK");
                        return err_result(&format!("COMMIT: {}", e));
                    }
                    result_ptr
                } else {
                    // Error -> ROLLBACK
                    let _ = pg_simple_command(conn, "ROLLBACK");
                    result_ptr // propagate the Err result
                }
            }
            Err(_) => {
                // Panic -> ROLLBACK
                let _ = pg_simple_command(conn, "ROLLBACK");
                err_result("transaction aborted: panic in callback")
            }
        }
    }
}

// ── Struct-to-Row Query ───────────────────────────────────────────────

type FromRowFn = unsafe extern "C" fn(*mut u8) -> *mut u8;

/// Execute a SELECT query and map each row through a from_row callback.
///
/// # Signature
///
/// `mesh_pg_query_as(conn_handle: u64, sql: *mut u8, params: *mut u8,
///     from_row_fn: *mut u8) -> *mut u8 (MeshResult<List<MeshResult>, String>)`
///
/// 1. Calls `mesh_pg_query` to get the raw rows.
/// 2. If query fails, propagates the error result as-is.
/// 3. If Ok: iterates the rows list, calling `from_row_fn` on each row map.
/// 4. Collects all per-row results into a new list.
/// 5. Returns Ok(list_of_results).
#[no_mangle]
pub extern "C" fn mesh_pg_query_as(
    conn_handle: u64,
    sql: *mut u8,
    params: *mut u8,
    from_row_fn: *mut u8,
) -> *mut u8 {
    unsafe {
        // Execute the query
        let query_result = mesh_pg_query(conn_handle, sql as *const MeshString, params);
        let r = &*(query_result as *const crate::io::MeshResult);
        if r.tag != 0 {
            return query_result; // propagate query error
        }

        // Extract the rows list from the Ok result
        let rows_list = r.value;
        let row_count = mesh_list_length(rows_list);
        let from_row: FromRowFn = std::mem::transmute(from_row_fn);

        // Map each row through the from_row callback
        let mut result_list = mesh_list_new();
        for i in 0..row_count {
            let row = mesh_list_get(rows_list, i);
            let mapped = from_row(row as *mut u8);
            result_list = mesh_list_append(result_list, mapped as u64);
        }

        alloc_result(0, result_list as *mut u8) as *mut u8
    }
}

// ── Pure Rust PG API (no MeshString/GC) ─────────────────────────────────
//
// These functions provide a direct Rust-level PostgreSQL client that does not
// depend on the GC or MeshString allocations. Used by meshc's migration runner
// for tracking table operations.

/// A native Rust PostgreSQL connection handle.
pub struct NativePgConn {
    inner: PgConn,
}

/// Connect to PostgreSQL using a URL string. Returns a native connection.
pub fn native_pg_connect(url: &str) -> Result<NativePgConn, String> {
    let pg_url = parse_pg_url(url)?;

    let addr_str = format!("{}:{}", pg_url.host, pg_url.port);
    let addr: std::net::SocketAddr = addr_str
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed: {}", e))?
        .next()
        .ok_or_else(|| "could not resolve host".to_string())?;

    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
        .map_err(|e| format!("connection failed: {}", e))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let mut stream = negotiate_tls(stream, &pg_url.host, pg_url.sslmode)?;

    // Send StartupMessage
    let mut buf = Vec::new();
    write_startup_message(&mut buf, &pg_url.user, &pg_url.database);
    stream
        .write_all(&buf)
        .map_err(|e| format!("send startup: {}", e))?;

    // Read authentication response
    let (tag, body) = read_message(&mut stream).map_err(|e| format!("read auth: {}", e))?;
    if tag != b'R' {
        if tag == b'E' {
            return Err(parse_error_response(&body));
        }
        return Err(format!("expected auth message, got '{}'", tag as char));
    }
    if body.len() < 4 {
        return Err("auth message too short".to_string());
    }

    let auth_type = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);

    match auth_type {
        0 => {} // AuthenticationOk -- no auth required
        3 => {
            // CleartextPassword
            let mut buf = Vec::new();
            write_password_message(&mut buf, &pg_url.password);
            stream
                .write_all(&buf)
                .map_err(|e| format!("send password: {}", e))?;
            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read auth response: {}", e))?;
            if tag == b'E' {
                return Err(parse_error_response(&body));
            }
        }
        5 => {
            // MD5Password
            let salt = &body[4..8];
            let hashed = compute_md5_password(&pg_url.user, &pg_url.password, salt);
            let mut buf = Vec::new();
            write_password_message(&mut buf, &hashed);
            stream
                .write_all(&buf)
                .map_err(|e| format!("send md5: {}", e))?;
            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read md5 response: {}", e))?;
            if tag == b'E' {
                return Err(parse_error_response(&body));
            }
        }
        10 => {
            // SASL
            let (client_first, client_nonce) = scram_client_first(&pg_url.user);
            let mut buf = Vec::new();
            write_sasl_initial_response(&mut buf, "SCRAM-SHA-256", client_first.as_bytes());
            stream
                .write_all(&buf)
                .map_err(|e| format!("send SASL init: {}", e))?;

            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read SASL continue: {}", e))?;
            if tag == b'E' {
                return Err(parse_error_response(&body));
            }
            if tag != b'R' {
                return Err("expected SASL continue".to_string());
            }
            let server_first = String::from_utf8_lossy(&body[4..]).to_string();
            let (client_final, _expected_sig) =
                scram_client_final(&pg_url.password, &client_nonce, &server_first)?;

            let mut buf = Vec::new();
            write_sasl_response(&mut buf, client_final.as_bytes());
            stream
                .write_all(&buf)
                .map_err(|e| format!("send SASL final: {}", e))?;

            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read SASL final: {}", e))?;
            if tag == b'E' {
                return Err(parse_error_response(&body));
            }
            // AuthenticationSASLFinal
            if tag == b'R' {
                let auth2 = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                if auth2 != 12 && auth2 != 0 {
                    return Err(format!("unexpected SASL auth type: {}", auth2));
                }
            }
            // Read AuthenticationOk if not already received
            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read auth ok: {}", e))?;
            if tag == b'E' {
                return Err(parse_error_response(&body));
            }
        }
        _ => {
            return Err(format!("unsupported auth type: {}", auth_type));
        }
    }

    // Read parameter status and ready-for-query messages
    let mut txn_status = b'I';
    loop {
        let (tag, body) =
            read_message(&mut stream).map_err(|e| format!("read startup params: {}", e))?;
        match tag {
            b'K' | b'S' | b'N' => {} // BackendKeyData, ParameterStatus, NoticeResponse
            b'E' => return Err(parse_error_response(&body)),
            b'Z' => {
                if !body.is_empty() {
                    txn_status = body[0];
                }
                break;
            }
            _ => {}
        }
    }

    Ok(NativePgConn {
        inner: PgConn {
            stream,
            txn_status,
            broken: false,
        },
    })
}

/// Execute a SQL statement via native connection. Returns rows affected.
pub fn native_pg_execute(
    conn: &mut NativePgConn,
    sql: &str,
    params: &[&str],
) -> Result<i64, String> {
    let mut buf = Vec::new();
    write_parse(&mut buf, sql);
    write_bind(&mut buf, params);
    write_execute(&mut buf);
    write_sync(&mut buf);

    conn.inner
        .write_wire_all(&buf)
        .map_err(|e| format!("send execute: {}", e))?;

    let mut rows_affected: i64 = 0;
    let mut error_msg: Option<String> = None;

    loop {
        let (tag, body) = conn
            .inner
            .read_wire_message()
            .map_err(|e| format!("read execute: {}", e))?;
        match tag {
            b'1' | b'2' => {}
            b'C' => {
                let tag_str = String::from_utf8_lossy(&body);
                let tag_str = tag_str.trim_end_matches('\0');
                rows_affected = parse_command_tag(tag_str);
            }
            b'E' => {
                error_msg = Some(parse_error_response(&body));
            }
            b'Z' => {
                conn.inner.txn_status = if !body.is_empty() { body[0] } else { b'I' };
                break;
            }
            b'N' => {}
            _ => {}
        }
    }

    match error_msg {
        Some(msg) => Err(msg),
        None => Ok(rows_affected),
    }
}

/// Execute a SQL query via native connection. Returns rows as Vec of column-value maps.
pub fn native_pg_query(
    conn: &mut NativePgConn,
    sql: &str,
    params: &[&str],
) -> Result<Vec<Vec<(String, String)>>, String> {
    let mut buf = Vec::new();
    write_parse(&mut buf, sql);
    write_bind(&mut buf, params);
    write_describe_portal(&mut buf);
    write_execute(&mut buf);
    write_sync(&mut buf);

    conn.inner
        .write_wire_all(&buf)
        .map_err(|e| format!("send query: {}", e))?;

    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<(String, String)>> = Vec::new();
    let mut error_msg: Option<String> = None;

    loop {
        let (tag, body) = conn
            .inner
            .read_wire_message()
            .map_err(|e| format!("read query: {}", e))?;
        match tag {
            b'1' | b'2' | b'n' => {} // ParseComplete, BindComplete, NoData
            b'T' => {
                // RowDescription
                if body.len() >= 2 {
                    let num_fields = i16::from_be_bytes([body[0], body[1]]) as usize;
                    let mut offset = 2;
                    for _ in 0..num_fields {
                        let name_end = body[offset..].iter().position(|&b| b == 0).unwrap_or(0);
                        let name =
                            String::from_utf8_lossy(&body[offset..offset + name_end]).to_string();
                        columns.push(name);
                        offset += name_end + 1 + 18; // name + null + fixed fields
                    }
                }
            }
            b'D' => {
                // DataRow
                if body.len() >= 2 {
                    let num_cols = i16::from_be_bytes([body[0], body[1]]) as usize;
                    let mut offset = 2;
                    let mut row = Vec::new();
                    for i in 0..num_cols {
                        if offset + 4 > body.len() {
                            break;
                        }
                        let col_len = i32::from_be_bytes([
                            body[offset],
                            body[offset + 1],
                            body[offset + 2],
                            body[offset + 3],
                        ]);
                        offset += 4;
                        let col_name = columns.get(i).cloned().unwrap_or_default();
                        if col_len < 0 {
                            // NULL
                            row.push((col_name, String::new()));
                        } else {
                            let end = offset + col_len as usize;
                            let val = String::from_utf8_lossy(&body[offset..end]).to_string();
                            row.push((col_name, val));
                            offset = end;
                        }
                    }
                    rows.push(row);
                }
            }
            b'C' => {} // CommandComplete
            b'E' => {
                error_msg = Some(parse_error_response(&body));
            }
            b'Z' => {
                conn.inner.txn_status = if !body.is_empty() { body[0] } else { b'I' };
                break;
            }
            b'N' => {}
            _ => {}
        }
    }

    match error_msg {
        Some(msg) => Err(msg),
        None => Ok(rows),
    }
}

/// Close a native PG connection.
pub fn native_pg_close(mut conn: NativePgConn) {
    let mut buf = Vec::new();
    write_terminate(&mut buf);
    let _ = conn.inner.stream.write_all(&buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::mesh_bytes_new;
    use crate::collections::list::{mesh_list_append, mesh_list_new};
    use crate::collections::map::{mesh_map_entry_value, mesh_map_size};
    use crate::gc::mesh_rt_init;

    #[test]
    fn typed_bind_keeps_binary_parameter_bytes_raw() {
        let params = [
            BindValue::Text(b"inbox"),
            BindValue::Binary(&[0, 0xff, 0x80]),
            BindValue::Null,
        ];
        let mut message = Vec::new();

        write_bind_values(&mut message, &params, &[]).unwrap();

        assert_eq!(message[0], b'B');
        assert!(message.windows(3).any(|bytes| bytes == [0, 0xff, 0x80]));
        assert!(!message.windows(4).any(|bytes| bytes == b"AP+A"));
        assert_eq!(&message[7..15], &[0, 3, 0, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn typed_bind_rejects_aggregate_message_before_encoding() {
        let chunk = vec![0; MAX_DB_VALUE_BYTES];
        let params = [BindValue::Binary(&chunk); 4];
        let mut message = vec![b'X'];

        let error = write_bind_values(&mut message, &params, &[]).unwrap_err();

        assert!(error.contains("Bind message exceeds"));
        assert_eq!(message, [b'X']);
    }

    #[test]
    fn mesh_db_values_extract_text_binary_and_null_without_conversion() {
        mesh_rt_init();
        let text = mesh_string_new(b"inbox".as_ptr(), 5) as *mut u8;
        let binary = mesh_bytes_new([0, 0xff, 0x80].as_ptr(), 3) as *mut u8;
        let values = [
            Box::into_raw(Box::new(MeshDbValue {
                tag: DB_VALUE_TEXT,
                payload: text,
            })),
            Box::into_raw(Box::new(MeshDbValue {
                tag: DB_VALUE_BINARY,
                payload: binary,
            })),
            Box::into_raw(Box::new(MeshDbValue {
                tag: DB_VALUE_NULL,
                payload: std::ptr::null_mut(),
            })),
        ];
        let params = values.iter().fold(mesh_list_new(), |list, value| {
            mesh_list_append(list, *value as u64)
        });

        let extracted = unsafe { extract_db_values(params) }.unwrap();

        assert!(matches!(extracted[0], BindValue::Text(b"inbox")));
        assert!(matches!(
            extracted[1],
            BindValue::Binary(bytes) if bytes == [0, 0xff, 0x80]
        ));
        assert!(matches!(extracted[2], BindValue::Null));
    }

    #[test]
    fn typed_row_decodes_bytea_as_raw_bytes_and_other_columns_as_text() {
        fn column(body: &mut Vec<u8>, name: &str, oid: u32) {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(&0_u32.to_be_bytes()); // table oid
            body.extend_from_slice(&0_i16.to_be_bytes()); // attribute number
            body.extend_from_slice(&oid.to_be_bytes());
            body.extend_from_slice(&(-1_i16).to_be_bytes()); // type size
            body.extend_from_slice(&(-1_i32).to_be_bytes()); // type modifier
            body.extend_from_slice(&0_i16.to_be_bytes()); // statement description format
        }

        let mut description = 3_i16.to_be_bytes().to_vec();
        column(&mut description, "label", 25);
        column(&mut description, "payload", 17);
        column(&mut description, "gone", 17);
        let columns = parse_row_description(&description).unwrap();

        let mut row = 3_i16.to_be_bytes().to_vec();
        row.extend_from_slice(&5_i32.to_be_bytes());
        row.extend_from_slice(b"inbox");
        row.extend_from_slice(&3_i32.to_be_bytes());
        row.extend_from_slice(&[0, 0xff, 0x80]);
        row.extend_from_slice(&(-1_i32).to_be_bytes());

        let values = parse_typed_row(&row, &columns).unwrap();

        assert!(matches!(values[0], RowValue::Text("inbox")));
        assert!(matches!(
            values[1],
            RowValue::Binary(bytes) if bytes == [0, 0xff, 0x80]
        ));
        assert!(matches!(values[2], RowValue::Null));
    }

    #[test]
    fn typed_row_map_keeps_the_last_duplicate_column_value() {
        mesh_rt_init();
        let columns = [
            PgColumn {
                name: "payload".to_string(),
                oid: 25,
            },
            PgColumn {
                name: "payload".to_string(),
                oid: 17,
            },
        ];
        let mut row = 2_i16.to_be_bytes().to_vec();
        row.extend_from_slice(&3_i32.to_be_bytes());
        row.extend_from_slice(b"old");
        row.extend_from_slice(&1_i32.to_be_bytes());
        row.push(0xff);

        let map = unsafe { typed_row_to_map(&row, &columns) }.unwrap();
        let value = mesh_map_entry_value(map, 0) as *const MeshDbValue;

        assert_eq!(mesh_map_size(map), 1);
        assert_eq!(unsafe { (*value).tag }, DB_VALUE_BINARY);
    }

    #[test]
    fn typed_row_rejects_oversized_and_truncated_cells() {
        let columns = [PgColumn {
            name: "payload".to_string(),
            oid: 17,
        }];
        let mut oversized = 1_i16.to_be_bytes().to_vec();
        oversized.extend_from_slice(&((MAX_DB_VALUE_BYTES + 1) as i32).to_be_bytes());

        let oversized_error = parse_typed_row(&oversized, &columns).unwrap_err();

        assert!(oversized_error.contains("exceeds"));

        let mut truncated = 1_i16.to_be_bytes().to_vec();
        truncated.extend_from_slice(&3_i32.to_be_bytes());
        truncated.push(0xff);

        let truncated_error = parse_typed_row(&truncated, &columns).unwrap_err();

        assert!(truncated_error.contains("truncated"));
    }

    #[test]
    fn typed_result_budget_rejects_aggregate_rows() {
        assert_eq!(
            add_result_bytes(MAX_PG_RESULT_BYTES - 1, 1).unwrap(),
            MAX_PG_RESULT_BYTES
        );
        assert!(add_result_bytes(MAX_PG_RESULT_BYTES, 1).is_err());
        assert!(add_result_bytes(usize::MAX, 1).is_err());
    }

    #[test]
    fn typed_result_budget_counts_decoded_null_row_overhead() {
        let columns = [PgColumn {
            name: "payload".to_string(),
            oid: 17,
        }];
        let mut row = 1_i16.to_be_bytes().to_vec();
        row.extend_from_slice(&(-1_i32).to_be_bytes());

        let cost = typed_row_result_cost(&row, &columns).unwrap();

        assert!(cost > row.len());
        assert!(add_result_bytes(MAX_PG_RESULT_BYTES - cost + 1, cost).is_err());
    }
}
