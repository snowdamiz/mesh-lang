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
//! TLS provides confidentiality and integrity. Authentication is handled by the
//! Autonomous peers require mTLS plus a signed, cluster-scoped identity claim.
//! The HMAC-SHA256 cookie handshake remains a compatibility and
//! defense-in-depth layer, with comma-separated keyrings for rolling rotation.
//! The client-side TLS config intentionally skips certificate verification.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
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
use super::discovery::{parse_host_port, start_from_env as start_discovery_from_env};
use super::protocol::{
    negotiate_protocol, CircuitBreaker, CircuitState, MessageClass, NegotiatedProtocol,
    ProtocolEnvelope, ProtocolHello, RetryBudget, PROTOCOL_V1, PROTOCOL_V2,
};
use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};

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
    /// Signals the listener thread to stop accepting connections
    pub listener_shutdown: AtomicBool,
    /// Processes monitoring specific nodes for :nodedown/:nodeup events.
    /// Maps node_name -> list of (monitoring_pid, is_once) pairs.
    pub node_monitors: RwLock<FxHashMap<String, Vec<(crate::actor::process::ProcessId, bool)>>>,
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

fn local_protocol_hello_with_identity(local_name: &str) -> Result<ProtocolHello, String> {
    let mut hello = local_protocol_hello();
    let autonomous = autonomous_mode_requested();
    let envelope = std::env::var(super::identity_claim::IDENTITY_ENVELOPE_ENV).ok();
    let verify_keys = std::env::var(super::identity_claim::IDENTITY_VERIFY_KEYS_ENV).ok();
    let cluster_id = std::env::var("MESH_CLUSTER_ID").ok();
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
                != std::env::var("MESH_STABLE_NODE_ID")
                    .unwrap_or_else(|_| claim.stable_node_id.clone())
                || claim.roles
                    != super::identity_claim::canonical_roles(
                        &std::env::var("MESH_ROLES")
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

/// Global registry mapping function names to their code pointers.
///
/// Populated at program startup by codegen-emitted `mesh_register_function`
/// calls. Used by the remote spawn handler to look up a function pointer
/// by name when a DIST_SPAWN request arrives from another node.
static FUNCTION_REGISTRY: OnceLock<RwLock<FxHashMap<String, FnPtr>>> = OnceLock::new();

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
fn function_registry() -> &'static RwLock<FxHashMap<String, FnPtr>> {
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
#[no_mangle]
pub extern "C" fn mesh_register_function(name_ptr: *const u8, name_len: u64, fn_ptr: *const u8) {
    if name_ptr.is_null() || fn_ptr.is_null() {
        return;
    }
    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        std::str::from_utf8_unchecked(slice).to_string()
    };
    function_registry().write().insert(name, FnPtr(fn_ptr));
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

/// Look up a registered function by name.
///
/// Returns `Some(fn_ptr)` if the function was registered, `None` otherwise.
pub(crate) fn lookup_function(name: &str) -> Option<*const u8> {
    function_registry().read().get(name).map(|p| p.0)
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

#[cfg(test)]
pub(crate) fn declared_handler_registry_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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

fn record_peer_transport_failure(peer: &str, now: Instant) {
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
    /// The TLS stream, shared between writer and reader threads
    pub(crate) stream: Mutex<NodeStream>,
    /// Signals the session's reader/heartbeat threads to stop
    pub shutdown: AtomicBool,
    /// When this connection was established
    pub connected_at: Instant,
    /// Version, bounds, and features negotiated during the authenticated handshake.
    pub negotiated_protocol: NegotiatedProtocol,
    /// Cluster/stable identity authenticated by the protocol-two signed claim.
    pub remote_identity: Option<super::identity_claim::NodeIdentityClaim>,
    /// Pending remote spawn requests: request_id -> requesting ProcessId.
    /// Used by DIST_SPAWN_REPLY handler to route the spawned PID back to
    /// the requesting process.
    pub(crate) pending_spawns: std::sync::Mutex<FxHashMap<u64, crate::actor::process::ProcessId>>,
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
    pub(crate) fn remote_has_role(&self, role: &str) -> bool {
        self.remote_identity
            .as_ref()
            .is_some_and(|identity| identity.roles.iter().any(|candidate| candidate == role))
    }

    fn new(
        endpoint: RemoteSessionEndpoint,
        stream: NodeStream,
        persistent: bool,
        negotiated_protocol: NegotiatedProtocol,
        remote_identity: Option<super::identity_claim::NodeIdentityClaim>,
    ) -> Self {
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
            stream: Mutex::new(stream),
            shutdown: AtomicBool::new(false),
            connected_at: Instant::now(),
            negotiated_protocol,
            remote_identity,
            pending_spawns: std::sync::Mutex::new(FxHashMap::default()),
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
            let mut stream = self.stream.lock().unwrap();
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
        // Heartbeats are liveness control, not application admission. Writing
        // them directly under the same stream mutex keeps frames atomic while
        // preventing a reservation burst from causing a false node failure.
        let mut stream = self.stream.lock().unwrap();
        write_msg(&mut *stream, &payload).map_err(|error| format!("peer_heartbeat_failed:{error}"))
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
/// Distribution message tag: remote process monitor setup.
/// Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
pub(crate) const DIST_MONITOR: u8 = 0x16;
/// Distribution message tag: remote process demonitor.
/// Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
pub(crate) const DIST_DEMONITOR: u8 = 0x17;
/// Distribution message tag: remote process monitor exit notification.
/// Wire format: [tag][u64 monitored_pid][u64 monitoring_pid][u64 ref][reason_bytes]
pub(crate) const DIST_MONITOR_EXIT: u8 = 0x18;

/// Reserved type_tag for :nodedown messages delivered to node monitors.
pub(crate) const NODEDOWN_TAG: u64 = u64::MAX - 2;
/// Reserved type_tag for :nodeup messages delivered to node monitors.
pub(crate) const NODEUP_TAG: u64 = u64::MAX - 3;
/// Distribution message tag: bidirectional link request.
/// Wire format: [tag][u64 from_pid][u64 to_pid]
pub(crate) const DIST_LINK: u8 = 0x13;
/// Distribution message tag: unlink request.
/// Wire format: [tag][u64 from_pid][u64 to_pid]
pub(crate) const DIST_UNLINK: u8 = 0x14;
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

/// Wire tag for transient clustered HTTP route requests.
/// Format: [tag 0x25][u16 runtime_name_len][runtime_name]
///         [u16 request_key_len][request_key][u16 attempt_id_len][attempt_id]
///         [u32 payload_len][encoded MeshHttpRequest payload]
pub(crate) const DIST_HTTP_ROUTE_QUERY: u8 = 0x25;

/// Wire tag for transient clustered HTTP route replies.
/// Format: [tag 0x26][u8 status][u32 payload_len][payload or UTF-8 reason]
pub(crate) const DIST_HTTP_ROUTE_REPLY: u8 = 0x26;

/// Wire tag for compact protocol-two node load reports.
/// Format: [tag 0x27][bounded NodeLoadReport payload]
pub(crate) const DIST_LOAD_REPORT: u8 = 0x27;

/// Protocol-two multiplexed HTTP request over an authenticated peer session.
/// Format: [tag 0x28][u64 correlation_id][protocol-one route request fields]
pub(crate) const DIST_HTTP_ROUTE_V2_QUERY: u8 = 0x28;

/// Protocol-two multiplexed HTTP completion over an authenticated peer session.
/// Format: [tag 0x29][u64 correlation_id][u8 status][u32 payload_len][payload]
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

/// Reserved type_tag for spawn reply messages in mailbox.
pub(crate) const SPAWN_REPLY_TAG: u64 = u64::MAX - 4;

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

    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

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
// DIST_LINK / DIST_UNLINK / DIST_EXIT send helpers
// ---------------------------------------------------------------------------

/// Send DIST_LINK to register a bidirectional link on the remote node.
/// Wire format: [DIST_LINK][u64 from_pid][u64 to_pid]
/// Silently drops if session unavailable (node already disconnected).
pub(crate) fn send_dist_link(from_pid: crate::actor::ProcessId, to_pid: crate::actor::ProcessId) {
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };
    let node_id = to_pid.node_id();
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return,
        }
    };
    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => Arc::clone(s),
            None => return,
        }
    };
    let mut payload = Vec::with_capacity(1 + 8 + 8);
    payload.push(DIST_LINK);
    payload.extend_from_slice(&from_pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&to_pid.as_u64().to_le_bytes());
    let _ = session.send(OutboundClass::Application, payload);
}

