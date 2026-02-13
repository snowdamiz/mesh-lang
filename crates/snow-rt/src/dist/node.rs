//! Node identity, TLS configuration, and TCP listener for Snow distribution.
//!
//! This module implements the foundational layer for Snow's distributed actor
//! system. A Snow runtime becomes a named, addressable node by calling
//! `snow_node_start`, which:
//!
//! 1. Parses the node name ("name@host" or "name@host:port")
//! 2. Generates an ephemeral ECDSA P-256 self-signed certificate
//! 3. Builds TLS server/client configs (cert verification skipped; cookie provides auth)
//! 4. Initializes the global `NODE_STATE` singleton
//! 5. Binds a TCP listener and spawns an accept loop thread
//!
//! ## Trust Model
//!
//! TLS provides confidentiality and integrity. Authentication is handled by the
//! HMAC-SHA256 cookie challenge/response in Plan 02's handshake, NOT by PKI.
//! The client-side TLS config intentionally skips certificate verification.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use hmac::{Hmac, Mac};
use parking_lot::RwLock;
use ring::rand::SystemRandom;
use ring::signature::{self, EcdsaKeyPair, KeyPair};
use rustc_hash::FxHashMap;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, ServerConfig, SignatureScheme, StreamOwned};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// NodeState -- global singleton for the local node
// ---------------------------------------------------------------------------

/// Global node state, initialized once by `snow_node_start`.
///
/// Holds the node's identity, TLS configs, and connected sessions.
/// Follows the same `OnceLock` pattern as `GLOBAL_SCHEDULER` and
/// `GLOBAL_REGISTRY` in the actor system.
pub struct NodeState {
    /// Full node name, e.g. "name@host" or "name@host:4000"
    pub name: String,
    /// Host portion of the name
    pub host: String,
    /// TCP listener port (may differ from parsed port if OS-assigned via port 0)
    pub port: u16,
    /// Shared secret for HMAC-SHA256 authentication
    pub cookie: String,
    /// Monotonically incrementing creation counter (wraps at 255).
    /// Distinguishes different incarnations of the same node name.
    pub creation: AtomicU8,
    /// Assigns node_ids to remote nodes (starts at 1; 0 = local)
    next_node_id: AtomicU16,
    /// TLS server config for accepting incoming connections
    pub tls_server_config: Arc<ServerConfig>,
    /// TLS client config for initiating outgoing connections
    pub tls_client_config: Arc<ClientConfig>,
    /// Connected nodes: remote_name -> session
    pub sessions: RwLock<FxHashMap<String, Arc<NodeSession>>>,
    /// Reverse map: node_id -> node name (for PID routing in Phase 65)
    pub node_id_map: RwLock<FxHashMap<u16, String>>,
    /// Signals the listener thread to stop accepting connections
    pub listener_shutdown: AtomicBool,
}

impl NodeState {
    /// Atomically assign the next node_id for a remote node.
    ///
    /// Node IDs start at 1 (0 is reserved for the local node).
    pub fn assign_node_id(&self) -> u16 {
        self.next_node_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Load the current creation counter value.
    pub fn creation(&self) -> u8 {
        self.creation.load(Ordering::Relaxed)
    }
}

/// Global node state singleton.
static NODE_STATE: OnceLock<NodeState> = OnceLock::new();

/// Get a reference to the global node state, if initialized.
///
/// Returns `Some` if `snow_node_start` has been called, `None` otherwise.
/// This is the primary access point for code that needs to check whether
/// the runtime is operating as a named node.
pub fn node_state() -> Option<&'static NodeState> {
    NODE_STATE.get()
}

// ---------------------------------------------------------------------------
// NodeSession -- placeholder for Plan 02
// ---------------------------------------------------------------------------

/// Represents a connection to a remote node.
///
/// Holds the authenticated TLS stream, identity info, and shutdown flag.
/// Plan 03 will add reader and heartbeat threads using the stream and
/// shutdown flag.
pub struct NodeSession {
    /// Full name of the remote node
    pub remote_name: String,
    /// Creation counter of the remote node at connection time
    pub remote_creation: u8,
    /// The node_id assigned to this remote node (for PID encoding)
    pub node_id: u16,
    /// The TLS stream, shared between writer and reader threads
    pub stream: Mutex<NodeStream>,
    /// Signals the session's reader/heartbeat threads to stop
    pub shutdown: AtomicBool,
    /// When this connection was established
    pub connected_at: Instant,
}

// ---------------------------------------------------------------------------
// NodeStream -- TLS stream abstraction for node connections
// ---------------------------------------------------------------------------

