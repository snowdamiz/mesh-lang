//! PostgreSQL wire protocol v3 client for the Snow runtime.
//!
//! Provides four extern "C" functions that Snow programs call to interact
//! with PostgreSQL databases:
//! - `snow_pg_connect`: Connect to a PostgreSQL server via URL
//! - `snow_pg_close`: Close a connection
//! - `snow_pg_execute`: Execute a write query (INSERT/UPDATE/DELETE/CREATE)
//! - `snow_pg_query`: Execute a read query (SELECT), returns rows
//!
//! Connection handles are opaque u64 values (Box::into_raw as u64) for GC
//! safety. The GC never traces integer values, so the connection won't be
//! corrupted by garbage collection.
//!
//! Authentication supports both SCRAM-SHA-256 (production/cloud) and MD5
//! (local development). The wire protocol is implemented from scratch using
//! `std::net::TcpStream` and crypto crates from the RustCrypto project.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use pbkdf2::pbkdf2_hmac;
use rand::Rng;
use sha2::Sha256;

use crate::collections::list::{snow_list_append, snow_list_new};
use crate::collections::map::{snow_map_new_typed, snow_map_put};
use crate::io::alloc_result;
use crate::string::{snow_string_new, SnowString};

type HmacSha256 = Hmac<Sha256>;

/// Wrapper around a TCP stream to a PostgreSQL server.
struct PgConn {
    stream: TcpStream,
}

// ── URL Parsing ────────────────────────────────────────────────────────

/// Parsed PostgreSQL connection URL components.
struct PgUrl {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
}

/// Percent-decode a URL component (handles %XX sequences).
fn percent_decode(s: &str) -> String {
    let mut result = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
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

/// Parse a `postgres://user:pass@host:port/database` URL.
fn parse_pg_url(url: &str) -> Result<PgUrl, String> {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or_else(|| "URL must start with postgres:// or postgresql://".to_string())?;

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

/// Write Describe (Portal) message: Byte1('D') Int32(len) Byte1('P') String("")
fn write_describe_portal(buf: &mut Vec<u8>) {
    buf.push(b'D');
    let len = (4 + 1 + 1) as i32; // length_field + 'P' + null byte
    buf.extend_from_slice(&len.to_be_bytes());
    buf.push(b'P'); // Portal variant
    buf.push(0); // unnamed portal
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

/// Read a single message from the server: returns (tag_byte, body_bytes).
fn read_message(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), String> {
    let mut tag = [0u8; 1];
    stream
        .read_exact(&mut tag)
        .map_err(|e| format!("read tag: {}", e))?;

    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|e| format!("read length: {}", e))?;
    let len = i32::from_be_bytes(len_buf) as usize;

    if len < 4 {
        return Err("invalid message length".to_string());
    }

    let body_len = len - 4;
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
    let mut mac =
        HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
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
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        &salt,
        iterations,
        &mut salted_password,
    );

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

    let client_final = format!(
        "{},p={}",
        client_final_without_proof,
        BASE64.encode(&proof)
    );
    Ok((client_final, server_signature))
}

// ── Error Response Parsing ─────────────────────────────────────────────

/// Extract the human-readable message from an ErrorResponse body.
/// Format: sequence of (Byte1 field_type, String value) pairs, terminated by Byte1(0).
/// Field 'M' = human-readable message.
fn parse_error_response(body: &[u8]) -> String {
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
        if field_type == b'M' {
            return String::from_utf8_lossy(&body[start..i]).into_owned();
        }
        i += 1; // skip null terminator
    }
    "unknown PostgreSQL error".to_string()
}

// ── SnowString / SnowResult Helpers ────────────────────────────────────

/// Extract a Rust &str from a raw SnowString pointer.
///
/// # Safety
///
/// The pointer must reference a valid SnowString allocation.
unsafe fn snow_str_to_rust(s: *const SnowString) -> &'static str {
    (*s).as_str()
}

/// Create a SnowString from a Rust &str and return as *mut u8.
fn rust_str_to_snow(s: &str) -> *mut u8 {
    snow_string_new(s.as_ptr(), s.len() as u64) as *mut u8
}

/// Create an error SnowResult from a Rust string.
fn err_result(msg: &str) -> *mut u8 {
    let s = rust_str_to_snow(msg);
    alloc_result(1, s) as *mut u8
}

/// Extract param strings from a Snow List<String>.
///
/// SnowList layout: `{ len: u64, cap: u64, data: [u64; cap] }`
/// Each element is a u64 that is actually a pointer to a SnowString.
unsafe fn extract_params(params: *mut u8) -> Vec<String> {
    let len = *(params as *const u64);
    let data_ptr = (params as *const u64).add(2); // skip len + cap
    let mut result = Vec::with_capacity(len as usize);
    for i in 0..len as usize {
        let param_ptr = *data_ptr.add(i) as *const SnowString;
        let param_str = snow_str_to_rust(param_ptr);
        result.push(param_str.to_string());
    }
    result
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
/// `snow_pg_connect(url: *const SnowString) -> *mut u8 (SnowResult<u64, String>)`
///
/// Returns SnowResult with tag 0 (Ok) containing the connection handle as
/// a u64, or tag 1 (Err) containing an error message string.
#[no_mangle]
pub extern "C" fn snow_pg_connect(url: *const SnowString) -> *mut u8 {
    unsafe {
        let url_str = snow_str_to_rust(url);
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

        let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
            Ok(s) => s,
            Err(e) => return err_result(&format!("connection failed: {}", e)),
        };

        // Set read/write timeouts for protocol messages
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

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
                if tag != b'R' || body.len() < 4 || i32::from_be_bytes([body[0], body[1], body[2], body[3]]) != 0 {
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
                write_sasl_initial_response(
                    &mut buf,
                    "SCRAM-SHA-256",
                    client_first.as_bytes(),
                );
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
                if tag != b'R' || body.len() < 4 || i32::from_be_bytes([body[0], body[1], body[2], body[3]]) != 0 {
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
                if tag != b'R' || body.len() < 4 || i32::from_be_bytes([body[0], body[1], body[2], body[3]]) != 0 {
                    return err_result("cleartext authentication failed");
                }
            }
            _ => {
                return err_result(&format!("unsupported auth type: {}", auth_type));
            }
        }

        // Read messages until ReadyForQuery ('Z')
        loop {
            let (tag, body) = match read_message(&mut stream) {
                Ok(m) => m,
                Err(e) => return err_result(&format!("post-auth read: {}", e)),
            };
            match tag {
                b'Z' => break,                // ReadyForQuery
                b'S' => {}                    // ParameterStatus -- skip
                b'K' => {}                    // BackendKeyData -- skip
                b'N' => {}                    // NoticeResponse -- skip
                b'E' => {
                    return err_result(&parse_error_response(&body));
                }
                _ => {} // skip unknown
            }
        }

        // Create the PgConn handle
        let conn = Box::new(PgConn { stream });
        let handle = Box::into_raw(conn) as u64;
        alloc_result(0, handle as *mut u8) as *mut u8
    }
}

