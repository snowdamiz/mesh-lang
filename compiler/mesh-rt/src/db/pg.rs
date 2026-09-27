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
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use pbkdf2::pbkdf2_hmac;
use rand::Rng;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::{pem::PemObject, CertificateDer, ServerName};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::collections::list::{
    mesh_list_append, mesh_list_from_array, mesh_list_get, mesh_list_length, mesh_list_new,
};
use crate::collections::map::mesh_map_from_string_entries;
use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, err_result, MeshResult};
use crate::string::text_of;
use crate::string::{mesh_str, MeshString};

type HmacSha256 = Hmac<Sha256>;

// ponytail: fixed safety caps; make these pool options if legitimate workloads need larger cells.
pub(crate) const MAX_DB_VALUE_BYTES: usize = 16 * 1024 * 1024;
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
        // Flushed: TLS may hold what it was given, and report failing to
        // send it only on the next read.
        let result = self
            .stream
            .write_all(bytes)
            .and_then(|()| self.stream.flush());
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
    /// A PEM file of CA certificates to trust besides the public roots.
    sslrootcert: Option<String>,
}

/// Percent-decode a URL component (handles %XX sequences).
fn percent_decode(s: &str) -> String {
    let mut result = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Exactly two hex digits: `from_str_radix` would take "+1" too.
        if let Some([b'%', high, low]) = bytes.get(i..i + 3) {
            if let (Some(high), Some(low)) =
                ((*high as char).to_digit(16), (*low as char).to_digit(16))
            {
                result.push((high * 16 + low) as u8);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

/// Parse an sslmode. Mesh verifies every TLS server's certificate, so
/// `require` is what libpq calls `verify-full`, and `verify-ca` and
/// `verify-full` are that too: read as `prefer`, as they were, a server (or
/// anything between) declining TLS got the connection in the clear.
fn parse_sslmode(value: &str) -> Result<SslMode, String> {
    match value {
        "disable" => Ok(SslMode::Disable),
        "allow" | "prefer" => Ok(SslMode::Prefer),
        "require" | "verify-ca" | "verify-full" => Ok(SslMode::Require),
        other => Err(format!("unsupported sslmode: {other}")),
    }
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
    let mut sslmode = SslMode::Prefer;
    let mut sslrootcert = None;
    for param in query_str.split('&') {
        match param.split_once('=') {
            Some(("sslmode", value)) => sslmode = parse_sslmode(value)?,
            Some(("sslrootcert", path)) => sslrootcert = Some(percent_decode(path)),
            _ => {}
        }
    }

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

    // An IPv6 address is bracketed: [::1] or [::1]:5432.
    let (host, port) = match host_port
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
    {
        Some((host, "")) => (host, None),
        Some((host, port)) => (host, Some(port.strip_prefix(':').unwrap_or(port))),
        None => match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        },
    };
    let port = match port {
        Some(port) => port
            .parse::<u16>()
            .map_err(|_| format!("invalid port: {}", port))?,
        None => 5432,
    };
    let host = host.to_string();

    Ok(PgUrl {
        host,
        port,
        user,
        password,
        database,
        sslmode,
        sslrootcert,
    })
}

// ── Wire Protocol Helpers ──────────────────────────────────────────────

/// Write a StartupMessage to a buffer.
/// Format: Int32(length) Int32(196608=v3.0) String("user") String(username)
///         String("database") String(dbname)
///         String("client_encoding") String("UTF8") Byte1(0)
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
    // Mesh text is UTF-8: the server converts to and from the database's
    // encoding, which it otherwise assumes the client speaks.
    body.extend_from_slice(b"client_encoding\0UTF8\0");
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

#[derive(Clone, Copy)]
pub(crate) enum BindValue<'a> {
    Text(&'a [u8]),
    Binary(&'a [u8]),
    Null,
}

/// Write a Bind of the unnamed statement: each parameter in text or binary
/// format as its value is, and the result columns in `result_formats` (none:
/// all text), which come from a RowDescription and so number at most
/// `MAX_PG_VALUES`, each 0 or 1.
fn write_bind_values(
    buf: &mut Vec<u8>,
    params: &[BindValue<'_>],
    result_formats: &[i16],
) -> Result<(), String> {
    if params.len() > MAX_PG_VALUES {
        return Err(format!(
            "too many PostgreSQL parameters: {} (maximum {MAX_PG_VALUES})",
            params.len()
        ));
    }
    // Sized in full before any value is copied: values each within their
    // limit can still make a message past the protocol's. At most
    // MAX_PG_VALUES values of MAX_DB_VALUE_BYTES each, the sum cannot
    // overflow a 64-bit usize.
    let mut body_len = 8 + 2 * params.len() + 2 * result_formats.len();
    for (index, param) in params.iter().enumerate() {
        let value_len = match param {
            BindValue::Text(bytes) | BindValue::Binary(bytes) => bytes.len(),
            BindValue::Null => 0,
        };
        if value_len > MAX_DB_VALUE_BYTES {
            return Err(format!(
                "PostgreSQL parameter at index {index} exceeds {MAX_DB_VALUE_BYTES} byte limit"
            ));
        }
        body_len += 4 + value_len;
    }
    let message_len = body_len + 4;
    if message_len > MAX_PG_MESSAGE_BYTES {
        return Err(format!(
            "PostgreSQL Bind message exceeds {MAX_PG_MESSAGE_BYTES} byte limit"
        ));
    }

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
    let len: i32 = 4 + 1 + 1; // length_field + 'P' + null byte
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
    let len: i32 = body_len + 4;
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

/// Upgrade a TCP stream to a TLS-wrapped stream using rustls, trusting the
/// public roots and the CAs in `root_cert` (a PEM file), if any.
fn upgrade_to_tls(
    stream: TcpStream,
    hostname: &str,
    root_cert: Option<&str>,
) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
    let mut root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = root_cert {
        let unreadable = |error: &dyn std::fmt::Display| format!("sslrootcert {path}: {error}");
        let certs = CertificateDer::pem_file_iter(path).map_err(|e| unreadable(&e))?;
        for cert in certs {
            let cert = cert.map_err(|e| unreadable(&e))?;
            root_store.add(cert).map_err(|e| unreadable(&e))?;
        }
    }
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
fn negotiate_tls(mut stream: TcpStream, url: &PgUrl) -> Result<PgStream, String> {
    let sslmode = url.sslmode;
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
            let tls = upgrade_to_tls(stream, &url.host, url.sslrootcert.as_deref())?;
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

/// A server must prove knowledge of the password before the connection is usable.
fn verify_scram_server_final(body: &[u8], expected: &[u8]) -> Result<(), String> {
    let signature = std::str::from_utf8(body)
        .ok()
        .and_then(|message| message.strip_prefix("v="))
        .and_then(|encoded| BASE64.decode(encoded).ok())
        .ok_or_else(|| "SCRAM: invalid server signature".to_string())?;
    if !bool::from(signature.as_slice().ct_eq(expected)) {
        return Err("SCRAM: server signature mismatch".to_string());
    }
    Ok(())
}

fn authentication_body(tag: u8, body: &[u8], expected: i32) -> Result<&[u8], String> {
    if tag == b'E' {
        return Err(parse_error_response(body));
    }
    if tag != b'R' || body.get(..4) != Some(expected.to_be_bytes().as_slice()) {
        return Err(format!("expected authentication message {expected}"));
    }
    Ok(&body[4..])
}

// ── Error Response Parsing ─────────────────────────────────────────────

/// Structured PostgreSQL error extracted from an ErrorResponse message.
///
/// Contains SQLSTATE code, human-readable message, and optional constraint/table/column info
/// for mapping database constraint violations to user-friendly changeset errors.
struct PgError {
    sqlstate: String,           // 'C' field (e.g., "23505")
    message: String,            // 'M' field
    constraint: Option<String>, // 'n' field
    table: Option<String>,      // 't' field
    column: Option<String>,     // 'c' field
}

/// Parse all tagged fields from an ErrorResponse body.
///
/// The body format is: `[field_type_byte][null_terminated_string]...[0]`
/// This is the same format the existing `parse_error_response()` uses but extracts
/// additional fields beyond just the message ('M').
fn parse_error_response_full(body: &[u8]) -> PgError {
    let mut sqlstate = String::new();
    let mut message = String::new();
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
            b'n' => constraint = Some(value),
            b't' => table = Some(value),
            b'c' => column = Some(value),
            _ => {} // skip other fields (S, V, D, P, q, W, etc.)
        }
        i += 1; // skip null terminator
    }

    if message.is_empty() {
        message = "unknown PostgreSQL error".to_string();
    }

    PgError {
        sqlstate,
        message,
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
    /// Sent in binary format (a BYTEA column of a typed query).
    binary: bool,
}

const BYTEA_OID: u32 = 17;

#[derive(Debug, PartialEq, Eq)]
enum RowValue<'a> {
    Text(&'a [u8]),
    Binary(&'a [u8]),
    Null,
}

impl RowValue<'_> {
    /// The value as the text APIs give it: NULL as "", and bytes that are
    /// not UTF-8 with U+FFFD.
    fn lossy(&self) -> std::borrow::Cow<'_, str> {
        match self {
            RowValue::Text(bytes) | RowValue::Binary(bytes) => String::from_utf8_lossy(bytes),
            RowValue::Null => "".into(),
        }
    }
}

fn parse_row_description(body: &[u8]) -> Result<Vec<PgColumn>, String> {
    if body.len() < 2 {
        return Err("invalid PostgreSQL RowDescription".to_string());
    }
    let count = i16::from_be_bytes([body[0], body[1]]);
    if count < 0 {
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
        // Table OID, attribute number, type OID, size, modifier, format.
        let fields = body
            .get(offset..offset + 18)
            .ok_or_else(|| "truncated PostgreSQL RowDescription".to_string())?;
        let oid = u32::from_be_bytes([fields[6], fields[7], fields[8], fields[9]]);
        let binary = i16::from_be_bytes([fields[16], fields[17]]) == 1;
        columns.push(PgColumn { name, oid, binary });
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
        let end = offset + length as usize;
        let bytes = body
            .get(offset..end)
            .ok_or_else(|| format!("truncated PostgreSQL column `{}`", column.name))?;
        offset = end;
        values.push(if column.binary {
            RowValue::Binary(bytes)
        } else {
            RowValue::Text(bytes)
        });
    }
    if offset != body.len() {
        return Err("trailing bytes in PostgreSQL DataRow".to_string());
    }
    Ok(values)
}

fn validate_sql(sql: &str) -> Result<(), String> {
    if sql.as_bytes().contains(&0) {
        return Err("PostgreSQL query contains a NUL byte".to_string());
    }
    if sql.len() + 9 > MAX_PG_MESSAGE_BYTES {
        return Err(format!(
            "PostgreSQL query exceeds {MAX_PG_MESSAGE_BYTES} byte message limit"
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

/// What a decoded row costs besides its DataRow's bytes: the result-list
/// slot, the map, GC headers, keys, tagged values and a payload allocation
/// for every cell. The wire body already counts each non-null payload, so
/// nulls are deliberately over-counted. (Column names come from one
/// bounded message, so the sum cannot overflow.)
fn decoded_row_bytes(columns: &[PgColumn]) -> usize {
    40 + columns
        .iter()
        .map(|column| 96 + column.name.len())
        .sum::<usize>()
}

pub(crate) unsafe fn alloc_db_value(tag: u8, payload: *mut u8) -> *mut MeshDbValue {
    let value = mesh_gc_alloc_actor(
        std::mem::size_of::<MeshDbValue>() as u64,
        std::mem::align_of::<MeshDbValue>() as u64,
    ) as *mut MeshDbValue;
    value.write(MeshDbValue { tag, payload });
    value
}

/// A row as a `Map<String, String>` or, typed, a `Map<String, DbValue>`. A
/// later column of the same name wins.
unsafe fn row_map(columns: &[PgColumn], row: Vec<RowValue>, values: Values) -> Result<u64, String> {
    let mut entries = Vec::<[u64; 2]>::with_capacity(columns.len());
    let mut indexes = HashMap::<&str, usize>::with_capacity(columns.len());
    for (column, value) in columns.iter().zip(row) {
        let value = match (values, value) {
            (Values::Text, value) => mesh_str(&value.lossy()) as u64,
            (Values::Typed, RowValue::Text(bytes)) => {
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    format!("PostgreSQL text column `{}` is not UTF-8", column.name)
                })?;
                alloc_db_value(DB_VALUE_TEXT, mesh_str(text) as *mut u8) as u64
            }
            (Values::Typed, RowValue::Binary(bytes)) => alloc_db_value(
                DB_VALUE_BINARY,
                mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64) as *mut u8,
            ) as u64,
            (Values::Typed, RowValue::Null) => {
                alloc_db_value(DB_VALUE_NULL, std::ptr::null_mut()) as u64
            }
        };
        if let Some(index) = indexes.get(column.name.as_str()).copied() {
            entries[index][1] = value;
        } else {
            indexes.insert(column.name.as_str(), entries.len());
            entries.push([mesh_str(&column.name) as u64, value]);
        }
    }
    Ok(mesh_map_from_string_entries(&entries) as u64)
}

// ── Requests ───────────────────────────────────────────────────────────

/// How a statement's parameters and rows are typed: text (`List<String>`
/// parameters, every column in text format) or `DbValue`s (BYTEA columns in
/// binary format).
#[derive(Clone, Copy, PartialEq)]
enum Values {
    Text,
    Typed,
}

/// Why a request failed: the server's ErrorResponse, or anything else (the
/// connection failing, a reply the driver cannot decode, a limit).
enum Failure {
    Server(PgError),
    Other(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Other(message)
    }
}

impl Failure {
    /// The failure as the Mesh query APIs report it: a server error in the
    /// structured form Repo maps constraint violations from.
    fn structured(self) -> String {
        match self {
            Failure::Server(error) => format_pg_error_string(&error),
            Failure::Other(message) => message,
        }
    }

    /// Only the message.
    fn message(self) -> String {
        match self {
            Failure::Server(error) => error.message,
            Failure::Other(message) => message,
        }
    }
}

/// Send `message` and read the replies up to ReadyForQuery, which gives the
/// connection its transaction status, passing each RowDescription, NoData
/// and DataRow to `rows` as (tag, body). Returns the row count of the last
/// CommandComplete, or the first failure, the server's or `rows`'s. Either
/// way every reply is read, which leaves the connection ready for the next
/// request unless it broke. `what` names the request in I/O errors.
fn request(
    conn: &mut PgConn,
    what: &str,
    message: &[u8],
    mut rows: impl FnMut(u8, &[u8]) -> Result<(), String>,
) -> Result<i64, Failure> {
    send(conn, what, message)?;
    let mut count = 0;
    let mut failure = None;
    loop {
        let (tag, body) = conn
            .read_wire_message()
            .map_err(|error| format!("read {what}: {error}"))?;
        match tag {
            b'C' => {
                count = parse_command_tag(String::from_utf8_lossy(&body).trim_end_matches('\0'))
            }
            b'E' if failure.is_none() => {
                failure = Some(Failure::Server(parse_error_response_full(&body)))
            }
            b'T' | b'n' | b'D' if failure.is_none() => {
                failure = rows(tag, &body).err().map(Failure::Other)
            }
            b'Z' => {
                conn.txn_status = body.first().copied().unwrap_or(b'I');
                return failure.map_or(Ok(count), Err);
            }
            // CopyInResponse: COPY FROM STDIN waits for rows, and the driver
            // has none to send. It is refused; the server ignores the Sync
            // that ended the request while it copies, so another follows.
            // (Only the extended protocol gets here: the simple commands
            // never copy.)
            b'G' => {
                let reason = b"COPY FROM STDIN is not supported\0";
                let mut refusal = vec![b'f'];
                refusal.extend_from_slice(&(4 + reason.len() as i32).to_be_bytes());
                refusal.extend_from_slice(reason);
                write_sync(&mut refusal);
                send(conn, what, &refusal)?;
            }
            // ParseComplete, BindComplete, ParameterDescription, notices,
            // COPY TO STDOUT's data, and what follows a failure.
            _ => {}
        }
    }
}

fn send(conn: &mut PgConn, what: &str, message: &[u8]) -> Result<(), String> {
    conn.write_wire_all(message)
        .map_err(|error| format!("send {what}: {error}"))
}

/// Parse `sql` as the unnamed statement, which it stays for the Bind that
/// follows: the columns it returns.
fn prepare(conn: &mut PgConn, sql: &str) -> Result<Vec<PgColumn>, Failure> {
    let mut message = Vec::new();
    write_parse(&mut message, sql);
    write_describe_statement(&mut message);
    write_sync(&mut message);
    let mut columns = None;
    request(conn, "prepare", &message, |tag, body| {
        // A RowDescription, or NoData.
        columns = Some(if tag == b'T' {
            parse_row_description(body)?
        } else {
            Vec::new()
        });
        Ok(())
    })?;
    Ok(columns.ok_or_else(|| "PostgreSQL prepare returned no row description".to_string())?)
}

/// Run `sql` with `params` for its effect: the number of rows it affected.
fn execute(conn: &mut PgConn, sql: &str, params: &[BindValue]) -> Result<i64, Failure> {
    validate_sql(sql)?;
    let mut message = Vec::new();
    write_parse(&mut message, sql);
    write_bind_values(&mut message, params, &[])?;
    write_execute(&mut message);
    write_sync(&mut message);
    request(conn, "execute", &message, |_, _| Ok(()))
}

/// Run a query with `params`: its rows, each decoded by `decode` from the
/// columns and the row's values, within the row and byte limits.
fn query<R>(
    conn: &mut PgConn,
    sql: &str,
    params: &[BindValue],
    values: Values,
    mut decode: impl FnMut(&[PgColumn], Vec<RowValue>) -> Result<R, String>,
) -> Result<Vec<R>, Failure> {
    validate_sql(sql)?;
    let mut message = Vec::new();
    let formats: Vec<i16> = match values {
        // The Bind names each column's format, so the columns come first.
        Values::Typed => prepare(conn, sql)?
            .iter()
            .map(|column| i16::from(column.oid == BYTEA_OID))
            .collect(),
        Values::Text => {
            write_parse(&mut message, sql);
            Vec::new()
        }
    };
    write_bind_values(&mut message, params, &formats)?;
    write_describe_portal(&mut message);
    write_execute(&mut message);
    write_sync(&mut message);

    let mut columns = Vec::new();
    let mut row_bytes = decoded_row_bytes(&columns);
    let mut rows = Vec::new();
    // The final Result and list allocations, with their GC headers.
    let mut result_bytes = 64;
    request(conn, "query", &message, |tag, body| {
        match tag {
            b'T' => {
                columns = parse_row_description(body)?;
                row_bytes = decoded_row_bytes(&columns);
            }
            b'D' if rows.len() == MAX_PG_ROWS => {
                return Err(format!("PostgreSQL result exceeds {MAX_PG_ROWS} row limit"))
            }
            b'D' => {
                result_bytes = add_result_bytes(result_bytes, row_bytes + body.len())?;
                rows.push(decode(&columns, parse_typed_row(body, &columns)?)?);
            }
            _ => {} // NoData: a statement that returns no rows.
        }
        Ok(())
    })?;
    Ok(rows)
}

/// Run `sql` through the Simple Query protocol (BEGIN, COMMIT, ROLLBACK and
/// the pool's health check), ignoring any rows: the server's message when
/// it fails.
pub(super) fn pg_simple_command(conn: &mut PgConn, sql: &str) -> Result<(), String> {
    let mut message = vec![b'Q'];
    message.extend_from_slice(&(sql.len() as i32 + 5).to_be_bytes());
    message.extend_from_slice(sql.as_bytes());
    message.push(0);
    request(conn, sql, &message, |_, _| Ok(()))
        .map(drop)
        .map_err(Failure::message)
}

/// Say goodbye (Terminate) and close the connection.
fn close(mut conn: PgConn) {
    let mut message = Vec::new();
    write_terminate(&mut message);
    let _ = conn.write_wire_all(&message);
}

// ── MeshString / MeshResult Helpers ────────────────────────────────────

/// The values a `List<DbValue>` holds, at most `maximum` of them and each
/// within the byte limit; `database` names the driver in the errors.
pub(crate) unsafe fn db_values<'a>(
    params: *mut u8,
    maximum: usize,
    database: &str,
) -> Result<Vec<BindValue<'a>>, String> {
    list_values(params, maximum, database, |slot| {
        let value = &*(slot as *const MeshDbValue);
        match value.tag {
            DB_VALUE_TEXT => {
                BindValue::Text((*(value.payload as *const MeshString)).as_str().as_bytes())
            }
            DB_VALUE_BINARY => BindValue::Binary((*(value.payload as *const MeshBytes)).as_slice()),
            // `DB_VALUE_NULL`, the one other tag a DbValue has.
            _ => BindValue::Null,
        }
    })
}

/// A `List<String>` as text values, bounded as `db_values` bounds them.
pub(crate) unsafe fn text_values<'a>(
    params: *mut u8,
    maximum: usize,
    database: &str,
) -> Result<Vec<BindValue<'a>>, String> {
    list_values(params, maximum, database, |slot| {
        BindValue::Text((*(slot as *const MeshString)).as_str().as_bytes())
    })
}

unsafe fn list_values<'a>(
    params: *mut u8,
    maximum: usize,
    database: &str,
    decode: impl Fn(u64) -> BindValue<'a>,
) -> Result<Vec<BindValue<'a>>, String> {
    // Through `list_slots`: the list may be a view of another's buffer.
    let (len, slots) = crate::collections::list::list_slots(params);
    if len > maximum {
        return Err(format!(
            "too many {database} parameters: {len} (maximum {maximum})"
        ));
    }
    (0..len)
        .map(|index| {
            let value = decode(*slots.add(index));
            if let BindValue::Text(bytes) | BindValue::Binary(bytes) = value {
                if bytes.len() > MAX_DB_VALUE_BYTES {
                    return Err(format!(
                        "{database} parameter at index {index} exceeds {MAX_DB_VALUE_BYTES} byte limit"
                    ));
                }
            }
            Ok(value)
        })
        .collect()
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
    match open(unsafe { text_of(url) }) {
        // Result payloads with integer semantics are represented by pointers
        // to boxed integers, as SQLite's handles are.
        Ok(handle) => alloc_result(0, crate::io::box_scalar(handle)) as *mut u8,
        Err(error) => err_result(&error),
    }
}

