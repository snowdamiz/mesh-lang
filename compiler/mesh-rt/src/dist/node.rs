//! Node identity, TLS configuration, and TCP listener for Mesh distribution.
//!
//! This module implements the foundational layer for Mesh's distributed actor
//! system. A Mesh runtime becomes a named, addressable node by calling
//! `mesh_node_start`, which:
//!
//! 1. Parses the node name ("name@host" or "name@host:port")
//! 2. Generates an ephemeral ECDSA P-256 self-signed certificate
//! 3. Builds mutually authenticated TLS configs in autonomous mode (manual
//!    protocol-one mode uses the ephemeral-certificate/cookie path)
//! 4. Initializes the global `NODE_STATE` singleton
//! 5. Binds a TCP listener and spawns an accept loop thread
//!
//! ## Trust Model
//!
//! TLS provides confidentiality and integrity. Autonomous peers require mTLS
//! plus a signed, cluster-scoped identity claim. The HMAC-SHA256 cookie
//! handshake remains a compatibility and defense-in-depth layer, with
//! comma-separated keyrings for rolling rotation.
//!
//! In legacy (non-mTLS) mode the client-side TLS config intentionally skips
//! certificate verification, so the cookie handshake is the only peer
//! authentication. Every cookie proof is therefore bound to the RFC 9266
//! `tls-exporter` value of the TLS session it travels over; a relay that
//! terminates TLS towards both peers cannot splice two sessions together by
//! forwarding the handshake messages.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use parking_lot::RwLock;
use ring::rand::SystemRandom;
use ring::signature::{self, EcdsaKeyPair, KeyPair};
use rustc_hash::FxHashMap;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    ClientConfig, DigitallySignedStruct, Error, RootCertStore, ServerConfig, SignatureScheme,
    StreamOwned,
};
use sha2::{Digest, Sha256};

use super::bootstrap::{bootstrap_from_env_with, BootstrapStatus};
use super::discovery::start_from_env as start_discovery_from_env;
use super::protocol::{
    negotiate_protocol, CircuitBreaker, CircuitState, MessageClass, NegotiatedProtocol,
    ProtocolEnvelope, ProtocolHello, RetryBudget, PROTOCOL_V1, PROTOCOL_V2,
};
use crate::io::{alloc_result, err_result, MeshResult};
use crate::string::{mesh_str, MeshString};

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// NodeState -- global singleton for the local node
// ---------------------------------------------------------------------------

/// Global node state, initialized once by `mesh_node_start`.
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
    /// Messages for processes watching a node, sent once when it disconnects.
    pub node_monitors: RwLock<
        FxHashMap<
            String,
            Vec<(
                crate::actor::process::ProcessId,
                crate::actor::heap::MessageBuffer,
            )>,
        >,
    >,
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
// Replica counts are u64 on the wire and in records, and index here as usize.
const _: () = assert!(usize::BITS >= u64::BITS);
static PROTOCOL_BOOT_ID: OnceLock<[u8; 16]> = OnceLock::new();
static ACTIVE_INCOMING_HANDSHAKES: AtomicUsize = AtomicUsize::new(0);
static AUTH_FAILURE_WINDOW: OnceLock<Mutex<FixedWindowCounter>> = OnceLock::new();
static OPERATOR_QUERY_WINDOW: OnceLock<Mutex<FixedWindowCounter>> = OnceLock::new();
const MAX_INCOMING_HANDSHAKES: usize = 64;
const NODE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_AUTH_FAILURES_PER_SECOND: u32 = 128;
const MAX_OPERATOR_QUERIES_PER_SECOND: u32 = 64;

struct FixedWindowCounter {
    started_at: Instant,
    count: u32,
}

impl FixedWindowCounter {
    fn new(now: Instant) -> Self {
        Self {
            started_at: now,
            count: 0,
        }
    }

    fn reset_if_elapsed(&mut self, now: Instant) {
        if now.saturating_duration_since(self.started_at) >= Duration::from_secs(1) {
            self.started_at = now;
            self.count = 0;
        }
    }

    fn below(&mut self, limit: u32, now: Instant) -> bool {
        self.reset_if_elapsed(now);
        self.count < limit
    }

    fn take(&mut self, limit: u32, now: Instant) -> bool {
        if !self.below(limit, now) {
            return false;
        }
        self.count = self.count.saturating_add(1);
        true
    }
}

fn auth_failures_below_limit() -> bool {
    AUTH_FAILURE_WINDOW
        .get_or_init(|| Mutex::new(FixedWindowCounter::new(Instant::now())))
        .lock()
        .unwrap()
        .below(MAX_AUTH_FAILURES_PER_SECOND, Instant::now())
}

fn record_auth_failure() {
    let mut window = AUTH_FAILURE_WINDOW
        .get_or_init(|| Mutex::new(FixedWindowCounter::new(Instant::now())))
        .lock()
        .unwrap();
    window.reset_if_elapsed(Instant::now());
    window.count = window.count.saturating_add(1);
}

fn operator_query_allowed() -> bool {
    OPERATOR_QUERY_WINDOW
        .get_or_init(|| Mutex::new(FixedWindowCounter::new(Instant::now())))
        .lock()
        .unwrap()
        .take(MAX_OPERATOR_QUERIES_PER_SECOND, Instant::now())
}

struct IncomingHandshakeGuard;

impl Drop for IncomingHandshakeGuard {
    fn drop(&mut self) {
        ACTIVE_INCOMING_HANDSHAKES.fetch_sub(1, Ordering::AcqRel);
    }
}

fn local_protocol_hello() -> ProtocolHello {
    ProtocolHello::current(*PROTOCOL_BOOT_ID.get_or_init(rand::random))
}

/// A setting of the process environment, where a node reads its identity.
fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// This node's protocol hello, carrying the signed identity its settings
/// (`env`) give it, which must name it as it is.
fn local_protocol_hello_with_identity(
    local_name: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Result<ProtocolHello, String> {
    let mut hello = local_protocol_hello();
    let autonomous = autonomous_mode_requested();
    let envelope = env(super::identity_claim::IDENTITY_ENVELOPE_ENV);
    let verify_keys = env(super::identity_claim::IDENTITY_VERIFY_KEYS_ENV);
    let cluster_id = env("MESH_CLUSTER_ID");
    match (envelope, verify_keys, cluster_id) {
        (Some(envelope), Some(verify_keys), Some(cluster_id)) => {
            hello.identity_envelope = super::identity_claim::decode_envelope_b64(&envelope)?;
            let claim = super::identity_claim::decode_and_verify_identity(
                &hello.identity_envelope,
                &verify_keys,
                &cluster_id,
                local_name,
                super::identity_claim::unix_millis(),
            )?;
            if claim.stable_node_id
                != env("MESH_STABLE_NODE_ID").unwrap_or_else(|| claim.stable_node_id.clone())
                || claim.roles
                    != super::identity_claim::canonical_roles(
                        &env("MESH_ROLES")
                            .unwrap_or_default()
                            .split(',')
                            .map(str::to_string)
                            .collect::<Vec<_>>(),
                    )?
            {
                return Err("local_node_identity_claim_mismatch".to_string());
            }
        }
        (None, None, None) if !autonomous => {}
        _ if autonomous => return Err("autonomous_mode_requires_signed_node_identity".to_string()),
        _ => return Err("node_identity_configuration_incomplete".to_string()),
    }
    Ok(hello)
}

fn protocol_one_hello() -> ProtocolHello {
    ProtocolHello {
        minimum_version: PROTOCOL_V1,
        maximum_version: PROTOCOL_V1,
        capabilities: super::protocol::Capabilities::default(),
        max_frame_bytes: super::protocol::DEFAULT_MAX_FRAME_BYTES,
        boot_id: [0; 16],
        identity_envelope: Vec::new(),
    }
}

/// Get a reference to the global node state, if initialized.
///
/// Returns `Some` if `mesh_node_start` has been called, `None` otherwise.
/// This is the primary access point for code that needs to check whether
/// the runtime is operating as a named node.
pub fn node_state() -> Option<&'static NodeState> {
    NODE_STATE.get()
}

/// This node's state, for code that runs only on a started node: the code
/// of a session, or of a peer's message, as only a started node has
/// sessions, and a node once started never stops.
fn started_node() -> &'static NodeState {
    node_state().expect("only a started node has sessions")
}

// ---------------------------------------------------------------------------
// Function name registry for remote spawn (Phase 67)
// ---------------------------------------------------------------------------

/// A wrapper around `*const u8` that is `Send + Sync`.
///
/// Function pointers in the registry are valid for the lifetime of the program
/// (they point to compiled code in the text segment) and are never freed.
#[derive(Clone, Copy)]
struct FnPtr(*const u8);
unsafe impl Send for FnPtr {}
unsafe impl Sync for FnPtr {}

/// A function remote nodes may spawn by name, together with the argument
/// signature its generated actor entry expects.
#[derive(Clone)]
struct RegisteredFunction {
    fn_ptr: FnPtr,
    /// One `REMOTE_SPAWN_ARG_*` tag per parameter the actor wrapper loads from
    /// the args buffer. `REMOTE_SPAWN_ARG_UNSUPPORTED` marks parameters that
    /// cannot be supplied over the wire; such entries are never spawned remotely.
    arg_signature: Vec<u8>,
}

/// Global registry mapping function names to their code pointers and
/// expected argument signatures.
///
/// Populated at program startup by codegen-emitted `mesh_register_function`
/// calls. Used by the remote spawn handler to look up a function pointer
/// by name when a DIST_SPAWN request arrives from another node, and to reject
/// requests whose arity or argument types do not match what the generated
/// entry will load from the args buffer.
static FUNCTION_REGISTRY: OnceLock<RwLock<FxHashMap<String, RegisteredFunction>>> = OnceLock::new();

#[derive(Clone)]
struct DeclaredHandlerEntry {
    executable_name: String,
    replication_count: u64,
    fn_ptr: FnPtr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeclaredHandlerRouteMetadata {
    pub runtime_name: String,
    pub replication_count: u64,
}

/// Global registry mapping manifest-approved runtime handler names to the
/// executable symbols and code pointers that may run through the clustered
/// declared-handler path.
static DECLARED_HANDLER_REGISTRY: OnceLock<RwLock<FxHashMap<String, DeclaredHandlerEntry>>> =
    OnceLock::new();

/// Global ordered list of clustered work runtime names that should auto-trigger
/// after the app entrypoint returns.
static STARTUP_WORK_REGISTRY: OnceLock<RwLock<Vec<String>>> = OnceLock::new();
static STARTUP_KEEPALIVE_SPAWNED: AtomicBool = AtomicBool::new(false);
static STARTUP_WORK_TRIGGERED: AtomicBool = AtomicBool::new(false);

/// Get or initialize the function registry.
fn function_registry() -> &'static RwLock<FxHashMap<String, RegisteredFunction>> {
    FUNCTION_REGISTRY.get_or_init(|| RwLock::new(FxHashMap::default()))
}

fn declared_handler_registry() -> &'static RwLock<FxHashMap<String, DeclaredHandlerEntry>> {
    DECLARED_HANDLER_REGISTRY.get_or_init(|| RwLock::new(FxHashMap::default()))
}

pub(crate) fn declared_handler_count() -> usize {
    declared_handler_registry().read().len()
}

fn startup_work_registry() -> &'static RwLock<Vec<String>> {
    STARTUP_WORK_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a function by name for remote spawn.
///
/// Called by codegen-emitted code in the main wrapper at program startup.
/// Each top-level (non-closure) function is registered so that remote nodes
/// can spawn it by name.
///
/// `arg_tags_ptr`/`arg_count` describe the `REMOTE_SPAWN_ARG_*` tag of every
/// parameter the generated actor entry loads from its args buffer. A
/// DIST_SPAWN request is only honoured when its argument tags match this
/// signature exactly; otherwise the generated loads would read past the
/// buffer or reinterpret integers as pointers. `arg_tags_ptr` may be null
/// when `arg_count` is zero.
#[no_mangle]
pub extern "C" fn mesh_register_function(
    name_ptr: *const u8,
    name_len: u64,
    fn_ptr: *const u8,
    arg_tags_ptr: *const u8,
    arg_count: u64,
) {
    if name_ptr.is_null() || fn_ptr.is_null() {
        return;
    }
    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        std::str::from_utf8_unchecked(slice).to_string()
    };
    let arg_signature = if arg_count == 0 {
        Vec::new()
    } else if arg_tags_ptr.is_null() {
        // No signature was supplied for a function that takes arguments, so
        // nothing can be validated: keep it unreachable from remote spawn.
        vec![REMOTE_SPAWN_ARG_UNSUPPORTED; arg_count.min(u16::MAX as u64) as usize]
    } else {
        unsafe { std::slice::from_raw_parts(arg_tags_ptr, arg_count as usize) }.to_vec()
    };
    function_registry().write().insert(
        name,
        RegisteredFunction {
            fn_ptr: FnPtr(fn_ptr),
            arg_signature,
        },
    );
}

#[no_mangle]
pub extern "C" fn mesh_register_declared_handler(
    runtime_name_ptr: *const u8,
    runtime_name_len: u64,
    executable_name_ptr: *const u8,
    executable_name_len: u64,
    replication_count: u64,
    fn_ptr: *const u8,
) {
    if runtime_name_ptr.is_null() || executable_name_ptr.is_null() || fn_ptr.is_null() {
        return;
    }

    let runtime_name = unsafe {
        let slice = std::slice::from_raw_parts(runtime_name_ptr, runtime_name_len as usize);
        std::str::from_utf8_unchecked(slice).to_string()
    };
    let executable_name = unsafe {
        let slice = std::slice::from_raw_parts(executable_name_ptr, executable_name_len as usize);
        std::str::from_utf8_unchecked(slice).to_string()
    };

    if runtime_name.is_empty() || executable_name.is_empty() {
        return;
    }

    declared_handler_registry().write().insert(
        runtime_name,
        DeclaredHandlerEntry {
            executable_name,
            replication_count,
            fn_ptr: FnPtr(fn_ptr),
        },
    );
}

#[no_mangle]
pub extern "C" fn mesh_register_startup_work(runtime_name_ptr: *const u8, runtime_name_len: u64) {
    if runtime_name_ptr.is_null() {
        log_startup_rejected_without_identity("", STARTUP_RUNTIME_NAME_MISSING);
        return;
    }

    let runtime_name = unsafe {
        let slice = std::slice::from_raw_parts(runtime_name_ptr, runtime_name_len as usize);
        std::str::from_utf8_unchecked(slice).to_string()
    };

    let identity = match startup_work_identity(&runtime_name) {
        Ok(identity) => identity,
        Err(reason) => {
            log_startup_rejected_without_identity(&runtime_name, &reason);
            return;
        }
    };

    let mut registrations = startup_work_registry().write();
    if registrations
        .iter()
        .any(|existing| existing == &identity.runtime_name)
    {
        log_startup_rejected(&identity, None, None, None, STARTUP_DUPLICATE_REGISTRATION);
        return;
    }

    registrations.push(identity.runtime_name.clone());
    log_startup_registered(&identity);
}

#[no_mangle]
pub extern "C" fn mesh_trigger_startup_work() {
    let runtime_names = startup_work_registry().read().clone();
    if runtime_names.is_empty() {
        return;
    }

    if STARTUP_WORK_TRIGGERED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let authority = crate::dist::continuity::continuity_registry().authority_status();
    trigger_startup_work_registrations(
        &runtime_names,
        node_state().is_some(),
        authority.cluster_role,
        authority.promotion_epoch,
        spawn_startup_work_actor,
        spawn_startup_keepalive_actor,
    );
}

/// Look up a remotely spawnable function and its argument signature by name.
fn lookup_registered_function(name: &str) -> Option<RegisteredFunction> {
    function_registry().read().get(name).cloned()
}

fn lookup_declared_handler(name: &str) -> Option<DeclaredHandlerEntry> {
    declared_handler_registry().read().get(name).cloned()
}

pub(crate) fn lookup_declared_handler_route_metadata(
    fn_ptr: *mut u8,
) -> Option<DeclaredHandlerRouteMetadata> {
    if fn_ptr.is_null() {
        return None;
    }

    declared_handler_registry()
        .read()
        .iter()
        .find_map(|(runtime_name, entry)| {
            std::ptr::eq(entry.fn_ptr.0, fn_ptr.cast_const()).then(|| {
                DeclaredHandlerRouteMetadata {
                    runtime_name: runtime_name.clone(),
                    replication_count: entry.replication_count,
                }
            })
        })
}

#[cfg(test)]
pub(crate) fn clear_declared_handler_registry_for_test() {
    declared_handler_registry().write().clear();
}

/// Held (shared) by every test that plays a peer of the test node, whose
/// sessions make that peer a member of the cluster.
#[cfg(test)]
pub(crate) static TEST_PEERS: parking_lot::RwLock<()> = parking_lot::const_rwlock(());

/// The clustered runtime state tests share (declared handlers, the
/// continuity registry, the test node's members), for one test at a time,
/// with no test peer connected: clustered work placed now stays here.
#[cfg(test)]
pub(crate) fn declared_handler_registry_test_lock() -> parking_lot::RwLockWriteGuard<'static, ()> {
    TEST_PEERS.write()
}

fn lookup_declared_handler_executable(name: &str) -> Option<DeclaredHandlerEntry> {
    declared_handler_registry()
        .read()
        .values()
        .find(|entry| entry.executable_name == name)
        .cloned()
}

fn required_replica_count_for_replication_count(replication_count: u64) -> Result<u64, String> {
    if replication_count == 0 {
        return Err("invalid_replication_count".to_string());
    }

    Ok(replication_count.saturating_sub(1))
}

pub(crate) fn required_replica_count_for_runtime_name(runtime_name: &str) -> Result<u64, String> {
    let entry = lookup_declared_handler(runtime_name)
        .ok_or_else(|| format!("declared_handler_not_registered:{runtime_name}"))?;
    required_replica_count_for_replication_count(entry.replication_count)
}

fn startup_effective_required_replica_count(
    desired_required_replica_count: u64,
    saw_peer: bool,
) -> u64 {
    if saw_peer || desired_required_replica_count == 0 || desired_required_replica_count > 1 {
        desired_required_replica_count
    } else {
        0
    }
}

fn automatic_recovery_effective_required_replica_count(
    request_key: &str,
    desired_required_replica_count: u64,
    saw_peer: bool,
) -> u64 {
    if request_key.starts_with(STARTUP_REQUEST_KEY_PREFIX) {
        startup_effective_required_replica_count(desired_required_replica_count, saw_peer)
    } else {
        desired_required_replica_count
    }
}

/// Monotonic counter for generating unique spawn request IDs.
static SPAWN_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
/// How long a remote spawn waits for its peer's reply, which it sends as
/// soon as the process is spawned.
const REMOTE_SPAWN_TIMEOUT: Duration = Duration::from_secs(30);
/// Monotonic counter for generating unique continuity prepare request IDs.
static CONTINUITY_PREPARE_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
/// Correlation IDs for multiplexed clustered HTTP dispatch over peer sessions.
static HTTP_ROUTE_CORRELATION_ID: AtomicU64 = AtomicU64::new(1);
/// Correlation IDs for embedded OpenRaft RPCs over peer sessions.
static CONSENSUS_RPC_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static PEER_RETRY_BUDGETS: OnceLock<Mutex<FxHashMap<String, RetryBudget>>> = OnceLock::new();
static PEER_CIRCUITS: OnceLock<Mutex<FxHashMap<String, CircuitBreaker>>> = OnceLock::new();

fn peer_circuits() -> &'static Mutex<FxHashMap<String, CircuitBreaker>> {
    PEER_CIRCUITS.get_or_init(|| Mutex::new(FxHashMap::default()))
}

fn peer_circuit_allow(peer: &str, now: Instant) -> bool {
    let allowed = peer_circuits()
        .lock()
        .unwrap()
        .entry(peer.to_string())
        .or_insert_with(|| CircuitBreaker::new(3, Duration::from_secs(5)).unwrap())
        .allow(now);
    if !allowed {
        crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_circuit_rejection();
    }
    allowed
}

fn record_peer_transport_success(peer: &str) {
    if let Some(circuit) = peer_circuits().lock().unwrap().get_mut(peer) {
        circuit.record_success();
    }
}

pub(crate) fn record_peer_transport_failure(peer: &str, now: Instant) {
    peer_circuits()
        .lock()
        .unwrap()
        .entry(peer.to_string())
        .or_insert_with(|| CircuitBreaker::new(3, Duration::from_secs(5)).unwrap())
        .record_failure(now);
}

pub(crate) fn peer_circuit_open(peer: &str, now: Instant) -> bool {
    peer_circuit_state(peer, now) == CircuitState::Open
}

fn peer_circuit_state(peer: &str, now: Instant) -> CircuitState {
    peer_circuits()
        .lock()
        .unwrap()
        .get(peer)
        .map_or(CircuitState::Closed, |circuit| circuit.state(now))
}

fn record_peer_original_attempt(peer: &str, now: Instant) {
    let mut budgets = PEER_RETRY_BUDGETS
        .get_or_init(|| Mutex::new(FxHashMap::default()))
        .lock()
        .unwrap();
    budgets
        .entry(peer.to_string())
        .or_insert_with(|| {
            RetryBudget::new(
                crate::dist::routing::runtime_retry_budget_percent(),
                1,
                Duration::from_secs(10),
                now,
            )
            .expect("validated retry budget defaults")
        })
        .record_original(now);
}

fn allow_peer_retry(peer: &str, now: Instant) -> bool {
    let allowed = PEER_RETRY_BUDGETS
        .get_or_init(|| Mutex::new(FxHashMap::default()))
        .lock()
        .unwrap()
        .entry(peer.to_string())
        .or_insert_with(|| {
            RetryBudget::new(
                crate::dist::routing::runtime_retry_budget_percent(),
                1,
                Duration::from_secs(10),
                now,
            )
            .unwrap()
        })
        .try_retry(now);
    if allowed {
        crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_retry();
    }
    allowed
}

// ---------------------------------------------------------------------------
// Node sessions
// ---------------------------------------------------------------------------

type PendingCooperativeReplies<T> =
    std::sync::Mutex<FxHashMap<u64, crate::actor::CooperativeSender<Result<T, String>>>>;
type PendingOperatorQueries =
    std::sync::Mutex<FxHashMap<u64, mpsc::Sender<Result<Vec<u8>, String>>>>;
type PendingConsensusRpcs =
    std::sync::Mutex<FxHashMap<u64, tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>>>;

struct RemoteSessionEndpoint {
    remote_name: String,
    remote_creation: u8,
    node_id: u16,
    direction: SessionDirection,
}

/// Represents a connection to a remote node.
///
/// Holds the authenticated TLS stream, identity info, and shutdown flag.
pub struct NodeSession {
    /// Full name of the remote node
    pub remote_name: String,
    /// Creation counter of the remote node at connection time
    pub remote_creation: u8,
    /// The node_id assigned to this remote node (for PID encoding)
    pub node_id: u16,
    /// Whether this transport was accepted locally or initiated outbound.
    pub(crate) direction: SessionDirection,
    /// The TLS stream, shared between writer and reader threads. A
    /// `parking_lot` mutex so the reader can hand it over fairly.
    pub(crate) stream: parking_lot::Mutex<NodeStream>,
    /// Encrypted bytes a heartbeat left in the stream's send buffer when the
    /// socket took no more: the writer flushes them when it is idle. Set
    /// and cleared under the stream lock.
    tls_output_pending: AtomicBool,
    /// Signals the session's reader/heartbeat threads to stop
    pub shutdown: AtomicBool,
    /// When this connection was established
    pub connected_at: Instant,
    /// Version, bounds, and features negotiated during the authenticated handshake.
    pub negotiated_protocol: NegotiatedProtocol,
    /// Cluster/stable identity authenticated by the protocol-two signed claim.
    pub remote_identity: Option<super::identity_claim::NodeIdentityClaim>,
    /// Pending remote spawn requests, waiting for the local id of the
    /// process the peer spawned (DIST_SPAWN_REPLY).
    pub(crate) pending_spawns: PendingCooperativeReplies<u64>,
    /// Set once the peer's global registry snapshot has been merged.
    global_names_received: AtomicBool,
    /// Pending continuity prepare requests waiting for a replica ack.
    /// The sender side resolves to Ok(()) on ack or Err(reason) on reject/timeout.
    pub(crate) pending_continuity_prepares: PendingCooperativeReplies<()>,
    /// Pending read-only operator queries waiting for a reply frame.
    /// The sender side resolves to Ok(payload) on success or Err(reason) on reject.
    pub(crate) pending_operator_queries: PendingOperatorQueries,
    /// Pending embedded-consensus RPCs. Tokio one-shot channels keep OpenRaft's
    /// async network path off the distribution reader thread.
    pub(crate) pending_consensus_rpcs: PendingConsensusRpcs,
    /// Pending protocol-two HTTP dispatches multiplexed over this peer session.
    pub(crate) pending_http_routes: PendingCooperativeReplies<Vec<u8>>,
    /// Pending two-phase owner reservation replies.
    pending_http_reservations: PendingCooperativeReplies<()>,
    /// Accepted owner reservations, held until the matching payload starts or expires.
    accepted_http_reservations: std::sync::Mutex<FxHashMap<u64, AcceptedHttpReservation>>,
    persistent: bool,
    control_outbound: crossbeam_channel::Sender<OutboundFrame>,
    admission_outbound: crossbeam_channel::Sender<OutboundFrame>,
    continuity_outbound: crossbeam_channel::Sender<OutboundFrame>,
    application_outbound: crossbeam_channel::Sender<OutboundFrame>,
    snapshot_outbound: crossbeam_channel::Sender<OutboundFrame>,
    outbound_receivers: Mutex<Option<OutboundReceivers>>,
    control_queued_bytes: AtomicUsize,
    admission_queued_bytes: AtomicUsize,
    continuity_queued_bytes: AtomicUsize,
    application_queued_bytes: AtomicUsize,
    snapshot_queued_bytes: AtomicUsize,
}

const CONTROL_QUEUE_ITEMS: usize = 256;
const CONTROL_QUEUE_BYTES: usize = 4 * 1024 * 1024;
const ADMISSION_QUEUE_ITEMS: usize = 4_096;
const ADMISSION_QUEUE_BYTES: usize = 16 * 1024 * 1024;
const CONTINUITY_QUEUE_ITEMS: usize = 4_096;
const CONTINUITY_QUEUE_BYTES: usize = 32 * 1024 * 1024;
const APPLICATION_QUEUE_ITEMS: usize = 1_024;
const APPLICATION_QUEUE_BYTES: usize = 64 * 1024 * 1024;
const SNAPSHOT_QUEUE_ITEMS: usize = 64;
const SNAPSHOT_QUEUE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONSECUTIVE_CONTROL_FRAMES: usize = 4;
const MAX_OUTBOUND_WRITE_BATCH: usize = 32;
const CONTINUITY_PREPARE_DISPATCH_ITEMS: usize = 8_192;
const CONTINUITY_PREPARE_WORKERS: usize = 64;

struct ContinuityPrepareTask {
    session: Arc<NodeSession>,
    request_id: u64,
    record: crate::dist::continuity::ContinuityRecord,
}

static CONTINUITY_PREPARE_DISPATCHER: OnceLock<crossbeam_channel::Sender<ContinuityPrepareTask>> =
    OnceLock::new();

#[derive(Clone, Copy, Debug)]
pub(crate) enum OutboundClass {
    Control,
    Admission,
    Continuity,
    Application,
    Snapshot,
}

struct OutboundFrame {
    payload: Vec<u8>,
    class: OutboundClass,
}

struct OutboundReceivers {
    control: crossbeam_channel::Receiver<OutboundFrame>,
    admission: crossbeam_channel::Receiver<OutboundFrame>,
    continuity: crossbeam_channel::Receiver<OutboundFrame>,
    application: crossbeam_channel::Receiver<OutboundFrame>,
    snapshot: crossbeam_channel::Receiver<OutboundFrame>,
}

impl NodeSession {
    /// A pid this peer sent for one of its own processes, as this node
    /// addresses it: a peer sends its local pids as they are.
    fn peer_pid(&self, raw: u64) -> crate::actor::process::ProcessId {
        use crate::actor::process::ProcessId;
        let pid = ProcessId(raw);
        if pid.is_local() {
            ProcessId::from_remote(self.node_id, self.remote_creation, pid.local_id())
        } else {
            pid
        }
    }

    pub(crate) fn remote_has_role(&self, role: &str) -> bool {
        self.remote_identity
            .as_ref()
            .is_some_and(|identity| identity.roles.iter().any(|candidate| candidate == role))
    }

    fn new(
        endpoint: RemoteSessionEndpoint,
        mut stream: NodeStream,
        persistent: bool,
        negotiated_protocol: NegotiatedProtocol,
        remote_identity: Option<super::identity_claim::NodeIdentityClaim>,
    ) -> Self {
        if persistent {
            if let Err(error) = stream.prepare_for_session() {
                eprintln!(
                    "mesh transport: transition=session_timeouts_failed remote={} reason={error}",
                    endpoint.remote_name
                );
            }
        }
        let RemoteSessionEndpoint {
            remote_name,
            remote_creation,
            node_id,
            direction,
        } = endpoint;
        let (control_outbound, control) = crossbeam_channel::bounded(CONTROL_QUEUE_ITEMS);
        let (admission_outbound, admission) = crossbeam_channel::bounded(ADMISSION_QUEUE_ITEMS);
        let (continuity_outbound, continuity) = crossbeam_channel::bounded(CONTINUITY_QUEUE_ITEMS);
        let (application_outbound, application) =
            crossbeam_channel::bounded(APPLICATION_QUEUE_ITEMS);
        let (snapshot_outbound, snapshot) = crossbeam_channel::bounded(SNAPSHOT_QUEUE_ITEMS);
        Self {
            remote_name,
            remote_creation,
            node_id,
            direction,
            stream: parking_lot::Mutex::new(stream),
            tls_output_pending: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            connected_at: Instant::now(),
            negotiated_protocol,
            remote_identity,
            pending_spawns: std::sync::Mutex::new(FxHashMap::default()),
            global_names_received: AtomicBool::new(false),
            pending_continuity_prepares: std::sync::Mutex::new(FxHashMap::default()),
            pending_operator_queries: std::sync::Mutex::new(FxHashMap::default()),
            pending_consensus_rpcs: std::sync::Mutex::new(FxHashMap::default()),
            pending_http_routes: std::sync::Mutex::new(FxHashMap::default()),
            pending_http_reservations: std::sync::Mutex::new(FxHashMap::default()),
            accepted_http_reservations: std::sync::Mutex::new(FxHashMap::default()),
            persistent,
            control_outbound,
            admission_outbound,
            continuity_outbound,
            application_outbound,
            snapshot_outbound,
            outbound_receivers: Mutex::new(Some(OutboundReceivers {
                control,
                admission,
                continuity,
                application,
                snapshot,
            })),
            control_queued_bytes: AtomicUsize::new(0),
            admission_queued_bytes: AtomicUsize::new(0),
            continuity_queued_bytes: AtomicUsize::new(0),
            application_queued_bytes: AtomicUsize::new(0),
            snapshot_queued_bytes: AtomicUsize::new(0),
        }
    }

    pub(crate) fn send(&self, class: OutboundClass, payload: Vec<u8>) -> Result<(), String> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err("peer_session_shutdown".to_string());
        }
        if !self.persistent {
            let mut stream = self.stream.lock();
            return write_msg(&mut *stream, &payload)
                .map_err(|error| format!("peer_session_write_failed:{error}"));
        }
        if matches!(class, OutboundClass::Application)
            && !peer_circuit_allow(&self.remote_name, Instant::now())
        {
            return Err("peer_circuit_open".to_string());
        }
        let payload = encode_session_payload(class, payload, &self.negotiated_protocol)?;
        let (sender, bytes, byte_limit) = match class {
            OutboundClass::Control => (
                &self.control_outbound,
                &self.control_queued_bytes,
                CONTROL_QUEUE_BYTES,
            ),
            OutboundClass::Admission => (
                &self.admission_outbound,
                &self.admission_queued_bytes,
                ADMISSION_QUEUE_BYTES,
            ),
            OutboundClass::Continuity => (
                &self.continuity_outbound,
                &self.continuity_queued_bytes,
                CONTINUITY_QUEUE_BYTES,
            ),
            OutboundClass::Application => (
                &self.application_outbound,
                &self.application_queued_bytes,
                APPLICATION_QUEUE_BYTES,
            ),
            OutboundClass::Snapshot => (
                &self.snapshot_outbound,
                &self.snapshot_queued_bytes,
                SNAPSHOT_QUEUE_BYTES,
            ),
        };
        enqueue_outbound(sender, bytes, byte_limit, class, payload)
    }

    /// Like `send`, but waits for room in the lane instead of failing when it
    /// is full. For bulk state transfer, whose dropped frames nobody resends:
    /// initial sync pushes one frame per record into a 64-frame lane.
    pub(crate) fn send_waiting(
        &self,
        class: OutboundClass,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.send(class, payload.clone()) {
                Err(error)
                    if (error == "peer_outbound_queue_full"
                        || error == "peer_outbound_byte_limit")
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
                result => return result,
            }
        }
    }

    fn send_heartbeat(&self, payload: Vec<u8>) -> Result<(), String> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err("peer_session_shutdown".to_string());
        }
        if !matches!(
            payload.first(),
            Some(&HEARTBEAT_PING) | Some(&HEARTBEAT_PONG)
        ) {
            return Err("heartbeat_frame_invalid".to_string());
        }
        let payload =
            encode_session_payload(OutboundClass::Control, payload, &self.negotiated_protocol)?;
        // Heartbeats are liveness control, not application admission: they
        // skip the outbound lanes, so a reservation burst cannot delay them
        // into a false node failure. The frame goes into the send buffer
        // whole and out as far as the socket takes it now; the reader
        // answers pings here and must not wait on its peer, so what is left
        // goes out with the writer's next flush.
        let mut stream = self.stream.lock();
        let result = stream
            .queue_frame(&payload)
            .and_then(|()| stream.flush_queued());
        match result {
            Ok((_, flushed)) => {
                if !flushed {
                    self.tls_output_pending.store(true, Ordering::Release);
                }
                Ok(())
            }
            Err(error) => Err(format!("peer_heartbeat_failed:{error}")),
        }
    }

    /// Writes whole frames to the peer, and what heartbeats left queued. The
    /// stream goes back to the reader whenever the socket takes no more, so
    /// a peer that reads slowly slows this writer without stopping this
    /// session's reads; a peer that takes nothing for `SESSION_WRITE_STALL`
    /// ends the session.
    fn write_frames<'p>(&self, payloads: impl IntoIterator<Item = &'p [u8]>) -> io::Result<()> {
        let mut stream = self.stream.lock();
        for payload in payloads {
            stream.queue_frame(payload)?;
        }
        let mut last_progress = Instant::now();
        loop {
            let (written, flushed) = stream.flush_queued()?;
            if flushed {
                self.tls_output_pending.store(false, Ordering::Release);
                return Ok(());
            }
            parking_lot::MutexGuard::unlock_fair(stream);
            let now = Instant::now();
            if written > 0 {
                last_progress = now;
            } else {
                // The socket took nothing: give the peer a moment.
                std::thread::sleep(Duration::from_millis(1));
            }
            if self.shutdown.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "peer session shut down",
                ));
            }
            if now.duration_since(last_progress) >= SESSION_WRITE_STALL {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("peer took nothing for {SESSION_WRITE_STALL:?}"),
                ));
            }
            stream = self.stream.lock();
        }
    }

    pub(crate) fn telemetry_snapshot(
        &self,
        now: Instant,
    ) -> crate::dist::telemetry::PeerSessionTelemetrySnapshot {
        let lanes = [
            outbound_lane_snapshot(
                "control",
                self.control_outbound.len(),
                self.control_queued_bytes.load(Ordering::Relaxed),
                CONTROL_QUEUE_ITEMS,
                CONTROL_QUEUE_BYTES,
            ),
            outbound_lane_snapshot(
                "admission",
                self.admission_outbound.len(),
                self.admission_queued_bytes.load(Ordering::Relaxed),
                ADMISSION_QUEUE_ITEMS,
                ADMISSION_QUEUE_BYTES,
            ),
            outbound_lane_snapshot(
                "continuity",
                self.continuity_outbound.len(),
                self.continuity_queued_bytes.load(Ordering::Relaxed),
                CONTINUITY_QUEUE_ITEMS,
                CONTINUITY_QUEUE_BYTES,
            ),
            outbound_lane_snapshot(
                "application",
                self.application_outbound.len(),
                self.application_queued_bytes.load(Ordering::Relaxed),
                APPLICATION_QUEUE_ITEMS,
                APPLICATION_QUEUE_BYTES,
            ),
            outbound_lane_snapshot(
                "snapshot",
                self.snapshot_outbound.len(),
                self.snapshot_queued_bytes.load(Ordering::Relaxed),
                SNAPSHOT_QUEUE_ITEMS,
                SNAPSHOT_QUEUE_BYTES,
            ),
        ];
        crate::dist::telemetry::PeerSessionTelemetrySnapshot {
            peer: self.remote_name.clone(),
            healthy: !self.shutdown.load(Ordering::Acquire),
            connected_millis: now
                .saturating_duration_since(self.connected_at)
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            circuit_state: match peer_circuit_state(&self.remote_name, now) {
                CircuitState::Closed => "closed",
                CircuitState::Open => "open",
                CircuitState::HalfOpen => "half_open",
            }
            .to_string(),
            lanes: lanes.into(),
        }
    }

    fn queued_totals(&self) -> (usize, usize) {
        let items = self
            .control_outbound
            .len()
            .saturating_add(self.admission_outbound.len())
            .saturating_add(self.continuity_outbound.len())
            .saturating_add(self.application_outbound.len())
            .saturating_add(self.snapshot_outbound.len());
        let bytes = self
            .control_queued_bytes
            .load(Ordering::Relaxed)
            .saturating_add(self.admission_queued_bytes.load(Ordering::Relaxed))
            .saturating_add(self.continuity_queued_bytes.load(Ordering::Relaxed))
            .saturating_add(self.application_queued_bytes.load(Ordering::Relaxed))
            .saturating_add(self.snapshot_queued_bytes.load(Ordering::Relaxed));
        (items, bytes)
    }
}

fn outbound_lane_snapshot(
    class: &str,
    queued_items: usize,
    queued_bytes: usize,
    item_capacity: usize,
    byte_capacity: usize,
) -> crate::dist::telemetry::OutboundLaneTelemetrySnapshot {
    let item_utilization = queued_items as f64 / item_capacity.max(1) as f64;
    let byte_utilization = queued_bytes as f64 / byte_capacity.max(1) as f64;
    crate::dist::telemetry::OutboundLaneTelemetrySnapshot {
        class: class.to_string(),
        queued_items: queued_items.try_into().unwrap_or(u32::MAX),
        queued_bytes: queued_bytes.try_into().unwrap_or(u64::MAX),
        item_capacity: item_capacity.try_into().unwrap_or(u32::MAX),
        byte_capacity: byte_capacity.try_into().unwrap_or(u64::MAX),
        utilization: item_utilization.max(byte_utilization),
    }
}

pub(crate) fn local_peer_session_telemetry(
) -> Vec<crate::dist::telemetry::PeerSessionTelemetrySnapshot> {
    let Some(state) = node_state() else {
        return Vec::new();
    };
    let sessions: Vec<_> = state.sessions.read().values().cloned().collect();
    let now = Instant::now();
    let snapshots: Vec<_> = sessions
        .iter()
        .map(|session| session.telemetry_snapshot(now))
        .collect();
    refresh_peer_session_telemetry();
    snapshots
}

pub(crate) fn refresh_peer_session_telemetry() {
    let Some(state) = node_state() else {
        return;
    };
    let sessions = state.sessions.read();
    let (queued_items, queued_bytes) =
        sessions
            .values()
            .fold((0usize, 0usize), |(total_items, total_bytes), session| {
                let (items, bytes) = session.queued_totals();
                (
                    total_items.saturating_add(items),
                    total_bytes.saturating_add(bytes),
                )
            });
    let now = Instant::now();
    let circuits = peer_circuits().lock().unwrap();
    let open_circuits = sessions
        .keys()
        .filter(|peer| {
            circuits
                .get(*peer)
                .is_some_and(|circuit| circuit.state(now) == CircuitState::Open)
        })
        .count();
    crate::dist::telemetry::runtime_telemetry().set_remote_dispatch_queue(
        queued_items.try_into().unwrap_or(u32::MAX),
        queued_bytes.try_into().unwrap_or(u64::MAX),
        open_circuits.try_into().unwrap_or(u32::MAX),
    );
}

fn enqueue_outbound(
    sender: &crossbeam_channel::Sender<OutboundFrame>,
    bytes: &AtomicUsize,
    byte_limit: usize,
    class: OutboundClass,
    payload: Vec<u8>,
) -> Result<(), String> {
    if let Err(error) = reserve_queued_bytes(bytes, payload.len(), byte_limit) {
        crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_queue_rejection();
        return Err(error);
    }
    if let Err(error) = sender.try_send(OutboundFrame { payload, class }) {
        let length = match &error {
            crossbeam_channel::TrySendError::Full(frame)
            | crossbeam_channel::TrySendError::Disconnected(frame) => frame.payload.len(),
        };
        bytes.fetch_sub(length, Ordering::AcqRel);
        crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_queue_rejection();
        return Err(match error {
            crossbeam_channel::TrySendError::Full(_) => "peer_outbound_queue_full",
            crossbeam_channel::TrySendError::Disconnected(_) => "peer_outbound_queue_disconnected",
        }
        .to_string());
    }
    Ok(())
}

fn encode_session_payload(
    class: OutboundClass,
    payload: Vec<u8>,
    negotiated: &NegotiatedProtocol,
) -> Result<Vec<u8>, String> {
    if negotiated.version < PROTOCOL_V2 {
        // A protocol-one peer ends the session over a larger frame
        // (`PersistentFrameReader`), so it never goes.
        if payload.len() > MAX_DIST_MSG as usize {
            return Err("protocol_frame_bound_exceeded".to_string());
        }
        return Ok(payload);
    }
    let kind = payload
        .first()
        .copied()
        .ok_or_else(|| "protocol_empty_distribution_message".to_string())?;
    ProtocolEnvelope {
        class: match (class, kind) {
            (_, HEARTBEAT_PING | HEARTBEAT_PONG) => MessageClass::Heartbeat,
            (OutboundClass::Control | OutboundClass::Admission | OutboundClass::Continuity, _) => {
                MessageClass::Control
            }
            (OutboundClass::Application, _) => MessageClass::Application,
            (OutboundClass::Snapshot, _) => MessageClass::Snapshot,
        },
        kind: u16::from(kind),
        correlation_id: correlation_id_from_payload(kind, &payload),
        chunk_sequence: 0,
        final_chunk: true,
        payload,
    }
    .encode(negotiated.max_frame_bytes)
}

fn decode_session_payload(
    frame: Vec<u8>,
    negotiated: &NegotiatedProtocol,
) -> Result<Vec<u8>, String> {
    if negotiated.version < PROTOCOL_V2 {
        return Ok(frame);
    }
    let envelope = ProtocolEnvelope::decode(&frame, negotiated.max_frame_bytes)?;
    let kind = envelope
        .payload
        .first()
        .copied()
        .ok_or_else(|| "protocol_empty_distribution_message".to_string())?;
    if envelope.kind != u16::from(kind) {
        return Err("protocol_envelope_kind_mismatch".to_string());
    }
    if !envelope.final_chunk || envelope.chunk_sequence != 0 {
        return Err("protocol_unexpected_unreassembled_chunk".to_string());
    }
    Ok(envelope.payload)
}

fn correlation_id_from_payload(kind: u8, payload: &[u8]) -> u64 {
    if matches!(
        kind,
        DIST_HTTP_ROUTE_V2_QUERY
            | DIST_HTTP_ROUTE_V2_REPLY
            | DIST_HTTP_RESERVE
            | DIST_HTTP_RESERVE_REPLY
            | DIST_OPERATOR_QUERY
            | DIST_OPERATOR_REPLY
            | DIST_CONSENSUS_RPC
            | DIST_CONSENSUS_RPC_REPLY
    ) && payload.len() >= 9
    {
        u64::from_le_bytes(payload[1..9].try_into().unwrap())
    } else {
        0
    }
}

fn reserve_queued_bytes(counter: &AtomicUsize, amount: usize, limit: usize) -> Result<(), String> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(amount).filter(|next| *next <= limit)
        })
        .map(|_| ())
        .map_err(|_| "peer_outbound_byte_limit".to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionDirection {
    Incoming,
    Outgoing,
}

impl SessionDirection {
    fn from_stream(stream: &NodeStream) -> Self {
        match stream {
            NodeStream::ServerTls(_) => Self::Incoming,
            NodeStream::ClientTls(_) => Self::Outgoing,
        }
    }
}

// ---------------------------------------------------------------------------
// NodeStream -- TLS stream abstraction for node connections
// ---------------------------------------------------------------------------

/// Stream abstraction for inter-node TLS connections.
///
/// Server variant is used when we accepted the connection; Client variant
/// when we initiated it. Both implement Read + Write by delegating to
/// the inner `StreamOwned`, except that a read never writes (see
/// `read_without_flushing`).
pub(crate) enum NodeStream {
    ServerTls(StreamOwned<rustls::ServerConnection, TcpStream>),
    ClientTls(StreamOwned<rustls::ClientConnection, TcpStream>),
}

/// How long a persistent session's socket read waits before handing the
/// stream back: its reader and writers share one TLS connection, and none
/// may hold it while waiting on the peer. (Writes do not wait at all.)
const SESSION_IO_POLL: Duration = Duration::from_millis(25);

/// How long a peer may take none of what a session writes before the
/// session is dead: its reader is stuck, not just slow. (Shorter in tests,
/// which wait it out.)
const SESSION_WRITE_STALL: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(15)
};

impl Read for NodeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            NodeStream::ServerTls(s) => read_without_flushing(&mut *s.conn, &mut s.sock, buf),
            NodeStream::ClientTls(s) => read_without_flushing(&mut *s.conn, &mut s.sock, buf),
        }
    }
}

/// `StreamOwned::read` without its first step, which writes out what TLS
/// holds to send: that write waits while the peer takes nothing, and a
/// reader waiting on its own session's writes stops reading what the peer
/// sends. Two nodes streaming state to each other (a new node's initial
/// sync) each stopped reading, for good.
fn read_without_flushing<S>(
    conn: &mut rustls::ConnectionCommon<S>,
    sock: &mut TcpStream,
    buf: &mut [u8],
) -> io::Result<usize> {
    loop {
        match conn.reader().read(buf) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            result => return result,
        }
        let received = conn.read_tls(sock)?;
        conn.process_new_packets()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if received == 0 {
            // The end of the stream: what is left, or how it ended.
            return conn.reader().read(buf);
        }
    }
}

/// Encrypts `[u32 length][payload]` into the connection's send buffer,
/// whole: a frame is never split between two holders of the stream.
fn queue_frame_in<S>(conn: &mut rustls::ConnectionCommon<S>, payload: &[u8]) -> io::Result<()> {
    let mut writer = conn.writer();
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(payload)
}

/// Writes what the connection holds to send until the socket takes no
/// more: the bytes written, and whether none are left. The socket does not
/// wait meanwhile (a send that times out can leave a Windows socket
/// unusable); reads, under the same lock, never overlap this.
fn flush_queued_in<S>(
    conn: &mut rustls::ConnectionCommon<S>,
    sock: &mut TcpStream,
) -> io::Result<(usize, bool)> {
    if !conn.wants_write() {
        return Ok((0, true));
    }
    sock.set_nonblocking(true)?;
    let mut written = 0;
    let result = loop {
        if !conn.wants_write() {
            break Ok((written, true));
        }
        match conn.write_tls(sock) {
            Ok(0) => break Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break Ok((written, false)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
        }
    };
    sock.set_nonblocking(false)?;
    result
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

    /// Readies an authenticated stream for a persistent session: a socket
    /// read gives the stream back after `SESSION_IO_POLL`, and the send
    /// buffer takes any frame whole.
    fn prepare_for_session(&mut self) -> io::Result<()> {
        match self {
            NodeStream::ServerTls(s) => s.conn.set_buffer_limit(None),
            NodeStream::ClientTls(s) => s.conn.set_buffer_limit(None),
        }
        self.set_read_timeout(Some(SESSION_IO_POLL))
    }

    fn queue_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        match self {
            NodeStream::ServerTls(s) => queue_frame_in(&mut *s.conn, payload),
            NodeStream::ClientTls(s) => queue_frame_in(&mut *s.conn, payload),
        }
    }

    fn flush_queued(&mut self) -> io::Result<(usize, bool)> {
        match self {
            NodeStream::ServerTls(s) => flush_queued_in(&mut *s.conn, &mut s.sock),
            NodeStream::ClientTls(s) => flush_queued_in(&mut *s.conn, &mut s.sock),
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
/// Distribution message tag: peer list exchange for automatic mesh formation.
/// Wire format: [tag][u16 count][u16 name_len, name bytes, ...]
pub(crate) const DIST_PEER_LIST: u8 = 0x12;
/// Distribution message tag: remote process monitor setup.
/// Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
pub(crate) const DIST_MONITOR: u8 = 0x16;
/// Distribution message tag: remote process demonitor.
/// Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
pub(crate) const DIST_DEMONITOR: u8 = 0x17;
/// Distribution message tag: remote process monitor exit notification.
/// Wire format: [tag][u64 monitored_pid][u64 monitoring_pid][u64 ref][reason_bytes]
pub(crate) const DIST_MONITOR_EXIT: u8 = 0x18;

/// Distribution message tag: bidirectional link request.
/// Wire format: [tag][u64 from_pid][u64 to_pid]
pub(crate) const DIST_LINK: u8 = 0x13;
/// Distribution message tag: exit signal propagation.
/// Wire format: [tag][u64 from_pid][u64 to_pid][reason_bytes]
pub(crate) const DIST_EXIT: u8 = 0x15;
/// Distribution message tag: remote spawn request (Phase 67).
/// Wire format:
/// [tag][u64 request_id][u64 requester_pid][u8 link_flag]
/// [u16 fn_name_len][fn_name bytes][u16 arg_count][arg_tags bytes][encoded args]
pub(crate) const DIST_SPAWN: u8 = 0x19;
/// Distribution message tag: remote spawn reply (Phase 67).
/// Wire format: [tag][u64 request_id][u8 status][u64 spawned_pid]
pub(crate) const DIST_SPAWN_REPLY: u8 = 0x1A;

/// Registry-only marker for a parameter that cannot be supplied through the
/// remote spawn wire format (or an entry that does not follow the actor args
/// ABI at all). Never valid on the wire.
const REMOTE_SPAWN_ARG_UNSUPPORTED: u8 = 0;
const REMOTE_SPAWN_ARG_INT: u8 = 1;
const REMOTE_SPAWN_ARG_FLOAT: u8 = 2;
const REMOTE_SPAWN_ARG_BOOL: u8 = 3;
const REMOTE_SPAWN_ARG_STRING: u8 = 4;
const REMOTE_SPAWN_ARG_PID: u8 = 5;
const REMOTE_SPAWN_ARG_UNIT: u8 = 6;

/// Wire tag for global registry: register a name across the cluster.
/// Format: [tag 0x1B][u16 name_len][name bytes][u64 pid][u16 node_name_len][node_name bytes]
pub(crate) const DIST_GLOBAL_REGISTER: u8 = 0x1B;

/// Wire tag for global registry: unregister a name across the cluster.
/// Format: [tag 0x1C][u16 name_len][name bytes]
pub(crate) const DIST_GLOBAL_UNREGISTER: u8 = 0x1C;

/// Wire tag for global registry: bulk sync snapshot on node connect.
/// Format: [tag 0x1D][u32 count][(u16 name_len, name, u64 pid, u16 node_len, node_name)*]
pub(crate) const DIST_GLOBAL_SYNC: u8 = 0x1D;

/// Wire tag for distributed room broadcast (Phase 69).
/// Format: [tag 0x1E][u16 room_name_len][room_name bytes][u32 msg_len][msg bytes]
pub(crate) const DIST_ROOM_BROADCAST: u8 = 0x1E;

/// Wire tag for continuity registry: upsert a single request record.
/// Format: [tag 0x1F][u64 next_attempt_token][u32 record_len][record bytes]
pub(crate) const DIST_CONTINUITY_UPSERT: u8 = 0x1F;

/// Wire tag for continuity registry: sync a full snapshot on node connect.
/// Format: [tag 0x20][u64 next_attempt_token][u32 count][u32 record_len][record bytes]...
pub(crate) const DIST_CONTINUITY_SYNC: u8 = 0x20;

/// Wire tag for continuity registry: targeted replica prepare request.
/// Format: [tag 0x21][u64 request_id][u32 record_len][record bytes]
pub(crate) const DIST_CONTINUITY_PREPARE: u8 = 0x21;

/// Wire tag for continuity registry: targeted replica prepare response.
/// Format: [tag 0x22][u64 request_id][u8 status][u16 reason_len][reason bytes]
pub(crate) const DIST_CONTINUITY_PREPARE_ACK: u8 = 0x22;

/// Wire tag for runtime-owned operator query requests.
/// Format: [tag 0x23][u64 request_id][u8 kind][u32 payload_len][payload bytes]
pub(crate) const DIST_OPERATOR_QUERY: u8 = 0x23;

/// Wire tag for runtime-owned operator query replies.
/// Format: [tag 0x24][u64 request_id][u8 status][u32 payload_len][payload bytes]
pub(crate) const DIST_OPERATOR_REPLY: u8 = 0x24;

/// Wire tag for compact protocol-two node load reports.
/// Format: [tag 0x27][bounded NodeLoadReport payload]
pub(crate) const DIST_LOAD_REPORT: u8 = 0x27;

/// A clustered HTTP request for its owner, over the peer session.
/// Format: [tag 0x28][u64 correlation_id][u16 runtime_name_len][runtime_name]
///         [u16 request_key_len][request_key][u16 attempt_id_len][attempt_id]
///         [u32 payload_len][encoded MeshHttpRequest payload]
/// (Tags 0x25 and 0x26 were a transient connection's query and reply.)
pub(crate) const DIST_HTTP_ROUTE_V2_QUERY: u8 = 0x28;

/// The owner's completion of a clustered HTTP request.
/// Format: [tag 0x29][u64 correlation_id][u8 status][u32 payload_len]
///         [response payload, or UTF-8 reason]
pub(crate) const DIST_HTTP_ROUTE_V2_REPLY: u8 = 0x29;

/// Replicates a retained successful response to continuity peers.
/// Format: [tag 0x2A][u32 operation_key_len][operation_key][u32 payload_len][payload]
pub(crate) const DIST_CONTINUITY_RESPONSE: u8 = 0x2A;

/// Checksummed durable SQLite continuity snapshot chunk.
pub(crate) const DIST_CONTINUITY_STORE_SNAPSHOT: u8 = 0x2B;
/// Resume acknowledgement for a durable continuity snapshot.
pub(crate) const DIST_CONTINUITY_STORE_SNAPSHOT_ACK: u8 = 0x2C;
/// Incremental durable continuity log entry after a snapshot high-water mark.
pub(crate) const DIST_CONTINUITY_STORE_LOG_ENTRY: u8 = 0x2D;
/// Two-phase remote owner admission request.
pub(crate) const DIST_HTTP_RESERVE: u8 = 0x2E;
/// Accepted or rejected response to a remote owner admission request.
pub(crate) const DIST_HTTP_RESERVE_REPLY: u8 = 0x2F;

/// Embedded OpenRaft RPC over the authenticated protocol-two control channel.
/// Format: [tag 0x30][u64 correlation_id][u32 JSON length][JSON request]
pub(crate) const DIST_CONSENSUS_RPC: u8 = 0x30;
/// Embedded OpenRaft RPC reply.
/// Format: [tag 0x31][u64 correlation_id][u32 JSON length][JSON reply]
pub(crate) const DIST_CONSENSUS_RPC_REPLY: u8 = 0x31;

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
    let state = started_node();

    let sessions = state.sessions.read();
    let peers: Vec<&String> = sessions
        .keys()
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

    let _ = session.send(OutboundClass::Control, payload);
}

/// Handle an incoming DIST_PEER_LIST -- connect to unknown peers on a separate thread.
///
/// Parses the peer list, filters out self and already-connected nodes,
/// then spawns a thread to connect to the remaining peers. The thread spawn
/// avoids deadlock (see Pitfall 7 in RESEARCH.md).
fn handle_peer_list(data: &[u8]) {
    if data.len() < 2 {
        return;
    }
    let count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
    let mut pos = 2;
    let mut to_connect = Vec::new();
    let state = started_node();

    for _ in 0..count {
        if pos + 2 > data.len() {
            break;
        }
        let name_len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;
        if pos + name_len > data.len() {
            break;
        }
        if let Ok(peer_name) = std::str::from_utf8(&data[pos..pos + name_len]) {
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
                mesh_node_connect(bytes.as_ptr(), bytes.len() as u64);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// DIST_LINK / DIST_EXIT send helpers
// ---------------------------------------------------------------------------

/// A pid a peer sent for one of this node's processes. The peer qualifies it
/// with the node id its own table gives this node; here it is local.
fn own_pid(raw: u64) -> crate::actor::process::ProcessId {
    let pid = crate::actor::process::ProcessId(raw);
    crate::actor::process::ProcessId(pid.local_id())
}

/// A `DIST_SEND` frame for local message bytes `data`, which reference what
/// `captured` holds: `[tag][u64 target][u64 data len][data][capture]`. Each
/// pid in it goes as its local id, and the capture names its node.
pub(crate) fn encode_dist_send(
    target: crate::actor::process::ProcessId,
    mut data: Vec<u8>,
    mut captured: crate::actor::msg_shape::Captured,
) -> Vec<u8> {
    let mut nodes = Vec::new();
    captured.map_pids(&mut data, |pid| {
        let pid = crate::actor::process::ProcessId(pid);
        nodes.push(pid_node_name(pid).unwrap_or_default());
        pid.local_id()
    });
    let mut payload = vec![DIST_SEND];
    payload.extend_from_slice(&target.as_u64().to_le_bytes());
    payload.extend_from_slice(&(data.len() as u64).to_le_bytes());
    payload.extend_from_slice(&data);
    captured.encode(&mut payload, &nodes);
    payload
}

/// The name of the node `pid` is on: this one, or a connected one. `None`
/// for no process (0) and for a node this one no longer knows.
fn pid_node_name(pid: crate::actor::process::ProcessId) -> Option<String> {
    let state = node_state()?;
    match pid.as_u64() {
        0 => None,
        _ if pid.is_local() => Some(state.name.clone()),
        _ => state.node_id_map.read().get(&pid.node_id()).cloned(),
    }
}

/// A `DIST_SEND` frame `encode_dist_send` made: the local process it is for,
/// the message's bytes and what they reference, its pids as this node
/// addresses them. A pid on a node this one is not connected to becomes 0.
fn decode_dist_send(
    msg: &[u8],
) -> Option<(
    crate::actor::process::ProcessId,
    Vec<u8>,
    crate::actor::msg_shape::Captured,
)> {
    let target = own_pid(u64::from_le_bytes(msg.get(1..9)?.try_into().ok()?));
    let len = usize::try_from(u64::from_le_bytes(msg.get(9..17)?.try_into().ok()?)).ok()?;
    let end = 17usize.checked_add(len)?;
    let mut data = msg.get(17..end)?.to_vec();
    let (mut captured, nodes) = crate::actor::msg_shape::Captured::decode(&msg[end..], len)?;
    let mut nodes = nodes.into_iter();
    captured.map_pids(&mut data, |local| {
        pid_on_node(&nodes.next().unwrap_or_default(), local)
    });
    Some((target, data, captured))
}

/// The pid, as this node addresses it, of process `local` on the node named
/// `node`: 0 for no node, and for one this node is not connected to.
fn pid_on_node(node: &str, local: u64) -> u64 {
    use crate::actor::process::ProcessId;
    let local = ProcessId(local).local_id();
    let state = started_node();
    match node {
        "" => 0,
        _ if node == state.name => local,
        _ => state.sessions.read().get(node).map_or(0, |session| {
            ProcessId::from_remote(session.node_id, session.remote_creation, local).as_u64()
        }),
    }
}

/// The session to the node `pid` lives on, if it is connected.
pub(crate) fn session_for_pid(pid: crate::actor::ProcessId) -> Option<Arc<NodeSession>> {
    let state = node_state()?;
    let name = state.node_id_map.read().get(&pid.node_id())?.clone();
    state.sessions.read().get(&name).cloned()
}

/// Send DIST_LINK to register a bidirectional link on the remote node.
/// Wire format: [DIST_LINK][u64 from_pid][u64 to_pid]
/// Silently drops if session unavailable (node already disconnected).
pub(crate) fn send_dist_link(from_pid: crate::actor::ProcessId, to_pid: crate::actor::ProcessId) {
    let Some(session) = session_for_pid(to_pid) else {
        return;
    };
    let mut payload = Vec::with_capacity(1 + 8 + 8);
    payload.push(DIST_LINK);
    payload.extend_from_slice(&from_pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&to_pid.as_u64().to_le_bytes());
    let _ = session.send(OutboundClass::Application, payload);
}

/// Send DIST_EXIT to propagate an exit signal to a remote linked process.
/// Wire format: [DIST_EXIT][u64 from_pid][u64 to_pid][reason_bytes]
/// Silently drops if session unavailable (node already disconnected).
pub(crate) fn send_dist_exit(
    from_pid: crate::actor::ProcessId,
    to_pid: crate::actor::ProcessId,
    reason: &crate::actor::ExitReason,
) {
    let Some(session) = session_for_pid(to_pid) else {
        return;
    };
    let mut payload = Vec::with_capacity(1 + 8 + 8 + 16);
    payload.push(DIST_EXIT);
    payload.extend_from_slice(&from_pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&to_pid.as_u64().to_le_bytes());
    crate::actor::link::encode_reason(&mut payload, reason);
    let _ = session.send(OutboundClass::Application, payload);
}

/// Send DIST_MONITOR_EXIT to notify a remote monitoring process about a local process exit.
/// Uses PID-based session lookup (unlike send_dist_monitor_exit which takes a session directly).
pub(crate) fn send_dist_monitor_exit_by_pid(
    monitored_pid: crate::actor::ProcessId,
    monitoring_pid: crate::actor::ProcessId,
    monitor_ref: u64,
    reason: &crate::actor::ExitReason,
) {
    let Some(session) = session_for_pid(monitoring_pid) else {
        return;
    };
    send_dist_monitor_exit(&session, monitored_pid, monitoring_pid, monitor_ref, reason);
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

    let session_for_writer = Arc::clone(session);
    let writer_name = format!("mesh-node-writer-{}", session.remote_name);
    std::thread::Builder::new()
        .name(writer_name)
        .spawn(move || writer_loop_session(session_for_writer))
        .expect("failed to spawn node writer thread");

    // Reader thread
    let reader_name = format!("mesh-node-reader-{}", session.remote_name);
    std::thread::Builder::new()
        .name(reader_name)
        .spawn(move || {
            reader_loop_session(session_for_reader, hs_for_reader);
        })
        .expect("failed to spawn node reader thread");

    // Heartbeat thread
    let hb_name = format!("mesh-node-heartbeat-{}", remote_name);
    let remote_name_hb = session.remote_name.clone();
    std::thread::Builder::new()
        .name(hb_name)
        .spawn(move || {
            heartbeat_loop_session(session_for_heartbeat, hs_for_heartbeat, remote_name_hb);
        })
        .expect("failed to spawn node heartbeat thread");
}

fn note_outbound_class(class: OutboundClass, consecutive_control_frames: &mut usize) {
    if matches!(class, OutboundClass::Control) {
        *consecutive_control_frames = consecutive_control_frames.saturating_add(1);
    } else {
        *consecutive_control_frames = 0;
    }
}

fn try_next_outbound_frame(
    receivers: &OutboundReceivers,
    consecutive_control_frames: &mut usize,
) -> Option<OutboundFrame> {
    // Control traffic gets bounded priority, not an unbounded drain. A
    // reservation followed by an application frame must make progress even
    // while a synchronized burst keeps the control lane non-empty.
    let frame = if *consecutive_control_frames >= MAX_CONSECUTIVE_CONTROL_FRAMES {
        crossbeam_channel::select! {
            recv(receivers.admission) -> frame => frame.ok(),
            recv(receivers.application) -> frame => frame.ok(),
            recv(receivers.continuity) -> frame => frame.ok(),
            recv(receivers.snapshot) -> frame => frame.ok(),
            default => receivers.control.try_recv().ok(),
        }
    } else {
        receivers.control.try_recv().ok()
    }
    .or_else(|| {
        crossbeam_channel::select! {
            recv(receivers.control) -> frame => frame.ok(),
            recv(receivers.admission) -> frame => frame.ok(),
            recv(receivers.application) -> frame => frame.ok(),
            recv(receivers.continuity) -> frame => frame.ok(),
            recv(receivers.snapshot) -> frame => frame.ok(),
            default => None,
        }
    });
    if let Some(frame) = &frame {
        note_outbound_class(frame.class, consecutive_control_frames);
    }
    frame
}

fn wait_for_outbound_frame(
    receivers: &OutboundReceivers,
    consecutive_control_frames: &mut usize,
) -> Option<OutboundFrame> {
    try_next_outbound_frame(receivers, consecutive_control_frames).or_else(|| {
        let frame = crossbeam_channel::select! {
            recv(receivers.control) -> frame => frame.ok(),
            recv(receivers.admission) -> frame => frame.ok(),
            recv(receivers.application) -> frame => frame.ok(),
            recv(receivers.continuity) -> frame => frame.ok(),
            recv(receivers.snapshot) -> frame => frame.ok(),
            default(Duration::from_millis(25)) => None,
        };
        if let Some(frame) = &frame {
            note_outbound_class(frame.class, consecutive_control_frames);
        }
        frame
    })
}

fn release_outbound_frame_bytes(session: &NodeSession, frame: &OutboundFrame) {
    let length = frame.payload.len();
    match frame.class {
        OutboundClass::Control => {
            session
                .control_queued_bytes
                .fetch_sub(length, Ordering::AcqRel);
        }
        OutboundClass::Admission => {
            session
                .admission_queued_bytes
                .fetch_sub(length, Ordering::AcqRel);
        }
        OutboundClass::Continuity => {
            session
                .continuity_queued_bytes
                .fetch_sub(length, Ordering::AcqRel);
        }
        OutboundClass::Application => {
            session
                .application_queued_bytes
                .fetch_sub(length, Ordering::AcqRel);
        }
        OutboundClass::Snapshot => {
            session
                .snapshot_queued_bytes
                .fetch_sub(length, Ordering::AcqRel);
        }
    }
}

fn writer_loop_session(session: Arc<NodeSession>) {
    // `spawn_session_threads` starts one writer per session.
    let receivers = session
        .outbound_receivers
        .lock()
        .unwrap()
        .take()
        .expect("a session's one writer takes its lanes");
    let mut consecutive_control_frames = 0usize;
    while !session.shutdown.load(Ordering::Acquire) {
        let result = match wait_for_outbound_frame(&receivers, &mut consecutive_control_frames) {
            // Nothing to send: flush what a heartbeat left, if anything.
            None if session.tls_output_pending.load(Ordering::Acquire) => session.write_frames([]),
            None => continue,
            Some(first) => {
                let mut batch = Vec::with_capacity(MAX_OUTBOUND_WRITE_BATCH);
                batch.push(first);
                while batch.len() < MAX_OUTBOUND_WRITE_BATCH {
                    let Some(frame) =
                        try_next_outbound_frame(&receivers, &mut consecutive_control_frames)
                    else {
                        break;
                    };
                    batch.push(frame);
                }
                // A rustls StreamOwned cannot be split into independent
                // reader/writer halves. Batching amortizes contention with the
                // bounded reader poll instead of reacquiring this lock for
                // every small protocol frame.
                let written_application = batch
                    .iter()
                    .any(|frame| matches!(frame.class, OutboundClass::Application));
                let result =
                    session.write_frames(batch.iter().map(|frame| frame.payload.as_slice()));
                for frame in &batch {
                    release_outbound_frame_bytes(&session, frame);
                }
                if result.is_ok() && written_application {
                    record_peer_transport_success(&session.remote_name);
                }
                result
            }
        };
        if let Err(error) = result {
            record_peer_transport_failure(&session.remote_name, Instant::now());
            eprintln!(
                "mesh transport: transition=writer_failed remote={} reason={}",
                session.remote_name, error
            );
            session.shutdown.store(true, Ordering::Release);
        }
    }
}

// ---------------------------------------------------------------------------
// reader_loop_session -- receives messages on a dedicated OS thread
// ---------------------------------------------------------------------------

/// Reader thread for a node session: reads the peer's frames off the TLS
/// stream and hands each to `handle_session_message`, until the session
/// shuts down, the peer goes, or it breaks the protocol.
///
/// Uses a 25ms read timeout to allow periodic shutdown checks and writer
/// turns without busy-waiting.
fn reader_loop_session(session: Arc<NodeSession>, heartbeat_state: Arc<Mutex<HeartbeatState>>) {
    // The incremental frame reader preserves partial prefixes/bodies across
    // socket timeouts (`SESSION_IO_POLL`, set when the session was made),
    // allowing the shared rustls stream lock to be released frequently for
    // control-plane writes without desynchronizing framing.
    let mut frame_reader = PersistentFrameReader::default();

    loop {
        if session.shutdown.load(Ordering::SeqCst) {
            break;
        }

        let result = {
            let mut s = session.stream.lock();
            let maximum = if session.negotiated_protocol.version >= PROTOCOL_V2 {
                session.negotiated_protocol.max_frame_bytes
            } else {
                MAX_DIST_MSG
            };
            let result = frame_reader.read_next(&mut *s, maximum);
            // This loop takes the lock straight back, so a plain unlock lets
            // it win every time and a writer waits for inbound traffic
            // instead of at most one read timeout. Hand the stream over.
            parking_lot::MutexGuard::unlock_fair(s);
            result
        };

        match result {
            Ok(Some(frame)) => match decode_session_payload(frame, &session.negotiated_protocol) {
                Ok(msg) => handle_session_message(&session, &heartbeat_state, msg),
                Err(error) => {
                    eprintln!(
                        "mesh transport: transition=protocol_violation remote={} reason={}",
                        session.remote_name, error
                    );
                    session.shutdown.store(true, Ordering::Release);
                    break;
                }
            },
            Ok(None) => continue,
            Err(error) => {
                // A peer that exits closes its connection without TLS's
                // close_notify; frames are length-prefixed, so that says
                // nothing more than that it is gone.
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    eprintln!(
                        "mesh transport: transition=peer_closed remote={}",
                        session.remote_name
                    );
                } else {
                    eprintln!(
                        "mesh transport: transition=reader_failed remote={} kind={:?} reason={}",
                        session.remote_name,
                        error.kind(),
                        error
                    );
                }
                session.shutdown.store(true, Ordering::SeqCst);
                break;
            }
        }
    }
}

/// Acts on one message the peer of `session` sent.
fn handle_session_message(
    session: &Arc<NodeSession>,
    heartbeat_state: &Mutex<HeartbeatState>,
    msg: Vec<u8>,
) {
    let Some(&tag) = msg.first() else {
        return;
    };
    match tag {
        HEARTBEAT_PING => {
            if msg.len() >= 9 {
                let mut pong = Vec::with_capacity(9);
                pong.push(HEARTBEAT_PONG);
                pong.extend_from_slice(&msg[1..9]);
                if session.send_heartbeat(pong).is_err() {
                    session.shutdown.store(true, Ordering::Release);
                }
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
        DIST_SEND => match decode_dist_send(&msg) {
            Some((target, data, captured)) => {
                crate::actor::deliver_remote(target, data, captured);
            }
            None => eprintln!(
                "mesh transport: transition=message_malformed remote={}",
                session.remote_name
            ),
        },
        DIST_PEER_LIST => {
            handle_peer_list(&msg[1..]);
        }
        DIST_MONITOR => {
            // Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
            if msg.len() >= 25 {
                use crate::actor::process::{ExitReason, ProcessState};
                let from_pid = session.peer_pid(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                let to_pid = own_pid(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                let monitor_ref = u64::from_le_bytes(msg[17..25].try_into().unwrap());

                let sched = crate::actor::global_scheduler();
                match sched.get_process(to_pid) {
                    Some(target_arc) => {
                        let mut target_proc = target_arc.lock();
                        if matches!(target_proc.state, ProcessState::Exited(_)) {
                            // Target already dead -- send DIST_MONITOR_EXIT back with noproc.
                            drop(target_proc);
                            let noproc = ExitReason::Error("noproc".to_string());
                            send_dist_monitor_exit(session, to_pid, from_pid, monitor_ref, &noproc);
                        } else {
                            // Register monitor on local target.
                            target_proc.monitored_by.insert(monitor_ref, from_pid);
                        }
                    }
                    None => {
                        // Target does not exist -- send DIST_MONITOR_EXIT back.
                        let noproc = ExitReason::Error("noproc".to_string());
                        send_dist_monitor_exit(session, to_pid, from_pid, monitor_ref, &noproc);
                    }
                }
            }
        }
        DIST_DEMONITOR => {
            // Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
            if msg.len() >= 25 {
                let to_pid = own_pid(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                let monitor_ref = u64::from_le_bytes(msg[17..25].try_into().unwrap());

                let sched = crate::actor::global_scheduler();
                if let Some(target_arc) = sched.get_process(to_pid) {
                    target_arc.lock().monitored_by.remove(&monitor_ref);
                }
            }
        }
        DIST_MONITOR_EXIT => {
            // [tag][u64 monitored_pid][u64 monitoring_pid][u64 ref][reason]
            if msg.len() >= 25 {
                let monitoring_pid = own_pid(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                let monitor_ref = u64::from_le_bytes(msg[17..25].try_into().unwrap());
                let sched = crate::actor::global_scheduler();
                if let Some(mon_arc) = sched.get_process(monitoring_pid) {
                    let mut mon_proc = mon_arc.lock();
                    if mon_proc.fire_monitor(monitor_ref) {
                        sched.wake_if_waiting(monitoring_pid, mon_proc);
                    }
                }
            }
        }
        DIST_LINK => {
            // Wire format: [tag][u64 from_pid][u64 to_pid]
            if msg.len() >= 17 {
                let from_pid = session.peer_pid(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                let to_pid = own_pid(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                // Add from_pid to the local process's links set
                let sched = crate::actor::global_scheduler();
                if let Some(proc_arc) = sched.get_process(to_pid) {
                    proc_arc.lock().links.insert(from_pid);
                }
            }
        }
        DIST_EXIT => {
            // Wire format: [tag][u64 from_pid][u64 to_pid][reason_bytes]
            if msg.len() >= 17 {
                use crate::actor::heap::MessageBuffer;
                use crate::actor::link;
                use crate::actor::process::{ExitReason, Message, ProcessState};

                let from_pid = session.peer_pid(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                let to_pid = own_pid(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                let reason_bytes = &msg[17..];
                if let Some((reason, _)) = link::decode_reason(reason_bytes) {
                    let sched = crate::actor::global_scheduler();
                    if let Some(proc_arc) = sched.get_process(to_pid) {
                        let mut proc = proc_arc.lock();
                        if matches!(proc.state, ProcessState::Exited(_)) {
                            return; // Already dead, skip
                        }
                        proc.links.remove(&from_pid);
                        // As `link::propagate_exit`: only a process
                        // that traps exits gets the signal as a
                        // message; another ignores a normal exit.
                        let is_non_crashing =
                            matches!(reason, ExitReason::Normal | ExitReason::Shutdown);
                        if is_non_crashing && !proc.trap_exit {
                            return;
                        }
                        if proc.trap_exit {
                            let signal_data = link::encode_exit_signal(from_pid, &reason);
                            let buffer = MessageBuffer::new(signal_data, link::EXIT_SIGNAL_TAG);
                            proc.mailbox.push(Message { buffer });
                            sched.wake_if_waiting(to_pid, proc);
                        } else {
                            proc.mark_exited(ExitReason::Linked(from_pid, Box::new(reason)));
                        }
                    }
                }
            }
        }
        DIST_SPAWN => {
            // Wire format: [tag][u64 req_id][u64 requester_pid][u8 link_flag]
            //              [u16 fn_name_len][fn_name bytes][u16 arg_count][arg_tags][encoded args]
            if msg.len() >= 20 {
                use crate::actor::process::ProcessId;

                let req_id = u64::from_le_bytes(msg[1..9].try_into().unwrap());
                let requester_pid = ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                let link_flag = msg[17];
                let fn_name_len = u16::from_le_bytes(msg[18..20].try_into().unwrap()) as usize;
                // A name cut short, or not UTF-8, names no function: the
                // peer is told so rather than left waiting.
                let fn_name = msg
                    .get(20..20 + fn_name_len)
                    .and_then(|name| std::str::from_utf8(name).ok())
                    .unwrap_or("");
                let encoded_args = msg.get(20 + fn_name_len..).unwrap_or(&[]);

                match prepare_remote_spawn(fn_name, encoded_args) {
                    Ok((fn_ptr, decoded_args)) => {
                        let args_ptr = allocate_remote_spawn_args(&decoded_args);
                        let args_size = (decoded_args.len() * std::mem::size_of::<u64>()) as u64;

                        // Spawn the actor locally.
                        let spawned_pid = crate::actor::mesh_actor_spawn(
                            fn_ptr, args_ptr, args_size, 1, // normal priority
                        );
                        let spawned = ProcessId(spawned_pid);

                        // If spawn_link, establish bidirectional link.
                        if link_flag == 1 {
                            let sched = crate::actor::global_scheduler();
                            // Add requester_pid to the new process's links set.
                            // The requester_pid as received over the wire has node_id=0
                            // (it's the caller's local PID). We need to construct a
                            // remote-qualified PID using this session's node_id and creation.
                            let remote_requester = session.peer_pid(requester_pid.as_u64());
                            if let Some(proc_arc) = sched.get_process(spawned) {
                                proc_arc.lock().links.insert(remote_requester);
                            }
                            // Send DIST_LINK back so the requester's node records
                            // the reverse link. from=spawned (local), to=requester (remote).
                            // We send the local spawned PID as-is; the remote side will
                            // use its own session info to qualify it.
                            send_dist_link_via_session(session, spawned, requester_pid);
                        }

                        // Reply with the spawned process's local_id.
                        send_spawn_reply(session, req_id, 0, spawned.local_id());
                    }
                    Err(reason) => {
                        eprintln!(
                            "mesh node spawn rejected from {} for fn {}: {}",
                            session.remote_name, fn_name, reason
                        );
                        send_spawn_reply(session, req_id, 1, 0);
                    }
                }
            }
        }
        DIST_SPAWN_REPLY => {
            // Wire format: [tag][u64 req_id][u8 status][u64 spawned_local_id]
            if msg.len() >= 18 {
                let req_id = u64::from_le_bytes(msg[1..9].try_into().unwrap());
                let spawned_local_id = u64::from_le_bytes(msg[10..18].try_into().unwrap());
                let reply = session.pending_spawns.lock().unwrap().remove(&req_id);
                if let Some(reply) = reply {
                    let _ = reply.send(match msg[9] {
                        0 => Ok(spawned_local_id),
                        status => Err(format!("remote_reply_status={status}")),
                    });
                }
            }
        }
        DIST_GLOBAL_REGISTER => {
            if let Some((name, pid, node_name)) = crate::dist::global::decode_entry(&msg, &mut 1) {
                // A name already taken stays with its holder.
                let _ = crate::dist::global::global_name_registry().register(
                    name,
                    session.peer_pid(pid),
                    node_name,
                );
            }
        }
        DIST_GLOBAL_UNREGISTER => {
            if let Some(name) = crate::dist::global::decode_str(&msg, &mut 1) {
                crate::dist::global::global_name_registry().unregister(&name);
            }
        }
        DIST_GLOBAL_SYNC => {
            let entries = crate::dist::global::decode_sync(&msg)
                .into_iter()
                .map(|(name, pid, node_name)| (name, session.peer_pid(pid), node_name))
                .collect();
            crate::dist::global::global_name_registry().merge_snapshot(entries);
            session.global_names_received.store(true, Ordering::Release);
        }
        DIST_CONTINUITY_UPSERT => match crate::dist::continuity::decode_upsert_payload(&msg) {
            Ok((next_attempt_token, record)) => {
                if let Err(error) = crate::dist::continuity::continuity_registry()
                    .merge_remote_record(next_attempt_token, record)
                {
                    eprintln!(
                        "mesh continuity: transition=upsert_rejected remote={} error={}",
                        session.remote_name, error
                    );
                }
            }
            Err(error) => {
                eprintln!(
                    "mesh continuity: transition=upsert_malformed remote={} error={}",
                    session.remote_name, error
                );
            }
        },
        DIST_CONTINUITY_SYNC => match crate::dist::continuity::decode_sync_payload(&msg) {
            Ok(snapshot) => {
                if let Err(error) =
                    crate::dist::continuity::continuity_registry().merge_snapshot(snapshot)
                {
                    eprintln!(
                        "mesh continuity: transition=sync_rejected remote={} error={}",
                        session.remote_name, error
                    );
                } else {
                    crate::dist::readiness::mark_initial_state_synchronized();
                }
            }
            Err(error) => {
                eprintln!(
                    "mesh continuity: transition=sync_malformed remote={} error={}",
                    session.remote_name, error
                );
            }
        },
        DIST_CONTINUITY_STORE_SNAPSHOT => {
            if let Err(error) = crate::dist::continuity::handle_store_snapshot_chunk(session, &msg)
            {
                eprintln!(
                    "mesh continuity: transition=store_snapshot_rejected remote={} error={}",
                    session.remote_name, error
                );
            }
        }
        DIST_CONTINUITY_STORE_SNAPSHOT_ACK => {
            if let Err(error) = crate::dist::continuity::handle_store_snapshot_ack(session, &msg) {
                eprintln!(
                    "mesh continuity: transition=store_snapshot_ack_rejected remote={} error={}",
                    session.remote_name, error
                );
            }
        }
        DIST_CONTINUITY_STORE_LOG_ENTRY => {
            if let Err(error) = crate::dist::continuity::handle_store_log_entry(session, &msg) {
                eprintln!(
                    "mesh continuity: transition=store_log_rejected remote={} error={}",
                    session.remote_name, error
                );
            }
        }
        DIST_CONTINUITY_PREPARE => {
            if let Ok((request_id, record)) = decode_continuity_prepare_payload(&msg) {
                dispatch_continuity_prepare(Arc::clone(session), request_id, record);
            }
        }
        DIST_CONTINUITY_PREPARE_ACK => {
            if let Ok((request_id, result)) = decode_continuity_prepare_ack(&msg) {
                if let Some(sender) = session
                    .pending_continuity_prepares
                    .lock()
                    .unwrap()
                    .remove(&request_id)
                {
                    let _ = sender.send(result);
                }
            }
        }
        DIST_OPERATOR_QUERY => {
            if autonomous_mode_requested()
                && !session.remote_has_role("operator")
                && !session.remote_has_role("controller")
            {
                eprintln!(
                    "mesh operator: transition=query_rejected remote={} reason=operator_identity_required",
                    session.remote_name
                );
            } else {
                crate::dist::operator::handle_operator_query_message(session, &msg);
            }
        }
        DIST_OPERATOR_REPLY => {
            crate::dist::operator::handle_operator_reply_message(session, &msg);
        }
        DIST_CONSENSUS_RPC => {
            if autonomous_mode_requested() && !session.remote_has_role("controller") {
                eprintln!(
                    "mesh consensus: transition=rpc_request_rejected remote={} reason=controller_identity_required",
                    session.remote_name
                );
                return;
            }
            match decode_consensus_rpc_frame(&msg, DIST_CONSENSUS_RPC) {
                Ok((correlation_id, request)) => {
                    crate::dist::consensus::handle_mesh_consensus_rpc(
                        Arc::clone(session),
                        correlation_id,
                        request,
                    );
                }
                Err(error) => eprintln!(
                    "mesh consensus: transition=rpc_request_rejected remote={} reason={}",
                    session.remote_name, error
                ),
            }
        }
        DIST_CONSENSUS_RPC_REPLY => {
            if autonomous_mode_requested() && !session.remote_has_role("controller") {
                eprintln!(
                    "mesh consensus: transition=rpc_reply_rejected remote={} reason=controller_identity_required",
                    session.remote_name
                );
                return;
            }
            match decode_consensus_rpc_frame(&msg, DIST_CONSENSUS_RPC_REPLY) {
                Ok((correlation_id, reply)) => {
                    if let Some(sender) = session
                        .pending_consensus_rpcs
                        .lock()
                        .unwrap()
                        .remove(&correlation_id)
                    {
                        let _ = sender.send(Ok(reply));
                    }
                }
                Err(error) => eprintln!(
                    "mesh consensus: transition=rpc_reply_rejected remote={} reason={}",
                    session.remote_name, error
                ),
            }
        }
        DIST_LOAD_REPORT => match crate::dist::routing::NodeLoadReport::decode(&msg[1..]) {
            Ok(report) if report.node_id == session.remote_name => {
                if let Err(error) =
                    crate::dist::routing::load_report_registry().apply(report, Instant::now())
                {
                    eprintln!(
                        "mesh routing: transition=load_report_rejected remote={} reason={}",
                        session.remote_name, error
                    );
                }
            }
            Ok(_) => {
                eprintln!(
                        "mesh routing: transition=load_report_rejected remote={} reason=identity_mismatch",
                        session.remote_name
                    );
            }
            Err(error) => {
                eprintln!(
                    "mesh routing: transition=load_report_rejected remote={} reason={}",
                    session.remote_name, error
                );
            }
        },
        DIST_HTTP_ROUTE_V2_QUERY => {
            dispatch_http_route_v2_reply(Arc::clone(session), msg);
        }
        DIST_HTTP_ROUTE_V2_REPLY => {
            if let Ok((correlation_id, result)) = decode_http_route_v2_reply_frame(&msg) {
                if let Some(sender) = session
                    .pending_http_routes
                    .lock()
                    .unwrap()
                    .remove(&correlation_id)
                {
                    let _ = sender.send(result);
                }
            }
        }
        DIST_HTTP_RESERVE => {
            handle_http_reserve(session, &msg);
        }
        DIST_HTTP_RESERVE_REPLY => {
            if let Ok((correlation_id, result)) = decode_http_reserve_reply(&msg) {
                if let Some(sender) = session
                    .pending_http_reservations
                    .lock()
                    .unwrap()
                    .remove(&correlation_id)
                {
                    let _ = sender.send(result);
                }
            }
        }
        DIST_CONTINUITY_RESPONSE => match decode_continuity_response_frame(&msg) {
            Ok((operation_key, response)) => {
                if let Err(error) = crate::dist::continuity_store::persist_runtime_response(
                    &operation_key,
                    &response,
                ) {
                    eprintln!(
                        "mesh continuity: response_replica_failed operation={} reason={}",
                        operation_key, error
                    );
                }
            }
            Err(error) => eprintln!(
                "mesh continuity: response_replica_rejected remote={} reason={}",
                session.remote_name, error
            ),
        },
        DIST_ROOM_BROADCAST => {
            // Wire format: [tag 0x1E][u16 room_name_len][room_name][u32 msg_len][msg]
            // Deliver to local room members only -- do NOT re-forward to other
            // nodes (prevents infinite broadcast storms; see RESEARCH.md Pitfall 1).
            if msg.len() >= 3 {
                let room_name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
                if msg.len() >= 3 + room_name_len + 4 {
                    if let Ok(room_name) = std::str::from_utf8(&msg[3..3 + room_name_len]) {
                        let msg_len = u32::from_le_bytes(
                            msg[3 + room_name_len..7 + room_name_len]
                                .try_into()
                                .unwrap(),
                        ) as usize;
                        if msg.len() >= 7 + room_name_len + msg_len {
                            if let Ok(text) = std::str::from_utf8(
                                &msg[7 + room_name_len..7 + room_name_len + msg_len],
                            ) {
                                crate::ws::rooms::local_room_broadcast(room_name, text);
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // Unknown tag -- silently ignore for forward compatibility.
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
    let load_report_interval =
        crate::dist::routing::runtime_load_report_interval().max(Duration::from_millis(25));
    let loop_interval = load_report_interval.min(Duration::from_millis(500));
    let mut last_load_report = Instant::now()
        .checked_sub(load_report_interval)
        .unwrap_or_else(Instant::now);
    loop {
        std::thread::sleep(loop_interval);

        if session.shutdown.load(Ordering::SeqCst) {
            break;
        }

        // A reservation can outlive its query when the application lane is
        // saturated or a session is replaced between the two frames. Reap it
        // from the always-running control loop so admission capacity is
        // released even when no later application message arrives.
        expire_http_reservations(&session, Instant::now());

        if last_load_report.elapsed() >= load_report_interval {
            send_load_report(&session);
            last_load_report = Instant::now();
        }

        let mut hs = heartbeat_state.lock().unwrap();

        if hs.is_pong_overdue() {
            eprintln!("mesh node heartbeat timeout: {}", session_name);
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

            if session.send_heartbeat(ping).is_err() {
                session.shutdown.store(true, Ordering::Release);
                break;
            }
        }
    }

    cleanup_session_if_current(&session);
}

fn send_load_report(session: &Arc<NodeSession>) {
    let state = started_node();
    crate::dist::routing::refresh_local_routing_telemetry();
    let handlers: BTreeSet<String> = declared_handler_registry().read().keys().cloned().collect();
    let report = crate::dist::routing::local_load_report(&state.name, handlers);
    let _ = crate::dist::routing::load_report_registry().apply(report.clone(), Instant::now());
    let Ok(encoded) = report.encode() else {
        return;
    };
    let mut payload = Vec::with_capacity(1 + encoded.len());
    payload.push(DIST_LOAD_REPORT);
    payload.extend_from_slice(&encoded);
    let _ = session.send(OutboundClass::Control, payload);
}

// ---------------------------------------------------------------------------
// cleanup_session_if_current -- removes a disconnected node from NodeState
// ---------------------------------------------------------------------------

/// Remove a disconnected node's session from NodeState only if this exact
/// session instance is still the registered one for the remote node.
///
/// This lets duplicate-session resolution replace a stale half-connection
/// without letting the old reader/heartbeat threads later remove the live
/// replacement by name alone.
fn cleanup_session_if_current(session: &Arc<NodeSession>) {
    if let Some(state) = NODE_STATE.get() {
        let removed = {
            let mut sessions = state.sessions.write();
            match sessions.get(&session.remote_name) {
                Some(current) if Arc::ptr_eq(current, session) => {
                    sessions.remove(&session.remote_name)
                }
                _ => None,
            }
        };
        if let Some(session) = removed {
            record_peer_transport_failure(&session.remote_name, Instant::now());
            fail_pending_session_requests(&session, "peer_session_disconnected");
            let node_id = session.node_id;
            let mut id_map = state.node_id_map.write();
            id_map.remove(&node_id);
            drop(id_map);
            // Phase 66: Fire all failure signals for the disconnected node.
            handle_node_disconnect(&session.remote_name, node_id);
        }
    }
}

fn fail_pending_session_requests(session: &NodeSession, reason: &str) {
    for (_, sender) in session.pending_spawns.lock().unwrap().drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
    for (_, sender) in session.pending_continuity_prepares.lock().unwrap().drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
    for (_, sender) in session.pending_operator_queries.lock().unwrap().drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
    for (_, sender) in session.pending_consensus_rpcs.lock().unwrap().drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
    for (_, sender) in session.pending_http_routes.lock().unwrap().drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
    for (_, sender) in session.pending_http_reservations.lock().unwrap().drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
    session.accepted_http_reservations.lock().unwrap().clear();
}

fn encode_consensus_rpc_frame(
    tag: u8,
    correlation_id: u64,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    if !matches!(tag, DIST_CONSENSUS_RPC | DIST_CONSENSUS_RPC_REPLY) {
        return Err("consensus_rpc_tag_invalid".to_string());
    }
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| "consensus_rpc_payload_too_large".to_string())?;
    let mut frame = Vec::with_capacity(13 + payload.len());
    frame.push(tag);
    frame.extend_from_slice(&correlation_id.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

fn decode_consensus_rpc_frame(msg: &[u8], expected_tag: u8) -> Result<(u64, Vec<u8>), String> {
    if !matches!(expected_tag, DIST_CONSENSUS_RPC | DIST_CONSENSUS_RPC_REPLY)
        || msg.first().copied() != Some(expected_tag)
        || msg.len() < 13
    {
        return Err("consensus_rpc_frame_invalid".to_string());
    }
    let correlation_id = u64::from_le_bytes(msg[1..9].try_into().unwrap());
    if correlation_id == 0 {
        return Err("consensus_rpc_correlation_invalid".to_string());
    }
    let payload_len = u32::from_le_bytes(msg[9..13].try_into().unwrap()) as usize;
    if msg.len() != 13usize.saturating_add(payload_len) {
        return Err("consensus_rpc_length_invalid".to_string());
    }
    Ok((correlation_id, msg[13..].to_vec()))
}

/// Send one OpenRaft RPC over an already-authenticated persistent Mesh
/// session. A dedicated async waiter prevents Raft traffic from blocking the
/// distribution reader or actor scheduler.
pub(crate) async fn execute_mesh_consensus_rpc(
    target: &str,
    payload: Vec<u8>,
    snapshot: bool,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    if target.trim().is_empty() || timeout.is_zero() {
        return Err("consensus_rpc_target_invalid".to_string());
    }
    let state = node_state().ok_or_else(|| "consensus_rpc_node_not_started".to_string())?;
    let session = state
        .sessions
        .read()
        .get(target)
        .cloned()
        .ok_or_else(|| format!("consensus_rpc_session_unavailable:{target}"))?;
    if !session.negotiated_protocol.autonomous_enabled {
        return Err(session
            .negotiated_protocol
            .disabled_reason
            .clone()
            .unwrap_or_else(|| "consensus_rpc_capability_unavailable".to_string()));
    }

    let correlation_id = CONSENSUS_RPC_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let frame = encode_consensus_rpc_frame(DIST_CONSENSUS_RPC, correlation_id, &payload)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    session
        .pending_consensus_rpcs
        .lock()
        .unwrap()
        .insert(correlation_id, sender);
    let class = if snapshot {
        OutboundClass::Snapshot
    } else {
        OutboundClass::Control
    };
    if let Err(error) = session.send(class, frame) {
        session
            .pending_consensus_rpcs
            .lock()
            .unwrap()
            .remove(&correlation_id);
        return Err(format!("consensus_rpc_write_failed:{error}"));
    }

    // The reply handler and a disconnect (fail_pending_session_requests) send
    // before they drop the sender, so the wait ends in a reply or here.
    match tokio::time::timeout(timeout, receiver).await {
        Ok(Ok(result)) => result,
        _ => {
            session
                .pending_consensus_rpcs
                .lock()
                .unwrap()
                .remove(&correlation_id);
            crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
            Err("consensus_rpc_reply_timeout".to_string())
        }
    }
}

pub(crate) fn send_mesh_consensus_rpc_reply(
    session: &Arc<NodeSession>,
    correlation_id: u64,
    payload: &[u8],
) -> Result<(), String> {
    if !session.negotiated_protocol.autonomous_enabled {
        return Err("consensus_rpc_capability_unavailable".to_string());
    }
    let frame = encode_consensus_rpc_frame(DIST_CONSENSUS_RPC_REPLY, correlation_id, payload)?;
    session.send(OutboundClass::Control, frame)
}

// ---------------------------------------------------------------------------
// handle_node_disconnect -- propagate failure signals on node loss
// ---------------------------------------------------------------------------

/// Handle node disconnection: fire all failure signals locally.
///
/// This is the central failure handler for distributed fault tolerance.
/// Called from cleanup_session after removing the session from NodeState.
///
/// Two-phase approach to avoid deadlocks:
/// 1. Under process table READ lock, collect all actions to take
/// 2. Drop lock, then execute collected actions
fn handle_node_disconnect(node_name: &str, node_id: u16) {
    use crate::actor::heap::MessageBuffer;
    use crate::actor::link;
    use crate::actor::process::{ExitReason, Message, ProcessId, ProcessState};

    // A node starts from code the scheduler runs.
    let sched = crate::actor::global_scheduler();

    let noconnection = ExitReason::Noconnection;

    // Phase 1: Collect under read lock.
    // For links: (local_pid, Vec<remote_pid_to_unlink>)
    let mut link_actions: Vec<(ProcessId, Vec<ProcessId>)> = Vec::new();
    // For monitors: (local_pid, Vec<monitor_ref>)
    let mut monitor_actions: Vec<(ProcessId, Vec<u64>)> = Vec::new();

    {
        let table = sched.process_table().read();
        for (&pid, proc_arc) in table.iter() {
            let proc = proc_arc.lock();

            // Collect remote links to the disconnected node.
            let remote_links: Vec<ProcessId> = proc
                .links
                .iter()
                .filter(|linked_pid| linked_pid.node_id() == node_id)
                .cloned()
                .collect();

            if !remote_links.is_empty() {
                link_actions.push((pid, remote_links));
            }

            // Collect remote monitors to the disconnected node.
            let remote_monitors: Vec<u64> = proc
                .monitors
                .iter()
                .filter(|(_, monitor)| monitor.target.node_id() == node_id)
                .map(|(monitor_ref, _)| *monitor_ref)
                .collect();

            if !remote_monitors.is_empty() {
                monitor_actions.push((pid, remote_monitors));
            }
        }
    }
    // Process table read lock dropped here.

    // Phase 2: Execute collected actions.
    // Process remote link disconnections.
    for (local_pid, remote_pids) in &link_actions {
        if let Some(proc_arc) = sched.get_process(*local_pid) {
            let mut proc = proc_arc.lock();

            // Skip already-exited processes.
            if matches!(proc.state, ProcessState::Exited(_)) {
                continue;
            }

            // Remove the remote links.
            for remote_pid in remote_pids {
                proc.links.remove(remote_pid);
            }

            // Deliver :noconnection exit signal.
            // Track whether we need to wake after processing all links.
            let mut need_wake = false;
            for remote_pid in remote_pids {
                if matches!(proc.state, ProcessState::Exited(_)) {
                    break;
                }

                if proc.trap_exit {
                    let signal_data = link::encode_exit_signal(*remote_pid, &noconnection);
                    let buffer = MessageBuffer::new(signal_data, link::EXIT_SIGNAL_TAG);
                    proc.mailbox.push(Message { buffer });
                    if matches!(proc.state, ProcessState::Waiting) {
                        need_wake = proc.set_live_state(ProcessState::Ready);
                    }
                } else {
                    proc.mark_exited(ExitReason::Linked(
                        *remote_pid,
                        Box::new(noconnection.clone()),
                    ));
                    break;
                }
            }

            if need_wake {
                drop(proc);
                sched.wake_process(*local_pid);
            }
        }
    }

    // Process remote monitor disconnections.
    for (local_pid, monitors) in &monitor_actions {
        if let Some(proc_arc) = sched.get_process(*local_pid) {
            let mut proc = proc_arc.lock();

            // Skip already-exited processes.
            if matches!(proc.state, ProcessState::Exited(_)) {
                continue;
            }

            for monitor_ref in monitors {
                proc.fire_monitor(*monitor_ref);
            }

            sched.wake_if_waiting(*local_pid, proc);
        }
    }

    // Tell the processes watching the node, once.
    if let Some(state) = node_state() {
        let watchers = state.node_monitors.write().remove(node_name);
        for (watcher_pid, buffer) in watchers.into_iter().flatten() {
            if let Some(proc_arc) = sched.get_process(watcher_pid) {
                let proc = proc_arc.lock();
                proc.mailbox.push(Message { buffer });
                sched.wake_if_waiting(watcher_pid, proc);
            }
        }
    }

    let registry = crate::dist::continuity::continuity_registry();
    let continuity_affected = registry.snapshot().records.into_iter().any(|record| {
        record.phase == crate::dist::continuity::ContinuityPhase::Submitted
            && record.result == crate::dist::continuity::ContinuityResult::Pending
            && (record.owner_node == node_name
                || record
                    .acknowledged_replica_nodes()
                    .iter()
                    .any(|replica| replica == node_name))
    });

    // Mark pending continuity records that just lost their owner as recovery-eligible.
    let owner_lost_records = crate::dist::continuity::continuity_registry()
        .mark_owner_loss_records_for_node_loss(node_name);

    let authority = registry.authority_status();
    if continuity_affected
        && authority.cluster_role == crate::dist::continuity::ContinuityClusterRole::Primary
    {
        maybe_spawn_primary_owner_loss_recovery(node_name);
    }

    // Downgrade mirrored continuity records that just lost replica safety.
    let _ = crate::dist::continuity::continuity_registry()
        .degrade_replica_records_for_node_loss(node_name);

    // Standby-mirrored continuity should degrade replication health instead of implying promotion.
    let _ = crate::dist::continuity::continuity_registry()
        .degrade_replication_health_for_node_loss(node_name);

    if authority.cluster_role == crate::dist::continuity::ContinuityClusterRole::Standby
        && (!owner_lost_records.is_empty() || continuity_affected)
    {
        maybe_automatic_promote_and_resume(node_name);
    }

    // Phase 68: Clean up global registrations for the disconnected node.
    let removed_names = crate::dist::global::global_name_registry().cleanup_node(node_name);
    for name in &removed_names {
        crate::dist::global::broadcast_global_unregister(name);
    }
}

// ---------------------------------------------------------------------------
// send_dist_monitor_exit -- send DIST_MONITOR_EXIT to a remote node
// ---------------------------------------------------------------------------

/// Send a DIST_MONITOR_EXIT wire message back to a remote node.
///
/// Used when a locally monitored process is dead/not found when a
/// DIST_MONITOR request arrives, or during local process exit to notify
/// remote monitoring processes.
fn send_dist_monitor_exit(
    session: &Arc<NodeSession>,
    monitored_pid: crate::actor::process::ProcessId,
    monitoring_pid: crate::actor::process::ProcessId,
    monitor_ref: u64,
    reason: &crate::actor::process::ExitReason,
) {
    let mut payload = Vec::with_capacity(1 + 8 + 8 + 8 + 16);
    payload.push(DIST_MONITOR_EXIT);
    payload.extend_from_slice(&monitored_pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&monitoring_pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&monitor_ref.to_le_bytes());
    crate::actor::link::encode_reason(&mut payload, reason);
    let _ = session.send(OutboundClass::Control, payload);
}

// ---------------------------------------------------------------------------
// send_spawn_reply -- reply to a DIST_SPAWN request with status and pid
// ---------------------------------------------------------------------------

/// Send a DIST_SPAWN_REPLY back to the requesting node.
///
/// Wire format: [DIST_SPAWN_REPLY][u64 request_id LE][u8 status][u64 spawned_local_id LE]
/// Status 0 = success (pid is the spawned process's local_id).
/// Status 1 = error (function not found; pid is 0).
fn send_spawn_reply(session: &NodeSession, req_id: u64, status: u8, spawned_local_id: u64) {
    let mut payload = Vec::with_capacity(18);
    payload.push(DIST_SPAWN_REPLY);
    payload.extend_from_slice(&req_id.to_le_bytes());
    payload.push(status);
    payload.extend_from_slice(&spawned_local_id.to_le_bytes());
    let _ = session.send(OutboundClass::Control, payload);
}

pub(crate) fn continuity_owner_loss_recovery_eligible(
    existing: &crate::dist::continuity::ContinuityRecord,
    request: &crate::dist::continuity::SubmitRequest,
) -> bool {
    existing.cluster_role == crate::dist::continuity::ContinuityClusterRole::Primary
        && request.cluster_role == crate::dist::continuity::ContinuityClusterRole::Primary
        && existing.phase == crate::dist::continuity::ContinuityPhase::Submitted
        && existing.result == crate::dist::continuity::ContinuityResult::Pending
        && existing.replica_status == crate::dist::continuity::ReplicaStatus::OwnerLost
        && request.promotion_epoch >= existing.promotion_epoch
        && existing.owner_node != request.owner_node
}

pub(crate) fn prepare_continuity_replica(
    record: &crate::dist::continuity::ContinuityRecord,
) -> Result<Vec<String>, String> {
    let replicas = record_replica_set(record)?;
    let required_acknowledgements = (record.replication_count / 2) as usize;
    let mut acknowledged = Vec::new();
    let mut failed = Vec::new();
    for replica in replicas {
        let mut replica_record = record.clone();
        replica_record.replica_node = replica.clone();
        match prepare_one_continuity_replica(&replica_record) {
            Ok(()) => acknowledged.push(replica),
            Err(reason) => failed.push((replica, reason)),
        }
    }
    if acknowledged.len() < required_acknowledgements
        && !crate::dist::continuity_store::degraded_durability_enabled()
    {
        return Err(format!(
            "continuity_replica_ack_threshold_unmet:required={required_acknowledgements}:acknowledged={}:failures={failed:?}",
            acknowledged.len()
        ));
    }
    if !failed.is_empty() {
        spawn_continuity_replica_repair(record.clone(), failed);
    }
    Ok(acknowledged)
}

fn spawn_continuity_replica_repair(
    record: crate::dist::continuity::ContinuityRecord,
    failed: Vec<(String, String)>,
) {
    let _ = std::thread::Builder::new()
        .name("mesh-continuity-replica-repair".to_string())
        .spawn(move || {
            for (replica, _) in failed {
                let mut replica_record = record.clone();
                replica_record.replica_node = replica.clone();
                for attempt in 0..5_u32 {
                    if prepare_one_continuity_replica(&replica_record).is_ok() {
                        let _ = crate::dist::continuity::continuity_registry()
                            .acknowledge_replica_node(
                                &record.request_key,
                                &record.attempt_id,
                                &replica,
                            );
                        break;
                    }
                    let base = 50_u64.saturating_mul(1_u64 << attempt.min(4));
                    let jitter = rand::random::<u64>() % base.max(1);
                    std::thread::park_timeout(Duration::from_millis(base + jitter));
                }
            }
        });
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DrainContinuityOutcome {
    pub runtime_node_id: String,
    pub ownership_transfers: u32,
    pub replica_replacements: u32,
    pub records_examined: u32,
}

static ACTIVE_OWNERSHIP_TRANSFERS: OnceLock<Mutex<BTreeMap<String, u32>>> = OnceLock::new();
static ACTIVE_OWNER_LOSS_RECOVERIES: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();

fn active_ownership_transfers() -> &'static Mutex<BTreeMap<String, u32>> {
    ACTIVE_OWNERSHIP_TRANSFERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn active_owner_loss_recoveries() -> &'static Mutex<BTreeSet<String>> {
    ACTIVE_OWNER_LOSS_RECOVERIES.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn local_coordinates_node_loss_recovery(disconnected_node: &str) -> bool {
    let Some(state) = node_state() else {
        return false;
    };
    if let Some(consensus) = crate::dist::consensus::consensus_runtime_snapshot() {
        return consensus.state == "leader"
            && consensus.current_leader == Some(consensus.node_id)
            && consensus.node_name == state.name;
    }
    let coordinator = canonical_declared_membership()
        .into_iter()
        .find(|node| node != disconnected_node);
    coordinator.as_deref() == Some(state.name.as_str())
}

fn log_owner_loss_recovery_failure(disconnected_node: &str, reason: &str) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "owner_loss_recovery_failed".to_string(),
        reason: Some(reason.to_string()),
        metadata: vec![(
            "disconnected_node".to_string(),
            disconnected_node.to_string(),
        )],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "mesh continuity: owner_loss_recovery_failed node={} reason={}",
        disconnected_node, reason
    );
}

fn maybe_spawn_primary_owner_loss_recovery(disconnected_node: &str) {
    if !local_coordinates_node_loss_recovery(disconnected_node) {
        return;
    }

    let disconnected_node = disconnected_node.to_string();
    {
        let mut active = active_owner_loss_recoveries().lock().unwrap();
        if !active.insert(disconnected_node.clone()) {
            return;
        }
    }
    let recovery_node = disconnected_node.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("mesh-continuity-owner-loss".to_string())
        .spawn(move || {
            let result = prepare_continuity_for_runtime_node(&recovery_node);
            active_owner_loss_recoveries()
                .lock()
                .unwrap()
                .remove(&recovery_node);
            if let Err(reason) = result {
                log_owner_loss_recovery_failure(&recovery_node, &reason);
            }
        })
    {
        active_owner_loss_recoveries()
            .lock()
            .unwrap()
            .remove(&disconnected_node);
        eprintln!(
            "mesh continuity: owner_loss_recovery_thread_failed node={} reason={}",
            disconnected_node, error
        );
    }
}

/// Re-drive owner-loss recovery from the fenced controller leader. A
/// disconnect notification is edge-triggered and can race record replication;
/// this level-triggered sweep makes recovery converge after either ordering.
pub(crate) fn recover_pending_owner_losses_if_coordinator() {
    let records = crate::dist::continuity::continuity_registry()
        .snapshot()
        .records;
    let owners: BTreeSet<String> = records
        .iter()
        .filter(|record| {
            record.cluster_role == crate::dist::continuity::ContinuityClusterRole::Primary
                && record.phase == crate::dist::continuity::ContinuityPhase::Submitted
                && record.result == crate::dist::continuity::ContinuityResult::Pending
                && record.replica_status == crate::dist::continuity::ReplicaStatus::OwnerLost
        })
        .map(|record| record.owner_node.clone())
        .collect();
    let membership: BTreeSet<String> = canonical_declared_membership().into_iter().collect();
    // Replica loss can be observed before a replacement leader is elected.
    // Re-sweep missing configured replica participants from level-triggered
    // state so that an edge-triggered disconnect cannot leave owner-only work
    // permanently blocking scale-down.
    let missing_replicas = missing_continuity_replica_participants(&records, &membership);
    for participant in owners.union(&missing_replicas) {
        maybe_spawn_primary_owner_loss_recovery(participant);
    }
}

fn missing_continuity_replica_participants(
    records: &[crate::dist::continuity::ContinuityRecord],
    membership: &BTreeSet<String>,
) -> BTreeSet<String> {
    records
        .iter()
        .filter(|record| {
            record.cluster_role == crate::dist::continuity::ContinuityClusterRole::Primary
                && record.phase == crate::dist::continuity::ContinuityPhase::Submitted
                && record.result == crate::dist::continuity::ContinuityResult::Pending
        })
        .flat_map(|record| record.replica_nodes().to_vec())
        .filter(|replica| !membership.contains(replica))
        .collect()
}

pub(crate) fn continuity_active_ownership_transfers(node_id: &str) -> u32 {
    active_ownership_transfers()
        .lock()
        .unwrap()
        .get(node_id)
        .copied()
        .unwrap_or(0)
}

struct OwnershipTransferGuard {
    node_id: String,
}

impl OwnershipTransferGuard {
    fn new(node_id: &str) -> Self {
        *active_ownership_transfers()
            .lock()
            .unwrap()
            .entry(node_id.to_string())
            .or_default() += 1;
        Self {
            node_id: node_id.to_string(),
        }
    }
}

impl Drop for OwnershipTransferGuard {
    fn drop(&mut self) {
        let mut active = active_ownership_transfers().lock().unwrap();
        let count = active
            .get_mut(&self.node_id)
            .expect("a transfer is counted from when it starts until it ends");
        *count -= 1;
        if *count == 0 {
            active.remove(&self.node_id);
        }
    }
}

/// Resolves a capacity-provider identifier to the stable Mesh runtime name.
/// Docker exposes a full container id while Mesh names use its hostname
/// prefix; exact names always win and ambiguous prefixes fail closed.
pub(crate) fn resolve_runtime_node_id(identifier: &str) -> Result<String, String> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err("runtime_node_identifier_missing".to_string());
    }
    let membership = canonical_declared_membership();
    if membership.iter().any(|node| node == identifier) {
        return Ok(identifier.to_string());
    }
    let prefix = &identifier[..identifier.len().min(12)];
    let matches: Vec<_> = membership
        .into_iter()
        .filter(|node| node.starts_with(prefix))
        .collect();
    match matches.as_slice() {
        [node] => Ok(node.clone()),
        [] => Err(format!("runtime_node_not_found:{identifier}")),
        _ => Err(format!("runtime_node_identifier_ambiguous:{identifier}")),
    }
}

/// Cooperatively removes a node from every active continuity record before a
/// capacity driver may terminate it. Replica preparation happens before the
/// compare-and-swap record update. Ownership changes allocate a new attempt id
/// which fences late completion by the draining owner.
pub(crate) fn prepare_continuity_for_drain(
    identifier: &str,
) -> Result<DrainContinuityOutcome, String> {
    let runtime_node_id = resolve_runtime_node_id(identifier)?;
    prepare_continuity_for_runtime_node(&runtime_node_id)
}

fn continuity_replacement_superseded(request_key: &str, expected_attempt_id: &str) -> bool {
    crate::dist::continuity::continuity_registry()
        .record(request_key)
        .is_none_or(|current| {
            current.attempt_id != expected_attempt_id
                || current.phase != crate::dist::continuity::ContinuityPhase::Submitted
                || current.result != crate::dist::continuity::ContinuityResult::Pending
        })
}

fn dispatch_recovered_http_record(
    record: crate::dist::continuity::ContinuityRecord,
) -> Result<(), String> {
    let request_key = record.request_key.clone();
    let attempt_id = record.attempt_id.clone();
    let thread_request_key = request_key.clone();
    let thread_attempt_id = attempt_id.clone();
    std::thread::Builder::new()
        .name("mesh-continuity-http-recovery".to_string())
        .spawn(move || {
            let result = (|| -> Result<Vec<u8>, String> {
                let entry = lookup_declared_handler(record.declared_handler_runtime_name())
                    .ok_or_else(|| {
                        format!(
                            "continuity_recovery_handler_unavailable:{}",
                            record.declared_handler_runtime_name()
                        )
                    })?;
                if record.owner_node == node_state().map_or("", |state| state.name.as_str()) {
                    execute_clustered_http_route_locally(
                        entry.fn_ptr.0,
                        &record.request_key,
                        &record.attempt_id,
                        record.request_payload(),
                    )
                } else {
                    execute_clustered_http_route_remote(
                        &record.owner_node,
                        record.declared_handler_runtime_name(),
                        &record.request_key,
                        &record.attempt_id,
                        record.request_payload(),
                    )
                }
            })();
            match result {
                Ok(response) => {
                    retain_and_broadcast_continuity_response(&thread_request_key, &response)
                }
                Err(reason)
                    if !continuity_replacement_superseded(
                        &thread_request_key,
                        &thread_attempt_id,
                    ) =>
                {
                    reject_clustered_http_route_attempt(
                        &thread_request_key,
                        &thread_attempt_id,
                        &reason,
                    );
                    log_owner_loss_recovery_failure(&thread_request_key, &reason);
                }
                Err(_) => {}
            }
        })
        .map(|_| ())
        .map_err(|error| {
            let reason = format!("continuity_recovery_thread_failed:{error}");
            reject_clustered_http_route_attempt(&request_key, &attempt_id, &reason);
            reason
        })
}

fn prepare_continuity_for_runtime_node(
    runtime_node_id: &str,
) -> Result<DrainContinuityOutcome, String> {
    use crate::dist::continuity::{
        ContinuityPhase, ContinuityResult, ReplicaStatus, ReplicationHealth,
    };

    let runtime_node_id = runtime_node_id.to_string();
    let _transfer_guard = OwnershipTransferGuard::new(&runtime_node_id);
    let membership: Vec<String> = canonical_declared_membership()
        .into_iter()
        .filter(|node| node != &runtime_node_id)
        .collect();
    let registry = crate::dist::continuity::continuity_registry();
    let active: Vec<_> = registry
        .snapshot()
        .records
        .into_iter()
        .filter(|record| {
            record.phase == ContinuityPhase::Submitted
                && record.result == ContinuityResult::Pending
                && (record.owner_node == runtime_node_id
                    || record
                        .acknowledged_replica_nodes()
                        .iter()
                        .any(|node| node == &runtime_node_id)
                    || record
                        .replica_nodes()
                        .iter()
                        .any(|node| node == &runtime_node_id))
        })
        .collect();
    let mut outcome = DrainContinuityOutcome {
        runtime_node_id: runtime_node_id.clone(),
        records_examined: active.len().try_into().unwrap_or(u32::MAX),
        ..DrainContinuityOutcome::default()
    };

    'records: for record in active {
        let previous_attempt_id = record.attempt_id.clone();
        if continuity_replacement_superseded(&record.request_key, &previous_attempt_id) {
            continue;
        }
        let owner_transfer = record.owner_node == runtime_node_id;
        if owner_transfer && record.declared_handler_runtime_name().is_empty() {
            return Err(format!(
                "continuity_drain_untransferable_active_record:{}",
                record.request_key
            ));
        }
        let required = record.replication_count.saturating_sub(1) as usize;
        let previous_replicas = record.acknowledged_replica_nodes().to_vec();
        let reports = crate::dist::routing::load_report_registry();
        let now = Instant::now();
        let policy = crate::dist::routing::runtime_routing_policy();
        let new_owner = if owner_transfer {
            let mut eligible_workers: Vec<String> = membership
                .iter()
                .filter(|node| {
                    reports
                        .report(node, now, policy.load_report_ttl)
                        .is_some_and(|report| {
                            report.state.routing_eligible()
                                && report
                                    .roles
                                    .contains(crate::dist::telemetry::NodeRoles::WORKER)
                                && report
                                    .handlers
                                    .contains(record.declared_handler_runtime_name())
                        })
                })
                .cloned()
                .collect();
            eligible_workers.sort_by_key(|node| {
                (
                    !previous_replicas.contains(node),
                    stable_hash_u64(node),
                    node.clone(),
                )
            });
            eligible_workers
                .into_iter()
                .next()
                .ok_or_else(|| "continuity_drain_owner_transfer_unavailable".to_string())?
        } else {
            record.owner_node.clone()
        };

        let mut replica_nodes: Vec<String> = previous_replicas
            .iter()
            .filter(|node| {
                *node != &runtime_node_id && *node != &new_owner && membership.contains(*node)
            })
            .cloned()
            .collect();
        let mut candidates: Vec<_> = membership
            .iter()
            .filter(|node| *node != &new_owner && !replica_nodes.contains(*node))
            .filter(|node| {
                reports
                    .report(node, now, policy.load_report_ttl)
                    .is_some_and(|report| {
                        report.state.routing_eligible()
                            && (record.declared_handler_runtime_name().is_empty()
                                || report
                                    .handlers
                                    .contains(record.declared_handler_runtime_name()))
                    })
            })
            .cloned()
            .collect();
        candidates.sort();
        for candidate in candidates {
            if replica_nodes.len() >= required {
                break;
            }
            replica_nodes.push(candidate);
        }
        if replica_nodes.len() != required {
            return Err(format!(
                "continuity_drain_replica_capacity_unavailable:request={}:required={required}:available={}",
                crate::dist::continuity::request_key_fingerprint(&record.request_key),
                replica_nodes.len()
            ));
        }

        let (watermark, next_attempt_id) = if owner_transfer {
            registry.reserve_transfer_attempt()
        } else {
            (registry.next_attempt_token(), previous_attempt_id.clone())
        };
        let mut next = record.clone();
        next.record_version = next.record_version.saturating_add(1);
        next.owner_node = new_owner.clone();
        next.replica_nodes = replica_nodes.clone();
        next.acknowledged_replica_nodes.clear();
        next.replica_node = replica_nodes.first().cloned().unwrap_or_default();
        next.attempt_id = next_attempt_id;
        next.replica_status = if replica_nodes.is_empty() {
            ReplicaStatus::Unassigned
        } else {
            ReplicaStatus::Preparing
        };
        next.replication_health = if replica_nodes.is_empty() {
            ReplicationHealth::LocalOnly
        } else {
            ReplicationHealth::Unavailable
        };
        next.execution_node.clear();
        next.error.clear();

        let must_prepare_all = owner_transfer;
        for replica in &replica_nodes {
            if !must_prepare_all && previous_replicas.contains(replica) {
                continue;
            }
            let mut replica_record = next.clone();
            replica_record.replica_node = replica.clone();
            if let Err(reason) = prepare_one_continuity_replica(&replica_record) {
                if continuity_replacement_superseded(&record.request_key, &previous_attempt_id) {
                    continue 'records;
                }
                return Err(reason);
            }
        }
        if !replica_nodes.is_empty() {
            next.replica_status = ReplicaStatus::Mirrored;
            next.acknowledged_replica_nodes = replica_nodes.clone();
        }
        let committed = match registry.commit_drain_replacement(&previous_attempt_id, next) {
            Ok(committed) => committed,
            Err(_reason)
                if continuity_replacement_superseded(&record.request_key, &previous_attempt_id) =>
            {
                continue
            }
            Err(reason) => return Err(reason),
        };
        if owner_transfer {
            if !committed.request_payload().is_empty() {
                dispatch_recovered_http_record(committed.clone())?;
            } else {
                let entry = lookup_declared_handler(committed.declared_handler_runtime_name())
                    .ok_or_else(|| {
                        format!(
                            "continuity_drain_handler_unavailable:{}",
                            committed.declared_handler_runtime_name()
                        )
                    })?;
                if committed.owner_node == node_state().map_or("", |state| state.name.as_str()) {
                    spawn_declared_work_local(
                        &entry,
                        &committed.request_key,
                        &committed.attempt_id,
                    );
                } else {
                    spawn_declared_work_remote(
                        &committed.owner_node,
                        &entry,
                        &committed.request_key,
                        &committed.attempt_id,
                    )?;
                }
            }
            outcome.ownership_transfers = outcome.ownership_transfers.saturating_add(1);
        } else {
            outcome.replica_replacements = outcome.replica_replacements.saturating_add(1);
        }
        let _ = watermark;
    }

    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "drain_continuity_prepared".to_string(),
        reason: Some(runtime_node_id.clone()),
        metadata: vec![
            (
                "ownership_transfers".to_string(),
                outcome.ownership_transfers.to_string(),
            ),
            (
                "replica_replacements".to_string(),
                outcome.replica_replacements.to_string(),
            ),
            (
                "records_examined".to_string(),
                outcome.records_examined.to_string(),
            ),
        ],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    Ok(outcome)
}

pub(crate) fn record_replica_set(
    record: &crate::dist::continuity::ContinuityRecord,
) -> Result<Vec<String>, String> {
    let required = record.replication_count.saturating_sub(1) as usize;
    if required == 0 {
        return Ok(Vec::new());
    }
    let recorded = record.canonical_replica_nodes();
    if recorded.len() == required {
        return Ok(recorded);
    }
    if !recorded.is_empty() {
        return Err(format!(
            "continuity_replica_set_size_mismatch:required={required}:recorded={}",
            recorded.len()
        ));
    }
    select_continuity_replica_set(&record.owner_node, record.replication_count)
}

pub(crate) fn select_continuity_replica_set(
    owner_node: &str,
    replication_count: u64,
) -> Result<Vec<String>, String> {
    let required = replication_count.saturating_sub(1) as usize;
    if required == 0 {
        return Ok(Vec::new());
    }
    let membership = canonical_declared_membership();
    if membership.len() <= required {
        return Err(format!(
            "replica_capacity_unavailable:required={required}:available={}",
            membership.len().saturating_sub(1)
        ));
    }
    let now = Instant::now();
    let state = node_state().ok_or_else(|| "continuity_node_not_started".to_string())?;
    let reports: Vec<_> = membership
        .iter()
        .filter(|node| {
            if *node == &state.name {
                return true;
            }
            state
                .sessions
                .read()
                .get(*node)
                .is_some_and(|session| !session.shutdown.load(Ordering::Acquire))
                && !peer_circuit_open(node, now)
        })
        .filter_map(|node| {
            crate::dist::routing::load_report_registry().report(
                node,
                now,
                crate::dist::routing::runtime_routing_policy().load_report_ttl,
            )
        })
        .collect();
    crate::dist::routing::select_record_replicas(owner_node, required, &reports)
}

fn prepare_one_continuity_replica(
    record: &crate::dist::continuity::ContinuityRecord,
) -> Result<(), String> {
    let state = node_state().ok_or_else(|| "replica_required_unavailable".to_string())?;
    if state.name == record.replica_node {
        return Ok(());
    }
    let session = {
        let sessions = state.sessions.read();
        sessions.get(&record.replica_node).cloned()
    }
    .ok_or_else(|| "replica_required_unavailable".to_string())?;

    let request_id = CONTINUITY_PREPARE_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let payload = encode_continuity_prepare_payload(request_id, record)?;
    let (tx, rx) = crate::actor::cooperative_channel();
    session
        .pending_continuity_prepares
        .lock()
        .unwrap()
        .insert(request_id, tx);

    let failed = |transition: &str, error: &str| {
        session
            .pending_continuity_prepares
            .lock()
            .unwrap()
            .remove(&request_id);
        replica_prepare_failed(record, transition, error)
    };
    if session.send(OutboundClass::Continuity, payload).is_err() {
        return Err(failed(
            "prepare_write_failed",
            "replica_required_unavailable",
        ));
    }
    // The ack handler and a disconnect (fail_pending_session_requests) both
    // send before they drop the sender, so the wait ends in a reply or here.
    crate::actor::cooperative_recv_timeout(&rx, Duration::from_secs(5)).unwrap_or_else(|_| {
        crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
        Err(failed("prepare_timeout", "replica_prepare_timeout"))
    })
}

/// Records that preparing `record` on its replica failed at `transition`,
/// and returns `error`.
fn replica_prepare_failed(
    record: &crate::dist::continuity::ContinuityRecord,
    transition: &str,
    error: &str,
) -> String {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: transition.to_string(),
        request_key: Some(record.request_key.clone()),
        attempt_id: Some(record.attempt_id.clone()),
        owner_node: Some(record.owner_node.clone()),
        replica_node: Some(record.replica_node.clone()),
        cluster_role: Some(record.cluster_role.as_str().to_string()),
        promotion_epoch: Some(record.promotion_epoch),
        replication_health: Some(record.replication_health.as_str().to_string()),
        replica_status: Some(record.replica_status.as_str().to_string()),
        reason: Some(error.to_string()),
        metadata: vec![("target_node".to_string(), record.replica_node.clone())],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "mesh continuity: transition={transition} request_key={} attempt_id={} cluster_role={} promotion_epoch={} replication_health={} replica={} error={error}",
        crate::dist::continuity::request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.cluster_role.as_str(),
        record.promotion_epoch,
        record.replication_health.as_str(),
        record.replica_node,
    );
    error.to_string()
}

fn encode_continuity_prepare_payload(
    request_id: u64,
    record: &crate::dist::continuity::ContinuityRecord,
) -> Result<Vec<u8>, String> {
    let encoded = crate::dist::continuity::encode_record_payload(record)?;
    let mut payload = Vec::with_capacity(1 + 8 + 4 + encoded.len());
    payload.push(DIST_CONTINUITY_PREPARE);
    payload.extend_from_slice(&request_id.to_le_bytes());
    payload.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    payload.extend_from_slice(&encoded);
    Ok(payload)
}

fn decode_continuity_prepare_payload(
    data: &[u8],
) -> Result<(u64, crate::dist::continuity::ContinuityRecord), String> {
    if data.len() < 13 {
        return Err("continuity prepare payload too short".to_string());
    }
    let request_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let record_len = u32::from_le_bytes(data[9..13].try_into().unwrap()) as usize;
    if data.len() != 13 + record_len {
        return Err("continuity prepare payload length mismatch".to_string());
    }
    let record = crate::dist::continuity::decode_record_payload(&data[13..])?;
    Ok((request_id, record))
}

fn continuity_prepare_dispatcher() -> &'static crossbeam_channel::Sender<ContinuityPrepareTask> {
    CONTINUITY_PREPARE_DISPATCHER.get_or_init(|| {
        let (sender, receiver) =
            crossbeam_channel::bounded::<ContinuityPrepareTask>(CONTINUITY_PREPARE_DISPATCH_ITEMS);
        for worker in 0..CONTINUITY_PREPARE_WORKERS {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("mesh-continuity-prepare-{worker}"))
                .spawn(move || {
                    while let Ok(task) = receiver.recv() {
                        let result = if started_node().name == task.record.replica_node {
                            crate::dist::continuity::continuity_registry()
                                .mirror_prepare(task.record)
                                .map(|_| ())
                        } else {
                            Err("replica_prepare_target_mismatch".to_string())
                        };
                        send_continuity_prepare_reply(&task.session, task.request_id, &result);
                    }
                })
                .expect("failed to spawn continuity prepare worker");
        }
        sender
    })
}

fn dispatch_continuity_prepare(
    session: Arc<NodeSession>,
    request_id: u64,
    record: crate::dist::continuity::ContinuityRecord,
) {
    let task = ContinuityPrepareTask {
        session,
        request_id,
        record,
    };
    if let Err(error) = continuity_prepare_dispatcher().try_send(task) {
        let (task, reason) = match error {
            crossbeam_channel::TrySendError::Full(task) => {
                (task, "replica_prepare_overloaded".to_string())
            }
            crossbeam_channel::TrySendError::Disconnected(task) => {
                (task, "replica_required_unavailable".to_string())
            }
        };
        send_continuity_prepare_reply(&task.session, task.request_id, &Err(reason));
    }
}

fn encode_continuity_prepare_ack(request_id: u64, result: &Result<(), String>) -> Vec<u8> {
    let reason = match result {
        Ok(()) => "",
        Err(reason) => reason.as_str(),
    };
    let reason_bytes = reason.as_bytes();
    let mut payload = Vec::with_capacity(1 + 8 + 1 + 2 + reason_bytes.len());
    payload.push(DIST_CONTINUITY_PREPARE_ACK);
    payload.extend_from_slice(&request_id.to_le_bytes());
    payload.push(if result.is_ok() { 0 } else { 1 });
    payload.extend_from_slice(&(reason_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(reason_bytes);
    payload
}

fn decode_continuity_prepare_ack(data: &[u8]) -> Result<(u64, Result<(), String>), String> {
    if data.len() < 12 {
        return Err("continuity prepare ack payload too short".to_string());
    }
    let request_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let status = data[9];
    let reason_len = u16::from_le_bytes(data[10..12].try_into().unwrap()) as usize;
    if data.len() != 12 + reason_len {
        return Err("continuity prepare ack payload length mismatch".to_string());
    }
    let reason = std::str::from_utf8(&data[12..])
        .map_err(|_| "invalid UTF-8 in continuity prepare ack".to_string())?
        .to_string();
    match status {
        0 => Ok((request_id, Ok(()))),
        1 => Ok((request_id, Err(reason))),
        _ => Err(format!("invalid continuity prepare ack status {status}")),
    }
}

fn send_continuity_prepare_reply(
    session: &NodeSession,
    request_id: u64,
    result: &Result<(), String>,
) {
    let payload = encode_continuity_prepare_ack(request_id, result);
    let _ = session.send(OutboundClass::Continuity, payload);
}

/// Send a DIST_LINK using a known session (no PID-based routing).
///
/// Used by the DIST_SPAWN handler to establish a bidirectional link between
/// the locally-spawned process and the remote requester. Unlike `send_dist_link`
/// which routes by `to_pid.node_id()`, this takes the session directly since
/// the DIST_SPAWN handler already has it.
fn send_dist_link_via_session(
    session: &NodeSession,
    from_pid: crate::actor::process::ProcessId,
    to_pid: crate::actor::process::ProcessId,
) {
    let mut payload = Vec::with_capacity(1 + 8 + 8);
    payload.push(DIST_LINK);
    payload.extend_from_slice(&from_pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&to_pid.as_u64().to_le_bytes());
    let _ = session.send(OutboundClass::Application, payload);
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
            format!(
                "handshake message too large: {} bytes (max {})",
                len, MAX_HANDSHAKE_MSG
            ),
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
    read_dist_msg_bounded(stream, MAX_DIST_MSG)
}

#[derive(Default)]
struct PersistentFrameReader {
    length: [u8; 4],
    length_read: usize,
    payload: Vec<u8>,
    payload_read: usize,
}

impl PersistentFrameReader {
    /// The next whole frame, or None when the stream has no more of it for
    /// now; what was read of it is kept for the next call.
    fn read_next(
        &mut self,
        stream: &mut impl Read,
        max_frame_bytes: u32,
    ) -> io::Result<Option<Vec<u8>>> {
        if !fill_from(stream, &mut self.length, &mut self.length_read)? {
            return Ok(None);
        }
        if self.payload.is_empty() && self.payload_read == 0 {
            let length = checked_frame_length(self.length, max_frame_bytes)?;
            self.payload.resize(length, 0);
        }
        if !fill_from(stream, &mut self.payload, &mut self.payload_read)? {
            return Ok(None);
        }
        let frame = std::mem::take(&mut self.payload);
        self.reset();
        Ok(Some(frame))
    }

    fn reset(&mut self) {
        self.length = [0; 4];
        self.length_read = 0;
        self.payload.clear();
        self.payload_read = 0;
    }
}

/// Reads into `buffer` past its first `*filled` bytes until it is full
/// (true) or the stream has nothing more for now (false).
fn fill_from(stream: &mut impl Read, buffer: &mut [u8], filled: &mut usize) -> io::Result<bool> {
    while *filled < buffer.len() {
        match stream.read(&mut buffer[*filled..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => *filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

/// The payload length a frame's prefix gives, if the reader takes frames
/// that long.
fn checked_frame_length(prefix: [u8; 4], max_frame_bytes: u32) -> io::Result<usize> {
    let length = u32::from_le_bytes(prefix);
    let maximum = max_frame_bytes.min(MAX_DIST_MSG);
    if length > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("dist message too large: {length} bytes (max {maximum})"),
        ));
    }
    Ok(length as usize)
}

fn read_dist_msg_bounded(stream: &mut impl Read, max_frame_bytes: u32) -> io::Result<Vec<u8>> {
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix)?;
    let mut buf = vec![0u8; checked_frame_length(prefix, max_frame_bytes)?];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// TLS channel binding for the cookie handshake
// ---------------------------------------------------------------------------

/// RFC 9266 `tls-exporter` channel binding of the TLS session that carries
/// the cookie handshake.
///
/// Legacy (non-mTLS) node TLS does not verify certificates, so on its own the
/// cookie proof would authenticate whoever is at the far end of *some* TLS
/// session: a relay that terminates TLS towards both endpoints could forward
/// the four handshake messages unchanged and then read or modify distribution
/// traffic. Mixing this exporter into every HMAC ties each proof to the exact
/// TLS session it was sent over. A relay sees two different bindings, so the
/// proofs it forwards fail verification at both endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChannelBinding([u8; 32]);

/// Exporter label defined by RFC 9266 for the `tls-exporter` channel binding.
const TLS_EXPORTER_CHANNEL_BINDING_LABEL: &[u8] = b"EXPORTER-Channel-Binding";

/// Transport the cookie handshake runs over.
///
/// The transport must be able to finish its own security handshake and hand
/// out a channel binding before the first cookie message is exchanged.
trait HandshakeTransport: Read + Write {
    fn channel_binding(&mut self) -> Result<ChannelBinding, String>;
}

impl<C, T, S> HandshakeTransport for StreamOwned<C, T>
where
    C: DerefMut + Deref<Target = rustls::ConnectionCommon<S>>,
    T: Read + Write,
    S: rustls::SideData,
{
    /// Drive the TLS handshake to completion (rustls otherwise completes it
    /// lazily on the first read or write), then export the RFC 9266 binding.
    fn channel_binding(&mut self) -> Result<ChannelBinding, String> {
        while self.conn.is_handshaking() {
            let (read, written) = self
                .conn
                .complete_io(&mut self.sock)
                .map_err(|error| format!("tls_handshake_failed:{error}"))?;
            if read == 0 && written == 0 && self.conn.is_handshaking() {
                return Err("tls_handshake_stalled".to_string());
            }
        }
        let exported = self
            .conn
            .export_keying_material([0u8; 32], TLS_EXPORTER_CHANNEL_BINDING_LABEL, Some(&[]))
            .map_err(|error| format!("tls_channel_binding_unavailable:{error}"))?;
        Ok(ChannelBinding(exported))
    }
}

/// In-process handshake tests pair plain Unix sockets without TLS; both ends
/// share a fixed binding so the cookie exchange itself can be exercised.
#[cfg(test)]
impl HandshakeTransport for std::os::unix::net::UnixStream {
    fn channel_binding(&mut self) -> Result<ChannelBinding, String> {
        Ok(ChannelBinding::TEST_PLAIN_TRANSPORT)
    }
}

#[cfg(test)]
impl ChannelBinding {
    const TEST_PLAIN_TRANSPORT: Self = Self([0x5A; 32]);
}

// ---------------------------------------------------------------------------
// HMAC-SHA256 challenge/response functions
// ---------------------------------------------------------------------------

/// Generate a 32-byte random challenge.
fn generate_challenge() -> [u8; 32] {
    rand::random()
}

/// Compute HMAC-SHA256(cookie, challenge || channel_binding) as the challenge
/// response, proving knowledge of the cookie for this TLS session only.
///
/// Follows the pattern from `db/pg.rs` SCRAM-SHA-256 authentication.
fn cluster_cookie_keys(cookie: &str) -> impl Iterator<Item = &str> {
    cookie
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

fn validate_cluster_cookie_strength(cookie: &str, autonomous: bool) -> Result<(), String> {
    let keys = cluster_cookie_keys(cookie).collect::<Vec<_>>();
    if keys.is_empty() {
        return Err("cluster_cookie_missing".to_string());
    }
    if autonomous && keys.iter().any(|key| key.len() < 32) {
        return Err("autonomous_cluster_cookie_too_short".to_string());
    }
    Ok(())
}

fn compute_response(
    cookie: &str,
    challenge: &[u8; 32],
    channel_binding: &ChannelBinding,
) -> [u8; 32] {
    let signing_key = cluster_cookie_keys(cookie).next().unwrap_or(cookie);
    let mut mac =
        HmacSha256::new_from_slice(signing_key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(challenge);
    mac.update(&channel_binding.0);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Verify a challenge response using constant-time comparison.
///
/// The response must have been computed over the same challenge *and* the
/// same TLS channel binding, so a proof relayed from another TLS session is
/// rejected even though the peer knows the cookie.
///
/// Uses `Mac::verify_slice` for constant-time comparison, preventing
/// timing attacks (research pitfall 3).
fn verify_response(
    cookie: &str,
    challenge: &[u8; 32],
    channel_binding: &ChannelBinding,
    response: &[u8; 32],
) -> bool {
    cluster_cookie_keys(cookie).any(|key| {
        let mut mac =
            HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
        mac.update(challenge);
        mac.update(&channel_binding.0);
        mac.verify_slice(response).is_ok()
    })
}

// ---------------------------------------------------------------------------
// Handshake message builders and parsers
// ---------------------------------------------------------------------------

/// Sends a handshake message that names its sender: `[tag][u16 name_len]
/// [name][u8 creation][extra][protocol hello]`. NAME carries no extra bytes,
/// CHALLENGE its 32-byte challenge.
fn send_named(
    stream: &mut impl Write,
    tag: u8,
    name: &str,
    creation: u8,
    extra: &[u8],
) -> Result<(), String> {
    let hello = local_protocol_hello_with_identity(name, process_env)?.encode()?;
    let mut payload = vec![tag];
    payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload.push(creation);
    payload.extend_from_slice(extra);
    payload.extend_from_slice(&hello);
    send_handshake(stream, &payload)
}

fn send_handshake(stream: &mut impl Write, payload: &[u8]) -> Result<(), String> {
    write_msg(stream, payload)
        .map_err(|error| format!("handshake message {} not sent: {error}", payload[0]))
}

/// The next handshake message, which must be a `tag` one of at least
/// `min_len` bytes.
fn recv_handshake(stream: &mut impl Read, tag: u8, min_len: usize) -> Result<Vec<u8>, String> {
    let msg = read_msg(stream)
        .map_err(|error| format!("handshake message {tag} not received: {error}"))?;
    if msg.first() != Some(&tag) {
        return Err(format!(
            "expected handshake message {tag}, got {}",
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < min_len {
        return Err(format!("handshake message {tag} too short"));
    }
    Ok(msg)
}

/// A message `send_named` sent: its sender's name and creation, the `EXTRA`
/// bytes it carries, and the sender's protocol hello (a protocol-one peer's,
/// which sends none, when there is none).
fn recv_named<const EXTRA: usize>(
    stream: &mut impl Read,
    tag: u8,
) -> Result<(String, u8, [u8; EXTRA], ProtocolHello), String> {
    let msg = recv_handshake(stream, tag, 4)?;
    let name_len = u16::from_le_bytes([msg[1], msg[2]]) as usize;
    let rest = msg
        .get(3 + name_len + 1..)
        .filter(|rest| rest.len() >= EXTRA)
        .ok_or_else(|| format!("handshake message {tag} truncated"))?;
    let name = std::str::from_utf8(&msg[3..3 + name_len])
        .map_err(|_| "invalid UTF-8 in node name".to_string())?
        .to_string();
    let hello = match &rest[EXTRA..] {
        [] => protocol_one_hello(),
        hello => ProtocolHello::decode(hello)?,
    };
    Ok((
        name,
        msg[3 + name_len],
        rest[..EXTRA].try_into().unwrap(),
        hello,
    ))
}

/// Sends REPLY: `[tag=3][32 bytes response][32 bytes own challenge]`.
fn send_challenge_reply(
    stream: &mut impl Write,
    response: &[u8; 32],
    own_challenge: &[u8; 32],
) -> Result<(), String> {
    send_handshake(
        stream,
        &[&[HANDSHAKE_REPLY][..], response, own_challenge].concat(),
    )
}

/// Receives REPLY: the peer's response and its own challenge.
fn recv_challenge_reply(stream: &mut impl Read) -> Result<([u8; 32], [u8; 32]), String> {
    let msg = recv_handshake(stream, HANDSHAKE_REPLY, 1 + 32 + 32)?;
    Ok((
        msg[1..33].try_into().unwrap(),
        msg[33..65].try_into().unwrap(),
    ))
}

/// Sends ACK: `[tag=4][32 bytes response]`.
fn send_challenge_ack(stream: &mut impl Write, response: &[u8; 32]) -> Result<(), String> {
    send_handshake(stream, &[&[HANDSHAKE_ACK][..], response].concat())
}

/// Receives ACK: the peer's response.
fn recv_challenge_ack(stream: &mut impl Read) -> Result<[u8; 32], String> {
    let msg = recv_handshake(stream, HANDSHAKE_ACK, 1 + 32)?;
    Ok(msg[1..33].try_into().unwrap())
}

/// Checks the peer's `response` to `challenge`, over this TLS session.
fn authenticate_peer(
    cookie: &str,
    challenge: &[u8; 32],
    channel_binding: &ChannelBinding,
    response: &[u8; 32],
    remote_name: &str,
) -> Result<(), String> {
    if verify_response(cookie, challenge, channel_binding, response) {
        Ok(())
    } else {
        Err(format!(
            "cookie mismatch: authentication failed from {remote_name}"
        ))
    }
}

// ---------------------------------------------------------------------------
// validate_advertised_node_name -- preserve membership truth from handshake
// ---------------------------------------------------------------------------

fn validate_advertised_node_name(name: &str) -> Result<(), String> {
    parse_node_name(name)
        .map(|_| ())
        .map_err(|err| format!("invalid remote node name: {}", err))
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
/// Every response is `HMAC(cookie, challenge || tls-exporter)`, so it is only
/// valid on the TLS session it was produced for (see [`ChannelBinding`]).
///
/// Returns `(remote_name, remote_creation)` on success, or an error string.
fn validate_remote_node_identity(
    remote_name: &str,
    remote_hello: &ProtocolHello,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<super::identity_claim::NodeIdentityClaim>, String> {
    if remote_hello.identity_envelope.is_empty() {
        return if autonomous_mode_requested() {
            Err("autonomous_peer_missing_signed_identity".to_string())
        } else {
            Ok(None)
        };
    }
    let cluster_id =
        env("MESH_CLUSTER_ID").ok_or_else(|| "node_identity_cluster_missing".to_string())?;
    let verify_keys = env(super::identity_claim::IDENTITY_VERIFY_KEYS_ENV)
        .ok_or_else(|| "node_identity_verify_keys_missing".to_string())?;
    let claim = super::identity_claim::decode_and_verify_identity(
        &remote_hello.identity_envelope,
        &verify_keys,
        &cluster_id,
        remote_name,
        super::identity_claim::unix_millis(),
    )?;
    authorize_node_identity(
        remote_name,
        &claim,
        &env("MESH_CONTROLLER_VOTERS").unwrap_or_default(),
    )?;
    Ok(Some(claim))
}

/// Whether a verified identity may take the channel it came in on, given
/// the controller voters (`stable_id|name`, comma separated): a controller
/// only under the voter name bound to its stable id (as its advertised
/// name when it comes in as a transient operator client), no other role
/// under a voter's name, and an operator only as a transient operator
/// client, which nothing but an operator or a controller may be.
fn authorize_node_identity(
    remote_name: &str,
    claim: &super::identity_claim::NodeIdentityClaim,
    voters: &str,
) -> Result<(), String> {
    let has_role = |wanted: &str| claim.roles.iter().any(|role| role == wanted);
    let (controller, operator) = (has_role("controller"), has_role("operator"));
    let transient_operator = is_transient_operator_client(remote_name);
    let authenticated_name = if transient_operator && controller {
        claim.advertised_name.as_str()
    } else {
        remote_name
    };
    let mut voters = voters
        .split(',')
        .filter_map(|entry| entry.trim().split_once('|'));
    if controller {
        if !voters.any(|(stable_id, name)| {
            stable_id == claim.stable_node_id && name == authenticated_name
        }) {
            return Err("controller_identity_not_bound_to_voter".to_string());
        }
    } else if voters.any(|(_, name)| name == authenticated_name) {
        return Err("non_controller_claimed_voter_name".to_string());
    }
    if (operator && !transient_operator) || (transient_operator && !operator && !controller) {
        return Err("operator_identity_channel_mismatch".to_string());
    }
    Ok(())
}

fn perform_handshake_with_identity(
    stream: &mut impl HandshakeTransport,
    local_name: &str,
    local_cookie: &str,
    local_creation: u8,
    is_initiator: bool,
) -> Result<Authenticated, String> {
    // Finish the TLS handshake first so every cookie proof below is bound to
    // this exact TLS session rather than to whoever relays the messages.
    let channel_binding = stream.channel_binding()?;

    let (remote_name, remote_creation, remote_hello) = if is_initiator {
        // Our name, then their name and challenge.
        send_named(stream, HANDSHAKE_NAME, local_name, local_creation, &[])?;
        let (remote_name, remote_creation, their_challenge, remote_hello) =
            recv_named::<32>(stream, HANDSHAKE_CHALLENGE)?;
        validate_advertised_node_name(&remote_name)?;
        // Our response and challenge, then their response.
        let our_challenge = generate_challenge();
        let our_response = compute_response(local_cookie, &their_challenge, &channel_binding);
        send_challenge_reply(stream, &our_response, &our_challenge)?;
        let their_response = recv_challenge_ack(stream)?;
        authenticate_peer(
            local_cookie,
            &our_challenge,
            &channel_binding,
            &their_response,
            &remote_name,
        )?;
        (remote_name, remote_creation, remote_hello)
    } else {
        // Their name, then ours and our challenge.
        let (remote_name, remote_creation, [], remote_hello) =
            recv_named::<0>(stream, HANDSHAKE_NAME)?;
        validate_advertised_node_name(&remote_name)?;

        // Duplicate-session resolution now happens in register_session after the
        // authenticated stream is fully built. Do not reject same-name reconnects
        // mid-handshake here; stale-session takeover and simultaneous connect both
        // rely on the later registration step being able to replace the old entry.

        let our_challenge = generate_challenge();
        send_named(
            stream,
            HANDSHAKE_CHALLENGE,
            local_name,
            local_creation,
            &our_challenge,
        )?;
        // Their response and challenge, then our response.
        let (their_response, their_challenge) = recv_challenge_reply(stream)?;
        authenticate_peer(
            local_cookie,
            &our_challenge,
            &channel_binding,
            &their_response,
            &remote_name,
        )?;
        let our_response = compute_response(local_cookie, &their_challenge, &channel_binding);
        send_challenge_ack(stream, &our_response)?;
        (remote_name, remote_creation, remote_hello)
    };
    let negotiated = negotiate_protocol(&local_protocol_hello(), &remote_hello)?;
    let identity = validate_remote_node_identity(&remote_name, &remote_hello, process_env)?;
    Ok((remote_name, remote_creation, negotiated, identity))
}

fn perform_handshake_negotiated(
    stream: &mut impl HandshakeTransport,
    state: &NodeState,
    is_initiator: bool,
) -> Result<Authenticated, String> {
    perform_handshake_with_identity(
        stream,
        &state.name,
        &state.cookie,
        state.creation(),
        is_initiator,
    )
}

#[cfg(test)]
fn perform_handshake(
    stream: &mut impl HandshakeTransport,
    state: &NodeState,
    is_initiator: bool,
) -> Result<(String, u8), String> {
    perform_handshake_negotiated(stream, state, is_initiator)
        .map(|(name, creation, _, _)| (name, creation))
}

// ---------------------------------------------------------------------------
// register_session -- inserts authenticated session into NodeState
// ---------------------------------------------------------------------------

fn preferred_session_direction(local_name: &str, remote_name: &str) -> SessionDirection {
    if local_name < remote_name {
        SessionDirection::Outgoing
    } else {
        SessionDirection::Incoming
    }
}

/// Register an authenticated session in `NodeState`.
///
/// Duplicate connects are resolved deterministically so both nodes keep the
/// same underlying transport. If both sides connect simultaneously, the node
/// whose name sorts earlier keeps the outgoing side while the later-sorting
/// node keeps the incoming side.
fn register_session(
    state: &NodeState,
    remote_name: String,
    remote_creation: u8,
    node_id: u16,
    stream: NodeStream,
    negotiated_protocol: NegotiatedProtocol,
    remote_identity: Option<super::identity_claim::NodeIdentityClaim>,
) -> Result<Arc<NodeSession>, String> {
    let direction = SessionDirection::from_stream(&stream);
    let preferred_direction = preferred_session_direction(&state.name, &remote_name);
    let session = Arc::new(NodeSession::new(
        RemoteSessionEndpoint {
            remote_name: remote_name.clone(),
            remote_creation,
            node_id,
            direction,
        },
        stream,
        true,
        negotiated_protocol,
        remote_identity,
    ));

    let mut replaced_node_id = None;
    {
        let mut sessions = state.sessions.write();
        match sessions.get(&remote_name).cloned() {
            Some(existing) => {
                let replace_existing = existing.shutdown.load(Ordering::SeqCst)
                    || (existing.direction != preferred_direction
                        && direction == preferred_direction);
                if !replace_existing {
                    return Err(format!("already_connected:{}", remote_name));
                }
                let replaced = sessions
                    .remove(&remote_name)
                    .expect("duplicate session missing during replacement");
                replaced.shutdown.store(true, Ordering::SeqCst);
                replaced_node_id = Some(replaced.node_id);
                sessions.insert(remote_name.clone(), Arc::clone(&session));
            }
            None => {
                sessions.insert(remote_name.clone(), Arc::clone(&session));
            }
        }
    }

    let mut id_map = state.node_id_map.write();
    if let Some(previous_node_id) = replaced_node_id {
        id_map.remove(&previous_node_id);
    }
    id_map.insert(node_id, remote_name.clone());
    drop(id_map);

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
    let pkcs8_bytes =
        EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
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

    let key_der = PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
        pkcs8_bytes.as_ref().to_vec(),
    ));

    (CertificateDer::from(cert_der), key_der)
}

/// Build the TBS (To-Be-Signed) Certificate portion of an X.509 v3 cert.
///
/// This is a minimal ASN.1 DER structure:
/// - Version: v3
/// - Serial: 1
/// - Signature algorithm: ECDSA with SHA-256
/// - Issuer: CN=mesh-node
/// - Validity: 2020-01-01 to 2099-12-31 (effectively forever)
/// - Subject: CN=mesh-node
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

    // issuer: RDNSequence with CN=mesh-node
    let issuer = build_dn(b"mesh-node");

    // validity: NotBefore 2020-01-01, NotAfter 2099-12-31
    let not_before = der_utc_time(b"200101000000Z");
    let not_after = der_utc_time(b"991231235959Z");
    let validity = der_sequence(&[&not_before, &not_after]);

    // subject: same as issuer
    let subject = build_dn(b"mesh-node");

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
/// by the HMAC-SHA256 cookie challenge/response (Plan 02), not by PKI. The
/// cookie proofs are bound to the TLS session's exporter (RFC 9266), which is
/// what prevents a certificate-less relay from splicing two sessions together.
fn build_node_client_config() -> Arc<ClientConfig> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipCertVerification))
        .with_no_client_auth();
    Arc::new(config)
}

#[cfg(test)]
pub(crate) fn ws_test_tls_configs() -> (Arc<ServerConfig>, Arc<ClientConfig>) {
    let (certificate, key) = generate_ephemeral_cert();
    (
        build_node_server_config(certificate, key),
        build_node_client_config(),
    )
}

const TLS_CA_DER_B64_ENV: &str = "MESH_TLS_CA_DER_B64";
const TLS_CERT_DER_B64_ENV: &str = "MESH_TLS_CERT_DER_B64";
const TLS_KEY_DER_B64_ENV: &str = "MESH_TLS_KEY_DER_B64";

#[cfg(test)]
thread_local! {
    /// What `autonomous_mode_requested` answers on this thread, when set: a
    /// test of an autonomous path cannot set the process's environment
    /// under the tests running beside it.
    pub(crate) static AUTONOMOUS_ON_THIS_THREAD: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Runs `body` with this thread's node in autonomous mode.
#[cfg(test)]
pub(crate) fn in_autonomous_mode<T>(body: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AUTONOMOUS_ON_THIS_THREAD.with(|mode| mode.set(None));
        }
    }
    AUTONOMOUS_ON_THIS_THREAD.with(|mode| mode.set(Some(true)));
    let _reset = Reset;
    body()
}

/// Whether this node runs in autonomous mode: `MESH_CLUSTER_MODE=autonomous`,
/// the legacy `MESH_AUTONOMOUS_MODE`, or an embedded manifest that enables it.
/// Every part of the runtime asks here, so they agree.
pub(crate) fn autonomous_mode_requested() -> bool {
    #[cfg(test)]
    if let Some(forced) = AUTONOMOUS_ON_THIS_THREAD.with(std::cell::Cell::get) {
        return forced;
    }
    std::env::var("MESH_CLUSTER_MODE")
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("autonomous"))
        || std::env::var("MESH_AUTONOMOUS_MODE")
            .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "on"))
        || super::autonomous::embedded_autonomous_config()
            .is_some_and(|config| config.enabled && config.features.protocol_two)
}

fn decode_tls_der(name: &str, value: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|_| format!("{name}_invalid_base64"))
        .and_then(|bytes| {
            if bytes.is_empty() {
                Err(format!("{name}_empty"))
            } else {
                Ok(bytes)
            }
        })
}

/// The mTLS identity the environment configures: the CA certificates, the
/// node's certificate and its key, each DER in base64 (the CAs
/// comma-separated).
fn configured_mtls_values() -> MtlsValues {
    [
        std::env::var(TLS_CA_DER_B64_ENV).ok(),
        std::env::var(TLS_CERT_DER_B64_ENV).ok(),
        std::env::var(TLS_KEY_DER_B64_ENV).ok(),
    ]
}

type MtlsValues = [Option<String>; 3];

fn mtls_material(
    values: &MtlsValues,
) -> Result<
    Option<(
        Vec<CertificateDer<'static>>,
        CertificateDer<'static>,
        PrivateKeyDer<'static>,
    )>,
    String,
> {
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    if values.iter().any(Option::is_none) {
        return Err("mesh_mtls_configuration_incomplete".to_string());
    }
    let ca = values[0]
        .as_deref()
        .unwrap()
        .split(',')
        .map(str::trim)
        .map(|value| decode_tls_der(TLS_CA_DER_B64_ENV, value).map(CertificateDer::from))
        .collect::<Result<Vec<_>, _>>()?;
    let cert = decode_tls_der(TLS_CERT_DER_B64_ENV, values[1].as_deref().unwrap())?;
    let key = decode_tls_der(TLS_KEY_DER_B64_ENV, values[2].as_deref().unwrap())?;
    Ok(Some((
        ca,
        CertificateDer::from(cert),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
    )))
}

type ConfiguredMtls = (Arc<ServerConfig>, Arc<ClientConfig>);

fn mtls_configs(values: &MtlsValues) -> Result<Option<ConfiguredMtls>, String> {
    let Some((cas, certificate, private_key)) = mtls_material(values)? else {
        return Ok(None);
    };
    let mut roots = RootCertStore::empty();
    for ca in cas {
        roots
            .add(ca)
            .map_err(|error| format!("mesh_mtls_ca_invalid:{error}"))?;
    }
    let client_verifier = WebPkiClientVerifier::builder(Arc::new(roots.clone()))
        .build()
        .map_err(|error| format!("mesh_mtls_client_verifier_invalid:{error}"))?;
    let server = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(vec![certificate.clone()], private_key.clone_key())
        .map_err(|error| format!("mesh_mtls_server_identity_invalid:{error}"))?;
    let client = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![certificate], private_key)
        .map_err(|error| format!("mesh_mtls_client_identity_invalid:{error}"))?;
    Ok(Some((Arc::new(server), Arc::new(client))))
}

/// A node's TLS: the mTLS identity `mtls` configures, or else (not in
/// autonomous mode, which requires one) an ephemeral certificate.
fn node_tls_configs(mtls: &MtlsValues) -> Result<ConfiguredMtls, String> {
    if let Some(configs) = mtls_configs(mtls)? {
        return Ok(configs);
    }
    if autonomous_mode_requested() {
        return Err("autonomous_mode_requires_mtls_identity".to_string());
    }
    let (certificate, key) = generate_ephemeral_cert();
    Ok((
        build_node_server_config(certificate, key),
        build_node_client_config(),
    ))
}

fn operator_tls_client_config() -> Result<Arc<ClientConfig>, String> {
    Ok(mtls_configs(&configured_mtls_values())?
        .map(|(_, client)| client)
        .unwrap_or_else(build_node_client_config))
}

// ---------------------------------------------------------------------------
// SkipCertVerification -- trusts all server certificates
// ---------------------------------------------------------------------------

/// A `ServerCertVerifier` that accepts any certificate without validation.
///
/// This is intentional: inter-node TLS provides encryption and integrity,
/// while authentication is handled by the HMAC-SHA256 cookie challenge
/// that runs after the TLS handshake completes. Because the certificate is
/// unauthenticated, the cookie proofs are bound to the TLS exporter of the
/// session (see [`ChannelBinding`]) rather than trusting the transport alone.
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
/// - `"name@[ipv6]"` -> (name, ipv6, 9000)  (default port)
/// - `"name@[ipv6]:port"` -> (name, ipv6, parsed_port)
///
/// Returns `Err` for invalid formats (no @, empty parts, invalid port).
pub fn parse_node_name(name: &str) -> Result<(&str, &str, u16), String> {
    super::discovery::split_node_name(name, false)
}

const TRANSIENT_OPERATOR_CLIENT_NAME_PART: &str = "mesh-operator-query";

fn transient_operator_client_name() -> String {
    format!("{TRANSIENT_OPERATOR_CLIENT_NAME_PART}@127.0.0.1:1")
}

fn is_transient_operator_client(remote_name: &str) -> bool {
    remote_name.starts_with(&format!("{TRANSIENT_OPERATOR_CLIENT_NAME_PART}@"))
}

pub(crate) fn handle_transient_operator_query_connection(
    remote_name: String,
    remote_creation: u8,
    stream: NodeStream,
    timeout: Duration,
    negotiated_protocol: NegotiatedProtocol,
    remote_identity: Option<super::identity_claim::NodeIdentityClaim>,
) -> Result<(), String> {
    let direction = SessionDirection::from_stream(&stream);
    let session = Arc::new(NodeSession::new(
        RemoteSessionEndpoint {
            remote_name,
            remote_creation,
            node_id: 0,
            direction,
        },
        stream,
        false,
        negotiated_protocol,
        remote_identity,
    ));

    {
        let stream = session.stream.lock();
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("transient_operator_timeout_set_failed:{error}"))?;
    }

    let msg = {
        let mut stream = session.stream.lock();
        read_dist_msg(&mut *stream)
            .map_err(|error| format!("transient_operator_read_failed:{error}"))?
    };

    if msg.is_empty() {
        return Err("transient_operator_query_empty".to_string());
    }
    if msg[0] != DIST_OPERATOR_QUERY {
        return Err(format!(
            "transient_operator_query_unexpected_tag:{}",
            msg[0]
        ));
    }

    crate::dist::operator::handle_operator_query_message(&session, &msg);
    Ok(())
}

pub(crate) fn execute_transient_operator_query(
    target: &str,
    cookie: &str,
    payload: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let (mut tls_stream, (_, _, negotiated, _)) = connect_authenticated(
        target,
        operator_tls_client_config()?,
        &transient_operator_client_name(),
        cookie,
        0,
        timeout,
    )?;
    write_msg(&mut tls_stream, payload)
        .map_err(|e| format!("transient_operator_query_write_failed:{e}"))?;
    read_dist_msg_bounded(&mut tls_stream, negotiated.max_frame_bytes)
        .map_err(|e| format!("transient_operator_reply_read_failed:{e}"))
}

const CLUSTERED_HTTP_ROUTE_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_RESERVATION_TIMEOUT: Duration = Duration::from_secs(3);
// The lease starts on the owner before the acceptance reply crosses the
// transport. It must outlive the ingress's complete post-acceptance route
// timeout while remaining bounded against clients that never send a query.
const HTTP_RESERVATION_LEASE: Duration = Duration::from_secs(10);

fn encode_http_route_string(payload: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let len = u16::try_from(value.len())
        .map_err(|_| format!("clustered_http_route_string_too_large:{}", value.len()))?;
    payload.extend_from_slice(&len.to_le_bytes());
    payload.extend_from_slice(value.as_bytes());
    Ok(())
}

fn decode_http_route_string(data: &[u8], pos: &mut usize, label: &str) -> Result<String, String> {
    if *pos + 2 > data.len() {
        return Err(format!("clustered_http_route_{}_len_missing", label));
    }
    let len = u16::from_le_bytes(data[*pos..*pos + 2].try_into().unwrap()) as usize;
    *pos += 2;
    if *pos + len > data.len() {
        return Err(format!("clustered_http_route_{}_truncated", label));
    }
    let value = std::str::from_utf8(&data[*pos..*pos + len])
        .map_err(|_| format!("clustered_http_route_{}_invalid_utf8", label))?
        .to_string();
    *pos += len;
    Ok(value)
}

fn encode_http_route_v2_query_frame(
    correlation_id: u64,
    runtime_name: &str,
    request_key: &str,
    attempt_id: &str,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    let mut frame = vec![DIST_HTTP_ROUTE_V2_QUERY];
    frame.extend_from_slice(&correlation_id.to_le_bytes());
    encode_http_route_string(&mut frame, runtime_name)?;
    encode_http_route_string(&mut frame, request_key)?;
    encode_http_route_string(&mut frame, attempt_id)?;
    frame.extend_from_slice(&request_payload_len(request_payload).to_le_bytes());
    frame.extend_from_slice(request_payload);
    Ok(frame)
}

/// A query `encode_http_route_v2_query_frame` made: its correlation id,
/// runtime name, request key, attempt id and request.
type HttpRouteQuery = (u64, String, String, String, Vec<u8>);

fn decode_http_route_v2_query_frame(data: &[u8]) -> Result<HttpRouteQuery, String> {
    if data.len() < 9 || data[0] != DIST_HTTP_ROUTE_V2_QUERY {
        return Err("clustered_http_route_v2_query_invalid".to_string());
    }
    let correlation_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let mut pos = 9usize;
    let runtime_name = decode_http_route_string(data, &mut pos, "runtime_name")?;
    let request_key = decode_http_route_string(data, &mut pos, "request_key")?;
    let attempt_id = decode_http_route_string(data, &mut pos, "attempt_id")?;
    if pos + 4 > data.len() {
        return Err("clustered_http_route_payload_len_missing".to_string());
    }
    let payload_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;
    if pos + payload_len != data.len() {
        return Err("clustered_http_route_payload_length_mismatch".to_string());
    }
    Ok((
        correlation_id,
        runtime_name,
        request_key,
        attempt_id,
        data[pos..].to_vec(),
    ))
}

fn encode_http_route_v2_reply_frame(
    correlation_id: u64,
    result: Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let (status, payload) = match result {
        Ok(response_payload) => (0u8, response_payload),
        Err(reason) => (1u8, reason.into_bytes()),
    };
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| format!("clustered_http_route_reply_too_large:{}", payload.len()))?;
    let mut frame = Vec::with_capacity(1 + 8 + 1 + 4 + payload.len());
    frame.push(DIST_HTTP_ROUTE_V2_REPLY);
    frame.extend_from_slice(&correlation_id.to_le_bytes());
    frame.push(status);
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_http_route_v2_reply_frame(data: &[u8]) -> Result<(u64, Result<Vec<u8>, String>), String> {
    if data.len() < 14 || data[0] != DIST_HTTP_ROUTE_V2_REPLY {
        return Err("clustered_http_route_v2_reply_invalid".to_string());
    }
    let correlation_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let status = data[9];
    let payload_len = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    if data.len() != 14 + payload_len {
        return Err("clustered_http_route_reply_length_mismatch".to_string());
    }
    let payload = &data[14..];
    let result = match status {
        0 => Ok(payload.to_vec()),
        1 => Err(std::str::from_utf8(payload)
            .map_err(|_| "clustered_http_route_reply_reason_invalid_utf8".to_string())?
            .to_string()),
        other => return Err(format!("invalid_clustered_http_route_reply_status:{other}")),
    };
    Ok((correlation_id, result))
}

struct AcceptedHttpReservation {
    _permit: crate::dist::telemetry::AdmissionPermit,
    expires_at: Instant,
}

/// A clustered request's length, as its frames carry it: the request
/// is one the HTTP server read, its body within 1 MiB and its head within
/// 8 KiB, or one recovered from a record that came in a frame within
/// 16 MiB, so it always fits.
fn request_payload_len(request_payload: &[u8]) -> u32 {
    request_payload.len() as u32
}

fn encode_http_reserve(
    correlation_id: u64,
    runtime_name: &str,
    request_key: &str,
    payload_bytes: u32,
) -> Result<Vec<u8>, String> {
    let mut frame = Vec::with_capacity(1 + 8 + 4 + 2 + runtime_name.len() + 2 + request_key.len());
    frame.push(DIST_HTTP_RESERVE);
    frame.extend_from_slice(&correlation_id.to_le_bytes());
    frame.extend_from_slice(&payload_bytes.to_le_bytes());
    encode_http_route_string(&mut frame, runtime_name)?;
    encode_http_route_string(&mut frame, request_key)?;
    Ok(frame)
}

fn decode_http_reserve(frame: &[u8]) -> Result<(u64, u32, String, String), String> {
    if frame.len() < 13 || frame[0] != DIST_HTTP_RESERVE {
        return Err("clustered_http_reservation_invalid".to_string());
    }
    let correlation_id = u64::from_le_bytes(frame[1..9].try_into().unwrap());
    let payload_bytes = u32::from_le_bytes(frame[9..13].try_into().unwrap());
    let mut position = 13;
    let runtime_name = decode_http_route_string(frame, &mut position, "reservation_runtime")?;
    let request_key = decode_http_route_string(frame, &mut position, "reservation_key")?;
    if position != frame.len() || runtime_name.is_empty() || request_key.is_empty() {
        return Err("clustered_http_reservation_metadata_invalid".to_string());
    }
    Ok((correlation_id, payload_bytes, runtime_name, request_key))
}

fn encode_http_reserve_reply(
    correlation_id: u64,
    result: Result<(), String>,
) -> Result<Vec<u8>, String> {
    let (accepted, reason) = match result {
        Ok(()) => (1u8, Vec::new()),
        Err(reason) => (0u8, reason.into_bytes()),
    };
    let reason_len = u16::try_from(reason.len())
        .map_err(|_| "clustered_http_reservation_reason_too_large".to_string())?;
    let mut frame = Vec::with_capacity(12 + reason.len());
    frame.push(DIST_HTTP_RESERVE_REPLY);
    frame.extend_from_slice(&correlation_id.to_le_bytes());
    frame.push(accepted);
    frame.extend_from_slice(&reason_len.to_le_bytes());
    frame.extend_from_slice(&reason);
    Ok(frame)
}

fn decode_http_reserve_reply(frame: &[u8]) -> Result<(u64, Result<(), String>), String> {
    if frame.len() < 12 || frame[0] != DIST_HTTP_RESERVE_REPLY {
        return Err("clustered_http_reservation_reply_invalid".to_string());
    }
    let correlation_id = u64::from_le_bytes(frame[1..9].try_into().unwrap());
    let accepted = frame[9];
    let reason_len = u16::from_le_bytes(frame[10..12].try_into().unwrap()) as usize;
    if frame.len() != 12 + reason_len {
        return Err("clustered_http_reservation_reply_length_invalid".to_string());
    }
    match accepted {
        1 if reason_len == 0 => Ok((correlation_id, Ok(()))),
        0 => Ok((
            correlation_id,
            Err(std::str::from_utf8(&frame[12..])
                .map_err(|_| "clustered_http_reservation_reason_invalid".to_string())?
                .to_string()),
        )),
        _ => Err("clustered_http_reservation_reply_status_invalid".to_string()),
    }
}

fn expire_http_reservation_map(
    reservations: &mut FxHashMap<u64, AcceptedHttpReservation>,
    now: Instant,
) {
    reservations.retain(|_, reservation| reservation.expires_at > now);
}

fn expire_http_reservations(session: &NodeSession, now: Instant) {
    expire_http_reservation_map(&mut session.accepted_http_reservations.lock().unwrap(), now);
}

fn handle_http_reserve(session: &Arc<NodeSession>, frame: &[u8]) {
    let decoded = decode_http_reserve(frame);
    let (correlation_id, result) = match decoded {
        Ok((correlation_id, payload_bytes, runtime_name, _request_key)) => {
            expire_http_reservations(session, Instant::now());
            let result = if payload_bytes as usize > MAX_DIST_MSG as usize {
                Err("owner_reservation_payload_limit".to_string())
            } else if lookup_declared_handler(&runtime_name).is_none() {
                Err(format!("declared_handler_not_registered:{runtime_name}"))
            } else {
                let mut reservations = session.accepted_http_reservations.lock().unwrap();
                match reservations.entry(correlation_id) {
                    std::collections::hash_map::Entry::Occupied(_) => Ok(()),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        crate::dist::telemetry::global_admission_controller()
                            .reserve_application()
                            .map(|permit| {
                                entry.insert(AcceptedHttpReservation {
                                    _permit: permit,
                                    expires_at: Instant::now() + HTTP_RESERVATION_LEASE,
                                });
                            })
                            .map_err(|rejection| {
                                format!("owner_reservation_rejected:{rejection:?}")
                            })
                    }
                }
            };
            (correlation_id, result)
        }
        Err(error) => (0, Err(error)),
    };
    if let Ok(reply) = encode_http_reserve_reply(correlation_id, result) {
        if session.send(OutboundClass::Admission, reply).is_err() {
            session
                .accepted_http_reservations
                .lock()
                .unwrap()
                .remove(&correlation_id);
        }
    }
}

fn encode_continuity_response_frame(
    operation_key: &str,
    response: &[u8],
) -> Result<Vec<u8>, String> {
    let key_len: u32 = operation_key
        .len()
        .try_into()
        .map_err(|_| "continuity_response_key_too_large".to_string())?;
    let response_len: u32 = response
        .len()
        .try_into()
        .map_err(|_| "continuity_response_payload_too_large".to_string())?;
    let frame_len = 1usize
        .saturating_add(4)
        .saturating_add(operation_key.len())
        .saturating_add(4)
        .saturating_add(response.len());
    if frame_len > MAX_DIST_MSG as usize {
        return Err("continuity_response_frame_too_large".to_string());
    }
    let mut frame = Vec::with_capacity(frame_len);
    frame.push(DIST_CONTINUITY_RESPONSE);
    frame.extend_from_slice(&key_len.to_le_bytes());
    frame.extend_from_slice(operation_key.as_bytes());
    frame.extend_from_slice(&response_len.to_le_bytes());
    frame.extend_from_slice(response);
    Ok(frame)
}

fn decode_continuity_response_frame(data: &[u8]) -> Result<(String, Vec<u8>), String> {
    if data.len() < 9 || data[0] != DIST_CONTINUITY_RESPONSE {
        return Err("continuity_response_frame_invalid".to_string());
    }
    let key_len = u32::from_le_bytes(data[1..5].try_into().unwrap()) as usize;
    let key_end = 5usize
        .checked_add(key_len)
        .ok_or_else(|| "continuity_response_key_length_invalid".to_string())?;
    if key_end + 4 > data.len() {
        return Err("continuity_response_key_truncated".to_string());
    }
    let operation_key = std::str::from_utf8(&data[5..key_end])
        .map_err(|_| "continuity_response_key_invalid_utf8".to_string())?
        .to_string();
    let response_len = u32::from_le_bytes(data[key_end..key_end + 4].try_into().unwrap()) as usize;
    let response_start = key_end + 4;
    if response_start.saturating_add(response_len) != data.len() {
        return Err("continuity_response_payload_length_invalid".to_string());
    }
    if operation_key.is_empty() || response_len == 0 {
        return Err("continuity_response_payload_invalid".to_string());
    }
    Ok((operation_key, data[response_start..].to_vec()))
}

fn retain_and_broadcast_continuity_response(operation_key: &str, response: &[u8]) {
    if let Err(error) =
        crate::dist::continuity_store::persist_runtime_response(operation_key, response)
    {
        eprintln!(
            "mesh continuity: response_store_failed operation={} reason={}",
            operation_key, error
        );
    }
    let Ok(frame) = encode_continuity_response_frame(operation_key, response) else {
        return;
    };
    let targets: BTreeSet<String> = crate::dist::continuity::continuity_registry()
        .record(operation_key)
        .map(|record| {
            let mut targets =
                BTreeSet::from([record.ingress_node.clone(), record.owner_node.clone()]);
            targets.extend(record.replica_nodes().iter().cloned());
            targets
        })
        .unwrap_or_default();
    let sessions: Vec<_> = node_state()
        .map(|state| {
            state
                .sessions
                .read()
                .values()
                .filter(|session| targets.contains(&session.remote_name))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for session in sessions {
        let _ = session.send(OutboundClass::Application, frame.clone());
    }
}

struct HttpRouteV2ReplyTask {
    session: Arc<NodeSession>,
    query: HttpRouteQuery,
    _reservation: AcceptedHttpReservation,
}

extern "C-unwind" fn http_route_v2_reply_entry(args: *const u8) {
    let words = unsafe { Box::from_raw(args as *mut [u64; 1]) };
    let task = unsafe { Box::from_raw(words[0] as *mut HttpRouteV2ReplyTask) };
    let (correlation_id, runtime_name, request_key, attempt_id, request_payload) = &task.query;
    let result = execute_http_route_query(runtime_name, request_key, attempt_id, request_payload);
    if let Ok(reply) = encode_http_route_v2_reply_frame(*correlation_id, result) {
        let _ = task.session.send(OutboundClass::Application, reply);
    }
}

fn dispatch_http_route_v2_reply(session: Arc<NodeSession>, message: Vec<u8>) {
    expire_http_reservations(&session, Instant::now());
    let Ok(query) = decode_http_route_v2_query_frame(&message) else {
        return;
    };
    let correlation_id = query.0;
    let Some(reservation) = session
        .accepted_http_reservations
        .lock()
        .unwrap()
        .remove(&correlation_id)
    else {
        if let Ok(reply) = encode_http_route_v2_reply_frame(
            correlation_id,
            Err("owner_reservation_missing_or_expired".to_string()),
        ) {
            let _ = session.send(OutboundClass::Application, reply);
        }
        return;
    };
    let task_ptr = Box::into_raw(Box::new(HttpRouteV2ReplyTask {
        session,
        query,
        _reservation: reservation,
    })) as u64;
    let args_ptr = Box::into_raw(Box::new([task_ptr]));
    crate::actor::mesh_actor_spawn(
        http_route_v2_reply_entry as *const u8,
        args_ptr.cast(),
        std::mem::size_of::<u64>() as u64,
        1,
    );
}

fn reject_clustered_http_route_attempt(request_key: &str, attempt_id: &str, reason: &str) {
    if request_key.is_empty() || attempt_id.is_empty() {
        return;
    }
    let _ = crate::dist::continuity::continuity_registry().reject_durable_request(
        request_key,
        attempt_id,
        reason,
    );
}

fn execute_clustered_http_route_locally(
    fn_ptr: *const u8,
    request_key: &str,
    attempt_id: &str,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    let response_payload = match crate::http::server::invoke_route_handler_from_payload(
        fn_ptr as *mut u8,
        request_payload,
    ) {
        Ok(response_payload) => response_payload,
        Err(reason) => {
            reject_clustered_http_route_attempt(request_key, attempt_id, &reason);
            return Err(reason);
        }
    };

    if let Err(reason) = complete_declared_work(request_key, attempt_id) {
        reject_clustered_http_route_attempt(request_key, attempt_id, &reason);
        return Err(reason);
    }

    retain_and_broadcast_continuity_response(request_key, &response_payload);

    Ok(response_payload)
}

fn execute_clustered_http_route_remote(
    target: &str,
    runtime_name: &str,
    request_key: &str,
    attempt_id: &str,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    let state = node_state().ok_or_else(|| "clustered_http_route_node_not_started".to_string())?;
    let session = state
        .sessions
        .read()
        .get(target)
        .cloned()
        .ok_or_else(|| format!("clustered_http_route_session_unavailable:{target}"))?;
    let correlation_id = HTTP_ROUTE_CORRELATION_ID.fetch_add(1, Ordering::Relaxed);
    let payload = encode_http_route_v2_query_frame(
        correlation_id,
        runtime_name,
        request_key,
        attempt_id,
        request_payload,
    )?;
    let reservation = encode_http_reserve(
        correlation_id,
        runtime_name,
        request_key,
        request_payload_len(request_payload),
    )?;
    let (reservation_sender, reservation_receiver) = crate::actor::cooperative_channel();
    session
        .pending_http_reservations
        .lock()
        .unwrap()
        .insert(correlation_id, reservation_sender);
    // Reservation traffic has its own bounded lane so a burst cannot consume
    // critical operator/consensus control capacity or application payload
    // capacity. The writer schedules it fairly with the accepted payloads.
    if let Err(error) = session.send(OutboundClass::Admission, reservation) {
        session
            .pending_http_reservations
            .lock()
            .unwrap()
            .remove(&correlation_id);
        return Err(format!("clustered_http_reservation_write_failed:{error}"));
    }
    // The reply handler and a disconnect (fail_pending_session_requests) send
    // before they drop a sender, so each wait ends in a reply or its timeout.
    match crate::actor::cooperative_recv_timeout(&reservation_receiver, HTTP_RESERVATION_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(reason)) => return Err(reason),
        Err(_) => {
            session
                .pending_http_reservations
                .lock()
                .unwrap()
                .remove(&correlation_id);
            crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
            return Err("clustered_http_reservation_timeout".to_string());
        }
    }
    let (sender, receiver) = crate::actor::cooperative_channel();
    session
        .pending_http_routes
        .lock()
        .unwrap()
        .insert(correlation_id, sender);
    {
        if let Err(error) = session.send(OutboundClass::Application, payload) {
            session
                .pending_http_routes
                .lock()
                .unwrap()
                .remove(&correlation_id);
            return Err(format!("clustered_http_route_query_write_failed:{error}"));
        }
    }
    crate::actor::cooperative_recv_timeout(&receiver, CLUSTERED_HTTP_ROUTE_TIMEOUT).unwrap_or_else(
        |_| {
            session
                .pending_http_routes
                .lock()
                .unwrap()
                .remove(&correlation_id);
            crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
            Err("clustered_http_route_reply_timeout".to_string())
        },
    )
}

/// Runs a clustered HTTP request another node routed here, as its owner.
fn execute_http_route_query(
    runtime_name: &str,
    request_key: &str,
    attempt_id: &str,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    match lookup_declared_handler(runtime_name) {
        Some(entry) => execute_clustered_http_route_locally(
            entry.fn_ptr.0,
            request_key,
            attempt_id,
            request_payload,
        ),
        None => {
            let reason = format!("declared_handler_not_registered:{runtime_name}");
            reject_clustered_http_route_attempt(request_key, attempt_id, &reason);
            Err(reason)
        }
    }
}

pub(crate) struct ClusteredHttpRouteExecution {
    pub response_payload: Vec<u8>,
    pub replayed: bool,
    pub ingress_node: String,
    pub execution_node: String,
    pub routed_remotely: bool,
}

fn clustered_http_execution(
    response_payload: Vec<u8>,
    replayed: bool,
    record: &crate::dist::continuity::ContinuityRecord,
) -> ClusteredHttpRouteExecution {
    ClusteredHttpRouteExecution {
        response_payload,
        replayed,
        ingress_node: record.ingress_node.clone(),
        execution_node: if record.execution_node.is_empty() {
            record.owner_node.clone()
        } else {
            record.execution_node.clone()
        },
        routed_remotely: record.routed_remotely,
    }
}

fn retryable_clustered_http_transport_failure(reason: &str) -> bool {
    reason.starts_with("clustered_http_route_session_unavailable:")
        || reason.starts_with("clustered_http_reservation_write_failed:")
        // A draining owner rejects before handler execution and therefore
        // provides a safe placement fence. The coordinator's drain transfer
        // can move safe/idempotent work to another owner without ambiguity.
        || reason == "owner_reservation_rejected:Draining"
        || reason == "clustered_http_reservation_timeout"
        // The owner's session ended while the reservation or the query
        // waited (fail_pending_session_requests).
        || reason == "peer_session_disconnected"
        || reason.starts_with("clustered_http_route_query_write_failed:")
        || reason == "clustered_http_route_reply_timeout"
        || reason == "attempt_id_mismatch"
}

fn continuity_recovery_is_observable(
    request_key: &str,
    failed_attempt_id: &str,
    failed_owner: &str,
) -> bool {
    use crate::dist::continuity::{ContinuityPhase, ContinuityResult, ReplicaStatus};

    crate::dist::continuity::continuity_registry()
        .record(request_key)
        .is_some_and(|record| {
            if record.phase == ContinuityPhase::Rejected
                || record.result == ContinuityResult::Rejected
            {
                return false;
            }
            record.phase == ContinuityPhase::Completed
                || record.result == ContinuityResult::Succeeded
                || record.replica_status == ReplicaStatus::OwnerLost
                || record.attempt_id != failed_attempt_id
                || record.owner_node != failed_owner
        })
}

fn await_recovered_continuity_response(request_key: &str) -> Option<Vec<u8>> {
    let deadline = Instant::now()
        + CLUSTERED_HTTP_ROUTE_TIMEOUT
        + HTTP_RESERVATION_TIMEOUT
        + Duration::from_millis(500);
    loop {
        if let Ok(Some(response)) =
            crate::dist::continuity_store::replay_runtime_response(request_key)
        {
            return Some(response);
        }
        let record = crate::dist::continuity::continuity_registry().record(request_key);
        if record.is_some_and(|record| {
            record.phase == crate::dist::continuity::ContinuityPhase::Rejected
        }) || Instant::now() >= deadline
        {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn execute_clustered_http_route(
    runtime_name: &str,
    request_key: &str,
    payload_hash: &str,
    request_payload: &[u8],
) -> Result<ClusteredHttpRouteExecution, String> {
    if runtime_name.trim().is_empty() {
        return Err("declared_handler_runtime_name_missing".to_string());
    }
    if request_key.is_empty() {
        return Err("request_key_missing".to_string());
    }
    if payload_hash.is_empty() {
        return Err("payload_hash_missing".to_string());
    }
    if request_payload.is_empty() {
        return Err("clustered_http_route_request_payload_missing".to_string());
    }

    let required_replica_count = required_replica_count_for_runtime_name(runtime_name)?;
    let prepared = prepare_declared_handler_submission(
        runtime_name,
        request_key,
        payload_hash,
        required_replica_count,
        request_payload,
    )?;
    match prepared.decision.outcome {
        crate::dist::continuity::SubmitOutcome::Created => {}
        crate::dist::continuity::SubmitOutcome::Duplicate => {
            if prepared.decision.record.phase != crate::dist::continuity::ContinuityPhase::Completed
            {
                return Err("idempotent_operation_in_progress".to_string());
            }
            let response = crate::dist::continuity_store::replay_runtime_response(request_key)?
                .ok_or_else(|| "idempotent_response_not_retained".to_string())?;
            return Ok(clustered_http_execution(
                response,
                true,
                &prepared.decision.record,
            ));
        }
        crate::dist::continuity::SubmitOutcome::Conflict
        | crate::dist::continuity::SubmitOutcome::Rejected => {
            return Err(rejected_submit_reason(&prepared.decision));
        }
    }
    if prepared.decision.record.phase == crate::dist::continuity::ContinuityPhase::Rejected {
        return Err(rejected_submit_reason(&prepared.decision));
    }

    let dispatch = if prepared.placement.routed_remotely {
        record_peer_original_attempt(&prepared.decision.record.owner_node, Instant::now());
        execute_clustered_http_route_remote(
            &prepared.decision.record.owner_node,
            runtime_name,
            &prepared.decision.record.request_key,
            &prepared.decision.record.attempt_id,
            request_payload,
        )
    } else {
        execute_clustered_http_route_locally(
            prepared.entry.fn_ptr.0,
            &prepared.decision.record.request_key,
            &prepared.decision.record.attempt_id,
            request_payload,
        )
    };

    match dispatch {
        Ok(response_payload) => {
            retain_and_broadcast_continuity_response(request_key, &response_payload);
            Ok(clustered_http_execution(
                response_payload,
                false,
                &prepared.decision.record,
            ))
        }
        Err(reason) => {
            if prepared.placement.routed_remotely
                && retryable_clustered_http_transport_failure(&reason)
                && crate::http::server::http_request_payload_is_replay_safe(request_payload)?
                && allow_peer_retry(&prepared.decision.record.owner_node, Instant::now())
            {
                let jitter_millis = rand::random_range(0..=100_u64);
                std::thread::park_timeout(Duration::from_millis(jitter_millis));
                let owner = prepared.decision.record.owner_node.clone();
                let registry = crate::dist::continuity::continuity_registry();
                let transitioned = registry
                    .mark_owner_loss_for_request(
                        &prepared.decision.record.request_key,
                        &prepared.decision.record.attempt_id,
                        &owner,
                    )
                    .ok()
                    .flatten()
                    .is_some();
                if transitioned {
                    maybe_spawn_primary_owner_loss_recovery(&owner);
                }
                if transitioned
                    // The remote owner can reject a late completion after it
                    // has already observed a newer fenced attempt, while this
                    // ingress has not received that upsert yet. The mismatch
                    // itself is therefore sufficient evidence to wait for the
                    // authoritative safe-method recovery instead of exposing
                    // the expected replication race to the client.
                    || reason == "attempt_id_mismatch"
                    || continuity_recovery_is_observable(
                        &prepared.decision.record.request_key,
                        &prepared.decision.record.attempt_id,
                        &owner,
                    )
                {
                    if let Some(response_payload) =
                        await_recovered_continuity_response(&prepared.decision.record.request_key)
                    {
                        let recovered = crate::dist::continuity::continuity_registry()
                            .record(&prepared.decision.record.request_key)
                            .unwrap_or_else(|| prepared.decision.record.clone());
                        return Ok(clustered_http_execution(
                            response_payload,
                            false,
                            &recovered,
                        ));
                    }
                }
            }
            if prepared.placement.routed_remotely {
                reject_clustered_http_route_attempt(
                    &prepared.decision.record.request_key,
                    &prepared.decision.record.attempt_id,
                    &reason,
                );
            }
            Err(reason)
        }
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
fn accept_loop(listener: TcpListener, state: &'static NodeState) {
    // A node listens for as long as the process runs.
    for tcp_stream in listener.incoming() {
        let Ok(tcp_stream) = tcp_stream else {
            // A connection that went before it was taken, or no descriptor
            // to take it with for now.
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        if ACTIVE_INCOMING_HANDSHAKES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_INCOMING_HANDSHAKES).then_some(active + 1)
            })
            .is_err()
        {
            eprintln!("mesh node: incoming connection rejected: handshake_limit_reached");
            continue;
        }
        let spawn = std::thread::Builder::new()
            .name("mesh-node-handshake".to_string())
            .spawn(move || {
                let _active = IncomingHandshakeGuard;
                handle_accepted_connection(tcp_stream, state);
            });
        if let Err(error) = spawn {
            ACTIVE_INCOMING_HANDSHAKES.fetch_sub(1, Ordering::AcqRel);
            eprintln!("mesh node: handshake worker spawn failed: {error}");
        }
    }
}

/// Bounds a connection's reads and writes by `timeout` while it has not
/// authenticated (`None` once it has: its session polls instead).
fn set_handshake_timeouts(stream: &TcpStream, timeout: Option<Duration>) -> io::Result<()> {
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)
}

fn handle_accepted_connection(tcp_stream: TcpStream, state: &NodeState) {
    if !auth_failures_below_limit() {
        eprintln!("mesh node: incoming connection rejected: authentication_rate_limited");
        return;
    }
    if let Err(error) = set_handshake_timeouts(&tcp_stream, Some(NODE_HANDSHAKE_TIMEOUT)) {
        eprintln!("mesh node: accepted stream setup failed: {error}");
        return;
    }

    let server_conn = match rustls::ServerConnection::new(Arc::clone(&state.tls_server_config)) {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("mesh node: TLS server connection failed: {error}");
            return;
        }
    };
    let mut tls_stream = StreamOwned::new(server_conn, tcp_stream);
    let (remote_name, remote_creation, negotiated_protocol, remote_identity) =
        match perform_handshake_negotiated(&mut tls_stream, state, false) {
            Ok(result) => result,
            Err(error) => {
                record_auth_failure();
                eprintln!("mesh node: handshake failed: {error}");
                return;
            }
        };

    if is_transient_operator_client(&remote_name) {
        if !operator_query_allowed() {
            eprintln!("mesh node: transient operator query rejected: operator_query_rate_limited");
            return;
        }
        let stream = NodeStream::ServerTls(tls_stream);
        if let Err(error) = handle_transient_operator_query_connection(
            remote_name.clone(),
            remote_creation,
            stream,
            Duration::from_secs(5),
            negotiated_protocol,
            remote_identity,
        ) {
            eprintln!(
                "mesh node: transient operator query failed for {}: {}",
                remote_name, error
            );
        }
        return;
    }

    if let Err(error) = set_handshake_timeouts(&tls_stream.sock, None) {
        eprintln!("mesh node: accepted stream timeout reset failed: {error}");
        return;
    }
    let node_id = state.assign_node_id();
    let stream = NodeStream::ServerTls(tls_stream);
    match register_session(
        state,
        remote_name.clone(),
        remote_creation,
        node_id,
        stream,
        negotiated_protocol,
        remote_identity,
    ) {
        Ok(session) => {
            spawn_session_threads(&session);
            send_peer_list(&session);
            crate::dist::global::send_global_sync(&session);
            crate::dist::continuity::spawn_continuity_sync(&session);
        }
        Err(error) if error == format!("already_connected:{remote_name}") => {}
        Err(error) => {
            eprintln!(
                "mesh node: session registration failed for {}: {}",
                remote_name, error
            );
        }
    }
}

/// The cookie of the node `test_node` starts.
#[cfg(test)]
pub(crate) const TEST_NODE_COOKIE: &str = "mesh-runtime-test-node-cookie";

/// This process's node, which every test that needs one shares: started
/// once, on an ephemeral port. (A process has one node, so no test starts
/// another.)
#[cfg(test)]
pub(crate) fn test_node() -> &'static NodeState {
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        crate::actor::mesh_rt_init_actor(2);
        assert_eq!(
            start_named_node("test-node@127.0.0.1:0", TEST_NODE_COOKIE),
            0,
            "the test node starts"
        );
    });
    node_state().expect("the test node is started")
}

/// Start a fresh one-shot listener for tests that share process-global node state.
#[cfg(test)]
pub(crate) fn start_one_shot_test_listener() -> Result<String, String> {
    let state = node_state().ok_or_else(|| "test node is not initialized".to_string())?;
    let listener = TcpListener::bind((state.host.as_str(), 0))
        .map_err(|error| format!("test listener bind failed: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("test listener address failed: {error}"))?
        .port();
    std::thread::spawn(move || match listener.accept() {
        Ok((stream, _)) => handle_accepted_connection(stream, state),
        Err(error) => eprintln!("mesh node: one-shot test listener failed: {error}"),
    });
    Ok(format!("operator-query-test@{}:{port}", state.host))
}

// ---------------------------------------------------------------------------
// Runtime-owned bootstrap entry point
// ---------------------------------------------------------------------------

/// Starts this process's node, `mesh_node_start` without the raw text: 0
/// once it listens, -1 when a node is already started, or `bind_node`'s
/// code.
fn start_named_node(name: &str, cookie: &str) -> i64 {
    if NODE_STATE.get().is_some() {
        return -1;
    }
    let (node, listener) = match bind_node(name, cookie) {
        Ok(bound) => bound,
        Err(code) => return code,
    };
    let state = NODE_STATE.get_or_init(|| node);
    std::thread::spawn(move || accept_loop(listener, state));
    start_discovery_from_env();
    0
}

#[repr(C)]
pub struct MeshBootstrapStatus {
    pub mode: *mut MeshString,
    pub node_name: *mut MeshString,
    pub cluster_port: i64,
    pub discovery_seed: *mut MeshString,
}

fn alloc_mesh_value<T>(value: T) -> *mut T {
    unsafe {
        let ptr = crate::gc::mesh_gc_alloc_actor(
            std::mem::size_of::<T>() as u64,
            std::mem::align_of::<T>() as u64,
        ) as *mut T;
        ptr.write(value);
        ptr
    }
}

fn mesh_bootstrap_status(status: BootstrapStatus) -> MeshBootstrapStatus {
    MeshBootstrapStatus {
        mode: mesh_str(status.mode_label()),
        node_name: mesh_str(&status.node_name),
        cluster_port: i64::from(status.cluster_port),
        discovery_seed: mesh_str(&status.discovery_seed),
    }
}

fn bootstrap_ok_status(status: BootstrapStatus) -> *mut MeshResult {
    alloc_result(
        0,
        alloc_mesh_value(mesh_bootstrap_status(status)) as *mut u8,
    )
}

/// Resolve startup mode from the public environment contract and start the
/// node only when cluster mode is valid.
pub fn start_from_env() -> Result<BootstrapStatus, String> {
    let status = bootstrap_from_env_with(start_named_node)?;
    let hydrated = super::continuity::hydrate_runtime_continuity_from_store()?;
    if hydrated > 0 {
        eprintln!("mesh continuity: transition=hydrated records={hydrated}");
    }
    let consensus_enabled = super::autonomous::embedded_autonomous_config()
        .is_none_or(|config| config.features.controller_quorum);
    if consensus_enabled {
        super::consensus::start_mesh_consensus_from_env(&status.node_name)?;
    }
    super::autonomous::start_autonomous_controller()?;
    Ok(status)
}

#[no_mangle]
pub extern "C" fn mesh_node_start_from_env() -> *mut MeshResult {
    match start_from_env() {
        Ok(status) => bootstrap_ok_status(status),
        Err(reason) => err_result(&reason),
    }
}

#[cfg(test)]
fn start_from_inputs_for_test<F>(
    inputs: super::bootstrap::BootstrapInputs,
    start_node: F,
) -> Result<BootstrapStatus, String>
where
    F: FnOnce(&str, &str) -> i64,
{
    super::bootstrap::bootstrap_with_inputs(inputs, start_node)
}

// ---------------------------------------------------------------------------
// mesh_node_start -- extern "C" entry point
// ---------------------------------------------------------------------------

/// Initialize the local node and start listening for connections.
///
/// Called from compiled Mesh code via `Node.start("name@host", cookie: "secret")`.
///
/// # Arguments
/// - `name_ptr`, `name_len`: UTF-8 node name ("name@host" or "name@host:port")
/// - `cookie_ptr`, `cookie_len`: UTF-8 shared secret
///
/// # Returns
/// - `0` on success
/// - `-1` if node already started
/// - `-2` if TCP bind failed
/// - `-3` for a name, cookie or TLS setup it cannot start with
#[no_mangle]
pub extern "C" fn mesh_node_start(
    name_ptr: *const u8,
    name_len: u64,
    cookie_ptr: *const u8,
    cookie_len: u64,
) -> i64 {
    let text = |ptr: *const u8, len: u64| {
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len as usize) }).ok()
    };
    match (text(name_ptr, name_len), text(cookie_ptr, cookie_len)) {
        (Some(name), Some(cookie)) => start_named_node(name, cookie),
        _ => -3,
    }
}

/// The state of a node named `name` (`name@host[:port]`, where port 0 asks
/// the system for a free one) with `cookie`, and the listener it takes
/// connections on: -3 for a name, cookie or TLS setup it cannot start
/// with, -2 for an address it cannot bind.
fn bind_node(name: &str, cookie: &str) -> Result<(NodeState, TcpListener), i64> {
    if let Err(error) = validate_cluster_cookie_strength(cookie, autonomous_mode_requested()) {
        eprintln!("mesh node: cluster authentication configuration failed: {error}");
        return Err(-3);
    }
    let (name_part, host, port) = super::discovery::split_node_name(name, true).map_err(|_| -3)?;
    let (tls_server_config, tls_client_config) = node_tls_configs(&configured_mtls_values())
        .map_err(|error| {
            eprintln!("mesh node: TLS configuration failed: {error}");
            -3
        })?;
    let listener = TcpListener::bind((host, port)).map_err(|_| -2)?;
    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let advertised_name = if port == 0 {
        advertised_node_name(name_part, host, actual_port)
    } else {
        name.to_string()
    };
    let node = NodeState {
        name: advertised_name,
        host: host.to_string(),
        port: actual_port,
        cookie: cookie.to_string(),
        creation: AtomicU8::new(1),
        next_node_id: AtomicU16::new(1),
        tls_server_config,
        tls_client_config,
        sessions: RwLock::new(FxHashMap::default()),
        node_id_map: RwLock::new(FxHashMap::default()),
        node_monitors: RwLock::new(FxHashMap::default()),
    };
    Ok((node, listener))
}

/// The name a node started on port 0 goes by: its host, an IPv6 address in
/// brackets as a node name needs, and the port the system gave it.
fn advertised_node_name(name_part: &str, host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("{name_part}@[{host}]:{port}")
    } else {
        format!("{name_part}@{host}:{port}")
    }
}

// ---------------------------------------------------------------------------
// connect_to_remote_node -- establish an outgoing authenticated session
// ---------------------------------------------------------------------------

/// What the cookie handshake says of the node at the other end: its name,
/// creation, the protocol the two agreed, and its signed identity.
type Authenticated = (
    String,
    u8,
    NegotiatedProtocol,
    Option<super::identity_claim::NodeIdentityClaim>,
);

/// A TLS connection to the node `target` names, authenticated through the
/// cookie handshake as `local_name`: its reads and writes wait at most
/// `timeout`, until the caller says otherwise.
fn connect_authenticated(
    target: &str,
    client_config: Arc<ClientConfig>,
    local_name: &str,
    cookie: &str,
    creation: u8,
    timeout: Duration,
) -> Result<
    (
        StreamOwned<rustls::ClientConnection, TcpStream>,
        Authenticated,
    ),
    String,
> {
    let (_name_part, host, port) =
        parse_node_name(target).map_err(|e| format!("invalid connect target: {}", e))?;
    let tcp_stream = TcpStream::connect((host, port))
        .map_err(|e| format!("TCP connect to {}:{} failed: {}", host, port, e))?;
    set_handshake_timeouts(&tcp_stream, Some(timeout))
        .map_err(|error| format!("TCP timeout setup failed: {error}"))?;
    // Server name is "mesh-node" -- doesn't matter since we skip verification.
    let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
    let client_conn = rustls::ClientConnection::new(client_config, server_name)
        .map_err(|e| format!("TLS client connection failed: {}", e))?;
    let mut tls_stream = StreamOwned::new(client_conn, tcp_stream);
    let authenticated =
        perform_handshake_with_identity(&mut tls_stream, local_name, cookie, creation, true)
            .map_err(|e| format!("handshake with {}:{} failed: {}", host, port, e))?;
    Ok((tls_stream, authenticated))
}

fn connect_to_remote_node(state: &NodeState, target: &str) -> Result<Arc<NodeSession>, String> {
    let (tls_stream, (remote_name, remote_creation, negotiated_protocol, remote_identity)) =
        connect_authenticated(
            target,
            Arc::clone(&state.tls_client_config),
            &state.name,
            &state.cookie,
            state.creation(),
            NODE_HANDSHAKE_TIMEOUT,
        )?;
    set_handshake_timeouts(&tls_stream.sock, None)
        .map_err(|error| format!("TCP timeout reset failed: {error}"))?;

    // Register the authenticated session
    let node_id = state.assign_node_id();
    let stream = NodeStream::ClientTls(tls_stream);
    match register_session(
        state,
        remote_name.clone(),
        remote_creation,
        node_id,
        stream,
        negotiated_protocol,
        remote_identity,
    ) {
        Ok(session) => {
            spawn_session_threads(&session);
            send_peer_list(&session);
            crate::dist::global::send_global_sync(&session);
            crate::dist::continuity::spawn_continuity_sync(&session);
            Ok(session)
        }
        Err(error) if error == format!("already_connected:{}", remote_name) => {
            let sessions = state.sessions.read();
            sessions.get(&remote_name).cloned().ok_or_else(|| {
                format!(
                    "session registration raced but no live session remained for {}",
                    remote_name
                )
            })
        }
        Err(error) => Err(format!(
            "session registration failed for {}: {}",
            remote_name, error
        )),
    }
}

// ---------------------------------------------------------------------------
// mesh_node_connect -- extern "C" entry point for outgoing connections
// ---------------------------------------------------------------------------

/// How long `Node.connect` waits for the peer's global names.
const GLOBAL_NAMES_WAIT: Duration = Duration::from_secs(5);

/// Connect to a remote node and perform mutual cookie authentication.
///
/// Called from compiled Mesh code via `Node.connect("name@host:port")`.
///
/// # Arguments
/// - `name_ptr`, `name_len`: UTF-8 target address ("name@host:port")
///
/// # Returns
/// - `0` on success (authenticated connection established)
/// - `-1` if node not started (mesh_node_start not called)
/// - `-2` if TCP connection failed
/// - `-3` if handshake failed (wrong cookie, I/O error, or invalid format)
#[no_mangle]
pub extern "C" fn mesh_node_connect(name_ptr: *const u8, name_len: u64) -> i64 {
    // Check NODE_STATE is initialized
    let state = match NODE_STATE.get() {
        Some(s) => s,
        None => {
            eprintln!("mesh node: node not started");
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

    match connect_to_remote_node(state, &target) {
        Ok(session) => {
            // Each side sends its global names as the session starts; wait
            // for the peer's, so they resolve as soon as this returns.
            let deadline = Instant::now() + GLOBAL_NAMES_WAIT;
            while !session.global_names_received.load(Ordering::Acquire)
                && !session.shutdown.load(Ordering::Acquire)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            0
        }
        Err(error) => {
            eprintln!("mesh node: {}", error);
            if error.starts_with("TCP connect") {
                -2
            } else {
                -3
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Node query APIs -- Node.self() and Node.list()
// ---------------------------------------------------------------------------

/// Return the current node's name as a Mesh string pointer.
///
/// Returns an empty string if node is not started (mesh_node_start not called).
/// The returned string is GC-allocated via mesh_string_new.
#[no_mangle]
pub extern "C" fn mesh_node_self() -> *const u8 {
    match node_state() {
        Some(state) => crate::string::mesh_str(&state.name) as *const u8,
        None => {
            // Return an empty string instead of null to prevent null pointer
            // dereference when Mesh code compares the result (e.g., `Node.self() != ""`).
            crate::string::mesh_string_new(b"".as_ptr(), 0) as *const u8
        }
    }
}

/// Return a list of connected node names as a Mesh list of strings.
///
/// Returns an empty list if node is not started or no connections exist.
/// Each element is a GC-allocated Mesh string. The list itself is allocated
/// via mesh_list_from_array.
#[no_mangle]
pub extern "C" fn mesh_node_list() -> *mut u8 {
    let state = match node_state() {
        Some(s) => s,
        None => {
            return crate::collections::list::mesh_list_new();
        }
    };

    let sessions = state.sessions.read();
    if sessions.is_empty() {
        return crate::collections::list::mesh_list_new();
    }

    let names: Vec<String> = sessions.keys().cloned().collect();
    drop(sessions);

    // Build array of Mesh string pointers, then create list from array
    let mut string_ptrs: Vec<u64> = Vec::with_capacity(names.len());
    for name in &names {
        let s = crate::string::mesh_str(name);
        string_ptrs.push(s as u64);
    }

    crate::collections::list::mesh_list_from_array(string_ptrs.as_ptr(), string_ptrs.len() as i64)
}

// ---------------------------------------------------------------------------
// Remote spawn argument encoding helpers
// ---------------------------------------------------------------------------

fn encode_remote_spawn_args(args_data: &[u8], arg_tags: &[u8]) -> Result<Vec<u8>, String> {
    if args_data.len() != arg_tags.len() * 8 {
        return Err("remote_spawn_args_size_mismatch".to_string());
    }

    let mut payload = Vec::new();
    payload.extend_from_slice(&(arg_tags.len() as u16).to_le_bytes());
    payload.extend_from_slice(arg_tags);

    for (raw_bytes, tag) in args_data.chunks_exact(8).zip(arg_tags.iter().copied()) {
        let raw = u64::from_le_bytes(raw_bytes.try_into().unwrap());
        match tag {
            REMOTE_SPAWN_ARG_INT | REMOTE_SPAWN_ARG_FLOAT => {
                payload.extend_from_slice(&raw.to_le_bytes());
            }
            REMOTE_SPAWN_ARG_PID => {
                // Its local id, and the node it is on: `[u64][u16 len][name]`.
                let pid = crate::actor::process::ProcessId(raw);
                let node = pid_node_name(pid).unwrap_or_default();
                payload.extend_from_slice(&pid.local_id().to_le_bytes());
                payload.extend_from_slice(&(node.len() as u16).to_le_bytes());
                payload.extend_from_slice(node.as_bytes());
            }
            REMOTE_SPAWN_ARG_BOOL => {
                payload.push((raw != 0) as u8);
            }
            REMOTE_SPAWN_ARG_STRING => {
                let bytes = if raw == 0 {
                    &[][..]
                } else {
                    let mesh_str = unsafe { &*(raw as *const crate::string::MeshString) };
                    unsafe { mesh_str.as_bytes() }
                };
                let len: u32 = bytes
                    .len()
                    .try_into()
                    .map_err(|_| format!("remote_spawn_string_too_large:{}", bytes.len()))?;
                payload.extend_from_slice(&len.to_le_bytes());
                payload.extend_from_slice(bytes);
            }
            REMOTE_SPAWN_ARG_UNIT => {}
            other => return Err(format!("remote_spawn_arg_tag_unsupported:{other}")),
        }
    }

    Ok(payload)
}

/// Resolve a DIST_SPAWN target and decode its arguments.
///
/// The peer is authenticated by the time a DIST_SPAWN arrives, but it still
/// chooses the arity and argument types on the wire while the generated actor
/// entry loads a fixed number of typed words from the args buffer. The
/// registered signature is therefore checked before any argument value is
/// materialized, so a mismatched request is rejected without ever reaching
/// the generated loads.
fn prepare_remote_spawn(
    fn_name: &str,
    encoded_args: &[u8],
) -> Result<(*const u8, Vec<u64>), String> {
    let Some(registered) = lookup_registered_function(fn_name) else {
        return Err(if lookup_declared_handler_executable(fn_name).is_some() {
            "declared_handler_executable_not_remote_registered".to_string()
        } else {
            "function_not_found".to_string()
        });
    };
    let decoded_args = decode_remote_spawn_args(encoded_args, &registered.arg_signature)?;
    Ok((registered.fn_ptr.0, decoded_args))
}

/// Check the argument tags a peer sent against the registered signature.
fn validate_remote_spawn_signature(expected: &[u8], provided: &[u8]) -> Result<(), String> {
    if expected.contains(&REMOTE_SPAWN_ARG_UNSUPPORTED) {
        return Err("remote_spawn_target_not_remotely_spawnable".to_string());
    }
    if expected.len() != provided.len() {
        return Err(format!(
            "remote_spawn_arity_mismatch:expected={}:received={}",
            expected.len(),
            provided.len()
        ));
    }
    if let Some(index) = (0..expected.len()).find(|&index| expected[index] != provided[index]) {
        return Err(format!(
            "remote_spawn_arg_type_mismatch:index={}:expected={}:received={}",
            index, expected[index], provided[index]
        ));
    }
    Ok(())
}

fn decode_remote_spawn_args(data: &[u8], expected_tags: &[u8]) -> Result<Vec<u64>, String> {
    if data.len() < 2 {
        return Err("remote_spawn_args_too_short".to_string());
    }

    let arg_count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
    if data.len() < 2 + arg_count {
        return Err("remote_spawn_arg_tags_truncated".to_string());
    }

    let arg_tags = &data[2..2 + arg_count];
    validate_remote_spawn_signature(expected_tags, arg_tags)?;
    let mut pos = 2 + arg_count;
    let mut values = Vec::with_capacity(arg_count);

    for tag in arg_tags.iter().copied() {
        match tag {
            REMOTE_SPAWN_ARG_INT | REMOTE_SPAWN_ARG_FLOAT => {
                if pos + 8 > data.len() {
                    return Err("remote_spawn_arg_value_truncated".to_string());
                }
                values.push(u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()));
                pos += 8;
            }
            REMOTE_SPAWN_ARG_PID => {
                let local = data.get(pos..pos + 8);
                pos += 8;
                let node = crate::dist::global::decode_str(data, &mut pos);
                let (Some(local), Some(node)) = (local, node) else {
                    return Err("remote_spawn_arg_pid_truncated".to_string());
                };
                values.push(pid_on_node(
                    &node,
                    u64::from_le_bytes(local.try_into().unwrap()),
                ));
            }
            REMOTE_SPAWN_ARG_BOOL => {
                if pos + 1 > data.len() {
                    return Err("remote_spawn_arg_bool_truncated".to_string());
                }
                values.push((data[pos] != 0) as u64);
                pos += 1;
            }
            REMOTE_SPAWN_ARG_STRING => {
                if pos + 4 > data.len() {
                    return Err("remote_spawn_arg_string_length_truncated".to_string());
                }
                let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;
                if pos + len > data.len() {
                    return Err("remote_spawn_arg_string_truncated".to_string());
                }
                let mesh_str =
                    crate::string::mesh_string_new(data[pos..pos + len].as_ptr(), len as u64);
                values.push(mesh_str as u64);
                pos += len;
            }
            REMOTE_SPAWN_ARG_UNIT => values.push(0),
            other => return Err(format!("remote_spawn_arg_tag_unsupported:{other}")),
        }
    }

    if pos != data.len() {
        return Err("remote_spawn_args_trailing_bytes".to_string());
    }

    Ok(values)
}

fn allocate_remote_spawn_args(values: &[u64]) -> *mut u8 {
    if values.is_empty() {
        return std::ptr::null_mut();
    }

    let total_size = std::mem::size_of_val(values);
    let ptr = crate::gc::mesh_gc_alloc_actor(total_size as u64, 8);
    unsafe {
        std::ptr::copy_nonoverlapping(values.as_ptr(), ptr as *mut u64, values.len());
    }
    ptr
}

const DECLARED_WORK_LOCAL_NODE: &str = "standalone@local";
const AUTOMATIC_PROMOTION_REJECTED_NOT_STANDBY: &str = "automatic_promotion_rejected:not_standby";
const AUTOMATIC_PROMOTION_REJECTED_PEERS_REMAINING: &str =
    "automatic_promotion_rejected:peers_remaining";
const AUTOMATIC_PROMOTION_REJECTED_NO_MIRRORED_STATE: &str =
    "automatic_promotion_rejected:no_mirrored_state";
const AUTOMATIC_PROMOTION_REJECTED_AMBIGUOUS_PENDING: &str =
    "automatic_promotion_rejected:ambiguous_pending_state";
const AUTOMATIC_RECOVERY_REJECTED_HANDLER_MISSING: &str =
    "automatic_recovery_rejected:missing_handler_metadata";
const STARTUP_REQUEST_KEY_PREFIX: &str = "startup::";
const STARTUP_PAYLOAD_HASH_PREFIX: &str = "startup-payload::";
const STARTUP_RUNTIME_NAME_MISSING: &str = "startup_runtime_name_missing";
const STARTUP_DUPLICATE_REGISTRATION: &str = "startup_duplicate_registration";
const STARTUP_HANDLER_MISSING: &str = "startup_handler_not_registered";
const STARTUP_CONVERGENCE_TIMEOUT: &str = "startup_convergence_timeout";
const STARTUP_ATTEMPT_FENCED: &str = "startup_attempt_fenced";
const STARTUP_TRIGGER_POLL_MS: i64 = 50;
const STARTUP_TRIGGER_MAX_POLLS: usize = 40;
const STARTUP_TRIGGER_STABLE_POLLS: usize = 3;
const STARTUP_KEEPALIVE_SLEEP_MS: i64 = 1_000;
/// Bounded language-owned pending window for clustered startup work.
///
/// This keeps the first mirrored startup record observable through Mesh-owned
/// CLI surfaces before the runtime dispatches the handler, without asking app
/// code, examples, or users to inject timing logic.
const STARTUP_CLUSTERED_PENDING_WINDOW_MS: i64 = 2_500;

#[derive(Clone, Debug, PartialEq, Eq)]
struct StartupWorkIdentity {
    runtime_name: String,
    request_key: String,
    payload_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StartupConvergenceState {
    membership: Vec<String>,
    required_replica_count: u64,
    saw_peer: bool,
    polls: usize,
}

#[derive(Debug)]
struct DeclaredWorkPlacement {
    ingress_node: String,
    owner_node: String,
    routed_remotely: bool,
    fell_back_locally: bool,
    _routing_reservation: Option<crate::dist::routing::RoutingReservation>,
}

fn declared_work_membership() -> Vec<String> {
    let mut members = Vec::new();
    if let Some(state) = node_state() {
        members.push(state.name.clone());
        let sessions = state.sessions.read();
        members.extend(sessions.keys().cloned());
    } else {
        members.push(DECLARED_WORK_LOCAL_NODE.to_string());
    }

    normalize_declared_membership(members)
}

fn stable_hash_u64(value: &str) -> u64 {
    let digest = Sha256::digest(value.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes)
}

fn normalize_declared_membership<I>(membership: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut membership: Vec<String> = membership
        .into_iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    membership.sort_by_key(|value| (stable_hash_u64(value), value.clone()));
    membership.dedup();
    membership
}

fn canonical_declared_membership() -> Vec<String> {
    normalize_declared_membership(declared_work_membership())
}

fn startup_request_key(runtime_name: &str) -> String {
    format!("{STARTUP_REQUEST_KEY_PREFIX}{runtime_name}")
}

fn startup_payload_hash(runtime_name: &str) -> String {
    format!("{STARTUP_PAYLOAD_HASH_PREFIX}{runtime_name}")
}

const STARTUP_WORK_DELAY_ENV: &str = "MESH_STARTUP_WORK_DELAY_MS";

fn configured_startup_dispatch_window_ms() -> i64 {
    match std::env::var(STARTUP_WORK_DELAY_ENV) {
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(value) if value > 0 => value,
            _ => STARTUP_CLUSTERED_PENDING_WINDOW_MS,
        },
        Err(_) => STARTUP_CLUSTERED_PENDING_WINDOW_MS,
    }
}

fn startup_dispatch_window_ms(request_key: &str, required_replica_count: u64) -> i64 {
    if !request_key.starts_with(STARTUP_REQUEST_KEY_PREFIX) || required_replica_count == 0 {
        return 0;
    }

    configured_startup_dispatch_window_ms()
}

fn startup_work_identity(runtime_name: &str) -> Result<StartupWorkIdentity, String> {
    let runtime_name = runtime_name.trim();
    if runtime_name.is_empty() {
        return Err(STARTUP_RUNTIME_NAME_MISSING.to_string());
    }

    Ok(StartupWorkIdentity {
        runtime_name: runtime_name.to_string(),
        request_key: startup_request_key(runtime_name),
        payload_hash: startup_payload_hash(runtime_name),
    })
}

fn wait_for_startup_convergence_with<F, G>(
    mut observe_membership: F,
    mut sleep_between_polls: G,
    desired_required_replica_count: u64,
    max_polls: usize,
) -> Result<StartupConvergenceState, String>
where
    F: FnMut() -> Vec<String>,
    G: FnMut(),
{
    let mut membership = normalize_declared_membership(observe_membership());
    if membership.is_empty() {
        return Err("declared_work_membership_empty".to_string());
    }

    let mut saw_peer = membership.len() > 1;
    let mut stable_polls = 1usize;
    let mut polls = 0usize;

    loop {
        if saw_peer && stable_polls >= STARTUP_TRIGGER_STABLE_POLLS {
            return Ok(StartupConvergenceState {
                membership,
                required_replica_count: startup_effective_required_replica_count(
                    desired_required_replica_count,
                    true,
                ),
                saw_peer,
                polls,
            });
        }

        if polls >= max_polls {
            break;
        }

        sleep_between_polls();
        polls += 1;

        let observed = normalize_declared_membership(observe_membership());
        if observed.is_empty() {
            return Err("declared_work_membership_empty".to_string());
        }

        if observed == membership {
            stable_polls += 1;
        } else {
            membership = observed;
            stable_polls = 1;
        }
        if membership.len() > 1 {
            saw_peer = true;
        }
    }

    if saw_peer {
        Err(STARTUP_CONVERGENCE_TIMEOUT.to_string())
    } else {
        Ok(StartupConvergenceState {
            membership,
            required_replica_count: startup_effective_required_replica_count(
                desired_required_replica_count,
                false,
            ),
            saw_peer,
            polls,
        })
    }
}

fn wait_for_startup_convergence(runtime_name: &str) -> Result<StartupConvergenceState, String> {
    let desired_required_replica_count = required_replica_count_for_runtime_name(runtime_name)?;
    if node_state().is_none() {
        return Ok(StartupConvergenceState {
            membership: canonical_declared_membership(),
            required_replica_count: startup_effective_required_replica_count(
                desired_required_replica_count,
                false,
            ),
            saw_peer: false,
            polls: 0,
        });
    }

    wait_for_startup_convergence_with(
        canonical_declared_membership,
        || crate::actor::mesh_timer_sleep(STARTUP_TRIGGER_POLL_MS),
        desired_required_replica_count,
        STARTUP_TRIGGER_MAX_POLLS,
    )
}

fn declared_work_placement(
    request_key: &str,
    runtime_name: &str,
) -> Result<DeclaredWorkPlacement, String> {
    // This node is always a member.
    let membership = canonical_declared_membership();
    let ingress_node = node_state()
        .map(|state| state.name.clone())
        .unwrap_or_else(|| DECLARED_WORK_LOCAL_NODE.to_string());
    let adaptive_routing = crate::dist::routing::runtime_adaptive_routing_enabled();
    let (owner_node, routing_reservation) = if adaptive_routing {
        let handlers: BTreeSet<String> =
            declared_handler_registry().read().keys().cloned().collect();
        let local_report = crate::dist::routing::local_load_report(&ingress_node, handlers);
        let _ = crate::dist::routing::load_report_registry().apply(local_report, Instant::now());
        let (decision, reservation) = crate::dist::routing::select_owner_and_reserve(
            request_key,
            runtime_name,
            &ingress_node,
            &membership,
            None,
            &crate::dist::routing::runtime_routing_policy(),
            Instant::now(),
        )?;
        (decision.selected_node, Some(reservation))
    } else {
        let owner_index =
            (stable_hash_u64(&format!("request::{request_key}")) as usize) % membership.len();
        (membership[owner_index].clone(), None)
    };
    let routed_remotely = owner_node != ingress_node;

    Ok(DeclaredWorkPlacement {
        ingress_node,
        owner_node,
        routed_remotely,
        fell_back_locally: !routed_remotely,
        _routing_reservation: routing_reservation,
    })
}

fn declared_work_arg_payload(request_key: &str, attempt_id: &str) -> (*mut u8, [u8; 2]) {
    let request_key_ptr = crate::string::mesh_str(request_key);
    let attempt_id_ptr = crate::string::mesh_str(attempt_id);
    let values = [request_key_ptr as u64, attempt_id_ptr as u64];
    (
        allocate_remote_spawn_args(&values),
        [REMOTE_SPAWN_ARG_STRING, REMOTE_SPAWN_ARG_STRING],
    )
}

fn startup_work_arg_payload(runtime_name: &str) -> *mut u8 {
    let runtime_name_ptr = crate::string::mesh_str(runtime_name);
    allocate_remote_spawn_args(&[runtime_name_ptr as u64])
}

fn startup_metadata(runtime_name: &str, extra: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut metadata = Vec::with_capacity(extra.len() + 1);
    metadata.push(("runtime_name".to_string(), runtime_name.to_string()));
    metadata.extend(extra);
    metadata
}

fn log_startup_registered(identity: &StartupWorkIdentity) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_registered".to_string(),
        request_key: Some(identity.request_key.clone()),
        metadata: startup_metadata(
            &identity.runtime_name,
            vec![("payload_hash".to_string(), identity.payload_hash.clone())],
        ),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_registered runtime_name={} request_key={}",
        identity.runtime_name,
        crate::dist::continuity::request_key_fingerprint(&identity.request_key),
    );
}

fn log_startup_trigger(identity: &StartupWorkIdentity, convergence: &StartupConvergenceState) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_trigger".to_string(),
        request_key: Some(identity.request_key.clone()),
        metadata: startup_metadata(
            &identity.runtime_name,
            vec![
                (
                    "required_replicas".to_string(),
                    convergence.required_replica_count.to_string(),
                ),
                ("membership".to_string(), convergence.membership.join(",")),
                ("polls".to_string(), convergence.polls.to_string()),
            ],
        ),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_trigger runtime_name={} request_key={} required_replicas={} membership={}",
        identity.runtime_name,
        crate::dist::continuity::request_key_fingerprint(&identity.request_key),
        convergence.required_replica_count,
        convergence.membership.join(","),
    );
}

fn log_startup_dispatch_window(runtime_name: &str, request_key: &str, pending_window_ms: i64) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_dispatch_window".to_string(),
        request_key: Some(request_key.to_string()),
        metadata: startup_metadata(
            runtime_name,
            vec![
                (
                    "pending_window_ms".to_string(),
                    pending_window_ms.to_string(),
                ),
                ("ownership".to_string(), "language_owned".to_string()),
            ],
        ),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_dispatch_window runtime_name={} request_key={} pending_window_ms={} ownership=language_owned",
        runtime_name,
        crate::dist::continuity::request_key_fingerprint(request_key),
        pending_window_ms,
    );
}

fn maybe_hold_startup_work_dispatch(
    runtime_name: &str,
    request_key: &str,
    required_replica_count: u64,
) {
    let pending_window_ms = startup_dispatch_window_ms(request_key, required_replica_count);
    if pending_window_ms <= 0 {
        return;
    }

    log_startup_dispatch_window(runtime_name, request_key, pending_window_ms);
    crate::actor::mesh_timer_sleep(pending_window_ms);
}

fn log_startup_convergence_timeout(
    identity: &StartupWorkIdentity,
    convergence: &StartupConvergenceState,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_convergence_timeout".to_string(),
        request_key: Some(identity.request_key.clone()),
        reason: Some(STARTUP_CONVERGENCE_TIMEOUT.to_string()),
        metadata: startup_metadata(
            &identity.runtime_name,
            vec![
                (
                    "required_replicas".to_string(),
                    convergence.required_replica_count.max(1).to_string(),
                ),
                ("membership".to_string(), convergence.membership.join(",")),
                ("polls".to_string(), convergence.polls.to_string()),
                ("saw_peer".to_string(), convergence.saw_peer.to_string()),
            ],
        ),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_convergence_timeout runtime_name={} request_key={} membership={} polls={} saw_peer={}",
        identity.runtime_name,
        crate::dist::continuity::request_key_fingerprint(&identity.request_key),
        convergence.membership.join(","),
        convergence.polls,
        convergence.saw_peer,
    );
}

fn log_startup_rejected_without_identity(runtime_name: &str, reason: &str) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_rejected".to_string(),
        reason: Some(reason.to_string()),
        metadata: startup_metadata(runtime_name, Vec::new()),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_rejected runtime_name={} reason={}",
        runtime_name, reason,
    );
}

fn log_startup_rejected(
    identity: &StartupWorkIdentity,
    attempt_id: Option<&str>,
    owner_node: Option<&str>,
    replica_node: Option<&str>,
    reason: &str,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_rejected".to_string(),
        request_key: Some(identity.request_key.clone()),
        attempt_id: attempt_id.map(str::to_string),
        owner_node: owner_node.map(str::to_string),
        replica_node: replica_node.map(str::to_string),
        reason: Some(reason.to_string()),
        metadata: startup_metadata(&identity.runtime_name, Vec::new()),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_rejected runtime_name={} request_key={} attempt_id={} owner={} replica={} reason={}",
        identity.runtime_name,
        crate::dist::continuity::request_key_fingerprint(&identity.request_key),
        attempt_id.unwrap_or(""),
        owner_node.unwrap_or(""),
        replica_node.unwrap_or(""),
        reason,
    );
}

fn log_startup_completed(runtime_name: &str, record: &crate::dist::continuity::ContinuityRecord) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_completed".to_string(),
        request_key: Some(record.request_key.clone()),
        attempt_id: Some(record.attempt_id.clone()),
        owner_node: Some(record.owner_node.clone()),
        replica_node: Some(record.replica_node.clone()),
        execution_node: Some(record.execution_node.clone()),
        metadata: startup_metadata(runtime_name, Vec::new()),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_completed runtime_name={} request_key={} attempt_id={} execution_node={}",
        runtime_name,
        crate::dist::continuity::request_key_fingerprint(&record.request_key),
        record.attempt_id,
        record.execution_node,
    );
}

fn log_startup_fenced(
    runtime_name: &str,
    request_key: &str,
    previous_attempt_id: &str,
    active_record: &crate::dist::continuity::ContinuityRecord,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_fenced".to_string(),
        request_key: Some(request_key.to_string()),
        attempt_id: Some(previous_attempt_id.to_string()),
        owner_node: Some(active_record.owner_node.clone()),
        replica_node: Some(active_record.replica_node.clone()),
        execution_node: if active_record.execution_node.is_empty() {
            None
        } else {
            Some(active_record.execution_node.clone())
        },
        reason: Some(STARTUP_ATTEMPT_FENCED.to_string()),
        metadata: startup_metadata(
            runtime_name,
            vec![(
                "active_attempt_id".to_string(),
                active_record.attempt_id.clone(),
            )],
        ),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_fenced runtime_name={} request_key={} previous_attempt_id={} active_attempt_id={}",
        runtime_name,
        crate::dist::continuity::request_key_fingerprint(request_key),
        previous_attempt_id,
        active_record.attempt_id,
    );
}

fn log_startup_keepalive(registration_count: usize) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_keepalive".to_string(),
        metadata: vec![(
            "registration_count".to_string(),
            registration_count.to_string(),
        )],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_keepalive registration_count={}",
        registration_count,
    );
}

fn log_startup_skipped(
    identity: &StartupWorkIdentity,
    cluster_role: crate::dist::continuity::ContinuityClusterRole,
    promotion_epoch: u64,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "startup_skipped".to_string(),
        request_key: Some(identity.request_key.clone()),
        reason: Some("startup_skipped:standby_authority".to_string()),
        cluster_role: Some(cluster_role.as_str().to_string()),
        promotion_epoch: Some(promotion_epoch),
        metadata: startup_metadata(&identity.runtime_name, Vec::new()),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt startup] transition=startup_skipped runtime_name={} request_key={} cluster_role={} promotion_epoch={} reason=startup_skipped:standby_authority",
        identity.runtime_name,
        crate::dist::continuity::request_key_fingerprint(&identity.request_key),
        cluster_role.as_str(),
        promotion_epoch,
    );
}

fn wait_for_startup_terminal_state(identity: &StartupWorkIdentity, attempt_id: &str) {
    loop {
        let Some(record) =
            crate::dist::continuity::continuity_registry().record(&identity.request_key)
        else {
            log_startup_rejected(
                identity,
                Some(attempt_id),
                None,
                None,
                "request_key_not_found",
            );
            return;
        };

        if record.attempt_id != attempt_id {
            log_startup_fenced(
                &identity.runtime_name,
                &identity.request_key,
                attempt_id,
                &record,
            );
            return;
        }

        match record.phase {
            crate::dist::continuity::ContinuityPhase::Completed => {
                log_startup_completed(&identity.runtime_name, &record);
                return;
            }
            crate::dist::continuity::ContinuityPhase::Rejected => {
                log_startup_rejected(
                    identity,
                    Some(&record.attempt_id),
                    Some(&record.owner_node),
                    Some(&record.replica_node),
                    &record.error,
                );
                return;
            }
            crate::dist::continuity::ContinuityPhase::Submitted => {
                crate::actor::mesh_timer_sleep(STARTUP_TRIGGER_POLL_MS);
            }
        }
    }
}

extern "C" fn startup_work_entry(args: *const u8) {
    let words = unsafe { std::slice::from_raw_parts(args as *const u64, 1) };
    let runtime_name = mesh_string_arg_to_owned(words[0]);
    let identity = match startup_work_identity(&runtime_name) {
        Ok(identity) => identity,
        Err(reason) => {
            log_startup_rejected_without_identity(&runtime_name, &reason);
            return;
        }
    };

    let desired_required_replica_count =
        match required_replica_count_for_runtime_name(&identity.runtime_name) {
            Ok(value) => value,
            Err(reason) => {
                log_startup_rejected(&identity, None, None, None, &reason);
                return;
            }
        };

    let convergence = match wait_for_startup_convergence(&identity.runtime_name) {
        Ok(state) => state,
        Err(reason) if reason == STARTUP_CONVERGENCE_TIMEOUT => {
            let state = StartupConvergenceState {
                membership: canonical_declared_membership(),
                required_replica_count: desired_required_replica_count,
                saw_peer: true,
                polls: STARTUP_TRIGGER_MAX_POLLS,
            };
            log_startup_convergence_timeout(&identity, &state);
            log_startup_rejected(&identity, None, None, None, &reason);
            return;
        }
        Err(reason) => {
            log_startup_rejected(&identity, None, None, None, &reason);
            return;
        }
    };

    log_startup_trigger(&identity, &convergence);

    match submit_declared_work(
        &identity.runtime_name,
        &identity.request_key,
        &identity.payload_hash,
        convergence.required_replica_count,
    ) {
        Ok(decision)
            if matches!(
                decision.outcome,
                crate::dist::continuity::SubmitOutcome::Created
                    | crate::dist::continuity::SubmitOutcome::Duplicate
            ) =>
        {
            wait_for_startup_terminal_state(&identity, &decision.record.attempt_id);
        }
        Ok(decision) => {
            let reason = if decision.record.error.is_empty() {
                decision.outcome.as_str().to_string()
            } else {
                decision.record.error.clone()
            };
            log_startup_rejected(
                &identity,
                Some(&decision.record.attempt_id),
                Some(&decision.record.owner_node),
                Some(&decision.record.replica_node),
                &reason,
            );
        }
        Err(reason) => {
            log_startup_rejected(&identity, None, None, None, &reason);
        }
    }
}

extern "C" fn startup_keepalive_entry(_args: *const u8) {
    loop {
        crate::actor::mesh_timer_sleep(STARTUP_KEEPALIVE_SLEEP_MS);
    }
}

fn spawn_startup_work_actor(runtime_name: &str) {
    let args_ptr = startup_work_arg_payload(runtime_name);
    crate::actor::mesh_actor_spawn(
        startup_work_entry as *const u8,
        args_ptr,
        std::mem::size_of::<u64>() as u64,
        1,
    );
}

fn spawn_startup_keepalive_actor() {
    crate::actor::mesh_actor_spawn(startup_keepalive_entry as *const u8, std::ptr::null(), 0, 2);
}

fn trigger_startup_work_registrations<F, G>(
    runtime_names: &[String],
    cluster_mode: bool,
    cluster_role: crate::dist::continuity::ContinuityClusterRole,
    promotion_epoch: u64,
    mut spawn_startup: F,
    mut spawn_keepalive: G,
) where
    F: FnMut(&str),
    G: FnMut(),
{
    if runtime_names.is_empty() {
        return;
    }

    if cluster_mode && !STARTUP_KEEPALIVE_SPAWNED.swap(true, Ordering::SeqCst) {
        spawn_keepalive();
        log_startup_keepalive(runtime_names.len());
    }

    for runtime_name in runtime_names {
        let identity = match startup_work_identity(runtime_name) {
            Ok(identity) => identity,
            Err(reason) => {
                log_startup_rejected_without_identity(runtime_name, &reason);
                continue;
            }
        };

        if lookup_declared_handler(&identity.runtime_name).is_none() {
            log_startup_rejected(&identity, None, None, None, STARTUP_HANDLER_MISSING);
            continue;
        }

        if cluster_mode && cluster_role == crate::dist::continuity::ContinuityClusterRole::Standby {
            log_startup_skipped(&identity, cluster_role, promotion_epoch);
            continue;
        }

        spawn_startup(&identity.runtime_name);
    }
}

pub(crate) fn declared_work_execution_node() -> String {
    node_state()
        .map(|state| state.name.clone())
        .unwrap_or_else(|| DECLARED_WORK_LOCAL_NODE.to_string())
}

pub(crate) fn complete_declared_work(
    request_key: &str,
    attempt_id: &str,
) -> Result<crate::dist::continuity::ContinuityRecord, String> {
    crate::dist::continuity::continuity_registry().mark_completed(
        request_key,
        attempt_id,
        &declared_work_execution_node(),
    )
}

fn automatic_recovery_arg_payload(
    runtime_name: &str,
    request_key: &str,
    payload_hash: &str,
    previous_attempt_id: &str,
) -> *mut u8 {
    let runtime_name_ptr = crate::string::mesh_str(runtime_name);
    let request_key_ptr = crate::string::mesh_str(request_key);
    let payload_hash_ptr = crate::string::mesh_str(payload_hash);
    let previous_attempt_id_ptr = crate::string::mesh_str(previous_attempt_id);
    allocate_remote_spawn_args(&[
        runtime_name_ptr as u64,
        request_key_ptr as u64,
        payload_hash_ptr as u64,
        previous_attempt_id_ptr as u64,
    ])
}

fn mesh_string_arg_to_owned(raw: u64) -> String {
    if raw == 0 {
        String::new()
    } else {
        unsafe {
            (*(raw as *const crate::string::MeshString))
                .as_str()
                .to_string()
        }
    }
}

extern "C" fn automatic_recovery_submit_entry(args: *const u8) {
    let words = unsafe { std::slice::from_raw_parts(args as *const u64, 4) };
    let runtime_name = mesh_string_arg_to_owned(words[0]);
    let request_key = mesh_string_arg_to_owned(words[1]);
    let payload_hash = mesh_string_arg_to_owned(words[2]);
    let previous_attempt_id = mesh_string_arg_to_owned(words[3]);

    let desired_required_replica_count =
        match required_replica_count_for_runtime_name(&runtime_name) {
            Ok(value) => value,
            Err(reason) => {
                log_automatic_recovery_rejected(&request_key, &previous_attempt_id, &reason);
                return;
            }
        };
    let required_replica_count = automatic_recovery_effective_required_replica_count(
        &request_key,
        desired_required_replica_count,
        canonical_declared_membership().len() > 1,
    );

    match submit_declared_work(
        &runtime_name,
        &request_key,
        &payload_hash,
        required_replica_count,
    ) {
        Ok(decision)
            if decision.outcome == crate::dist::continuity::SubmitOutcome::Created
                && decision.record.attempt_id != previous_attempt_id =>
        {
            log_automatic_recovery(
                &previous_attempt_id,
                &decision.record.attempt_id,
                &request_key,
                &runtime_name,
            );
        }
        Ok(decision) => {
            log_automatic_recovery_rejected(
                &request_key,
                &previous_attempt_id,
                &format!("automatic_recovery_rejected:{}", decision.outcome.as_str()),
            );
        }
        Err(reason) => {
            log_automatic_recovery_rejected(&request_key, &previous_attempt_id, &reason);
        }
    }
}

fn spawn_automatic_recovery_submission(
    runtime_name: &str,
    request_key: &str,
    payload_hash: &str,
    previous_attempt_id: &str,
) {
    let args_ptr = automatic_recovery_arg_payload(
        runtime_name,
        request_key,
        payload_hash,
        previous_attempt_id,
    );
    crate::actor::mesh_actor_spawn(
        automatic_recovery_submit_entry as *const u8,
        args_ptr,
        (4 * std::mem::size_of::<u64>()) as u64,
        1,
    );
}

fn spawn_declared_work_local(entry: &DeclaredHandlerEntry, request_key: &str, attempt_id: &str) {
    let (args_ptr, _tags) = declared_work_arg_payload(request_key, attempt_id);
    crate::actor::mesh_actor_spawn(entry.fn_ptr.0, args_ptr, 16, 1);
}

fn spawn_declared_work_remote(
    owner_node: &str,
    entry: &DeclaredHandlerEntry,
    request_key: &str,
    attempt_id: &str,
) -> Result<(), String> {
    let (args_ptr, arg_tags) = declared_work_arg_payload(request_key, attempt_id);
    let pid = mesh_node_spawn(
        owner_node.as_ptr(),
        owner_node.len() as u64,
        entry.executable_name.as_ptr(),
        entry.executable_name.len() as u64,
        args_ptr,
        16,
        arg_tags.as_ptr(),
        arg_tags.len() as u64,
        0,
    );
    if pid == 0 {
        Err(format!(
            "declared_work_remote_spawn_failed:{}:{}",
            owner_node, entry.executable_name
        ))
    } else {
        Ok(())
    }
}

fn log_automatic_promotion(previous_epoch: u64, next_epoch: u64, disconnected_node: &str) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "automatic_promotion".to_string(),
        cluster_role: Some("primary".to_string()),
        promotion_epoch: Some(next_epoch),
        reason: Some(format!("peer_lost:{disconnected_node}")),
        metadata: vec![
            ("previous_epoch".to_string(), previous_epoch.to_string()),
            (
                "disconnected_node".to_string(),
                disconnected_node.to_string(),
            ),
        ],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt continuity] transition=automatic_promotion disconnected_node={} previous_epoch={} next_epoch={}",
        disconnected_node, previous_epoch, next_epoch,
    );
}

fn log_automatic_promotion_rejected(
    disconnected_node: &str,
    reason: &str,
    authority: crate::dist::continuity::ContinuityAuthorityStatus,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "automatic_promotion_rejected".to_string(),
        cluster_role: Some(authority.cluster_role.as_str().to_string()),
        promotion_epoch: Some(authority.promotion_epoch),
        replication_health: Some(authority.replication_health.as_str().to_string()),
        reason: Some(reason.to_string()),
        metadata: vec![(
            "disconnected_node".to_string(),
            disconnected_node.to_string(),
        )],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt continuity] transition=automatic_promotion_rejected disconnected_node={} cluster_role={} promotion_epoch={} replication_health={} reason={}",
        disconnected_node,
        authority.cluster_role.as_str(),
        authority.promotion_epoch,
        authority.replication_health.as_str(),
        reason,
    );
}

fn log_automatic_recovery(
    previous_attempt_id: &str,
    next_attempt_id: &str,
    request_key: &str,
    runtime_name: &str,
) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "automatic_recovery".to_string(),
        request_key: Some(request_key.to_string()),
        attempt_id: Some(next_attempt_id.to_string()),
        metadata: vec![
            (
                "previous_attempt_id".to_string(),
                previous_attempt_id.to_string(),
            ),
            ("runtime_name".to_string(), runtime_name.to_string()),
        ],
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt continuity] transition=automatic_recovery request_key={} previous_attempt_id={} next_attempt_id={} runtime_name={}",
        crate::dist::continuity::request_key_fingerprint(request_key),
        previous_attempt_id,
        next_attempt_id,
        runtime_name,
    );
}

fn log_automatic_recovery_rejected(request_key: &str, previous_attempt_id: &str, reason: &str) {
    crate::dist::operator::record_diagnostic(crate::dist::operator::OperatorDiagnosticRecord {
        transition: "automatic_recovery_rejected".to_string(),
        request_key: Some(request_key.to_string()),
        attempt_id: Some(previous_attempt_id.to_string()),
        reason: Some(reason.to_string()),
        ..crate::dist::operator::OperatorDiagnosticRecord::default()
    });
    eprintln!(
        "[mesh-rt continuity] transition=automatic_recovery_rejected request_key={} previous_attempt_id={} reason={}",
        crate::dist::continuity::request_key_fingerprint(request_key),
        previous_attempt_id,
        reason,
    );
}

fn automatic_promotion_reason(
    local_node: &str,
    disconnected_node: &str,
    remaining_peer_count: usize,
    authority: crate::dist::continuity::ContinuityAuthorityStatus,
    snapshot: &crate::dist::continuity::ContinuitySnapshot,
) -> Result<(), &'static str> {
    use crate::dist::continuity::{
        ContinuityClusterRole, ContinuityPhase, ContinuityResult, ReplicaStatus,
    };

    if authority.cluster_role != ContinuityClusterRole::Standby {
        return Err(AUTOMATIC_PROMOTION_REJECTED_NOT_STANDBY);
    }
    if remaining_peer_count != 0 {
        return Err(AUTOMATIC_PROMOTION_REJECTED_PEERS_REMAINING);
    }

    let mut promotable_records = 0usize;
    for record in snapshot.records.iter().filter(|record| {
        record.phase == ContinuityPhase::Submitted && record.result == ContinuityResult::Pending
    }) {
        if record.cluster_role != ContinuityClusterRole::Standby {
            return Err(AUTOMATIC_PROMOTION_REJECTED_AMBIGUOUS_PENDING);
        }
        if record.owner_node == disconnected_node
            && record.replica_node == local_node
            && matches!(
                record.replica_status,
                ReplicaStatus::Preparing | ReplicaStatus::Mirrored
            )
        {
            promotable_records += 1;
            continue;
        }
        return Err(AUTOMATIC_PROMOTION_REJECTED_AMBIGUOUS_PENDING);
    }

    if promotable_records == 0 {
        return Err(AUTOMATIC_PROMOTION_REJECTED_NO_MIRRORED_STATE);
    }

    Ok(())
}

fn automatic_recovery_candidates(
    disconnected_node: &str,
    snapshot: &crate::dist::continuity::ContinuitySnapshot,
) -> Vec<(String, String, String, String)> {
    use crate::dist::continuity::{
        ContinuityClusterRole, ContinuityPhase, ContinuityResult, ReplicaStatus,
    };

    snapshot
        .records
        .iter()
        .filter(|record| {
            record.phase == ContinuityPhase::Submitted
                && record.result == ContinuityResult::Pending
                && record.cluster_role == ContinuityClusterRole::Primary
                && record.replica_status == ReplicaStatus::OwnerLost
                && record.owner_node == disconnected_node
        })
        .map(|record| {
            (
                record.request_key.clone(),
                record.attempt_id.clone(),
                record.payload_hash.clone(),
                record.declared_handler_runtime_name.clone(),
            )
        })
        .collect()
}

fn maybe_automatic_promote_and_resume(disconnected_node: &str) {
    let state = started_node();

    let registry = crate::dist::continuity::continuity_registry();
    let authority = registry.authority_status();
    let snapshot = registry.snapshot();
    let local_node = state.name.clone();
    let remaining_peer_count = state.sessions.read().len();

    if let Err(reason) = automatic_promotion_reason(
        &local_node,
        disconnected_node,
        remaining_peer_count,
        authority,
        &snapshot,
    ) {
        log_automatic_promotion_rejected(disconnected_node, reason, authority);
        return;
    }

    let previous_epoch = authority.promotion_epoch;
    let _promoted = match registry.promote_authority() {
        Ok(promoted) => promoted,
        Err(reason) => {
            log_automatic_promotion_rejected(
                disconnected_node,
                &reason,
                registry.authority_status(),
            );
            return;
        }
    };
    let promoted_epoch = registry.authority_status().promotion_epoch;
    log_automatic_promotion(previous_epoch, promoted_epoch, disconnected_node);

    let promoted_snapshot = registry.snapshot();
    for (request_key, previous_attempt_id, payload_hash, runtime_name) in
        automatic_recovery_candidates(disconnected_node, &promoted_snapshot)
    {
        if runtime_name.is_empty() {
            log_automatic_recovery_rejected(
                &request_key,
                &previous_attempt_id,
                AUTOMATIC_RECOVERY_REJECTED_HANDLER_MISSING,
            );
            continue;
        }

        spawn_automatic_recovery_submission(
            &runtime_name,
            &request_key,
            &payload_hash,
            &previous_attempt_id,
        );
    }
}

struct DeclaredHandlerSubmission {
    entry: DeclaredHandlerEntry,
    placement: DeclaredWorkPlacement,
    decision: crate::dist::continuity::SubmitDecision,
}

fn prepare_declared_handler_submission(
    runtime_name: &str,
    request_key: &str,
    payload_hash: &str,
    required_replica_count: u64,
    request_payload: &[u8],
) -> Result<DeclaredHandlerSubmission, String> {
    let entry = lookup_declared_handler(runtime_name)
        .ok_or_else(|| format!("declared_handler_not_registered:{runtime_name}"))?;
    let placement = declared_work_placement(request_key, runtime_name)?;
    let authority = crate::dist::continuity::continuity_registry().authority_status();
    let replica_nodes =
        match select_continuity_replica_set(&placement.owner_node, entry.replication_count) {
            Ok(replica_nodes) => replica_nodes,
            Err(reason) if reason.starts_with("replica_capacity_unavailable:") => Vec::new(),
            Err(reason) => return Err(reason),
        };
    let replica_node = replica_nodes.first().cloned().unwrap_or_default();
    let request = crate::dist::continuity::SubmitRequest {
        request_key: request_key.to_string(),
        payload_hash: payload_hash.to_string(),
        request_payload: request_payload.to_vec(),
        ingress_node: placement.ingress_node.clone(),
        owner_node: placement.owner_node.clone(),
        replica_nodes,
        replica_node,
        replication_count: entry.replication_count,
        required_replica_count,
        routed_remotely: placement.routed_remotely,
        fell_back_locally: placement.fell_back_locally,
        cluster_role: authority.cluster_role,
        promotion_epoch: authority.promotion_epoch,
        declared_handler_runtime_name: runtime_name.to_string(),
    };

    let decision = crate::dist::continuity::continuity_registry().submit(request)?;
    Ok(DeclaredHandlerSubmission {
        entry,
        placement,
        decision,
    })
}

fn rejected_submit_reason(decision: &crate::dist::continuity::SubmitDecision) -> String {
    if !decision.record.error.is_empty() {
        return decision.record.error.clone();
    }
    if !decision.conflict_reason.is_empty() {
        return decision.conflict_reason.clone();
    }
    format!(
        "declared_handler_submit_rejected:{}",
        decision.outcome.as_str()
    )
}

pub fn submit_declared_work(
    runtime_name: &str,
    request_key: &str,
    payload_hash: &str,
    required_replica_count: u64,
) -> Result<crate::dist::continuity::SubmitDecision, String> {
    let prepared = prepare_declared_handler_submission(
        runtime_name,
        request_key,
        payload_hash,
        required_replica_count,
        &[],
    )?;
    if prepared.decision.outcome != crate::dist::continuity::SubmitOutcome::Created {
        return Ok(prepared.decision);
    }
    if prepared.decision.record.phase == crate::dist::continuity::ContinuityPhase::Rejected {
        return Ok(prepared.decision);
    }

    maybe_hold_startup_work_dispatch(
        runtime_name,
        &prepared.decision.record.request_key,
        required_replica_count,
    );

    let dispatch_result = if prepared.placement.routed_remotely {
        spawn_declared_work_remote(
            &prepared.decision.record.owner_node,
            &prepared.entry,
            &prepared.decision.record.request_key,
            &prepared.decision.record.attempt_id,
        )
    } else {
        spawn_declared_work_local(
            &prepared.entry,
            &prepared.decision.record.request_key,
            &prepared.decision.record.attempt_id,
        );
        Ok(())
    };

    match dispatch_result {
        Ok(()) => Ok(prepared.decision),
        Err(reason) => {
            let rejected = crate::dist::continuity::continuity_registry().reject_durable_request(
                &prepared.decision.record.request_key,
                &prepared.decision.record.attempt_id,
                &reason,
            )?;
            Ok(crate::dist::continuity::SubmitDecision {
                outcome: crate::dist::continuity::SubmitOutcome::Rejected,
                record: rejected,
                conflict_reason: String::new(),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// mesh_node_spawn -- spawn an actor on a remote node
// ---------------------------------------------------------------------------

/// Spawn an actor on a remote node and return its PID.
///
/// Called from compiled Mesh code via `Node.spawn(node, function, args)` or
/// `Node.spawn_link(node, function, args)`. Sends a DIST_SPAWN request to the
/// target node containing the function name and packed argument buffer. Blocks
/// the calling actor (yields coroutine) until the remote node replies with the
/// spawned PID via DIST_SPAWN_REPLY.
///
/// # Arguments
/// - `node_ptr`, `node_len`: Target node name (UTF-8 bytes)
/// - `fn_name_ptr`, `fn_name_len`: Function name to spawn (UTF-8 bytes)
/// - `args_ptr`, `args_size`: Packed raw argument values (u64 words)
/// - `arg_tags_ptr`, `arg_count`: Per-argument runtime type tags for deep-copying remote values
/// - `link_flag`: 0 = spawn, 1 = spawn_link (establishes bidirectional link)
///
/// # Returns
/// - Remote PID (u64) on success
/// - 0 on failure (not connected, function not found, write error, etc.)
#[no_mangle]
pub extern "C-unwind" fn mesh_node_spawn(
    node_ptr: *const u8,
    node_len: u64,
    fn_name_ptr: *const u8,
    fn_name_len: u64,
    args_ptr: *const u8,
    args_size: u64,
    arg_tags_ptr: *const u8,
    arg_count: u64,
    link_flag: u8,
) -> u64 {
    use crate::actor::process::ProcessId;

    // The process asking, which a linked spawn links to. Outside an actor (a
    // node's main, or a runtime thread moving declared work) the wait for
    // the reply below blocks instead of yielding.
    let my_pid = crate::actor::stack::get_current_pid().unwrap_or(ProcessId(0));

    let state = match node_state() {
        Some(s) => s,
        None => return 0,
    };

    let node_name = unsafe {
        if node_ptr.is_null() {
            return 0;
        }
        std::str::from_utf8(std::slice::from_raw_parts(node_ptr, node_len as usize)).unwrap_or("")
    };

    let fn_name = unsafe {
        if fn_name_ptr.is_null() {
            return 0;
        }
        std::str::from_utf8(std::slice::from_raw_parts(
            fn_name_ptr,
            fn_name_len as usize,
        ))
        .unwrap_or("")
    };

    if node_name.is_empty() || fn_name.is_empty() {
        return 0;
    }

    // Look up session for the target node. If the cached session is already
    // gone, try to re-establish it once before failing the spawn.
    let mut session = {
        let sessions = state.sessions.read();
        sessions.get(node_name).cloned()
    }
    .or_else(|| match connect_to_remote_node(state, node_name) {
        Ok(session) => Some(session),
        Err(error) => {
            eprintln!(
                "mesh node spawn failed target={} fn={}: {}",
                node_name, fn_name, error
            );
            None
        }
    });

    let mut session = match session.take() {
        Some(session) => session,
        None => return 0,
    };

    // Generate a unique request ID for correlation.
    let req_id = SPAWN_REQUEST_ID.fetch_add(1, Ordering::Relaxed);

    // Register pending spawn so the reader thread can route the reply.
    let (reply, answer) = crate::actor::cooperative_channel();
    session
        .pending_spawns
        .lock()
        .unwrap()
        .insert(req_id, reply.clone());

    // Copy args data immediately (do NOT retain pointer to GC heap) and encode
    // remote-safe values using the compile-time tags supplied by codegen.
    let args_data = if args_ptr.is_null() || args_size == 0 {
        &[] as &[u8]
    } else {
        unsafe { std::slice::from_raw_parts(args_ptr, args_size as usize) }
    };
    let arg_tags = if arg_count == 0 {
        &[] as &[u8]
    } else {
        if arg_tags_ptr.is_null() {
            session.pending_spawns.lock().unwrap().remove(&req_id);
            return 0;
        }
        unsafe { std::slice::from_raw_parts(arg_tags_ptr, arg_count as usize) }
    };
    let encoded_args = match encode_remote_spawn_args(args_data, arg_tags) {
        Ok(encoded) => encoded,
        Err(reason) => {
            eprintln!(
                "mesh node spawn failed target={} fn={}: {}",
                node_name, fn_name, reason
            );
            session.pending_spawns.lock().unwrap().remove(&req_id);
            return 0;
        }
    };

    // Build DIST_SPAWN payload.
    let fn_name_bytes = fn_name.as_bytes();
    let mut payload =
        Vec::with_capacity(1 + 8 + 8 + 1 + 2 + fn_name_bytes.len() + encoded_args.len());
    payload.push(DIST_SPAWN);
    payload.extend_from_slice(&req_id.to_le_bytes());
    payload.extend_from_slice(&my_pid.as_u64().to_le_bytes());
    payload.push(link_flag);
    payload.extend_from_slice(&(fn_name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(fn_name_bytes);
    payload.extend_from_slice(&encoded_args);

    // Send the request over the TLS stream. If the cached stream is stale,
    // tear it down, reconnect once, and retry the same request on the fresh
    // authenticated session.
    {
        record_peer_original_attempt(node_name, Instant::now());
        let write_result = session.send(OutboundClass::Application, payload.clone());

        if write_result.is_err() {
            eprintln!(
                "mesh node spawn failed target={} fn={}: write_error",
                node_name, fn_name
            );
            session.pending_spawns.lock().unwrap().remove(&req_id);
            session.shutdown.store(true, Ordering::SeqCst);
            cleanup_session_if_current(&session);

            if !allow_peer_retry(node_name, Instant::now()) {
                eprintln!(
                    "mesh node spawn failed target={} fn={}: retry_budget_exhausted",
                    node_name, fn_name
                );
                return 0;
            }
            let jitter_millis = rand::random_range(0..=100_u64);
            std::thread::park_timeout(Duration::from_millis(jitter_millis));

            session = match connect_to_remote_node(state, node_name) {
                Ok(new_session) => new_session,
                Err(error) => {
                    eprintln!(
                        "mesh node spawn failed target={} fn={}: reconnect_error: {}",
                        node_name, fn_name, error
                    );
                    return 0;
                }
            };
            session.pending_spawns.lock().unwrap().insert(req_id, reply);

            let retry_result = session.send(OutboundClass::Application, payload);
            if retry_result.is_err() {
                eprintln!(
                    "mesh node spawn failed target={} fn={}: write_error_after_reconnect",
                    node_name, fn_name
                );
                session.pending_spawns.lock().unwrap().remove(&req_id);
                return 0;
            }
        }
    }

    // Wait for DIST_SPAWN_REPLY, or for the session to end without one.
    let result = crate::actor::cooperative_recv_timeout(&answer, REMOTE_SPAWN_TIMEOUT)
        .unwrap_or_else(|_| Err("remote_spawn_reply_timeout".to_string()));
    session.pending_spawns.lock().unwrap().remove(&req_id);
    match result {
        Ok(spawned_local_id) => {
            let remote_pid =
                ProcessId::from_remote(session.node_id, session.remote_creation, spawned_local_id);
            if link_flag == 1 {
                if let Some(process) = crate::actor::process(my_pid) {
                    process.lock().links.insert(remote_pid);
                }
            }
            remote_pid.as_u64()
        }
        Err(reason) => {
            eprintln!(
                "mesh node spawn failed target={} fn={}: {} request_id={}",
                node_name, fn_name, reason, req_id
            );
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dist::bootstrap::{BootstrapInputs, BootstrapMode};

    extern "C" fn startup_work_test_declared_handler(_args: *const u8) {}

    fn startup_work_test_lock() -> parking_lot::RwLockWriteGuard<'static, ()> {
        declared_handler_registry_test_lock()
    }

    fn clear_startup_work_test_state() {
        startup_work_registry().write().clear();
        declared_handler_registry().write().clear();
        STARTUP_KEEPALIVE_SPAWNED.store(false, Ordering::SeqCst);
        STARTUP_WORK_TRIGGERED.store(false, Ordering::SeqCst);
    }

    struct StartupWorkDelayEnvGuard {
        original: Option<std::ffi::OsString>,
    }

    impl Drop for StartupWorkDelayEnvGuard {
        fn drop(&mut self) {
            match self.original.take() {
                Some(value) => std::env::set_var(STARTUP_WORK_DELAY_ENV, value),
                None => std::env::remove_var(STARTUP_WORK_DELAY_ENV),
            }
        }
    }

    fn set_startup_work_delay_env(value: Option<&str>) -> StartupWorkDelayEnvGuard {
        let original = std::env::var_os(STARTUP_WORK_DELAY_ENV);
        match value {
            Some(value) => std::env::set_var(STARTUP_WORK_DELAY_ENV, value),
            None => std::env::remove_var(STARTUP_WORK_DELAY_ENV),
        }
        StartupWorkDelayEnvGuard { original }
    }

    fn register_startup_work_test_handler(runtime_name: &str) {
        mesh_register_declared_handler(
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            2,
            startup_work_test_declared_handler as *const u8,
        );
    }

    #[test]
    fn declared_handler_registry_preserves_replication_count_by_runtime_name() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();

        let default_runtime = "Work.handle_submit";
        let explicit_runtime = "Work.handle_retry";
        let default_exec = "__declared_work_work_handle_submit";
        let explicit_exec = "__declared_work_work_handle_retry";

        mesh_register_declared_handler(
            default_runtime.as_ptr(),
            default_runtime.len() as u64,
            default_exec.as_ptr(),
            default_exec.len() as u64,
            2,
            startup_work_test_declared_handler as *const u8,
        );
        mesh_register_declared_handler(
            explicit_runtime.as_ptr(),
            explicit_runtime.len() as u64,
            explicit_exec.as_ptr(),
            explicit_exec.len() as u64,
            3,
            startup_work_test_declared_handler as *const u8,
        );

        let default_entry = lookup_declared_handler(default_runtime).expect("default handler");
        assert_eq!(default_entry.executable_name, default_exec);
        assert_eq!(default_entry.replication_count, 2);

        let explicit_entry = lookup_declared_handler(explicit_runtime).expect("explicit handler");
        assert_eq!(explicit_entry.executable_name, explicit_exec);
        assert_eq!(explicit_entry.replication_count, 3);
    }

    #[test]
    fn required_replica_count_derives_from_registered_handler_metadata() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();

        let local_runtime = "Work.handle_local";
        let mirrored_runtime = "Work.handle_submit";
        let explicit_runtime = "Work.handle_retry";
        let executable = "__declared_work_test";

        mesh_register_declared_handler(
            local_runtime.as_ptr(),
            local_runtime.len() as u64,
            executable.as_ptr(),
            executable.len() as u64,
            1,
            startup_work_test_declared_handler as *const u8,
        );
        mesh_register_declared_handler(
            mirrored_runtime.as_ptr(),
            mirrored_runtime.len() as u64,
            executable.as_ptr(),
            executable.len() as u64,
            2,
            startup_work_test_declared_handler as *const u8,
        );
        mesh_register_declared_handler(
            explicit_runtime.as_ptr(),
            explicit_runtime.len() as u64,
            executable.as_ptr(),
            executable.len() as u64,
            3,
            startup_work_test_declared_handler as *const u8,
        );

        assert_eq!(
            required_replica_count_for_runtime_name(local_runtime).unwrap(),
            0
        );
        assert_eq!(
            required_replica_count_for_runtime_name(mirrored_runtime).unwrap(),
            1
        );
        assert_eq!(
            required_replica_count_for_runtime_name(explicit_runtime).unwrap(),
            2
        );
    }

    #[test]
    fn declared_handler_registry_rejects_empty_runtime_or_executable_names() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();

        let explicit_exec = "__declared_work_work_handle_submit";
        mesh_register_declared_handler(
            b"".as_ptr(),
            0,
            explicit_exec.as_ptr(),
            explicit_exec.len() as u64,
            2,
            startup_work_test_declared_handler as *const u8,
        );

        let runtime_name = "Work.handle_submit";
        mesh_register_declared_handler(
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            b"".as_ptr(),
            0,
            2,
            startup_work_test_declared_handler as *const u8,
        );

        assert!(lookup_declared_handler(runtime_name).is_none());
        assert!(declared_handler_registry().read().is_empty());
    }

    #[test]
    fn startup_work_registration_deduplicates_runtime_names_and_keeps_stable_identity() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();

        let runtime_name = "Runtime__startup_work";
        mesh_register_startup_work(runtime_name.as_ptr(), runtime_name.len() as u64);
        mesh_register_startup_work(runtime_name.as_ptr(), runtime_name.len() as u64);
        mesh_register_startup_work(b"".as_ptr(), 0);

        let registrations = startup_work_registry().read().clone();
        assert_eq!(registrations, vec![runtime_name.to_string()]);

        let identity = startup_work_identity(runtime_name).expect("identity");
        assert_eq!(identity.request_key, startup_request_key(runtime_name));
        assert_eq!(identity.payload_hash, startup_payload_hash(runtime_name));
        assert_eq!(identity, startup_work_identity(runtime_name).unwrap());
    }

    #[test]
    fn startup_work_dispatch_window_falls_back_to_default_when_env_is_missing() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();
        let _env = set_startup_work_delay_env(None);

        assert_eq!(
            startup_dispatch_window_ms(&startup_request_key("Work.handle_submit"), 1),
            STARTUP_CLUSTERED_PENDING_WINDOW_MS
        );
    }

    #[test]
    fn startup_work_dispatch_window_uses_positive_env_override_for_clustered_startup_requests() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();

        let _short_env = set_startup_work_delay_env(Some("1"));
        assert_eq!(
            startup_dispatch_window_ms(&startup_request_key("Work.handle_submit"), 1),
            1
        );
        drop(_short_env);

        let _long_env = set_startup_work_delay_env(Some("20000"));
        assert_eq!(
            startup_dispatch_window_ms(&startup_request_key("Work.handle_submit"), 1),
            20_000
        );
    }

    #[test]
    fn startup_work_dispatch_window_falls_back_to_default_for_zero_negative_or_malformed_env() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();

        for raw in ["0", "-5", "not-a-number"] {
            let _env = set_startup_work_delay_env(Some(raw));
            assert_eq!(
                startup_dispatch_window_ms(&startup_request_key("Work.handle_submit"), 1),
                STARTUP_CLUSTERED_PENDING_WINDOW_MS,
                "expected default fallback for {raw:?}"
            );
        }
    }

    #[test]
    fn startup_work_dispatch_window_keeps_zero_delay_for_non_startup_or_replica_free_requests() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();
        let _env = set_startup_work_delay_env(Some("20000"));

        assert_eq!(startup_dispatch_window_ms("request-1", 1), 0);
        assert_eq!(
            startup_dispatch_window_ms(&startup_request_key("Work.handle_submit"), 0),
            0
        );
        assert_eq!(
            startup_dispatch_window_ms(&startup_request_key("Work.handle_submit"), 1),
            20_000
        );
    }

    #[test]
    fn startup_work_convergence_allows_single_node_cluster_without_peer() {
        let convergence = wait_for_startup_convergence_with(
            || vec!["node-a@127.0.0.1:4370".to_string()],
            || {},
            1,
            2,
        )
        .expect("single-node convergence should succeed");

        assert_eq!(
            convergence.membership,
            vec!["node-a@127.0.0.1:4370".to_string()]
        );
        assert_eq!(convergence.required_replica_count, 0);
        assert!(!convergence.saw_peer);
    }

    #[test]
    fn startup_work_convergence_preserves_unsupported_explicit_count_without_peer() {
        let convergence = wait_for_startup_convergence_with(
            || vec!["node-a@127.0.0.1:4370".to_string()],
            || {},
            2,
            2,
        )
        .expect("single-node convergence should still report unsupported count truth");

        assert_eq!(convergence.required_replica_count, 2);
        assert!(!convergence.saw_peer);
    }

    #[test]
    fn startup_automatic_recovery_relaxes_single_node_required_replica_count() {
        let request_key = startup_request_key("Work.handle_submit");
        assert_eq!(
            automatic_recovery_effective_required_replica_count(&request_key, 1, false),
            0
        );
        assert_eq!(
            automatic_recovery_effective_required_replica_count(&request_key, 1, true),
            1
        );
        assert_eq!(
            automatic_recovery_effective_required_replica_count("request-1", 1, false),
            1
        );
    }

    #[test]
    fn startup_work_convergence_times_out_after_peer_flaps() {
        let snapshots = [
            vec!["node-a@127.0.0.1:4370".to_string()],
            vec![
                "node-a@127.0.0.1:4370".to_string(),
                "node-b@127.0.0.1:4370".to_string(),
            ],
            vec!["node-a@127.0.0.1:4370".to_string()],
            vec![
                "node-a@127.0.0.1:4370".to_string(),
                "node-b@127.0.0.1:4370".to_string(),
            ],
        ];
        let mut next = 0usize;

        let err = wait_for_startup_convergence_with(
            || {
                let index = next.min(snapshots.len() - 1);
                next += 1;
                snapshots[index].clone()
            },
            || {},
            1,
            3,
        )
        .expect_err("flapping peer convergence should fail closed");

        assert_eq!(err, STARTUP_CONVERGENCE_TIMEOUT);
    }

    #[test]
    fn startup_work_trigger_spawns_keepalive_once_for_cluster_mode() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();
        register_startup_work_test_handler("Runtime__startup_work");

        let runtime_names = vec!["Runtime__startup_work".to_string()];
        let mut startup_spawns = Vec::new();
        let mut keepalive_spawns = 0usize;

        trigger_startup_work_registrations(
            &runtime_names,
            true,
            crate::dist::continuity::ContinuityClusterRole::Primary,
            0,
            |runtime_name| {
                startup_spawns.push(runtime_name.to_string());
            },
            || {
                keepalive_spawns += 1;
            },
        );

        trigger_startup_work_registrations(
            &runtime_names,
            true,
            crate::dist::continuity::ContinuityClusterRole::Primary,
            0,
            |runtime_name| {
                startup_spawns.push(format!("repeat:{runtime_name}"));
            },
            || {
                keepalive_spawns += 1;
            },
        );

        assert_eq!(keepalive_spawns, 1, "keepalive should be deduplicated");
        assert_eq!(
            startup_spawns,
            vec![
                "Runtime__startup_work".to_string(),
                "repeat:Runtime__startup_work".to_string(),
            ]
        );
    }

    #[test]
    fn startup_work_trigger_skips_spawn_for_standby_authority() {
        let _guard = startup_work_test_lock();
        clear_startup_work_test_state();
        register_startup_work_test_handler("Runtime__startup_work");

        let runtime_names = vec!["Runtime__startup_work".to_string()];
        let mut startup_spawns = Vec::new();
        let mut keepalive_spawns = 0usize;

        trigger_startup_work_registrations(
            &runtime_names,
            true,
            crate::dist::continuity::ContinuityClusterRole::Standby,
            0,
            |runtime_name| {
                startup_spawns.push(runtime_name.to_string());
            },
            || {
                keepalive_spawns += 1;
            },
        );

        assert_eq!(
            keepalive_spawns, 1,
            "standby should still keep route-free apps alive"
        );
        assert!(
            startup_spawns.is_empty(),
            "standby must not auto-trigger startup work"
        );
    }

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
        let (name, host, port) = parse_node_name("ipv6@[::1]:9010").unwrap();
        assert_eq!(name, "ipv6");
        assert_eq!(host, "::1");
        assert_eq!(port, 9010);

        let (name, host, port) = parse_node_name("ipv6@[::1]").unwrap();
        assert_eq!(name, "ipv6");
        assert_eq!(host, "::1");
        assert_eq!(port, 9000);

        let (name, host, port) = parse_node_name("ipv6@::1").unwrap();
        assert_eq!(name, "ipv6");
        assert_eq!(host, "::1");
        assert_eq!(port, 9000);

        // Invalid port / malformed bracket handling
        assert!(parse_node_name("name@host:abc").is_err());
        assert!(parse_node_name("name@host:99999").is_err());
        assert!(parse_node_name("name@[::1").is_err());

        // Port 0, asking the system for a free port, names only a node that
        // binds; a node to connect to has a port of its own. A name a node
        // binds is parsed as any other.
        assert!(parse_node_name("name@127.0.0.1:0").is_err());
        let bind = |name| super::super::discovery::split_node_name(name, true);
        assert_eq!(bind("name@127.0.0.1:0"), Ok(("name", "127.0.0.1", 0)));
        assert_eq!(bind("name@[::1]:0"), Ok(("name", "::1", 0)));
        assert!(bind("name@[]:0").is_err());
        assert!(bind("name@fe80::1:4000:x").is_err());
        assert!(bind("name@host:port").is_err());
    }

    /// A node started on port 0 advertises a name its peers can parse back
    /// to where it listens, an IPv6 host included.
    #[test]
    fn a_node_on_port_zero_advertises_a_name_that_parses_back() {
        for (host, advertised) in [
            ("127.0.0.1", "zero@127.0.0.1:4100"),
            ("::1", "zero@[::1]:4100"),
        ] {
            let name = advertised_node_name("zero", host, 4100);
            assert_eq!(name, advertised);
            assert_eq!(parse_node_name(&name), Ok(("zero", host, 4100)));
        }
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
        // node_state() returns None when mesh_node_start hasn't been called.
        // NOTE: Since tests share the process, if another test initializes
        // NODE_STATE first, this may return Some. We test the accessor itself.
        let _result = node_state(); // should not panic
    }

    const TEST_BINDING: ChannelBinding = ChannelBinding::TEST_PLAIN_TRANSPORT;

    /// A node's state, as far as the cookie handshake reads it.
    fn handshake_state(name: &str, cookie: impl Into<String>, creation: u8) -> NodeState {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert, key) = generate_ephemeral_cert();
        NodeState {
            name: name.to_string(),
            host: "127.0.0.1".to_string(),
            port: 0,
            cookie: cookie.into(),
            creation: AtomicU8::new(creation),
            next_node_id: AtomicU16::new(1),
            tls_server_config: build_node_server_config(cert, key),
            tls_client_config: build_node_client_config(),
            sessions: RwLock::new(FxHashMap::default()),
            node_id_map: RwLock::new(FxHashMap::default()),
            node_monitors: RwLock::new(FxHashMap::default()),
        }
    }

    #[test]
    fn test_compute_response_deterministic() {
        // Same inputs must produce the same output
        let cookie = "secret_cookie";
        let challenge = [42u8; 32];
        let r1 = compute_response(cookie, &challenge, &TEST_BINDING);
        let r2 = compute_response(cookie, &challenge, &TEST_BINDING);
        assert_eq!(r1, r2);

        // Different challenge produces different output
        let different_challenge = [99u8; 32];
        let r3 = compute_response(cookie, &different_challenge, &TEST_BINDING);
        assert_ne!(r1, r3);
    }

    #[test]
    fn test_verify_response_correct() {
        let cookie = "my_cookie";
        let challenge = generate_challenge();
        let response = compute_response(cookie, &challenge, &TEST_BINDING);
        assert!(verify_response(
            cookie,
            &challenge,
            &TEST_BINDING,
            &response
        ));
    }

    #[test]
    fn test_verify_response_wrong_cookie() {
        let challenge = generate_challenge();
        let response = compute_response("correct_cookie", &challenge, &TEST_BINDING);
        // Wrong cookie should fail verification
        assert!(!verify_response(
            "wrong_cookie",
            &challenge,
            &TEST_BINDING,
            &response
        ));
    }

    #[test]
    fn cookie_response_is_bound_to_the_tls_channel() {
        let cookie = "shared_cookie";
        let challenge = generate_challenge();
        let response = compute_response(cookie, &challenge, &TEST_BINDING);

        // A proof produced on one TLS session must not verify on another, even
        // though the cookie and the challenge are identical.
        let other_session = ChannelBinding([0xA5; 32]);
        assert_ne!(TEST_BINDING, other_session);
        assert!(!verify_response(
            cookie,
            &challenge,
            &other_session,
            &response
        ));
        assert!(verify_response(
            cookie,
            &challenge,
            &TEST_BINDING,
            &response
        ));
    }

    #[test]
    fn test_cookie_keyring_allows_rolling_rotation() {
        let challenge = [99_u8; 32];
        let old_response = compute_response("old-cookie,new-cookie", &challenge, &TEST_BINDING);
        let new_response = compute_response("new-cookie,old-cookie", &challenge, &TEST_BINDING);

        assert!(verify_response(
            "new-cookie,old-cookie",
            &challenge,
            &TEST_BINDING,
            &old_response
        ));
        assert!(verify_response(
            "old-cookie,new-cookie",
            &challenge,
            &TEST_BINDING,
            &new_response
        ));
        assert!(!verify_response(
            "new-cookie",
            &challenge,
            &TEST_BINDING,
            &old_response
        ));
    }

    #[test]
    fn autonomous_cookie_requires_256_bits_of_configured_secret_material() {
        assert_eq!(
            validate_cluster_cookie_strength("short-development-cookie", true),
            Err("autonomous_cluster_cookie_too_short".to_string())
        );
        assert!(validate_cluster_cookie_strength("0123456789abcdef0123456789abcdef", true).is_ok());
        assert!(validate_cluster_cookie_strength("short-development-cookie", false).is_ok());
    }

    /// A node started on port 0 listens on a port of the system's choosing
    /// and advertises it; a process starts one node only.
    #[test]
    fn test_mesh_node_start_binds_listener() {
        let state = test_node();
        assert!(state.port > 0, "port should be assigned");
        assert_eq!(state.name, format!("test-node@127.0.0.1:{}", state.port));
        assert_eq!(state.cookie, TEST_NODE_COOKIE);
        assert_eq!(state.creation(), 1);
        assert!(TcpStream::connect(("127.0.0.1", state.port)).is_ok());
        assert_eq!(start_named_node("again@127.0.0.1:0", TEST_NODE_COOKIE), -1);

        let first = state.assign_node_id();
        assert!(first >= 1 && state.assign_node_id() > first);
    }

    #[test]
    fn test_bootstrap_from_env_returns_standalone_without_starting_node() {
        let status = super::start_from_inputs_for_test(BootstrapInputs::default(), |_, _| {
            panic!("standalone bootstrap should not start the node")
        })
        .expect("standalone bootstrap should succeed");

        assert_eq!(
            status,
            BootstrapStatus {
                mode: BootstrapMode::Standalone,
                node_name: String::new(),
                cluster_port: 4370,
                discovery_seed: String::new(),
            }
        );
    }

    #[test]
    fn test_bootstrap_from_env_uses_explicit_node_name_in_cluster_mode() {
        let inputs = BootstrapInputs {
            cluster_port: Some("4370".to_string()),
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("mesh.internal".to_string()),
            node_name: Some("primary@127.0.0.1:4370".to_string()),
            ..BootstrapInputs::default()
        };

        let mut started = None;
        let status = super::start_from_inputs_for_test(inputs, |name, cookie| {
            started = Some((name.to_string(), cookie.to_string()));
            0
        })
        .expect("cluster bootstrap should succeed with explicit node name");

        assert_eq!(status.mode, BootstrapMode::Cluster);
        assert_eq!(status.mode_label(), "cluster");
        assert_eq!(status.node_name, "primary@127.0.0.1:4370");
        assert_eq!(status.cluster_port, 4370);
        assert_eq!(status.discovery_seed, "mesh.internal");
        assert_eq!(
            started,
            Some((
                "primary@127.0.0.1:4370".to_string(),
                "shared-cookie".to_string(),
            ))
        );
    }

    #[test]
    fn test_bootstrap_from_env_composes_identity_from_node_host_without_explicit_node_name() {
        let inputs = BootstrapInputs {
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("mesh.internal".to_string()),
            node_host: Some("fdaa:0:1::10".to_string()),
            ..BootstrapInputs::default()
        };

        let status = super::start_from_inputs_for_test(inputs, |name, _| {
            assert!(name.ends_with("@[fdaa:0:1::10]:4370"), "{name}");
            0
        })
        .expect("cluster bootstrap should succeed with the host name identity");

        assert_eq!(status.mode, BootstrapMode::Cluster);
        assert!(status.node_name.ends_with("@[fdaa:0:1::10]:4370"));
        assert_eq!(status.cluster_port, 4370);
        assert_eq!(status.discovery_seed, "mesh.internal");
    }

    #[test]
    fn test_bootstrap_from_env_rejects_cluster_hints_without_cookie() {
        let inputs = BootstrapInputs {
            discovery_seed: Some("mesh.internal".to_string()),
            node_name: Some("primary@127.0.0.1:4370".to_string()),
            ..BootstrapInputs::default()
        };

        let error = super::start_from_inputs_for_test(inputs, |_, _| 0).unwrap_err();
        assert_eq!(
            error,
            "MESH_CLUSTER_COOKIE is required when discovery or identity env is set"
        );
    }

    #[test]
    fn test_bootstrap_from_env_rejects_blank_discovery_seed_in_cluster_mode() {
        let inputs = BootstrapInputs {
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("   ".to_string()),
            node_name: Some("primary@127.0.0.1:4370".to_string()),
            ..BootstrapInputs::default()
        };

        let error = super::start_from_inputs_for_test(inputs, |_, _| 0).unwrap_err();
        assert_eq!(
            error,
            "Missing required environment variable MESH_DISCOVERY_SEED"
        );
    }

    #[test]
    fn test_bootstrap_from_env_rejects_malformed_mesh_node_name() {
        let inputs = BootstrapInputs {
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("mesh.internal".to_string()),
            node_name: Some("bad-node".to_string()),
            ..BootstrapInputs::default()
        };

        let error = super::start_from_inputs_for_test(inputs, |_, _| 0).unwrap_err();
        assert_eq!(error, "Invalid MESH_NODE_NAME: expected name@host:port");
    }

    #[test]
    fn test_bootstrap_from_env_rejects_invalid_cluster_port() {
        let inputs = BootstrapInputs {
            cluster_port: Some("0".to_string()),
            ..BootstrapInputs::default()
        };

        let error = super::start_from_inputs_for_test(inputs, |_, _| 0).unwrap_err();
        assert_eq!(
            error,
            "Invalid MESH_CLUSTER_PORT: expected a positive integer"
        );
    }

    #[test]
    fn test_bootstrap_from_env_rejects_explicit_node_name_port_mismatch() {
        let inputs = BootstrapInputs {
            cluster_port: Some("4371".to_string()),
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("mesh.internal".to_string()),
            node_name: Some("primary@127.0.0.1:4370".to_string()),
            ..BootstrapInputs::default()
        };

        let error = super::start_from_inputs_for_test(inputs, |_, _| 0).unwrap_err();
        assert_eq!(
            error,
            "Invalid MESH_NODE_NAME: port must match MESH_CLUSTER_PORT"
        );
    }

    #[test]
    fn test_bootstrap_from_env_surfaces_bind_failures_with_node_identity() {
        let inputs = BootstrapInputs {
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("mesh.internal".to_string()),
            node_name: Some("primary@127.0.0.1:4370".to_string()),
            ..BootstrapInputs::default()
        };

        let error = super::start_from_inputs_for_test(inputs, |_, _| -2).unwrap_err();
        assert_eq!(
            error,
            "mesh bootstrap start failed node=primary@127.0.0.1:4370: listener bind failed"
        );
    }

    /// A clustered HTTP query and its reply cross whole; bytes cut short,
    /// mislabelled or padded are refused rather than misread.
    #[test]
    fn clustered_http_route_frames_round_trip_and_refuse_malformed_bytes() {
        let query = encode_http_route_v2_query_frame(7, "Api.handle", "key", "attempt-1", b"GET /")
            .unwrap();
        assert_eq!(
            decode_http_route_v2_query_frame(&query),
            Ok((
                7,
                "Api.handle".to_string(),
                "key".to_string(),
                "attempt-1".to_string(),
                b"GET /".to_vec()
            ))
        );
        let refused = |bytes: &[u8]| decode_http_route_v2_query_frame(bytes).unwrap_err();
        assert_eq!(
            refused(&query[..8]),
            "clustered_http_route_v2_query_invalid"
        );
        assert_eq!(
            refused(&[&[DIST_HTTP_ROUTE_V2_REPLY], &query[1..]].concat()),
            "clustered_http_route_v2_query_invalid"
        );
        assert_eq!(
            refused(&query[..10]),
            "clustered_http_route_runtime_name_len_missing"
        );
        assert_eq!(
            refused(&query[..12]),
            "clustered_http_route_runtime_name_truncated"
        );
        let mut bad_name = query.clone();
        bad_name[11] = 0xFF;
        assert_eq!(
            refused(&bad_name),
            "clustered_http_route_runtime_name_invalid_utf8"
        );
        let payload_len_at = query.len() - 5 - 4;
        assert_eq!(
            refused(&query[..payload_len_at + 2]),
            "clustered_http_route_payload_len_missing"
        );
        assert_eq!(
            refused(&[&query[..], b"!"].concat()),
            "clustered_http_route_payload_length_mismatch"
        );
        assert_eq!(
            encode_http_route_v2_query_frame(7, &"x".repeat(70_000), "key", "a", b""),
            Err("clustered_http_route_string_too_large:70000".to_string())
        );

        for result in [Ok(b"200 OK".to_vec()), Err("handler_failed".to_string())] {
            let reply = encode_http_route_v2_reply_frame(9, result.clone()).unwrap();
            assert_eq!(decode_http_route_v2_reply_frame(&reply), Ok((9, result)));
        }
        let reply = encode_http_route_v2_reply_frame(9, Err("no".to_string())).unwrap();
        let refused = |bytes: &[u8]| decode_http_route_v2_reply_frame(bytes).unwrap_err();
        assert_eq!(
            refused(&reply[..13]),
            "clustered_http_route_v2_reply_invalid"
        );
        assert_eq!(
            refused(&[&[DIST_HTTP_ROUTE_V2_QUERY], &reply[1..]].concat()),
            "clustered_http_route_v2_reply_invalid"
        );
        assert_eq!(
            refused(&reply[..reply.len() - 1]),
            "clustered_http_route_reply_length_mismatch"
        );
        let mut bad_reason = reply.clone();
        bad_reason[14] = 0xFF;
        assert_eq!(
            refused(&bad_reason),
            "clustered_http_route_reply_reason_invalid_utf8"
        );
        let mut bad_status = reply;
        bad_status[9] = 2;
        assert_eq!(
            refused(&bad_status),
            "invalid_clustered_http_route_reply_status:2"
        );
    }

    #[test]
    fn protocol_two_session_payload_uses_live_versioned_envelope() {
        let negotiated = NegotiatedProtocol {
            version: PROTOCOL_V2,
            capabilities: super::super::protocol::Capabilities::AUTONOMOUS_REQUIRED,
            max_frame_bytes: 4096,
            autonomous_enabled: true,
            disabled_reason: None,
        };
        let correlation = 41_u64;
        let mut payload = vec![DIST_HTTP_ROUTE_V2_QUERY];
        payload.extend_from_slice(&correlation.to_le_bytes());
        payload.extend_from_slice(b"request");

        let frame =
            encode_session_payload(OutboundClass::Application, payload.clone(), &negotiated)
                .expect("encode protocol-two frame");
        let envelope = ProtocolEnvelope::decode(&frame, negotiated.max_frame_bytes)
            .expect("decode protocol-two envelope");
        assert_eq!(envelope.class, MessageClass::Application);
        assert_eq!(envelope.kind, u16::from(DIST_HTTP_ROUTE_V2_QUERY));
        assert_eq!(envelope.correlation_id, correlation);
        assert_eq!(
            decode_session_payload(frame, &negotiated).expect("unwrap session frame"),
            payload
        );
    }

    #[test]
    fn clustered_http_execution_uses_reserved_owner_until_completion_is_observed() {
        let record = crate::dist::continuity::ContinuityRecord {
            request_key: "operation-key".to_string(),
            payload_hash: "sha256:payload".to_string(),
            record_version: 1,
            request_payload: Vec::new(),
            attempt_id: "attempt-1".to_string(),
            phase: crate::dist::continuity::ContinuityPhase::Submitted,
            result: crate::dist::continuity::ContinuityResult::Pending,
            ingress_node: "gateway@127.0.0.1:4300".to_string(),
            owner_node: "worker@127.0.0.1:4301".to_string(),
            replica_nodes: vec!["replica@127.0.0.1:4302".to_string()],
            acknowledged_replica_nodes: vec!["replica@127.0.0.1:4302".to_string()],
            replica_node: "replica@127.0.0.1:4302".to_string(),
            replication_count: 2,
            replica_status: crate::dist::continuity::ReplicaStatus::Mirrored,
            cluster_role: crate::dist::continuity::ContinuityClusterRole::Primary,
            promotion_epoch: 0,
            replication_health: crate::dist::continuity::ReplicationHealth::Healthy,
            execution_node: String::new(),
            routed_remotely: true,
            fell_back_locally: false,
            error: String::new(),
            declared_handler_runtime_name: "Proof.handle".to_string(),
        };

        let execution = clustered_http_execution(vec![1, 2, 3], false, &record);

        assert_eq!(execution.execution_node, record.owner_node);
        assert!(execution.routed_remotely);
    }

    #[test]
    fn leader_sweep_redrives_missing_replica_after_election() {
        let active = crate::dist::continuity::ContinuityRecord {
            request_key: "operation-key".to_string(),
            payload_hash: "sha256:payload".to_string(),
            record_version: 3,
            request_payload: vec![1],
            attempt_id: "attempt-1".to_string(),
            phase: crate::dist::continuity::ContinuityPhase::Submitted,
            result: crate::dist::continuity::ContinuityResult::Pending,
            ingress_node: "gateway@host".to_string(),
            owner_node: "worker@host".to_string(),
            replica_nodes: vec!["controller1@host".to_string()],
            acknowledged_replica_nodes: Vec::new(),
            replica_node: String::new(),
            replication_count: 2,
            replica_status: crate::dist::continuity::ReplicaStatus::DegradedContinuing,
            cluster_role: crate::dist::continuity::ContinuityClusterRole::Primary,
            promotion_epoch: 0,
            replication_health: crate::dist::continuity::ReplicationHealth::Degraded,
            execution_node: String::new(),
            routed_remotely: true,
            fell_back_locally: false,
            error: "replica_lost:controller1@host".to_string(),
            declared_handler_runtime_name: "Proof.handle".to_string(),
        };
        let membership =
            BTreeSet::from(["controller2@host".to_string(), "worker@host".to_string()]);

        assert_eq!(
            missing_continuity_replica_participants(std::slice::from_ref(&active), &membership),
            BTreeSet::from(["controller1@host".to_string()])
        );

        let mut terminal = active;
        terminal.phase = crate::dist::continuity::ContinuityPhase::Completed;
        terminal.result = crate::dist::continuity::ContinuityResult::Succeeded;
        assert!(missing_continuity_replica_participants(&[terminal], &membership).is_empty());
    }

    #[test]
    fn protocol_one_session_payload_remains_wire_compatible() {
        let negotiated = NegotiatedProtocol {
            version: PROTOCOL_V1,
            capabilities: super::super::protocol::Capabilities::default(),
            max_frame_bytes: 4096,
            autonomous_enabled: false,
            disabled_reason: Some("protocol_two_not_negotiated".to_string()),
        };
        let payload = vec![HEARTBEAT_PING, 1, 2, 3];
        assert_eq!(
            encode_session_payload(OutboundClass::Control, payload.clone(), &negotiated)
                .expect("protocol-one frame"),
            payload
        );
    }

    #[test]
    fn negotiated_distribution_reader_accepts_operator_payload_above_handshake_limit() {
        let payload = vec![DIST_OPERATOR_REPLY; 8 * 1024];
        let mut framed = Vec::new();
        write_msg(&mut framed, &payload).expect("frame operator reply");

        assert_eq!(
            read_dist_msg_bounded(&mut std::io::Cursor::new(framed), 16 * 1024)
                .expect("read negotiated distribution frame"),
            payload
        );
    }

    #[test]
    fn persistent_frame_reader_preserves_partial_frame_across_timeouts() {
        struct ScriptedRead {
            steps: std::collections::VecDeque<Result<Vec<u8>, io::ErrorKind>>,
        }

        impl Read for ScriptedRead {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                match self.steps.pop_front().expect("scripted read step") {
                    Ok(bytes) => {
                        assert!(bytes.len() <= output.len());
                        output[..bytes.len()].copy_from_slice(&bytes);
                        Ok(bytes.len())
                    }
                    Err(kind) => Err(io::Error::from(kind)),
                }
            }
        }

        let payload = b"raft-frame".to_vec();
        let length = (payload.len() as u32).to_le_bytes();
        let mut input = ScriptedRead {
            steps: std::collections::VecDeque::from([
                Ok(length[..2].to_vec()),
                Err(io::ErrorKind::TimedOut),
                Ok(length[2..].to_vec()),
                Ok(payload[..3].to_vec()),
                Err(io::ErrorKind::WouldBlock),
                Ok(payload[3..].to_vec()),
            ]),
        };
        let mut reader = PersistentFrameReader::default();
        assert_eq!(reader.read_next(&mut input, 1024).unwrap(), None);
        assert_eq!(reader.read_next(&mut input, 1024).unwrap(), None);
        assert_eq!(reader.read_next(&mut input, 1024).unwrap(), Some(payload));

        // An empty frame is whole at once; an interrupted read is retried;
        // a stream that ends, or fails, mid-frame fails the read.
        let mut input = ScriptedRead {
            steps: std::collections::VecDeque::from([
                Ok(0u32.to_le_bytes().to_vec()),
                Err(io::ErrorKind::Interrupted),
                Ok(1u32.to_le_bytes().to_vec()),
                Ok(Vec::new()),
                Ok(1u32.to_le_bytes().to_vec()),
                Err(io::ErrorKind::ConnectionReset),
            ]),
        };
        let mut reader = PersistentFrameReader::default();
        assert_eq!(
            reader.read_next(&mut input, 1024).unwrap(),
            Some(Vec::new())
        );
        let failure = |result: io::Result<Option<Vec<u8>>>| result.unwrap_err().kind();
        assert_eq!(
            failure(reader.read_next(&mut input, 1024)),
            io::ErrorKind::UnexpectedEof
        );
        let mut reader = PersistentFrameReader::default();
        assert_eq!(
            failure(reader.read_next(&mut input, 1024)),
            io::ErrorKind::ConnectionReset
        );
    }

    /// A persistent session over loopback TLS, with its reader and writer
    /// threads running, to a peer that never writes and reports when each
    /// frame arrives. Set `shutdown`, join the threads, then drop the session
    /// before joining the peer.
    fn quiet_peer_session() -> (
        Arc<NodeSession>,
        mpsc::Receiver<Instant>,
        [std::thread::JoinHandle<()>; 3],
    ) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (arrivals, arrived) = mpsc::channel();
        let peer = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let (cert, key) = generate_ephemeral_cert();
            let mut tls = StreamOwned::new(
                rustls::ServerConnection::new(build_node_server_config(cert, key)).unwrap(),
                tcp,
            );
            // Never writes, so the session's reader only ever times out.
            while read_msg(&mut tls).is_ok() {
                if arrivals.send(Instant::now()).is_err() {
                    break;
                }
            }
        });

        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
        let mut tls = StreamOwned::new(
            rustls::ClientConnection::new(build_node_client_config(), server_name).unwrap(),
            tcp,
        );
        while tls.conn.is_handshaking() {
            tls.conn.complete_io(&mut tls.sock).unwrap();
        }
        let session = Arc::new(NodeSession::new(
            RemoteSessionEndpoint {
                remote_name: "quiet@127.0.0.1".to_string(),
                remote_creation: 1,
                node_id: 1,
                direction: SessionDirection::Outgoing,
            },
            NodeStream::ClientTls(tls),
            true,
            NegotiatedProtocol {
                version: PROTOCOL_V1,
                capabilities: super::super::protocol::Capabilities::default(),
                max_frame_bytes: 4096,
                autonomous_enabled: false,
                disabled_reason: None,
            },
            None,
        ));
        let heartbeat = Arc::new(Mutex::new(HeartbeatState::new(
            Duration::from_secs(60),
            Duration::from_secs(15),
        )));
        let reader = std::thread::spawn({
            let session = Arc::clone(&session);
            move || reader_loop_session(session, heartbeat)
        });
        let writer = std::thread::spawn({
            let session = Arc::clone(&session);
            move || writer_loop_session(session)
        });
        (session, arrived, [reader, writer, peer])
    }

    fn stop_quiet_peer_session(
        session: Arc<NodeSession>,
        [reader, writer, peer]: [std::thread::JoinHandle<()>; 3],
    ) {
        session.shutdown.store(true, Ordering::SeqCst);
        reader.join().unwrap();
        writer.join().unwrap();
        drop(session);
        peer.join().unwrap();
    }

    /// On a connection whose peer is quiet, the reader spends nearly all its
    /// time holding the stream lock inside a read that waits out its poll
    /// timeout, then takes the lock straight back. Unless it hands the lock to
    /// a waiting writer, outbound frames wait for inbound traffic: seconds per
    /// hop, which timed out replica prepares and request dispatch in the
    /// Docker cluster proof.
    #[test]
    fn idle_reader_does_not_starve_outbound_frames() {
        let (session, arrived, threads) = quiet_peer_session();
        let mut latencies = Vec::new();
        for _ in 0..15 {
            std::thread::sleep(Duration::from_millis(40));
            let sent = Instant::now();
            session
                .send(OutboundClass::Control, vec![DIST_PEER_LIST])
                .unwrap();
            let delivered = arrived
                .recv_timeout(Duration::from_secs(5))
                .expect("frame delivered while the peer stays quiet");
            latencies.push(delivered - sent);
        }
        stop_quiet_peer_session(session, threads);

        latencies.sort();
        // The reader's poll timeout is 25 ms, so a writer that is handed the
        // lock waits at most that long; the median leaves room for a loaded host.
        assert!(
            latencies[latencies.len() / 2] < Duration::from_millis(150),
            "outbound frame latencies: {latencies:?}"
        );
    }

    /// Initial sync sends one frame per continuity record through the
    /// 64-frame snapshot lane. After a busy period that is thousands of
    /// frames; dropping the overflow left a new worker `warming` for good.
    #[test]
    fn bulk_sends_wait_for_lane_room_instead_of_dropping_frames() {
        let (session, arrived, threads) = quiet_peer_session();
        let frames = SNAPSHOT_QUEUE_ITEMS * 16;
        for _ in 0..frames {
            session
                .send_waiting(OutboundClass::Snapshot, vec![DIST_PEER_LIST; 512])
                .expect("frame queued once the writer makes room");
        }
        for index in 0..frames {
            arrived
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("frame {index} of {frames} never arrived"));
        }
        stop_quiet_peer_session(session, threads);
    }

    /// Both ends of one session in this process, over TLS, each with its
    /// reader and writer running.
    fn session_pair() -> ([Arc<NodeSession>; 2], Vec<std::thread::JoinHandle<()>>) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let (cert, key) = generate_ephemeral_cert();
            let mut tls = StreamOwned::new(
                rustls::ServerConnection::new(build_node_server_config(cert, key)).unwrap(),
                tcp,
            );
            while tls.conn.is_handshaking() {
                tls.conn.complete_io(&mut tls.sock).unwrap();
            }
            NodeStream::ServerTls(tls)
        });
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
        let mut tls = StreamOwned::new(
            rustls::ClientConnection::new(build_node_client_config(), server_name).unwrap(),
            tcp,
        );
        while tls.conn.is_handshaking() {
            tls.conn.complete_io(&mut tls.sock).unwrap();
        }
        let ends = [
            (NodeStream::ClientTls(tls), SessionDirection::Outgoing),
            (server.join().unwrap(), SessionDirection::Incoming),
        ];
        let mut threads = Vec::new();
        let sessions = ends.map(|(stream, direction)| {
            let session = Arc::new(NodeSession::new(
                RemoteSessionEndpoint {
                    remote_name: format!("{direction:?}@127.0.0.1"),
                    remote_creation: 1,
                    node_id: 1,
                    direction,
                },
                stream,
                true,
                NegotiatedProtocol {
                    version: PROTOCOL_V1,
                    capabilities: super::super::protocol::Capabilities::default(),
                    max_frame_bytes: MAX_DIST_MSG,
                    autonomous_enabled: false,
                    disabled_reason: None,
                },
                None,
            ));
            let heartbeat = Arc::new(Mutex::new(HeartbeatState::new(
                Duration::from_secs(60),
                Duration::from_secs(15),
            )));
            threads.push(std::thread::spawn({
                let session = Arc::clone(&session);
                move || reader_loop_session(session, heartbeat)
            }));
            threads.push(std::thread::spawn({
                let session = Arc::clone(&session);
                move || writer_loop_session(session)
            }));
            session
        });
        (sessions, threads)
    }

    /// A new node and each peer stream their continuity state to each other
    /// at once. A writer waiting for its peer to take bytes held the stream,
    /// so its own reader stopped taking the peer's: with both ends' writers
    /// waiting, neither read again, the session stayed "healthy", and the
    /// new worker never joined (the Docker cluster proof's scale-down).
    #[test]
    fn sessions_streaming_to_each_other_do_not_deadlock() {
        const FRAMES: usize = 400;
        let (sessions, threads) = session_pair();
        let senders: Vec<_> = sessions
            .iter()
            .map(|session| {
                let session = Arc::clone(session);
                std::thread::spawn(move || {
                    for index in 0..FRAMES {
                        session
                            .send_waiting(OutboundClass::Snapshot, vec![HEARTBEAT_PONG; 16 * 1024])
                            .unwrap_or_else(|error| panic!("frame {index}: {error}"));
                    }
                })
            })
            .collect();
        for sender in senders {
            sender.join().expect("every frame queued");
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while sessions
            .iter()
            .any(|session| session.snapshot_queued_bytes.load(Ordering::Acquire) > 0)
        {
            assert!(Instant::now() < deadline, "the lanes never drained");
            std::thread::sleep(Duration::from_millis(10));
        }
        for session in &sessions {
            assert!(!session.shutdown.load(Ordering::Acquire));
            session.shutdown.store(true, Ordering::SeqCst);
        }
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn outbound_queue_enforces_item_and_byte_bounds() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let bytes = AtomicUsize::new(0);
        enqueue_outbound(&sender, &bytes, 8, OutboundClass::Application, vec![1; 4])
            .expect("first frame fits");
        assert_eq!(bytes.load(Ordering::Acquire), 4);
        assert_eq!(
            enqueue_outbound(&sender, &bytes, 8, OutboundClass::Application, vec![2; 4],),
            Err("peer_outbound_queue_full".to_string())
        );
        assert_eq!(bytes.load(Ordering::Acquire), 4);
        let frame = receiver.recv().expect("queued frame");
        bytes.fetch_sub(frame.payload.len(), Ordering::AcqRel);
        assert_eq!(
            enqueue_outbound(&sender, &bytes, 8, OutboundClass::Application, vec![3; 9],),
            Err("peer_outbound_byte_limit".to_string())
        );
        assert_eq!(bytes.load(Ordering::Acquire), 0);
    }

    #[test]
    fn admission_burst_cannot_consume_critical_control_queue_capacity() {
        let (control_sender, _control_receiver) = crossbeam_channel::bounded(1);
        let (admission_sender, _admission_receiver) = crossbeam_channel::bounded(1);
        let control_bytes = AtomicUsize::new(0);
        let admission_bytes = AtomicUsize::new(0);

        enqueue_outbound(
            &control_sender,
            &control_bytes,
            CONTROL_QUEUE_BYTES,
            OutboundClass::Control,
            vec![1],
        )
        .expect("critical control frame");
        assert_eq!(
            enqueue_outbound(
                &control_sender,
                &control_bytes,
                CONTROL_QUEUE_BYTES,
                OutboundClass::Control,
                vec![2],
            ),
            Err("peer_outbound_queue_full".to_string())
        );
        enqueue_outbound(
            &admission_sender,
            &admission_bytes,
            ADMISSION_QUEUE_BYTES,
            OutboundClass::Admission,
            vec![3],
        )
        .expect("independent admission capacity");

        assert_eq!(control_bytes.load(Ordering::Acquire), 1);
        assert_eq!(admission_bytes.load(Ordering::Acquire), 1);
    }

    #[test]
    fn outbound_lane_snapshot_reports_item_or_byte_saturation() {
        let lane = outbound_lane_snapshot("application", 2, 80, 10, 100);

        assert_eq!(lane.class, "application");
        assert_eq!(lane.queued_items, 2);
        assert_eq!(lane.queued_bytes, 80);
        assert_eq!(lane.utilization, 0.8);
    }

    #[test]
    fn accepted_http_reservations_expire_without_a_followup_query() {
        let controller = crate::dist::telemetry::global_admission_controller();
        let now = Instant::now();
        let mut reservations = FxHashMap::default();
        reservations.insert(
            1,
            AcceptedHttpReservation {
                _permit: controller
                    .reserve_application()
                    .expect("reserve expired application slot"),
                expires_at: now.checked_sub(Duration::from_millis(1)).unwrap(),
            },
        );

        expire_http_reservation_map(&mut reservations, now);
        assert!(reservations.is_empty());

        reservations.insert(
            2,
            AcceptedHttpReservation {
                _permit: controller
                    .reserve_application()
                    .expect("reserve live application slot"),
                expires_at: now + Duration::from_secs(1),
            },
        );
        expire_http_reservation_map(&mut reservations, now);
        assert!(reservations.contains_key(&2));
    }

    #[test]
    fn outbound_control_priority_is_bounded_when_application_is_waiting() {
        let (control_tx, control) = crossbeam_channel::bounded(16);
        let (_admission_tx, admission) = crossbeam_channel::bounded(1);
        let (_continuity_tx, continuity) = crossbeam_channel::bounded(1);
        let (application_tx, application) = crossbeam_channel::bounded(1);
        let (_snapshot_tx, snapshot) = crossbeam_channel::bounded(1);
        for _ in 0..8 {
            control_tx
                .send(OutboundFrame {
                    payload: vec![1],
                    class: OutboundClass::Control,
                })
                .unwrap();
        }
        application_tx
            .send(OutboundFrame {
                payload: vec![2],
                class: OutboundClass::Application,
            })
            .unwrap();
        let receivers = OutboundReceivers {
            control,
            admission,
            continuity,
            application,
            snapshot,
        };
        let mut consecutive_control_frames = 0;

        for _ in 0..MAX_CONSECUTIVE_CONTROL_FRAMES {
            let frame = try_next_outbound_frame(&receivers, &mut consecutive_control_frames)
                .expect("queued control frame");
            assert!(matches!(frame.class, OutboundClass::Control));
        }
        let frame = try_next_outbound_frame(&receivers, &mut consecutive_control_frames)
            .expect("waiting application frame");
        assert!(matches!(frame.class, OutboundClass::Application));
        assert_eq!(consecutive_control_frames, 0);
    }

    #[test]
    fn draining_owner_reservation_is_recoverable_before_execution() {
        assert!(retryable_clustered_http_transport_failure(
            "owner_reservation_rejected:Draining"
        ));
        assert!(!retryable_clustered_http_transport_failure(
            "owner_reservation_rejected:InflightLimit"
        ));
    }

    // -------------------------------------------------------------------
    // Plan 03 tests: HeartbeatState, handshake, wire format, lifecycle
    // -------------------------------------------------------------------

    #[test]
    fn test_heartbeat_state_timing() {
        // Short intervals for test speed: 100ms ping, 50ms pong timeout.
        let mut hs = HeartbeatState::new(Duration::from_millis(100), Duration::from_millis(50));

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
        let state_a = handshake_state("alice@127.0.0.1", cookie.clone(), 1);

        let state_b = handshake_state("bob@127.0.0.1", cookie.clone(), 2);

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
    fn test_mixed_version_handshake_keeps_protocol_one_service_and_fences_autonomy() {
        use std::os::unix::net::UnixStream;

        let (current_stream, protocol_one_stream) = UnixStream::pair().unwrap();
        let cookie = "rolling-upgrade-cookie".to_string();
        let protocol_one_cookie = cookie.clone();

        let current = std::thread::spawn(move || {
            let mut stream = current_stream;
            perform_handshake_with_identity(&mut stream, "current@127.0.0.1:9100", &cookie, 2, true)
        });
        let protocol_one = std::thread::spawn(move || -> Result<String, String> {
            let mut stream = protocol_one_stream;

            // A protocol-one decoder consumes only the original name prefix and
            // ignores extension bytes that a new peer appends after creation.
            let name_message = read_msg(&mut stream)
                .map_err(|error| format!("protocol_one_recv_name_failed:{error}"))?;
            if name_message.first() != Some(&HANDSHAKE_NAME) || name_message.len() < 4 {
                return Err("protocol_one_name_message_invalid".to_string());
            }
            let name_len = u16::from_le_bytes([name_message[1], name_message[2]]) as usize;
            if name_message.len() < 4 + name_len {
                return Err("protocol_one_name_message_truncated".to_string());
            }
            let remote_name = std::str::from_utf8(&name_message[3..3 + name_len])
                .map_err(|_| "protocol_one_name_invalid_utf8".to_string())?
                .to_string();

            // Emit the exact protocol-one challenge shape: no version hello.
            let challenge = generate_challenge();
            let protocol_one_name = b"protocol-one@127.0.0.1:9101";
            let mut challenge_message = Vec::new();
            challenge_message.push(HANDSHAKE_CHALLENGE);
            challenge_message.extend_from_slice(&(protocol_one_name.len() as u16).to_le_bytes());
            challenge_message.extend_from_slice(protocol_one_name);
            challenge_message.push(1);
            challenge_message.extend_from_slice(&challenge);
            write_msg(&mut stream, &challenge_message)
                .map_err(|error| format!("protocol_one_send_challenge_failed:{error}"))?;

            let (response, remote_challenge) = recv_challenge_reply(&mut stream)?;
            let binding = stream.channel_binding()?;
            if !verify_response(&protocol_one_cookie, &challenge, &binding, &response) {
                return Err("protocol_one_cookie_response_invalid".to_string());
            }
            let ack = compute_response(&protocol_one_cookie, &remote_challenge, &binding);
            send_challenge_ack(&mut stream, &ack)?;
            Ok(remote_name)
        });

        let (_, _, negotiated, identity) = current
            .join()
            .expect("current handshake thread")
            .expect("current peer accepts protocol one");
        assert_eq!(
            protocol_one
                .join()
                .expect("protocol-one handshake thread")
                .expect("protocol-one peer accepts extended current name"),
            "current@127.0.0.1:9100"
        );
        assert_eq!(negotiated.version, PROTOCOL_V1);
        assert!(!negotiated.autonomous_enabled);
        assert_eq!(
            negotiated.disabled_reason.as_deref(),
            Some("protocol_two_not_negotiated")
        );
        assert!(identity.is_none());
        let application_payload = vec![DIST_SPAWN, 7, 8, 9];
        let wire = encode_session_payload(
            OutboundClass::Application,
            application_payload.clone(),
            &negotiated,
        )
        .expect("mixed-version data frame remains available");
        assert_eq!(wire, application_payload);
        assert_eq!(
            decode_session_payload(wire, &negotiated)
                .expect("mixed-version data frame remains decodable"),
            application_payload
        );
    }

    #[test]
    fn test_handshake_wrong_cookie() {
        use std::os::unix::net::UnixStream;

        let (stream_a, stream_b) = UnixStream::pair().unwrap();

        // Set a read timeout so the test doesn't hang on failure.
        stream_a
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream_b
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let state_a = handshake_state("alice@127.0.0.1", "correct_cookie".to_string(), 1);

        let state_b = handshake_state("bob@127.0.0.1", "wrong_cookie".to_string(), 2);

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
    fn test_handshake_rejects_invalid_remote_name() {
        use std::os::unix::net::UnixStream;

        let (stream_a, stream_b) = UnixStream::pair().unwrap();
        stream_a
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream_b
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let state_a = handshake_state("alice@127.0.0.1", "shared_cookie".to_string(), 1);

        let state_b = handshake_state("broken@[::1", "shared_cookie".to_string(), 2);

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

        assert!(
            result_a.is_err(),
            "initiator should reject malformed remote names"
        );
        assert!(
            result_a.unwrap_err().contains("invalid remote node name"),
            "unexpected error for malformed remote name"
        );
        assert!(
            result_b.is_err(),
            "acceptor should observe the failed handshake"
        );
    }

    #[test]
    fn test_node_connect_full_lifecycle() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Create two independent TLS configurations (simulating two nodes).
        let (cert_a, key_a) = generate_ephemeral_cert();
        let server_config_a = build_node_server_config(cert_a, key_a);
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

            let state = handshake_state("server@127.0.0.1", cookie_a, 1);

            perform_handshake(&mut tls_stream, &state, false)
        });

        // Client (initiator) connects.
        let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
        let client_conn =
            rustls::ClientConnection::new(Arc::clone(&client_config_b), server_name).unwrap();
        let mut tls_stream = StreamOwned::new(client_conn, tcp_stream);

        let client_state = handshake_state("client@127.0.0.1", cookie_b, 3);

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

    /// A relay that terminates legacy node TLS towards both peers and forwards
    /// the four cookie handshake messages verbatim must not end up with an
    /// authenticated session on either side: every proof is bound to the TLS
    /// session it was produced on, and the relay sits on two different ones.
    #[test]
    fn legacy_tls_relay_cannot_splice_cookie_handshake() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        const IO_TIMEOUT: Duration = Duration::from_secs(10);

        let cookie = "relay_test_cookie".to_string();
        let bob_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bob_port = bob_listener.local_addr().unwrap().port();
        let relay_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let relay_port = relay_listener.local_addr().unwrap().port();

        let bob_cookie = cookie.clone();
        let bob = std::thread::spawn(move || {
            let (tcp, _) = bob_listener.accept().unwrap();
            tcp.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
            let (cert, key) = generate_ephemeral_cert();
            let conn = rustls::ServerConnection::new(build_node_server_config(cert, key)).unwrap();
            let mut tls = StreamOwned::new(conn, tcp);
            perform_handshake_with_identity(&mut tls, "bob@127.0.0.1", &bob_cookie, 2, false)
        });

        // The relay never learns the cookie. Legacy mode checks no certificates,
        // so both of its TLS sessions complete normally.
        let relay = std::thread::spawn(
            move || -> Result<(usize, ChannelBinding, ChannelBinding), String> {
                let (from_alice, _) = relay_listener.accept().unwrap();
                from_alice.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
                let (cert, key) = generate_ephemeral_cert();
                let mut alice_side = StreamOwned::new(
                    rustls::ServerConnection::new(build_node_server_config(cert, key)).unwrap(),
                    from_alice,
                );

                let to_bob = TcpStream::connect(format!("127.0.0.1:{bob_port}")).unwrap();
                to_bob.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
                let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
                let mut bob_side = StreamOwned::new(
                    rustls::ClientConnection::new(build_node_client_config(), server_name).unwrap(),
                    to_bob,
                );

                let alice_binding = alice_side.channel_binding()?;
                let bob_binding = bob_side.channel_binding()?;

                fn forward(source: &mut impl Read, sink: &mut impl Write) -> io::Result<()> {
                    let message = read_msg(source)?;
                    write_msg(sink, &message)
                }

                // The cookie handshake strictly alternates: NAME (alice),
                // CHALLENGE (bob), REPLY (alice), ACK (bob). Forward until a
                // peer gives up.
                let mut forwarded = 0;
                while forwarded < 4 {
                    let step = if forwarded % 2 == 0 {
                        forward(&mut alice_side, &mut bob_side)
                    } else {
                        forward(&mut bob_side, &mut alice_side)
                    };
                    if step.is_err() {
                        break;
                    }
                    forwarded += 1;
                }
                Ok((forwarded, alice_binding, bob_binding))
            },
        );

        let alice_tcp = TcpStream::connect(format!("127.0.0.1:{relay_port}")).unwrap();
        alice_tcp.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
        let mut alice = StreamOwned::new(
            rustls::ClientConnection::new(build_node_client_config(), server_name).unwrap(),
            alice_tcp,
        );
        let alice_result =
            perform_handshake_with_identity(&mut alice, "alice@127.0.0.1", &cookie, 1, true);
        drop(alice);

        let bob_result = bob.join().unwrap();
        let (forwarded, alice_binding, bob_binding) = relay.join().unwrap().unwrap();

        assert_ne!(
            alice_binding, bob_binding,
            "relay must sit on two distinct TLS sessions"
        );
        // Bob receives alice's proof unchanged, but it was bound to the
        // alice<->relay session, so verification against bob's own session fails.
        let bob_error = bob_result.expect_err("relayed handshake must be rejected by the acceptor");
        assert!(bob_error.contains("cookie mismatch"), "{bob_error}");
        // Bob refused to answer, so alice never receives a valid ACK either.
        assert!(
            alice_result.is_err(),
            "initiator must not authenticate through a relay: {alice_result:?}"
        );
        assert_eq!(
            forwarded, 3,
            "NAME, CHALLENGE and REPLY were relayed intact; the ACK was never produced"
        );
    }

    // -------------------------------------------------------------------
    // Plan 65-03 Task 1: Wire format and message routing unit tests
    // -------------------------------------------------------------------

    extern "C" fn remote_spawn_test_entry(_args: *const u8) {}

    fn register_remote_spawn_test_function(name: &str, signature: &[u8]) {
        mesh_register_function(
            name.as_ptr(),
            name.len() as u64,
            remote_spawn_test_entry as *const u8,
            signature.as_ptr(),
            signature.len() as u64,
        );
    }

    /// Encode the argument section of a DIST_SPAWN request:
    /// `[u16 count][tags][encoded values]`, exactly as a peer chooses it.
    fn encode_spawn_arg_section(args: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut payload = (args.len() as u16).to_le_bytes().to_vec();
        payload.extend(args.iter().map(|(tag, _)| *tag));
        for (_, value) in args {
            payload.extend_from_slice(value);
        }
        payload
    }

    fn spawn_string_value(value: &str) -> Vec<u8> {
        let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    fn spawn_int_value(value: u64) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }

    #[test]
    fn authenticated_remote_spawn_rejects_arguments_that_do_not_match_the_signature() {
        let name = "remote_spawn_signature_test_string_actor";
        register_remote_spawn_test_function(name, &[REMOTE_SPAWN_ARG_STRING]);

        // Zero arguments for an actor whose generated entry loads one String pointer.
        assert_eq!(
            prepare_remote_spawn(name, &encode_spawn_arg_section(&[])).err(),
            Some("remote_spawn_arity_mismatch:expected=1:received=0".to_string())
        );
        // An integer where the entry would dereference a String pointer.
        assert_eq!(
            prepare_remote_spawn(
                name,
                &encode_spawn_arg_section(&[(REMOTE_SPAWN_ARG_INT, spawn_int_value(7))])
            )
            .err(),
            Some(format!(
                "remote_spawn_arg_type_mismatch:index=0:expected={REMOTE_SPAWN_ARG_STRING}:received={REMOTE_SPAWN_ARG_INT}"
            ))
        );
        // More arguments than the entry reads.
        assert_eq!(
            prepare_remote_spawn(
                name,
                &encode_spawn_arg_section(&[
                    (REMOTE_SPAWN_ARG_STRING, spawn_string_value("a")),
                    (REMOTE_SPAWN_ARG_STRING, spawn_string_value("b")),
                ])
            )
            .err(),
            Some("remote_spawn_arity_mismatch:expected=1:received=2".to_string())
        );
        // A tag that is never valid on the wire.
        assert_eq!(
            prepare_remote_spawn(
                name,
                &encode_spawn_arg_section(&[(REMOTE_SPAWN_ARG_UNSUPPORTED, Vec::new())])
            )
            .err(),
            Some(format!(
                "remote_spawn_arg_type_mismatch:index=0:expected={REMOTE_SPAWN_ARG_STRING}:received={REMOTE_SPAWN_ARG_UNSUPPORTED}"
            ))
        );
        // Signature satisfied but the value bytes are truncated.
        let mut truncated =
            encode_spawn_arg_section(&[(REMOTE_SPAWN_ARG_STRING, spawn_string_value("hello"))]);
        truncated.truncate(truncated.len() - 2);
        assert_eq!(
            prepare_remote_spawn(name, &truncated).err(),
            Some("remote_spawn_arg_string_truncated".to_string())
        );

        // The node keeps serving: the registration is intact and a well-formed
        // request is still honoured after the malformed ones were rejected.
        let (fn_ptr, args) = prepare_remote_spawn(
            name,
            &encode_spawn_arg_section(&[(REMOTE_SPAWN_ARG_STRING, spawn_string_value("hello"))]),
        )
        .unwrap();
        assert_eq!(fn_ptr, remote_spawn_test_entry as *const u8);
        assert_eq!(args.len(), 1);
        let decoded = unsafe { &*(args[0] as *const crate::string::MeshString) };
        assert_eq!(unsafe { decoded.as_bytes() }, b"hello");
    }

    /// A pid argument goes as its local id and the name of its node; one
    /// whose node is unknown here, or whose bytes are cut short, is none.
    #[test]
    fn a_remote_spawn_pid_argument_names_its_node() {
        let tags = [REMOTE_SPAWN_ARG_PID];
        let unknown = crate::actor::process::ProcessId::from_remote(u16::MAX, 1, 5);
        let encoded = encode_remote_spawn_args(&unknown.as_u64().to_le_bytes(), &tags).unwrap();
        assert_eq!(&encoded[3..11], &5u64.to_le_bytes(), "the local id");
        assert_eq!(&encoded[11..], &0u16.to_le_bytes(), "and no node");
        assert_eq!(decode_remote_spawn_args(&encoded, &tags), Ok(vec![0]));
        assert_eq!(
            decode_remote_spawn_args(&encoded[..12], &tags),
            Err("remote_spawn_arg_pid_truncated".to_string())
        );
    }

    #[test]
    fn remote_spawn_refuses_functions_without_a_complete_wire_signature() {
        // Registered with parameters but no signature: nothing can be validated.
        let unsigned = "remote_spawn_signature_test_unsigned_function";
        mesh_register_function(
            unsigned.as_ptr(),
            unsigned.len() as u64,
            remote_spawn_test_entry as *const u8,
            std::ptr::null(),
            2,
        );
        for args in [
            encode_spawn_arg_section(&[]),
            encode_spawn_arg_section(&[
                (REMOTE_SPAWN_ARG_INT, spawn_int_value(1)),
                (REMOTE_SPAWN_ARG_INT, spawn_int_value(2)),
            ]),
        ] {
            assert_eq!(
                prepare_remote_spawn(unsigned, &args).err(),
                Some("remote_spawn_target_not_remotely_spawnable".to_string())
            );
        }

        // Codegen marks parameters it cannot transfer as unsupported; one such
        // parameter makes the whole function unreachable from remote spawn.
        let partial = "remote_spawn_signature_test_partially_supported";
        register_remote_spawn_test_function(
            partial,
            &[REMOTE_SPAWN_ARG_INT, REMOTE_SPAWN_ARG_UNSUPPORTED],
        );
        assert_eq!(
            prepare_remote_spawn(
                partial,
                &encode_spawn_arg_section(&[
                    (REMOTE_SPAWN_ARG_INT, spawn_int_value(1)),
                    (REMOTE_SPAWN_ARG_UNSUPPORTED, Vec::new()),
                ])
            )
            .err(),
            Some("remote_spawn_target_not_remotely_spawnable".to_string())
        );

        // Zero-parameter functions accept exactly zero arguments.
        let zero_arity = "remote_spawn_signature_test_zero_arity";
        register_remote_spawn_test_function(zero_arity, &[]);
        let (fn_ptr, args) =
            prepare_remote_spawn(zero_arity, &encode_spawn_arg_section(&[])).unwrap();
        assert_eq!(fn_ptr, remote_spawn_test_entry as *const u8);
        assert!(args.is_empty());
        assert_eq!(
            prepare_remote_spawn(
                zero_arity,
                &encode_spawn_arg_section(&[(REMOTE_SPAWN_ARG_INT, spawn_int_value(1))])
            )
            .err(),
            Some("remote_spawn_arity_mismatch:expected=0:received=1".to_string())
        );

        assert_eq!(
            prepare_remote_spawn(
                "remote_spawn_signature_test_never_registered",
                &encode_spawn_arg_section(&[])
            )
            .err(),
            Some("function_not_found".to_string())
        );
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
    fn test_mesh_node_self_returns_value_or_null() {
        // mesh_node_self returns an empty string when NODE_STATE is not initialized,
        // or the node name string when it IS initialized.
        // Since tests share a process and NODE_STATE is a OnceLock, another
        // test may have initialized it. We test both cases:
        let result = mesh_node_self();
        // Should always return a valid (non-null) pointer, even when node not started.
        assert!(
            !result.is_null(),
            "expected non-null pointer from mesh_node_self"
        );
        if node_state().is_none() {
            // Not initialized: should return empty string
            let s = unsafe { &*(result as *const crate::string::MeshString) };
            assert_eq!(s.len, 0, "expected empty string when node not started");
        }
    }

    #[test]
    fn test_mesh_node_list_returns_valid_list() {
        // mesh_node_list should always return a valid list, never null.
        // When not initialized or no connections, returns an empty list.
        let result = mesh_node_list();
        assert!(!result.is_null(), "mesh_node_list should never return null");

        // The returned list should be a valid Mesh list with length >= 0
        let len = crate::collections::list::mesh_list_length(result);
        assert!(len >= 0, "list length should be non-negative");
    }

    // -------------------------------------------------------------------
    // What a node does with each message its peer sends
    // -------------------------------------------------------------------

    use crate::actor::heap::MessageBuffer;
    use crate::actor::process::{ExitReason, Monitor, Priority, Process, ProcessId, ProcessState};

    /// Both ends of a TLS connection over loopback, handshake done.
    fn tls_pair() -> (
        StreamOwned<rustls::ClientConnection, TcpStream>,
        StreamOwned<rustls::ServerConnection, TcpStream>,
    ) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let (cert, key) = generate_ephemeral_cert();
            let mut tls = StreamOwned::new(
                rustls::ServerConnection::new(build_node_server_config(cert, key)).unwrap(),
                tcp,
            );
            while tls.conn.is_handshaking() {
                tls.conn.complete_io(&mut tls.sock).unwrap();
            }
            tls
        });
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
        let mut tls = StreamOwned::new(
            rustls::ClientConnection::new(build_node_client_config(), server_name).unwrap(),
            tcp,
        );
        while tls.conn.is_handshaking() {
            tls.conn.complete_io(&mut tls.sock).unwrap();
        }
        (tls, server.join().unwrap())
    }

    fn protocol_one() -> NegotiatedProtocol {
        NegotiatedProtocol {
            version: PROTOCOL_V1,
            capabilities: super::super::protocol::Capabilities::default(),
            max_frame_bytes: MAX_DIST_MSG,
            autonomous_enabled: false,
            disabled_reason: None,
        }
    }

    /// A session of the test node to a peer the test plays. No thread serves
    /// it: what it queues waits in its lanes for `sent`, what it writes to
    /// the stream itself (heartbeats) reaches `stream`, and `receive` acts on
    /// a message as its reader would. Dropping it disconnects the peer.
    struct TestPeer {
        session: Arc<NodeSession>,
        stream: StreamOwned<rustls::ServerConnection, TcpStream>,
        heartbeat: Mutex<HeartbeatState>,
        _member: Option<parking_lot::RwLockReadGuard<'static, ()>>,
    }

    impl TestPeer {
        fn new(name: &str) -> Self {
            Self::build(
                name,
                protocol_one(),
                None,
                Some(TEST_PEERS.read_recursive()),
            )
        }

        /// A peer in a test that holds the clustered state alone.
        fn within(_exclusive: &parking_lot::RwLockWriteGuard<'static, ()>, name: &str) -> Self {
            Self::build(name, protocol_one(), None, None)
        }

        fn build(
            name: &str,
            protocol: NegotiatedProtocol,
            identity: Option<super::super::identity_claim::NodeIdentityClaim>,
            member: Option<parking_lot::RwLockReadGuard<'static, ()>>,
        ) -> Self {
            let state = test_node();
            let (client, server) = tls_pair();
            let session = register_session(
                state,
                name.to_string(),
                1,
                state.assign_node_id(),
                NodeStream::ClientTls(client),
                protocol,
                identity,
            )
            .expect("a peer name no other test uses");
            Self {
                session,
                stream: server,
                heartbeat: Mutex::new(HeartbeatState::new(
                    Duration::from_secs(60),
                    Duration::from_secs(15),
                )),
                _member: member,
            }
        }

        fn receive(&self, msg: Vec<u8>) {
            handle_session_message(&self.session, &self.heartbeat, msg);
        }

        /// The oldest frame the session has queued for the peer (by lane),
        /// but for the broadcasts every session of the node gets (from the
        /// other tests too).
        fn take_sent(&self) -> Option<Vec<u8>> {
            let receivers = self.session.outbound_receivers.lock().unwrap();
            let receivers = receivers.as_ref().expect("no writer took the lanes");
            let lanes = [
                &receivers.control,
                &receivers.admission,
                &receivers.continuity,
                &receivers.application,
                &receivers.snapshot,
            ];
            let frame = lanes
                .into_iter()
                .flat_map(|lane| lane.try_iter())
                .find_map(|frame| {
                    release_outbound_frame_bytes(&self.session, &frame);
                    let payload =
                        decode_session_payload(frame.payload, &self.session.negotiated_protocol)
                            .expect("a frame the session encoded");
                    (!matches!(
                        payload[0],
                        DIST_GLOBAL_REGISTER | DIST_GLOBAL_UNREGISTER | DIST_CONTINUITY_UPSERT
                    ))
                    .then_some(payload)
                });
            frame
        }

        /// The frames the session queued for the peer since the last call.
        fn sent(&self) -> Vec<Vec<u8>> {
            std::iter::from_fn(|| self.take_sent()).collect()
        }

        /// The next frame the session queues, which an actor or worker
        /// thread sends a moment from now.
        fn next_sent(&self) -> Vec<u8> {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(frame) = self.take_sent() {
                    return frame;
                }
                assert!(Instant::now() < deadline, "the session sent nothing");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        /// The peer's process `local`, as this node addresses it.
        fn pid(&self, local: u64) -> ProcessId {
            ProcessId::from_remote(self.session.node_id, self.session.remote_creation, local)
        }
    }

    impl Drop for TestPeer {
        fn drop(&mut self) {
            self.session.shutdown.store(true, Ordering::SeqCst);
            cleanup_session_if_current(&self.session);
        }
    }

    /// A process of the running scheduler that nothing runs: what arrives
    /// for it stays in its mailbox, and its links and monitors stay as set.
    struct ParkedProcess {
        pid: ProcessId,
        process: Arc<parking_lot::Mutex<Process>>,
    }

    impl ParkedProcess {
        fn new() -> Self {
            test_node();
            let pid = ProcessId::next();
            let process = Arc::new(parking_lot::Mutex::new(Process::new(pid, Priority::Normal)));
            crate::actor::global_scheduler()
                .process_table()
                .write()
                .insert(pid, Arc::clone(&process));
            Self { pid, process }
        }

        fn mailbox_len(&self) -> usize {
            self.process.lock().mailbox.len()
        }
    }

    impl Drop for ParkedProcess {
        fn drop(&mut self) {
            crate::actor::global_scheduler()
                .process_table()
                .write()
                .remove(&self.pid);
        }
    }

    fn frame(tag: u8, fields: &[&[u8]]) -> Vec<u8> {
        let mut frame = vec![tag];
        for field in fields {
            frame.extend_from_slice(field);
        }
        frame
    }

    fn u16_str(text: &str) -> Vec<u8> {
        let mut field = (text.len() as u16).to_le_bytes().to_vec();
        field.extend_from_slice(text.as_bytes());
        field
    }

    fn continuity_record(request_key: &str, owner: &str, replica: &str) -> ContinuityRecord {
        use crate::dist::continuity::{
            ContinuityClusterRole, ContinuityPhase, ContinuityResult, ReplicaStatus,
            ReplicationHealth,
        };
        ContinuityRecord {
            request_key: request_key.to_string(),
            payload_hash: "sha256:payload".to_string(),
            record_version: 1,
            request_payload: Vec::new(),
            attempt_id: "attempt-1".to_string(),
            phase: ContinuityPhase::Submitted,
            result: ContinuityResult::Pending,
            ingress_node: owner.to_string(),
            owner_node: owner.to_string(),
            replica_nodes: vec![replica.to_string()],
            acknowledged_replica_nodes: Vec::new(),
            replica_node: replica.to_string(),
            replication_count: 2,
            replica_status: ReplicaStatus::Preparing,
            cluster_role: ContinuityClusterRole::Primary,
            promotion_epoch: 0,
            // What every record the shared registry holds says, so what the
            // operator tests read of its health stays true.
            replication_health: ReplicationHealth::LocalOnly,
            execution_node: String::new(),
            routed_remotely: false,
            fell_back_locally: false,
            error: String::new(),
            declared_handler_runtime_name: String::new(),
        }
    }

    use crate::dist::continuity::ContinuityRecord;

    /// A ping is answered straight on the stream with a pong carrying its
    /// payload; a pong clears only the ping it answers. A session that can
    /// no longer answer shuts down.
    #[test]
    fn a_peer_ping_is_answered_and_its_pong_clears_the_ping_it_answers() {
        let mut peer = TestPeer::new("heartbeat-peer@127.0.0.1:1");
        peer.receive(frame(HEARTBEAT_PING, &[&[7; 8]]));
        peer.stream
            .sock
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(
            read_msg(&mut peer.stream).unwrap(),
            frame(HEARTBEAT_PONG, &[&[7; 8]])
        );

        peer.heartbeat.lock().unwrap().pending_ping_payload = Some([3; 8]);
        peer.receive(frame(HEARTBEAT_PONG, &[&[4; 8]]));
        peer.receive(vec![HEARTBEAT_PONG, 3]);
        assert_eq!(
            peer.heartbeat.lock().unwrap().pending_ping_payload,
            Some([3; 8])
        );
        peer.receive(frame(HEARTBEAT_PONG, &[&[3; 8]]));
        assert_eq!(peer.heartbeat.lock().unwrap().pending_ping_payload, None);
        peer.receive(frame(HEARTBEAT_PONG, &[&[3; 8]]));
        peer.receive(vec![HEARTBEAT_PING, 1]);
        assert!(peer.sent().is_empty());

        peer.session.shutdown.store(true, Ordering::SeqCst);
        peer.receive(frame(HEARTBEAT_PING, &[&[7; 8]]));
        assert!(peer.session.shutdown.load(Ordering::SeqCst));
    }

    /// An empty message, one of a kind this node does not know, and a
    /// message too short to be what its tag says change nothing.
    #[test]
    fn empty_unknown_and_cut_short_messages_change_nothing() {
        let peer = TestPeer::new("malformed-peer@127.0.0.1:1");
        for msg in [
            Vec::new(),
            vec![0xEE, 1, 2],
            vec![DIST_SEND, 1, 2],
            vec![DIST_MONITOR, 1],
            vec![DIST_DEMONITOR, 1],
            vec![DIST_MONITOR_EXIT, 1],
            vec![DIST_LINK, 1],
            vec![DIST_EXIT, 1],
            vec![DIST_SPAWN, 1],
            vec![DIST_SPAWN_REPLY, 1],
            vec![DIST_GLOBAL_REGISTER, 1],
            vec![DIST_GLOBAL_UNREGISTER, 1],
            vec![DIST_CONTINUITY_PREPARE, 1],
            vec![DIST_CONTINUITY_PREPARE_ACK, 1],
            vec![DIST_HTTP_ROUTE_V2_QUERY, 1],
            vec![DIST_HTTP_ROUTE_V2_REPLY, 1],
            vec![DIST_HTTP_RESERVE_REPLY, 1],
            vec![DIST_ROOM_BROADCAST, 1],
        ] {
            peer.receive(msg);
        }
        assert!(peer.sent().is_empty());
        assert!(!peer.session.shutdown.load(Ordering::SeqCst));
    }

    /// A peer monitors a local process; one that has ended, or never was,
    /// is reported gone at once. A demonitor removes the monitor.
    #[test]
    fn a_peer_monitors_a_local_process_or_hears_at_once_it_is_gone() {
        let peer = TestPeer::new("monitoring-peer@127.0.0.1:1");
        let watched = ParkedProcess::new();
        let monitor = |tag: u8, target: ProcessId, reference: u64| {
            frame(
                tag,
                &[
                    &5u64.to_le_bytes(),
                    &target.as_u64().to_le_bytes(),
                    &reference.to_le_bytes(),
                ],
            )
        };
        peer.receive(monitor(DIST_MONITOR, watched.pid, 41));
        assert_eq!(
            watched.process.lock().monitored_by.get(&41),
            Some(&peer.pid(5))
        );
        peer.receive(monitor(DIST_DEMONITOR, watched.pid, 41));
        assert!(watched.process.lock().monitored_by.is_empty());
        peer.receive(monitor(DIST_DEMONITOR, ProcessId::next(), 41));
        assert!(peer.sent().is_empty());

        let gone = |target: ProcessId, reference: u64| {
            let mut exit = monitor(DIST_MONITOR_EXIT, target, reference);
            exit[9..17].copy_from_slice(&peer.pid(5).as_u64().to_le_bytes());
            exit[1..9].copy_from_slice(&target.as_u64().to_le_bytes());
            crate::actor::link::encode_reason(&mut exit, &ExitReason::Error("noproc".into()));
            exit
        };
        watched.process.lock().mark_exited(ExitReason::Normal);
        peer.receive(monitor(DIST_MONITOR, watched.pid, 42));
        assert_eq!(peer.sent(), vec![gone(watched.pid, 42)]);
        let never = ProcessId::next();
        peer.receive(monitor(DIST_MONITOR, never, 43));
        assert_eq!(peer.sent(), vec![gone(never, 43)]);
    }

    /// The peer telling of a watched process's end fires the monitor once.
    #[test]
    fn a_peer_telling_of_a_watched_process_end_fires_the_monitor_once() {
        let peer = TestPeer::new("monitored-peer@127.0.0.1:1");
        let watcher = ParkedProcess::new();
        watcher.process.lock().monitors.insert(
            7,
            Monitor {
                target: peer.pid(9),
                message: MessageBuffer::new(b"gone".to_vec(), 1),
            },
        );
        let mut exit = frame(
            DIST_MONITOR_EXIT,
            &[
                &9u64.to_le_bytes(),
                &watcher.pid.as_u64().to_le_bytes(),
                &7u64.to_le_bytes(),
            ],
        );
        crate::actor::link::encode_reason(&mut exit, &ExitReason::Normal);
        peer.receive(exit.clone());
        peer.receive(exit.clone());
        assert_eq!(watcher.mailbox_len(), 1);
        exit[9..17].copy_from_slice(&ProcessId::next().as_u64().to_le_bytes());
        peer.receive(exit);
        assert_eq!(watcher.mailbox_len(), 1);
    }

    /// A link the peer makes is recorded here, and its exit signal then
    /// reaches the local process as a local one would: a normal exit leaves
    /// a process that does not trap exits alone, a crash ends it, and one
    /// that traps exits gets the signal as a message.
    #[test]
    fn a_peer_link_and_its_exit_signal_reach_the_local_process() {
        let peer = TestPeer::new("linking-peer@127.0.0.1:1");
        let linked = ParkedProcess::new();
        let fields = |to: ProcessId| [3u64.to_le_bytes(), to.as_u64().to_le_bytes()].concat();
        peer.receive(frame(DIST_LINK, &[&fields(linked.pid)]));
        peer.receive(frame(DIST_LINK, &[&fields(ProcessId::next())]));
        assert!(linked.process.lock().links.contains(&peer.pid(3)));

        let exit = |to: ProcessId, reason: &ExitReason| {
            let mut exit = frame(DIST_EXIT, &[&fields(to)]);
            crate::actor::link::encode_reason(&mut exit, reason);
            exit
        };
        peer.receive(exit(linked.pid, &ExitReason::Normal));
        assert!(!linked.process.lock().links.contains(&peer.pid(3)));
        assert!(matches!(linked.process.lock().state, ProcessState::Ready));

        linked.process.lock().trap_exit = true;
        peer.receive(exit(linked.pid, &ExitReason::Error("boom".into())));
        let signal = linked.process.lock().mailbox.pop().expect("an exit signal");
        assert_eq!(signal.buffer.type_tag, crate::actor::link::EXIT_SIGNAL_TAG);

        linked.process.lock().trap_exit = false;
        let crash = ExitReason::Error("boom".into());
        peer.receive(exit(linked.pid, &crash));
        let linked_crash = ExitReason::Linked(peer.pid(3), Box::new(crash));
        let exited = format!("{:?}", ProcessState::Exited(linked_crash));
        assert_eq!(format!("{:?}", linked.process.lock().state), exited);
        peer.receive(exit(linked.pid, &ExitReason::Killed));
        assert_eq!(format!("{:?}", linked.process.lock().state), exited);

        peer.receive(exit(ProcessId::next(), &ExitReason::Killed));
        peer.receive(frame(DIST_EXIT, &[&fields(linked.pid), &[0xFF]]));
        assert_eq!(linked.mailbox_len(), 0);
    }

    /// A peer spawns a function registered for remote spawn and hears the
    /// local id of the process it got, or that there was none; a linked
    /// spawn links back.
    #[test]
    fn a_peer_spawns_a_registered_function_here_and_hears_back() {
        let peer = TestPeer::new("spawning-peer@127.0.0.1:1");
        register_remote_spawn_test_function("peer_spawned_test_actor", &[]);
        let spawn = |request: u64, link: u8, name: &str| {
            frame(
                DIST_SPAWN,
                &[
                    &request.to_le_bytes(),
                    &11u64.to_le_bytes(),
                    &[link],
                    &u16_str(name),
                    &encode_spawn_arg_section(&[]),
                ],
            )
        };
        // What the peer hears, but for the exit signal the spawned process,
        // linked to its requester, sends when it ends.
        let heard = || -> Vec<Vec<u8>> {
            peer.sent()
                .into_iter()
                .filter(|frame| frame[0] != DIST_EXIT)
                .collect()
        };
        peer.receive(spawn(1, 1, "peer_spawned_test_actor"));
        let sent = heard();
        assert_eq!(sent.len(), 2, "{sent:?}");
        let (reply, link) = (&sent[0], &sent[1]);
        assert_eq!(
            &reply[..10],
            &frame(DIST_SPAWN_REPLY, &[&1u64.to_le_bytes(), &[0]])[..]
        );
        let spawned = u64::from_le_bytes(reply[10..18].try_into().unwrap());
        assert_eq!(
            link,
            &frame(DIST_LINK, &[&spawned.to_le_bytes(), &11u64.to_le_bytes()])
        );

        let refused = |request: u64| {
            frame(
                DIST_SPAWN_REPLY,
                &[&request.to_le_bytes(), &[1], &0u64.to_le_bytes()],
            )
        };
        peer.receive(spawn(2, 0, "never_registered_for_remote_spawn"));
        assert_eq!(heard(), vec![refused(2)]);
        // A name longer than the frame, or not UTF-8, names nothing.
        let mut cut_short = spawn(3, 0, "peer_spawned_test_actor");
        cut_short.truncate(24);
        peer.receive(cut_short);
        let mut not_text = spawn(4, 0, "peer_spawned_test_actor");
        not_text[20] = 0xFF;
        peer.receive(not_text);
        assert_eq!(heard(), vec![refused(3), refused(4)]);
    }

    /// A spawn reply reaches the spawn waiting for it, once.
    #[test]
    fn a_spawn_reply_reaches_the_spawn_waiting_for_it_once() {
        let peer = TestPeer::new("spawn-reply-peer@127.0.0.1:1");
        let (waiting, answer) = crate::actor::cooperative_channel();
        let mut pending = peer.session.pending_spawns.lock().unwrap();
        pending.insert(77, waiting.clone());
        pending.insert(78, waiting);
        drop(pending);
        let reply = |request: u64, status: u8| {
            frame(
                DIST_SPAWN_REPLY,
                &[&request.to_le_bytes(), &[status], &5u64.to_le_bytes()],
            )
        };
        peer.receive(reply(77, 0));
        peer.receive(reply(77, 0));
        peer.receive(reply(78, 3));
        assert_eq!(answer.try_recv(), Ok(Ok(5)));
        assert_eq!(
            answer.try_recv(),
            Ok(Err("remote_reply_status=3".to_string()))
        );
        assert!(answer.try_recv().is_err());
    }

    /// A peer's global names register here under its processes, go when it
    /// unregisters them, merge from its sync (which also tells the waiting
    /// `Node.connect` they have arrived), and go with the peer.
    #[test]
    fn a_peer_global_names_come_and_go_with_it() {
        let registry = crate::dist::global::global_name_registry();
        let name = "global-names-peer@127.0.0.1:1";
        let peer = TestPeer::new(name);
        let entry = |global: &str, local: u64| {
            [u16_str(global), local.to_le_bytes().to_vec(), u16_str(name)].concat()
        };
        peer.receive(frame(DIST_GLOBAL_REGISTER, &[&entry("peer-registered", 4)]));
        assert_eq!(registry.whereis("peer-registered"), Some(peer.pid(4)));
        peer.receive(frame(
            DIST_GLOBAL_UNREGISTER,
            &[&u16_str("peer-registered")],
        ));
        assert_eq!(registry.whereis("peer-registered"), None);

        assert!(!peer.session.global_names_received.load(Ordering::Acquire));
        peer.receive(frame(
            DIST_GLOBAL_SYNC,
            &[&1u32.to_le_bytes(), &entry("peer-synced", 6)],
        ));
        assert!(peer.session.global_names_received.load(Ordering::Acquire));
        assert_eq!(registry.whereis("peer-synced"), Some(peer.pid(6)));
        drop(peer);
        assert_eq!(registry.whereis("peer-synced"), None);
    }

    fn protocol_two() -> NegotiatedProtocol {
        NegotiatedProtocol {
            version: PROTOCOL_V2,
            capabilities: super::super::protocol::Capabilities::AUTONOMOUS_REQUIRED,
            max_frame_bytes: MAX_DIST_MSG,
            autonomous_enabled: true,
            disabled_reason: None,
        }
    }

    impl TestPeer {
        /// A protocol-two peer whose signed identity gives it `roles`.
        fn authenticated(name: &str, roles: &[&str]) -> Self {
            let identity = super::super::identity_claim::NodeIdentityClaim {
                schema_version: 1,
                cluster_id: "test-cluster".to_string(),
                stable_node_id: format!("test-cluster/{name}"),
                advertised_name: name.to_string(),
                roles: roles.iter().map(|role| role.to_string()).collect(),
                issued_at_unix_millis: 0,
                expires_at_unix_millis: u64::MAX,
            };
            Self::build(
                name,
                protocol_two(),
                Some(identity),
                Some(TEST_PEERS.read_recursive()),
            )
        }
    }

    use super::in_autonomous_mode as autonomous;

    /// An operator query that names no query kind: answered with an error.
    fn bad_operator_query() -> Vec<u8> {
        frame(
            DIST_OPERATOR_QUERY,
            &[&7u64.to_le_bytes(), &[0xFF], &0u32.to_le_bytes()],
        )
    }

    /// An autonomous node answers operator queries from operators and
    /// controllers only, and takes consensus traffic from controllers only.
    #[test]
    fn autonomous_nodes_take_operator_and_consensus_traffic_from_their_roles_only() {
        let rpc = encode_consensus_rpc_frame(DIST_CONSENSUS_RPC, 5, b"{}").unwrap();
        let reply = encode_consensus_rpc_frame(DIST_CONSENSUS_RPC_REPLY, 5, b"{}").unwrap();

        let worker = TestPeer::authenticated("authz-worker@127.0.0.1:1", &["worker"]);
        let (waiting, answer) = tokio::sync::oneshot::channel();
        worker
            .session
            .pending_consensus_rpcs
            .lock()
            .unwrap()
            .insert(5, waiting);
        autonomous(|| {
            worker.receive(bad_operator_query());
            worker.receive(rpc.clone());
            worker.receive(reply.clone());
        });
        assert!(worker.sent().is_empty());
        assert!(worker
            .session
            .pending_consensus_rpcs
            .lock()
            .unwrap()
            .contains_key(&5));
        drop(answer);

        let operator = TestPeer::authenticated("authz-operator@127.0.0.1:1", &["operator"]);
        autonomous(|| operator.receive(bad_operator_query()));
        assert_eq!(operator.sent().len(), 1, "the operator is answered");

        let controller = TestPeer::authenticated("authz-controller@127.0.0.1:1", &["controller"]);
        let (waiting, mut answer) = tokio::sync::oneshot::channel();
        controller
            .session
            .pending_consensus_rpcs
            .lock()
            .unwrap()
            .insert(5, waiting);
        autonomous(|| {
            controller.receive(bad_operator_query());
            controller.receive(reply);
            controller.receive(vec![DIST_CONSENSUS_RPC_REPLY, 1]);
            controller.receive(vec![DIST_CONSENSUS_RPC, 1]);
            controller.receive(rpc);
        });
        assert_eq!(answer.try_recv(), Ok(Ok(b"{}".to_vec())));
        let sent = controller.sent();
        assert_eq!(sent.len(), 2, "the query and the rpc are answered");
        assert_eq!(sent[1][0], DIST_CONSENSUS_RPC_REPLY);
    }

    /// Replies from the peer reach the requests waiting for them; one for a
    /// request nothing waits on any more is dropped.
    #[test]
    fn peer_replies_reach_the_requests_waiting_for_them() {
        let peer = TestPeer::new("replying-peer@127.0.0.1:1");
        let (route, route_answer) = crate::actor::cooperative_channel();
        peer.session
            .pending_http_routes
            .lock()
            .unwrap()
            .insert(5, route);
        peer.receive(encode_http_route_v2_reply_frame(5, Ok(b"ok".to_vec())).unwrap());
        peer.receive(encode_http_route_v2_reply_frame(5, Ok(b"again".to_vec())).unwrap());
        assert_eq!(route_answer.try_recv(), Ok(Ok(b"ok".to_vec())));

        let (reservation, reservation_answer) = crate::actor::cooperative_channel();
        peer.session
            .pending_http_reservations
            .lock()
            .unwrap()
            .insert(6, reservation);
        peer.receive(encode_http_reserve_reply(6, Err("full".to_string())).unwrap());
        peer.receive(encode_http_reserve_reply(6, Ok(())).unwrap());
        assert_eq!(reservation_answer.try_recv(), Ok(Err("full".to_string())));

        let (rpc, mut rpc_answer) = tokio::sync::oneshot::channel();
        peer.session
            .pending_consensus_rpcs
            .lock()
            .unwrap()
            .insert(7, rpc);
        peer.receive(encode_consensus_rpc_frame(DIST_CONSENSUS_RPC_REPLY, 7, b"[]").unwrap());
        peer.receive(encode_consensus_rpc_frame(DIST_CONSENSUS_RPC_REPLY, 7, b"[]").unwrap());
        assert_eq!(rpc_answer.try_recv(), Ok(Ok(b"[]".to_vec())));

        let (query, query_answer) = mpsc::channel();
        peer.session
            .pending_operator_queries
            .lock()
            .unwrap()
            .insert(8, query);
        peer.receive(frame(
            DIST_OPERATOR_REPLY,
            &[&8u64.to_le_bytes(), &[0], &2u32.to_le_bytes(), b"{}"],
        ));
        peer.receive(vec![DIST_OPERATOR_REPLY, 1]);
        assert_eq!(query_answer.try_recv(), Ok(Ok(b"{}".to_vec())));

        let (prepare, prepare_answer) = crate::actor::cooperative_channel();
        peer.session
            .pending_continuity_prepares
            .lock()
            .unwrap()
            .insert(9, prepare);
        peer.receive(encode_continuity_prepare_ack(
            9,
            &Err("no room".to_string()),
        ));
        peer.receive(encode_continuity_prepare_ack(9, &Ok(())));
        assert_eq!(prepare_answer.try_recv(), Ok(Err("no room".to_string())));
        assert!(peer.sent().is_empty());
    }

    /// A node that is not autonomous answers a consensus request with the
    /// reason it cannot take part, and answers operator queries.
    #[test]
    fn a_manual_node_refuses_consensus_and_answers_operator_queries() {
        let peer = TestPeer::new("manual-consensus-peer@127.0.0.1:1");
        peer.receive(encode_consensus_rpc_frame(DIST_CONSENSUS_RPC, 5, b"{}").unwrap());
        peer.receive(vec![DIST_CONSENSUS_RPC, 1]);
        peer.receive(bad_operator_query());
        assert_eq!(peer.sent().len(), 1, "only the operator query is answered");
    }

    /// A peer's load report counts as its own and in order only.
    #[test]
    fn a_peer_load_report_counts_only_as_its_own_and_in_order() {
        let name = "load-report-peer@127.0.0.1:1";
        let peer = TestPeer::new(name);
        let report = |node: &str| {
            let report = crate::dist::routing::local_load_report(node, BTreeSet::new());
            frame(DIST_LOAD_REPORT, &[&report.encode().unwrap()])
        };
        let own = report(name);
        peer.receive(own.clone());
        let registry = crate::dist::routing::load_report_registry();
        let seen = |node: &str| {
            registry
                .report(node, Instant::now(), Duration::from_secs(600))
                .map(|report| report.sequence)
        };
        let first = seen(name).expect("the report counts");
        peer.receive(own);
        assert_eq!(seen(name), Some(first), "a repeat does not count");
        peer.receive(report("load-report-impostor@127.0.0.1:1"));
        assert_eq!(seen("load-report-impostor@127.0.0.1:1"), None);
        peer.receive(vec![DIST_LOAD_REPORT, 1]);
        assert!(peer.sent().is_empty());
    }

    /// Continuity records from a peer merge here, one at a time or as its
    /// snapshot; an invalid or garbled one is dropped, and durable-store
    /// traffic this node has no store for is refused.
    #[test]
    fn a_peer_continuity_records_merge_and_bad_ones_are_dropped() {
        use crate::dist::continuity::{
            continuity_registry, encode_sync_payload, encode_upsert_payload, ContinuitySnapshot,
        };
        let peer = TestPeer::new("continuity-peer@127.0.0.1:1");
        let registry = continuity_registry();
        let record = |key: &str| {
            let mut record = continuity_record(key, "record-owner-a@h:1", "record-owner-b@h:1");
            record.phase = crate::dist::continuity::ContinuityPhase::Completed;
            record.result = crate::dist::continuity::ContinuityResult::Succeeded;
            record
        };
        // A record whose replica is its owner does not validate.
        let invalid = |mut bytes: Vec<u8>| {
            let (from, to) = (b"record-owner-b", b"record-owner-a");
            while let Some(at) = bytes.windows(from.len()).position(|window| window == from) {
                bytes[at..at + to.len()].copy_from_slice(to);
            }
            bytes
        };

        peer.receive(encode_upsert_payload(1, &record("peer-upserted-key")).unwrap());
        assert!(registry.record("peer-upserted-key").is_some());
        peer.receive(invalid(
            encode_upsert_payload(1, &record("peer-invalid-upsert")).unwrap(),
        ));
        assert!(registry.record("peer-invalid-upsert").is_none());
        peer.receive(vec![DIST_CONTINUITY_UPSERT, 1]);

        let snapshot = |key: &str| {
            encode_sync_payload(&ContinuitySnapshot {
                next_attempt_token: 1,
                records: vec![record(key)],
            })
            .unwrap()
        };
        peer.receive(snapshot("peer-synced-key"));
        assert!(registry.record("peer-synced-key").is_some());
        peer.receive(invalid(snapshot("peer-invalid-sync")));
        assert!(registry.record("peer-invalid-sync").is_none());
        peer.receive(vec![DIST_CONTINUITY_SYNC, 1]);

        for tag in [
            DIST_CONTINUITY_STORE_SNAPSHOT,
            DIST_CONTINUITY_STORE_SNAPSHOT_ACK,
            DIST_CONTINUITY_STORE_LOG_ENTRY,
        ] {
            peer.receive(vec![tag]);
        }
        assert!(peer.sent().is_empty());
    }

    /// A peer prepares a replica of its record here and hears whether it
    /// took; a record for another replica is refused.
    #[test]
    fn a_peer_prepares_a_replica_here_and_hears_whether_it_took() {
        let state = test_node();
        let peer = TestPeer::new("preparing-peer@127.0.0.1:1");
        let record = continuity_record(
            "peer-prepared-key",
            "prepared-record-owner@h:1",
            &state.name,
        );
        peer.receive(encode_continuity_prepare_payload(3, &record).unwrap());
        assert_eq!(peer.next_sent(), encode_continuity_prepare_ack(3, &Ok(())));

        let elsewhere = continuity_record(
            "peer-misdirected-key",
            "prepared-record-owner@h:1",
            "prepared-record-replica@h:1",
        );
        peer.receive(encode_continuity_prepare_payload(4, &elsewhere).unwrap());
        assert_eq!(
            peer.next_sent(),
            encode_continuity_prepare_ack(4, &Err("replica_prepare_target_mismatch".to_string()))
        );
    }

    /// Continuity prepares and their acks are framed whole: a frame cut
    /// short, padded, or with a status or reason no sender writes is
    /// refused.
    #[test]
    fn continuity_prepare_frames_refuse_malformed_bytes() {
        let record = continuity_record("framed-key", "framed-owner@h:1", "framed-replica@h:1");
        let prepare = encode_continuity_prepare_payload(3, &record).unwrap();
        assert_eq!(
            decode_continuity_prepare_payload(&prepare)
                .map(|(id, record)| (id, record.request_key)),
            Ok((3, "framed-key".to_string()))
        );
        assert!(decode_continuity_prepare_payload(&prepare[..12]).is_err());
        assert!(decode_continuity_prepare_payload(&prepare[..prepare.len() - 1]).is_err());

        let ack = encode_continuity_prepare_ack(9, &Err("why".to_string()));
        assert_eq!(
            decode_continuity_prepare_ack(&ack),
            Ok((9, Err("why".to_string())))
        );
        assert!(decode_continuity_prepare_ack(&ack[..11]).is_err());
        assert!(decode_continuity_prepare_ack(&ack[..ack.len() - 1]).is_err());
        let mut bad_status = ack.clone();
        bad_status[9] = 2;
        assert_eq!(
            decode_continuity_prepare_ack(&bad_status),
            Err("invalid continuity prepare ack status 2".to_string())
        );
        let mut bad_reason = ack;
        bad_reason[12] = 0xFF;
        assert!(decode_continuity_prepare_ack(&bad_reason).is_err());
    }

    /// Consensus RPCs are framed whole under their own tags.
    #[test]
    fn consensus_rpc_frames_refuse_malformed_bytes() {
        assert_eq!(
            encode_consensus_rpc_frame(DIST_SEND, 1, b""),
            Err("consensus_rpc_tag_invalid".to_string())
        );
        let rpc = encode_consensus_rpc_frame(DIST_CONSENSUS_RPC, 4, b"{}").unwrap();
        assert_eq!(
            decode_consensus_rpc_frame(&rpc, DIST_CONSENSUS_RPC),
            Ok((4, b"{}".to_vec()))
        );
        for (bytes, tag, error) in [
            (&rpc[..], DIST_SEND, "consensus_rpc_frame_invalid"),
            (
                &rpc[..],
                DIST_CONSENSUS_RPC_REPLY,
                "consensus_rpc_frame_invalid",
            ),
            (
                &rpc[..12],
                DIST_CONSENSUS_RPC,
                "consensus_rpc_frame_invalid",
            ),
            (
                &rpc[..14],
                DIST_CONSENSUS_RPC,
                "consensus_rpc_length_invalid",
            ),
        ] {
            assert_eq!(
                decode_consensus_rpc_frame(bytes, tag),
                Err(error.to_string())
            );
        }
        let unaddressed = encode_consensus_rpc_frame(DIST_CONSENSUS_RPC, 0, b"").unwrap();
        assert_eq!(
            decode_consensus_rpc_frame(&unaddressed, DIST_CONSENSUS_RPC),
            Err("consensus_rpc_correlation_invalid".to_string())
        );
    }

    /// A peer's retained response is kept here for replay; a garbled one
    /// is not.
    #[test]
    fn a_peer_retained_response_is_kept_for_replay() {
        let peer = TestPeer::new("response-peer@127.0.0.1:1");
        peer.receive(encode_continuity_response_frame("peer-response-key", b"200 OK").unwrap());
        assert_eq!(
            crate::dist::continuity_store::replay_runtime_response("peer-response-key"),
            Ok(Some(b"200 OK".to_vec()))
        );
        peer.receive(vec![DIST_CONTINUITY_RESPONSE, 1]);
        assert!(peer.sent().is_empty());
    }

    /// Retained responses are framed whole, with a key and a body.
    #[test]
    fn continuity_response_frames_refuse_malformed_bytes() {
        let framed = encode_continuity_response_frame("key", b"body").unwrap();
        assert_eq!(
            decode_continuity_response_frame(&framed),
            Ok(("key".to_string(), b"body".to_vec()))
        );
        let refused = |bytes: &[u8]| decode_continuity_response_frame(bytes).unwrap_err();
        assert_eq!(refused(&framed[..8]), "continuity_response_frame_invalid");
        assert_eq!(
            refused(&[&[DIST_SEND], &framed[1..]].concat()),
            "continuity_response_frame_invalid"
        );
        let mut long_key = framed.clone();
        long_key[1..5].copy_from_slice(&100u32.to_le_bytes());
        assert_eq!(refused(&long_key), "continuity_response_key_truncated");
        let mut bad_key = framed.clone();
        bad_key[5] = 0xFF;
        assert_eq!(refused(&bad_key), "continuity_response_key_invalid_utf8");
        assert_eq!(
            refused(&framed[..framed.len() - 1]),
            "continuity_response_payload_length_invalid"
        );
        let empty = encode_continuity_response_frame("", b"").unwrap();
        assert_eq!(refused(&empty), "continuity_response_payload_invalid");
        assert_eq!(
            encode_continuity_response_frame("key", &vec![0; MAX_DIST_MSG as usize]),
            Err("continuity_response_frame_too_large".to_string())
        );
    }

    /// A room broadcast from a peer reaches this node's members of the
    /// room; one cut short or not text reaches no one.
    #[test]
    fn a_peer_room_broadcast_is_delivered_only_when_whole() {
        let peer = TestPeer::new("room-peer@127.0.0.1:1");
        let broadcast = |room: &[u8], text: &[u8]| {
            frame(
                DIST_ROOM_BROADCAST,
                &[
                    &(room.len() as u16).to_le_bytes(),
                    room,
                    &(text.len() as u32).to_le_bytes(),
                    text,
                ],
            )
        };
        let whole = broadcast(b"peer-room", b"hello");
        peer.receive(whole.clone());
        peer.receive(whole[..whole.len() - 1].to_vec());
        peer.receive(whole[..8].to_vec());
        peer.receive(broadcast(&[0xFF], b"hello"));
        peer.receive(broadcast(b"peer-room", &[0xFF]));
        assert!(peer.sent().is_empty());
    }

    /// A peer's list names the nodes it knows: this node connects, in the
    /// background, to the ones it does not, never to itself or to a node it
    /// is connected to. The list it sends a peer names its other peers.
    #[test]
    fn peer_lists_name_the_other_peers_and_lead_to_the_unknown_ones() {
        let state = test_node();
        let known = TestPeer::new("listed-known@127.0.0.1:1");
        let receiver = TestPeer::new("listed-receiver@127.0.0.1:1");
        send_peer_list(&receiver.session);
        let sent = receiver.sent();
        assert_eq!(sent.len(), 1);
        let listed = String::from_utf8_lossy(&sent[0]).into_owned();
        assert!(listed.contains("listed-known@127.0.0.1:1"), "{listed}");
        assert!(!listed.contains("listed-receiver@127.0.0.1:1"), "{listed}");

        let unreachable = "listed-unreachable@127.0.0.1:1";
        let names = [state.name.as_str(), "listed-known@127.0.0.1:1", unreachable];
        let mut list = frame(DIST_PEER_LIST, &[&4u16.to_le_bytes()]);
        for name in names {
            list.extend_from_slice(&u16_str(name));
        }
        list.extend_from_slice(&[2, 0, 0xFF, 0xFE]);
        receiver.receive(list.clone());
        receiver.receive(list[..list.len() - 3].to_vec());
        receiver.receive(frame(DIST_PEER_LIST, &[&1u16.to_le_bytes(), &[1]]));
        receiver.receive(vec![DIST_PEER_LIST, 1]);
        assert!(!state.sessions.read().contains_key(unreachable));
        drop(known);
    }

    /// When a peer goes, what waited on it fails, a local process linked to
    /// one of its processes gets `noconnection` (as a message if it traps
    /// exits), a monitor of one fires, and a watcher of the node hears.
    #[test]
    fn a_departing_peer_fails_what_waits_on_it_and_signals_what_watched_it() {
        let state = test_node();
        let name = "departing-peer@127.0.0.1:1";
        let peer = TestPeer::new(name);
        let session = Arc::clone(&peer.session);
        let (prepare, prepare_answer) = crate::actor::cooperative_channel();
        session
            .pending_continuity_prepares
            .lock()
            .unwrap()
            .insert(1, prepare);
        let (query, query_answer) = mpsc::channel();
        session
            .pending_operator_queries
            .lock()
            .unwrap()
            .insert(2, query);
        let (rpc, mut rpc_answer) = tokio::sync::oneshot::channel();
        session
            .pending_consensus_rpcs
            .lock()
            .unwrap()
            .insert(3, rpc);
        let (route, route_answer) = crate::actor::cooperative_channel();
        session.pending_http_routes.lock().unwrap().insert(4, route);
        let (reservation, reservation_answer) = crate::actor::cooperative_channel();
        session
            .pending_http_reservations
            .lock()
            .unwrap()
            .insert(5, reservation);
        let (spawn, spawn_answer) = crate::actor::cooperative_channel();
        session.pending_spawns.lock().unwrap().insert(6, spawn);

        let linked = ParkedProcess::new();
        linked.process.lock().links.insert(peer.pid(1));
        let trapping = ParkedProcess::new();
        {
            let mut process = trapping.process.lock();
            process.links.insert(peer.pid(2));
            process.trap_exit = true;
            process.state = ProcessState::Waiting;
        }
        let watching = ParkedProcess::new();
        {
            let mut process = watching.process.lock();
            process.monitors.insert(
                3,
                Monitor {
                    target: peer.pid(3),
                    message: MessageBuffer::new(b"down".to_vec(), 1),
                },
            );
            process.state = ProcessState::Waiting;
        }
        let ended = ParkedProcess::new();
        {
            let mut process = ended.process.lock();
            process.links.insert(peer.pid(4));
            process.monitors.insert(
                4,
                Monitor {
                    target: peer.pid(4),
                    message: MessageBuffer::new(b"down".to_vec(), 1),
                },
            );
            process.mark_exited(ExitReason::Normal);
        }
        let node_watcher = ParkedProcess::new();
        state
            .node_monitors
            .write()
            .entry(name.to_string())
            .or_default()
            .push((
                node_watcher.pid,
                MessageBuffer::new(b"node gone".to_vec(), 1),
            ));

        drop(peer);

        let gone = "peer_session_disconnected".to_string();
        assert_eq!(prepare_answer.try_recv(), Ok(Err(gone.clone())));
        assert_eq!(query_answer.try_recv(), Ok(Err(gone.clone())));
        assert_eq!(rpc_answer.try_recv(), Ok(Err(gone.clone())));
        assert_eq!(route_answer.try_recv(), Ok(Err(gone.clone())));
        assert_eq!(reservation_answer.try_recv(), Ok(Err(gone.clone())));
        assert_eq!(spawn_answer.try_recv(), Ok(Err(gone)));
        assert!(!state.sessions.read().contains_key(name));
        assert!(!state.node_id_map.read().contains_key(&session.node_id));

        let noconnection =
            ExitReason::Linked(peer_pid(&session, 1), Box::new(ExitReason::Noconnection));
        assert_eq!(
            format!("{:?}", linked.process.lock().state),
            format!("{:?}", ProcessState::Exited(noconnection))
        );
        assert_eq!(trapping.mailbox_len(), 1);
        assert!(matches!(trapping.process.lock().state, ProcessState::Ready));
        assert_eq!(watching.mailbox_len(), 1);
        assert!(watching.process.lock().monitors.is_empty());
        assert_eq!(ended.mailbox_len(), 0);
        assert_eq!(node_watcher.mailbox_len(), 1);
    }

    fn peer_pid(session: &NodeSession, local: u64) -> ProcessId {
        ProcessId::from_remote(session.node_id, session.remote_creation, local)
    }

    struct RemoteSpawnCall {
        target: String,
        link: u8,
        returned: mpsc::Sender<u64>,
    }

    /// `Node.spawn(target, peer_side_function)`, linked when `link` is 1.
    fn call_node_spawn(target: &str, link: u8) -> u64 {
        let function = "peer_side_function";
        mesh_node_spawn(
            target.as_ptr(),
            target.len() as u64,
            function.as_ptr(),
            function.len() as u64,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            link,
        )
    }

    extern "C" fn remote_spawn_caller(args: *const u8) {
        let call = unsafe { Box::from_raw(*(args as *const u64) as *mut RemoteSpawnCall) };
        let _ = call.returned.send(call_node_spawn(&call.target, call.link));
    }

    /// What `mesh_node_spawn`, called from an actor, returns for a spawn
    /// of `peer_side_function` on `target` (linked when `link` is 1).
    fn spawn_remotely(target: &str, link: u8) -> mpsc::Receiver<u64> {
        let (returned, result) = mpsc::channel();
        let call = Box::into_raw(Box::new(RemoteSpawnCall {
            target: target.to_string(),
            link,
            returned,
        })) as u64;
        let args = Box::leak(Box::new(call)) as *const u64 as *const u8;
        crate::actor::global_scheduler().spawn(remote_spawn_caller as *const u8, args, 8, 1);
        result
    }

    /// A remote spawn waits for the peer's reply and returns the pid it
    /// names, or 0 when the peer refuses. When the peer goes before it
    /// answers, the spawn fails rather than waiting for good.
    #[test]
    fn a_remote_spawn_returns_what_the_peer_answers_or_fails_when_it_goes() {
        let name = "spawn-target@127.0.0.1:1";
        let peer = TestPeer::new(name);
        // The request, past the exit signal a linked caller sends as it ends.
        let request = || loop {
            let frame = peer.next_sent();
            if frame[0] == DIST_SPAWN {
                return frame;
            }
            assert_eq!(frame[0], DIST_EXIT);
        };
        let answer = |status: u8, local: u64| {
            let request = request();
            peer.receive(frame(
                DIST_SPAWN_REPLY,
                &[&request[1..9], &[status], &local.to_le_bytes()],
            ));
        };
        let wait = Duration::from_secs(10);

        let returned = spawn_remotely(name, 1);
        answer(0, 12);
        assert_eq!(returned.recv_timeout(wait), Ok(peer.pid(12).as_u64()));

        let returned = spawn_remotely(name, 0);
        answer(1, 0);
        assert_eq!(returned.recv_timeout(wait), Ok(0));

        let returned = spawn_remotely(name, 0);
        request();
        drop(peer);
        assert_eq!(returned.recv_timeout(wait), Ok(0));
    }

    /// A remote spawn from outside an actor (a node's main, which has a
    /// process but no coroutine, or a runtime thread moving declared work,
    /// which has neither) blocks for the reply instead of yielding.
    #[test]
    fn a_remote_spawn_from_outside_an_actor_waits_for_the_reply() {
        let name = "thread-spawn-target@127.0.0.1:1";
        let peer = TestPeer::new(name);
        let main_like = ParkedProcess::new();
        for pid in [Some(main_like.pid), None] {
            let caller = std::thread::spawn(move || {
                if let Some(pid) = pid {
                    crate::actor::stack::set_current_pid(pid);
                }
                call_node_spawn(name, 1)
            });
            let request = peer.next_sent();
            assert_eq!(request[0], DIST_SPAWN);
            let requester = pid.map_or(0, |pid| pid.as_u64());
            assert_eq!(&request[9..17], &requester.to_le_bytes());
            peer.receive(frame(
                DIST_SPAWN_REPLY,
                &[&request[1..9], &[0], &12u64.to_le_bytes()],
            ));
            assert_eq!(caller.join().unwrap(), peer.pid(12).as_u64());
        }
        assert!(main_like.process.lock().links.contains(&peer.pid(12)));
    }

    /// Routes a request for `Owner.handle` to `owner` from a thread of its own.
    fn route_to(owner: &'static str) -> std::thread::JoinHandle<Result<Vec<u8>, String>> {
        std::thread::spawn(move || {
            execute_clustered_http_route_remote(
                owner,
                "Owner.handle",
                "routed-key",
                "attempt-1",
                b"GET /",
            )
        })
    }

    /// A request routed to its owner reserves capacity there, then runs
    /// there and returns what the owner answers. When the owner's session
    /// ends while it waits, the failure is one a replay-safe request may
    /// retry elsewhere.
    #[test]
    fn a_routed_request_reserves_then_runs_on_its_owner_and_retries_when_it_goes() {
        let name = "route-owner@127.0.0.1:1";
        let owner = TestPeer::new(name);
        let reserved = || {
            let (correlation, bytes, runtime, key) =
                decode_http_reserve(&owner.next_sent()).unwrap();
            assert_eq!(
                (bytes, runtime.as_str(), key.as_str()),
                (5, "Owner.handle", "routed-key")
            );
            correlation
        };

        let call = route_to(name);
        let correlation = reserved();
        owner.receive(encode_http_reserve_reply(correlation, Ok(())).unwrap());
        let query = decode_http_route_v2_query_frame(&owner.next_sent()).unwrap();
        assert_eq!(query.0, correlation);
        owner.receive(encode_http_route_v2_reply_frame(correlation, Ok(b"200".to_vec())).unwrap());
        assert_eq!(call.join().unwrap(), Ok(b"200".to_vec()));

        let call = route_to(name);
        let correlation = reserved();
        let draining = "owner_reservation_rejected:Draining".to_string();
        owner.receive(encode_http_reserve_reply(correlation, Err(draining.clone())).unwrap());
        assert_eq!(call.join().unwrap(), Err(draining));

        let call = route_to(name);
        let correlation = reserved();
        owner.receive(encode_http_reserve_reply(correlation, Ok(())).unwrap());
        assert_eq!(owner.next_sent()[0], DIST_HTTP_ROUTE_V2_QUERY);
        drop(owner);
        let reason = call.join().unwrap().unwrap_err();
        assert_eq!(reason, "peer_session_disconnected");
        assert!(retryable_clustered_http_transport_failure(&reason));

        assert_eq!(
            route_to(name).join().unwrap(),
            Err(format!("clustered_http_route_session_unavailable:{name}"))
        );
    }

    /// An owner that takes neither the reservation nor, later, the query
    /// times the request out; one that cannot be written to fails it at
    /// once. Each is a failure to retry.
    #[test]
    fn a_routed_request_times_out_or_fails_on_an_owner_that_does_not_take_it() {
        let silent = "silent-route-owner@127.0.0.1:1";
        let _silent_owner = TestPeer::new(silent);
        let slow = "slow-route-owner@127.0.0.1:1";
        let slow_owner = TestPeer::new(slow);
        let unanswered = route_to(silent);
        let accepted_only = route_to(slow);
        let (correlation, ..) = decode_http_reserve(&slow_owner.next_sent()).unwrap();
        slow_owner.receive(encode_http_reserve_reply(correlation, Ok(())).unwrap());
        for (call, reason) in [
            (unanswered, "clustered_http_reservation_timeout"),
            (accepted_only, "clustered_http_route_reply_timeout"),
        ] {
            let failure = call.join().unwrap().unwrap_err();
            assert_eq!(failure, reason);
            assert!(retryable_clustered_http_transport_failure(&failure));
        }

        let closed = "closed-route-owner@127.0.0.1:1";
        let closed_owner = TestPeer::new(closed);
        closed_owner.session.shutdown.store(true, Ordering::SeqCst);
        assert_eq!(
            route_to(closed).join().unwrap(),
            Err("clustered_http_reservation_write_failed:peer_session_shutdown".to_string())
        );
        let closing = "closing-route-owner@127.0.0.1:1";
        let closing_owner = TestPeer::new(closing);
        let call = route_to(closing);
        let (correlation, ..) = decode_http_reserve(&closing_owner.next_sent()).unwrap();
        closing_owner.session.shutdown.store(true, Ordering::SeqCst);
        closing_owner.receive(encode_http_reserve_reply(correlation, Ok(())).unwrap());
        let failure = call.join().unwrap().unwrap_err();
        assert_eq!(
            failure,
            "clustered_http_route_query_write_failed:peer_session_shutdown"
        );
        assert!(retryable_clustered_http_transport_failure(&failure));
    }

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// A consensus RPC goes to an autonomous peer over its session and
    /// returns the peer's reply; it fails at once for no peer, a peer that
    /// does not take part, or a session that cannot be written, and after
    /// its timeout for a peer that does not answer.
    #[test]
    fn a_consensus_rpc_returns_the_peer_reply_or_why_there_is_none() {
        test_node();
        let wait = Duration::from_secs(10);
        for (target, timeout) in [("", wait), ("someone@127.0.0.1:1", Duration::ZERO)] {
            assert_eq!(
                block_on(execute_mesh_consensus_rpc(
                    target,
                    Vec::new(),
                    false,
                    timeout
                )),
                Err("consensus_rpc_target_invalid".to_string())
            );
        }
        assert_eq!(
            block_on(execute_mesh_consensus_rpc(
                "no-consensus-peer@127.0.0.1:1",
                Vec::new(),
                false,
                wait
            )),
            Err("consensus_rpc_session_unavailable:no-consensus-peer@127.0.0.1:1".to_string())
        );

        let manual = TestPeer::new("manual-rpc-peer@127.0.0.1:1");
        assert_eq!(
            block_on(execute_mesh_consensus_rpc(
                &manual.session.remote_name,
                Vec::new(),
                false,
                wait
            )),
            Err("consensus_rpc_capability_unavailable".to_string())
        );
        assert_eq!(
            send_mesh_consensus_rpc_reply(&manual.session, 1, b"{}"),
            Err("consensus_rpc_capability_unavailable".to_string())
        );

        let name = "consensus-rpc-peer@127.0.0.1:1";
        let peer = TestPeer::authenticated(name, &["controller"]);
        for snapshot in [false, true] {
            let call = std::thread::spawn(move || {
                block_on(execute_mesh_consensus_rpc(
                    name,
                    b"{}".to_vec(),
                    snapshot,
                    wait,
                ))
            });
            let request = peer.next_sent();
            let (correlation, payload) =
                decode_consensus_rpc_frame(&request, DIST_CONSENSUS_RPC).unwrap();
            assert_eq!(payload, b"{}");
            send_mesh_consensus_rpc_reply(&peer.session, correlation, b"[]").unwrap();
            let reply = peer.next_sent();
            peer.receive(reply);
            assert_eq!(call.join().unwrap(), Ok(b"[]".to_vec()));
        }
        assert_eq!(
            block_on(execute_mesh_consensus_rpc(
                name,
                Vec::new(),
                false,
                Duration::from_millis(20)
            )),
            Err("consensus_rpc_reply_timeout".to_string())
        );
        assert!(peer
            .session
            .pending_consensus_rpcs
            .lock()
            .unwrap()
            .is_empty());
        peer.session.shutdown.store(true, Ordering::SeqCst);
        assert_eq!(
            block_on(execute_mesh_consensus_rpc(name, Vec::new(), false, wait)),
            Err("consensus_rpc_write_failed:peer_session_shutdown".to_string())
        );
    }

    /// A frame larger than a protocol-one peer reads is refused before it
    /// is queued: sent, it would make the peer end the session.
    #[test]
    fn a_session_refuses_a_frame_its_peer_would_end_the_session_over() {
        let peer = TestPeer::new("oversize-peer@127.0.0.1:1");
        let mut largest = vec![DIST_SEND];
        largest.resize(MAX_DIST_MSG as usize, 0);
        let mut oversized = largest.clone();
        oversized.push(0);
        assert_eq!(
            peer.session.send(OutboundClass::Application, oversized),
            Err("protocol_frame_bound_exceeded".to_string())
        );
        assert_eq!(
            peer.session
                .send(OutboundClass::Application, largest.clone()),
            Ok(())
        );
        let mut framed = (largest.len() as u32).to_le_bytes().to_vec();
        framed.extend_from_slice(&peer.sent()[0]);
        assert_eq!(
            PersistentFrameReader::default()
                .read_next(&mut std::io::Cursor::new(framed), MAX_DIST_MSG)
                .unwrap(),
            Some(largest)
        );
    }

    /// A handshake message is refused when it is not the one expected, is
    /// cut short, names its sender in bytes that are not text, or carries a
    /// protocol hello that does not decode; a stream that ends refuses too.
    #[test]
    fn handshake_messages_refuse_what_is_not_the_expected_message() {
        let framed = |payload: &[u8]| {
            let mut bytes = Vec::new();
            write_msg(&mut bytes, payload).unwrap();
            std::io::Cursor::new(bytes)
        };
        let mut sent = Vec::new();
        send_named(&mut sent, HANDSHAKE_CHALLENGE, "peer@host:1", 3, &[9; 32]).unwrap();
        let challenge = sent[4..].to_vec();
        let (name, creation, extra, _) =
            recv_named::<32>(&mut framed(&challenge), HANDSHAKE_CHALLENGE).unwrap();
        assert_eq!(
            (name.as_str(), creation, extra),
            ("peer@host:1", 3, [9; 32])
        );

        let refused =
            |bytes: &[u8]| recv_named::<32>(&mut framed(bytes), HANDSHAKE_CHALLENGE).unwrap_err();
        assert_eq!(
            refused(&[HANDSHAKE_NAME]),
            format!("expected handshake message {HANDSHAKE_CHALLENGE}, got {HANDSHAKE_NAME}")
        );
        assert_eq!(
            refused(&challenge[..3]),
            format!("handshake message {HANDSHAKE_CHALLENGE} too short")
        );
        assert_eq!(
            refused(&challenge[..20]),
            format!("handshake message {HANDSHAKE_CHALLENGE} truncated")
        );
        let mut not_text = challenge.clone();
        not_text[3] = 0xFF;
        assert_eq!(refused(&not_text), "invalid UTF-8 in node name");
        assert!(refused(&[&challenge[..], &[0xFF; 3]].concat()).contains("protocol"));
        assert!(
            recv_named::<32>(&mut std::io::Cursor::new(Vec::new()), HANDSHAKE_CHALLENGE)
                .unwrap_err()
                .contains("not received")
        );

        assert_eq!(
            recv_challenge_reply(&mut framed(&[HANDSHAKE_REPLY; 64])).unwrap_err(),
            format!("handshake message {HANDSHAKE_REPLY} too short")
        );
        assert_eq!(
            recv_challenge_ack(&mut framed(&[HANDSHAKE_REPLY; 33])).unwrap_err(),
            format!("expected handshake message {HANDSHAKE_ACK}, got {HANDSHAKE_REPLY}")
        );

        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(send_challenge_ack(&mut Closed, &[0; 32])
            .unwrap_err()
            .contains("not sent"));
    }

    /// Registers a session to `name` over a fresh loopback TLS connection,
    /// in the direction given (the stream end the test node holds), and
    /// returns what registration says with the other end of the stream.
    fn register_test_session(
        name: &str,
        direction: SessionDirection,
    ) -> Result<Arc<NodeSession>, String> {
        let state = test_node();
        let (client, server) = tls_pair();
        let stream = match direction {
            SessionDirection::Outgoing => NodeStream::ClientTls(client),
            SessionDirection::Incoming => NodeStream::ServerTls(server),
        };
        register_session(
            state,
            name.to_string(),
            1,
            state.assign_node_id(),
            stream,
            protocol_one(),
            None,
        )
    }

    /// Two nodes that connect to each other at once each end up with two
    /// transports; both keep the one the earlier-sorting name opened. A live
    /// session is not replaced otherwise, but a shut-down one is, and the
    /// session it replaced can no longer clean the replacement up.
    #[test]
    fn duplicate_sessions_keep_the_preferred_direction_and_replace_dead_ones() {
        let state = test_node();
        // The test node's name sorts before this one: it keeps its outgoing
        // transport.
        let later = "zz-duplicate-peer@127.0.0.1:1";
        assert_eq!(
            preferred_session_direction(&state.name, later),
            SessionDirection::Outgoing
        );
        let incoming = register_test_session(later, SessionDirection::Incoming).unwrap();
        let outgoing = register_test_session(later, SessionDirection::Outgoing).unwrap();
        assert!(incoming.shutdown.load(Ordering::SeqCst), "replaced");
        assert!(!state.node_id_map.read().contains_key(&incoming.node_id));
        assert_eq!(
            register_test_session(later, SessionDirection::Incoming).err(),
            Some(format!("already_connected:{later}"))
        );
        cleanup_session_if_current(&incoming);
        assert!(
            state.sessions.read().contains_key(later),
            "not by the replaced one"
        );

        outgoing.shutdown.store(true, Ordering::SeqCst);
        let replacement = register_test_session(later, SessionDirection::Incoming).unwrap();
        assert!(Arc::ptr_eq(
            state.sessions.read().get(later).unwrap(),
            &replacement
        ));
        replacement.shutdown.store(true, Ordering::SeqCst);
        cleanup_session_if_current(&replacement);
        assert!(!state.sessions.read().contains_key(later));

        // A name that sorts first keeps its own outgoing transport, which
        // is this node's incoming one.
        let earlier = "0-duplicate-peer@127.0.0.1:1";
        assert_eq!(
            preferred_session_direction(&state.name, earlier),
            SessionDirection::Incoming
        );
        let outgoing = register_test_session(earlier, SessionDirection::Outgoing).unwrap();
        let incoming = register_test_session(earlier, SessionDirection::Incoming).unwrap();
        assert!(outgoing.shutdown.load(Ordering::SeqCst));
        incoming.shutdown.store(true, Ordering::SeqCst);
        cleanup_session_if_current(&incoming);
    }

    /// An owner accepts a reservation for a handler it has, while it has
    /// room, and holds it for the query that follows; the query then runs
    /// on an actor and its reply says how it went. A query without a
    /// reservation, or a reservation for a handler it lacks or a request
    /// too large, is refused.
    #[test]
    fn an_owner_reserves_room_for_a_routed_request_then_runs_it() {
        extern "C" fn routed_test_handler(_request: *const u8) {}
        // The peer first: a test that clears the handlers waits for it.
        let peer = TestPeer::new("reserving-peer@127.0.0.1:1");
        let runtime_name = "PeerRouted.handle";
        let executable = "PeerRouted__handle";
        mesh_register_declared_handler(
            runtime_name.as_ptr(),
            runtime_name.len() as u64,
            executable.as_ptr(),
            executable.len() as u64,
            1,
            routed_test_handler as *const u8,
        );
        let reserve = |correlation: u64, runtime: &str, bytes: u32| {
            peer.receive(encode_http_reserve(correlation, runtime, "reserved-key", bytes).unwrap());
            decode_http_reserve_reply(&peer.next_sent()).unwrap()
        };
        let reserved = |correlation: u64| {
            peer.session
                .accepted_http_reservations
                .lock()
                .unwrap()
                .contains_key(&correlation)
        };
        assert_eq!(reserve(1, runtime_name, 5), (1, Ok(())));
        assert!(reserved(1));
        assert_eq!(
            reserve(1, runtime_name, 5),
            (1, Ok(())),
            "a repeat keeps it"
        );
        assert_eq!(
            reserve(2, "Unregistered.handle", 5),
            (
                2,
                Err("declared_handler_not_registered:Unregistered.handle".to_string())
            )
        );
        assert_eq!(
            reserve(3, runtime_name, MAX_DIST_MSG + 1),
            (3, Err("owner_reservation_payload_limit".to_string()))
        );
        peer.receive(vec![DIST_HTTP_RESERVE, 1]);
        assert_eq!(
            decode_http_reserve_reply(&peer.next_sent()).unwrap(),
            (0, Err("clustered_http_reservation_invalid".to_string()))
        );

        let query = |correlation: u64| {
            encode_http_route_v2_query_frame(
                correlation,
                runtime_name,
                "reserved-key",
                "attempt-1",
                b"not a request",
            )
            .unwrap()
        };
        peer.receive(query(9));
        assert_eq!(
            decode_http_route_v2_reply_frame(&peer.next_sent()).unwrap(),
            (9, Err("owner_reservation_missing_or_expired".to_string()))
        );
        peer.receive(query(1));
        let (correlation, result) = decode_http_route_v2_reply_frame(&peer.next_sent()).unwrap();
        assert_eq!(correlation, 1);
        assert!(
            result
                .as_ref()
                .unwrap_err()
                .starts_with("clustered_route_request_decode_failed"),
            "{result:?}"
        );
        assert!(!reserved(1), "the query took its reservation");

        peer.session.shutdown.store(true, Ordering::SeqCst);
        peer.receive(encode_http_reserve(4, runtime_name, "reserved-key", 5).unwrap());
        assert!(!reserved(4), "a reservation it cannot confirm is not held");
    }

    /// Reservations and their replies are framed whole.
    #[test]
    fn reservation_frames_refuse_malformed_bytes() {
        let reserve = encode_http_reserve(7, "Runtime.handle", "key", 12).unwrap();
        assert_eq!(
            decode_http_reserve(&reserve),
            Ok((7, 12, "Runtime.handle".to_string(), "key".to_string()))
        );
        let refused = |bytes: &[u8]| decode_http_reserve(bytes).unwrap_err();
        assert_eq!(
            refused(&reserve[..12]),
            "clustered_http_reservation_invalid"
        );
        assert_eq!(
            refused(&[&reserve[..], b"!"].concat()),
            "clustered_http_reservation_metadata_invalid"
        );
        let unnamed = encode_http_reserve(7, "", "key", 12).unwrap();
        assert_eq!(
            refused(&unnamed),
            "clustered_http_reservation_metadata_invalid"
        );

        let reply = encode_http_reserve_reply(7, Err("full".to_string())).unwrap();
        assert_eq!(
            decode_http_reserve_reply(&reply),
            Ok((7, Err("full".to_string())))
        );
        let refused = |bytes: &[u8]| decode_http_reserve_reply(bytes).unwrap_err();
        assert_eq!(
            refused(&reply[..11]),
            "clustered_http_reservation_reply_invalid"
        );
        assert_eq!(
            refused(&reply[..reply.len() - 1]),
            "clustered_http_reservation_reply_length_invalid"
        );
        let mut accepted_with_reason = reply.clone();
        accepted_with_reason[9] = 1;
        assert_eq!(
            refused(&accepted_with_reason),
            "clustered_http_reservation_reply_status_invalid"
        );
        let mut not_text = reply;
        not_text[12] = 0xFF;
        assert_eq!(
            refused(&not_text),
            "clustered_http_reservation_reason_invalid"
        );
        assert_eq!(
            encode_http_reserve_reply(7, Err("x".repeat(70_000))),
            Err("clustered_http_reservation_reason_too_large".to_string())
        );
    }

    /// A connection a `FakeNode` took through the handshake: the test
    /// plays the node on it.
    struct FakeConnection {
        stream: StreamOwned<rustls::ServerConnection, TcpStream>,
        remote: String,
        negotiated: NegotiatedProtocol,
    }

    impl FakeConnection {
        fn send(&mut self, payload: Vec<u8>) {
            let frame =
                encode_session_payload(OutboundClass::Control, payload, &self.negotiated).unwrap();
            write_msg(&mut self.stream, &frame).unwrap();
        }

        /// The next frame of kind `tag` the node sends, past any others.
        fn receive(&mut self, tag: u8) -> Vec<u8> {
            loop {
                let frame = read_dist_msg(&mut self.stream).expect("a frame from the node");
                let message = decode_session_payload(frame, &self.negotiated).unwrap();
                if message[0] == tag {
                    return message;
                }
            }
        }
    }

    /// A node of the test's own on a loopback port: it takes connections
    /// through the cookie handshake as a node does, sends its (empty)
    /// global names as a node does, and hands the test the connection.
    struct FakeNode {
        name: String,
        cookie: &'static str,
        listener: TcpListener,
        _member: parking_lot::RwLockReadGuard<'static, ()>,
    }

    impl FakeNode {
        fn new(prefix: &str, cookie: &'static str) -> Self {
            let member = TEST_PEERS.read_recursive();
            test_node();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let name = format!(
                "{prefix}@127.0.0.1:{}",
                listener.local_addr().unwrap().port()
            );
            Self {
                name,
                cookie,
                listener,
                _member: member,
            }
        }

        fn accept(&self) -> std::thread::JoinHandle<Result<FakeConnection, String>> {
            let listener = self.listener.try_clone().unwrap();
            let (name, cookie) = (self.name.clone(), self.cookie);
            std::thread::spawn(move || {
                let (tcp, _) = listener.accept().map_err(|error| error.to_string())?;
                tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let (cert, key) = generate_ephemeral_cert();
                let mut stream = StreamOwned::new(
                    rustls::ServerConnection::new(build_node_server_config(cert, key)).unwrap(),
                    tcp,
                );
                let (remote, _, negotiated, _) =
                    perform_handshake_with_identity(&mut stream, &name, cookie, 1, false)?;
                let mut connection = FakeConnection {
                    stream,
                    remote,
                    negotiated,
                };
                connection.send(vec![DIST_GLOBAL_SYNC, 0, 0, 0, 0]);
                Ok(connection)
            })
        }
    }

    fn node_connect(target: &str) -> i64 {
        mesh_node_connect(target.as_ptr(), target.len() as u64)
    }

    /// Waits until the test node has no session to `name`.
    fn await_session_gone(name: &str) {
        let state = test_node();
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.sessions.read().contains_key(name) {
            assert!(Instant::now() < deadline, "the session to {name} stays");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Node.connect authenticates with a node, keeps the session, and
    /// returns once it knows the node's global names; connecting again finds
    /// that session. When the node goes, so does the session.
    #[test]
    fn node_connect_opens_an_authenticated_session_that_ends_with_the_node() {
        let state = test_node();
        let node = FakeNode::new("connect-target", TEST_NODE_COOKIE);
        let accepted = node.accept();
        assert_eq!(node_connect(&node.name), 0);
        let mut connection = accepted.join().unwrap().unwrap();
        assert_eq!(connection.remote, state.name);
        connection.receive(DIST_GLOBAL_SYNC);
        let session = state.sessions.read().get(&node.name).cloned().unwrap();
        assert!(session.global_names_received.load(Ordering::Acquire));

        let again = node.accept();
        assert_eq!(node_connect(&node.name), 0);
        let _second = again.join().unwrap().unwrap();
        assert!(Arc::ptr_eq(
            state.sessions.read().get(&node.name).unwrap(),
            &session
        ));

        drop(connection);
        await_session_gone(&node.name);
    }

    /// Node.connect says why it could not connect: -2 when nothing listens
    /// there, -3 for a target it cannot parse or read and for a node that
    /// does not know the cookie.
    #[test]
    fn node_connect_says_why_it_could_not_connect() {
        test_node();
        assert_eq!(node_connect("unreachable-peer@127.0.0.1:1"), -2);
        assert_eq!(node_connect("no-at-sign"), -3);
        assert_eq!(mesh_node_connect([0xFF, 0xFE].as_ptr(), 2), -3);
        let stranger = FakeNode::new("cookie-stranger", "a-cookie-the-test-node-lacks");
        let accepted = stranger.accept();
        assert_eq!(node_connect(&stranger.name), -3);
        assert!(accepted
            .join()
            .unwrap()
            .err()
            .is_some_and(|error| error.contains("cookie mismatch")));
    }

    /// A remote spawn over a session that can no longer send drops it,
    /// connects afresh (within the retry budget) and sends there; it fails
    /// when the node cannot be reached again, or when the request cannot
    /// be sent even then.
    #[test]
    fn a_remote_spawn_reconnects_once_over_a_dead_session() {
        let node = FakeNode::new("respawn-target", TEST_NODE_COOKIE);
        let dead = TestPeer::new(&node.name);
        dead.session.shutdown.store(true, Ordering::SeqCst);
        let accepted = node.accept();
        let target: &'static str = Box::leak(node.name.clone().into_boxed_str());
        let call = std::thread::spawn(move || call_node_spawn(target, 0));
        let mut connection = accepted.join().unwrap().unwrap();
        let request = connection.receive(DIST_SPAWN);
        connection.send(frame(
            DIST_SPAWN_REPLY,
            &[&request[1..9], &[0], &21u64.to_le_bytes()],
        ));
        let spawned = call.join().unwrap();
        let session = test_node().sessions.read().get(target).cloned().unwrap();
        assert!(!Arc::ptr_eq(&session, &dead.session));
        assert_eq!(
            spawned,
            ProcessId::from_remote(session.node_id, session.remote_creation, 21).as_u64()
        );
        drop(connection);
        await_session_gone(target);

        let gone = TestPeer::new("respawn-gone@127.0.0.1:1");
        gone.session.shutdown.store(true, Ordering::SeqCst);
        assert_eq!(call_node_spawn("respawn-gone@127.0.0.1:1", 0), 0);
    }

    /// A remote spawn needs a node, a function, and arguments that match
    /// their tags; without them, or without a way to the node, it is 0.
    #[test]
    fn a_remote_spawn_without_a_target_or_whole_arguments_is_zero() {
        test_node();
        assert_eq!(call_node_spawn("", 0), 0);
        assert_eq!(call_node_spawn("unreachable-spawn@127.0.0.1:1", 0), 0);
        let peer = TestPeer::new("argument-spawn-peer@127.0.0.1:1");
        let spawn = |function: &str, args: &[u8], tags: Option<&[u8]>, count: u64| {
            mesh_node_spawn(
                peer.session.remote_name.as_ptr(),
                peer.session.remote_name.len() as u64,
                function.as_ptr(),
                function.len() as u64,
                args.as_ptr(),
                args.len() as u64,
                tags.map_or(std::ptr::null(), <[u8]>::as_ptr),
                count,
                0,
            )
        };
        assert_eq!(spawn("", &[], None, 0), 0);
        assert_eq!(spawn("f", &[0; 8], None, 1), 0, "tags missing");
        assert_eq!(
            spawn("f", &[0; 4], Some(&[REMOTE_SPAWN_ARG_INT]), 1),
            0,
            "cut short"
        );
        assert!(peer.sent().is_empty());
    }

    /// Connects to the node at `target` as `name`, with `cookie`.
    fn connect_as(
        target: &str,
        name: &str,
        cookie: &str,
    ) -> Result<
        (
            StreamOwned<rustls::ClientConnection, TcpStream>,
            Authenticated,
        ),
        String,
    > {
        connect_authenticated(
            target,
            build_node_client_config(),
            name,
            cookie,
            1,
            Duration::from_secs(10),
        )
    }

    /// Reads until the node closes `stream`.
    fn await_closed(stream: &mut impl Read) {
        while read_dist_msg(stream).is_ok() {}
    }

    /// The test node authenticates who connects to it: a peer that knows
    /// the cookie gets a session, which sends it the node's global names and
    /// ends when the peer goes, and a second connection from the same peer
    /// is dropped; one that does not know the cookie is turned away.
    #[test]
    fn the_node_accepts_peers_that_know_its_cookie() {
        let _member = TEST_PEERS.read_recursive();
        let state = test_node();
        // A name that sorts before the node's: the node prefers the
        // transport such a peer opened.
        let name = "0-accepted-peer@127.0.0.1:1";
        let listener = start_one_shot_test_listener().unwrap();
        let (mut stream, (remote, _, negotiated, _)) =
            connect_as(&listener, name, TEST_NODE_COOKIE).unwrap();
        assert_eq!(remote, state.name);
        while decode_session_payload(read_dist_msg(&mut stream).unwrap(), &negotiated).unwrap()[0]
            != DIST_GLOBAL_SYNC
        {}
        let session = state.sessions.read().get(name).cloned().unwrap();
        assert_eq!(session.direction, SessionDirection::Incoming);

        let listener = start_one_shot_test_listener().unwrap();
        let (mut second, _) = connect_as(&listener, name, TEST_NODE_COOKIE).unwrap();
        await_closed(&mut second);
        assert!(Arc::ptr_eq(
            state.sessions.read().get(name).unwrap(),
            &session
        ));
        drop(stream);
        await_session_gone(name);

        let listener = start_one_shot_test_listener().unwrap();
        assert!(connect_as(&listener, "stranger@127.0.0.1:1", "not-the-cookie").is_err());
    }

    /// A transient operator connection carries one operator query: one that
    /// sends none, an empty frame, or another kind of message is closed
    /// unanswered. A query to a node that cannot be reached, or does not
    /// take the cookie, says why.
    #[test]
    fn a_transient_operator_connection_carries_one_query() {
        test_node();
        for payload in [None, Some(Vec::new()), Some(vec![DIST_SEND])] {
            let listener = start_one_shot_test_listener().unwrap();
            let (mut stream, _) = connect_as(
                &listener,
                &transient_operator_client_name(),
                TEST_NODE_COOKIE,
            )
            .unwrap();
            match payload {
                Some(payload) => write_msg(&mut stream, &payload).unwrap(),
                None => stream.sock.shutdown(std::net::Shutdown::Write).unwrap(),
            }
            await_closed(&mut stream);
        }

        let query = |target: &str, cookie: &str| {
            execute_transient_operator_query(target, cookie, &[], Duration::from_secs(10))
                .unwrap_err()
        };
        assert!(query("no-at-sign", TEST_NODE_COOKIE).starts_with("invalid connect target"));
        assert!(query("absent@127.0.0.1:1", TEST_NODE_COOKIE).starts_with("TCP connect"));
        let listener = start_one_shot_test_listener().unwrap();
        assert!(query(&listener, "not-the-cookie").starts_with("handshake with"));
    }

    /// The node's listener takes at most `MAX_INCOMING_HANDSHAKES`
    /// connections that have not authenticated; one more is closed at once.
    #[test]
    fn the_listener_turns_away_connections_beyond_the_handshake_limit() {
        let state = test_node();
        let await_active = |count: usize| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while ACTIVE_INCOMING_HANDSHAKES.load(Ordering::Acquire) != count {
                assert!(Instant::now() < deadline, "handshakes in progress stay");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let pending: Vec<TcpStream> = (0..MAX_INCOMING_HANDSHAKES)
            .map(|_| TcpStream::connect(("127.0.0.1", state.port)).unwrap())
            .collect();
        await_active(MAX_INCOMING_HANDSHAKES);
        let mut extra = TcpStream::connect(("127.0.0.1", state.port)).unwrap();
        extra
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(extra.read(&mut [0; 1]).unwrap(), 0, "closed unread");
        drop(pending);
        await_active(0);
    }

    /// A node starts only with a name, a cookie and TLS it can use (-3),
    /// on an address it can bind (-2). Started on port 0, it advertises the
    /// port it got; on a port of its own, the name it was given.
    #[test]
    fn a_node_starts_only_with_what_it_can_use() {
        let state = test_node();
        let refused = |name: &str, cookie: &str| bind_node(name, cookie).err();
        assert_eq!(refused("no-at-sign", TEST_NODE_COOKIE), Some(-3));
        assert_eq!(refused("n@127.0.0.1:0", " , "), Some(-3), "no cookie");
        let taken = format!("n@127.0.0.1:{}", state.port);
        assert_eq!(refused(&taken, TEST_NODE_COOKIE), Some(-2));
        autonomous(|| {
            assert_eq!(refused("n@127.0.0.1:0", "short"), Some(-3));
            assert_eq!(
                refused("n@127.0.0.1:0", &"k".repeat(32)),
                Some(-3),
                "no mTLS"
            );
        });
        let bad_text = [0xFF, 0xFE];
        assert_eq!(
            mesh_node_start(bad_text.as_ptr(), 2, TEST_NODE_COOKIE.as_ptr(), 3),
            -3
        );

        let (node, listener) = bind_node("fresh@127.0.0.1:0", TEST_NODE_COOKIE).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(node.name, format!("fresh@127.0.0.1:{port}"));
        drop(listener);
        let fixed = format!("fixed@127.0.0.1:{port}");
        let (node, _listener) = bind_node(&fixed, TEST_NODE_COOKIE).unwrap();
        assert_eq!((node.name, node.port), (fixed, port));
    }

    /// A node's mTLS identity: none configured, all three parts, or an
    /// error naming the part that is missing, not base64, empty, or not a
    /// certificate or key TLS takes. Without one a node falls back to an
    /// ephemeral certificate, which autonomous mode does not allow.
    #[test]
    fn a_node_takes_an_mtls_identity_only_when_whole() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let base64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let (cert, key) = generate_ephemeral_cert();
        let cert = base64(cert.as_ref());
        let key = base64(key.secret_der());
        let values = |ca: &str, cert: &str, key: &str| {
            [
                Some(ca.to_string()),
                Some(cert.to_string()),
                Some(key.to_string()),
            ]
        };
        let refused = |values: MtlsValues| mtls_configs(&values).err().unwrap();

        assert!(mtls_configs(&[None, None, None]).unwrap().is_none());
        assert!(mtls_configs(&values(&cert, &cert, &key)).unwrap().is_some());
        assert_eq!(
            refused([Some(cert.clone()), None, None]),
            "mesh_mtls_configuration_incomplete"
        );
        assert_eq!(
            refused(values("%%", &cert, &key)),
            format!("{TLS_CA_DER_B64_ENV}_invalid_base64")
        );
        assert_eq!(
            refused(values(&cert, "", &key)),
            format!("{TLS_CERT_DER_B64_ENV}_empty")
        );
        let garbage = base64(b"not der");
        assert!(refused(values(&garbage, &cert, &key)).starts_with("mesh_mtls_ca_invalid"));
        assert!(refused(values(&cert, &cert, &garbage))
            .starts_with("mesh_mtls_server_identity_invalid"));

        assert!(node_tls_configs(&values(&cert, &cert, &key)).is_ok());
        assert!(node_tls_configs(&[None, None, None]).is_ok());
        assert_eq!(
            autonomous(|| node_tls_configs(&[None, None, None]).err()),
            Some("autonomous_mode_requires_mtls_identity".to_string())
        );
    }

    /// Remote spawn arguments cross by their tags: numbers and booleans as
    /// bits, text by value, unit as nothing, and a pid as its local id and
    /// its node. An argument of a kind that cannot cross, bytes cut short
    /// anywhere, or bytes left over are refused.
    #[test]
    fn remote_spawn_arguments_cross_by_their_tags() {
        test_node();
        let tags = [
            REMOTE_SPAWN_ARG_INT,
            REMOTE_SPAWN_ARG_FLOAT,
            REMOTE_SPAWN_ARG_BOOL,
            REMOTE_SPAWN_ARG_STRING,
            REMOTE_SPAWN_ARG_UNIT,
            REMOTE_SPAWN_ARG_STRING,
            REMOTE_SPAWN_ARG_PID,
        ];
        let words = [
            42,
            1.5f64.to_bits(),
            7,
            crate::string::mesh_str("hello") as u64,
            0,
            0,
            ProcessId(9).as_u64(),
        ];
        let data: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let encoded = encode_remote_spawn_args(&data, &tags).unwrap();
        let decoded = decode_remote_spawn_args(&encoded, &tags).unwrap();
        let text = |raw: u64| unsafe { (*(raw as *const crate::string::MeshString)).as_str() };
        assert_eq!(decoded[..3], [42, 1.5f64.to_bits(), 1]);
        assert_eq!(text(decoded[3]), "hello");
        assert_eq!(decoded[4], 0);
        assert_eq!(text(decoded[5]), "");
        assert_eq!(decoded[6], 9, "a pid on this node is local here");

        for cut in 2..encoded.len() {
            assert!(
                decode_remote_spawn_args(&encoded[..cut], &tags).is_err(),
                "{cut} bytes"
            );
        }
        assert_eq!(
            decode_remote_spawn_args(&[&encoded[..], &[0]].concat(), &tags),
            Err("remote_spawn_args_trailing_bytes".to_string())
        );
        assert_eq!(
            decode_remote_spawn_args(&[1], &[]),
            Err("remote_spawn_args_too_short".to_string())
        );
        assert_eq!(
            decode_remote_spawn_args(&[1, 0, 9], &[9]),
            Err("remote_spawn_arg_tag_unsupported:9".to_string())
        );
        assert_eq!(
            encode_remote_spawn_args(&data[..8], &tags),
            Err("remote_spawn_args_size_mismatch".to_string())
        );
        assert_eq!(
            encode_remote_spawn_args(&[0; 8], &[REMOTE_SPAWN_ARG_UNSUPPORTED]),
            Err("remote_spawn_arg_tag_unsupported:0".to_string())
        );
    }

    /// A persistent session over loopback TLS that nothing serves, and the
    /// stream its peer holds.
    fn loose_session(
        protocol: NegotiatedProtocol,
    ) -> (
        Arc<NodeSession>,
        StreamOwned<rustls::ServerConnection, TcpStream>,
    ) {
        let (client, server) = tls_pair();
        let session = Arc::new(NodeSession::new(
            RemoteSessionEndpoint {
                remote_name: "loose-peer@127.0.0.1:1".to_string(),
                remote_creation: 1,
                node_id: 0,
                direction: SessionDirection::Outgoing,
            },
            NodeStream::ClientTls(client),
            true,
            protocol,
            None,
        ));
        (session, server)
    }

    /// Queues frames on `session`'s stream until the socket takes no more
    /// (its peer not reading), then a heartbeat, which stays queued.
    fn fill_until_heartbeat_waits(session: &NodeSession) {
        loop {
            let mut stream = session.stream.lock();
            stream.queue_frame(&[HEARTBEAT_PONG; 64 * 1024]).unwrap();
            if !stream.flush_queued().unwrap().1 {
                break;
            }
        }
        session.send_heartbeat(vec![HEARTBEAT_PING; 9]).unwrap();
        assert!(session.tls_output_pending.load(Ordering::Acquire));
    }

    fn spawn_writer(session: &Arc<NodeSession>) -> std::thread::JoinHandle<()> {
        let session = Arc::clone(session);
        std::thread::spawn(move || writer_loop_session(session))
    }

    /// When nothing else is queued, the writer sends what a heartbeat left
    /// in the stream as soon as the peer takes it.
    #[test]
    fn the_writer_flushes_what_a_heartbeat_left_when_the_peer_reads() {
        let (session, mut peer) = loose_session(protocol_one());
        fill_until_heartbeat_waits(&session);
        let writer = spawn_writer(&session);
        peer.sock
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        while read_dist_msg(&mut peer).unwrap()[0] != HEARTBEAT_PING {}
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.tls_output_pending.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the heartbeat stays queued");
            std::thread::sleep(Duration::from_millis(5));
        }
        session.shutdown.store(true, Ordering::SeqCst);
        writer.join().unwrap();
    }

    /// A writer whose peer has gone ends the session, whether it was
    /// sending frames or flushing what a heartbeat left.
    #[test]
    fn the_writer_ends_its_session_when_the_peer_is_gone() {
        let (session, peer) = loose_session(protocol_one());
        drop(peer);
        let writer = spawn_writer(&session);
        while !session.shutdown.load(Ordering::Acquire) {
            let _ = session.send(OutboundClass::Application, vec![DIST_SEND; 64 * 1024]);
            std::thread::sleep(Duration::from_millis(5));
        }
        writer.join().unwrap();

        let (session, peer) = loose_session(protocol_one());
        fill_until_heartbeat_waits(&session);
        drop(peer);
        spawn_writer(&session).join().unwrap();
        assert!(session.shutdown.load(Ordering::Acquire));
        assert_eq!(
            session.send_heartbeat(vec![HEARTBEAT_PING; 9]),
            Err("peer_session_shutdown".to_string())
        );
        assert_eq!(
            session.send(OutboundClass::Control, vec![DIST_PEER_LIST]),
            Err("peer_session_shutdown".to_string())
        );
    }

    /// A reader ends its session on a frame it cannot take: one longer than
    /// the protocol allows, or, in protocol two, one that is no envelope.
    #[test]
    fn the_reader_ends_its_session_on_a_frame_it_cannot_take() {
        for (protocol, oversized) in [(protocol_one(), true), (protocol_two(), false)] {
            let (session, mut peer) = loose_session(protocol);
            let reader = std::thread::spawn({
                let session = Arc::clone(&session);
                move || {
                    reader_loop_session(
                        session,
                        Arc::new(Mutex::new(HeartbeatState::new(
                            Duration::from_secs(60),
                            Duration::from_secs(15),
                        ))),
                    )
                }
            });
            if oversized {
                peer.write_all(&(MAX_DIST_MSG + 1).to_le_bytes()).unwrap();
                peer.flush().unwrap();
            } else {
                write_msg(&mut peer, &[DIST_SEND, 1, 2]).unwrap();
            }
            reader.join().unwrap();
            assert!(session.shutdown.load(Ordering::Acquire));
        }
    }

    /// The heartbeat pings the peer and reports this node's load; when no
    /// pong comes in time, or the ping cannot be written, it ends the
    /// session and removes it.
    #[test]
    fn the_heartbeat_ends_a_session_whose_peer_does_not_answer() {
        let beat = |session: &Arc<NodeSession>| {
            let heartbeat = Arc::new(Mutex::new(HeartbeatState::new(
                Duration::ZERO,
                Duration::from_millis(50),
            )));
            let session = Arc::clone(session);
            let name = session.remote_name.clone();
            std::thread::spawn(move || heartbeat_loop_session(session, heartbeat, name))
        };
        let state = test_node();

        let mut silent = TestPeer::new("silent-heartbeat-peer@127.0.0.1:1");
        let beating = beat(&silent.session);
        silent
            .stream
            .sock
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(read_msg(&mut silent.stream).unwrap()[0], HEARTBEAT_PING);
        beating.join().unwrap();
        assert!(!state
            .sessions
            .read()
            .contains_key("silent-heartbeat-peer@127.0.0.1:1"));
        assert!(silent
            .sent()
            .iter()
            .any(|frame| frame[0] == DIST_LOAD_REPORT));

        let (session, peer) = loose_session(protocol_one());
        loop {
            let mut stream = session.stream.lock();
            stream.queue_frame(&[HEARTBEAT_PONG; 64 * 1024]).unwrap();
            if !stream.flush_queued().unwrap().1 {
                break;
            }
        }
        drop(peer);
        beat(&session).join().unwrap();
        assert!(session.shutdown.load(Ordering::Acquire));
    }

    /// Why a declared handler's submission was refused: the record's error,
    /// or else the conflict it hit, or else the outcome itself.
    #[test]
    fn a_refused_submission_says_why() {
        use crate::dist::continuity::{SubmitDecision, SubmitOutcome};
        let mut decision = SubmitDecision {
            outcome: SubmitOutcome::Conflict,
            record: continuity_record("refused-key", "owner@h:1", "replica@h:1"),
            conflict_reason: String::new(),
        };
        assert_eq!(
            rejected_submit_reason(&decision),
            "declared_handler_submit_rejected:conflict"
        );
        decision.conflict_reason = "payload_hash_mismatch".to_string();
        assert_eq!(rejected_submit_reason(&decision), "payload_hash_mismatch");
        decision.record.error = "replica_required_unavailable".to_string();
        assert_eq!(
            rejected_submit_reason(&decision),
            "replica_required_unavailable"
        );
    }

    /// A clustered HTTP request needs its handler's name, a request key, a
    /// payload hash, a payload, and a handler registered under that name.
    #[test]
    fn a_clustered_http_request_needs_its_identity_its_payload_and_a_handler() {
        let refused = |runtime: &str, key: &str, hash: &str, payload: &[u8]| {
            execute_clustered_http_route(runtime, key, hash, payload)
                .err()
                .unwrap()
        };
        assert_eq!(
            refused(" ", "key", "hash", b"GET /"),
            "declared_handler_runtime_name_missing"
        );
        assert_eq!(refused("R.h", "", "hash", b"GET /"), "request_key_missing");
        assert_eq!(refused("R.h", "key", "", b"GET /"), "payload_hash_missing");
        assert_eq!(
            refused("R.h", "key", "hash", b""),
            "clustered_http_route_request_payload_missing"
        );
        assert_eq!(
            refused("Never.registered", "key", "hash", b"GET /"),
            "declared_handler_not_registered:Never.registered"
        );
    }

    /// A capacity provider's name for a node resolves to the member it is:
    /// by its exact name, or by a 12-character prefix only one member has.
    #[test]
    fn a_provider_node_identifier_resolves_to_exactly_one_member() {
        let state = test_node();
        let _first = TestPeer::new("ambiguous-member-a@127.0.0.1:1");
        let _second = TestPeer::new("ambiguous-member-b@127.0.0.1:1");
        assert_eq!(
            resolve_runtime_node_id(&format!(" {} ", state.name)),
            Ok(state.name.clone())
        );
        let prefix: String = state.name.chars().take(12).collect();
        assert_eq!(
            resolve_runtime_node_id(&format!("{prefix}-container-id")),
            Ok(state.name.clone())
        );
        assert_eq!(
            resolve_runtime_node_id(" "),
            Err("runtime_node_identifier_missing".to_string())
        );
        assert_eq!(
            resolve_runtime_node_id("nobody-at-all"),
            Err("runtime_node_not_found:nobody-at-all".to_string())
        );
        assert_eq!(
            resolve_runtime_node_id("ambiguous-member-z"),
            Err("runtime_node_identifier_ambiguous:ambiguous-member-z".to_string())
        );
    }

    /// Startup work cannot wait for a cluster it cannot see at all.
    #[test]
    fn startup_convergence_needs_a_membership_to_watch() {
        assert_eq!(
            wait_for_startup_convergence_with(Vec::new, || {}, 1, 3),
            Err("declared_work_membership_empty".to_string())
        );
        let mut first = true;
        let observe = || {
            if std::mem::take(&mut first) {
                vec!["alone@127.0.0.1:1".to_string()]
            } else {
                Vec::new()
            }
        };
        assert_eq!(
            wait_for_startup_convergence_with(observe, || {}, 1, 3),
            Err("declared_work_membership_empty".to_string())
        );
    }

    fn authority(
        cluster_role: crate::dist::continuity::ContinuityClusterRole,
    ) -> crate::dist::continuity::ContinuityAuthorityStatus {
        crate::dist::continuity::ContinuityAuthorityStatus {
            cluster_role,
            promotion_epoch: 0,
            replication_health: crate::dist::continuity::ReplicationHealth::LocalOnly,
        }
    }

    /// A standby promotes itself when its primary goes only when nothing
    /// else could still be in charge: it is a standby, no peer remains, and
    /// every pending record is one the lost primary owned and this node
    /// mirrored. Those are the records it then resumes.
    #[test]
    fn a_standby_promotes_only_when_what_it_mirrored_is_all_that_is_pending() {
        use crate::dist::continuity::{
            ContinuityClusterRole::{Primary, Standby},
            ContinuitySnapshot, ReplicaStatus,
        };
        let (local, lost) = ("standby@h:1", "primary@h:1");
        let mirrored = |key: &str| {
            let mut record = continuity_record(key, lost, local);
            record.cluster_role = Standby;
            record.replica_status = ReplicaStatus::Mirrored;
            record.declared_handler_runtime_name = "Work.resume".to_string();
            record
        };
        let snapshot = |records: Vec<ContinuityRecord>| ContinuitySnapshot {
            next_attempt_token: 1,
            records,
        };
        let reason = |peers: usize, role, records: Vec<ContinuityRecord>| {
            automatic_promotion_reason(local, lost, peers, authority(role), &snapshot(records))
        };

        assert_eq!(reason(0, Standby, vec![mirrored("m")]), Ok(()));
        assert_eq!(
            reason(0, Primary, vec![mirrored("m")]),
            Err(AUTOMATIC_PROMOTION_REJECTED_NOT_STANDBY)
        );
        assert_eq!(
            reason(1, Standby, vec![mirrored("m")]),
            Err(AUTOMATIC_PROMOTION_REJECTED_PEERS_REMAINING)
        );
        assert_eq!(
            reason(0, Standby, Vec::new()),
            Err(AUTOMATIC_PROMOTION_REJECTED_NO_MIRRORED_STATE)
        );
        let mut primary_record = mirrored("p");
        primary_record.cluster_role = Primary;
        let mut elsewhere = mirrored("e");
        elsewhere.owner_node = "other@h:1".to_string();
        for stray in [primary_record, elsewhere] {
            assert_eq!(
                reason(0, Standby, vec![mirrored("m"), stray]),
                Err(AUTOMATIC_PROMOTION_REJECTED_AMBIGUOUS_PENDING)
            );
        }

        let mut owner_lost = mirrored("resume-me");
        owner_lost.cluster_role = Primary;
        owner_lost.replica_status = ReplicaStatus::OwnerLost;
        let mut done = owner_lost.clone();
        done.request_key = "done".to_string();
        done.phase = crate::dist::continuity::ContinuityPhase::Completed;
        assert_eq!(
            automatic_recovery_candidates(lost, &snapshot(vec![owner_lost, done, mirrored("m")])),
            vec![(
                "resume-me".to_string(),
                "attempt-1".to_string(),
                "sha256:payload".to_string(),
                "Work.resume".to_string()
            )]
        );
    }

    /// A node dialing a legacy peer that speaks TLS 1.2 still checks the
    /// server's handshake signature, though it trusts any certificate.
    #[test]
    fn a_legacy_client_checks_a_tls_1_2_server_signature() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert, key) = generate_ephemeral_cert();
        let server_config = Arc::new(
            ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut tls =
                StreamOwned::new(rustls::ServerConnection::new(server_config).unwrap(), tcp);
            tls.conn.complete_io(&mut tls.sock).map(|_| ())
        });
        let mut tls = StreamOwned::new(
            rustls::ClientConnection::new(
                build_node_client_config(),
                "mesh-node".try_into().unwrap(),
            )
            .unwrap(),
            TcpStream::connect(("127.0.0.1", port)).unwrap(),
        );
        while tls.conn.is_handshaking() {
            tls.conn.complete_io(&mut tls.sock).unwrap();
        }
        assert_eq!(
            tls.conn.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_2)
        );
        server.join().unwrap().unwrap();
    }

    /// Tells the test node that the node `name` is a ready worker serving
    /// `handlers`.
    fn report_worker(name: &str, handlers: &[&str]) {
        let mut report = crate::dist::routing::local_load_report(
            name,
            handlers.iter().map(|handler| handler.to_string()).collect(),
        );
        report.roles = crate::dist::telemetry::NodeRoles::new(false, true, true);
        report.state = crate::dist::telemetry::NodeLifecycleState::Ready;
        crate::dist::routing::load_report_registry()
            .apply(report, Instant::now())
            .unwrap();
    }

    /// A pending record of declared work `handler`, owned by `owner` with
    /// the one acknowledged replica `replica`, merged into this node's
    /// continuity registry.
    fn merge_pending_record(key: &str, owner: &str, replica: &str, handler: &str) {
        let mut record = continuity_record(key, owner, replica);
        record.acknowledged_replica_nodes = vec![replica.to_string()];
        record.replica_status = crate::dist::continuity::ReplicaStatus::Mirrored;
        record.declared_handler_runtime_name = handler.to_string();
        if key.contains("http") {
            record.request_payload = b"GET /drained".to_vec();
        }
        crate::dist::continuity::continuity_registry()
            .merge_remote_record(1, record)
            .unwrap();
    }

    /// Plays `peers` as live nodes until `done`: each replica prepare is
    /// acknowledged (refused for a record whose key says `unprepared`, and
    /// for one that says `superseded` after completing it), each
    /// reservation accepted (turned away, as by a draining owner, when its
    /// key says `turned-away`), each routed request answered (with an error
    /// when its key says `failing`) and each spawn given a pid.
    fn serve_as_nodes(peers: &[&TestPeer], done: &AtomicBool) {
        let registry = crate::dist::continuity::continuity_registry();
        while !done.load(Ordering::Acquire) {
            for peer in peers {
                while let Some(message) = peer.take_sent() {
                    match message[0] {
                        DIST_CONTINUITY_PREPARE => {
                            let (id, record) = decode_continuity_prepare_payload(&message).unwrap();
                            if record.request_key.contains("superseded") {
                                let _ = registry.mark_completed(
                                    &record.request_key,
                                    "attempt-1",
                                    "someone@h:1",
                                );
                            }
                            let result = if record.request_key.contains("unprepared")
                                || record.request_key.contains("superseded")
                            {
                                Err("replica_full".to_string())
                            } else {
                                Ok(())
                            };
                            peer.receive(encode_continuity_prepare_ack(id, &result));
                        }
                        DIST_SPAWN => peer.receive(frame(
                            DIST_SPAWN_REPLY,
                            &[&message[1..9], &[0], &1u64.to_le_bytes()],
                        )),
                        DIST_HTTP_RESERVE => {
                            let (correlation, _, _, key) = decode_http_reserve(&message).unwrap();
                            let result = if key.contains("turned-away") {
                                Err("owner_reservation_rejected:Draining".to_string())
                            } else {
                                Ok(())
                            };
                            peer.receive(encode_http_reserve_reply(correlation, result).unwrap());
                        }
                        DIST_HTTP_ROUTE_V2_QUERY => {
                            let (correlation, _, key, ..) =
                                decode_http_route_v2_query_frame(&message).unwrap();
                            let result = if key.contains("failing") {
                                Err("handler_failed".to_string())
                            } else {
                                Ok(b"200 drained".to_vec())
                            };
                            peer.receive(
                                encode_http_route_v2_reply_frame(correlation, result).unwrap(),
                            );
                        }
                        _ => {}
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Moves `node`'s continuity responsibilities elsewhere while `peers`
    /// play live nodes, and they go on playing until `settled` (what the
    /// drain started in the background has finished).
    fn drain_with(
        peers: &[&TestPeer],
        node: &str,
        settled: impl Fn() -> bool,
    ) -> Result<DrainContinuityOutcome, String> {
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| serve_as_nodes(peers, &done));
            let outcome = prepare_continuity_for_runtime_node(node);
            let deadline = Instant::now() + Duration::from_secs(20);
            while !settled() {
                assert!(
                    Instant::now() < deadline,
                    "the drain of {node} never settled"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            done.store(true, Ordering::Release);
            outcome
        })
    }

    fn at_once() -> bool {
        true
    }

    fn record_phase(key: &str) -> Option<crate::dist::continuity::ContinuityPhase> {
        crate::dist::continuity::continuity_registry()
            .record(key)
            .map(|record| record.phase)
    }

    /// Waits until `condition` holds of the registry's record `key`.
    fn await_record(key: &str, condition: impl Fn(&ContinuityRecord) -> bool) -> ContinuityRecord {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(record) = crate::dist::continuity::continuity_registry()
                .record(key)
                .filter(|record| condition(record))
            {
                return record;
            }
            assert!(Instant::now() < deadline, "the record {key} never changed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    extern "C" fn drained_work_handler(_args: *const u8) {}

    /// Draining a node moves the work it owns to a worker that has its
    /// handler, preferring one that already mirrors it, replaces it where it
    /// was a replica, and prepares each new replica before the record
    /// changes; the moved work starts again on its new owner. A drain that
    /// cannot place the work fails before touching it.
    #[test]
    fn draining_a_node_moves_its_work_and_replaces_it_as_a_replica() {
        let handler = "Drain.handle";
        let worker_a = TestPeer::new("drain-worker-a@127.0.0.1:1");
        let worker_b = TestPeer::new("drain-worker-b@127.0.0.1:1");
        mesh_register_declared_handler(
            handler.as_ptr(),
            handler.len() as u64,
            "Drain__handle".as_ptr(),
            13,
            2,
            drained_work_handler as *const u8,
        );
        for worker in [&worker_a, &worker_b] {
            report_worker(&worker.session.remote_name, &[handler]);
        }
        let (a, b) = (
            worker_a.session.remote_name.as_str(),
            worker_b.session.remote_name.as_str(),
        );
        let peers = [&worker_a, &worker_b];

        merge_pending_record("drained-owned-key", "drained-owner@h:1", a, handler);
        let outcome = drain_with(&peers, "drained-owner@h:1", at_once).unwrap();
        assert_eq!(
            (outcome.ownership_transfers, outcome.records_examined),
            (1, 1)
        );
        let moved = await_record("drained-owned-key", |record| record.owner_node == a);
        assert_eq!(moved.replica_nodes(), [b.to_string()]);
        assert_eq!(moved.acknowledged_replica_nodes(), [b.to_string()]);
        assert_ne!(moved.attempt_id, "attempt-1");

        merge_pending_record("drained-replica-key", a, "drained-replica@h:1", handler);
        let outcome = drain_with(&peers, "drained-replica@h:1", at_once).unwrap();
        assert_eq!(outcome.replica_replacements, 1);
        let replaced = await_record("drained-replica-key", |record| {
            record.replica_nodes() == [b.to_string()]
        });
        assert_eq!(replaced.owner_node, a);

        merge_pending_record("drained-http-key", "drained-http@h:1", a, handler);
        let answered = || {
            crate::dist::continuity_store::replay_runtime_response("drained-http-key")
                == Ok(Some(b"200 drained".to_vec()))
        };
        drain_with(&peers, "drained-http@h:1", answered).unwrap();

        merge_pending_record(
            "drained-http-failing-key",
            "drained-failing@h:1",
            a,
            handler,
        );
        let rejected = || {
            record_phase("drained-http-failing-key")
                == Some(crate::dist::continuity::ContinuityPhase::Rejected)
        };
        drain_with(&peers, "drained-failing@h:1", rejected).unwrap();

        merge_pending_record("drained-unprepared-key", "drained-prepare@h:1", a, handler);
        assert_eq!(
            drain_with(&peers, "drained-prepare@h:1", at_once),
            Err("replica_full".to_string())
        );
        merge_pending_record("drained-superseded-key", "drained-raced@h:1", a, handler);
        assert_eq!(
            drain_with(&peers, "drained-raced@h:1", at_once)
                .map(|outcome| outcome.ownership_transfers),
            Ok(0)
        );

        merge_pending_record("drained-bare-key", "drained-bare@h:1", a, "");
        assert_eq!(
            drain_with(&peers, "drained-bare@h:1", at_once),
            Err("continuity_drain_untransferable_active_record:drained-bare-key".to_string())
        );
        merge_pending_record(
            "drained-orphan-key",
            "drained-orphan@h:1",
            a,
            "Nobody.handle",
        );
        assert_eq!(
            drain_with(&peers, "drained-orphan@h:1", at_once),
            Err("continuity_drain_owner_transfer_unavailable".to_string())
        );
        let mut wide = continuity_record("drained-wide-key", "drained-wide@h:1", a);
        wide.replication_count = 4;
        wide.declared_handler_runtime_name = handler.to_string();
        crate::dist::continuity::continuity_registry()
            .merge_remote_record(1, wide)
            .unwrap();
        assert!(drain_with(&peers, "drained-wide@h:1", at_once)
            .unwrap_err()
            .starts_with("continuity_drain_replica_capacity_unavailable"));
        assert_eq!(
            prepare_continuity_for_drain("nobody-to-drain"),
            Err("runtime_node_not_found:nobody-to-drain".to_string())
        );
    }

    /// A session reports its health, age, circuit and each lane's use, and
    /// the node every session's. Three transport failures open a peer's
    /// circuit, which then refuses application frames until it half-opens.
    #[test]
    fn a_session_reports_its_lanes_and_its_circuit() {
        let name = "telemetry-peer@127.0.0.1:1";
        let peer = TestPeer::new(name);
        peer.session
            .send(OutboundClass::Application, vec![DIST_SEND; 100])
            .unwrap();
        let now = Instant::now();
        let snapshot = peer.session.telemetry_snapshot(now);
        assert_eq!(snapshot.peer, name);
        assert!(snapshot.healthy);
        assert_eq!(snapshot.circuit_state, "closed");
        let application = snapshot
            .lanes
            .iter()
            .find(|lane| lane.class == "application")
            .unwrap();
        assert_eq!(
            (application.queued_items, application.queued_bytes),
            (1, 100)
        );
        assert!(local_peer_session_telemetry()
            .iter()
            .any(|session| session.peer == name));

        for _ in 0..3 {
            record_peer_transport_failure(name, now);
        }
        assert_eq!(peer.session.telemetry_snapshot(now).circuit_state, "open");
        assert_eq!(
            peer.session
                .send(OutboundClass::Application, vec![DIST_SEND]),
            Err("peer_circuit_open".to_string())
        );
        refresh_peer_session_telemetry();
        assert_eq!(
            peer.session
                .telemetry_snapshot(now + Duration::from_secs(6))
                .circuit_state,
            "half_open"
        );
        record_peer_transport_success(name);
        assert!(!peer_circuit_open(name, Instant::now()));
    }

    /// A peer gets as many retries as its budget allows, each counted.
    #[test]
    fn a_peer_retry_budget_runs_out() {
        let name = "retrying-peer@127.0.0.1:1";
        let now = Instant::now();
        assert!(allow_peer_retry(name, now));
        let mut retries = 1;
        while allow_peer_retry(name, now) {
            retries += 1;
            assert!(retries < 1_000, "the budget never ran out");
        }
    }

    /// A heartbeat frame is a ping or a pong; nothing else goes that way.
    #[test]
    fn only_pings_and_pongs_go_as_heartbeats() {
        let (session, _peer) = loose_session(protocol_one());
        assert_eq!(
            session.send_heartbeat(vec![DIST_SEND; 9]),
            Err("heartbeat_frame_invalid".to_string())
        );
        assert_eq!(session.stream.lock().flush_queued().unwrap(), (0, true));
    }

    /// A bulk send waits for room in its lane rather than failing.
    #[test]
    fn a_bulk_send_waits_for_room_in_its_lane() {
        let (session, _peer) = loose_session(protocol_one());
        for _ in 0..SNAPSHOT_QUEUE_ITEMS {
            session
                .send(OutboundClass::Snapshot, vec![DIST_GLOBAL_SYNC])
                .unwrap();
        }
        let waiting = std::thread::spawn({
            let session = Arc::clone(&session);
            move || session.send_waiting(OutboundClass::Snapshot, vec![DIST_GLOBAL_SYNC])
        });
        std::thread::sleep(Duration::from_millis(20));
        let receivers = session.outbound_receivers.lock().unwrap();
        let frame = receivers.as_ref().unwrap().snapshot.recv().unwrap();
        release_outbound_frame_bytes(&session, &frame);
        drop(receivers);
        assert_eq!(waiting.join().unwrap(), Ok(()));
    }

    /// A write that the peer takes nothing of ends when its session shuts
    /// down, or once the peer has taken nothing for `SESSION_WRITE_STALL`.
    #[test]
    fn a_write_the_peer_takes_nothing_of_ends() {
        for shut_down in [true, false] {
            let (session, _peer) = loose_session(protocol_one());
            let writing = std::thread::spawn({
                let session = Arc::clone(&session);
                move || loop {
                    if let Err(error) = session.write_frames([&[HEARTBEAT_PONG; 64 * 1024][..]]) {
                        return error.kind();
                    }
                }
            });
            if shut_down {
                std::thread::sleep(Duration::from_millis(100));
                session.shutdown.store(true, Ordering::SeqCst);
            }
            let kind = writing.join().unwrap();
            let expected = if shut_down {
                io::ErrorKind::BrokenPipe
            } else {
                io::ErrorKind::TimedOut
            };
            assert_eq!(kind, expected);
        }
    }

    /// A transient session writes its one frame straight to the stream.
    #[test]
    fn a_transient_session_writes_straight_to_its_stream() {
        let (client, mut server) = tls_pair();
        let session = NodeSession::new(
            RemoteSessionEndpoint {
                remote_name: "transient-peer@127.0.0.1:1".to_string(),
                remote_creation: 1,
                node_id: 0,
                direction: SessionDirection::Outgoing,
            },
            NodeStream::ClientTls(client),
            false,
            protocol_one(),
            None,
        );
        session
            .send(OutboundClass::Control, vec![DIST_OPERATOR_REPLY, 1])
            .unwrap();
        server
            .sock
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(read_msg(&mut server).unwrap(), vec![DIST_OPERATOR_REPLY, 1]);
    }

    /// Pids cross between nodes as each node addresses them: a peer's pid
    /// for one of its own processes is qualified here, one it qualified
    /// already is kept, and a pid on a node this one does not know is none.
    #[test]
    fn pids_are_addressed_as_each_node_knows_them() {
        let state = test_node();
        let peer = TestPeer::new("pid-peer@127.0.0.1:1");
        assert_eq!(peer.session.peer_pid(5), peer.pid(5));
        assert_eq!(peer.session.peer_pid(peer.pid(5).as_u64()), peer.pid(5));
        assert_eq!(
            pid_node_name(peer.pid(5)),
            Some(peer.session.remote_name.clone())
        );
        assert_eq!(pid_node_name(ProcessId(5)), Some(state.name.clone()));
        assert_eq!(pid_node_name(ProcessId(0)), None);
        assert_eq!(pid_on_node("", 5), 0);
        assert_eq!(pid_on_node(&state.name, 5), 5);
        assert_eq!(
            pid_on_node(&peer.session.remote_name, 5),
            peer.pid(5).as_u64()
        );
        assert_eq!(pid_on_node("unknown-node@127.0.0.1:1", 5), 0);

        let local = ProcessId(7);
        send_dist_link(local, peer.pid(3));
        send_dist_monitor_exit_by_pid(local, peer.pid(3), 11, &ExitReason::Normal);
        send_dist_link(local, ProcessId::from_remote(u16::MAX, 1, 3));
        let sent = peer.sent();
        assert_eq!(
            sent.iter().map(|frame| frame[0]).collect::<Vec<_>>(),
            vec![DIST_MONITOR_EXIT, DIST_LINK]
        );
    }

    /// The newest diagnostic of `transition` for the request `key`.
    fn diagnosed(
        transition: &str,
        key: &str,
    ) -> Option<crate::dist::operator::OperatorDiagnosticEntry> {
        let fingerprint = crate::dist::continuity::request_key_fingerprint(key);
        crate::dist::operator::operator_recent_diagnostics(None)
            .entries
            .into_iter()
            .rev()
            .find(|entry| {
                entry.transition == transition
                    && entry.request_key.as_deref() == Some(fingerprint.as_str())
            })
    }

    /// Waits for the diagnostic of `transition` for the request `key`.
    fn await_diagnostic(
        transition: &str,
        key: &str,
    ) -> crate::dist::operator::OperatorDiagnosticEntry {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(entry) = diagnosed(transition, key) {
                return entry;
            }
            assert!(Instant::now() < deadline, "no {transition} for {key}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    extern "C" fn resumed_work_handler(_args: *const u8) {}

    /// A standby whose primary goes promotes itself when no other peer
    /// remains and every pending record is one it mirrored from that
    /// primary, then submits that work again under a new attempt; a record
    /// without a handler to resume is said to be refused. While a peer
    /// remains it stays a standby.
    #[test]
    fn a_standby_promotes_itself_when_its_primary_goes_and_resumes_its_work() {
        use crate::dist::continuity::{ContinuityClusterRole, ReplicaStatus};
        let exclusive = declared_handler_registry_test_lock();
        let state = test_node();
        let registry = crate::dist::continuity::continuity_registry();
        registry.clear_for_test();
        registry.make_standby_for_test();
        let handler = "Resumed.work";
        mesh_register_declared_handler(
            handler.as_ptr(),
            handler.len() as u64,
            "Resumed__work".as_ptr(),
            13,
            1,
            resumed_work_handler as *const u8,
        );

        let primary = TestPeer::within(&exclusive, "lost-primary@127.0.0.1:1");
        let lost = primary.session.remote_name.clone();
        for (key, runtime_name) in [("resumed-key", handler), ("unresumable-key", "")] {
            let mut record = continuity_record(key, &lost, &state.name);
            record.cluster_role = ContinuityClusterRole::Standby;
            record.replica_status = ReplicaStatus::Mirrored;
            record.declared_handler_runtime_name = runtime_name.to_string();
            registry.mirror_prepare(record).unwrap();
        }

        let remaining = TestPeer::within(&exclusive, "remaining-peer@127.0.0.1:1");
        maybe_automatic_promote_and_resume(&lost);
        assert_eq!(
            registry.authority_status().cluster_role,
            ContinuityClusterRole::Standby
        );
        drop(remaining);

        drop(primary);
        let authority = registry.authority_status();
        assert_eq!(
            (authority.cluster_role, authority.promotion_epoch),
            (ContinuityClusterRole::Primary, 1)
        );
        let resumed = await_record("resumed-key", |record| record.attempt_id != "attempt-1");
        assert_eq!(
            await_diagnostic("automatic_recovery_rejected", "unresumable-key").reason,
            Some(AUTOMATIC_RECOVERY_REJECTED_HANDLER_MISSING.to_string())
        );

        // Submitting the resumed work once more finds it already there, and
        // a handler this node lacks cannot be submitted at all.
        spawn_automatic_recovery_submission(
            handler,
            "resumed-key",
            "sha256:payload",
            &resumed.attempt_id,
        );
        spawn_automatic_recovery_submission("Absent.work", "absent-key", "sha256:x", "attempt-9");
        await_diagnostic("automatic_recovery_rejected", "absent-key");
        let deadline = Instant::now() + Duration::from_secs(10);
        while diagnosed("automatic_recovery_rejected", "resumed-key").is_none() {
            assert!(Instant::now() < deadline, "the repeat was not refused");
            std::thread::sleep(Duration::from_millis(5));
        }

        registry.clear_for_test();
        clear_declared_handler_registry_for_test();
    }

    /// A declared handler for startup work, which ends its attempt as its
    /// runtime name says: completed, rejected, fenced by a newer attempt,
    /// or forgotten (the record gone).
    extern "C" fn startup_outcome_handler(args: *const u8) {
        let words = unsafe { std::slice::from_raw_parts(args as *const u64, 2) };
        let (key, attempt) = (
            mesh_string_arg_to_owned(words[0]),
            mesh_string_arg_to_owned(words[1]),
        );
        let registry = crate::dist::continuity::continuity_registry();
        if key.ends_with(".complete") {
            complete_declared_work(&key, &attempt).unwrap();
        } else if key.ends_with(".reject") {
            registry
                .reject_durable_request(&key, &attempt, "startup_handler_failed")
                .unwrap();
        } else if key.ends_with(".fence") {
            let mut newer = registry.record(&key).unwrap();
            newer.attempt_id = "attempt-999".to_string();
            newer.record_version += 1;
            registry.merge_remote_record(1_000, newer).unwrap();
        } else {
            registry.clear_for_test();
        }
    }

    /// Startup work waits for the cluster to settle, submits itself, and
    /// waits for its attempt to end, saying how it ended: completed,
    /// rejected, fenced by a newer attempt, or lost. Work with no handler,
    /// or no name, is refused at once.
    #[test]
    fn startup_work_runs_once_the_cluster_settles_and_says_how_it_ended() {
        let _exclusive = declared_handler_registry_test_lock();
        test_node();
        let registry = crate::dist::continuity::continuity_registry();
        registry.clear_for_test();
        let outcomes = ["complete", "reject", "fence"];
        for outcome in outcomes {
            let name = format!("Startup.{outcome}");
            mesh_register_declared_handler(
                name.as_ptr(),
                name.len() as u64,
                name.as_ptr(),
                name.len() as u64,
                1,
                startup_outcome_handler as *const u8,
            );
            spawn_startup_work_actor(&name);
        }
        spawn_startup_work_actor("Startup.absent");
        let words = [crate::string::mesh_str(" ") as u64];
        crate::actor::global_scheduler().spawn(
            startup_work_entry as *const u8,
            words.as_ptr() as *const u8,
            8,
            1,
        );

        for (outcome, transition) in [
            ("complete", "startup_completed"),
            ("reject", "startup_rejected"),
            ("fence", "startup_fenced"),
            ("absent", "startup_rejected"),
        ] {
            await_diagnostic(
                transition,
                &startup_request_key(&format!("Startup.{outcome}")),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !crate::dist::operator::operator_recent_diagnostics(None)
            .entries
            .iter()
            .any(|entry| {
                entry.transition == "startup_rejected"
                    && entry.reason.as_deref() == Some(STARTUP_RUNTIME_NAME_MISSING)
            })
        {
            assert!(
                Instant::now() < deadline,
                "the unnamed work was not refused"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        let forgotten = "Startup.forget";
        mesh_register_declared_handler(
            forgotten.as_ptr(),
            forgotten.len() as u64,
            forgotten.as_ptr(),
            forgotten.len() as u64,
            1,
            startup_outcome_handler as *const u8,
        );
        spawn_startup_work_actor(forgotten);
        assert_eq!(
            await_diagnostic("startup_rejected", &startup_request_key(forgotten)).reason,
            Some("request_key_not_found".to_string())
        );
        registry.clear_for_test();
        clear_declared_handler_registry_for_test();
    }

    /// Answers each replica prepare `peers` receive until `done`: refused
    /// by a replica whose name the record's key names after `refused-by-`
    /// the first `refusals` times, acknowledged otherwise.
    fn answer_prepares(peers: &[&TestPeer], refusals: usize, done: &AtomicBool) {
        let mut refused = 0;
        while !done.load(Ordering::Acquire) {
            for peer in peers {
                while let Some(message) = peer.take_sent() {
                    if message[0] != DIST_CONTINUITY_PREPARE {
                        continue;
                    }
                    let (id, record) = decode_continuity_prepare_payload(&message).unwrap();
                    let refuses = record
                        .request_key
                        .contains(&format!("refused-by-{}", &record.replica_node[..9]));
                    let result = if refuses && refused < refusals {
                        refused += 1;
                        Err("replica_full".to_string())
                    } else {
                        Ok(())
                    };
                    peer.receive(encode_continuity_prepare_ack(id, &result));
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A record is prepared on each of its replicas before it is admitted:
    /// it goes on when enough of them take it (half its copies, rounded
    /// down), the rest repaired in the background, and fails when too few
    /// do, or when its replica set is not the size its copies need.
    #[test]
    fn a_record_is_prepared_on_its_replicas_before_it_is_admitted() {
        let first = TestPeer::new("replica-1@127.0.0.1:1");
        let second = TestPeer::new("replica-2@127.0.0.1:1");
        let (one, two) = (
            first.session.remote_name.clone(),
            second.session.remote_name.clone(),
        );
        let record = |key: &str, replicas: &[&str], copies: u64| {
            let mut record = continuity_record(key, "prepared-owner@h:1", replicas[0]);
            record.replica_nodes = replicas.iter().map(|node| node.to_string()).collect();
            record.replication_count = copies;
            record
        };
        let prepare = |record: &ContinuityRecord, refusals: usize| {
            let done = AtomicBool::new(false);
            std::thread::scope(|scope| {
                scope.spawn(|| answer_prepares(&[&first, &second], refusals, &done));
                let prepared = prepare_continuity_replica(record);
                done.store(true, Ordering::Release);
                prepared
            })
        };

        assert_eq!(
            prepare(&record("prepared-by-both", &[&one, &two], 3), 0),
            Ok(vec![one.clone(), two.clone()])
        );
        let mut alone = record("prepared-alone", &[&one], 1);
        alone.replica_nodes.clear();
        alone.replica_node.clear();
        assert_eq!(prepare(&alone, 0), Ok(Vec::new()));
        assert_eq!(
            prepare(&record("prepared-short", &[&one], 3), 0),
            Err("continuity_replica_set_size_mismatch:required=2:recorded=1".to_string())
        );
        assert!(prepare(
            &record(
                "refused-by-replica-1-refused-by-replica-2",
                &[&one, &two],
                3
            ),
            2
        )
        .unwrap_err()
        .starts_with("continuity_replica_ack_threshold_unmet:required=1:acknowledged=0"));

        // One of two refuses at first: the record goes on, and the repair
        // prepares the refusing replica and records its acknowledgement.
        let repaired = record("refused-by-replica-2", &[&one, &two], 3);
        crate::dist::continuity::continuity_registry()
            .merge_remote_record(1, repaired.clone())
            .unwrap();
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| answer_prepares(&[&first, &second], 1, &done));
            assert_eq!(prepare_continuity_replica(&repaired), Ok(vec![one.clone()]));
            await_record("refused-by-replica-2", |record| {
                record.acknowledged_replica_nodes().contains(&two)
            });
            done.store(true, Ordering::Release);
        });
    }

    /// A replica prepare fails without a session to the replica, or one
    /// that cannot send, and times out when the replica never answers; a
    /// replica that is this node takes the record at once.
    #[test]
    fn a_replica_prepare_fails_without_a_live_replica() {
        let state = test_node();
        let silent = TestPeer::new("silent-replica@127.0.0.1:1");
        let closed = TestPeer::new("closed-replica@127.0.0.1:1");
        closed.session.shutdown.store(true, Ordering::SeqCst);
        let prepare = |replica: &str| {
            prepare_one_continuity_replica(&continuity_record(
                "prepared-nowhere",
                "prepared-owner@h:1",
                replica,
            ))
        };
        assert_eq!(prepare(&state.name), Ok(()));
        assert_eq!(
            prepare("absent-replica@127.0.0.1:1"),
            Err("replica_required_unavailable".to_string())
        );
        assert_eq!(
            prepare(&closed.session.remote_name),
            Err("replica_required_unavailable".to_string())
        );
        assert_eq!(
            prepare(&silent.session.remote_name),
            Err("replica_prepare_timeout".to_string())
        );
        assert!(silent
            .session
            .pending_continuity_prepares
            .lock()
            .unwrap()
            .is_empty());
    }

    /// The replicas a new record gets: none when it keeps one copy, too few
    /// members fails, and otherwise live members with fresh load reports,
    /// never its owner.
    #[test]
    fn replicas_are_chosen_among_live_reporting_members() {
        let exclusive = declared_handler_registry_test_lock();
        let state = test_node();
        assert_eq!(
            select_continuity_replica_set(&state.name, 1),
            Ok(Vec::new())
        );
        assert!(select_continuity_replica_set(&state.name, 2)
            .unwrap_err()
            .starts_with("replica_capacity_unavailable"));
        let first = TestPeer::within(&exclusive, "0-chosen-replica-a@127.0.0.1:1");
        let second = TestPeer::within(&exclusive, "0-chosen-replica-b@127.0.0.1:1");
        for peer in [&first, &second] {
            report_worker(&peer.session.remote_name, &[]);
        }
        assert_eq!(
            select_continuity_replica_set(&state.name, 3),
            Ok(vec![
                first.session.remote_name.clone(),
                second.session.remote_name.clone()
            ])
        );
        second.session.shutdown.store(true, Ordering::SeqCst);
        assert!(select_continuity_replica_set(&state.name, 3).is_err());
    }

    /// Waits for an `owner_loss_recovery_failed` diagnostic about `node`.
    fn await_recovery_failure(node: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let failure = crate::dist::operator::operator_recent_diagnostics(None)
                .entries
                .into_iter()
                .rev()
                .find(|entry| {
                    entry.transition == "owner_loss_recovery_failed"
                        && entry
                            .metadata
                            .iter()
                            .any(|(key, value)| key == "disconnected_node" && value == node)
                });
            if let Some(failure) = failure {
                return failure.reason.unwrap_or_default();
            }
            assert!(Instant::now() < deadline, "no recovery failure for {node}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The node that coordinates recovery (the controller leader, or else
    /// the first remaining member) recovers the work of a lost owner, and
    /// of a replica no longer a member, once at a time; recovery that
    /// cannot place the work says so. Another node leaves it alone.
    #[test]
    fn the_coordinator_recovers_a_lost_owner_s_work_once_at_a_time() {
        use crate::dist::continuity::ReplicaStatus;
        let exclusive = declared_handler_registry_test_lock();
        let state = test_node();
        let registry = crate::dist::continuity::continuity_registry();
        registry.clear_for_test();
        let mut record = continuity_record("lost-owner-key", "lost-owner@h:1", "lost-replica@h:1");
        record.replica_status = ReplicaStatus::OwnerLost;
        record.declared_handler_runtime_name = "Recovered.work".to_string();
        registry.merge_remote_record(1, record).unwrap();

        assert!(local_coordinates_node_loss_recovery("lost-owner@h:1"));
        recover_pending_owner_losses_if_coordinator();
        assert_eq!(
            await_recovery_failure("lost-owner@h:1"),
            "continuity_drain_owner_transfer_unavailable"
        );
        await_recovery_failure("lost-replica@h:1");

        active_owner_loss_recoveries()
            .lock()
            .unwrap()
            .insert("held-owner@h:1".to_string());
        maybe_spawn_primary_owner_loss_recovery("held-owner@h:1");
        assert!(active_owner_loss_recoveries()
            .lock()
            .unwrap()
            .remove("held-owner@h:1"));

        // A member that sorts first coordinates instead.
        let first = (0..)
            .map(|index| format!("coordinator-{index}@127.0.0.1:1"))
            .find(|name| stable_hash_u64(name) < stable_hash_u64(&state.name))
            .unwrap();
        let coordinator = TestPeer::within(&exclusive, &first);
        assert!(!local_coordinates_node_loss_recovery("lost-owner@h:1"));
        maybe_spawn_primary_owner_loss_recovery("lost-owner@h:1");
        drop(coordinator);
        registry.clear_for_test();
    }

    extern "C" fn clustered_route_handler(request: *mut u8) -> *mut u8 {
        let body = crate::http::server::mesh_http_request_body(request);
        let body = unsafe {
            (*(body as *const crate::string::MeshString))
                .as_str()
                .to_string()
        };
        crate::http::server::mesh_http_response_new(
            200,
            crate::string::mesh_str(&format!("handled:{body}")),
        )
    }

    /// An encoded HTTP request, as a clustered route carries it.
    fn route_payload(method: &str, body: &str) -> Vec<u8> {
        use crate::collections::map;
        use crate::http::server::MeshHttpRequest;
        unsafe {
            let request = crate::gc::mesh_gc_alloc_actor(
                std::mem::size_of::<MeshHttpRequest>() as u64,
                std::mem::align_of::<MeshHttpRequest>() as u64,
            ) as *mut MeshHttpRequest;
            (*request).method = crate::string::mesh_str(method) as *mut u8;
            (*request).path = crate::string::mesh_str("/routed") as *mut u8;
            (*request).body = crate::string::mesh_str(body) as *mut u8;
            (*request).query_params = map::mesh_map_new_typed(1);
            (*request).headers = map::mesh_map_new_typed(1);
            (*request).path_params = map::mesh_map_new_typed(1);
            crate::http::server::encode_http_request_payload(request as *mut u8).unwrap()
        }
    }

    /// A request key whose clustered request the current members place on
    /// `owner`.
    fn key_owned_by(owner: &str, prefix: &str) -> String {
        let membership = canonical_declared_membership();
        (0..)
            .map(|index| format!("{prefix}-{index}"))
            .find(|key| {
                let index = stable_hash_u64(&format!("request::{key}")) as usize % membership.len();
                membership[index] == owner
            })
            .unwrap()
    }

    fn response_body(payload: &[u8]) -> String {
        let response = crate::http::server::decode_http_response_payload(payload).unwrap();
        let response = unsafe { &*(response as *const crate::http::server::MeshHttpResponse) };
        unsafe {
            (*(response.body as *const crate::string::MeshString))
                .as_str()
                .to_string()
        }
    }

    /// A clustered HTTP request runs on the member that owns its key, here
    /// or on a peer, and its response is kept: the same request again gets
    /// it back without running, one still running or whose response was
    /// not kept is refused, and one whose owner fails it is rejected.
    #[test]
    fn a_clustered_http_request_runs_on_its_owner_and_replays_once_done() {
        use crate::dist::continuity::{ContinuityPhase, ContinuityResult};
        let exclusive = declared_handler_registry_test_lock();
        let state = test_node();
        let registry = crate::dist::continuity::continuity_registry();
        registry.clear_for_test();
        let handler = "Routed.handle";
        mesh_register_declared_handler(
            handler.as_ptr(),
            handler.len() as u64,
            handler.as_ptr(),
            handler.len() as u64,
            1,
            clustered_route_handler as *const u8,
        );
        let run = |key: &str, hash: &str| {
            execute_clustered_http_route(handler, key, hash, &route_payload("GET", key))
        };

        let local = key_owned_by(&state.name, "local-route");
        let first = run(&local, "sha256:local").unwrap();
        assert!(!first.replayed && !first.routed_remotely);
        assert_eq!(
            response_body(&first.response_payload),
            format!("handled:{local}")
        );
        let replayed = run(&local, "sha256:local").unwrap();
        assert!(replayed.replayed);
        assert_eq!(replayed.response_payload, first.response_payload);
        assert!(run(&local, "sha256:another-payload").is_err());

        let mut running = continuity_record("route-running", &state.name, "unused@h:1");
        running.replica_nodes.clear();
        running.replica_node.clear();
        running.replication_count = 1;
        running.payload_hash = "sha256:running".to_string();
        registry.merge_remote_record(1, running.clone()).unwrap();
        assert_eq!(
            run("route-running", "sha256:running").err(),
            Some("idempotent_operation_in_progress".to_string())
        );
        let mut forgotten = running;
        forgotten.request_key = "route-forgotten".to_string();
        forgotten.phase = ContinuityPhase::Completed;
        forgotten.result = ContinuityResult::Succeeded;
        forgotten.execution_node = state.name.clone();
        registry.merge_remote_record(1, forgotten).unwrap();
        assert_eq!(
            run("route-forgotten", "sha256:running").err(),
            Some("idempotent_response_not_retained".to_string())
        );

        let owner = TestPeer::within(&exclusive, "route-owner@127.0.0.1:1");
        let (remote, failing) = (
            key_owned_by(&owner.session.remote_name, "remote-route"),
            key_owned_by(&owner.session.remote_name, "remote-failing-route"),
        );
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| serve_as_nodes(&[&owner], &done));
            let routed = run(&remote, "sha256:remote").unwrap();
            assert!(routed.routed_remotely && !routed.replayed);
            assert_eq!(routed.response_payload, b"200 drained");
            assert_eq!(
                run(&failing, "sha256:failing").err(),
                Some("handler_failed".to_string())
            );
            done.store(true, Ordering::Release);
        });
        assert_eq!(
            registry.record(&failing).map(|record| record.phase),
            Some(ContinuityPhase::Rejected)
        );
        drop(owner);
        registry.clear_for_test();
        clear_declared_handler_registry_for_test();
    }

    /// When a routed request's owner turns it away, as a draining owner
    /// does, a request safe to replay is recovered: its owner is marked
    /// lost, this node (the coordinator, and the record's replica) takes it
    /// over with a new replica, runs it, and returns the response kept.
    #[test]
    fn a_replay_safe_request_its_owner_turns_away_is_recovered_here() {
        let exclusive = declared_handler_registry_test_lock();
        let state = test_node();
        let registry = crate::dist::continuity::continuity_registry();
        registry.clear_for_test();
        let handler = "Recovered.route";
        mesh_register_declared_handler(
            handler.as_ptr(),
            handler.len() as u64,
            handler.as_ptr(),
            handler.len() as u64,
            2,
            clustered_route_handler as *const u8,
        );
        let owner = TestPeer::within(&exclusive, "draining-route-owner@127.0.0.1:1");
        // The spare replica sorts after this node both as a member (so this
        // node coordinates) and by name (so this node is the first replica).
        let spare = (0..)
            .map(|index| format!("zz-route-spare-{index}@127.0.0.1:1"))
            .find(|name| stable_hash_u64(name) > stable_hash_u64(&state.name))
            .unwrap();
        let spare = TestPeer::within(&exclusive, &spare);
        let key = key_owned_by(&owner.session.remote_name, "turned-away-route");
        // Both stay ready workers with the handler, as their heartbeats
        // would keep them.
        let report = || {
            report_worker(&state.name, &[handler]);
            report_worker(&spare.session.remote_name, &[handler]);
        };
        report();
        let done = AtomicBool::new(false);
        let recovered = std::thread::scope(|scope| {
            scope.spawn(|| serve_as_nodes(&[&owner, &spare], &done));
            scope.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    report();
                    std::thread::sleep(Duration::from_millis(200));
                }
            });
            let recovered = execute_clustered_http_route(
                handler,
                &key,
                "sha256:recovered",
                &route_payload("GET", &key),
            );
            done.store(true, Ordering::Release);
            recovered
        })
        .unwrap();
        assert_eq!(
            response_body(&recovered.response_payload),
            format!("handled:{key}")
        );
        let record = registry.record(&key).unwrap();
        assert_eq!(
            (record.owner_node, record.replica_nodes),
            (state.name.clone(), vec![spare.session.remote_name.clone()])
        );
        drop((owner, spare));
        registry.clear_for_test();
        clear_declared_handler_registry_for_test();
    }

    /// Whether the recovery of a failed routed attempt can be seen yet: the
    /// record completed or succeeded, marked owner-lost, or moved on to
    /// another attempt or owner; never once rejected.
    #[test]
    fn recovery_of_a_failed_attempt_is_observable_once_the_record_moves_on() {
        use crate::dist::continuity::{ContinuityPhase, ContinuityResult, ReplicaStatus};
        let _peers = TEST_PEERS.read_recursive();
        let registry = crate::dist::continuity::continuity_registry();
        let observable = |record: &ContinuityRecord| {
            registry.merge_remote_record(1, record.clone()).unwrap();
            continuity_recovery_is_observable(
                &record.request_key,
                "attempt-1",
                "observed-owner@h:1",
            )
        };
        assert!(!continuity_recovery_is_observable(
            "never-recorded",
            "attempt-1",
            "o@h:1"
        ));
        let pending = continuity_record("observed-pending", "observed-owner@h:1", "r@h:1");
        assert!(!observable(&pending));
        let mut lost = pending.clone();
        lost.request_key = "observed-lost".to_string();
        lost.replica_status = ReplicaStatus::OwnerLost;
        assert!(observable(&lost));
        let mut moved = pending.clone();
        moved.request_key = "observed-moved".to_string();
        moved.owner_node = "new-owner@h:1".to_string();
        assert!(observable(&moved));
        let mut rejected = pending;
        rejected.request_key = "observed-rejected".to_string();
        rejected.phase = ContinuityPhase::Rejected;
        rejected.result = ContinuityResult::Rejected;
        assert!(!observable(&rejected));
    }

    /// A verified identity takes only the channel its roles allow: a
    /// controller only under the voter name bound to its stable id (by its
    /// advertised name when it comes in as a transient operator client), no
    /// other role under a voter's name, and an operator only as a transient
    /// operator client, which a worker cannot be.
    #[test]
    fn an_identity_takes_only_the_channel_its_roles_allow() {
        let claim = |stable: &str, advertised: &str, role: &str| {
            super::super::identity_claim::NodeIdentityClaim {
                schema_version: 1,
                cluster_id: "cluster".to_string(),
                stable_node_id: stable.to_string(),
                advertised_name: advertised.to_string(),
                roles: vec![role.to_string()],
                issued_at_unix_millis: 0,
                expires_at_unix_millis: u64::MAX,
            }
        };
        let client = format!("{TRANSIENT_OPERATOR_CLIENT_NAME_PART}@127.0.0.1:1");
        let authorize = |remote: &str, stable: &str, advertised: &str, role: &str| {
            authorize_node_identity(
                remote,
                &claim(stable, advertised, role),
                "ctl-1|ctl@h:1, ctl-2|ctl-2@h:1",
            )
            .err()
        };
        let refused = |reason: &str| Some(reason.to_string());
        assert_eq!(authorize("ctl@h:1", "ctl-1", "ctl@h:1", "controller"), None);
        assert_eq!(authorize(&client, "ctl-1", "ctl@h:1", "controller"), None);
        assert_eq!(
            authorize("ctl@h:1", "ctl-2", "ctl@h:1", "controller"),
            refused("controller_identity_not_bound_to_voter")
        );
        assert_eq!(
            authorize("ctl@h:1", "w-1", "ctl@h:1", "worker"),
            refused("non_controller_claimed_voter_name")
        );
        assert_eq!(authorize("w@h:1", "w-1", "w@h:1", "worker"), None);
        assert_eq!(authorize(&client, "o-1", "*", "operator"), None);
        assert_eq!(
            authorize("w@h:1", "o-1", "*", "operator"),
            refused("operator_identity_channel_mismatch")
        );
        assert_eq!(
            authorize(&client, "w-1", "*", "worker"),
            refused("operator_identity_channel_mismatch")
        );
    }

    /// A node's hello carries the signed identity its settings give it,
    /// which must name this node, with the stable id and roles it runs
    /// with; part of those settings, or none in autonomous mode, is
    /// refused. A peer's identity is verified against the cluster and its
    /// keys, and a peer without one is taken only outside autonomous mode.
    #[test]
    fn a_node_s_identity_is_signed_carried_and_verified() {
        use super::super::identity_claim::{
            self as identity, NodeIdentityClaim, IDENTITY_ENVELOPE_ENV, IDENTITY_VERIFY_KEYS_ENV,
        };
        fn settings(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
            let pairs: std::collections::HashMap<String, String> = pairs
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect();
            move |name| pairs.get(name).cloned()
        }
        let (signing_key, verify_key) = identity::generate_identity_signing_material().unwrap();
        let now = identity::unix_millis();
        let claim = NodeIdentityClaim {
            schema_version: identity::IDENTITY_SCHEMA_VERSION,
            cluster_id: "cluster".to_string(),
            stable_node_id: "cluster/worker-1".to_string(),
            advertised_name: "worker@h:1".to_string(),
            roles: vec!["worker".to_string()],
            issued_at_unix_millis: now,
            expires_at_unix_millis: now + 60_000,
        };
        let envelope = identity::sign_identity_claim(&claim, &signing_key).unwrap();
        let signed = identity::decode_envelope_b64(&envelope).unwrap();
        let verifying = [
            (IDENTITY_VERIFY_KEYS_ENV, verify_key.as_str()),
            ("MESH_CLUSTER_ID", "cluster"),
        ];
        let hello = |name: &str, envelope: &str, extra: &[(&str, &str)]| {
            let pairs = [&verifying[..], &[(IDENTITY_ENVELOPE_ENV, envelope)], extra].concat();
            local_protocol_hello_with_identity(name, settings(&pairs))
                .map(|hello| hello.identity_envelope)
        };
        fn refused<T>(reason: &str) -> Result<T, String> {
            Err(reason.to_string())
        }

        assert_eq!(
            hello("worker@h:1", &envelope, &[("MESH_ROLES", "worker")]),
            Ok(signed.clone())
        );
        assert_eq!(
            hello(
                "worker@h:1",
                &envelope,
                &[
                    ("MESH_ROLES", "Worker"),
                    ("MESH_STABLE_NODE_ID", "cluster/worker-1")
                ]
            ),
            Ok(signed.clone())
        );
        for (name, envelope, roles, stable_id, reason) in [
            (
                "worker@h:1",
                envelope.as_str(),
                "gateway",
                "cluster/worker-1",
                "local_node_identity_claim_mismatch",
            ),
            (
                "worker@h:1",
                envelope.as_str(),
                "worker",
                "cluster/other",
                "local_node_identity_claim_mismatch",
            ),
            (
                "worker@h:1",
                envelope.as_str(),
                "janitor",
                "cluster/worker-1",
                "node_identity_roles_invalid",
            ),
            (
                "other@h:1",
                envelope.as_str(),
                "worker",
                "cluster/worker-1",
                "node_identity_claim_scope_invalid",
            ),
            (
                "worker@h:1",
                "not base64",
                "worker",
                "cluster/worker-1",
                "node_identity_envelope_invalid",
            ),
        ] {
            assert_eq!(
                hello(
                    name,
                    envelope,
                    &[("MESH_ROLES", roles), ("MESH_STABLE_NODE_ID", stable_id)]
                ),
                refused(reason)
            );
        }
        let bare = |pairs: &[(&str, &str)]| {
            local_protocol_hello_with_identity("worker@h:1", settings(pairs))
                .map(|hello| hello.identity_envelope)
        };
        assert_eq!(bare(&[]), Ok(Vec::new()));
        assert_eq!(
            bare(&verifying),
            refused("node_identity_configuration_incomplete")
        );
        assert_eq!(
            in_autonomous_mode(|| bare(&[])),
            refused("autonomous_mode_requires_signed_node_identity")
        );

        let peer = |envelope: &[u8], pairs: &[(&str, &str)]| {
            let mut hello = local_protocol_hello();
            hello.identity_envelope = envelope.to_vec();
            validate_remote_node_identity("worker@h:1", &hello, settings(pairs))
        };
        assert_eq!(peer(&[], &[]), Ok(None));
        assert_eq!(
            in_autonomous_mode(|| peer(&[], &[])),
            refused("autonomous_peer_missing_signed_identity")
        );
        assert_eq!(peer(&signed, &verifying), Ok(Some(claim)));
        assert_eq!(peer(&signed, &[]), refused("node_identity_cluster_missing"));
        assert_eq!(
            peer(&signed, &verifying[1..]),
            refused("node_identity_verify_keys_missing")
        );
        let (_, stranger) = identity::generate_identity_signing_material().unwrap();
        assert_eq!(
            peer(
                &signed,
                &[
                    (IDENTITY_VERIFY_KEYS_ENV, &stranger),
                    ("MESH_CLUSTER_ID", "cluster")
                ]
            ),
            refused("node_identity_signature_invalid")
        );
        assert_eq!(
            peer(
                &signed,
                &[
                    &verifying[..],
                    &[("MESH_CONTROLLER_VOTERS", "cluster/ctl|worker@h:1")]
                ]
                .concat()
            ),
            refused("non_controller_claimed_voter_name")
        );
    }

    /// With adaptive routing on, declared work goes where the members' load
    /// reports say, this node reporting its own load first: work its key
    /// would place on a member too busy to take it stays here. The switch
    /// reads on or off, and anything else leaves it to the manifest, which
    /// a test process lacks.
    #[test]
    fn adaptive_routing_places_work_by_the_members_load() {
        const SWITCH: &str = "MESH_ADAPTIVE_ROUTING";
        let exclusive = declared_handler_registry_test_lock();
        let state = test_node();
        // No handler here, so this node takes any work.
        clear_declared_handler_registry_for_test();
        let busy = TestPeer::within(&exclusive, "busy-member@127.0.0.1:1");
        let mut report = crate::dist::routing::local_load_report(
            &busy.session.remote_name,
            ["Adaptive.work".to_string()].into(),
        );
        report.inflight = crate::dist::routing::runtime_routing_policy().max_inflight;
        crate::dist::routing::load_report_registry()
            .apply(report, Instant::now())
            .unwrap();
        let key = key_owned_by(&busy.session.remote_name, "adaptive");

        let original = std::env::var_os(SWITCH);
        for (value, adaptive) in [("On", true), ("0", false), ("sometimes", false)] {
            std::env::set_var(SWITCH, value);
            assert_eq!(
                crate::dist::routing::runtime_adaptive_routing_enabled(),
                adaptive
            );
        }
        std::env::set_var(SWITCH, "true");
        let placement = declared_work_placement(&key, "Adaptive.work");
        match original {
            Some(value) => std::env::set_var(SWITCH, value),
            None => std::env::remove_var(SWITCH),
        }
        let placement = placement.unwrap();
        assert_eq!(placement.owner_node, state.name);
        assert!(!placement.routed_remotely);
        assert_eq!(
            declared_work_placement(&key, "Adaptive.work")
                .unwrap()
                .owner_node,
            busy.session.remote_name
        );
    }
}