/// Close a PostgreSQL connection.
///
/// # Signature
///
/// `snow_pg_close(conn_handle: u64)`
///
/// Recovers the Box<PgConn> from the handle, sends Terminate message,
/// and lets Box::drop free the Rust memory and close the TcpStream.
#[no_mangle]
pub extern "C" fn snow_pg_close(conn_handle: u64) {
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
/// `snow_pg_execute(conn_handle: u64, sql: *const SnowString, params: *mut u8)
///     -> *mut u8 (SnowResult<Int, String>)`
///
/// Parameters are bound via the Extended Query protocol using $1, $2, etc.
/// Returns the number of rows affected from the CommandComplete tag.
#[no_mangle]
pub extern "C" fn snow_pg_execute(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);
        let sql_str = snow_str_to_rust(sql);
        let param_strs = extract_params(params);
        let param_refs: Vec<&str> = param_strs.iter().map(|s| s.as_str()).collect();

        // Build pipelined message: Parse + Bind + Execute + Sync
        let mut buf = Vec::new();
        write_parse(&mut buf, sql_str);
        write_bind(&mut buf, &param_refs);
        write_execute(&mut buf);
        write_sync(&mut buf);

        if let Err(e) = conn.stream.write_all(&buf) {
            return err_result(&format!("send execute: {}", e));
        }

        let mut rows_affected: i64 = 0;
        let mut error_msg: Option<String> = None;

        // Read messages until ReadyForQuery
        loop {
            let (tag, body) = match read_message(&mut conn.stream) {
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
                    // ErrorResponse
                    error_msg = Some(parse_error_response(&body));
                }
                b'Z' => break, // ReadyForQuery
                b'N' => {}     // NoticeResponse -- skip
                _ => {}
            }
        }

        if let Some(msg) = error_msg {
            err_result(&msg)
        } else {
            alloc_result(0, rows_affected as *mut u8) as *mut u8
        }
    }
}

/// Execute a read SQL statement (SELECT) and return rows.
///
/// # Signature
///
/// `snow_pg_query(conn_handle: u64, sql: *const SnowString, params: *mut u8)
///     -> *mut u8 (SnowResult<List<Map<String, String>>, String>)`
///
/// Each row is a Map<String, String> where keys are column names and values
/// are the text representation of column values. NULL columns become empty
/// strings.
#[no_mangle]
pub extern "C" fn snow_pg_query(
    conn_handle: u64,
    sql: *const SnowString,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        let conn = &mut *(conn_handle as *mut PgConn);
        let sql_str = snow_str_to_rust(sql);
        let param_strs = extract_params(params);
        let param_refs: Vec<&str> = param_strs.iter().map(|s| s.as_str()).collect();

        // Build pipelined message: Parse + Bind + Describe(Portal) + Execute + Sync
        let mut buf = Vec::new();
        write_parse(&mut buf, sql_str);
        write_bind(&mut buf, &param_refs);
        write_describe_portal(&mut buf);
        write_execute(&mut buf);
        write_sync(&mut buf);

        if let Err(e) = conn.stream.write_all(&buf) {
            return err_result(&format!("send query: {}", e));
        }

        let mut col_names: Vec<String> = Vec::new();
        let mut result_list = snow_list_new();
        let mut error_msg: Option<String> = None;

        // Read messages until ReadyForQuery
        loop {
            let (tag, body) = match read_message(&mut conn.stream) {
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
                        let name =
                            String::from_utf8_lossy(&body[name_start..offset]).into_owned();
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
                    let mut row_map = snow_map_new_typed(1);

                    for col in 0..num_cols {
                        if offset + 4 > body.len() {
                            break;
                        }
                        let col_len =
                            i32::from_be_bytes([body[offset], body[offset + 1], body[offset + 2], body[offset + 3]]);
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

                        let key_snow = rust_str_to_snow(col_name);
                        let val_snow = rust_str_to_snow(&value_str);
                        row_map = snow_map_put(row_map, key_snow as u64, val_snow as u64);
                    }

                    result_list = snow_list_append(result_list, row_map as u64);
                }
                b'C' => {} // CommandComplete -- skip for query
                b'E' => {
                    // ErrorResponse
                    error_msg = Some(parse_error_response(&body));
                }
                b'Z' => break, // ReadyForQuery
                b'N' => {}     // NoticeResponse -- skip
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