/// A connection to `url`, as the handle Mesh holds: a `Box<PgConn>`.
pub(super) fn open(url: &str) -> Result<u64, String> {
    connect(url).map(|conn| Box::into_raw(Box::new(conn)) as u64)
}

/// Connect, authenticate and wait for the server to be ready: the handshake
/// `Pg.connect` and the native API share.
fn connect(url: &str) -> Result<PgConn, String> {
    let pg_url = parse_pg_url(url)?;

    let addrs = (pg_url.host.as_str(), pg_url.port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed: {}", e))?;
    // Each address in turn, as libpq tries them: `localhost` can name ::1
    // first for a server that listens on 127.0.0.1 alone.
    let mut failure = "could not resolve host".to_string();
    let stream = addrs
        .filter_map(|addr| {
            TcpStream::connect_timeout(&addr, Duration::from_secs(10))
                .map_err(|e| failure = format!("connection failed: {}", e))
                .ok()
        })
        .next();
    let stream = stream.ok_or(failure)?;
    // Set before TLS wrapping (StreamOwned inherits them).
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let mut stream = negotiate_tls(stream, &pg_url).map_err(|e| format!("TLS: {}", e))?;

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
            authentication_body(tag, &body, 0)?;
        }
        5 => {
            // MD5Password
            let salt = body.get(4..8).ok_or("MD5 auth: missing salt")?;
            let hashed = compute_md5_password(&pg_url.user, &pg_url.password, salt);
            let mut buf = Vec::new();
            write_password_message(&mut buf, &hashed);
            stream
                .write_all(&buf)
                .map_err(|e| format!("send md5: {}", e))?;
            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read md5 response: {}", e))?;
            authentication_body(tag, &body, 0)?;
        }
        10 => {
            // SASL: the body lists the server's mechanisms.
            if !body[4..]
                .split(|byte| *byte == 0)
                .any(|mechanism| mechanism == b"SCRAM-SHA-256")
            {
                return Err("server does not support SCRAM-SHA-256".to_string());
            }
            let (client_first, client_nonce) = scram_client_first(&pg_url.user);
            let mut buf = Vec::new();
            write_sasl_initial_response(&mut buf, "SCRAM-SHA-256", client_first.as_bytes());
            stream
                .write_all(&buf)
                .map_err(|e| format!("send SASL init: {}", e))?;

            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read SASL continue: {}", e))?;
            let server_first = std::str::from_utf8(authentication_body(tag, &body, 11)?)
                .map_err(|_| "invalid SCRAM server-first encoding")?;
            let (client_final, expected_sig) =
                scram_client_final(&pg_url.password, &client_nonce, server_first)?;

            let mut buf = Vec::new();
            write_sasl_response(&mut buf, client_final.as_bytes());
            stream
                .write_all(&buf)
                .map_err(|e| format!("send SASL final: {}", e))?;

            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read SASL final: {}", e))?;
            verify_scram_server_final(authentication_body(tag, &body, 12)?, &expected_sig)?;
            let (tag, body) =
                read_message(&mut stream).map_err(|e| format!("read auth ok: {}", e))?;
            authentication_body(tag, &body, 0)?;
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

    Ok(PgConn {
        stream,
        txn_status,
        broken: false,
    })
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
    close(*unsafe { Box::from_raw(conn_handle as *mut PgConn) });
}

/// The parameters a `List<String>` or `List<DbValue>` holds.
unsafe fn bind_params<'a>(params: *mut u8, values: Values) -> Result<Vec<BindValue<'a>>, Failure> {
    let params = match values {
        Values::Text => text_values(params, MAX_PG_VALUES, "PostgreSQL"),
        Values::Typed => db_values(params, MAX_PG_VALUES, "PostgreSQL"),
    };
    Ok(params?)
}

/// `Pg.execute` and `Pg.execute_values`: `Ok(rows affected)`.
unsafe fn mesh_execute(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
    values: Values,
) -> *mut u8 {
    let conn = &mut *(conn_handle as *mut PgConn);
    match bind_params(params, values).and_then(|params| execute(conn, text_of(sql), &params)) {
        Ok(count) => crate::io::ok_int(count).cast(),
        Err(failure) => err_result(&failure.structured()),
    }
}

/// `Pg.query` and `Pg.query_values`: `Ok(rows)`.
unsafe fn mesh_query(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
    values: Values,
) -> *mut u8 {
    let conn = &mut *(conn_handle as *mut PgConn);
    let rows = bind_params(params, values).and_then(|params| {
        query(conn, text_of(sql), &params, values, |columns, row| {
            row_map(columns, row, values)
        })
    });
    match rows {
        Ok(rows) => {
            alloc_result(0, mesh_list_from_array(rows.as_ptr(), rows.len() as i64)) as *mut u8
        }
        Err(failure) => err_result(&failure.structured()),
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
    unsafe { mesh_execute(conn_handle, sql, params, Values::Text) }
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
    unsafe { mesh_query(conn_handle, sql, params, Values::Text) }
}

/// Execute an unnamed prepared statement with typed `DbValue` parameters.
#[no_mangle]
pub extern "C" fn mesh_pg_execute_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { mesh_execute(conn_handle, sql, params, Values::Typed) }
}

/// Query typed values while keeping non-BYTEA columns in PostgreSQL text format.
#[no_mangle]
pub extern "C" fn mesh_pg_query_values(
    conn_handle: u64,
    sql: *const MeshString,
    params: *mut u8,
) -> *mut u8 {
    unsafe { mesh_query(conn_handle, sql, params, Values::Typed) }
}

// ── Transaction Management ─────────────────────────────────────────────

/// `Ok(())`, or the error.
fn unit_result(result: Result<(), String>) -> *mut u8 {
    match result {
        Ok(()) => alloc_result(0, std::ptr::null_mut()) as *mut u8,
        Err(error) => err_result(&error),
    }
}

/// COMMIT. A transaction a failed statement aborted cannot commit:
/// PostgreSQL rolls it back instead, without an error, so it is one here.
fn commit(conn: &mut PgConn) -> Result<(), String> {
    let failed = conn.txn_status == b'E';
    pg_simple_command(conn, "COMMIT")?;
    if failed {
        return Err("the transaction failed, so COMMIT rolled it back".to_string());
    }
    Ok(())
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
    let conn = unsafe { &mut *(conn_handle as *mut PgConn) };
    unit_result(pg_simple_command(conn, "BEGIN"))
}

/// Commit a PostgreSQL transaction.
///
/// # Signature
///
/// `mesh_pg_commit(conn_handle: u64) -> *mut u8 (MeshResult<Unit, String>)`
///
/// Sends `COMMIT` and returns Ok(()) or Err(error_message), an error too
/// when a failed statement had aborted the transaction.
#[no_mangle]
pub extern "C" fn mesh_pg_commit(conn_handle: u64) -> *mut u8 {
    let conn = unsafe { &mut *(conn_handle as *mut PgConn) };
    unit_result(commit(conn))
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
    let conn = unsafe { &mut *(conn_handle as *mut PgConn) };
    unit_result(pg_simple_command(conn, "ROLLBACK"))
}

pub(crate) unsafe fn invoke_transaction_callback(
    fn_ptr: *const u8,
    env_ptr: *const u8,
    conn_handle: u64,
) -> *mut u8 {
    let result = if env_ptr.is_null() {
        let callback: extern "C-unwind" fn(u64) -> MeshResult = std::mem::transmute(fn_ptr);
        callback(conn_handle)
    } else {
        let callback: extern "C-unwind" fn(*const u8, u64) -> MeshResult =
            std::mem::transmute(fn_ptr);
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
                    if let Err(e) = commit(conn) {
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

/// Decode every row of an Ok `query_result` with the callback `fn_ptr`,
/// called as `fn(env_ptr, row)` (or `fn(row)` for a null `env_ptr`), into a
/// list of what it returns: the uniform slot of a `Result`, as a list holds
/// one. A failed query is passed on as it is.
pub(crate) unsafe fn decode_rows(
    query_result: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    let result = &*(query_result as *const crate::io::MeshResult);
    if result.tag != 0 {
        return query_result;
    }
    let rows = result.value;
    let mut decoded = mesh_list_new();
    for index in 0..mesh_list_length(rows) {
        let row = mesh_list_get(rows, index);
        decoded = mesh_list_append(decoded, crate::callback::call1(fn_ptr, env_ptr, row));
    }
    alloc_result(0, decoded) as *mut u8
}

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
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    unsafe {
        let query_result = mesh_pg_query(conn_handle, sql as *const MeshString, params);
        decode_rows(query_result, fn_ptr, env_ptr)
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
    connect(url).map(|inner| NativePgConn { inner })
}

fn text_params<'a>(params: &[&'a str]) -> Vec<BindValue<'a>> {
    params
        .iter()
        .map(|param| BindValue::Text(param.as_bytes()))
        .collect()
}

/// Execute a SQL statement via native connection. Returns rows affected.
pub fn native_pg_execute(
    conn: &mut NativePgConn,
    sql: &str,
    params: &[&str],
) -> Result<i64, String> {
    execute(&mut conn.inner, sql, &text_params(params)).map_err(Failure::message)
}

/// Execute a SQL query via native connection. Returns rows as Vec of
/// column-value pairs, a NULL as "".
pub fn native_pg_query(
    conn: &mut NativePgConn,
    sql: &str,
    params: &[&str],
) -> Result<Vec<Vec<(String, String)>>, String> {
    query(
        &mut conn.inner,
        sql,
        &text_params(params),
        Values::Text,
        |columns, row| {
            Ok(columns
                .iter()
                .zip(row)
                .map(|(column, value)| (column.name.clone(), value.lossy().into_owned()))
                .collect())
        },
    )
    .map_err(Failure::message)
}

/// Close a native PG connection.
pub fn native_pg_close(conn: NativePgConn) {
    close(conn.inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::mesh_bytes_new;
    use crate::collections::list::{mesh_list_append, mesh_list_new};
    use crate::collections::map::{mesh_map_entry_key, mesh_map_entry_value, mesh_map_size};
    use crate::gc::mesh_rt_init;
    use crate::string::mesh_string_new;

    // A wire-level peer exercises both public connection APIs without a database.
    fn scram_test_server(final_message: Option<&[u8]>) -> (String, std::thread::JoinHandle<()>) {
        scram_test_server_reply(final_message.map(Vec::from), None)
    }

    /// A SCRAM peer that answers the client's final message with `tail`.
    fn scram_test_server_with(tail: Option<Vec<u8>>) -> (String, std::thread::JoinHandle<()>) {
        scram_test_server_reply(None, tail)
    }

    fn scram_test_server_reply(
        final_message: Option<Vec<u8>>,
        tail: Option<Vec<u8>>,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "postgres://user:password@{}/db?sslmode=disable",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut length = [0; 4];
            socket.read_exact(&mut length).unwrap();
            let mut startup = vec![0; u32::from_be_bytes(length) as usize - 4];
            socket.read_exact(&mut startup).unwrap();
            let mut stream = PgStream::Plain(socket);
            fn auth(output: &mut Vec<u8>, kind: i32, data: &[u8]) {
                output.push(b'R');
                output.extend_from_slice(&(8 + data.len() as i32).to_be_bytes());
                output.extend_from_slice(&kind.to_be_bytes());
                output.extend_from_slice(data);
            }
            let mut output = Vec::new();
            auth(&mut output, 10, b"SCRAM-SHA-256\0\0");
            stream.write_all(&output).unwrap();
            let (tag, initial) = read_message(&mut stream).unwrap();
            assert_eq!(tag, b'p');
            let mechanism_end = initial.iter().position(|b| *b == 0).unwrap();
            let first = std::str::from_utf8(&initial[mechanism_end + 5..]).unwrap();
            let nonce = first.strip_prefix("n,,n=,r=").unwrap();
            let challenge = format!("r={nonce}server,s=c2FsdA==,i=4096");
            let (_, signature) = scram_client_final("password", nonce, &challenge).unwrap();
            output.clear();
            auth(&mut output, 11, challenge.as_bytes());
            stream.write_all(&output).unwrap();
            assert_eq!(read_message(&mut stream).unwrap().0, b'p');
            output.clear();
            if let Some(tail) = tail {
                stream.write_all(&tail).unwrap();
                return;
            }
            let valid_final = format!("v={}", BASE64.encode(signature));
            auth(
                &mut output,
                12,
                final_message.as_deref().unwrap_or(valid_final.as_bytes()),
            );
            auth(&mut output, 0, b"");
            output.extend_from_slice(b"Z\0\0\0\x05I");
            stream.write_all(&output).unwrap();
        });
        (url, server)
    }

    fn auth(kind: i32, data: &[u8]) -> Vec<u8> {
        let mut output = vec![b'R'];
        output.extend_from_slice(&(8 + data.len() as i32).to_be_bytes());
        output.extend_from_slice(&kind.to_be_bytes());
        output.extend_from_slice(data);
        output
    }

    fn message(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut output = vec![tag];
        output.extend_from_slice(&(4 + body.len() as i32).to_be_bytes());
        output.extend_from_slice(body);
        output
    }

    /// An ErrorResponse with severity, SQLSTATE, message, detail and
    /// constraint fields.
    fn error_response(message_text: &str) -> Vec<u8> {
        let mut body = Vec::new();
        for (field, value) in [
            (b'S', "FATAL"),
            (b'C', "28P01"),
            (b'M', message_text),
            (b'D', "a detail"),
            (b'n', "a_constraint"),
        ] {
            body.push(field);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        message(b'E', &body)
    }

    fn ready(status: u8) -> Vec<u8> {
        message(b'Z', &[status])
    }

    /// A peer on a loopback port for `url_query`'s connection: `script`
    /// gets the accepted socket.
    fn wire_peer(
        url_query: &str,
        script: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "postgres://user:password@{}/db?{url_query}",
            listener.local_addr().unwrap()
        );
        let peer = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            script(socket);
        });
        (url, peer)
    }

    fn read_startup(socket: &mut TcpStream) {
        let mut length = [0; 4];
        socket.read_exact(&mut length).unwrap();
        let mut startup = vec![0; u32::from_be_bytes(length) as usize - 4];
        socket.read_exact(&mut startup).unwrap();
    }

    /// Connects to a peer that reads the startup message and answers with
    /// `reply`, then ends its side: the connection's transaction status, or
    /// why there is none.
    fn connect_to_reply(reply: Vec<u8>) -> Result<u8, String> {
        let (url, peer) = wire_peer("sslmode=disable", move |mut socket| {
            read_startup(&mut socket);
            socket.write_all(&reply).unwrap();
            socket.shutdown(std::net::Shutdown::Write).unwrap();
            // Drain what the client sends until it closes: closing with its
            // messages unread would reset the connection before it reads.
            while socket.read(&mut [0; 1024]).is_ok_and(|read| read > 0) {}
        });
        let result = connect(&url).map(|conn| conn.txn_status);
        peer.join().unwrap();
        result
    }

    #[test]
    fn connect_reports_each_malformed_or_refused_handshake() {
        let scram = auth(10, b"SCRAM-SHA-256\0\0");
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "error first",
                error_response("password authentication failed"),
                "password authentication failed",
            ),
            (
                "other tag",
                message(b'X', b""),
                "expected auth message, got 'X'",
            ),
            (
                "short auth",
                message(b'R', b"\0\0"),
                "auth message too short",
            ),
            ("unknown auth", auth(7, b""), "unsupported auth type: 7"),
            (
                "error after auth",
                [auth(0, b""), error_response("too many connections")].concat(),
                "too many connections",
            ),
            ("closed after auth", auth(0, b""), "read startup params"),
            (
                "cleartext refused",
                [auth(3, b""), error_response("bad password")].concat(),
                "bad password",
            ),
            (
                "cleartext answered oddly",
                [auth(3, b""), auth(5, b"salt")].concat(),
                "expected authentication message 0",
            ),
            (
                "md5 refused",
                [auth(5, b"salt"), error_response("md5 mismatch")].concat(),
                "md5 mismatch",
            ),
            ("md5 without salt", auth(5, b"sa"), "MD5 auth: missing salt"),
            (
                "no scram",
                auth(10, b"SCRAM-SHA-256-PLUS-ONLY\0\0"),
                "server does not support SCRAM-SHA-256",
            ),
            (
                "scram refused",
                [scram.clone(), error_response("no such role")].concat(),
                "no such role",
            ),
            (
                "scram skips continue",
                [scram.clone(), auth(12, b"v=")].concat(),
                "expected authentication message 11",
            ),
            (
                "scram garbled challenge",
                [scram.clone(), auth(11, b"\xff")].concat(),
                "invalid SCRAM server-first encoding",
            ),
            (
                "scram foreign nonce",
                [scram, auth(11, b"r=someone-else,s=c2FsdA==,i=4096")].concat(),
                "",
            ),
        ];
        for (case, reply, expected) in cases {
            match connect_to_reply(reply) {
                Ok(_) => panic!("{case}: connected"),
                Err(error) => assert!(error.contains(expected), "{case}: {error}"),
            }
        }
    }

    /// An IPv6 host is bracketed in the URL, not in the address or the
    /// name TLS checks the certificate against.
    #[test]
    fn connect_reaches_a_bracketed_ipv6_host() {
        for (url, host, port) in [
            ("postgres://u@[::1]:6543/d", "::1", 6543),
            ("postgres://u@[fe80::1]/d", "fe80::1", 5432),
        ] {
            let url = parse_pg_url(url).unwrap();
            assert_eq!((url.host.as_str(), url.port), (host, port));
        }
        assert_eq!(
            parse_pg_url("postgres://u@[::1]x/d").err().as_deref(),
            Some("invalid port: x")
        );
        // A host without IPv6 loopback (some containers) has nothing to reach.
        let Ok(listener) = std::net::TcpListener::bind("[::1]:0") else {
            return;
        };
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_startup(&mut socket);
            socket
                .write_all(&[auth(0, b""), ready(b'I')].concat())
                .unwrap();
            // TLS: offered, then the connection ends.
            let (mut socket, _) = listener.accept().unwrap();
            socket.read_exact(&mut [0; 8]).unwrap();
            socket.write_all(b"S").unwrap();
        });
        let url = format!("postgres://u@[::1]:{port}/d");
        let plain = connect(&format!("{url}?sslmode=disable")).map(|conn| conn.txn_status);
        assert_eq!(plain, Ok(b'I'));
        let tls = connect(&format!("{url}?sslmode=require")).err().unwrap();
        peer.join().unwrap();
        assert!(!tls.contains("invalid hostname"), "{tls}");
    }

    /// `localhost` can name ::1 first (it does on macOS) for a server that
    /// listens on 127.0.0.1 alone, which libpq reaches all the same.
    #[test]
    fn connect_tries_every_address_the_host_resolves_to() {
        let (url, peer) = wire_peer("sslmode=disable", |mut socket| {
            read_startup(&mut socket);
            socket
                .write_all(&[auth(0, b""), ready(b'I')].concat())
                .unwrap();
        });
        let conn = connect(&url.replace("127.0.0.1", "localhost"));
        // Before the join: a peer never reached waits on.
        assert_eq!(conn.map(|conn| conn.txn_status), Ok(b'I'));
        peer.join().unwrap();
    }

    #[test]
    fn connect_passes_notices_and_parameters_and_keeps_the_transaction_status() {
        for reply in [
            // Trust: no password asked.
            auth(0, b""),
            // Cleartext and md5, each accepted.
            [auth(3, b""), auth(0, b"")].concat(),
            [auth(5, b"salt"), auth(0, b"")].concat(),
        ] {
            let startup = [
                message(b'S', b"server_version\x0016\0"),
                message(b'K', &[0; 8]),
                message(b'N', b"Mnotice\0\0"),
                message(b'A', b"unknown"),
                ready(b'T'),
            ]
            .concat();
            assert_eq!(connect_to_reply([reply, startup].concat()), Ok(b'T'));
        }
    }

    #[test]
    fn connect_sends_md5_of_password_user_and_salt() {
        let (url, peer) = wire_peer("sslmode=disable", |mut socket| {
            read_startup(&mut socket);
            socket.write_all(&auth(5, b"salt")).unwrap();
            let mut stream = PgStream::Plain(socket);
            let (tag, body) = read_message(&mut stream).unwrap();
            assert_eq!(tag, b'p');
            let expected = compute_md5_password("user", "password", b"salt");
            assert_eq!(body, [expected.as_bytes(), b"\0"].concat());
            stream
                .write_all(&[auth(0, b""), ready(b'I')].concat())
                .unwrap();
        });
        let conn = native_pg_connect(&url);
        peer.join().unwrap();
        native_pg_close(conn.unwrap());
    }

    #[test]
    fn scram_connect_requires_the_final_messages_in_order() {
        for (tail, expected) in [
            (auth(11, b"v="), "expected authentication message 12"),
            (error_response("scram failed"), "scram failed"),
        ] {
            let (url, server) = scram_test_server_with(Some(tail));
            let result = native_pg_connect(&url);
            server.join().unwrap();
            let error = result.err().expect("connected");
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn tls_negotiation_follows_sslmode() {
        // require: a server declining TLS is refused.
        let (url, peer) = wire_peer("sslmode=require", |mut socket| {
            socket.read_exact(&mut [0; 8]).unwrap();
            socket.write_all(b"N").unwrap();
        });
        let error = connect(&url).err().unwrap();
        peer.join().unwrap();
        assert_eq!(error, "TLS: server does not support SSL");
        // verify-full is require, never prefer.
        let (url, peer) = wire_peer("sslmode=verify-full", |mut socket| {
            socket.read_exact(&mut [0; 8]).unwrap();
            socket.write_all(b"N").unwrap();
        });
        assert!(connect(&url).is_err());
        peer.join().unwrap();
        // prefer: the connection goes on in the clear.
        let (url, peer) = wire_peer("sslmode=prefer", |mut socket| {
            socket.read_exact(&mut [0; 8]).unwrap();
            socket.write_all(b"N").unwrap();
            read_startup(&mut socket);
            socket
                .write_all(&[auth(0, b""), ready(b'I')].concat())
                .unwrap();
        });
        let conn = connect(&url).unwrap();
        peer.join().unwrap();
        assert!(matches!(conn.stream, PgStream::Plain(_)));
        // Anything but S or N is refused.
        let (url, peer) = wire_peer("sslmode=require", |mut socket| {
            socket.read_exact(&mut [0; 8]).unwrap();
            socket.write_all(b"X").unwrap();
        });
        let error = connect(&url).err().unwrap();
        peer.join().unwrap();
        assert_eq!(error, "TLS: unexpected SSL response: 0x58");
        // S, then no TLS: the handshake fails.
        let (url, peer) = wire_peer("sslmode=require", |mut socket| {
            socket.read_exact(&mut [0; 8]).unwrap();
            socket.write_all(b"S").unwrap();
            let _ = socket.read(&mut [0; 1024]);
        });
        assert!(connect(&url).is_err());
        peer.join().unwrap();
        // An sslrootcert that cannot be read is an error, not the public roots.
        let (url, peer) = wire_peer(
            "sslmode=require&sslrootcert=/nonexistent/ca.pem",
            |mut socket| {
                socket.read_exact(&mut [0; 8]).unwrap();
                socket.write_all(b"S").unwrap();
            },
        );
        let error = connect(&url).err().unwrap();
        peer.join().unwrap();
        assert!(
            error.starts_with("TLS: sslrootcert /nonexistent/ca.pem"),
            "{error}"
        );
    }

    #[test]
    fn urls_parse_credentials_databases_ports_and_tls_options() {
        let url = parse_pg_url(
            "postgresql://us%40er:p%3Ass@db.example:6543/app%2Fdb?sslmode=verify-ca&sslrootcert=%2Fetc%2Fca.pem&application_name=x",
        )
        .unwrap();
        assert_eq!(
            (
                url.user.as_str(),
                url.password.as_str(),
                url.host.as_str(),
                url.port
            ),
            ("us@er", "p:ss", "db.example", 6543)
        );
        assert_eq!(url.database, "app/db");
        assert!(url.sslmode == SslMode::Require);
        assert_eq!(url.sslrootcert.as_deref(), Some("/etc/ca.pem"));
        // Defaults: port 5432, the user's database, prefer; a bad escape stays.
        let url = parse_pg_url("postgres://me%zz@host").unwrap();
        assert_eq!(
            (url.user.as_str(), url.port, url.database.as_str()),
            ("me%zz", 5432, "me%zz")
        );
        assert!(url.sslmode == SslMode::Prefer && url.password.is_empty());
        // Only two hex digits make an escape: "%+1" is not byte 1.
        assert_eq!(percent_decode("a%+1b%4"), "a%+1b%4");
        for (mode, parsed) in [
            ("disable", SslMode::Disable),
            ("allow", SslMode::Prefer),
            ("prefer", SslMode::Prefer),
            ("require", SslMode::Require),
            ("verify-full", SslMode::Require),
        ] {
            assert!(parse_sslmode(mode).unwrap() == parsed, "{mode}");
        }
        for (url, error) in [
            (
                "mysql://u@h/d",
                "URL must start with postgres:// or postgresql://",
            ),
            ("postgres://host/db", "URL missing '@' separator"),
            ("postgres://u@h:port/d", "invalid port: port"),
            (
                "postgres://u@h/d?sslmode=verify",
                "unsupported sslmode: verify",
            ),
        ] {
            assert_eq!(parse_pg_url(url).err().as_deref(), Some(error), "{url}");
        }
        // No listener, and no such host.
        let refused = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            connect(&format!("postgres://u@127.0.0.1:{port}/d"))
                .err()
                .unwrap()
        };
        assert!(refused.starts_with("connection failed"), "{refused}");
        let unresolved = connect("postgres://u@no-such-host.invalid/d")
            .err()
            .unwrap();
        assert!(
            unresolved.starts_with("DNS resolution failed"),
            "{unresolved}"
        );
    }

    /// MESH_TEST_DATABASE_URL with its query replaced by `query` and its
    /// user and password by `credentials`, when given.
    fn test_database_url(credentials: Option<&str>, query: &str) -> String {
        let url = std::env::var("MESH_TEST_DATABASE_URL")
            .expect("MESH_TEST_DATABASE_URL must be set (the coverage run starts a database)");
        let (base, _) = url.split_once('?').unwrap_or((&url, ""));
        let base = match credentials {
            Some(credentials) => {
                let (_, host) = base.rsplit_once('@').unwrap();
                format!("postgres://{credentials}@{host}")
            }
            None => base.to_string(),
        };
        format!("{base}?{query}")
    }

    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL and MESH_TEST_DATABASE_CA (the coverage run's TLS server)"]
    fn tls_verifies_the_server_against_sslrootcert() {
        let ca = std::env::var("MESH_TEST_DATABASE_CA").expect("MESH_TEST_DATABASE_CA");
        let mut conn = native_pg_connect(&test_database_url(
            None,
            &format!("sslmode=verify-full&sslrootcert={ca}"),
        ))
        .unwrap();
        assert!(matches!(conn.inner.stream, PgStream::Tls(_)));
        let rows = native_pg_query(
            &mut conn,
            "SELECT ssl::text FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .unwrap();
        assert_eq!(rows[0][0].1, "true");
        native_pg_close(conn);
        // Its CA is not a public one.
        let error = native_pg_connect(&test_database_url(None, "sslmode=require"))
            .err()
            .unwrap();
        assert!(error.contains("certificate"), "{error}");
    }

    /// MESH_TEST_DATABASE_URL with its database replaced by `name`.
    fn database_url(name: &str) -> String {
        let url = test_database_url(None, "sslmode=disable");
        let (base, query) = url.split_once('?').unwrap();
        format!("{}/{name}?{query}", base.rsplit_once('/').unwrap().0)
    }

    #[test]
    fn startup_asks_for_utf8() {
        let mut startup = Vec::new();
        write_startup_message(&mut startup, "user", "db");
        let asked = b"client_encoding\0UTF8\0";
        assert!(startup.windows(asked.len()).any(|bytes| bytes == asked));
    }

    /// Mesh text is UTF-8 whatever the database's encoding: the server
    /// converts both ways.
    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn text_crosses_to_a_latin1_database_as_utf8() {
        let mut admin = native_pg_connect(&test_database_url(None, "sslmode=disable")).unwrap();
        for sql in [
            "DROP DATABASE IF EXISTS mesh_latin1",
            "CREATE DATABASE mesh_latin1 ENCODING 'LATIN1' LC_COLLATE 'C' LC_CTYPE 'C' \
             TEMPLATE template0",
        ] {
            native_pg_execute(&mut admin, sql, &[]).unwrap();
        }
        let mut conn = native_pg_connect(&database_url("mesh_latin1")).unwrap();
        let rows = native_pg_query(
            &mut conn,
            "SELECT length($1::text)::text AS n, chr(233) AS e",
            &["é"],
        );
        native_pg_close(conn);
        native_pg_execute(&mut admin, "DROP DATABASE mesh_latin1", &[]).unwrap();
        native_pg_close(admin);
        let pair = |column: &str, value: &str| (column.to_string(), value.to_string());
        assert_eq!(rows, Ok(vec![vec![pair("n", "1"), pair("e", "é")]]));
    }

    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL on a server with md5 authentication (the coverage run's)"]
    fn md5_authenticates_a_role_with_an_md5_password() {
        let mut admin = native_pg_connect(&test_database_url(None, "sslmode=disable")).unwrap();
        for sql in [
            "SET password_encryption = 'md5'",
            "DROP ROLE IF EXISTS mesh_md5_test",
            "CREATE ROLE mesh_md5_test LOGIN PASSWORD 'md5-secret'",
        ] {
            native_pg_execute(&mut admin, sql, &[]).unwrap();
        }
        let mut conn = native_pg_connect(&test_database_url(
            Some("mesh_md5_test:md5-secret"),
            "sslmode=disable",
        ))
        .unwrap();
        let rows = native_pg_query(&mut conn, "SELECT current_user::text", &[]).unwrap();
        assert_eq!(rows[0][0].1, "mesh_md5_test");
        native_pg_close(conn);
        let wrong = native_pg_connect(&test_database_url(
            Some("mesh_md5_test:wrong"),
            "sslmode=disable",
        ));
        assert!(wrong.is_err());
        native_pg_execute(&mut admin, "DROP ROLE mesh_md5_test", &[]).unwrap();
        native_pg_close(admin);
    }

    #[test]
    fn native_connect_rejects_truncated_md5_auth_without_panicking() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "postgres://user:password@{}/db?sslmode=disable",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut length = [0; 4];
            socket.read_exact(&mut length).unwrap();
            let mut startup = vec![0; u32::from_be_bytes(length) as usize - 4];
            socket.read_exact(&mut startup).unwrap();
            // AuthenticationMD5Password requires four salt bytes after its type.
            socket.write_all(b"R\0\0\0\x08\0\0\0\x05").unwrap();
        });
        let result = std::panic::catch_unwind(|| native_pg_connect(&url));
        server.join().unwrap();
        assert!(result.is_ok(), "malformed authentication panicked");
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn scram_native_connect_requires_valid_server_signature() {
        for final_message in [
            b"".as_slice(),
            b"v=???",
            b"e=invalid-proof",
            b"v=AAAA",
            b"\xff",
        ] {
            let (url, server) = scram_test_server(Some(final_message));
            let result = native_pg_connect(&url);
            server.join().unwrap();
            assert!(
                result.is_err(),
                "accepted invalid SCRAM proof: {final_message:?}"
            );
        }
    }

    #[test]
    fn scram_mesh_connect_requires_valid_server_signature() {
        mesh_rt_init();
        for final_message in [
            b"".as_slice(),
            b"v=???",
            b"e=invalid-proof",
            b"v=AAAA",
            b"\xff",
        ] {
            let (url, server) = scram_test_server(Some(final_message));
            let result = mesh_pg_connect(mesh_str(&url));
            server.join().unwrap();
            let result = unsafe { &*(result as *const MeshResult) };
            if result.tag == 0 {
                mesh_pg_close(unsafe { *(result.value as *const u64) });
            }
            assert_eq!(
                result.tag, 1,
                "accepted invalid SCRAM proof: {final_message:?}"
            );
        }
    }

    #[test]
    fn scram_connect_accepts_valid_server_signature() {
        let (url, server) = scram_test_server(None);
        assert!(native_pg_connect(&url).is_ok());
        server.join().unwrap();
        mesh_rt_init();
        let (url, server) = scram_test_server(None);
        let result = mesh_pg_connect(mesh_str(&url));
        let result = unsafe { &*(result as *const MeshResult) };
        server.join().unwrap();
        assert_eq!(result.tag, 0);
        mesh_pg_close(unsafe { *(result.value as *const u64) });
    }

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

        let extracted = unsafe { db_values(params, MAX_PG_VALUES, "PostgreSQL") }.unwrap();

        assert!(matches!(extracted[0], BindValue::Text(b"inbox")));
        assert!(matches!(
            extracted[1],
            BindValue::Binary(bytes) if bytes == [0, 0xff, 0x80]
        ));
        assert!(matches!(extracted[2], BindValue::Null));
    }

    /// A RowDescription body of `(name, type OID, format)` columns.
    fn row_description(columns: &[(&str, u32, i16)]) -> Vec<u8> {
        let mut body = (columns.len() as i16).to_be_bytes().to_vec();
        for (name, oid, format) in columns {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
            body.extend_from_slice(&0_u32.to_be_bytes()); // table oid
            body.extend_from_slice(&0_i16.to_be_bytes()); // attribute number
            body.extend_from_slice(&oid.to_be_bytes());
            body.extend_from_slice(&(-1_i16).to_be_bytes()); // type size
            body.extend_from_slice(&(-1_i32).to_be_bytes()); // type modifier
            body.extend_from_slice(&format.to_be_bytes());
        }
        body
    }

    /// A DataRow body: each cell's bytes, or NULL.
    fn data_row(cells: &[Option<&[u8]>]) -> Vec<u8> {
        let mut body = (cells.len() as i16).to_be_bytes().to_vec();
        for cell in cells {
            match cell {
                Some(bytes) => {
                    body.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
                    body.extend_from_slice(bytes);
                }
                None => body.extend_from_slice(&(-1_i32).to_be_bytes()),
            }
        }
        body
    }

    fn column(name: &str, oid: u32, binary: bool) -> PgColumn {
        PgColumn {
            name: name.to_string(),
            oid,
            binary,
        }
    }

    #[test]
    fn typed_row_decodes_binary_columns_as_raw_bytes_and_others_as_text() {
        let description = row_description(&[("label", 25, 0), ("payload", 17, 1), ("gone", 17, 1)]);
        let columns = parse_row_description(&description).unwrap();
        assert_eq!(columns[1], column("payload", 17, true));

        let row = data_row(&[Some(b"inbox"), Some(&[0, 0xff, 0x80]), None]);

        assert_eq!(
            parse_typed_row(&row, &columns).unwrap(),
            [
                RowValue::Text(b"inbox"),
                RowValue::Binary(&[0, 0xff, 0x80]),
                RowValue::Null
            ]
        );
    }

    #[test]
    fn row_descriptions_and_rows_reject_malformed_bytes() {
        let one = row_description(&[("a", 25, 0)]);
        for (body, error) in [
            (vec![0], "invalid PostgreSQL RowDescription"),
            (
                (-1_i16).to_be_bytes().to_vec(),
                "invalid PostgreSQL column count: -1",
            ),
            (one[..3].to_vec(), "unterminated PostgreSQL column name"),
            (
                [&one[..2], b"\xff\0", &one[4..]].concat(),
                "PostgreSQL column name is not UTF-8",
            ),
            (one[..10].to_vec(), "truncated PostgreSQL RowDescription"),
            (
                [&one[..], b"x"].concat(),
                "trailing bytes in PostgreSQL RowDescription",
            ),
        ] {
            assert_eq!(parse_row_description(&body).unwrap_err(), error);
        }

        let columns = [column("a", 25, false)];
        let row = data_row(&[Some(b"abc")]);
        let length = |length: i32| [&1_i16.to_be_bytes()[..], &length.to_be_bytes()].concat();
        let too_long = format!("PostgreSQL column `a` exceeds {MAX_DB_VALUE_BYTES} byte limit");
        for (body, error) in [
            (vec![0], "invalid PostgreSQL DataRow"),
            (data_row(&[]), "PostgreSQL row has 0 columns; expected 1"),
            (
                (-1_i16).to_be_bytes().to_vec(),
                "PostgreSQL row has -1 columns; expected 1",
            ),
            (row[..4].to_vec(), "truncated PostgreSQL DataRow length"),
            (length(-2), &too_long),
            (length(MAX_DB_VALUE_BYTES as i32 + 1), &too_long),
            (row[..8].to_vec(), "truncated PostgreSQL column `a`"),
            (
                [&row[..], b"x"].concat(),
                "trailing bytes in PostgreSQL DataRow",
            ),
        ] {
            assert_eq!(parse_typed_row(&body, &columns).unwrap_err(), error);
        }
    }

    #[test]
    fn error_responses_keep_the_fields_repo_maps_and_name_a_missing_message() {
        let mut body = Vec::new();
        for (field, value) in [
            (b'S', "ERROR"),
            (b'C', "23502"),
            (b'M', "null value"),
            (b'D', "a detail"),
            (b't', "people"),
            (b'c', "name"),
            (b'n', "people_name_not_null"),
        ] {
            body.push(field);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);

        assert_eq!(
            format_pg_error_string(&parse_error_response_full(&body)),
            "23502\tpeople_name_not_null\tpeople\tname\tnull value"
        );
        assert_eq!(
            parse_error_response(b"C42000\0\0"),
            "unknown PostgreSQL error"
        );
    }

    #[test]
    fn statements_and_binds_stay_within_the_protocol() {
        assert_eq!(
            validate_sql("SELECT '\0'").unwrap_err(),
            "PostgreSQL query contains a NUL byte"
        );
        let long = "x".repeat(MAX_PG_MESSAGE_BYTES);
        assert_eq!(
            validate_sql(&long).unwrap_err(),
            format!("PostgreSQL query exceeds {MAX_PG_MESSAGE_BYTES} byte message limit")
        );

        let mut message = Vec::new();
        let nulls = vec![BindValue::Null; MAX_PG_VALUES + 1];
        assert_eq!(
            write_bind_values(&mut message, &nulls, &[]).unwrap_err(),
            "too many PostgreSQL parameters: 32768 (maximum 32767)"
        );
        let large = vec![0; MAX_DB_VALUE_BYTES + 1];
        assert_eq!(
            write_bind_values(
                &mut message,
                &[BindValue::Null, BindValue::Text(&large)],
                &[]
            )
            .unwrap_err(),
            format!("PostgreSQL parameter at index 1 exceeds {MAX_DB_VALUE_BYTES} byte limit")
        );
        assert!(message.is_empty());
    }

    #[test]
    fn typed_row_map_keeps_the_last_duplicate_column_value() {
        mesh_rt_init();
        let columns = [column("payload", 25, false), column("payload", 17, true)];
        let row = data_row(&[Some(b"old"), Some(&[0xff])]);

        let map = unsafe {
            row_map(
                &columns,
                parse_typed_row(&row, &columns).unwrap(),
                Values::Typed,
            )
        }
        .unwrap() as *mut u8;
        let value = mesh_map_entry_value(map, 0) as *const MeshDbValue;

        assert_eq!(mesh_map_size(map), 1);
        assert_eq!(unsafe { (*value).tag }, DB_VALUE_BINARY);
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
        let columns = [column("payload", 17, true)];
        let row = data_row(&[None]);

        let cost = decoded_row_bytes(&columns) + row.len();

        assert!(cost > row.len() + "payload".len());
        assert!(add_result_bytes(MAX_PG_RESULT_BYTES - cost + 1, cost).is_err());
    }

    // ── Requests against a scripted peer ──────────────────────────────

    /// A connection handle to a peer that runs `script` on its socket.
    fn peer_connection(
        script: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (u64, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || script(listener.accept().unwrap().0));
        let conn = PgConn::from_test_stream(TcpStream::connect(address).unwrap());
        (Box::into_raw(Box::new(conn)) as u64, peer)
    }

    /// A connection handle to a peer that sends `replies` at once, then reads
    /// what it is sent until the connection closes.
    fn scripted(replies: Vec<u8>) -> (u64, std::thread::JoinHandle<()>) {
        peer_connection(move |mut socket| {
            socket.write_all(&replies).unwrap();
            let _ = socket.read_to_end(&mut Vec::new());
        })
    }

    fn conn_of(handle: u64) -> &'static mut PgConn {
        unsafe { &mut *(handle as *mut PgConn) }
    }

    fn outcome(result: *mut u8) -> Result<*mut u8, String> {
        let result = unsafe { &*(result as *const MeshResult) };
        match result.tag {
            0 => Ok(result.value),
            _ => Err(unsafe { text_of(result.value) }.to_string()),
        }
    }

    fn int_outcome(result: *mut u8) -> Result<i64, String> {
        outcome(result).map(|value| unsafe { *(value as *const i64) })
    }

    /// A `List<Map<String, _>>` as its rows' `column=value` entries, a
    /// `DbValue` written `Text(..)`, `Binary(hex)` or `Null`.
    fn rows_of(list: *mut u8, typed: bool) -> Vec<String> {
        (0..mesh_list_length(list))
            .map(|index| {
                let map = mesh_list_get(list, index) as *mut u8;
                (0..mesh_map_size(map))
                    .map(|entry| unsafe {
                        let key = text_of(mesh_map_entry_key(map, entry) as *const u8);
                        let value = mesh_map_entry_value(map, entry) as *const u8;
                        let value = if !typed {
                            text_of(value).to_string()
                        } else {
                            let value = &*(value as *const MeshDbValue);
                            match value.tag {
                                DB_VALUE_TEXT => format!("Text({})", text_of(value.payload)),
                                DB_VALUE_BINARY => format!(
                                    "Binary({:02x?})",
                                    (*(value.payload as *const MeshBytes)).as_slice()
                                ),
                                _ => "Null".to_string(),
                            }
                        };
                        format!("{key}={value}")
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect()
    }

    fn complete(tag: &str) -> Vec<u8> {
        message(b'C', format!("{tag}\0").as_bytes())
    }

    fn text_list(values: &[&str]) -> *mut u8 {
        crate::collections::list::string_list(values)
    }

    #[test]
    fn text_requests_read_every_reply_up_to_ready_for_query() {
        mesh_rt_init();
        let names = row_description(&[("id", 23, 0), ("name", 25, 0)]);
        let replies = [
            // execute: notices, parameter changes and anything unknown pass.
            message(b'1', b""),
            message(b'2', b""),
            message(b'N', b"Mnotice\0\0"),
            message(b'S', b"TimeZone\0UTC\0"),
            message(b'A', b"unknown"),
            complete("UPDATE 3"),
            ready(b'T'),
            // query: a NULL reads as "", text that is not UTF-8 as U+FFFD.
            message(b'1', b""),
            message(b'2', b""),
            message(b'T', &names),
            message(b'D', &data_row(&[Some(b"1"), None])),
            message(b'D', &data_row(&[Some(b"2"), Some(b"\xffb")])),
            complete("SELECT 2"),
            ready(b'T'),
            // execute refused: the first error, structured, and the status.
            message(b'1', b""),
            error_response("bad insert"),
            error_response("a second error"),
            ready(b'E'),
            // query refused part way through its rows.
            message(b'1', b""),
            message(b'2', b""),
            message(b'T', &names),
            message(b'D', &data_row(&[Some(b"1"), None])),
            error_response("division by zero"),
            ready(b'E'),
            // BEGIN refused; COMMIT and ROLLBACK answered.
            error_response("cannot begin"),
            ready(b'E'),
            complete("COMMIT"),
            ready(b'I'),
            complete("ROLLBACK"),
            ready(b'I'),
        ]
        .concat();
        let (handle, peer) = scripted(replies);
        let sql = mesh_str("SELECT $1");

        assert_eq!(
            int_outcome(mesh_pg_execute(handle, sql, text_list(&["a"]))),
            Ok(3)
        );
        assert_eq!(conn_of(handle).txn_status, b'T');
        let rows = outcome(mesh_pg_query(handle, sql, text_list(&["a"]))).unwrap();
        assert_eq!(rows_of(rows, false), ["id=1,name=", "id=2,name=\u{fffd}b"]);
        let refused = "28P01\ta_constraint\t\t\t";
        assert_eq!(
            int_outcome(mesh_pg_execute(handle, sql, mesh_list_new())),
            Err(format!("{refused}bad insert"))
        );
        assert_eq!(conn_of(handle).txn_status, b'E');
        assert_eq!(
            outcome(mesh_pg_query(handle, sql, mesh_list_new())).err(),
            Some(format!("{refused}division by zero"))
        );
        assert_eq!(
            outcome(mesh_pg_begin(handle)).err().as_deref(),
            Some("cannot begin")
        );
        // The refusal left the transaction failed: it cannot commit.
        assert_eq!(
            outcome(mesh_pg_commit(handle)).err().as_deref(),
            Some("the transaction failed, so COMMIT rolled it back")
        );
        assert!(outcome(mesh_pg_rollback(handle)).is_ok());
        assert_eq!(conn_of(handle).txn_status, b'I');
        mesh_pg_close(handle);
        peer.join().unwrap();
    }

    #[test]
    fn native_requests_report_only_the_server_message() {
        let replies = [
            error_response("relation \"missing\" does not exist"),
            ready(b'I'),
            message(b'T', &row_description(&[("version", 20, 0)])),
            message(b'D', &data_row(&[Some(b"7")])),
            message(b'D', &data_row(&[None])),
            error_response("permission denied"),
            ready(b'I'),
            message(b'n', b""),
            complete("CREATE TABLE"),
            ready(b'I'),
            message(b'n', b""),
            complete("SELECT 0"),
            ready(b'I'),
        ]
        .concat();
        let (handle, peer) = scripted(replies);
        let mut conn = NativePgConn {
            inner: *unsafe { Box::from_raw(handle as *mut PgConn) },
        };

        assert_eq!(
            native_pg_execute(&mut conn, "SELECT * FROM missing", &[]),
            Err("relation \"missing\" does not exist".to_string())
        );
        assert_eq!(
            native_pg_query(&mut conn, "SELECT version", &[]),
            Err("permission denied".to_string())
        );
        assert_eq!(
            native_pg_execute(&mut conn, "CREATE TABLE t ()", &[]),
            Ok(0)
        );
        assert_eq!(native_pg_query(&mut conn, "SELECT", &["x"]), Ok(vec![]));
        native_pg_close(conn);
        peer.join().unwrap();
    }

    #[test]
    fn typed_queries_describe_the_statement_first_and_decode_bytea_raw() {
        mesh_rt_init();
        let statement = row_description(&[("payload", 17, 0), ("label", 25, 0)]);
        let portal = row_description(&[("payload", 17, 1), ("label", 25, 0)]);
        let replies = [
            // Described, then bound with the BYTEA column in binary.
            message(b'1', b""),
            message(b't', &[0, 0]),
            message(b'T', &statement),
            ready(b'I'),
            message(b'2', b""),
            message(b'T', &portal),
            message(b'D', &data_row(&[Some(&[0, 0xff]), Some(b"x")])),
            message(b'D', &data_row(&[None, Some(b"y")])),
            complete("SELECT 2"),
            ready(b'I'),
            // A statement without rows: NoData.
            message(b'1', b""),
            message(b'n', b""),
            ready(b'I'),
            message(b'2', b""),
            message(b'n', b""),
            complete("INSERT 0 1"),
            ready(b'I'),
            // The statement refused.
            error_response("syntax error"),
            ready(b'I'),
            // A server that describes nothing.
            message(b'1', b""),
            ready(b'I'),
            // Text a typed query cannot hold.
            message(b'1', b""),
            message(b'T', &row_description(&[("label", 25, 0)])),
            ready(b'I'),
            message(b'T', &row_description(&[("label", 25, 0)])),
            message(b'D', &data_row(&[Some(b"\xff")])),
            ready(b'I'),
            // execute_values: typed parameters, text results.
            message(b'1', b""),
            message(b'2', b""),
            complete("INSERT 0 2"),
            ready(b'I'),
        ]
        .concat();
        let (handle, peer) = scripted(replies);
        let sql = mesh_str("SELECT $1");
        let payload = mesh_bytes_new([1, 2].as_ptr(), 2) as *mut u8;
        let params = [
            unsafe { alloc_db_value(DB_VALUE_BINARY, payload) } as u64,
            unsafe { alloc_db_value(DB_VALUE_NULL, std::ptr::null_mut()) } as u64,
        ];
        let params = mesh_list_from_array(params.as_ptr(), 2);

        let rows = outcome(mesh_pg_query_values(handle, sql, params)).unwrap();
        assert_eq!(
            rows_of(rows, true),
            [
                "payload=Binary([00, ff]),label=Text(x)",
                "payload=Null,label=Text(y)"
            ]
        );
        let rows = outcome(mesh_pg_query_values(handle, sql, mesh_list_new())).unwrap();
        assert_eq!(mesh_list_length(rows), 0);
        assert_eq!(
            outcome(mesh_pg_query_values(handle, sql, mesh_list_new())).err(),
            Some("28P01\ta_constraint\t\t\tsyntax error".to_string())
        );
        assert_eq!(
            outcome(mesh_pg_query_values(handle, sql, mesh_list_new())).err(),
            Some("PostgreSQL prepare returned no row description".to_string())
        );
        assert_eq!(
            outcome(mesh_pg_query_values(handle, sql, mesh_list_new())).err(),
            Some("PostgreSQL text column `label` is not UTF-8".to_string())
        );
        assert_eq!(
            int_outcome(mesh_pg_execute_values(handle, sql, params)),
            Ok(2)
        );
        mesh_pg_close(handle);
        peer.join().unwrap();
    }

    /// Before the text queries shared the typed ones' decoding, a DataRow
    /// that ran past its message sliced out of bounds: a panic, which in an
    /// `extern "C"` function aborts the program.
    #[test]
    fn a_reply_the_driver_cannot_decode_fails_the_request_not_the_program() {
        mesh_rt_init();
        let one = row_description(&[("a", 25, 0)]);
        let replies = [
            message(b'T', &one),
            message(b'D', &[0, 1, 0, 0, 0, 9, b'x']),
            message(b'D', &data_row(&[Some(b"ignored")])),
            ready(b'I'),
            message(b'T', b"\0"),
            ready(b'I'),
            // The connection still answers.
            message(b'T', &one),
            message(b'D', &data_row(&[Some(b"ok")])),
            ready(b'I'),
        ]
        .concat();
        let (handle, peer) = scripted(replies);
        let sql = mesh_str("SELECT a");

        assert_eq!(
            outcome(mesh_pg_query(handle, sql, mesh_list_new()))
                .err()
                .as_deref(),
            Some("truncated PostgreSQL column `a`")
        );
        assert_eq!(
            outcome(mesh_pg_query(handle, sql, mesh_list_new()))
                .err()
                .as_deref(),
            Some("invalid PostgreSQL RowDescription")
        );
        let rows = outcome(mesh_pg_query(handle, sql, mesh_list_new())).unwrap();
        assert_eq!(rows_of(rows, false), ["a=ok"]);
        mesh_pg_close(handle);
        peer.join().unwrap();
    }

    #[test]
    fn results_stay_within_the_row_and_byte_limits() {
        let one = row_description(&[("a", 25, 0)]);
        let row = message(b'D', &data_row(&[Some(b"1")]));
        let mut replies = message(b'T', &one);
        for _ in 0..=MAX_PG_ROWS {
            replies.extend_from_slice(&row);
        }
        replies.extend_from_slice(&ready(b'I'));
        // Four rows of 15 MiB fit in 64; the fifth does not.
        let large = vec![b'x'; 15 * 1024 * 1024];
        replies.extend_from_slice(&message(b'T', &one));
        for _ in 0..5 {
            replies.extend_from_slice(&message(b'D', &data_row(&[Some(&large)])));
        }
        replies.extend_from_slice(&ready(b'I'));
        let (handle, peer) = scripted(replies);
        let mut conn = NativePgConn {
            inner: *unsafe { Box::from_raw(handle as *mut PgConn) },
        };

        assert_eq!(
            native_pg_query(&mut conn, "SELECT a", &[]),
            Err(format!("PostgreSQL result exceeds {MAX_PG_ROWS} row limit"))
        );
        assert_eq!(
            native_pg_query(&mut conn, "SELECT a", &[]),
            Err(format!(
                "PostgreSQL result exceeds {MAX_PG_RESULT_BYTES} byte limit"
            ))
        );
        native_pg_close(conn);
        peer.join().unwrap();
    }

    #[test]
    fn a_connection_that_fails_to_send_or_read_is_unusable_after() {
        mesh_rt_init();
        let sql = mesh_str("SELECT 1");
        // The peer reads the request, then goes.
        let (handle, peer) = peer_connection(|mut socket| {
            assert!(socket.read(&mut [0; 1024]).unwrap() > 0);
        });
        let error = outcome(mesh_pg_query(handle, sql, mesh_list_new())).unwrap_err();
        assert!(error.starts_with("read query: read tag: "), "{error}");
        peer.join().unwrap();
        let unusable = "PostgreSQL connection is unusable";
        assert_eq!(
            outcome(mesh_pg_execute(handle, sql, mesh_list_new())).err(),
            Some(format!("send execute: {unusable}"))
        );
        for result in [
            mesh_pg_begin(handle),
            mesh_pg_commit(handle),
            mesh_pg_rollback(handle),
        ] {
            assert!(outcome(result).unwrap_err().ends_with(unusable));
        }
        assert_eq!(
            outcome(mesh_pg_transaction(
                handle,
                std::ptr::null(),
                std::ptr::null()
            ))
            .err(),
            Some(format!("BEGIN: send BEGIN: {unusable}"))
        );
        mesh_pg_close(handle);

        // A message longer than any the driver accepts.
        let (handle, peer) = scripted(b"Z\x7f\xff\xff\xff".to_vec());
        assert_eq!(
            outcome(mesh_pg_query(handle, sql, mesh_list_new())).err(),
            Some(format!(
                "read query: invalid PostgreSQL message length: {} (maximum {MAX_PG_MESSAGE_BYTES})",
                i32::MAX
            ))
        );
        assert!(conn_of(handle).is_broken());
        mesh_pg_close(handle);
        peer.join().unwrap();

        // Sending fails.
        let (handle, peer) = peer_connection(|mut socket| {
            let _ = socket.read_to_end(&mut Vec::new());
        });
        let PgStream::Plain(stream) = &conn_of(handle).stream else {
            unreachable!()
        };
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let error = outcome(mesh_pg_execute(handle, sql, mesh_list_new())).unwrap_err();
        assert!(error.starts_with("send execute: "), "{error}");
        assert!(conn_of(handle).is_broken());
        mesh_pg_close(handle);
        peer.join().unwrap();
    }

    // ── Against PostgreSQL ────────────────────────────────────────────

    /// A connection handle to MESH_TEST_DATABASE_URL.
    fn test_connection() -> u64 {
        mesh_rt_init();
        open(&test_database_url(None, "sslmode=disable")).unwrap()
    }

    fn run(handle: u64, sql: &str) -> Result<i64, String> {
        int_outcome(mesh_pg_execute(handle, mesh_str(sql), mesh_list_new()))
    }

    /// A callback's `Ok(())`.
    fn unit_ok() -> MeshResult {
        MeshResult {
            tag: 0,
            value: std::ptr::null_mut(),
        }
    }

    extern "C-unwind" fn add_parent_one(conn: u64) -> MeshResult {
        run(conn, "INSERT INTO parent VALUES (1)").unwrap();
        unit_ok()
    }

    extern "C-unwind" fn add_parent_two_then_fail(conn: u64) -> MeshResult {
        run(conn, "INSERT INTO parent VALUES (2)").unwrap();
        MeshResult {
            tag: 1,
            value: mesh_str("changed my mind") as *mut u8,
        }
    }

    extern "C-unwind" fn add_parent_three_then_panic(conn: u64) -> MeshResult {
        run(conn, "INSERT INTO parent VALUES (3)").unwrap();
        panic!("the callback panicked");
    }

    /// Adds parent 4, ignoring a statement that fails after it.
    extern "C-unwind" fn add_parent_four_despite_a_failure(conn: u64) -> MeshResult {
        run(conn, "INSERT INTO parent VALUES (4)").unwrap();
        let _ = run(conn, "SELECT 1/0");
        unit_ok()
    }

    /// Adds a child of the parent `env` points at: a deferred foreign key,
    /// checked at COMMIT.
    extern "C-unwind" fn add_child(env: *const u8, conn: u64) -> MeshResult {
        let parent = unsafe { *(env as *const i64) };
        run(conn, &format!("INSERT INTO child VALUES ({parent})")).unwrap();
        unit_ok()
    }

    fn parents(handle: u64) -> Vec<String> {
        let rows = outcome(mesh_pg_query(
            handle,
            mesh_str("SELECT id FROM parent ORDER BY id"),
            mesh_list_new(),
        ))
        .unwrap();
        rows_of(rows, false)
    }

    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn transactions_commit_roll_back_and_outlive_a_panicking_callback() {
        let handle = test_connection();
        run(handle, "CREATE TEMP TABLE parent (id bigint PRIMARY KEY)").unwrap();
        run(
            handle,
            "CREATE TEMP TABLE child (parent bigint REFERENCES parent \
             DEFERRABLE INITIALLY DEFERRED)",
        )
        .unwrap();
        let transaction = |callback: *const u8, env: *const u8| {
            outcome(mesh_pg_transaction(handle, callback, env))
        };

        assert!(transaction(add_parent_one as *const u8, std::ptr::null()).is_ok());
        assert_eq!(
            transaction(add_parent_two_then_fail as *const u8, std::ptr::null()).err(),
            Some("changed my mind".to_string())
        );
        assert_eq!(
            transaction(add_parent_three_then_panic as *const u8, std::ptr::null()).err(),
            Some("transaction aborted: panic in callback".to_string())
        );
        let orphan = 9_i64;
        let error =
            transaction(add_child as *const u8, &orphan as *const i64 as *const u8).unwrap_err();
        assert!(
            error.starts_with("COMMIT: insert or update on table \"child\""),
            "{error}"
        );
        let parent = 1_i64;
        assert!(transaction(add_child as *const u8, &parent as *const i64 as *const u8).is_ok());
        // PostgreSQL answers the COMMIT of a transaction a failed statement
        // aborted by rolling it back, without an error.
        let aborted = "the transaction failed, so COMMIT rolled it back";
        assert_eq!(
            transaction(
                add_parent_four_despite_a_failure as *const u8,
                std::ptr::null()
            )
            .err(),
            Some(format!("COMMIT: {aborted}"))
        );
        assert!(outcome(mesh_pg_begin(handle)).is_ok());
        run(handle, "INSERT INTO parent VALUES (5)").unwrap();
        assert!(run(handle, "SELECT 1/0").is_err());
        assert_eq!(
            outcome(mesh_pg_commit(handle)).err().as_deref(),
            Some(aborted)
        );
        assert!(outcome(mesh_pg_begin(handle)).is_ok());
        run(handle, "INSERT INTO parent VALUES (6)").unwrap();
        assert!(outcome(mesh_pg_commit(handle)).is_ok());

        assert_eq!(parents(handle), ["id=1", "id=6"]);
        assert_eq!(conn_of(handle).txn_status, b'I');
        mesh_pg_close(handle);
    }

    /// COPY FROM STDIN waits for rows the driver has none of to send: it is
    /// refused, and the connection answers after.
    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn copy_from_stdin_is_refused_and_the_connection_goes_on() {
        let handle = test_connection();
        run(handle, "CREATE TEMP TABLE copied (id int)").unwrap();
        let sql = mesh_str("COPY copied FROM STDIN");
        for result in [
            mesh_pg_execute(handle, sql, mesh_list_new()),
            mesh_pg_query(handle, sql, mesh_list_new()),
            mesh_pg_query_values(handle, sql, mesh_list_new()),
        ] {
            let error = outcome(result).unwrap_err();
            assert!(
                error.ends_with("COPY FROM STDIN is not supported"),
                "{error}"
            );
        }
        assert_eq!(run(handle, "INSERT INTO copied VALUES (1)"), Ok(1));
        assert_eq!(run(handle, "COPY copied TO STDOUT"), Ok(1));
        mesh_pg_close(handle);
    }

    extern "C-unwind" fn column_count(row: u64) -> u64 {
        mesh_map_size(row as *mut u8) as u64
    }

    extern "C-unwind" fn column_count_plus(env: *mut u8, row: u64) -> u64 {
        unsafe { *(env as *const u64) + mesh_map_size(row as *mut u8) as u64 }
    }

    #[test]
    #[ignore = "requires MESH_TEST_DATABASE_URL (the coverage run starts a database)"]
    fn query_as_decodes_every_row_and_passes_a_failed_query_on() {
        let handle = test_connection();
        let sql = mesh_str("SELECT 1 AS a, 2 AS b UNION ALL SELECT 3, 4") as *mut u8;
        let decoded = outcome(mesh_pg_query_as(
            handle,
            sql,
            mesh_list_new(),
            column_count as *mut u8,
            std::ptr::null_mut(),
        ))
        .unwrap();
        assert_eq!(
            (0..mesh_list_length(decoded))
                .map(|index| mesh_list_get(decoded, index))
                .collect::<Vec<_>>(),
            [2, 2]
        );
        let mut ten = 10_u64;
        let decoded = outcome(mesh_pg_query_as(
            handle,
            sql,
            mesh_list_new(),
            column_count_plus as *mut u8,
            &mut ten as *mut u64 as *mut u8,
        ))
        .unwrap();
        assert_eq!(mesh_list_get(decoded, 1), 12);

        let error = outcome(mesh_pg_query_as(
            handle,
            mesh_str("SELECT * FROM no_such_table") as *mut u8,
            mesh_list_new(),
            column_count as *mut u8,
            std::ptr::null_mut(),
        ))
        .unwrap_err();
        assert!(error.starts_with("42P01\t"), "{error}");
        mesh_pg_close(handle);
    }
}