/// Stream abstraction for inter-node TLS connections.
///
/// Server variant is used when we accepted the connection; Client variant
/// when we initiated it. Both implement Read + Write by delegating to
/// the inner `StreamOwned`.
pub(crate) enum NodeStream {
    ServerTls(StreamOwned<rustls::ServerConnection, TcpStream>),
    ClientTls(StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl Read for NodeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            NodeStream::ServerTls(s) => s.read(buf),
            NodeStream::ClientTls(s) => s.read(buf),
        }
    }
}

impl Write for NodeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            NodeStream::ServerTls(s) => s.write(buf),
            NodeStream::ClientTls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            NodeStream::ServerTls(s) => s.flush(),
            NodeStream::ClientTls(s) => s.flush(),
        }
    }
}

// ---------------------------------------------------------------------------
// Handshake protocol constants
// ---------------------------------------------------------------------------

/// Initiator sends their name + creation.
const HANDSHAKE_NAME: u8 = 1;
/// Acceptor sends their name + creation + challenge.
const HANDSHAKE_CHALLENGE: u8 = 2;
/// Initiator sends response to challenge + own challenge.
const HANDSHAKE_REPLY: u8 = 3;
/// Acceptor sends response to initiator's challenge.
const HANDSHAKE_ACK: u8 = 4;

/// Maximum handshake message size (4 KiB). Prevents unbounded allocation
/// from a malicious or buggy peer during the handshake.
const MAX_HANDSHAKE_MSG: u32 = 4096;

// ---------------------------------------------------------------------------
// Wire format helpers (length-prefixed binary, little-endian)
// ---------------------------------------------------------------------------

/// Write a length-prefixed message: `[u32 length][payload]`.
fn write_msg(stream: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(payload)?;
    stream.flush()
}