/// Send DIST_UNLINK to remove a bidirectional link on the remote node.
/// Wire format: [DIST_UNLINK][u64 from_pid][u64 to_pid]
/// Silently drops if session unavailable.
#[allow(dead_code)]
pub(crate) fn send_dist_unlink(from_pid: crate::actor::ProcessId, to_pid: crate::actor::ProcessId) {
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };
    let node_id = to_pid.node_id();
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return,
        }
    };
    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => Arc::clone(s),
            None => return,
        }
    };
    let mut payload = Vec::with_capacity(1 + 8 + 8);
    payload.push(DIST_UNLINK);
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
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };
    let node_id = to_pid.node_id();
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return,
        }
    };
    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => Arc::clone(s),
            None => return,
        }
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
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };
    let node_id = monitoring_pid.node_id();
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return,
        }
    };
    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => Arc::clone(s),
            None => return,
        }
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
    let Some(receivers) = session.outbound_receivers.lock().unwrap().take() else {
        session.shutdown.store(true, Ordering::Release);
        return;
    };
    let mut consecutive_control_frames = 0usize;
    while !session.shutdown.load(Ordering::Acquire) {
        let Some(first) = wait_for_outbound_frame(&receivers, &mut consecutive_control_frames)
        else {
            continue;
        };
        let mut batch = Vec::with_capacity(MAX_OUTBOUND_WRITE_BATCH);
        batch.push(first);
        while batch.len() < MAX_OUTBOUND_WRITE_BATCH {
            let Some(frame) = try_next_outbound_frame(&receivers, &mut consecutive_control_frames)
            else {
                break;
            };
            batch.push(frame);
        }
        // A rustls StreamOwned cannot be split into independent reader/writer
        // halves. Batching amortizes contention with the bounded reader poll
        // instead of reacquiring this lock for every small protocol frame.
        let mut written_application = false;
        let result = {
            let mut stream = session.stream.lock().unwrap();
            let mut result = Ok(());
            for frame in &batch {
                if let Err(error) = write_msg(&mut *stream, &frame.payload) {
                    result = Err(error);
                    break;
                }
                written_application |= matches!(frame.class, OutboundClass::Application);
            }
            result
        };
        for frame in &batch {
            release_outbound_frame_bytes(&session, frame);
        }
        if let Err(error) = result {
            record_peer_transport_failure(&session.remote_name, Instant::now());
            eprintln!(
                "mesh transport: transition=writer_failed remote={} reason={}",
                session.remote_name, error
            );
            session.shutdown.store(true, Ordering::Release);
            break;
        } else if written_application {
            record_peer_transport_success(&session.remote_name);
        }
    }
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
fn reader_loop_session(session: Arc<NodeSession>, heartbeat_state: Arc<Mutex<HeartbeatState>>) {
    // The incremental frame reader preserves partial prefixes/bodies across
    // socket timeouts, allowing the shared rustls stream lock to be released
    // frequently for control-plane writes without desynchronizing framing.
    {
        let s = session.stream.lock().unwrap();
        s.set_read_timeout(Some(Duration::from_millis(25))).ok();
    }
    let mut frame_reader = PersistentFrameReader::default();

    loop {
        if session.shutdown.load(Ordering::SeqCst) {
            break;
        }

        let result = {
            let mut s = session.stream.lock().unwrap();
            let maximum = if session.negotiated_protocol.version >= PROTOCOL_V2 {
                session.negotiated_protocol.max_frame_bytes
            } else {
                MAX_DIST_MSG
            };
            frame_reader.read_next(&mut *s, maximum)
        };

        match result {
            Ok(Some(frame)) => {
                let msg = match decode_session_payload(frame, &session.negotiated_protocol) {
                    Ok(message) => message,
                    Err(error) => {
                        eprintln!(
                            "mesh transport: transition=protocol_violation remote={} reason={}",
                            session.remote_name, error
                        );
                        session.shutdown.store(true, Ordering::Release);
                        break;
                    }
                };
                if msg.is_empty() {
                    continue;
                }
                match msg[0] {
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
                    DIST_SEND => {
                        // Wire format: [tag][u64 target_pid LE][raw message bytes]
                        if msg.len() >= 9 {
                            let target_pid = u64::from_le_bytes(msg[1..9].try_into().unwrap());
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
                            let name_len =
                                u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
                            if msg.len() >= 3 + name_len {
                                if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                                    if let Some(pid) =
                                        crate::actor::registry::global_registry().whereis(name)
                                    {
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
                    DIST_MONITOR => {
                        // Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
                        if msg.len() >= 25 {
                            use crate::actor::process::{ExitReason, ProcessId, ProcessState};
                            let from_pid =
                                ProcessId(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                            let to_pid =
                                ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                            let monitor_ref = u64::from_le_bytes(msg[17..25].try_into().unwrap());

                            let sched = crate::actor::global_scheduler();
                            match sched.get_process(to_pid) {
                                Some(target_arc) => {
                                    let mut target_proc = target_arc.lock();
                                    if matches!(target_proc.state, ProcessState::Exited(_)) {
                                        // Target already dead -- send DIST_MONITOR_EXIT back with noproc.
                                        drop(target_proc);
                                        let noproc = ExitReason::Error("noproc".to_string());
                                        send_dist_monitor_exit(
                                            &session,
                                            to_pid,
                                            from_pid,
                                            monitor_ref,
                                            &noproc,
                                        );
                                    } else {
                                        // Register monitor on local target.
                                        target_proc.monitored_by.insert(monitor_ref, from_pid);
                                    }
                                }
                                None => {
                                    // Target does not exist -- send DIST_MONITOR_EXIT back.
                                    let noproc = ExitReason::Error("noproc".to_string());
                                    send_dist_monitor_exit(
                                        &session,
                                        to_pid,
                                        from_pid,
                                        monitor_ref,
                                        &noproc,
                                    );
                                }
                            }
                        }
                    }
                    DIST_DEMONITOR => {
                        // Wire format: [tag][u64 from_pid][u64 to_pid][u64 ref]
                        if msg.len() >= 25 {
                            use crate::actor::process::ProcessId;
                            let _from_pid =
                                ProcessId(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                            let to_pid =
                                ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                            let monitor_ref = u64::from_le_bytes(msg[17..25].try_into().unwrap());

                            let sched = crate::actor::global_scheduler();
                            if let Some(target_arc) = sched.get_process(to_pid) {
                                target_arc.lock().monitored_by.remove(&monitor_ref);
                            }
                        }
                    }
                    DIST_MONITOR_EXIT => {
                        // Wire format: [tag][u64 monitored_pid][u64 monitoring_pid][u64 ref][reason_bytes]
                        if msg.len() >= 25 {
                            use crate::actor::heap::MessageBuffer;
                            use crate::actor::link;
                            use crate::actor::process::{Message, ProcessId, ProcessState};

                            let monitored_pid =
                                ProcessId(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                            let monitoring_pid =
                                ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                            let monitor_ref = u64::from_le_bytes(msg[17..25].try_into().unwrap());
                            let reason_bytes = &msg[25..];

                            let reason = if let Some((r, _)) = link::decode_reason(reason_bytes) {
                                r
                            } else {
                                crate::actor::process::ExitReason::Error("unknown".to_string())
                            };

                            let sched = crate::actor::global_scheduler();
                            if let Some(mon_arc) = sched.get_process(monitoring_pid) {
                                let mut mon_proc = mon_arc.lock();
                                mon_proc.monitors.remove(&monitor_ref);
                                let down_data =
                                    link::encode_down_signal(monitor_ref, monitored_pid, &reason);
                                let buffer = MessageBuffer::new(down_data, link::DOWN_SIGNAL_TAG);
                                mon_proc.mailbox.push(Message { buffer });
                                if matches!(mon_proc.state, ProcessState::Waiting) {
                                    if mon_proc.set_live_state(ProcessState::Ready) {
                                        drop(mon_proc);
                                        sched.wake_process(monitoring_pid);
                                    }
                                }
                            }
                        }
                    }
                    DIST_LINK => {
                        // Wire format: [tag][u64 from_pid][u64 to_pid]
                        if msg.len() >= 17 {
                            use crate::actor::process::ProcessId;
                            let from_pid =
                                ProcessId(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                            let to_pid =
                                ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                            // Add from_pid to the local process's links set
                            let sched = crate::actor::global_scheduler();
                            if let Some(proc_arc) = sched.get_process(to_pid) {
                                proc_arc.lock().links.insert(from_pid);
                            }
                        }
                    }
                    DIST_UNLINK => {
                        // Wire format: [tag][u64 from_pid][u64 to_pid]
                        if msg.len() >= 17 {
                            use crate::actor::process::ProcessId;
                            let from_pid =
                                ProcessId(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                            let to_pid =
                                ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                            let sched = crate::actor::global_scheduler();
                            if let Some(proc_arc) = sched.get_process(to_pid) {
                                proc_arc.lock().links.remove(&from_pid);
                            }
                        }
                    }
                    DIST_EXIT => {
                        // Wire format: [tag][u64 from_pid][u64 to_pid][reason_bytes]
                        if msg.len() >= 17 {
                            use crate::actor::heap::MessageBuffer;
                            use crate::actor::link;
                            use crate::actor::process::{
                                ExitReason, Message, ProcessId, ProcessState,
                            };

                            let from_pid =
                                ProcessId(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
                            let to_pid =
                                ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                            let reason_bytes = &msg[17..];
                            if let Some((reason, _)) = link::decode_reason(reason_bytes) {
                                let sched = crate::actor::global_scheduler();
                                if let Some(proc_arc) = sched.get_process(to_pid) {
                                    let mut proc = proc_arc.lock();
                                    if matches!(proc.state, ProcessState::Exited(_)) {
                                        continue; // Already dead, skip
                                    }
                                    proc.links.remove(&from_pid);
                                    let is_non_crashing =
                                        matches!(reason, ExitReason::Normal | ExitReason::Shutdown);
                                    if is_non_crashing || proc.trap_exit {
                                        let signal_data =
                                            link::encode_exit_signal(from_pid, &reason);
                                        let buffer =
                                            MessageBuffer::new(signal_data, link::EXIT_SIGNAL_TAG);
                                        proc.mailbox.push(Message { buffer });
                                        if matches!(proc.state, ProcessState::Waiting) {
                                            if proc.set_live_state(ProcessState::Ready) {
                                                drop(proc);
                                                sched.wake_process(to_pid);
                                            }
                                        }
                                    } else {
                                        proc.mark_exited(ExitReason::Linked(
                                            from_pid,
                                            Box::new(reason),
                                        ));
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
                            let requester_pid =
                                ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
                            let link_flag = msg[17];
                            let fn_name_len =
                                u16::from_le_bytes(msg[18..20].try_into().unwrap()) as usize;

                            if msg.len() >= 20 + fn_name_len {
                                let fn_name =
                                    std::str::from_utf8(&msg[20..20 + fn_name_len]).unwrap_or("");
                                let decoded_args =
                                    decode_remote_spawn_args(&msg[20 + fn_name_len..]);

                                match (lookup_function(fn_name), decoded_args) {
                                    (Some(fn_ptr), Ok(decoded_args)) => {
                                        let args_ptr = allocate_remote_spawn_args(&decoded_args);
                                        let args_size = (decoded_args.len()
                                            * std::mem::size_of::<u64>())
                                            as u64;

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
                                            let remote_requester = ProcessId::from_remote(
                                                session.node_id,
                                                session.remote_creation,
                                                requester_pid.local_id(),
                                            );
                                            if let Some(proc_arc) = sched.get_process(spawned) {
                                                proc_arc.lock().links.insert(remote_requester);
                                            }
                                            // Send DIST_LINK back so the requester's node records
                                            // the reverse link. from=spawned (local), to=requester (remote).
                                            // We send the local spawned PID as-is; the remote side will
                                            // use its own session info to qualify it.
                                            send_dist_link_via_session(
                                                &session,
                                                spawned,
                                                requester_pid,
                                            );
                                        }

                                        // Reply with the spawned process's local_id.
                                        send_spawn_reply(&session, req_id, 0, spawned.local_id());
                                    }
                                    (Some(_), Err(reason)) => {
                                        eprintln!(
                                            "mesh node spawn rejected from {} for fn {}: {}",
                                            session.remote_name, fn_name, reason
                                        );
                                        send_spawn_reply(&session, req_id, 1, 0);
                                    }
                                    (None, _) => {
                                        if lookup_declared_handler_executable(fn_name).is_some() {
                                            eprintln!(
                                                "mesh node spawn rejected from {}: declared handler executable not remote-registered {}",
                                                session.remote_name, fn_name
                                            );
                                        } else {
                                            eprintln!(
                                                "mesh node spawn rejected from {}: function not found {}",
                                                session.remote_name, fn_name
                                            );
                                        }
                                        send_spawn_reply(&session, req_id, 1, 0);
                                    }
                                }
                            }
                        }
                    }
                    DIST_SPAWN_REPLY => {
                        // Wire format: [tag][u64 req_id][u8 status][u64 spawned_local_id]
                        if msg.len() >= 18 {
                            use crate::actor::heap::MessageBuffer;
                            use crate::actor::process::{Message, ProcessState};

                            let req_id = u64::from_le_bytes(msg[1..9].try_into().unwrap());
                            let status = msg[9];
                            let spawned_local_id =
                                u64::from_le_bytes(msg[10..18].try_into().unwrap());

                            // Look up which process is waiting for this spawn reply.
                            let requester = session.pending_spawns.lock().unwrap().remove(&req_id);

                            if let Some(requester_pid) = requester {
                                // Build spawn reply payload: [u64 req_id][u8 status][u64 spawned_local_id]
                                let mut reply_data = Vec::with_capacity(17);
                                reply_data.extend_from_slice(&req_id.to_le_bytes());
                                reply_data.push(status);
                                reply_data.extend_from_slice(&spawned_local_id.to_le_bytes());

                                let buffer = MessageBuffer::new(reply_data, SPAWN_REPLY_TAG);
                                let reply_msg = Message { buffer };

                                let sched = crate::actor::global_scheduler();
                                if let Some(proc_arc) = sched.get_process(requester_pid) {
                                    let mut proc = proc_arc.lock();
                                    proc.mailbox.push(reply_msg);
                                    if matches!(proc.state, ProcessState::Waiting) {
                                        if proc.set_live_state(ProcessState::Ready) {
                                            drop(proc);
                                            sched.wake_process(requester_pid);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    DIST_GLOBAL_REGISTER => {
                        // Wire format: [tag][u16 name_len][name][u64 pid][u16 node_name_len][node_name]
                        if msg.len() >= 3 {
                            let name_len =
                                u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
                            if msg.len() >= 3 + name_len + 8 + 2 {
                                if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                                    let pid_raw = u64::from_le_bytes(
                                        msg[3 + name_len..3 + name_len + 8].try_into().unwrap(),
                                    );
                                    let node_name_len = u16::from_le_bytes(
                                        msg[3 + name_len + 8..3 + name_len + 10]
                                            .try_into()
                                            .unwrap(),
                                    )
                                        as usize;
                                    if msg.len() >= 3 + name_len + 10 + node_name_len {
                                        if let Ok(node_name) = std::str::from_utf8(
                                            &msg[3 + name_len + 10
                                                ..3 + name_len + 10 + node_name_len],
                                        ) {
                                            // Reconstruct the PID for our local view.
                                            // If the PID has node_id=0 (local to sender), replace
                                            // with this session's node_id so it routes correctly.
                                            use crate::actor::process::ProcessId;
                                            let mut pid = ProcessId(pid_raw);
                                            if pid.node_id() == 0 {
                                                pid = ProcessId::from_remote(
                                                    session.node_id,
                                                    session.remote_creation,
                                                    pid.local_id(),
                                                );
                                            }
                                            let _ = crate::dist::global::global_name_registry()
                                                .register(
                                                    name.to_string(),
                                                    pid,
                                                    node_name.to_string(),
                                                );
                                            // Silently drop errors (name conflict).
                                        }
                                    }
                                }
                            }
                        }
                    }
                    DIST_GLOBAL_UNREGISTER => {
                        // Wire format: [tag][u16 name_len][name]
                        if msg.len() >= 3 {
                            let name_len =
                                u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
                            if msg.len() >= 3 + name_len {
                                if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                                    crate::dist::global::global_name_registry().unregister(name);
                                }
                            }
                        }
                    }
                    DIST_GLOBAL_SYNC => {
                        // Wire format: [tag][u32 count][(u16 name_len, name, u64 pid, u16 node_len, node)*]
                        if msg.len() >= 5 {
                            use crate::actor::process::ProcessId;
                            let count = u32::from_le_bytes(msg[1..5].try_into().unwrap()) as usize;
                            let mut pos = 5;
                            let mut entries = Vec::with_capacity(count);
                            for _ in 0..count {
                                if pos + 2 > msg.len() {
                                    break;
                                }
                                let name_len =
                                    u16::from_le_bytes(msg[pos..pos + 2].try_into().unwrap())
                                        as usize;
                                pos += 2;
                                if pos + name_len + 8 + 2 > msg.len() {
                                    break;
                                }
                                let name = match std::str::from_utf8(&msg[pos..pos + name_len]) {
                                    Ok(s) => s,
                                    Err(_) => break,
                                };
                                pos += name_len;
                                let pid_raw =
                                    u64::from_le_bytes(msg[pos..pos + 8].try_into().unwrap());
                                pos += 8;
                                let node_len =
                                    u16::from_le_bytes(msg[pos..pos + 2].try_into().unwrap())
                                        as usize;
                                pos += 2;
                                if pos + node_len > msg.len() {
                                    break;
                                }
                                let node_name = match std::str::from_utf8(&msg[pos..pos + node_len])
                                {
                                    Ok(s) => s,
                                    Err(_) => break,
                                };
                                pos += node_len;

                                // Reconstruct PID: if node_id=0, qualify with session info.
                                let mut pid = ProcessId(pid_raw);
                                if pid.node_id() == 0 {
                                    pid = ProcessId::from_remote(
                                        session.node_id,
                                        session.remote_creation,
                                        pid.local_id(),
                                    );
                                }
                                entries.push((name.to_string(), pid, node_name.to_string()));
                            }
                            crate::dist::global::global_name_registry().merge_snapshot(entries);
                        }
                    }
                    DIST_CONTINUITY_UPSERT => {
                        match crate::dist::continuity::decode_upsert_payload(&msg) {
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
                        }
                    }
                    DIST_CONTINUITY_SYNC => {
                        match crate::dist::continuity::decode_sync_payload(&msg) {
                            Ok(snapshot) => {
                                if let Err(error) = crate::dist::continuity::continuity_registry()
                                    .merge_snapshot(snapshot)
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
                        }
                    }
                    DIST_CONTINUITY_STORE_SNAPSHOT => {
                        if let Err(error) =
                            crate::dist::continuity::handle_store_snapshot_chunk(&session, &msg)
                        {
                            eprintln!(
                                "mesh continuity: transition=store_snapshot_rejected remote={} error={}",
                                session.remote_name, error
                            );
                        }
                    }
                    DIST_CONTINUITY_STORE_SNAPSHOT_ACK => {
                        if let Err(error) =
                            crate::dist::continuity::handle_store_snapshot_ack(&session, &msg)
                        {
                            eprintln!(
                                "mesh continuity: transition=store_snapshot_ack_rejected remote={} error={}",
                                session.remote_name, error
                            );
                        }
                    }
                    DIST_CONTINUITY_STORE_LOG_ENTRY => {
                        if let Err(error) =
                            crate::dist::continuity::handle_store_log_entry(&session, &msg)
                        {
                            eprintln!(
                                "mesh continuity: transition=store_log_rejected remote={} error={}",
                                session.remote_name, error
                            );
                        }
                    }
                    DIST_CONTINUITY_PREPARE => {
                        if let Ok((request_id, record)) = decode_continuity_prepare_payload(&msg) {
                            dispatch_continuity_prepare(Arc::clone(&session), request_id, record);
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
                            crate::dist::operator::handle_operator_query_message(&session, &msg);
                        }
                    }
                    DIST_OPERATOR_REPLY => {
                        crate::dist::operator::handle_operator_reply_message(&session, &msg);
                    }
                    DIST_CONSENSUS_RPC => {
                        if autonomous_mode_requested() && !session.remote_has_role("controller") {
                            eprintln!(
                                "mesh consensus: transition=rpc_request_rejected remote={} reason=controller_identity_required",
                                session.remote_name
                            );
                            continue;
                        }
                        match decode_consensus_rpc_frame(&msg, DIST_CONSENSUS_RPC) {
                            Ok((correlation_id, request)) => {
                                crate::dist::consensus::handle_mesh_consensus_rpc(
                                    Arc::clone(&session),
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
                            continue;
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
                    DIST_LOAD_REPORT => {
                        match crate::dist::routing::NodeLoadReport::decode(&msg[1..]) {
                            Ok(report) if report.node_id == session.remote_name => {
                                if let Err(error) = crate::dist::routing::load_report_registry()
                                    .apply(report, Instant::now())
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
                        }
                    }
                    DIST_HTTP_ROUTE_V2_QUERY => {
                        dispatch_http_route_v2_reply(Arc::clone(&session), msg);
                    }
                    DIST_HTTP_ROUTE_V2_REPLY => {
                        if let Ok((correlation_id, result)) = decode_http_route_v2_reply_frame(&msg)
                        {
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
                        handle_http_reserve(&session, &msg);
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
                            if let Err(error) =
                                crate::dist::continuity_store::persist_runtime_response(
                                    &operation_key,
                                    &response,
                                )
                            {
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
                            let room_name_len =
                                u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
                            if msg.len() >= 3 + room_name_len + 4 {
                                if let Ok(room_name) =
                                    std::str::from_utf8(&msg[3..3 + room_name_len])
                                {
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
            Ok(None) => continue,
            Err(error) => {
                eprintln!(
                    "mesh transport: transition=reader_failed remote={} kind={:?} reason={}",
                    session.remote_name,
                    error.kind(),
                    error
                );
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
    let Some(state) = node_state() else {
        return;
    };
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

/// Test cleanup helper that removes any session stored for the remote name.
#[allow(dead_code)]
fn cleanup_session(remote_name: &str) {
    if let Some(state) = NODE_STATE.get() {
        let removed = {
            let mut sessions = state.sessions.write();
            sessions.remove(remote_name)
        };
        if let Some(session) = removed {
            fail_pending_session_requests(&session, "peer_session_disconnected");
            let node_id = session.node_id;
            let mut id_map = state.node_id_map.write();
            id_map.remove(&node_id);
            drop(id_map);
            // Phase 66: Fire all failure signals for the disconnected node.
            handle_node_disconnect(remote_name, node_id);
        }
    }
}

fn fail_pending_session_requests(session: &NodeSession, reason: &str) {
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

    let correlation_id = CONSENSUS_RPC_REQUEST_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| "consensus_rpc_correlation_exhausted".to_string())?;
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

    match tokio::time::timeout(timeout, receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("consensus_rpc_reply_disconnected".to_string()),
        Err(_) => {
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

    let sched = match crate::actor::GLOBAL_SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };

    let noconnection = ExitReason::Noconnection;

    // Phase 1: Collect under read lock.
    // For links: (local_pid, Vec<remote_pid_to_unlink>)
    let mut link_actions: Vec<(ProcessId, Vec<ProcessId>)> = Vec::new();
    // For monitors: (local_pid, Vec<(monitor_ref, monitored_pid)>)
    let mut monitor_actions: Vec<(ProcessId, Vec<(u64, ProcessId)>)> = Vec::new();

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
            let remote_monitors: Vec<(u64, ProcessId)> = proc
                .monitors
                .iter()
                .filter(|(_, monitored_pid)| monitored_pid.node_id() == node_id)
                .map(|(ref_id, pid)| (*ref_id, *pid))
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

            for (monitor_ref, monitored_pid) in monitors {
                proc.monitors.remove(monitor_ref);
                let down_data =
                    link::encode_down_signal(*monitor_ref, *monitored_pid, &noconnection);
                let buffer = MessageBuffer::new(down_data, link::DOWN_SIGNAL_TAG);
                proc.mailbox.push(Message { buffer });
            }

            if matches!(proc.state, ProcessState::Waiting) {
                if proc.set_live_state(ProcessState::Ready) {
                    drop(proc);
                    sched.wake_process(*local_pid);
                }
            }
        }
    }

    // Deliver :nodedown to node monitors.
    if let Some(state) = node_state() {
        let watchers = {
            let monitors = state.node_monitors.read();
            monitors.get(node_name).cloned()
        };

        if let Some(watchers) = watchers {
            for (watcher_pid, _once) in &watchers {
                deliver_node_event(*watcher_pid, node_name, NODEDOWN_TAG, sched);
            }

            // Remove "once" monitors.
            let mut monitors = state.node_monitors.write();
            if let Some(watchers) = monitors.get_mut(node_name) {
                watchers.retain(|(_, once)| !once);
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

/// Deliver a :nodeup or :nodedown message to a process.
///
/// Encodes the node name as the payload with the given type_tag (NODEDOWN_TAG or NODEUP_TAG).
fn deliver_node_event(
    target_pid: crate::actor::process::ProcessId,
    node_name: &str,
    type_tag: u64,
    sched: &crate::actor::Scheduler,
) {
    use crate::actor::heap::MessageBuffer;
    use crate::actor::process::{Message, ProcessState};

    let data = node_name.as_bytes().to_vec();
    let buffer = MessageBuffer::new(data, type_tag);
    let msg = Message { buffer };

    if let Some(proc_arc) = sched.get_process(target_pid) {
        let mut proc = proc_arc.lock();
        proc.mailbox.push(msg);
        if matches!(proc.state, ProcessState::Waiting) {
            if proc.set_live_state(ProcessState::Ready) {
                drop(proc);
                sched.wake_process(target_pid);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// handle_node_connect -- deliver :nodeup to node monitors
// ---------------------------------------------------------------------------

/// Handle a new node connection: deliver :nodeup to all registered node monitors.
///
/// Called from register_session after the session and id_map are fully set up.
fn handle_node_connect(node_name: &str) {
    let sched = match crate::actor::GLOBAL_SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };

    if let Some(state) = node_state() {
        let watchers = {
            let monitors = state.node_monitors.read();
            monitors.get(node_name).cloned()
        };

        if let Some(watchers) = watchers {
            for (watcher_pid, _once) in &watchers {
                deliver_node_event(*watcher_pid, node_name, NODEUP_TAG, sched);
            }
        }
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
        let mut active = active_ownership_transfers().lock().unwrap();
        let count = active.entry(node_id.to_string()).or_default();
        *count = count.saturating_add(1);
        Self {
            node_id: node_id.to_string(),
        }
    }
}

impl Drop for OwnershipTransferGuard {
    fn drop(&mut self) {
        let mut active = active_ownership_transfers().lock().unwrap();
        if let Some(count) = active.get_mut(&self.node_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                active.remove(&self.node_id);
            }
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
                    let state = node_state()
                        .ok_or_else(|| "continuity_recovery_node_not_started".to_string())?;
                    execute_clustered_http_route_remote(
                        &record.owner_node,
                        &state.cookie,
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
        let required: usize = record
            .replication_count
            .saturating_sub(1)
            .try_into()
            .map_err(|_| "replication_count_exceeds_platform_limit".to_string())?;
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
                    )?;
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
    let required: usize = record
        .replication_count
        .saturating_sub(1)
        .try_into()
        .map_err(|_| "replication_count_exceeds_platform_limit".to_string())?;
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
    let required: usize = replication_count
        .saturating_sub(1)
        .try_into()
        .map_err(|_| "replication_count_exceeds_platform_limit".to_string())?;
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
    crate::dist::routing::select_record_replicas(owner_node, required + 1, &reports)
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

    {
        if session.send(OutboundClass::Continuity, payload).is_err() {
            session
                .pending_continuity_prepares
                .lock()
                .unwrap()
                .remove(&request_id);
            let error = "replica_required_unavailable".to_string();
            crate::dist::operator::record_diagnostic(
                crate::dist::operator::OperatorDiagnosticRecord {
                    transition: "prepare_write_failed".to_string(),
                    request_key: Some(record.request_key.clone()),
                    attempt_id: Some(record.attempt_id.clone()),
                    owner_node: Some(record.owner_node.clone()),
                    replica_node: Some(record.replica_node.clone()),
                    cluster_role: Some(record.cluster_role.as_str().to_string()),
                    promotion_epoch: Some(record.promotion_epoch),
                    replication_health: Some(record.replication_health.as_str().to_string()),
                    replica_status: Some(record.replica_status.as_str().to_string()),
                    reason: Some(error.clone()),
                    metadata: vec![("target_node".to_string(), record.replica_node.clone())],
                    ..crate::dist::operator::OperatorDiagnosticRecord::default()
                },
            );
            eprintln!(
                "mesh continuity: transition=prepare_write_failed request_key={} attempt_id={} cluster_role={} promotion_epoch={} replication_health={} replica={} error={}",
                crate::dist::continuity::request_key_fingerprint(&record.request_key),
                record.attempt_id,
                record.cluster_role.as_str(),
                record.promotion_epoch,
                record.replication_health.as_str(),
                record.replica_node,
                error
            );
            return Err(error);
        }
    }

    match crate::actor::cooperative_recv_timeout(&rx, Duration::from_secs(5)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            session
                .pending_continuity_prepares
                .lock()
                .unwrap()
                .remove(&request_id);
            crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
            let error = "replica_prepare_timeout".to_string();
            crate::dist::operator::record_diagnostic(
                crate::dist::operator::OperatorDiagnosticRecord {
                    transition: "prepare_timeout".to_string(),
                    request_key: Some(record.request_key.clone()),
                    attempt_id: Some(record.attempt_id.clone()),
                    owner_node: Some(record.owner_node.clone()),
                    replica_node: Some(record.replica_node.clone()),
                    cluster_role: Some(record.cluster_role.as_str().to_string()),
                    promotion_epoch: Some(record.promotion_epoch),
                    replication_health: Some(record.replication_health.as_str().to_string()),
                    replica_status: Some(record.replica_status.as_str().to_string()),
                    reason: Some(error.clone()),
                    metadata: vec![("target_node".to_string(), record.replica_node.clone())],
                    ..crate::dist::operator::OperatorDiagnosticRecord::default()
                },
            );
            eprintln!(
                "mesh continuity: transition=prepare_timeout request_key={} attempt_id={} cluster_role={} promotion_epoch={} replication_health={} replica={} error={}",
                crate::dist::continuity::request_key_fingerprint(&record.request_key),
                record.attempt_id,
                record.cluster_role.as_str(),
                record.promotion_epoch,
                record.replication_health.as_str(),
                record.replica_node,
                error
            );
            Err(error)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            session
                .pending_continuity_prepares
                .lock()
                .unwrap()
                .remove(&request_id);
            let error = "replica_required_unavailable".to_string();
            crate::dist::operator::record_diagnostic(
                crate::dist::operator::OperatorDiagnosticRecord {
                    transition: "prepare_disconnected".to_string(),
                    request_key: Some(record.request_key.clone()),
                    attempt_id: Some(record.attempt_id.clone()),
                    owner_node: Some(record.owner_node.clone()),
                    replica_node: Some(record.replica_node.clone()),
                    cluster_role: Some(record.cluster_role.as_str().to_string()),
                    promotion_epoch: Some(record.promotion_epoch),
                    replication_health: Some(record.replication_health.as_str().to_string()),
                    replica_status: Some(record.replica_status.as_str().to_string()),
                    reason: Some(error.clone()),
                    metadata: vec![("target_node".to_string(), record.replica_node.clone())],
                    ..crate::dist::operator::OperatorDiagnosticRecord::default()
                },
            );
            eprintln!(
                "mesh continuity: transition=prepare_disconnected request_key={} attempt_id={} cluster_role={} promotion_epoch={} replication_health={} replica={} error={}",
                record.request_key,
                record.attempt_id,
                record.cluster_role.as_str(),
                record.promotion_epoch,
                record.replication_health.as_str(),
                record.replica_node,
                error
            );
            Err(error)
        }
    }
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
                        let result = match node_state() {
                            Some(state) if state.name == task.record.replica_node => {
                                crate::dist::continuity::continuity_registry()
                                    .mirror_prepare(task.record)
                                    .map(|_| ())
                            }
                            Some(_) => Err("replica_prepare_target_mismatch".to_string()),
                            None => Err("replica_required_unavailable".to_string()),
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
    fn read_next(
        &mut self,
        stream: &mut impl Read,
        max_frame_bytes: u32,
    ) -> io::Result<Option<Vec<u8>>> {
        while self.length_read < self.length.len() {
            match stream.read(&mut self.length[self.length_read..]) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(read) => self.length_read += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        }

        if self.payload.is_empty() && self.payload_read == 0 {
            let length = u32::from_le_bytes(self.length);
            let maximum = max_frame_bytes.min(MAX_DIST_MSG);
            if length > maximum {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("dist message too large: {length} bytes (max {maximum})"),
                ));
            }
            if length == 0 {
                self.reset();
                return Ok(Some(Vec::new()));
            }
            self.payload.resize(length as usize, 0);
        }

        while self.payload_read < self.payload.len() {
            match stream.read(&mut self.payload[self.payload_read..]) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(read) => self.payload_read += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
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

fn read_dist_msg_bounded(stream: &mut impl Read, max_frame_bytes: u32) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    let maximum = max_frame_bytes.min(MAX_DIST_MSG);
    if len > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("dist message too large: {} bytes (max {})", len, maximum),
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

fn compute_response(cookie: &str, challenge: &[u8; 32]) -> [u8; 32] {
    let signing_key = cluster_cookie_keys(cookie).next().unwrap_or(cookie);
    let mut mac =
        HmacSha256::new_from_slice(signing_key.as_bytes()).expect("HMAC can take key of any size");
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
    cluster_cookie_keys(cookie).any(|key| {
        let mut mac =
            HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
        mac.update(challenge);
        mac.verify_slice(response).is_ok()
    })
}

// ---------------------------------------------------------------------------
// Handshake message builders and parsers
// ---------------------------------------------------------------------------

/// Send NAME message: `[tag=1][u16 name_len][name_bytes][u8 creation]`.
fn send_name(stream: &mut impl Write, name: &str, creation: u8) -> Result<(), String> {
    send_named_message(stream, HANDSHAKE_NAME, "send_name", name, creation)
}

fn send_named_message(
    stream: &mut impl Write,
    tag: u8,
    label: &str,
    name: &str,
    creation: u8,
) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    let hello = local_protocol_hello_with_identity(name)?.encode()?;
    let mut payload = Vec::with_capacity(1 + 2 + name_bytes.len() + 1 + hello.len());
    payload.push(tag);
    payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(name_bytes);
    payload.push(creation);
    payload.extend_from_slice(&hello);
    write_msg(stream, &payload).map_err(|e| format!("{label} failed: {e}"))
}

fn decode_named_message(
    msg: &[u8],
    expected_tag: u8,
    label: &str,
) -> Result<(String, u8, ProtocolHello), String> {
    if msg.is_empty() || msg[0] != expected_tag {
        return Err(format!(
            "expected {label} tag ({expected_tag}), got {}",
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < 4 {
        return Err(format!("{label} message too short"));
    }
    let name_len = u16::from_le_bytes([msg[1], msg[2]]) as usize;
    if msg.len() < 3 + name_len + 1 {
        return Err(format!("{label} message truncated"));
    }
    let name = std::str::from_utf8(&msg[3..3 + name_len])
        .map_err(|_| "invalid UTF-8 in node name".to_string())?
        .to_string();
    let creation = msg[3 + name_len];
    let hello_start = 3 + name_len + 1;
    let hello = if msg.len() == hello_start {
        protocol_one_hello()
    } else {
        ProtocolHello::decode(&msg[hello_start..])?
    };
    Ok((name, creation, hello))
}

/// Receive and parse NAME message. Returns (name, creation).
fn recv_name(stream: &mut impl Read) -> Result<(String, u8, ProtocolHello), String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_name failed: {e}"))?;
    decode_named_message(&msg, HANDSHAKE_NAME, "HANDSHAKE_NAME")
}

/// Send CHALLENGE message: `[tag=2][u16 name_len][name_bytes][u8 creation][32 bytes challenge]`.
fn send_challenge(
    stream: &mut impl Write,
    name: &str,
    creation: u8,
    challenge: &[u8; 32],
) -> Result<(), String> {
    send_challenge_message(
        stream,
        HANDSHAKE_CHALLENGE,
        "send_challenge",
        name,
        creation,
        challenge,
    )
}

fn send_challenge_message(
    stream: &mut impl Write,
    tag: u8,
    label: &str,
    name: &str,
    creation: u8,
    challenge: &[u8; 32],
) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    let hello = local_protocol_hello_with_identity(name)?.encode()?;
    let mut payload = Vec::with_capacity(1 + 2 + name_bytes.len() + 1 + 32 + hello.len());
    payload.push(tag);
    payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(name_bytes);
    payload.push(creation);
    payload.extend_from_slice(challenge);
    payload.extend_from_slice(&hello);
    write_msg(stream, &payload).map_err(|e| format!("{label} failed: {e}"))
}

fn decode_challenge_message(
    msg: &[u8],
    expected_tag: u8,
    label: &str,
) -> Result<(String, u8, [u8; 32], ProtocolHello), String> {
    if msg.is_empty() || msg[0] != expected_tag {
        return Err(format!(
            "expected {label} tag ({expected_tag}), got {}",
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < 4 {
        return Err(format!("{label} message too short"));
    }
    let name_len = u16::from_le_bytes([msg[1], msg[2]]) as usize;
    if msg.len() < 3 + name_len + 1 + 32 {
        return Err(format!("{label} message truncated"));
    }
    let name = std::str::from_utf8(&msg[3..3 + name_len])
        .map_err(|_| "invalid UTF-8 in node name".to_string())?
        .to_string();
    let creation = msg[3 + name_len];
    let mut challenge = [0u8; 32];
    challenge.copy_from_slice(&msg[3 + name_len + 1..3 + name_len + 1 + 32]);
    let hello_start = 3 + name_len + 1 + 32;
    let hello = if msg.len() == hello_start {
        protocol_one_hello()
    } else {
        ProtocolHello::decode(&msg[hello_start..])?
    };
    Ok((name, creation, challenge, hello))
}

/// Receive and parse CHALLENGE message. Returns (name, creation, challenge).
fn recv_challenge(stream: &mut impl Read) -> Result<(String, u8, [u8; 32], ProtocolHello), String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_challenge failed: {e}"))?;
    decode_challenge_message(&msg, HANDSHAKE_CHALLENGE, "HANDSHAKE_CHALLENGE")
}

/// Send CHALLENGE_REPLY message: `[tag=3][32 bytes response][32 bytes own_challenge]`.
fn send_challenge_reply(
    stream: &mut impl Write,
    response: &[u8; 32],
    own_challenge: &[u8; 32],
) -> Result<(), String> {
    send_reply_message(
        stream,
        HANDSHAKE_REPLY,
        "send_challenge_reply",
        response,
        own_challenge,
    )
}

fn send_reply_message(
    stream: &mut impl Write,
    tag: u8,
    label: &str,
    response: &[u8; 32],
    own_challenge: &[u8; 32],
) -> Result<(), String> {
    let mut payload = Vec::with_capacity(1 + 32 + 32);
    payload.push(tag);
    payload.extend_from_slice(response);
    payload.extend_from_slice(own_challenge);
    write_msg(stream, &payload).map_err(|e| format!("{label} failed: {e}"))
}

fn decode_reply_message(
    msg: &[u8],
    expected_tag: u8,
    label: &str,
) -> Result<([u8; 32], [u8; 32]), String> {
    if msg.is_empty() || msg[0] != expected_tag {
        return Err(format!(
            "expected {label} tag ({expected_tag}), got {}",
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < 1 + 32 + 32 {
        return Err(format!("{label} message too short"));
    }
    let mut response = [0u8; 32];
    response.copy_from_slice(&msg[1..33]);
    let mut their_challenge = [0u8; 32];
    their_challenge.copy_from_slice(&msg[33..65]);
    Ok((response, their_challenge))
}

/// Receive and parse CHALLENGE_REPLY message. Returns (response, their_challenge).
fn recv_challenge_reply(stream: &mut impl Read) -> Result<([u8; 32], [u8; 32]), String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_challenge_reply failed: {e}"))?;
    decode_reply_message(&msg, HANDSHAKE_REPLY, "HANDSHAKE_REPLY")
}

/// Send CHALLENGE_ACK message: `[tag=4][32 bytes response]`.
fn send_challenge_ack(stream: &mut impl Write, response: &[u8; 32]) -> Result<(), String> {
    send_ack_message(stream, HANDSHAKE_ACK, "send_challenge_ack", response)
}

fn send_ack_message(
    stream: &mut impl Write,
    tag: u8,
    label: &str,
    response: &[u8; 32],
) -> Result<(), String> {
    let mut payload = Vec::with_capacity(1 + 32);
    payload.push(tag);
    payload.extend_from_slice(response);
    write_msg(stream, &payload).map_err(|e| format!("{label} failed: {e}"))
}

fn decode_ack_message(msg: &[u8], expected_tag: u8, label: &str) -> Result<[u8; 32], String> {
    if msg.is_empty() || msg[0] != expected_tag {
        return Err(format!(
            "expected {label} tag ({expected_tag}), got {}",
            msg.first().copied().unwrap_or(0)
        ));
    }
    if msg.len() < 1 + 32 {
        return Err(format!("{label} message too short"));
    }
    let mut response = [0u8; 32];
    response.copy_from_slice(&msg[1..33]);
    Ok(response)
}

/// Receive and parse CHALLENGE_ACK message. Returns the response.
fn recv_challenge_ack(stream: &mut impl Read) -> Result<[u8; 32], String> {
    let msg = read_msg(stream).map_err(|e| format!("recv_challenge_ack failed: {e}"))?;
    decode_ack_message(&msg, HANDSHAKE_ACK, "HANDSHAKE_ACK")
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
/// Returns `(remote_name, remote_creation)` on success, or an error string.
fn validate_remote_node_identity(
    remote_name: &str,
    remote_hello: &ProtocolHello,
) -> Result<Option<super::identity_claim::NodeIdentityClaim>, String> {
    if remote_hello.identity_envelope.is_empty() {
        return if autonomous_mode_requested() {
            Err("autonomous_peer_missing_signed_identity".to_string())
        } else {
            Ok(None)
        };
    }
    let cluster_id = std::env::var("MESH_CLUSTER_ID")
        .map_err(|_| "node_identity_cluster_missing".to_string())?;
    let verify_keys = std::env::var(super::identity_claim::IDENTITY_VERIFY_KEYS_ENV)
        .map_err(|_| "node_identity_verify_keys_missing".to_string())?;
    let claim = super::identity_claim::decode_and_verify_identity(
        &remote_hello.identity_envelope,
        &verify_keys,
        &cluster_id,
        remote_name,
        super::identity_claim::unix_millis(),
    )?;
    let voters = std::env::var("MESH_CONTROLLER_VOTERS").unwrap_or_default();
    let configured_voter = voters
        .split(',')
        .filter_map(|entry| entry.trim().split_once('|'));
    let authenticated_name = if is_transient_operator_client(remote_name)
        && claim.roles.iter().any(|role| role == "controller")
    {
        claim.advertised_name.as_str()
    } else {
        remote_name
    };
    let voter_binding_matches = configured_voter
        .clone()
        .any(|(stable_id, name)| stable_id == claim.stable_node_id && name == authenticated_name);
    let name_is_voter = configured_voter
        .clone()
        .any(|(_, name)| name == authenticated_name);
    if claim.roles.iter().any(|role| role == "controller") {
        if !voter_binding_matches {
            return Err("controller_identity_not_bound_to_voter".to_string());
        }
    } else if name_is_voter {
        return Err("non_controller_claimed_voter_name".to_string());
    }
    let operator = claim.roles.iter().any(|role| role == "operator");
    let controller = claim.roles.iter().any(|role| role == "controller");
    let transient_operator = is_transient_operator_client(remote_name);
    if (operator && !transient_operator) || (transient_operator && !operator && !controller) {
        return Err("operator_identity_channel_mismatch".to_string());
    }
    Ok(Some(claim))
}

fn perform_handshake_with_identity(
    stream: &mut (impl Read + Write),
    local_name: &str,
    local_cookie: &str,
    local_creation: u8,
    is_initiator: bool,
) -> Result<
    (
        String,
        u8,
        NegotiatedProtocol,
        Option<super::identity_claim::NodeIdentityClaim>,
    ),
    String,
> {
    if is_initiator {
        // Step 1: Send our name
        send_name(stream, local_name, local_creation)?;

        // Step 2: Receive their name + challenge
        let (remote_name, remote_creation, their_challenge, remote_hello) = recv_challenge(stream)?;
        validate_advertised_node_name(&remote_name)?;

        // Step 3: Compute response + generate our own challenge
        let our_response = compute_response(local_cookie, &their_challenge);
        let our_challenge = generate_challenge();
        send_challenge_reply(stream, &our_response, &our_challenge)?;

        // Step 4: Receive and verify their response to our challenge
        let their_response = recv_challenge_ack(stream)?;
        if !verify_response(local_cookie, &our_challenge, &their_response) {
            return Err(format!(
                "cookie mismatch: authentication failed from {}",
                remote_name
            ));
        }

        let negotiated = negotiate_protocol(&local_protocol_hello(), &remote_hello)?;
        let identity = validate_remote_node_identity(&remote_name, &remote_hello)?;
        Ok((remote_name, remote_creation, negotiated, identity))
    } else {
        // Step 1: Receive their name
        let (remote_name, remote_creation, remote_hello) = recv_name(stream)?;
        validate_advertised_node_name(&remote_name)?;

        // Duplicate-session resolution now happens in register_session after the
        // authenticated stream is fully built. Do not reject same-name reconnects
        // mid-handshake here; stale-session takeover and simultaneous connect both
        // rely on the later registration step being able to replace the old entry.

        // Step 2: Generate our challenge and send it
        let our_challenge = generate_challenge();
        send_challenge(stream, local_name, local_creation, &our_challenge)?;

        // Step 3: Receive their response + their challenge
        let (their_response, their_challenge) = recv_challenge_reply(stream)?;

        // Verify their response to our challenge
        if !verify_response(local_cookie, &our_challenge, &their_response) {
            return Err(format!(
                "cookie mismatch: authentication failed from {}",
                remote_name
            ));
        }

        // Step 4: Compute our response to their challenge and send ACK
        let our_response = compute_response(local_cookie, &their_challenge);
        send_challenge_ack(stream, &our_response)?;

        let negotiated = negotiate_protocol(&local_protocol_hello(), &remote_hello)?;
        let identity = validate_remote_node_identity(&remote_name, &remote_hello)?;
        Ok((remote_name, remote_creation, negotiated, identity))
    }
}

fn perform_handshake_negotiated(
    stream: &mut (impl Read + Write),
    state: &NodeState,
    is_initiator: bool,
) -> Result<
    (
        String,
        u8,
        NegotiatedProtocol,
        Option<super::identity_claim::NodeIdentityClaim>,
    ),
    String,
> {
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
    stream: &mut (impl Read + Write),
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
    let inserted_fresh = {
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
                false
            }
            None => {
                sessions.insert(remote_name.clone(), Arc::clone(&session));
                true
            }
        }
    };

    let mut id_map = state.node_id_map.write();
    if let Some(previous_node_id) = replaced_node_id {
        id_map.remove(&previous_node_id);
    }
    id_map.insert(node_id, remote_name.clone());
    drop(id_map);

    // Deliver :nodeup only for a fresh node-name registration. Transport
    // replacement during simultaneous connect or stale-session takeover keeps
    // the node logically up.
    if inserted_fresh {
        handle_node_connect(&remote_name);
    }

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
/// by the HMAC-SHA256 cookie challenge/response (Plan 02), not by PKI.
fn build_node_client_config() -> Arc<ClientConfig> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipCertVerification))
        .with_no_client_auth();
    Arc::new(config)
}

const TLS_CA_DER_B64_ENV: &str = "MESH_TLS_CA_DER_B64";
const TLS_CERT_DER_B64_ENV: &str = "MESH_TLS_CERT_DER_B64";
const TLS_KEY_DER_B64_ENV: &str = "MESH_TLS_KEY_DER_B64";

fn autonomous_mode_requested() -> bool {
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

fn configured_mtls_material() -> Result<
    Option<(
        Vec<CertificateDer<'static>>,
        CertificateDer<'static>,
        PrivateKeyDer<'static>,
    )>,
    String,
> {
    let values = [
        std::env::var(TLS_CA_DER_B64_ENV).ok(),
        std::env::var(TLS_CERT_DER_B64_ENV).ok(),
        std::env::var(TLS_KEY_DER_B64_ENV).ok(),
    ];
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
    if ca.is_empty() {
        return Err(format!("{TLS_CA_DER_B64_ENV}_empty"));
    }
    let cert = decode_tls_der(TLS_CERT_DER_B64_ENV, values[1].as_deref().unwrap())?;
    let key = decode_tls_der(TLS_KEY_DER_B64_ENV, values[2].as_deref().unwrap())?;
    Ok(Some((
        ca,
        CertificateDer::from(cert),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
    )))
}

type ConfiguredMtls = (Arc<ServerConfig>, Arc<ClientConfig>);

fn configured_mtls_configs() -> Result<Option<ConfiguredMtls>, String> {
    let Some((cas, certificate, private_key)) = configured_mtls_material()? else {
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

fn node_tls_configs() -> Result<(Arc<ServerConfig>, Arc<ClientConfig>), String> {
    if let Some(configs) = configured_mtls_configs()? {
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
    Ok(configured_mtls_configs()?
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
/// - `"name@[ipv6]"` -> (name, ipv6, 9000)  (default port)
/// - `"name@[ipv6]:port"` -> (name, ipv6, parsed_port)
///
/// Returns `Err` for invalid formats (no @, empty parts, invalid port).
pub fn parse_node_name(name: &str) -> Result<(&str, &str, u16), String> {
    let at_pos = name
        .find('@')
        .ok_or_else(|| format!("invalid node name '{}': missing '@' separator", name))?;

    let name_part = &name[..at_pos];
    let host_port = &name[at_pos + 1..];

    if name_part.is_empty() {
        return Err(format!("invalid node name '{}': empty name part", name));
    }

    let (host, port) = parse_host_port(host_port, 9000, name)?;
    Ok((name_part, host, port))
}

fn parse_bind_node_name(name: &str) -> Result<(&str, &str, u16), String> {
    let at_pos = name
        .find('@')
        .ok_or_else(|| format!("invalid node name '{}': missing '@' separator", name))?;

    let name_part = &name[..at_pos];
    let host_port = &name[at_pos + 1..];

    if name_part.is_empty() {
        return Err(format!("invalid node name '{}': empty name part", name));
    }

    let (host, port) = parse_bind_host_port(host_port, 9000, name)?;
    Ok((name_part, host, port))
}

fn parse_bind_host_port<'a>(
    host_port: &'a str,
    default_port: u16,
    full_value: &str,
) -> Result<(&'a str, u16), String> {
    if host_port.is_empty() {
        return Err(format!(
            "invalid node name '{}': empty host part",
            full_value
        ));
    }

    if let Some(rest) = host_port.strip_prefix('[') {
        let end = rest.find(']').ok_or_else(|| {
            format!(
                "invalid node name '{}': missing closing ']' for bracketed host",
                full_value
            )
        })?;
        let host = &rest[..end];
        let tail = &rest[end + 1..];
        if tail.is_empty() {
            return Ok((host, default_port));
        }
        let port_str = tail.strip_prefix(':').ok_or_else(|| {
            format!(
                "invalid node name '{}': expected ':' after bracketed host",
                full_value
            )
        })?;
        let port = port_str
            .parse::<u16>()
            .map_err(|_| format!("{} must be a valid u16", full_value))?;
        return Ok((host, port));
    }

    if let Some((host, port_str)) = host_port.rsplit_once(':') {
        if host.contains(':') {
            return Ok((host_port, default_port));
        }
        if host.is_empty() {
            return Err(format!(
                "invalid node name '{}': empty host part",
                full_value
            ));
        }
        let port = port_str
            .parse::<u16>()
            .map_err(|_| format!("{} must be a valid u16", full_value))?;
        return Ok((host, port));
    }

    Ok((host_port, default_port))
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
        let stream = session.stream.lock().unwrap();
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("transient_operator_timeout_set_failed:{error}"))?;
    }

    let msg = {
        let mut stream = session.stream.lock().unwrap();
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
    let (_name_part, host, port) =
        parse_node_name(target).map_err(|e| format!("invalid operator target: {}", e))?;

    let tcp_stream = TcpStream::connect((host, port))
        .map_err(|e| format!("TCP connect to {}:{} failed: {}", host, port, e))?;
    tcp_stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("transient_operator_read_timeout_failed:{e}"))?;
    tcp_stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| format!("transient_operator_write_timeout_failed:{e}"))?;

    let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
    let client_conn = rustls::ClientConnection::new(operator_tls_client_config()?, server_name)
        .map_err(|e| format!("TLS client connection failed: {}", e))?;
    let mut tls_stream = StreamOwned::new(client_conn, tcp_stream);

    let (_, _, negotiated, _) = perform_handshake_with_identity(
        &mut tls_stream,
        &transient_operator_client_name(),
        cookie,
        0,
        true,
    )
    .map_err(|e| format!("handshake with {}:{} failed: {}", host, port, e))?;

    write_msg(&mut tls_stream, payload)
        .map_err(|e| format!("transient_operator_query_write_failed:{e}"))?;
    read_dist_msg_bounded(&mut tls_stream, negotiated.max_frame_bytes)
        .map_err(|e| format!("transient_operator_reply_read_failed:{e}"))
}

const TRANSIENT_HTTP_ROUTE_CLIENT_NAME_PART: &str = "mesh-http-route";
const CLUSTERED_HTTP_ROUTE_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_RESERVATION_TIMEOUT: Duration = Duration::from_secs(3);
// The lease starts on the owner before the acceptance reply crosses the
// transport. It must outlive the ingress's complete post-acceptance route
// timeout while remaining bounded against clients that never send a query.
const HTTP_RESERVATION_LEASE: Duration = Duration::from_secs(10);

struct TransientHttpRouteReplyTask {
    msg: Vec<u8>,
    tx: mpsc::Sender<Result<Vec<u8>, String>>,
}

fn is_transient_http_route_client(remote_name: &str) -> bool {
    remote_name.starts_with(&format!("{TRANSIENT_HTTP_ROUTE_CLIENT_NAME_PART}@"))
}

fn transient_http_route_compatibility_allowed(
    negotiated: &NegotiatedProtocol,
    autonomous_requested: bool,
) -> bool {
    !autonomous_requested && negotiated.version == PROTOCOL_V1
}

extern "C" fn transient_http_route_reply_entry(args: *const u8) {
    if args.is_null() {
        return;
    }

    let words = unsafe { Box::from_raw(args as *mut [u64; 1]) };
    let task_ptr = words[0] as *mut TransientHttpRouteReplyTask;
    if task_ptr.is_null() {
        return;
    }

    let task = unsafe { Box::from_raw(task_ptr) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        build_http_route_reply_frame(&task.msg)
    }))
    .unwrap_or_else(|_| Err("transient_http_route_execute_panicked".to_string()));
    let _ = task.tx.send(result);
}

fn build_http_route_reply_via_actor(msg: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, String> {
    let (tx, rx) = mpsc::channel();
    let task_ptr = Box::into_raw(Box::new(TransientHttpRouteReplyTask { msg, tx })) as u64;
    let args_ptr = Box::into_raw(Box::new([task_ptr]));
    let pid = crate::actor::mesh_actor_spawn(
        transient_http_route_reply_entry as *const u8,
        args_ptr.cast(),
        std::mem::size_of::<u64>() as u64,
        1,
    );
    if pid == 0 {
        unsafe {
            drop(Box::from_raw(args_ptr));
            drop(Box::from_raw(task_ptr as *mut TransientHttpRouteReplyTask));
        }
        return Err("transient_http_route_actor_spawn_failed".to_string());
    }

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
            Err("transient_http_route_execute_timeout".to_string())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("transient_http_route_actor_disconnected".to_string())
        }
    }
}

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

fn encode_http_route_query_frame(
    runtime_name: &str,
    request_key: &str,
    attempt_id: &str,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    let payload_len = u32::try_from(request_payload.len()).map_err(|_| {
        format!(
            "clustered_http_route_request_too_large:{}",
            request_payload.len()
        )
    })?;
    let mut frame = Vec::with_capacity(
        1 + 2
            + runtime_name.len()
            + 2
            + request_key.len()
            + 2
            + attempt_id.len()
            + 4
            + request_payload.len(),
    );
    frame.push(DIST_HTTP_ROUTE_QUERY);
    encode_http_route_string(&mut frame, runtime_name)?;
    encode_http_route_string(&mut frame, request_key)?;
    encode_http_route_string(&mut frame, attempt_id)?;
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(request_payload);
    Ok(frame)
}

fn decode_http_route_query_frame(data: &[u8]) -> Result<(String, String, String, Vec<u8>), String> {
    if data.is_empty() {
        return Err("clustered_http_route_query_empty".to_string());
    }
    if data[0] != DIST_HTTP_ROUTE_QUERY {
        return Err(format!(
            "clustered_http_route_query_unexpected_tag:{}",
            data[0]
        ));
    }
    let mut pos = 1usize;
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
    Ok((runtime_name, request_key, attempt_id, data[pos..].to_vec()))
}

fn encode_http_route_reply_frame(result: Result<Vec<u8>, String>) -> Result<Vec<u8>, String> {
    let (status, payload) = match result {
        Ok(response_payload) => (0u8, response_payload),
        Err(reason) => (1u8, reason.into_bytes()),
    };
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| format!("clustered_http_route_reply_too_large:{}", payload.len()))?;
    let mut frame = Vec::with_capacity(1 + 1 + 4 + payload.len());
    frame.push(DIST_HTTP_ROUTE_REPLY);
    frame.push(status);
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_http_route_reply_frame(data: &[u8]) -> Result<Result<Vec<u8>, String>, String> {
    if data.len() < 6 {
        return Err("clustered_http_route_reply_too_short".to_string());
    }
    if data[0] != DIST_HTTP_ROUTE_REPLY {
        return Err(format!(
            "clustered_http_route_reply_unexpected_tag:{}",
            data[0]
        ));
    }
    let status = data[1];
    let payload_len = u32::from_le_bytes(data[2..6].try_into().unwrap()) as usize;
    if data.len() != 6 + payload_len {
        return Err("clustered_http_route_reply_length_mismatch".to_string());
    }
    let payload = &data[6..];
    match status {
        0 => Ok(Ok(payload.to_vec())),
        1 => Ok(Err(std::str::from_utf8(payload)
            .map_err(|_| "clustered_http_route_reply_reason_invalid_utf8".to_string())?
            .to_string())),
        other => Err(format!("invalid_clustered_http_route_reply_status:{other}")),
    }
}

struct AcceptedHttpReservation {
    _permit: crate::dist::telemetry::AdmissionPermit,
    expires_at: Instant,
}

fn encode_http_reserve(
    correlation_id: u64,
    runtime_name: &str,
    request_key: &str,
    payload_bytes: usize,
) -> Result<Vec<u8>, String> {
    let payload_bytes = u32::try_from(payload_bytes)
        .map_err(|_| "clustered_http_reservation_payload_too_large".to_string())?;
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

fn encode_http_route_v2_query_frame(
    correlation_id: u64,
    runtime_name: &str,
    request_key: &str,
    attempt_id: &str,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    let protocol_one =
        encode_http_route_query_frame(runtime_name, request_key, attempt_id, request_payload)?;
    let mut frame = Vec::with_capacity(8 + protocol_one.len());
    frame.push(DIST_HTTP_ROUTE_V2_QUERY);
    frame.extend_from_slice(&correlation_id.to_le_bytes());
    frame.extend_from_slice(&protocol_one[1..]);
    Ok(frame)
}

fn decode_http_route_v2_query_frame(data: &[u8]) -> Result<(u64, Vec<u8>), String> {
    if data.len() < 9 || data[0] != DIST_HTTP_ROUTE_V2_QUERY {
        return Err("clustered_http_route_v2_query_invalid".to_string());
    }
    let correlation_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let mut protocol_one = Vec::with_capacity(data.len() - 8);
    protocol_one.push(DIST_HTTP_ROUTE_QUERY);
    protocol_one.extend_from_slice(&data[9..]);
    decode_http_route_query_frame(&protocol_one)?;
    Ok((correlation_id, protocol_one))
}

fn encode_http_route_v2_reply_frame(
    correlation_id: u64,
    result: Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let protocol_one = encode_http_route_reply_frame(result)?;
    let mut frame = Vec::with_capacity(8 + protocol_one.len());
    frame.push(DIST_HTTP_ROUTE_V2_REPLY);
    frame.extend_from_slice(&correlation_id.to_le_bytes());
    frame.extend_from_slice(&protocol_one[1..]);
    Ok(frame)
}

fn decode_http_route_v2_reply_frame(data: &[u8]) -> Result<(u64, Result<Vec<u8>, String>), String> {
    if data.len() < 9 || data[0] != DIST_HTTP_ROUTE_V2_REPLY {
        return Err("clustered_http_route_v2_reply_invalid".to_string());
    }
    let correlation_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let mut protocol_one = Vec::with_capacity(data.len() - 8);
    protocol_one.push(DIST_HTTP_ROUTE_REPLY);
    protocol_one.extend_from_slice(&data[9..]);
    Ok((
        correlation_id,
        decode_http_route_reply_frame(&protocol_one)?,
    ))
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
    message: Vec<u8>,
    _reservation: AcceptedHttpReservation,
}

extern "C" fn http_route_v2_reply_entry(args: *const u8) {
    if args.is_null() {
        return;
    }
    let words = unsafe { Box::from_raw(args as *mut [u64; 1]) };
    let task_ptr = words[0] as *mut HttpRouteV2ReplyTask;
    if task_ptr.is_null() {
        return;
    }
    let task = unsafe { Box::from_raw(task_ptr) };
    let reply = decode_http_route_v2_query_frame(&task.message).and_then(
        |(correlation_id, protocol_one)| {
            let result =
                build_http_route_reply_frame(&protocol_one).and_then(|protocol_one_reply| {
                    decode_http_route_reply_frame(&protocol_one_reply)
                })?;
            encode_http_route_v2_reply_frame(correlation_id, result)
        },
    );
    if let Ok(reply) = reply {
        let _ = task.session.send(OutboundClass::Application, reply);
    }
}

fn dispatch_http_route_v2_reply(session: Arc<NodeSession>, message: Vec<u8>) {
    expire_http_reservations(&session, Instant::now());
    let correlation_id = match decode_http_route_v2_query_frame(&message) {
        Ok((correlation_id, _)) => correlation_id,
        Err(_) => return,
    };
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
        message,
        _reservation: reservation,
    })) as u64;
    let args_ptr = Box::into_raw(Box::new([task_ptr]));
    let pid = crate::actor::mesh_actor_spawn(
        http_route_v2_reply_entry as *const u8,
        args_ptr.cast(),
        std::mem::size_of::<u64>() as u64,
        1,
    );
    if pid == 0 {
        unsafe {
            drop(Box::from_raw(args_ptr));
            drop(Box::from_raw(task_ptr as *mut HttpRouteV2ReplyTask));
        }
    }
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
    cookie: &str,
    runtime_name: &str,
    request_key: &str,
    attempt_id: &str,
    request_payload: &[u8],
) -> Result<Vec<u8>, String> {
    let _ = cookie;
    let state = node_state().ok_or_else(|| "clustered_http_route_node_not_started".to_string())?;
    let session = state
        .sessions
        .read()
        .get(target)
        .cloned()
        .ok_or_else(|| format!("clustered_http_route_session_unavailable:{target}"))?;
    let correlation_id = HTTP_ROUTE_CORRELATION_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| "clustered_http_route_correlation_exhausted".to_string())?;
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
        request_payload.len(),
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
    match crate::actor::cooperative_recv_timeout(&reservation_receiver, HTTP_RESERVATION_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(reason)) => return Err(reason),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            session
                .pending_http_reservations
                .lock()
                .unwrap()
                .remove(&correlation_id);
            crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
            return Err("clustered_http_reservation_timeout".to_string());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("clustered_http_reservation_disconnected".to_string());
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
    match crate::actor::cooperative_recv_timeout(&receiver, CLUSTERED_HTTP_ROUTE_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            session
                .pending_http_routes
                .lock()
                .unwrap()
                .remove(&correlation_id);
            crate::dist::telemetry::runtime_telemetry().record_remote_dispatch_timeout();
            Err("clustered_http_route_reply_timeout".to_string())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("clustered_http_route_reply_disconnected".to_string())
        }
    }
}

fn build_http_route_reply_frame(msg: &[u8]) -> Result<Vec<u8>, String> {
    let (runtime_name, request_key, attempt_id, request_payload) =
        decode_http_route_query_frame(msg)?;
    let result = match lookup_declared_handler(&runtime_name) {
        Some(entry) => execute_clustered_http_route_locally(
            entry.fn_ptr.0,
            &request_key,
            &attempt_id,
            &request_payload,
        ),
        None => {
            let reason = format!("declared_handler_not_registered:{runtime_name}");
            reject_clustered_http_route_attempt(&request_key, &attempt_id, &reason);
            Err(reason)
        }
    };
    encode_http_route_reply_frame(result)
}

pub(crate) fn handle_transient_http_route_connection(
    remote_name: String,
    mut stream: NodeStream,
    timeout: Duration,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("transient_http_route_timeout_set_failed:{error}"))?;

    let msg = read_dist_msg(&mut stream)
        .map_err(|error| format!("transient_http_route_read_failed:{error}"))?;
    if msg.is_empty() {
        return Err("transient_http_route_query_empty".to_string());
    }
    if msg[0] != DIST_HTTP_ROUTE_QUERY {
        return Err(format!(
            "transient_http_route_query_unexpected_tag:{}",
            msg[0]
        ));
    }

    let reply = build_http_route_reply_via_actor(msg, timeout)
        .map_err(|error| format!("transient_http_route_execute_failed:{error}"))?;
    write_msg(&mut stream, &reply)
        .map_err(|error| format!("transient_http_route_reply_write_failed:{error}"))?;
    eprintln!(
        "mesh node: transient clustered HTTP route served for {}",
        remote_name
    );
    Ok(())
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
        || reason == "clustered_http_reservation_disconnected"
        || reason.starts_with("clustered_http_route_query_write_failed:")
        || reason == "clustered_http_route_reply_timeout"
        || reason == "clustered_http_route_reply_disconnected"
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
        let state = node_state().ok_or_else(|| {
            format!(
                "clustered_http_route_owner_unavailable:{}",
                prepared.decision.record.owner_node
            )
        })?;
        record_peer_original_attempt(&prepared.decision.record.owner_node, Instant::now());
        execute_clustered_http_route_remote(
            &prepared.decision.record.owner_node,
            &state.cookie,
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

fn handle_accepted_connection(tcp_stream: TcpStream, state: &NodeState) {
    if !auth_failures_below_limit() {
        eprintln!("mesh node: incoming connection rejected: authentication_rate_limited");
        return;
    }
    if let Err(error) = tcp_stream.set_nonblocking(false) {
        eprintln!("mesh node: accepted stream setup failed: {error}");
        return;
    }
    if let Err(error) = tcp_stream.set_read_timeout(Some(NODE_HANDSHAKE_TIMEOUT)) {
        eprintln!("mesh node: accepted stream read timeout setup failed: {error}");
        return;
    }
    if let Err(error) = tcp_stream.set_write_timeout(Some(NODE_HANDSHAKE_TIMEOUT)) {
        eprintln!("mesh node: accepted stream write timeout setup failed: {error}");
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

    if is_transient_http_route_client(&remote_name) {
        if !transient_http_route_compatibility_allowed(
            &negotiated_protocol,
            autonomous_mode_requested(),
        ) {
            eprintln!(
                "mesh node: transient clustered HTTP route rejected for {}: compatibility_channel_disabled",
                remote_name
            );
            return;
        }
        let stream = NodeStream::ServerTls(tls_stream);
        if let Err(error) = handle_transient_http_route_connection(
            remote_name.clone(),
            stream,
            CLUSTERED_HTTP_ROUTE_TIMEOUT,
        ) {
            eprintln!(
                "mesh node: transient clustered HTTP route failed for {}: {}",
                remote_name, error
            );
        }
        return;
    }

    if let Err(error) = tls_stream.sock.set_read_timeout(None) {
        eprintln!("mesh node: accepted stream read timeout reset failed: {error}");
        return;
    }
    if let Err(error) = tls_stream.sock.set_write_timeout(None) {
        eprintln!("mesh node: accepted stream write timeout reset failed: {error}");
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
            crate::dist::continuity::send_continuity_sync(&session);
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

fn start_named_node(name: &str, cookie: &str) -> i64 {
    mesh_node_start(
        name.as_ptr(),
        name.len() as u64,
        cookie.as_ptr(),
        cookie.len() as u64,
    )
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

fn mesh_string_ptr(value: &str) -> *mut MeshString {
    mesh_string_new(value.as_ptr(), value.len() as u64)
}

fn mesh_bootstrap_status(status: BootstrapStatus) -> MeshBootstrapStatus {
    MeshBootstrapStatus {
        mode: mesh_string_ptr(status.mode_label()),
        node_name: mesh_string_ptr(&status.node_name),
        cluster_port: i64::from(status.cluster_port),
        discovery_seed: mesh_string_ptr(&status.discovery_seed),
    }
}

fn bootstrap_ok_status(status: BootstrapStatus) -> *mut MeshResult {
    alloc_result(
        0,
        alloc_mesh_value(mesh_bootstrap_status(status)) as *mut u8,
    )
}

fn bootstrap_err_string(reason: &str) -> *mut MeshResult {
    alloc_result(
        1,
        mesh_string_new(reason.as_ptr(), reason.len() as u64) as *mut u8,
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
        Err(reason) => bootstrap_err_string(&reason),
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
#[no_mangle]
pub extern "C" fn mesh_node_start(
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

    if let Err(error) = validate_cluster_cookie_strength(&cookie, autonomous_mode_requested()) {
        eprintln!("mesh node: cluster authentication configuration failed: {error}");
        return -3;
    }

    // Parse "name@host" or "name@host:port"
    let (name_part, host, port) = match parse_bind_node_name(&name) {
        Ok(parsed) => parsed,
        Err(_) => return -3,
    };

    let host_owned = host.to_string();

    let (tls_server_config, tls_client_config) = match node_tls_configs() {
        Ok(configs) => configs,
        Err(error) => {
            eprintln!("mesh node: TLS configuration failed: {error}");
            return -3;
        }
    };

    // Bind TCP listener
    let listener = match TcpListener::bind((host_owned.as_str(), port)) {
        Ok(l) => l,
        Err(_) => return -2,
    };

    // Determine actual port (may differ if port 0 was requested)
    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let advertised_name = if port == 0 {
        format!("{name_part}@{host_owned}:{actual_port}")
    } else {
        name.clone()
    };

    // Initialize the global node state
    let _state = NODE_STATE.get_or_init(|| NodeState {
        name: advertised_name,
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
        node_monitors: RwLock::new(FxHashMap::default()),
    });

    // Spawn accept loop on a background thread.
    // Access NodeState via the static NODE_STATE, which is 'static.
    std::thread::spawn(move || {
        let state = NODE_STATE.get().expect("NODE_STATE initialized above");
        accept_loop(listener, state);
    });

    start_discovery_from_env();

    0
}

// ---------------------------------------------------------------------------
// connect_to_remote_node -- establish an outgoing authenticated session
// ---------------------------------------------------------------------------

fn connect_to_remote_node(state: &NodeState, target: &str) -> Result<Arc<NodeSession>, String> {
    // Parse host:port from target. Port is REQUIRED for connect.
    let (_name_part, host, port) =
        parse_node_name(target).map_err(|e| format!("invalid connect target: {}", e))?;

    // Open TCP connection
    let tcp_stream = TcpStream::connect((host, port))
        .map_err(|e| format!("TCP connect to {}:{} failed: {}", host, port, e))?;
    tcp_stream
        .set_read_timeout(Some(NODE_HANDSHAKE_TIMEOUT))
        .map_err(|error| format!("TCP read timeout setup failed: {error}"))?;
    tcp_stream
        .set_write_timeout(Some(NODE_HANDSHAKE_TIMEOUT))
        .map_err(|error| format!("TCP write timeout setup failed: {error}"))?;

    // Wrap in TLS client connection.
    // Server name is "mesh-node" -- doesn't matter since we skip verification.
    let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
    let client_conn =
        rustls::ClientConnection::new(Arc::clone(&state.tls_client_config), server_name)
            .map_err(|e| format!("TLS client connection failed: {}", e))?;
    let mut tls_stream = StreamOwned::new(client_conn, tcp_stream);

    // Perform HMAC-SHA256 cookie handshake (initiator side)
    let (remote_name, remote_creation, negotiated_protocol, remote_identity) =
        perform_handshake_negotiated(&mut tls_stream, state, true)
            .map_err(|e| format!("handshake with {}:{} failed: {}", host, port, e))?;
    tls_stream
        .sock
        .set_read_timeout(None)
        .map_err(|error| format!("TCP read timeout reset failed: {error}"))?;
    tls_stream
        .sock
        .set_write_timeout(None)
        .map_err(|error| format!("TCP write timeout reset failed: {error}"))?;

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
            crate::dist::continuity::send_continuity_sync(&session);
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
        Ok(_) => 0,
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
        Some(state) => crate::string::mesh_string_new(state.name.as_ptr(), state.name.len() as u64)
            as *const u8,
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
        let s = crate::string::mesh_string_new(name.as_ptr(), name.len() as u64);
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
            REMOTE_SPAWN_ARG_INT | REMOTE_SPAWN_ARG_FLOAT | REMOTE_SPAWN_ARG_PID => {
                payload.extend_from_slice(&raw.to_le_bytes());
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

fn decode_remote_spawn_args(data: &[u8]) -> Result<Vec<u64>, String> {
    if data.len() < 2 {
        return Err("remote_spawn_args_too_short".to_string());
    }

    let arg_count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
    if data.len() < 2 + arg_count {
        return Err("remote_spawn_arg_tags_truncated".to_string());
    }

    let arg_tags = &data[2..2 + arg_count];
    let mut pos = 2 + arg_count;
    let mut values = Vec::with_capacity(arg_count);

    for tag in arg_tags.iter().copied() {
        match tag {
            REMOTE_SPAWN_ARG_INT | REMOTE_SPAWN_ARG_FLOAT | REMOTE_SPAWN_ARG_PID => {
                if pos + 8 > data.len() {
                    return Err("remote_spawn_arg_value_truncated".to_string());
                }
                values.push(u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()));
                pos += 8;
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
const STARTUP_REQUEST_KEY_MISSING: &str = "startup_request_key_missing";
const STARTUP_DUPLICATE_REGISTRATION: &str = "startup_duplicate_registration";
const STARTUP_HANDLER_MISSING: &str = "startup_handler_not_registered";
const STARTUP_WORK_SPAWN_FAILED: &str = "startup_spawn_failed";
const STARTUP_KEEPALIVE_SPAWN_FAILED: &str = "startup_keepalive_spawn_failed";
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

    let request_key = startup_request_key(runtime_name);
    if request_key.is_empty() {
        return Err(STARTUP_REQUEST_KEY_MISSING.to_string());
    }

    Ok(StartupWorkIdentity {
        runtime_name: runtime_name.to_string(),
        request_key,
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
    let membership = canonical_declared_membership();
    if membership.is_empty() {
        return Err("declared_work_membership_empty".to_string());
    }

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
    if !membership.iter().any(|member| member == &owner_node) {
        return Err("declared_work_owner_not_in_membership".to_string());
    }
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
    let request_key_ptr =
        crate::string::mesh_string_new(request_key.as_ptr(), request_key.len() as u64);
    let attempt_id_ptr =
        crate::string::mesh_string_new(attempt_id.as_ptr(), attempt_id.len() as u64);
    let values = [request_key_ptr as u64, attempt_id_ptr as u64];
    (
        allocate_remote_spawn_args(&values),
        [REMOTE_SPAWN_ARG_STRING, REMOTE_SPAWN_ARG_STRING],
    )
}

fn startup_work_arg_payload(runtime_name: &str) -> *mut u8 {
    let runtime_name_ptr =
        crate::string::mesh_string_new(runtime_name.as_ptr(), runtime_name.len() as u64);
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
    if args.is_null() {
        log_startup_rejected_without_identity("", STARTUP_RUNTIME_NAME_MISSING);
        return;
    }

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

fn spawn_startup_work_actor(runtime_name: &str) -> Result<(), String> {
    let args_ptr = startup_work_arg_payload(runtime_name);
    let pid = crate::actor::mesh_actor_spawn(
        startup_work_entry as *const u8,
        args_ptr,
        std::mem::size_of::<u64>() as u64,
        1,
    );
    if pid == 0 {
        Err(STARTUP_WORK_SPAWN_FAILED.to_string())
    } else {
        Ok(())
    }
}

fn spawn_startup_keepalive_actor() -> Result<(), String> {
    let pid = crate::actor::mesh_actor_spawn(
        startup_keepalive_entry as *const u8,
        std::ptr::null(),
        0,
        2,
    );
    if pid == 0 {
        Err(STARTUP_KEEPALIVE_SPAWN_FAILED.to_string())
    } else {
        Ok(())
    }
}

fn trigger_startup_work_registrations<F, G>(
    runtime_names: &[String],
    cluster_mode: bool,
    cluster_role: crate::dist::continuity::ContinuityClusterRole,
    promotion_epoch: u64,
    mut spawn_startup: F,
    mut spawn_keepalive: G,
) where
    F: FnMut(&str) -> Result<(), String>,
    G: FnMut() -> Result<(), String>,
{
    if runtime_names.is_empty() {
        return;
    }

    if cluster_mode && !STARTUP_KEEPALIVE_SPAWNED.swap(true, Ordering::SeqCst) {
        match spawn_keepalive() {
            Ok(()) => log_startup_keepalive(runtime_names.len()),
            Err(reason) => {
                STARTUP_KEEPALIVE_SPAWNED.store(false, Ordering::SeqCst);
                log_startup_rejected_without_identity("", &reason);
            }
        }
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

        if let Err(reason) = spawn_startup(&identity.runtime_name) {
            log_startup_rejected(&identity, None, None, None, &reason);
        }
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
    let runtime_name_ptr =
        crate::string::mesh_string_new(runtime_name.as_ptr(), runtime_name.len() as u64);
    let request_key_ptr =
        crate::string::mesh_string_new(request_key.as_ptr(), request_key.len() as u64);
    let payload_hash_ptr =
        crate::string::mesh_string_new(payload_hash.as_ptr(), payload_hash.len() as u64);
    let previous_attempt_id_ptr = crate::string::mesh_string_new(
        previous_attempt_id.as_ptr(),
        previous_attempt_id.len() as u64,
    );
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
    if args.is_null() {
        return;
    }

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
) -> Result<(), String> {
    let args_ptr = automatic_recovery_arg_payload(
        runtime_name,
        request_key,
        payload_hash,
        previous_attempt_id,
    );
    let pid = crate::actor::mesh_actor_spawn(
        automatic_recovery_submit_entry as *const u8,
        args_ptr,
        (4 * std::mem::size_of::<u64>()) as u64,
        1,
    );
    if pid == 0 {
        Err("automatic_recovery_spawn_failed".to_string())
    } else {
        Ok(())
    }
}

fn spawn_declared_work_local(
    entry: &DeclaredHandlerEntry,
    request_key: &str,
    attempt_id: &str,
) -> Result<(), String> {
    let (args_ptr, _tags) = declared_work_arg_payload(request_key, attempt_id);
    let pid = crate::actor::mesh_actor_spawn(entry.fn_ptr.0, args_ptr, 16, 1);
    if pid == 0 {
        Err(format!(
            "declared_work_local_spawn_failed:{}",
            entry.executable_name
        ))
    } else {
        Ok(())
    }
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
    let Some(state) = node_state() else {
        return;
    };

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

        if let Err(reason) = spawn_automatic_recovery_submission(
            &runtime_name,
            &request_key,
            &payload_hash,
            &previous_attempt_id,
        ) {
            log_automatic_recovery_rejected(&request_key, &previous_attempt_id, &reason);
        }
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
        )
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
pub extern "C" fn mesh_node_spawn(
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
    use crate::actor::process::{ProcessId, ProcessState};
    use crate::actor::stack;

    // Must be called from within an actor context (coroutine).
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return 0,
    };

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
    session
        .pending_spawns
        .lock()
        .unwrap()
        .insert(req_id, my_pid);

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
            session
                .pending_spawns
                .lock()
                .unwrap()
                .insert(req_id, my_pid);

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

    // Wait for DIST_SPAWN_REPLY in the mailbox.
    // The reader thread will deliver it as a message with SPAWN_REPLY_TAG
    // containing [u64 req_id][u8 status][u64 spawned_local_id].
    let sched = crate::actor::global_scheduler();
    loop {
        // Check mailbox for a matching spawn reply (selective receive).
        if let Some(proc_arc) = sched.get_process(my_pid) {
            let reply = proc_arc.lock().mailbox.remove_first(|msg| {
                if msg.buffer.type_tag != SPAWN_REPLY_TAG {
                    return false;
                }
                if msg.buffer.data.len() < 17 {
                    return false;
                }
                let msg_req_id = u64::from_le_bytes(msg.buffer.data[0..8].try_into().unwrap());
                msg_req_id == req_id
            });

            if let Some(reply_msg) = reply {
                let status = reply_msg.buffer.data[8];
                let spawned_local_id =
                    u64::from_le_bytes(reply_msg.buffer.data[9..17].try_into().unwrap());

                if status == 0 {
                    // Construct the remote PID using session's node_id and creation.
                    let remote_pid = ProcessId::from_remote(
                        session.node_id,
                        session.remote_creation,
                        spawned_local_id,
                    );

                    // If spawn_link, add remote PID to our links set.
                    if link_flag == 1 {
                        if let Some(proc_arc) = sched.get_process(my_pid) {
                            proc_arc.lock().links.insert(remote_pid);
                        }
                    }

                    return remote_pid.as_u64();
                } else {
                    eprintln!(
                        "mesh node spawn failed target={} fn={}: remote_reply_status={} request_id={}",
                        node_name, fn_name, status, req_id
                    );
                    // Function not found or other error.
                    return 0;
                }
            }
        } else {
            // Our process no longer exists -- bail out.
            return 0;
        }

        // No matching reply yet. Enter Waiting state and yield.
        if let Some(proc_arc) = sched.get_process(my_pid) {
            let mut proc = proc_arc.lock();
            proc.set_live_state(ProcessState::Waiting);
            drop(proc);
        }
        stack::yield_current();
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

    fn startup_work_test_lock() -> std::sync::MutexGuard<'static, ()> {
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
                Ok(())
            },
            || {
                keepalive_spawns += 1;
                Ok(())
            },
        );

        trigger_startup_work_registrations(
            &runtime_names,
            true,
            crate::dist::continuity::ContinuityClusterRole::Primary,
            0,
            |runtime_name| {
                startup_spawns.push(format!("repeat:{runtime_name}"));
                Ok(())
            },
            || {
                keepalive_spawns += 1;
                Ok(())
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
                Ok(())
            },
            || {
                keepalive_spawns += 1;
                Ok(())
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
    fn test_cookie_keyring_allows_rolling_rotation() {
        let challenge = [99_u8; 32];
        let old_response = compute_response("old-cookie,new-cookie", &challenge);
        let new_response = compute_response("new-cookie,old-cookie", &challenge);

        assert!(verify_response(
            "new-cookie,old-cookie",
            &challenge,
            &old_response
        ));
        assert!(verify_response(
            "old-cookie,new-cookie",
            &challenge,
            &new_response
        ));
        assert!(!verify_response("new-cookie", &challenge, &old_response));
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

    #[test]
    fn test_mesh_node_start_binds_listener() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Use port 0 to get an OS-assigned port (avoids conflicts)
        let name = b"test@127.0.0.1:0";
        let cookie = b"secret";
        let result = mesh_node_start(
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
    fn test_bootstrap_from_env_composes_fly_identity_without_explicit_node_name() {
        let inputs = BootstrapInputs {
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("mesh.internal".to_string()),
            fly_app_name: Some("mesh-app".to_string()),
            fly_region: Some("iad".to_string()),
            fly_machine_id: Some("machine-1".to_string()),
            fly_private_ip: Some("fdaa:0:1::10".to_string()),
            ..BootstrapInputs::default()
        };

        let status = super::start_from_inputs_for_test(inputs, |name, _| {
            assert_eq!(name, "mesh-app-iad-machine-1@[fdaa:0:1::10]:4370");
            0
        })
        .expect("cluster bootstrap should succeed with Fly identity fallback");

        assert_eq!(status.mode, BootstrapMode::Cluster);
        assert_eq!(
            status.node_name,
            "mesh-app-iad-machine-1@[fdaa:0:1::10]:4370"
        );
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
    fn test_bootstrap_from_env_rejects_partial_fly_identity() {
        let inputs = BootstrapInputs {
            cookie: Some("shared-cookie".to_string()),
            discovery_seed: Some("mesh.internal".to_string()),
            fly_app_name: Some("mesh-app".to_string()),
            fly_region: Some("iad".to_string()),
            fly_machine_id: Some("machine-1".to_string()),
            ..BootstrapInputs::default()
        };

        let error = super::start_from_inputs_for_test(inputs, |_, _| 0).unwrap_err();
        assert_eq!(
            error,
            "Invalid cluster identity: Fly cluster identity requires FLY_APP_NAME, FLY_REGION, FLY_MACHINE_ID, and FLY_PRIVATE_IP"
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
        assert!(transient_http_route_compatibility_allowed(
            &negotiated,
            false
        ));
        assert!(!transient_http_route_compatibility_allowed(
            &negotiated,
            true
        ));
        let protocol_two = NegotiatedProtocol {
            version: PROTOCOL_V2,
            capabilities: super::super::protocol::Capabilities::AUTONOMOUS_REQUIRED,
            max_frame_bytes: 4096,
            autonomous_enabled: true,
            disabled_reason: None,
        };
        assert!(!transient_http_route_compatibility_allowed(
            &protocol_two,
            false
        ));
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
            node_monitors: RwLock::new(FxHashMap::default()),
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
            node_monitors: RwLock::new(FxHashMap::default()),
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
            if !verify_response(&protocol_one_cookie, &challenge, &response) {
                return Err("protocol_one_cookie_response_invalid".to_string());
            }
            let ack = compute_response(&protocol_one_cookie, &remote_challenge);
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
            node_monitors: RwLock::new(FxHashMap::default()),
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
            node_monitors: RwLock::new(FxHashMap::default()),
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
    fn test_handshake_rejects_invalid_remote_name() {
        use std::os::unix::net::UnixStream;

        let (stream_a, stream_b) = UnixStream::pair().unwrap();
        stream_a
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream_b
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        let state_a = NodeState {
            name: "alice@127.0.0.1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 9000,
            cookie: "shared_cookie".to_string(),
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
            node_monitors: RwLock::new(FxHashMap::default()),
        };

        let state_b = NodeState {
            name: "broken@[::1".to_string(),
            host: "127.0.0.1".to_string(),
            port: 9001,
            cookie: "shared_cookie".to_string(),
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
            node_monitors: RwLock::new(FxHashMap::default()),
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
                node_monitors: RwLock::new(FxHashMap::default()),
            };

            perform_handshake(&mut tls_stream, &state, false)
        });

        // Client (initiator) connects.
        let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let server_name: ServerName<'static> = "mesh-node".try_into().unwrap();
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
            node_monitors: RwLock::new(FxHashMap::default()),
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
        let peers = vec![
            "alpha@10.0.0.1:9000",
            "beta@10.0.0.2:9001",
            "gamma@10.0.0.3:9002",
        ];

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
            let name_len = u16::from_le_bytes(msg[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            let name = std::str::from_utf8(&msg[pos..pos + name_len]).unwrap();
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

    #[test]
    fn test_handle_peer_list_parsing_logic() {
        // Test the peer list wire format parsing logic that handle_peer_list uses.
        // We verify the parsing inline since handle_peer_list requires NODE_STATE
        // and spawns threads. This tests the same byte-reading code path.

        let peers = vec![
            "node_a@10.0.0.1:9000",
            "node_b@10.0.0.2:9001",
            "node_c@10.0.0.3:9002",
        ];

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
            let name_len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            let name = std::str::from_utf8(&data[pos..pos + name_len]).unwrap();
            decoded.push(name.to_string());
            pos += name_len;
        }

        assert_eq!(decoded, peers);

        // Test filtering logic: given a self-name and known-names, filter correctly
        let self_name = "node_a@10.0.0.1:9000";
        let known_names: Vec<&str> = vec!["node_b@10.0.0.2:9001"];

        let to_connect: Vec<&str> = decoded
            .iter()
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
        let all_sessions = [
            "peer_x@10.0.0.10:5000".to_string(),
            "peer_y@10.0.0.11:5001".to_string(),
            "receiving_node@10.0.0.12:5002".to_string(),
        ];
        let receiving_node = "receiving_node@10.0.0.12:5002";

        // Filter like send_peer_list does
        let peers: Vec<&String> = all_sessions
            .iter()
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
            let name_len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            let name = std::str::from_utf8(&data[pos..pos + name_len]).unwrap();
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
            if pos + 2 > data.len() {
                break;
            }
            let name_len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + name_len > data.len() {
                break;
            } // This should trigger
            let name = std::str::from_utf8(&data[pos..pos + name_len]).unwrap();
            decoded.push(name.to_string());
            pos += name_len;
        }

        // Should have decoded 0 peers (truncated name caused early break)
        assert_eq!(decoded.len(), 0);
    }
}
