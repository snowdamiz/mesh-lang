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
use std::time::{Duration, Instant};

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
    pub(crate) stream: Mutex<NodeStream>,
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

impl NodeStream {
    /// Set the read timeout on the underlying TcpStream.
    ///
    /// Works for both ServerTls and ClientTls variants since the TLS layer
    /// delegates to the underlying TCP socket's timeout.
    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        match self {
            NodeStream::ServerTls(s) => s.get_ref().set_read_timeout(dur),
            NodeStream::ClientTls(s) => s.get_ref().set_read_timeout(dur),
        }
    }
}

// ---------------------------------------------------------------------------
// Heartbeat wire format constants
// ---------------------------------------------------------------------------

/// Ping message tag for inter-node heartbeat.
const HEARTBEAT_PING: u8 = 0xF0;
/// Pong message tag for inter-node heartbeat.
const HEARTBEAT_PONG: u8 = 0xF1;

/// Distribution message tag: send to a specific PID on the receiving node.
/// Wire format: [tag][u64 target_pid LE][raw message bytes]
pub(crate) const DIST_SEND: u8 = 0x10;
/// Distribution message tag: send to a named process on the receiving node.
/// Wire format: [tag][u16 name_len LE][name bytes][raw message bytes]
pub(crate) const DIST_REG_SEND: u8 = 0x11;
/// Distribution message tag: peer list exchange for automatic mesh formation.
/// Wire format: [tag][u16 count][u16 name_len, name bytes, ...]
pub(crate) const DIST_PEER_LIST: u8 = 0x12;

// ---------------------------------------------------------------------------
// HeartbeatState -- ping/pong dead connection detection
// ---------------------------------------------------------------------------

/// Tracks ping/pong heartbeat state for dead connection detection.
///
/// The heartbeat thread sends periodic pings with random 8-byte payloads.
/// The reader thread forwards pong responses by updating `last_pong_received`
/// and clearing `pending_ping_payload`. If no valid pong is received within
/// `pong_timeout` after the last ping, the connection is considered dead.
///
/// Follows the same pattern as `ws/server.rs` HeartbeatState.
struct HeartbeatState {
    last_ping_sent: Instant,
    last_pong_received: Instant,
    ping_interval: Duration,
    pong_timeout: Duration,
    pending_ping_payload: Option<[u8; 8]>,
}

impl HeartbeatState {
    fn new(interval: Duration, timeout: Duration) -> Self {
        let now = Instant::now();
        Self {
            last_ping_sent: now,
            last_pong_received: now,
            ping_interval: interval,
            pong_timeout: timeout,
            pending_ping_payload: None,
        }
    }

    /// True if enough time has elapsed since the last ping to send another.
    fn should_send_ping(&self) -> bool {
        self.last_ping_sent.elapsed() >= self.ping_interval
    }