/// Read a length-prefixed message: read `[u32 length]`, then read exactly
/// that many bytes. Enforces MAX_HANDSHAKE_MSG to prevent allocation bombs.
fn read_msg(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_HANDSHAKE_MSG {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("handshake message too large: {} bytes (max {})", len, MAX_HANDSHAKE_MSG),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// HMAC-SHA256 challenge/response functions
// ---------------------------------------------------------------------------

/// Generate a 32-byte random challenge.
fn generate_challenge() -> [u8; 32] {
    rand::random()
}

/// Compute HMAC-SHA256(cookie, challenge) as the challenge response.
///
/// Follows the pattern from `db/pg.rs` SCRAM-SHA-256 authentication.
fn compute_response(cookie: &str, challenge: &[u8; 32]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(cookie.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(challenge);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Verify a challenge response using constant-time comparison.
///
/// Uses `Mac::verify_slice` for constant-time comparison, preventing
/// timing attacks (research pitfall 3).
fn verify_response(cookie: &str, challenge: &[u8; 32], response: &[u8; 32]) -> bool {
    let mut mac = HmacSha256::new_from_slice(cookie.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(challenge);
    mac.verify_slice(response).is_ok()
}

// ---------------------------------------------------------------------------
// Handshake message builders and parsers
// ---------------------------------------------------------------------------

/// Send NAME message: `[tag=1][u16 name_len][name_bytes][u8 creation]`.
fn send_name(stream: &mut impl Write, name: &str, creation: u8) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    let mut payload = Vec::with_capacity(1 + 2 + name_bytes.len() + 1);
    payload.push(HANDSHAKE_NAME);
    payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(name_bytes);
    payload.push(creation);
    write_msg(stream, &payload).map_err(|e| format!("send_name failed: {}", e))
}

/// Receive and parse NAME message. Returns (name, creation).
fn recv_name(stream: &mut impl Read) -> Result<(String, u8), String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_name failed: {}", e))?;
    if msg.is_empty() || msg[0] != HANDSHAKE_NAME {
        return Err(format!(
            "expected HANDSHAKE_NAME tag ({}), got {}",
            HANDSHAKE_NAME,
            msg.first().copied().unwrap_or(0)
        ));
    }
    // [tag=1][u16 name_len][name_bytes][u8 creation]
    if msg.len() < 4 {
        return Err("NAME message too short".to_string());
    }
    let name_len = u16::from_le_bytes([msg[1], msg[2]]) as usize;
    if msg.len() < 3 + name_len + 1 {
        return Err("NAME message truncated".to_string());
    }
    let name = std::str::from_utf8(&msg[3..3 + name_len])
        .map_err(|_| "invalid UTF-8 in node name".to_string())?
        .to_string();
    let creation = msg[3 + name_len];
    Ok((name, creation))
}

/// Send CHALLENGE message: `[tag=2][u16 name_len][name_bytes][u8 creation][32 bytes challenge]`.
fn send_challenge(
    stream: &mut impl Write,
    name: &str,
    creation: u8,
    challenge: &[u8; 32],
) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    let mut payload = Vec::with_capacity(1 + 2 + name_bytes.len() + 1 + 32);
    payload.push(HANDSHAKE_CHALLENGE);
    payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(name_bytes);
    payload.push(creation);
    payload.extend_from_slice(challenge);
    write_msg(stream, &payload).map_err(|e| format!("send_challenge failed: {}", e))
}

/// Receive and parse CHALLENGE message. Returns (name, creation, challenge).
fn recv_challenge(stream: &mut impl Read) -> Result<(String, u8, [u8; 32]), String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_challenge failed: {}", e))?;
    if msg.is_empty() || msg[0] != HANDSHAKE_CHALLENGE {
        return Err(format!(
            "expected HANDSHAKE_CHALLENGE tag ({}), got {}",
            HANDSHAKE_CHALLENGE,
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < 4 {
        return Err("CHALLENGE message too short".to_string());
    }
    let name_len = u16::from_le_bytes([msg[1], msg[2]]) as usize;
    if msg.len() < 3 + name_len + 1 + 32 {
        return Err("CHALLENGE message truncated".to_string());
    }
    let name = std::str::from_utf8(&msg[3..3 + name_len])
        .map_err(|_| "invalid UTF-8 in node name".to_string())?
        .to_string();
    let creation = msg[3 + name_len];
    let mut challenge = [0u8; 32];
    challenge.copy_from_slice(&msg[3 + name_len + 1..3 + name_len + 1 + 32]);
    Ok((name, creation, challenge))
}

/// Send CHALLENGE_REPLY message: `[tag=3][32 bytes response][32 bytes own_challenge]`.
fn send_challenge_reply(
    stream: &mut impl Write,
    response: &[u8; 32],
    own_challenge: &[u8; 32],
) -> Result<(), String> {
    let mut payload = Vec::with_capacity(1 + 32 + 32);
    payload.push(HANDSHAKE_REPLY);
    payload.extend_from_slice(response);
    payload.extend_from_slice(own_challenge);
    write_msg(stream, &payload).map_err(|e| format!("send_challenge_reply failed: {}", e))
}

/// Receive and parse CHALLENGE_REPLY message. Returns (response, their_challenge).
fn recv_challenge_reply(stream: &mut impl Read) -> Result<([u8; 32], [u8; 32]), String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_challenge_reply failed: {}", e))?;
    if msg.is_empty() || msg[0] != HANDSHAKE_REPLY {
        return Err(format!(
            "expected HANDSHAKE_REPLY tag ({}), got {}",
            HANDSHAKE_REPLY,
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < 1 + 32 + 32 {
        return Err("CHALLENGE_REPLY message too short".to_string());
    }
    let mut response = [0u8; 32];
    response.copy_from_slice(&msg[1..33]);
    let mut their_challenge = [0u8; 32];
    their_challenge.copy_from_slice(&msg[33..65]);
    Ok((response, their_challenge))
}

/// Send CHALLENGE_ACK message: `[tag=4][32 bytes response]`.
fn send_challenge_ack(stream: &mut impl Write, response: &[u8; 32]) -> Result<(), String> {
    let mut payload = Vec::with_capacity(1 + 32);
    payload.push(HANDSHAKE_ACK);
    payload.extend_from_slice(response);
    write_msg(stream, &payload).map_err(|e| format!("send_challenge_ack failed: {}", e))
}

/// Receive and parse CHALLENGE_ACK message. Returns the response.
fn recv_challenge_ack(stream: &mut impl Read) -> Result<[u8; 32], String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_challenge_ack failed: {}", e))?;
    if msg.is_empty() || msg[0] != HANDSHAKE_ACK {
        return Err(format!(
            "expected HANDSHAKE_ACK tag ({}), got {}",
            HANDSHAKE_ACK,
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < 1 + 32 {
        return Err("CHALLENGE_ACK message too short".to_string());
    }
    let mut response = [0u8; 32];
    response.copy_from_slice(&msg[1..33]);
    Ok(response)
}

// ---------------------------------------------------------------------------
// perform_handshake -- 4-message HMAC-SHA256 challenge/response exchange
// ---------------------------------------------------------------------------

/// Perform the HMAC-SHA256 cookie challenge/response handshake.
///
/// This runs AFTER TLS is established. Both sides prove they know the shared
/// cookie via a 4-message binary exchange:
///
/// 1. Initiator sends NAME (their name + creation)
/// 2. Acceptor sends CHALLENGE (their name + creation + random challenge)
/// 3. Initiator sends REPLY (response to challenge + own challenge)
/// 4. Acceptor sends ACK (response to initiator's challenge)
///
/// Returns `(remote_name, remote_creation)` on success, or an error string.
fn perform_handshake(
    stream: &mut (impl Read + Write),
    state: &NodeState,
    is_initiator: bool,
) -> Result<(String, u8), String> {
    let creation = state.creation();

    if is_initiator {
        // Step 1: Send our name
        send_name(stream, &state.name, creation)?;

        // Step 2: Receive their name + challenge
        let (remote_name, remote_creation, their_challenge) = recv_challenge(stream)?;

        // Step 3: Compute response + generate our own challenge
        let our_response = compute_response(&state.cookie, &their_challenge);
        let our_challenge = generate_challenge();
        send_challenge_reply(stream, &our_response, &our_challenge)?;

        // Step 4: Receive and verify their response to our challenge
        let their_response = recv_challenge_ack(stream)?;
        if !verify_response(&state.cookie, &our_challenge, &their_response) {
            return Err(format!(
                "cookie mismatch: authentication failed from {}",
                remote_name
            ));
        }

        Ok((remote_name, remote_creation))
    } else {
        // Step 1: Receive their name
        let (remote_name, remote_creation) = recv_name(stream)?;

        // Tiebreaker: if we already have a session to this remote, reject.
        // The node with the lexicographically smaller name keeps its outgoing
        // connection; the other drops.
        {
            let sessions = state.sessions.read();
            if sessions.contains_key(&remote_name) {
                return Err("already connected".to_string());
            }
        }

        // Step 2: Generate our challenge and send it
        let our_challenge = generate_challenge();
        send_challenge(stream, &state.name, creation, &our_challenge)?;

        // Step 3: Receive their response + their challenge
        let (their_response, their_challenge) = recv_challenge_reply(stream)?;

        // Verify their response to our challenge
        if !verify_response(&state.cookie, &our_challenge, &their_response) {
            return Err(format!(
                "cookie mismatch: authentication failed from {}",
                remote_name
            ));
        }

        // Step 4: Compute our response to their challenge and send ACK
        let our_response = compute_response(&state.cookie, &their_challenge);
        send_challenge_ack(stream, &our_response)?;

        Ok((remote_name, remote_creation))
    }
}

// ---------------------------------------------------------------------------
// register_session -- inserts authenticated session into NodeState
// ---------------------------------------------------------------------------

/// Register an authenticated session in `NodeState`.
///
/// Checks for duplicate connections. If the remote_name already has a session,
/// applies the tiebreaker: the node with the lexicographically smaller name
/// keeps its connection. If we lose, returns an error.
fn register_session(
    state: &NodeState,
    remote_name: String,
    remote_creation: u8,
    node_id: u16,
    stream: NodeStream,
) -> Result<Arc<NodeSession>, String> {
    let mut sessions = state.sessions.write();

    // Tiebreaker for duplicate connections
    if sessions.contains_key(&remote_name) {
        // Lexicographically smaller name wins
        if state.name < remote_name {
            // We are smaller -- keep our existing connection, reject this one
            return Err(format!("duplicate connection to {}: keeping existing", remote_name));
        } else {
            // We are larger -- this new connection wins, remove old
            sessions.remove(&remote_name);
            let mut id_map = state.node_id_map.write();
            // Find and remove the old node_id mapping
            id_map.retain(|_, v| v != &remote_name);
        }
    }

    let session = Arc::new(NodeSession {
        remote_name: remote_name.clone(),
        remote_creation,
        node_id,
        stream: Mutex::new(stream),
        shutdown: AtomicBool::new(false),
        connected_at: Instant::now(),
    });

    sessions.insert(remote_name.clone(), Arc::clone(&session));
    drop(sessions);

    let mut id_map = state.node_id_map.write();
    id_map.insert(node_id, remote_name);

    Ok(session)
}

// ---------------------------------------------------------------------------
// Ephemeral TLS certificate generation
// ---------------------------------------------------------------------------

/// Generate an ephemeral ECDSA P-256 self-signed certificate and private key.
///
/// The certificate is minimal and structurally valid enough for rustls's
/// `with_single_cert()` to accept it. It is never validated by clients
/// (we skip cert verification), so it only needs to be well-formed DER.
///
/// Uses ring's `EcdsaKeyPair::generate_pkcs8` for key generation and
/// constructs a minimal X.509 v3 certificate programmatically.
fn generate_ephemeral_cert() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let rng = SystemRandom::new();

    // Generate ECDSA P-256 key pair in PKCS#8 format
    let pkcs8_bytes = EcdsaKeyPair::generate_pkcs8(
        &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &rng,
    )
    .expect("ECDSA P-256 key generation failed");

    let key_pair = EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        pkcs8_bytes.as_ref(),
        &rng,
    )
    .expect("ECDSA key pair from PKCS#8 failed");

    // Extract the public key (uncompressed point: 0x04 || x || y, 65 bytes)
    let public_key = key_pair.public_key().as_ref();

    // Build minimal self-signed X.509 v3 DER certificate
    let tbs_cert = build_tbs_certificate(public_key);
    let signature_bytes = key_pair
        .sign(&rng, &tbs_cert)
        .expect("ECDSA signing failed");

    let cert_der = wrap_signed_certificate(&tbs_cert, signature_bytes.as_ref());

    let key_der = PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(pkcs8_bytes.as_ref().to_vec()),
    );

    (CertificateDer::from(cert_der), key_der)
}

/// Build the TBS (To-Be-Signed) Certificate portion of an X.509 v3 cert.
///
/// This is a minimal ASN.1 DER structure:
/// - Version: v3
/// - Serial: 1
/// - Signature algorithm: ECDSA with SHA-256
/// - Issuer: CN=snow-node
/// - Validity: 2020-01-01 to 2099-12-31 (effectively forever)
/// - Subject: CN=snow-node
/// - Subject Public Key Info: ECDSA P-256
fn build_tbs_certificate(public_key: &[u8]) -> Vec<u8> {
    // OID for ECDSA with SHA-256: 1.2.840.10045.4.3.2
    let oid_ecdsa_sha256: &[u8] = &[0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
    // OID for EC public key: 1.2.840.10045.2.1
    let oid_ec_public_key: &[u8] = &[0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
    // OID for P-256 curve (secp256r1): 1.2.840.10045.3.1.7
    let oid_secp256r1: &[u8] = &[0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];

    let mut tbs = Vec::with_capacity(256);

    // version [0] EXPLICIT INTEGER v3 (2)
    let version = &[0xA0, 0x03, 0x02, 0x01, 0x02];

    // serialNumber INTEGER 1
    let serial = &[0x02, 0x01, 0x01];

    // signature AlgorithmIdentifier (ECDSA-SHA256)
    let sig_alg = der_sequence(&[oid_ecdsa_sha256]);

    // issuer: RDNSequence with CN=snow-node
    let issuer = build_dn(b"snow-node");

    // validity: NotBefore 2020-01-01, NotAfter 2099-12-31
    let not_before = der_utc_time(b"200101000000Z");
    let not_after = der_utc_time(b"991231235959Z");
    let validity = der_sequence(&[&not_before, &not_after]);

    // subject: same as issuer
    let subject = build_dn(b"snow-node");

    // subjectPublicKeyInfo
    let spki_alg = der_sequence(&[oid_ec_public_key, oid_secp256r1]);
    let pub_key_bits = der_bit_string(public_key);
    let spki = der_sequence(&[&spki_alg, &pub_key_bits]);

    // Assemble TBS Certificate SEQUENCE
    tbs.extend_from_slice(version);
    tbs.extend_from_slice(serial);
    tbs.extend_from_slice(&sig_alg);
    tbs.extend_from_slice(&issuer);
    tbs.extend_from_slice(&validity);
    tbs.extend_from_slice(&subject);
    tbs.extend_from_slice(&spki);

    der_sequence_from_bytes(&tbs)
}

/// Wrap the TBS certificate + signature into a full X.509 Certificate SEQUENCE.
fn wrap_signed_certificate(tbs_cert: &[u8], signature: &[u8]) -> Vec<u8> {
    // OID for ECDSA with SHA-256
    let oid_ecdsa_sha256: &[u8] = &[0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
    let sig_alg = der_sequence(&[oid_ecdsa_sha256]);
    let sig_bits = der_bit_string(signature);

    let mut cert = Vec::with_capacity(tbs_cert.len() + sig_alg.len() + sig_bits.len() + 8);
    cert.extend_from_slice(tbs_cert);
    cert.extend_from_slice(&sig_alg);
    cert.extend_from_slice(&sig_bits);

    der_sequence_from_bytes(&cert)
}

// ---------------------------------------------------------------------------
// ASN.1 DER encoding helpers
// ---------------------------------------------------------------------------

/// Encode a DER SEQUENCE from pre-encoded contents.
fn der_sequence_from_bytes(contents: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(contents.len() + 4);
    out.push(0x30); // SEQUENCE tag
    der_push_length(&mut out, contents.len());
    out.extend_from_slice(contents);
    out
}

/// Encode a DER SEQUENCE from multiple pre-encoded elements.
fn der_sequence(elements: &[&[u8]]) -> Vec<u8> {
    let total_len: usize = elements.iter().map(|e| e.len()).sum();
    let mut out = Vec::with_capacity(total_len + 4);
    out.push(0x30); // SEQUENCE tag
    der_push_length(&mut out, total_len);
    for e in elements {
        out.extend_from_slice(e);
    }
    out
}

/// Encode a DER BIT STRING (with zero unused bits).
fn der_bit_string(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    out.push(0x03); // BIT STRING tag
    der_push_length(&mut out, data.len() + 1); // +1 for unused-bits byte
    out.push(0x00); // zero unused bits
    out.extend_from_slice(data);
    out
}

/// Encode a DER UTCTime.
fn der_utc_time(time_str: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(time_str.len() + 2);
    out.push(0x17); // UTCTime tag
    der_push_length(&mut out, time_str.len());
    out.extend_from_slice(time_str);
    out
}

/// Build a minimal Distinguished Name: SEQUENCE { SET { SEQUENCE { OID(CN), UTF8String(name) } } }
fn build_dn(cn: &[u8]) -> Vec<u8> {
    // OID for CommonName: 2.5.4.3
    let oid_cn: &[u8] = &[0x06, 0x03, 0x55, 0x04, 0x03];

    // UTF8String for the CN value
    let mut cn_value = Vec::with_capacity(cn.len() + 2);
    cn_value.push(0x0C); // UTF8String tag
    der_push_length(&mut cn_value, cn.len());
    cn_value.extend_from_slice(cn);

    // SEQUENCE { OID, UTF8String }
    let attr = der_sequence(&[oid_cn, &cn_value]);
    // SET { SEQUENCE }
    let rdn = der_set(&[&attr]);
    // SEQUENCE { SET }
    der_sequence(&[&rdn])
}

/// Encode a DER SET from pre-encoded elements.
fn der_set(elements: &[&[u8]]) -> Vec<u8> {
    let total_len: usize = elements.iter().map(|e| e.len()).sum();
    let mut out = Vec::with_capacity(total_len + 4);
    out.push(0x31); // SET tag
    der_push_length(&mut out, total_len);
    for e in elements {
        out.extend_from_slice(e);
    }
    out
}

/// Push DER length encoding (short or long form).
fn der_push_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.push(0x81);
        out.push(len as u8);
    } else {
        out.push(0x82);
        out.push((len >> 8) as u8);
        out.push(len as u8);
    }
}

// ---------------------------------------------------------------------------
// TLS configuration builders
// ---------------------------------------------------------------------------

/// Build the TLS server config for accepting incoming node connections.
///
/// Uses the ephemeral self-signed certificate. No client authentication
/// is required (trust is established by the cookie challenge in Plan 02).
fn build_node_server_config(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
) -> Arc<ServerConfig> {
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("TLS server config with ephemeral cert failed");
    Arc::new(config)
}

/// Build the TLS client config for connecting to remote nodes.
///
/// Certificate verification is intentionally skipped. Trust is established
/// by the HMAC-SHA256 cookie challenge/response (Plan 02), not by PKI.
fn build_node_client_config() -> Arc<ClientConfig> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipCertVerification))
        .with_no_client_auth();
    Arc::new(config)
}

// ---------------------------------------------------------------------------
// SkipCertVerification -- trusts all server certificates
// ---------------------------------------------------------------------------

/// A `ServerCertVerifier` that accepts any certificate without validation.
///
/// This is intentional: inter-node TLS provides encryption and integrity,
/// while authentication is handled by the HMAC-SHA256 cookie challenge
/// that runs after the TLS handshake completes.
#[derive(Debug)]
struct SkipCertVerification;

impl ServerCertVerifier for SkipCertVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// ---------------------------------------------------------------------------
// Node name parsing
// ---------------------------------------------------------------------------

/// Parse a node name string into (name_part, host, port).
///
/// Accepted formats:
/// - `"name@host"` -> (name, host, 9000)  (default port)
/// - `"name@host:port"` -> (name, host, parsed_port)
///
/// Returns `Err` for invalid formats (no @, empty parts, invalid port).
pub fn parse_node_name(name: &str) -> Result<(&str, &str, u16), String> {
    let at_pos = name.find('@').ok_or_else(|| {
        format!("invalid node name '{}': missing '@' separator", name)
    })?;

    let name_part = &name[..at_pos];
    let host_port = &name[at_pos + 1..];

    if name_part.is_empty() {
        return Err(format!("invalid node name '{}': empty name part", name));
    }

    if host_port.is_empty() {
        return Err(format!("invalid node name '{}': empty host part", name));
    }

    // Check for port: "host:port"
    if let Some(colon_pos) = host_port.rfind(':') {
        let host = &host_port[..colon_pos];
        let port_str = &host_port[colon_pos + 1..];

        if host.is_empty() {
            return Err(format!("invalid node name '{}': empty host part", name));
        }

        let port: u16 = port_str.parse().map_err(|_| {
            format!("invalid node name '{}': invalid port '{}'", name, port_str)
        })?;

        Ok((name_part, host, port))
    } else {
        Ok((name_part, host_port, 9000))
    }
}

// ---------------------------------------------------------------------------
// TCP listener and accept loop
// ---------------------------------------------------------------------------

/// Accept loop for incoming node connections.
///
/// Runs on a dedicated OS thread. Accepts TCP connections and (for now)
/// drops them with a stub comment. Plan 02 will add the TLS handshake
/// and cookie authentication here.
fn accept_loop(listener: TcpListener, shutdown: &AtomicBool) {
    // Use non-blocking mode with periodic shutdown checks.
    listener
        .set_nonblocking(true)
        .expect("set_nonblocking failed on node listener");

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        match listener.accept() {
            Ok((_stream, _addr)) => {
                // Plan 02: perform TLS handshake + cookie authentication here.
                // For now, drop the connection.
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No pending connection -- brief sleep to avoid busy-wait,
                // then check shutdown flag again.
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(_e) => {
                // Transient accept error -- continue looping.
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// snow_node_start -- extern "C" entry point
// ---------------------------------------------------------------------------

/// Initialize the local node and start listening for connections.
///
/// Called from compiled Snow code via `Node.start("name@host", cookie: "secret")`.
///
/// # Arguments
/// - `name_ptr`, `name_len`: UTF-8 node name ("name@host" or "name@host:port")
/// - `cookie_ptr`, `cookie_len`: UTF-8 shared secret
///
/// # Returns
/// - `0` on success
/// - `-1` if node already started
/// - `-2` if TCP bind failed
#[no_mangle]
pub extern "C" fn snow_node_start(
    name_ptr: *const u8,
    name_len: u64,
    cookie_ptr: *const u8,
    cookie_len: u64,
) -> i64 {
    // Already initialized?
    if NODE_STATE.get().is_some() {
        return -1;
    }

    // Extract name and cookie from raw pointers
    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => return -3,
        }
    };

    let cookie = unsafe {
        let slice = std::slice::from_raw_parts(cookie_ptr, cookie_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => return -3,
        }
    };

    // Parse "name@host" or "name@host:port"
    let (_name_part, host, port) = match parse_node_name(&name) {
        Ok(parsed) => parsed,
        Err(_) => return -3,
    };

    let host_owned = host.to_string();

    // Generate ephemeral TLS certificate
    let (cert, key) = generate_ephemeral_cert();

    // Build TLS configs
    let tls_server_config = build_node_server_config(cert, key);
    let tls_client_config = build_node_client_config();

    // Bind TCP listener
    let bind_addr = format!("{}:{}", host_owned, port);
    let listener = match TcpListener::bind(&bind_addr) {
        Ok(l) => l,
        Err(_) => return -2,
    };

    // Determine actual port (may differ if port 0 was requested)
    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);

    // Initialize the global node state
    let _state = NODE_STATE.get_or_init(|| NodeState {
        name: name.clone(),
        host: host_owned,
        port: actual_port,
        cookie,
        creation: AtomicU8::new(1),
        next_node_id: AtomicU16::new(1),
        tls_server_config,
        tls_client_config,
        sessions: RwLock::new(FxHashMap::default()),
        node_id_map: RwLock::new(FxHashMap::default()),
        listener_shutdown: AtomicBool::new(false),
    });

    // Spawn accept loop on a background thread.
    // Access the shutdown flag via the static NODE_STATE, which is 'static.
    std::thread::spawn(move || {
        let state = NODE_STATE.get().expect("NODE_STATE initialized above");
        accept_loop(listener, &state.listener_shutdown);
    });

    0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_node_name() {
        // Standard: name@host -> default port 9000
        let (name, host, port) = parse_node_name("foo@localhost").unwrap();
        assert_eq!(name, "foo");
        assert_eq!(host, "localhost");
        assert_eq!(port, 9000);

        // With explicit port
        let (name, host, port) = parse_node_name("bar@10.0.0.1:4000").unwrap();
        assert_eq!(name, "bar");
        assert_eq!(host, "10.0.0.1");
        assert_eq!(port, 4000);

        // Error: no @ symbol
        assert!(parse_node_name("invalid").is_err());

        // Error: empty name part
        assert!(parse_node_name("@host").is_err());

        // Error: empty host part
        assert!(parse_node_name("name@").is_err());
    }

    #[test]
    fn test_parse_node_name_edge_cases() {
        // IPv6-style host (no port) -- the colon is in the host part
        // but since we use rfind, "name@::1" would parse as host="::" port="1"
        // which is actually valid for our use case (connect to port 1 on ::).
        // For real IPv6, users would use brackets: "name@[::1]:9000"

        // Invalid port
        assert!(parse_node_name("name@host:abc").is_err());
        assert!(parse_node_name("name@host:99999").is_err());
    }

    #[test]
    fn test_generate_ephemeral_cert() {
        // Ensure ring crypto provider is installed
        let _ = rustls::crypto::ring::default_provider().install_default();

        let (cert, key) = generate_ephemeral_cert();

        // Certificate should be non-empty DER
        assert!(!cert.as_ref().is_empty());

        // Key should be non-empty
        match &key {
            PrivateKeyDer::Pkcs8(k) => assert!(!k.secret_pkcs8_der().is_empty()),
            _ => panic!("Expected PKCS#8 key"),
        }

        // The cert + key should be accepted by ServerConfig
        let _config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("ServerConfig should accept ephemeral cert");
    }

    #[test]
    fn test_build_tls_configs() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let (cert, key) = generate_ephemeral_cert();
        let _server = build_node_server_config(cert, key);
        let _client = build_node_client_config();
    }

    #[test]
    fn test_node_state_accessor_before_init() {
        // node_state() returns None when snow_node_start hasn't been called.
        // NOTE: Since tests share the process, if another test initializes
        // NODE_STATE first, this may return Some. We test the accessor itself.
        let _result = node_state(); // should not panic
    }

    #[test]
    fn test_compute_response_deterministic() {
        // Same inputs must produce the same output
        let cookie = "secret_cookie";
        let challenge = [42u8; 32];
        let r1 = compute_response(cookie, &challenge);
        let r2 = compute_response(cookie, &challenge);
        assert_eq!(r1, r2);

        // Different challenge produces different output
        let different_challenge = [99u8; 32];
        let r3 = compute_response(cookie, &different_challenge);
        assert_ne!(r1, r3);
    }

    #[test]
    fn test_verify_response_correct() {
        let cookie = "my_cookie";
        let challenge = generate_challenge();
        let response = compute_response(cookie, &challenge);
        assert!(verify_response(cookie, &challenge, &response));
    }

    #[test]
    fn test_verify_response_wrong_cookie() {
        let challenge = generate_challenge();
        let response = compute_response("correct_cookie", &challenge);
        // Wrong cookie should fail verification
        assert!(!verify_response("wrong_cookie", &challenge, &response));
    }

    #[test]
    fn test_snow_node_start_binds_listener() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Use port 0 to get an OS-assigned port (avoids conflicts)
        let name = b"test@127.0.0.1:0";
        let cookie = b"secret";
        let result = snow_node_start(
            name.as_ptr(),
            name.len() as u64,
            cookie.as_ptr(),
            cookie.len() as u64,
        );

        // Either success (0) or already initialized (-1) if another test ran first.
        // Both are acceptable in a test environment with shared process state.
        assert!(result == 0 || result == -1, "unexpected result: {}", result);

        // node_state should return Some after initialization
        if result == 0 {
            let state = node_state().expect("node_state should be initialized");
            assert!(state.port > 0, "port should be assigned");
            assert_eq!(state.cookie, "secret");
            assert_eq!(state.creation(), 1);

            // assign_node_id should start at 1 and increment
            let id1 = state.assign_node_id();
            let id2 = state.assign_node_id();
            assert_eq!(id1, 1);
            assert_eq!(id2, 2);

            // Signal shutdown to clean up the listener thread
            state.listener_shutdown.store(true, Ordering::Relaxed);
        }
    }
}