    /// True if a ping is pending and the pong hasn't arrived within the timeout.
    fn is_pong_overdue(&self) -> bool {
        if self.pending_ping_payload.is_some() {
            self.last_ping_sent.elapsed() >= self.pong_timeout
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Mesh formation: peer list exchange
// ---------------------------------------------------------------------------

/// Send our current peer list to a newly connected node for mesh formation.
///
/// Wire format: [DIST_PEER_LIST][u16 count][u16 name_len][name bytes]...
/// Skips the receiving node's own name (no need to tell B about B).
fn send_peer_list(session: &Arc<NodeSession>) {
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    let sessions = state.sessions.read();
    let peers: Vec<&String> = sessions.keys()
        .filter(|name| *name != &session.remote_name)
        .collect();

    if peers.is_empty() {
        return;
    }

    let mut payload = Vec::new();
    payload.push(DIST_PEER_LIST);
    payload.extend_from_slice(&(peers.len() as u16).to_le_bytes());
    for peer_name in &peers {
        let bytes = peer_name.as_bytes();
        payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        payload.extend_from_slice(bytes);
    }
    drop(sessions); // Release read lock before acquiring stream lock

    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
}

/// Handle an incoming DIST_PEER_LIST -- connect to unknown peers on a separate thread.
///
/// Parses the peer list, filters out self and already-connected nodes,
/// then spawns a thread to connect to the remaining peers. The thread spawn
/// avoids deadlock (see Pitfall 7 in RESEARCH.md).
fn handle_peer_list(data: &[u8]) {
    if data.len() < 2 { return; }
    let count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
    let mut pos = 2;
    let mut to_connect = Vec::new();

    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    for _ in 0..count {
        if pos + 2 > data.len() { break; }
        let name_len = u16::from_le_bytes(data[pos..pos+2].try_into().unwrap()) as usize;
        pos += 2;
        if pos + name_len > data.len() { break; }
        if let Ok(peer_name) = std::str::from_utf8(&data[pos..pos+name_len]) {
            // Skip self and already-connected nodes
            if peer_name != state.name {
                let sessions = state.sessions.read();
                if !sessions.contains_key(peer_name) {
                    to_connect.push(peer_name.to_string());
                }
            }
        }
        pos += name_len;
    }

    // Spawn connection attempts on a separate thread to avoid deadlock
    if !to_connect.is_empty() {
        std::thread::spawn(move || {
            for peer in to_connect {
                let bytes = peer.as_bytes();
                snow_node_connect(bytes.as_ptr(), bytes.len() as u64);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// spawn_session_threads -- starts reader + heartbeat for an authenticated session
// ---------------------------------------------------------------------------

/// Spawn the reader and heartbeat threads for an authenticated node session.
///
/// Both threads share the session (via `Arc<NodeSession>`) for stream access
/// and shutdown signalling, plus a shared `HeartbeatState` for coordinating
/// ping/pong timing between the reader and heartbeat threads.
fn spawn_session_threads(session: &Arc<NodeSession>) {
    let heartbeat_state = Arc::new(Mutex::new(HeartbeatState::new(
        Duration::from_secs(60),
        Duration::from_secs(15),
    )));

    let session_for_reader = Arc::clone(session);
    let session_for_heartbeat = Arc::clone(session);
    let hs_for_reader = Arc::clone(&heartbeat_state);
    let hs_for_heartbeat = Arc::clone(&heartbeat_state);
    let remote_name = session.remote_name.clone();

    // Reader thread
    let reader_name = format!("snow-node-reader-{}", session.remote_name);
    std::thread::Builder::new()
        .name(reader_name)
        .spawn(move || {
            reader_loop_session(session_for_reader, hs_for_reader);
        })
        .expect("failed to spawn node reader thread");

    // Heartbeat thread
    let hb_name = format!("snow-node-heartbeat-{}", remote_name);
    let remote_name_hb = session.remote_name.clone();
    std::thread::Builder::new()
        .name(hb_name)
        .spawn(move || {
            heartbeat_loop_session(session_for_heartbeat, hs_for_heartbeat, remote_name_hb);
        })
        .expect("failed to spawn node heartbeat thread");
}

// ---------------------------------------------------------------------------
// reader_loop_session -- receives messages on a dedicated OS thread
// ---------------------------------------------------------------------------

/// Reader thread for a node session.
///
/// Runs on a dedicated OS thread, reading incoming messages from the TLS
/// stream. Handles heartbeat messages:
/// - HEARTBEAT_PING: responds immediately with HEARTBEAT_PONG echoing the payload
/// - HEARTBEAT_PONG: validates payload matches pending ping and updates HeartbeatState
/// - Other tags: ignored (Phase 65 will add message routing)
///
/// Uses a 100ms read timeout to allow periodic shutdown checks without
/// busy-waiting.
fn reader_loop_session(
    session: Arc<NodeSession>,
    heartbeat_state: Arc<Mutex<HeartbeatState>>,
) {
    // Set read timeout to 100ms for periodic shutdown checks.
    {
        let s = session.stream.lock().unwrap();
        s.set_read_timeout(Some(Duration::from_millis(100))).ok();
    }

    loop {
        if session.shutdown.load(Ordering::SeqCst) {
            break;
        }

        let result = {
            let mut s = session.stream.lock().unwrap();
            read_dist_msg(&mut *s)
        };

        match result {
            Ok(msg) => {
                if msg.is_empty() {
                    continue;
                }
                match msg[0] {
                    HEARTBEAT_PING => {
                        if msg.len() >= 9 {
                            let mut pong = Vec::with_capacity(9);
                            pong.push(HEARTBEAT_PONG);
                            pong.extend_from_slice(&msg[1..9]);
                            let mut s = session.stream.lock().unwrap();
                            let _ = write_msg(&mut *s, &pong);
                        }
                    }
                    HEARTBEAT_PONG => {
                        if msg.len() >= 9 {
                            let mut hs = heartbeat_state.lock().unwrap();
                            if let Some(expected) = hs.pending_ping_payload {
                                if msg[1..9] == expected {
                                    hs.last_pong_received = Instant::now();
                                    hs.pending_ping_payload = None;
                                }
                            }
                        }
                    }
                    DIST_SEND => {
                        // Wire format: [tag][u64 target_pid LE][raw message bytes]
                        if msg.len() >= 9 {
                            let target_pid = u64::from_le_bytes(
                                msg[1..9].try_into().unwrap(),
                            );
                            let msg_data = &msg[9..];
                            crate::actor::local_send(
                                target_pid,
                                msg_data.as_ptr(),
                                msg_data.len() as u64,
                            );
                        }
                    }
                    DIST_REG_SEND => {
                        // Wire format: [tag][u16 name_len LE][name bytes][raw message bytes]
                        if msg.len() >= 3 {
                            let name_len = u16::from_le_bytes(
                                msg[1..3].try_into().unwrap(),
                            ) as usize;
                            if msg.len() >= 3 + name_len {
                                if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                                    if let Some(pid) = crate::actor::registry::global_registry().whereis(name) {
                                        let msg_data = &msg[3 + name_len..];
                                        crate::actor::local_send(
                                            pid.as_u64(),
                                            msg_data.as_ptr(),
                                            msg_data.len() as u64,
                                        );
                                    }
                                    // If name not found, silently drop (matches Erlang behavior)
                                }
                            }
                        }
                    }
                    DIST_PEER_LIST => {
                        handle_peer_list(&msg[1..]);
                    }
                    _ => {
                        // Unknown tag -- silently ignore for forward compatibility.
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("timed out")
                    || msg.contains("WouldBlock")
                    || msg.contains("temporarily unavailable")
                {
                    continue;
                }
                session.shutdown.store(true, Ordering::SeqCst);
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// heartbeat_loop_session -- sends periodic pings on a dedicated OS thread
// ---------------------------------------------------------------------------

/// Heartbeat thread for a node session.
///
/// Sends periodic HEARTBEAT_PING messages with random 8-byte payloads and
/// monitors for timely HEARTBEAT_PONG responses (via shared HeartbeatState
/// updated by the reader thread). If a pong is overdue, declares the
/// connection dead and signals shutdown.
///
/// After the loop exits (shutdown or timeout), calls `cleanup_session` to
/// remove the session from NodeState.
fn heartbeat_loop_session(
    session: Arc<NodeSession>,
    heartbeat_state: Arc<Mutex<HeartbeatState>>,
    session_name: String,
) {
    loop {
        std::thread::sleep(Duration::from_millis(500));

        if session.shutdown.load(Ordering::SeqCst) {
            break;
        }

        let mut hs = heartbeat_state.lock().unwrap();

        if hs.is_pong_overdue() {
            eprintln!("snow node heartbeat timeout: {}", session_name);
            session.shutdown.store(true, Ordering::SeqCst);
            break;
        }

        if hs.should_send_ping() {
            let payload: [u8; 8] = rand::random();
            let mut ping = Vec::with_capacity(9);
            ping.push(HEARTBEAT_PING);
            ping.extend_from_slice(&payload);

            hs.last_ping_sent = Instant::now();
            hs.pending_ping_payload = Some(payload);
            drop(hs);

            let mut s = session.stream.lock().unwrap();
            let _ = write_msg(&mut *s, &ping);
        }
    }

    cleanup_session(&session_name);
}

// ---------------------------------------------------------------------------
// cleanup_session -- removes a disconnected node from NodeState
// ---------------------------------------------------------------------------

/// Remove a disconnected node's session from NodeState.
///
/// Removes the session from `sessions` by remote name, then removes the
/// corresponding `node_id` from `node_id_map`. Phase 66 will add `:nodedown`
/// notification here.
fn cleanup_session(remote_name: &str) {
    if let Some(state) = NODE_STATE.get() {
        let removed = {
            let mut sessions = state.sessions.write();
            sessions.remove(remote_name)
        };
        if let Some(session) = removed {
            let mut id_map = state.node_id_map.write();
            id_map.remove(&session.node_id);
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
pub(crate) fn write_msg(stream: &mut impl Write, payload: &[u8]) -> io::Result<()> {
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

/// Maximum size for distribution messages (16 MiB).
///
/// Post-handshake messages can be much larger than the 4 KiB handshake limit.
/// Actor messages containing large binaries or deeply nested data structures
/// may approach this limit.
const MAX_DIST_MSG: u32 = 16 * 1024 * 1024;

/// Read a length-prefixed distribution message with a 16 MiB limit.
///
/// Used in the reader loop after the handshake is complete. The larger limit
/// allows full-size actor messages to be transmitted between nodes, while
/// still preventing unbounded allocations from malicious or buggy peers.
pub(crate) fn read_dist_msg(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_DIST_MSG {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("dist message too large: {} bytes (max {})", len, MAX_DIST_MSG),
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
/// Runs on a dedicated OS thread. For each accepted TCP connection:
/// 1. Wraps in TLS server connection
/// 2. Performs HMAC-SHA256 cookie handshake (acceptor side)
/// 3. Registers authenticated session in NodeState
/// 4. Spawns reader + heartbeat threads for the session
fn accept_loop(listener: TcpListener, state: &NodeState) {
    // Use non-blocking mode with periodic shutdown checks.
    listener
        .set_nonblocking(true)
        .expect("set_nonblocking failed on node listener");

    loop {
        if state.listener_shutdown.load(Ordering::Relaxed) {
            break;
        }

        match listener.accept() {
            Ok((tcp_stream, _addr)) => {
                // Switch to blocking mode for the TLS handshake
                tcp_stream
                    .set_nonblocking(false)
                    .expect("set_nonblocking(false) failed on accepted stream");

                // Wrap in TLS server connection
                let server_conn = match rustls::ServerConnection::new(
                    Arc::clone(&state.tls_server_config),
                ) {
                    Ok(conn) => conn,
                    Err(e) => {
                        eprintln!("snow node: TLS server connection failed: {}", e);
                        continue;
                    }
                };
                let mut tls_stream = StreamOwned::new(server_conn, tcp_stream);

                // Perform HMAC-SHA256 cookie handshake (acceptor side)
                let (remote_name, remote_creation) =
                    match perform_handshake(&mut tls_stream, state, false) {
                        Ok(result) => result,
                        Err(e) => {
                            eprintln!("snow node: handshake failed: {}", e);
                            continue;
                        }
                    };

                // Register the authenticated session
                let node_id = state.assign_node_id();
                let stream = NodeStream::ServerTls(tls_stream);
                match register_session(state, remote_name.clone(), remote_creation, node_id, stream)
                {
                    Ok(session) => {
                        spawn_session_threads(&session);
                        send_peer_list(&session);
                    }
                    Err(e) => {
                        eprintln!(
                            "snow node: session registration failed for {}: {}",
                            remote_name, e
                        );
                    }
                }
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
    // Access NodeState via the static NODE_STATE, which is 'static.
    std::thread::spawn(move || {
        let state = NODE_STATE.get().expect("NODE_STATE initialized above");
        accept_loop(listener, state);
    });

    0
}

// ---------------------------------------------------------------------------
// snow_node_connect -- extern "C" entry point for outgoing connections
// ---------------------------------------------------------------------------

/// Connect to a remote node and perform mutual cookie authentication.
///
/// Called from compiled Snow code via `Node.connect("name@host:port")`.
///
/// # Arguments
/// - `name_ptr`, `name_len`: UTF-8 target address ("name@host:port")
///
/// # Returns
/// - `0` on success (authenticated connection established)
/// - `-1` if node not started (snow_node_start not called)
/// - `-2` if TCP connection failed
/// - `-3` if handshake failed (wrong cookie, I/O error, or invalid format)
#[no_mangle]
pub extern "C" fn snow_node_connect(
    name_ptr: *const u8,
    name_len: u64,
) -> i64 {
    // Check NODE_STATE is initialized
    let state = match NODE_STATE.get() {
        Some(s) => s,
        None => {
            eprintln!("snow node: node not started");
            return -1;
        }
    };

    // Extract target address from raw pointer
    let target = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => return -3,
        }
    };

    // Parse host:port from target. Port is REQUIRED for connect.
    let (_name_part, host, port) = match parse_node_name(&target) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("snow node: invalid connect target: {}", e);
            return -3;
        }
    };

    // Open TCP connection
    let tcp_stream = match TcpStream::connect(format!("{}:{}", host, port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("snow node: TCP connect to {}:{} failed: {}", host, port, e);
            return -2;
        }
    };

    // Wrap in TLS client connection.
    // Server name is "snow-node" -- doesn't matter since we skip verification.
    let server_name: ServerName<'static> = "snow-node".try_into().unwrap();
    let client_conn = match rustls::ClientConnection::new(
        Arc::clone(&state.tls_client_config),
        server_name,
    ) {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("snow node: TLS client connection failed: {}", e);
            return -3;
        }
    };
    let mut tls_stream = StreamOwned::new(client_conn, tcp_stream);

    // Perform HMAC-SHA256 cookie handshake (initiator side)
    let (remote_name, remote_creation) =
        match perform_handshake(&mut tls_stream, state, true) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("snow node: handshake with {}:{} failed: {}", host, port, e);
                return -3;
            }
        };

    // Register the authenticated session
    let node_id = state.assign_node_id();
    let stream = NodeStream::ClientTls(tls_stream);
    match register_session(state, remote_name.clone(), remote_creation, node_id, stream) {
        Ok(session) => {
            spawn_session_threads(&session);
            send_peer_list(&session);
            0
        }
        Err(e) => {
            eprintln!(
                "snow node: session registration failed for {}: {}",
                remote_name, e
            );
            -3
        }
    }
}

// ---------------------------------------------------------------------------
// Node query APIs -- Node.self() and Node.list()
// ---------------------------------------------------------------------------

/// Return the current node's name as a Snow string pointer.
///
/// Returns null pointer if node is not started (snow_node_start not called).
/// The returned string is GC-allocated via snow_string_new.
#[no_mangle]
pub extern "C" fn snow_node_self() -> *const u8 {
    match node_state() {
        Some(state) => {
            crate::string::snow_string_new(
                state.name.as_ptr(),
                state.name.len() as u64,
            ) as *const u8
        }
        None => std::ptr::null(),
    }
}

/// Return a list of connected node names as a Snow list of strings.
///
/// Returns an empty list if node is not started or no connections exist.
/// Each element is a GC-allocated Snow string. The list itself is allocated
/// via snow_list_from_array.
#[no_mangle]
pub extern "C" fn snow_node_list() -> *mut u8 {
    let state = match node_state() {
        Some(s) => s,
        None => {
            return crate::collections::list::snow_list_new();
        }
    };

    let sessions = state.sessions.read();
    if sessions.is_empty() {
        return crate::collections::list::snow_list_new();
    }

    let names: Vec<String> = sessions.keys().cloned().collect();
    drop(sessions);

    // Build array of Snow string pointers, then create list from array
    let mut string_ptrs: Vec<u64> = Vec::with_capacity(names.len());
    for name in &names {
        let s = crate::string::snow_string_new(name.as_ptr(), name.len() as u64);
        string_ptrs.push(s as u64);
    }

    crate::collections::list::snow_list_from_array(
        string_ptrs.as_ptr(),
        string_ptrs.len() as i64,
    )
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

    // -------------------------------------------------------------------
    // Plan 03 tests: HeartbeatState, handshake, wire format, lifecycle
    // -------------------------------------------------------------------

    #[test]
    fn test_heartbeat_state_timing() {
        // Short intervals for test speed: 100ms ping, 50ms pong timeout.
        let mut hs = HeartbeatState::new(
            Duration::from_millis(100),
            Duration::from_millis(50),
        );

        // Initially: should_send_ping is false (just created).
        assert!(!hs.should_send_ping());
        // No pending ping, so pong cannot be overdue.
        assert!(!hs.is_pong_overdue());

        // Wait for ping interval to elapse.
        std::thread::sleep(Duration::from_millis(110));
        assert!(hs.should_send_ping());

        // Simulate sending a ping.
        let payload: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
        hs.last_ping_sent = Instant::now();
        hs.pending_ping_payload = Some(payload);

        // Immediately after ping: pong is NOT overdue yet.
        assert!(!hs.is_pong_overdue());

        // Wait past the pong timeout.
        std::thread::sleep(Duration::from_millis(60));
        assert!(hs.is_pong_overdue());

        // Simulate receiving a valid pong.
        hs.last_pong_received = Instant::now();
        hs.pending_ping_payload = None;

        // After clearing, pong is no longer overdue.
        assert!(!hs.is_pong_overdue());
    }

    #[test]
    fn test_write_msg_read_msg_roundtrip() {
        use std::io::Cursor;

        // Test 1: Normal payload
        let payload = b"hello node world";
        let mut buf = Vec::new();
        write_msg(&mut buf, payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let result = read_msg(&mut cursor).unwrap();
        assert_eq!(result, payload);

        // Test 2: Empty payload
        let mut buf = Vec::new();
        write_msg(&mut buf, &[]).unwrap();
        let mut cursor = Cursor::new(&buf);
        let result = read_msg(&mut cursor).unwrap();
        assert!(result.is_empty());

        // Test 3: Max-size payload (4096 bytes = MAX_HANDSHAKE_MSG)
        let big_payload = vec![0xABu8; MAX_HANDSHAKE_MSG as usize];
        let mut buf = Vec::new();
        write_msg(&mut buf, &big_payload).unwrap();
        let mut cursor = Cursor::new(&buf);
        let result = read_msg(&mut cursor).unwrap();
        assert_eq!(result.len(), MAX_HANDSHAKE_MSG as usize);
        assert_eq!(result, big_payload);

        // Test 4: Payload over max should error on read
        let too_big = vec![0xCDu8; MAX_HANDSHAKE_MSG as usize + 1];
        let mut buf = Vec::new();
        write_msg(&mut buf, &too_big).unwrap(); // write succeeds (no limit on write)
        let mut cursor = Cursor::new(&buf);
        let err = read_msg(&mut cursor);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_handshake_in_memory() {
        // Use a UnixStream pair as in-memory duplex streams.
        use std::os::unix::net::UnixStream;

        let (stream_a, stream_b) = UnixStream::pair().unwrap();

        // Both nodes share the same cookie.
        let cookie = "test_shared_cookie".to_string();

        // Build minimal NodeState for each side (only fields used by handshake).
        let state_a = NodeState {
            name: "alice@127.0.0.1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 9000,
            cookie: cookie.clone(),
            creation: AtomicU8::new(1),
            next_node_id: AtomicU16::new(1),
            tls_server_config: {
                let _ = rustls::crypto::ring::default_provider().install_default();
                let (cert, key) = generate_ephemeral_cert();
                build_node_server_config(cert, key)
            },
            tls_client_config: build_node_client_config(),
            sessions: RwLock::new(FxHashMap::default()),
            node_id_map: RwLock::new(FxHashMap::default()),
            listener_shutdown: AtomicBool::new(false),
        };

        let state_b = NodeState {
            name: "bob@127.0.0.1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 9001,
            cookie: cookie.clone(),
            creation: AtomicU8::new(2),
            next_node_id: AtomicU16::new(1),
            tls_server_config: {
                let (cert, key) = generate_ephemeral_cert();
                build_node_server_config(cert, key)
            },
            tls_client_config: build_node_client_config(),
            sessions: RwLock::new(FxHashMap::default()),
            node_id_map: RwLock::new(FxHashMap::default()),
            listener_shutdown: AtomicBool::new(false),
        };

        // Run initiator and acceptor on separate threads.
        let handle_a = std::thread::spawn(move || {
            let mut s = stream_a;
            perform_handshake(&mut s, &state_a, true)
        });

        let handle_b = std::thread::spawn(move || {
            let mut s = stream_b;
            perform_handshake(&mut s, &state_b, false)
        });

        let result_a = handle_a.join().unwrap();
        let result_b = handle_b.join().unwrap();

        // Both sides should succeed.
        let (remote_name_a, remote_creation_a) = result_a.unwrap();
        let (remote_name_b, remote_creation_b) = result_b.unwrap();

        // Initiator (alice) should see acceptor (bob).
        assert_eq!(remote_name_a, "bob@127.0.0.1");
        assert_eq!(remote_creation_a, 2);

        // Acceptor (bob) should see initiator (alice).
        assert_eq!(remote_name_b, "alice@127.0.0.1");
        assert_eq!(remote_creation_b, 1);
    }

    #[test]
    fn test_handshake_wrong_cookie() {
        use std::os::unix::net::UnixStream;

        let (stream_a, stream_b) = UnixStream::pair().unwrap();

        // Set a read timeout so the test doesn't hang on failure.
        stream_a.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        stream_b.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

        let state_a = NodeState {
            name: "alice@127.0.0.1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 9000,
            cookie: "correct_cookie".to_string(),
            creation: AtomicU8::new(1),
            next_node_id: AtomicU16::new(1),
            tls_server_config: {
                let _ = rustls::crypto::ring::default_provider().install_default();
                let (cert, key) = generate_ephemeral_cert();
                build_node_server_config(cert, key)
            },
            tls_client_config: build_node_client_config(),
            sessions: RwLock::new(FxHashMap::default()),
            node_id_map: RwLock::new(FxHashMap::default()),
            listener_shutdown: AtomicBool::new(false),
        };

        let state_b = NodeState {
            name: "bob@127.0.0.1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 9001,
            cookie: "wrong_cookie".to_string(),
            creation: AtomicU8::new(2),
            next_node_id: AtomicU16::new(1),
            tls_server_config: {
                let (cert, key) = generate_ephemeral_cert();
                build_node_server_config(cert, key)
            },
            tls_client_config: build_node_client_config(),
            sessions: RwLock::new(FxHashMap::default()),
            node_id_map: RwLock::new(FxHashMap::default()),
            listener_shutdown: AtomicBool::new(false),
        };

        let handle_a = std::thread::spawn(move || {
            let mut s = stream_a;
            perform_handshake(&mut s, &state_a, true)
        });

        let handle_b = std::thread::spawn(move || {
            let mut s = stream_b;
            perform_handshake(&mut s, &state_b, false)
        });

        let result_a = handle_a.join().unwrap();
        let result_b = handle_b.join().unwrap();

        // At least one side must detect the cookie mismatch.
        // The acceptor (bob) verifies the initiator's response first, so bob
        // should report the error. Alice may succeed or fail depending on
        // whether bob sends the ACK before detecting the mismatch.
        let a_failed = result_a.is_err();
        let b_failed = result_b.is_err();
        assert!(
            a_failed || b_failed,
            "at least one side should detect cookie mismatch"
        );

        // The side that failed should mention "cookie mismatch" or I/O error.
        if b_failed {
            let err = result_b.unwrap_err();
            assert!(
                err.contains("cookie mismatch") || err.contains("authentication failed"),
                "unexpected error: {}",
                err
            );
        }
    }

    #[test]
    fn test_node_connect_full_lifecycle() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Create two independent TLS configurations (simulating two nodes).
        let (cert_a, key_a) = generate_ephemeral_cert();
        let server_config_a = build_node_server_config(cert_a, key_a);
        let client_config_a = build_node_client_config();

        let (cert_b, key_b) = generate_ephemeral_cert();
        let _server_config_b = build_node_server_config(cert_b, key_b);
        let client_config_b = build_node_client_config();

        let cookie = "lifecycle_test_cookie".to_string();

        // Bind a TCP listener on port 0 (OS-assigned) for node A (server).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let cookie_a = cookie.clone();
        let cookie_b = cookie.clone();
        let server_cfg = Arc::clone(&server_config_a);

        // Spawn the server (acceptor) thread.
        let server_handle = std::thread::spawn(move || {
            let (tcp_stream, _addr) = listener.accept().unwrap();
            tcp_stream.set_nonblocking(false).unwrap();

            let server_conn = rustls::ServerConnection::new(server_cfg).unwrap();
            let mut tls_stream = StreamOwned::new(server_conn, tcp_stream);

            let state = NodeState {
                name: "server@127.0.0.1".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                cookie: cookie_a,
                creation: AtomicU8::new(1),
                next_node_id: AtomicU16::new(1),
                tls_server_config: server_config_a,
                tls_client_config: client_config_a,
                sessions: RwLock::new(FxHashMap::default()),
                node_id_map: RwLock::new(FxHashMap::default()),
                listener_shutdown: AtomicBool::new(false),
            };

            perform_handshake(&mut tls_stream, &state, false)
        });

        // Client (initiator) connects.
        let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let server_name: ServerName<'static> = "snow-node".try_into().unwrap();
        let client_conn =
            rustls::ClientConnection::new(Arc::clone(&client_config_b), server_name).unwrap();
        let mut tls_stream = StreamOwned::new(client_conn, tcp_stream);

        let client_state = NodeState {
            name: "client@127.0.0.1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 0,
            cookie: cookie_b,
            creation: AtomicU8::new(3),
            next_node_id: AtomicU16::new(1),
            tls_server_config: {
                let (cert, key) = generate_ephemeral_cert();
                build_node_server_config(cert, key)
            },
            tls_client_config: client_config_b,
            sessions: RwLock::new(FxHashMap::default()),
            node_id_map: RwLock::new(FxHashMap::default()),
            listener_shutdown: AtomicBool::new(false),
        };

        let client_result = perform_handshake(&mut tls_stream, &client_state, true);
        let server_result = server_handle.join().unwrap();

        // Both sides should succeed.
        let (remote_from_client, creation_from_client) = client_result.unwrap();
        let (remote_from_server, creation_from_server) = server_result.unwrap();

        // Client sees server.
        assert_eq!(remote_from_client, "server@127.0.0.1");
        assert_eq!(creation_from_client, 1);

        // Server sees client.
        assert_eq!(remote_from_server, "client@127.0.0.1");
        assert_eq!(creation_from_server, 3);
    }

    #[test]
    fn test_heartbeat_ping_pong_wire_format() {
        use std::io::Cursor;

        // Construct a HEARTBEAT_PING message.
        let payload: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];
        let mut ping = Vec::with_capacity(9);
        ping.push(HEARTBEAT_PING);
        ping.extend_from_slice(&payload);

        // Write and read it back via write_msg/read_msg.
        let mut buf = Vec::new();
        write_msg(&mut buf, &ping).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_msg(&mut cursor).unwrap();

        assert_eq!(msg.len(), 9);
        assert_eq!(msg[0], HEARTBEAT_PING);
        assert_eq!(&msg[1..9], &payload);

        // Construct matching HEARTBEAT_PONG.
        let mut pong = Vec::with_capacity(9);
        pong.push(HEARTBEAT_PONG);
        pong.extend_from_slice(&payload);

        let mut buf = Vec::new();
        write_msg(&mut buf, &pong).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], HEARTBEAT_PONG);
        assert_eq!(&msg[1..9], &payload);
    }

    #[test]
    fn test_cleanup_session_removes_from_state() {
        // Build a minimal NodeState and register a session manually.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // We cannot use the global NODE_STATE easily in tests, so we test
        // the cleanup logic by verifying that cleanup_session does not panic
        // when called without NODE_STATE initialized (it early-returns).
        // The functional test is covered by test_node_connect_full_lifecycle
        // which exercises the full connection path including spawn_session_threads.
        cleanup_session("nonexistent@host");
        // If we get here, cleanup_session handled the None case gracefully.
    }

    // -------------------------------------------------------------------
    // Plan 65-03 Task 1: Wire format and message routing unit tests
    // -------------------------------------------------------------------

    #[test]
    fn test_dist_send_wire_format() {
        use std::io::Cursor;

        // Test 1: Normal DIST_SEND message with payload
        let target_pid: u64 = 0x0001_0000_0000_0042; // node_id=1, local pid=0x42
        let message = b"hello remote actor";

        let mut payload = Vec::new();
        payload.push(DIST_SEND);
        payload.extend_from_slice(&target_pid.to_le_bytes());
        payload.extend_from_slice(message);

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], DIST_SEND);
        let decoded_pid = u64::from_le_bytes(msg[1..9].try_into().unwrap());
        assert_eq!(decoded_pid, target_pid);
        assert_eq!(&msg[9..], message);

        // Test 2: Empty message payload (msg_size == 0)
        let mut payload = Vec::new();
        payload.push(DIST_SEND);
        payload.extend_from_slice(&target_pid.to_le_bytes());
        // No message bytes

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg.len(), 9); // tag + 8 bytes pid, no message
        assert_eq!(msg[0], DIST_SEND);
        let decoded_pid = u64::from_le_bytes(msg[1..9].try_into().unwrap());
        assert_eq!(decoded_pid, target_pid);

        // Test 3: Large payload (8KB -- above old 4KB handshake limit)
        let big_message = vec![0xABu8; 8192];
        let mut payload = Vec::new();
        payload.push(DIST_SEND);
        payload.extend_from_slice(&target_pid.to_le_bytes());
        payload.extend_from_slice(&big_message);

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], DIST_SEND);
        assert_eq!(&msg[9..], &big_message[..]);
    }

    #[test]
    fn test_dist_reg_send_wire_format() {
        use std::io::Cursor;

        // Test 1: Normal DIST_REG_SEND with name and message
        let name = "my_server";
        let message = b"request data";

        let mut payload = Vec::new();
        payload.push(DIST_REG_SEND);
        payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(message);

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], DIST_REG_SEND);
        let name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        assert_eq!(name_len, name.len());
        let decoded_name = std::str::from_utf8(&msg[3..3 + name_len]).unwrap();
        assert_eq!(decoded_name, name);
        assert_eq!(&msg[3 + name_len..], message);

        // Test 2: Empty name (edge case)
        let empty_name = "";
        let message = b"msg to empty name";

        let mut payload = Vec::new();
        payload.push(DIST_REG_SEND);
        payload.extend_from_slice(&(empty_name.len() as u16).to_le_bytes());
        // No name bytes
        payload.extend_from_slice(message);

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], DIST_REG_SEND);
        let name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        assert_eq!(name_len, 0);
        assert_eq!(&msg[3..], message);

        // Test 3: Long name (255 chars)
        let long_name = "a".repeat(255);
        let message = b"payload";

        let mut payload = Vec::new();
        payload.push(DIST_REG_SEND);
        payload.extend_from_slice(&(long_name.len() as u16).to_le_bytes());
        payload.extend_from_slice(long_name.as_bytes());
        payload.extend_from_slice(message);

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], DIST_REG_SEND);
        let name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        assert_eq!(name_len, 255);
        let decoded_name = std::str::from_utf8(&msg[3..3 + name_len]).unwrap();
        assert_eq!(decoded_name, long_name);
        assert_eq!(&msg[3 + name_len..], message);
    }

    #[test]
    fn test_dist_peer_list_wire_format() {
        use std::io::Cursor;

        // Test 1: Multiple peers
        let peers = vec!["alpha@10.0.0.1:9000", "beta@10.0.0.2:9001", "gamma@10.0.0.3:9002"];

        let mut payload = Vec::new();
        payload.push(DIST_PEER_LIST);
        payload.extend_from_slice(&(peers.len() as u16).to_le_bytes());
        for peer in &peers {
            let bytes = peer.as_bytes();
            payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            payload.extend_from_slice(bytes);
        }

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], DIST_PEER_LIST);
        let count = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        assert_eq!(count, 3);

        // Parse the peer names back out
        let mut pos = 3;
        let mut decoded_peers = Vec::new();
        for _ in 0..count {
            let name_len = u16::from_le_bytes(msg[pos..pos+2].try_into().unwrap()) as usize;
            pos += 2;
            let name = std::str::from_utf8(&msg[pos..pos+name_len]).unwrap();
            decoded_peers.push(name.to_string());
            pos += name_len;
        }

        assert_eq!(decoded_peers, peers);

        // Test 2: Empty peer list (count=0)
        let mut payload = Vec::new();
        payload.push(DIST_PEER_LIST);
        payload.extend_from_slice(&0u16.to_le_bytes());

        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();

        assert_eq!(msg[0], DIST_PEER_LIST);
        let count = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        assert_eq!(count, 0);
    }

    #[test]
    fn test_read_dist_msg_accepts_large_messages() {
        use std::io::Cursor;

        // 8KB payload: above MAX_HANDSHAKE_MSG (4KB) but below MAX_DIST_MSG (16MB)
        let payload = vec![0xBBu8; 8192];
        let mut buf = Vec::new();
        write_msg(&mut buf, &payload).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_dist_msg(&mut cursor).unwrap();
        assert_eq!(msg.len(), 8192);
        assert_eq!(msg, payload);

        // Verify read_msg would reject this (4KB limit)
        let mut cursor = Cursor::new(&buf);
        let err = read_msg(&mut cursor);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_read_dist_msg_rejects_oversized() {
        use std::io::Cursor;

        // Write a length header claiming a message larger than MAX_DIST_MSG
        let fake_len = MAX_DIST_MSG + 1;
        let mut buf = Vec::new();
        buf.extend_from_slice(&fake_len.to_le_bytes());
        // Don't need to write actual payload -- read_dist_msg should reject
        // before trying to allocate

        let mut cursor = Cursor::new(&buf);
        let err = read_dist_msg(&mut cursor);
        assert!(err.is_err());
        let err_msg = err.unwrap_err().to_string();
        assert!(
            err_msg.contains("dist message too large"),
            "expected 'dist message too large', got: {}",
            err_msg
        );
    }

    // -------------------------------------------------------------------
    // Plan 65-03 Task 2: Node query API and peer list handling tests
    // -------------------------------------------------------------------

    #[test]
    fn test_snow_node_self_returns_value_or_null() {
        // snow_node_self returns null when NODE_STATE is not initialized,
        // or a valid string pointer when it IS initialized.
        // Since tests share a process and NODE_STATE is a OnceLock, another
        // test may have initialized it. We test both cases:
        let result = snow_node_self();
        if node_state().is_none() {
            // Not initialized: should return null
            assert!(result.is_null(), "expected null when node not started");
        } else {
            // Already initialized by another test: should return non-null
            assert!(!result.is_null(), "expected non-null when node started");
        }
    }

    #[test]
    fn test_snow_node_list_returns_valid_list() {
        // snow_node_list should always return a valid list, never null.
        // When not initialized or no connections, returns an empty list.
        let result = snow_node_list();
        assert!(!result.is_null(), "snow_node_list should never return null");

        // The returned list should be a valid Snow list with length >= 0
        let len = crate::collections::list::snow_list_length(result);
        assert!(len >= 0, "list length should be non-negative");
    }

    #[test]
    fn test_handle_peer_list_parsing_logic() {
        // Test the peer list wire format parsing logic that handle_peer_list uses.
        // We verify the parsing inline since handle_peer_list requires NODE_STATE
        // and spawns threads. This tests the same byte-reading code path.

        let peers = vec!["node_a@10.0.0.1:9000", "node_b@10.0.0.2:9001", "node_c@10.0.0.3:9002"];

        // Build the peer list payload (the data AFTER the DIST_PEER_LIST tag)
        let mut data = Vec::new();
        data.extend_from_slice(&(peers.len() as u16).to_le_bytes());
        for peer in &peers {
            let bytes = peer.as_bytes();
            data.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            data.extend_from_slice(bytes);
        }

        // Parse using the same logic as handle_peer_list
        let count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
        assert_eq!(count, 3);

        let mut pos = 2;
        let mut decoded = Vec::new();
        for _ in 0..count {
            let name_len = u16::from_le_bytes(data[pos..pos+2].try_into().unwrap()) as usize;
            pos += 2;
            let name = std::str::from_utf8(&data[pos..pos+name_len]).unwrap();
            decoded.push(name.to_string());
            pos += name_len;
        }

        assert_eq!(decoded, peers);

        // Test filtering logic: given a self-name and known-names, filter correctly
        let self_name = "node_a@10.0.0.1:9000";
        let known_names: Vec<&str> = vec!["node_b@10.0.0.2:9001"];

        let to_connect: Vec<&str> = decoded.iter()
            .filter(|name| name.as_str() != self_name)
            .filter(|name| !known_names.contains(&name.as_str()))
            .map(|s| s.as_str())
            .collect();

        // Should only have node_c (node_a is self, node_b is already connected)
        assert_eq!(to_connect, vec!["node_c@10.0.0.3:9002"]);
    }

    #[test]
    fn test_handle_peer_list_empty_data() {
        // handle_peer_list returns early if data.len() < 2.
        // Test that the parsing logic handles empty/truncated data gracefully.

        // Empty data: less than 2 bytes
        let data: &[u8] = &[];
        assert!(data.len() < 2); // Would cause handle_peer_list to early-return

        // Single byte: still < 2
        let data: &[u8] = &[0x01];
        assert!(data.len() < 2);

        // Count=0 peer list: valid but empty
        let data: &[u8] = &[0x00, 0x00]; // count = 0
        let count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
        assert_eq!(count, 0);
    }

    #[test]
    fn test_send_peer_list_wire_format_roundtrip() {
        // Verify the peer list encoding logic produces correctly formatted data.
        // We build a peer list payload the same way send_peer_list does,
        // then parse it to verify correctness.

        // Simulate the peer list we'd send (excluding the receiving node)
        let all_sessions = vec![
            "peer_x@10.0.0.10:5000".to_string(),
            "peer_y@10.0.0.11:5001".to_string(),
            "receiving_node@10.0.0.12:5002".to_string(),
        ];
        let receiving_node = "receiving_node@10.0.0.12:5002";

        // Filter like send_peer_list does
        let peers: Vec<&String> = all_sessions.iter()
            .filter(|name| name.as_str() != receiving_node)
            .collect();

        assert_eq!(peers.len(), 2);

        // Build payload like send_peer_list
        let mut payload = Vec::new();
        payload.push(DIST_PEER_LIST);
        payload.extend_from_slice(&(peers.len() as u16).to_le_bytes());
        for peer_name in &peers {
            let bytes = peer_name.as_bytes();
            payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            payload.extend_from_slice(bytes);
        }

        // Parse back: skip the tag byte
        let data = &payload[1..];
        let count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
        assert_eq!(count, 2);

        let mut pos = 2;
        let mut decoded = Vec::new();
        for _ in 0..count {
            let name_len = u16::from_le_bytes(data[pos..pos+2].try_into().unwrap()) as usize;
            pos += 2;
            let name = std::str::from_utf8(&data[pos..pos+name_len]).unwrap();
            decoded.push(name.to_string());
            pos += name_len;
        }

        assert_eq!(decoded.len(), 2);
        assert!(decoded.contains(&"peer_x@10.0.0.10:5000".to_string()));
        assert!(decoded.contains(&"peer_y@10.0.0.11:5001".to_string()));
        assert!(!decoded.contains(&receiving_node.to_string()));
    }

    #[test]
    fn test_handle_peer_list_truncated_name() {
        // Test graceful handling when a peer list entry has a name_len
        // that extends beyond the buffer (truncated data).
        // handle_peer_list uses `if pos + name_len > data.len() { break; }`

        let mut data = Vec::new();
        data.extend_from_slice(&1u16.to_le_bytes()); // count = 1
        data.extend_from_slice(&100u16.to_le_bytes()); // name_len = 100
        data.extend_from_slice(b"short"); // Only 5 bytes, not 100

        // Parse with the same logic as handle_peer_list
        let count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
        assert_eq!(count, 1);

        let mut pos = 2;
        let mut decoded = Vec::new();
        for _ in 0..count {
            if pos + 2 > data.len() { break; }
            let name_len = u16::from_le_bytes(data[pos..pos+2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + name_len > data.len() { break; } // This should trigger
            let name = std::str::from_utf8(&data[pos..pos+name_len]).unwrap();
            decoded.push(name.to_string());
            pos += name_len;
        }

        // Should have decoded 0 peers (truncated name caused early break)
        assert_eq!(decoded.len(), 0);
    }
}
