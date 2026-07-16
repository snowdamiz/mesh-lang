use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::Duration;

use hmac::{Hmac, Mac};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::continuity::{
    continuity_registry, decode_record_payload, encode_record_payload, ContinuityAuthorityStatus,
    ContinuityRecord, ContinuityRegistry,
};
use super::node::{
    execute_transient_operator_query, node_state, NodeSession, DIST_OPERATOR_QUERY,
    DIST_OPERATOR_REPLY,
};

pub const DEFAULT_OPERATOR_QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_DIAGNOSTIC_CAPACITY: usize = 128;
const MAX_CONTINUITY_LIST_RECORDS: usize = 2_000;
const QUERY_KIND_STATUS: u8 = 0;
const QUERY_KIND_CONTINUITY_LOOKUP: u8 = 1;
const QUERY_KIND_CONTINUITY_LIST: u8 = 2;
const QUERY_KIND_DIAGNOSTICS: u8 = 3;
const QUERY_KIND_RUNTIME: u8 = 4;
const QUERY_KIND_CONTROL: u8 = 5;
const REPLY_STATUS_OK: u8 = 0;
const REPLY_STATUS_ERR: u8 = 1;

static OPERATOR_QUERY_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static DRAIN_PROPAGATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorMembershipSnapshot {
    pub local_node: String,
    pub peer_nodes: Vec<String>,
    pub nodes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorAuthoritySnapshot {
    pub cluster_role: String,
    pub promotion_epoch: u64,
    pub replication_health: String,
}

impl From<ContinuityAuthorityStatus> for OperatorAuthoritySnapshot {
    fn from(value: ContinuityAuthorityStatus) -> Self {
        Self {
            cluster_role: value.cluster_role.as_str().to_string(),
            promotion_epoch: value.promotion_epoch,
            replication_health: value.replication_health.as_str().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorStatusSnapshot {
    pub membership: OperatorMembershipSnapshot,
    pub authority: OperatorAuthoritySnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperatorNodeRuntimeSnapshot {
    pub node_id: String,
    #[serde(default)]
    pub protocol_version: u16,
    #[serde(default)]
    pub protocol_capabilities: u64,
    #[serde(default)]
    pub autonomous_protocol_enabled: bool,
    #[serde(default)]
    pub protocol_disabled_reason: Option<String>,
    pub roles: Vec<String>,
    pub state: String,
    pub routing_eligible: bool,
    pub capacity_units: u16,
    pub active_workers: u16,
    pub runnable_actors: u64,
    pub inflight: u32,
    #[serde(default)]
    pub continuity_active_work: u32,
    #[serde(default)]
    pub continuity_replica_responsibilities: u32,
    #[serde(default)]
    pub continuity_active_ownership_transfers: u32,
    #[serde(default)]
    pub continuity_only_active_copy: bool,
    pub queued_items: u32,
    pub queued_bytes: u64,
    pub reservations: u32,
    pub pressure: f64,
    pub dominant_signal: String,
    pub report_sequence: u64,
    pub control_term: u64,
    pub membership_generation: u64,
    pub failure_domain: String,
    #[serde(default)]
    pub handlers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperatorRuntimeSnapshot {
    pub schema_version: u16,
    pub local_node: String,
    pub telemetry_complete: bool,
    pub desired_capacity: u16,
    pub observed_capacity: u16,
    pub ready_capacity: u16,
    pub draining_capacity: u16,
    pub autoscaler_paused: bool,
    pub scheduler_min_workers: u16,
    pub scheduler_max_workers: u16,
    pub scheduler_active_workers: u16,
    #[serde(default)]
    pub local_readiness: super::readiness::NodeReadinessStatus,
    #[serde(default)]
    pub consensus: Option<super::consensus::ConsensusRuntimeSnapshot>,
    #[serde(default)]
    pub autonomous: super::autonomous::AutonomousControllerStatus,
    #[serde(default)]
    pub local_telemetry: super::telemetry::LocalTelemetrySnapshot,
    #[serde(default)]
    pub local_peer_sessions: Vec<super::telemetry::PeerSessionTelemetrySnapshot>,
    #[serde(default)]
    pub local_continuity_store: Option<super::continuity_store::ContinuityStoreStats>,
    #[serde(default)]
    pub local_continuity_store_error: Option<String>,
    pub nodes: Vec<OperatorNodeRuntimeSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OperatorControlAction {
    PauseAutoscaler,
    ResumeAutoscaler,
    SetDesiredCapacity {
        worker_nodes: u16,
    },
    DrainNode {
        node_id: String,
    },
    CancelDrain {
        node_id: String,
    },
    CommitControlMutation {
        command_id: String,
        mutation: super::scaling::ControlMutation,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorControlRequest {
    pub schema_version: u16,
    pub cluster_id: String,
    pub actor: String,
    pub sequence: u64,
    pub expires_at_unix_millis: u64,
    pub reason: String,
    pub action: OperatorControlAction,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorControlOutcome {
    pub schema_version: u16,
    pub accepted: bool,
    pub control_sequence: u64,
    pub autoscaler_paused: bool,
    pub desired_capacity_override: Option<u16>,
    pub drain_intents: Vec<String>,
    #[serde(default)]
    pub consensus: Option<super::consensus::ConsensusResponse>,
}

#[derive(Default)]
struct OperatorControlState {
    autoscaler_paused: bool,
    desired_capacity_override: Option<u16>,
    drain_intents: BTreeSet<String>,
    actor_sequences: BTreeMap<String, u64>,
    control_sequence: u64,
    last_consensus_log_index: u64,
}

static OPERATOR_CONTROL_STATE: OnceLock<Mutex<OperatorControlState>> = OnceLock::new();

fn operator_control_state() -> &'static Mutex<OperatorControlState> {
    OPERATOR_CONTROL_STATE.get_or_init(|| Mutex::new(OperatorControlState::default()))
}

pub(crate) fn autoscaler_paused() -> bool {
    refresh_operator_control_from_consensus();
    operator_control_state().lock().autoscaler_paused
}

pub(crate) fn drain_requested(node_id: &str) -> bool {
    refresh_operator_control_from_consensus();
    operator_control_state()
        .lock()
        .drain_intents
        .contains(node_id)
}

pub(crate) fn set_runtime_drain_intent(node_id: &str, draining: bool) {
    if node_id.is_empty() {
        return;
    }
    let runtime_node_id =
        super::node::resolve_runtime_node_id(node_id).unwrap_or_else(|_| node_id.to_string());
    let mut state = operator_control_state().lock();
    if draining {
        state.drain_intents.insert(runtime_node_id.clone());
    } else {
        state.drain_intents.remove(node_id);
        state.drain_intents.remove(&runtime_node_id);
    }
    state.control_sequence = state.control_sequence.saturating_add(1);
    drop(state);
    if node_state().is_some_and(|state| state.name == runtime_node_id) {
        super::telemetry::global_admission_controller().set_draining(draining);
    }
}

pub(crate) fn prepare_committed_drain(node_id: &str) {
    let runtime_node_id =
        super::node::resolve_runtime_node_id(node_id).unwrap_or_else(|_| node_id.to_string());
    set_runtime_drain_intent(&runtime_node_id, true);
    if node_state().is_some_and(|state| state.name == runtime_node_id) && node_id == runtime_node_id
    {
        return;
    }
    if node_state().is_some_and(|state| state.name != runtime_node_id) {
        let sequence_floor = unix_millis();
        let sequence = DRAIN_PROPAGATION_SEQUENCE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_add(1).max(sequence_floor))
            })
            .unwrap_or(sequence_floor)
            .saturating_add(1)
            .max(sequence_floor);
        let propagated = std::env::var("MESH_OPERATOR_KEY")
            .map_err(|_| "operator_control_not_configured".to_string())
            .and_then(|key| {
                sign_operator_control_request(
                    OperatorControlRequest {
                        schema_version: 1,
                        cluster_id: std::env::var("MESH_CLUSTER_ID")
                            .unwrap_or_else(|_| "mesh".to_string()),
                        actor: "mesh-drain-propagator".to_string(),
                        sequence,
                        expires_at_unix_millis: unix_millis().saturating_add(30_000),
                        reason: "quorum-committed local drain admission fence".to_string(),
                        action: OperatorControlAction::DrainNode {
                            node_id: runtime_node_id.clone(),
                        },
                        signature: String::new(),
                    },
                    &key,
                )
            })
            .and_then(|request| {
                query_operator_control_remote(
                    &runtime_node_id,
                    &std::env::var("MESH_CLUSTER_COOKIE").unwrap_or_default(),
                    request,
                    Duration::from_secs(5),
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
            });
        if let Err(error) = propagated {
            record_diagnostic(OperatorDiagnosticRecord {
                transition: "drain_target_propagation_failed".to_string(),
                reason: Some(error),
                metadata: vec![("node_id".to_string(), runtime_node_id.clone())],
                ..OperatorDiagnosticRecord::default()
            });
        }
    }
    if let Err(error) = super::node::prepare_continuity_for_drain(&runtime_node_id) {
        record_diagnostic(OperatorDiagnosticRecord {
            transition: "drain_continuity_blocked".to_string(),
            reason: Some(error.clone()),
            metadata: vec![("node_id".to_string(), runtime_node_id)],
            ..OperatorDiagnosticRecord::default()
        });
        eprintln!(
            "[mesh-rt drain] transition=continuity_blocked node_id={} reason={}",
            node_id, error
        );
    }
}

pub(crate) fn cancel_committed_drain(node_id: &str) {
    let runtime_node_id =
        super::node::resolve_runtime_node_id(node_id).unwrap_or_else(|_| node_id.to_string());
    set_runtime_drain_intent(&runtime_node_id, false);
    if node_state().is_none_or(|state| state.name == runtime_node_id) {
        return;
    }
    let sequence_floor = unix_millis();
    let sequence = DRAIN_PROPAGATION_SEQUENCE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            Some(current.saturating_add(1).max(sequence_floor))
        })
        .unwrap_or(sequence_floor)
        .saturating_add(1)
        .max(sequence_floor);
    let propagated = std::env::var("MESH_OPERATOR_KEY")
        .map_err(|_| "operator_control_not_configured".to_string())
        .and_then(|key| {
            sign_operator_control_request(
                OperatorControlRequest {
                    schema_version: 1,
                    cluster_id: std::env::var("MESH_CLUSTER_ID")
                        .unwrap_or_else(|_| "mesh".to_string()),
                    actor: "mesh-drain-propagator".to_string(),
                    sequence,
                    expires_at_unix_millis: unix_millis().saturating_add(30_000),
                    reason: "quorum-committed drain cancellation".to_string(),
                    action: OperatorControlAction::CancelDrain {
                        node_id: runtime_node_id.clone(),
                    },
                    signature: String::new(),
                },
                &key,
            )
        })
        .and_then(|request| {
            query_operator_control_remote(
                &runtime_node_id,
                &std::env::var("MESH_CLUSTER_COOKIE").unwrap_or_default(),
                request,
                Duration::from_secs(5),
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        });
    if let Err(error) = propagated {
        record_diagnostic(OperatorDiagnosticRecord {
            transition: "drain_cancel_propagation_failed".to_string(),
            reason: Some(error),
            metadata: vec![("node_id".to_string(), runtime_node_id)],
            ..OperatorDiagnosticRecord::default()
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorContinuityList {
    pub records: Vec<ContinuityRecord>,
    pub total_records: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperatorDiagnosticRecord {
    pub transition: String,
    pub request_key: Option<String>,
    pub attempt_id: Option<String>,
    pub owner_node: Option<String>,
    pub replica_node: Option<String>,
    pub execution_node: Option<String>,
    pub cluster_role: Option<String>,
    pub promotion_epoch: Option<u64>,
    pub replication_health: Option<String>,
    pub replica_status: Option<String>,
    pub reason: Option<String>,
    pub metadata: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorDiagnosticEntry {
    pub sequence: u64,
    pub transition: String,
    pub request_key: Option<String>,
    pub attempt_id: Option<String>,
    pub owner_node: Option<String>,
    pub replica_node: Option<String>,
    pub execution_node: Option<String>,
    pub cluster_role: Option<String>,
    pub promotion_epoch: Option<u64>,
    pub replication_health: Option<String>,
    pub replica_status: Option<String>,
    pub reason: Option<String>,
    pub metadata: Vec<(String, String)>,
}

impl OperatorDiagnosticRecord {
    fn into_entry(self, sequence: u64) -> OperatorDiagnosticEntry {
        OperatorDiagnosticEntry {
            sequence,
            transition: self.transition,
            request_key: self.request_key,
            attempt_id: self.attempt_id,
            owner_node: self.owner_node,
            replica_node: self.replica_node,
            execution_node: self.execution_node,
            cluster_role: self.cluster_role,
            promotion_epoch: self.promotion_epoch,
            replication_health: self.replication_health,
            replica_status: self.replica_status,
            reason: self.reason,
            metadata: self.metadata,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorDiagnosticsSnapshot {
    pub entries: Vec<OperatorDiagnosticEntry>,
    pub total_entries: usize,
    pub dropped_entries: u64,
    pub buffer_capacity: usize,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorQueryKind {
    Status,
    ContinuityLookup,
    ContinuityList,
    Diagnostics,
    Runtime,
    Control,
}

impl OperatorQueryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::ContinuityLookup => "continuity_lookup",
            Self::ContinuityList => "continuity_list",
            Self::Diagnostics => "diagnostics",
            Self::Runtime => "runtime",
            Self::Control => "control",
        }
    }

    fn to_wire(self) -> u8 {
        match self {
            Self::Status => QUERY_KIND_STATUS,
            Self::ContinuityLookup => QUERY_KIND_CONTINUITY_LOOKUP,
            Self::ContinuityList => QUERY_KIND_CONTINUITY_LIST,
            Self::Diagnostics => QUERY_KIND_DIAGNOSTICS,
            Self::Runtime => QUERY_KIND_RUNTIME,
            Self::Control => QUERY_KIND_CONTROL,
        }
    }

    fn from_wire(value: u8) -> Result<Self, String> {
        match value {
            QUERY_KIND_STATUS => Ok(Self::Status),
            QUERY_KIND_CONTINUITY_LOOKUP => Ok(Self::ContinuityLookup),
            QUERY_KIND_CONTINUITY_LIST => Ok(Self::ContinuityList),
            QUERY_KIND_DIAGNOSTICS => Ok(Self::Diagnostics),
            QUERY_KIND_RUNTIME => Ok(Self::Runtime),
            QUERY_KIND_CONTROL => Ok(Self::Control),
            other => Err(format!("invalid operator query kind {other}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperatorQueryError {
    InvalidRequest {
        query: OperatorQueryKind,
        reason: String,
    },
    LocalRejected {
        query: OperatorQueryKind,
        reason: String,
    },
    TargetUnavailable {
        target: String,
        query: OperatorQueryKind,
        reason: String,
    },
    Timeout {
        target: String,
        query: OperatorQueryKind,
        timeout: Duration,
    },
    RemoteRejected {
        target: String,
        query: OperatorQueryKind,
        reason: String,
    },
    Decode {
        target: String,
        query: OperatorQueryKind,
        reason: String,
    },
}

impl fmt::Display for OperatorQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest { query, reason } => {
                write!(f, "operator query {} invalid: {}", query.as_str(), reason)
            }
            Self::LocalRejected { query, reason } => {
                write!(
                    f,
                    "local operator query {} rejected: {}",
                    query.as_str(),
                    reason
                )
            }
            Self::TargetUnavailable {
                target,
                query,
                reason,
            } => write!(
                f,
                "operator query {} target {} unavailable: {}",
                query.as_str(),
                target,
                reason
            ),
            Self::Timeout {
                target,
                query,
                timeout,
            } => write!(
                f,
                "operator query {} target {} timed out after {}ms",
                query.as_str(),
                target,
                timeout.as_millis()
            ),
            Self::RemoteRejected {
                target,
                query,
                reason,
            } => write!(
                f,
                "operator query {} target {} rejected: {}",
                query.as_str(),
                target,
                reason
            ),
            Self::Decode {
                target,
                query,
                reason,
            } => write!(
                f,
                "operator query {} target {} decode failed: {}",
                query.as_str(),
                target,
                reason
            ),
        }
    }
}

impl std::error::Error for OperatorQueryError {}

#[derive(Default)]
struct OperatorDiagnosticsInner {
    next_sequence: u64,
    dropped_entries: u64,
    entries: VecDeque<OperatorDiagnosticEntry>,
}

pub struct OperatorDiagnosticsBuffer {
    capacity: usize,
    inner: RwLock<OperatorDiagnosticsInner>,
}

impl OperatorDiagnosticsBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: RwLock::new(OperatorDiagnosticsInner::default()),
        }
    }

    pub fn record(&self, record: OperatorDiagnosticRecord) {
        let mut inner = self.inner.write();
        inner.next_sequence = inner.next_sequence.saturating_add(1);
        let sequence = inner.next_sequence;
        if inner.entries.len() == self.capacity {
            inner.entries.pop_front();
            inner.dropped_entries = inner.dropped_entries.saturating_add(1);
        }
        inner.entries.push_back(record.into_entry(sequence));
    }

    pub fn snapshot(&self, limit: Option<usize>) -> OperatorDiagnosticsSnapshot {
        let inner = self.inner.read();
        let total_entries = inner.entries.len();
        let take = limit.unwrap_or(total_entries).min(total_entries);
        let start = total_entries.saturating_sub(take);
        let entries = inner.entries.iter().skip(start).cloned().collect();
        OperatorDiagnosticsSnapshot {
            entries,
            total_entries,
            dropped_entries: inner.dropped_entries,
            buffer_capacity: self.capacity,
            truncated: inner.dropped_entries > 0 || take < total_entries,
        }
    }

    #[cfg(test)]
    fn clear(&self) {
        *self.inner.write() = OperatorDiagnosticsInner::default();
    }
}

static OPERATOR_DIAGNOSTICS: OnceLock<OperatorDiagnosticsBuffer> = OnceLock::new();

fn diagnostics_buffer() -> &'static OperatorDiagnosticsBuffer {
    OPERATOR_DIAGNOSTICS.get_or_init(|| OperatorDiagnosticsBuffer::new(DEFAULT_DIAGNOSTIC_CAPACITY))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OperatorQuery {
    Status,
    ContinuityLookup { request_key: String },
    ContinuityList { limit: Option<usize> },
    Diagnostics { limit: Option<usize> },
    Runtime,
    Control(OperatorControlRequest),
}

impl OperatorQuery {
    fn kind(&self) -> OperatorQueryKind {
        match self {
            Self::Status => OperatorQueryKind::Status,
            Self::ContinuityLookup { .. } => OperatorQueryKind::ContinuityLookup,
            Self::ContinuityList { .. } => OperatorQueryKind::ContinuityList,
            Self::Diagnostics { .. } => OperatorQueryKind::Diagnostics,
            Self::Runtime => OperatorQueryKind::Runtime,
            Self::Control(_) => OperatorQueryKind::Control,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum OperatorReply {
    Status(OperatorStatusSnapshot),
    ContinuityRecord(ContinuityRecord),
    ContinuityList(OperatorContinuityList),
    Diagnostics(OperatorDiagnosticsSnapshot),
    Runtime(OperatorRuntimeSnapshot),
    Control(OperatorControlOutcome),
}

pub(crate) fn record_diagnostic(record: OperatorDiagnosticRecord) {
    let mut record = record;
    if let Some(request_key) = record.request_key.as_deref() {
        if !request_key.starts_with("sha256:") {
            record.request_key = Some(super::continuity::request_key_fingerprint(request_key));
        }
    }
    diagnostics_buffer().record(record);
}

pub fn operator_recent_diagnostics(limit: Option<usize>) -> OperatorDiagnosticsSnapshot {
    diagnostics_buffer().snapshot(limit)
}

pub fn operator_continuity_list(limit: Option<usize>) -> OperatorContinuityList {
    continuity_list_from_registry(continuity_registry(), limit)
}

pub fn operator_continuity_status(
    request_key: &str,
) -> Result<ContinuityRecord, OperatorQueryError> {
    if request_key.is_empty() {
        return Err(OperatorQueryError::InvalidRequest {
            query: OperatorQueryKind::ContinuityLookup,
            reason: "request_key_missing".to_string(),
        });
    }

    continuity_registry()
        .record(request_key)
        .ok_or_else(|| OperatorQueryError::LocalRejected {
            query: OperatorQueryKind::ContinuityLookup,
            reason: "request_key_not_found".to_string(),
        })
}

pub fn operator_status() -> Result<OperatorStatusSnapshot, OperatorQueryError> {
    let state = node_state().ok_or_else(|| OperatorQueryError::TargetUnavailable {
        target: "<local>".to_string(),
        query: OperatorQueryKind::Status,
        reason: "node_not_started".to_string(),
    })?;
    let peer_nodes = peer_names(state);
    Ok(status_snapshot_from_parts(
        &state.name,
        &peer_nodes,
        continuity_registry().authority_status(),
    ))
}

pub fn operator_runtime_snapshot() -> Result<OperatorRuntimeSnapshot, OperatorQueryError> {
    let state = node_state().ok_or_else(|| OperatorQueryError::TargetUnavailable {
        target: "<local>".to_string(),
        query: OperatorQueryKind::Runtime,
        reason: "node_not_started".to_string(),
    })?;
    Ok(runtime_snapshot_from_state(state))
}

fn runtime_snapshot_from_state(state: &super::node::NodeState) -> OperatorRuntimeSnapshot {
    refresh_operator_control_from_consensus();
    let now = std::time::Instant::now();
    let routing_policy = super::routing::runtime_routing_policy();
    let local_report = super::routing::local_load_report(&state.name, BTreeSet::new());
    let _ = super::routing::load_report_registry().apply(local_report, now);
    let mut membership = peer_names(state);
    membership.push(state.name.clone());
    membership.sort();
    membership.dedup();
    let live_nodes: BTreeSet<String> = membership.iter().cloned().collect();
    let reports: Vec<_> = membership
        .iter()
        .filter_map(|node| {
            super::routing::load_report_registry().report(node, now, routing_policy.load_report_ttl)
        })
        .collect();
    let peer_protocols: BTreeMap<_, _> = state
        .sessions
        .read()
        .iter()
        .map(|(node, session)| (node.clone(), session.negotiated_protocol.clone()))
        .collect();
    let telemetry_complete = reports.len() == membership.len();
    let nodes: Vec<_> = reports
        .iter()
        .map(|report| {
            let pressure = report.pressure(
                routing_policy.target_inflight,
                routing_policy.target_queue_wait,
            );
            let mut roles = Vec::new();
            if report
                .roles
                .contains(crate::dist::telemetry::NodeRoles::CONTROLLER)
            {
                roles.push("controller".to_string());
            }
            if report
                .roles
                .contains(crate::dist::telemetry::NodeRoles::GATEWAY)
            {
                roles.push("gateway".to_string());
            }
            if report
                .roles
                .contains(crate::dist::telemetry::NodeRoles::WORKER)
            {
                roles.push("worker".to_string());
            }
            let drain_intent = drain_requested(&report.node_id);
            let protocol = peer_protocols.get(&report.node_id);
            let local_protocol = report.node_id == state.name;
            let continuity_safety =
                super::continuity_store::continuity_node_safety(&report.node_id, &live_nodes)
                    .unwrap_or(super::continuity_store::ContinuityNodeSafety {
                        active_owned_records: u32::MAX,
                        required_replica_responsibilities: u32::MAX,
                        only_active_copy: true,
                    });
            OperatorNodeRuntimeSnapshot {
                node_id: report.node_id.clone(),
                protocol_version: protocol.map_or_else(
                    || u16::from(local_protocol) * super::protocol::PROTOCOL_V2,
                    |protocol| protocol.version,
                ),
                protocol_capabilities: protocol.map_or_else(
                    || {
                        if local_protocol {
                            super::protocol::Capabilities::AUTONOMOUS_REQUIRED.bits()
                        } else {
                            0
                        }
                    },
                    |protocol| protocol.capabilities.bits(),
                ),
                autonomous_protocol_enabled: protocol
                    .is_some_and(|protocol| protocol.autonomous_enabled)
                    || local_protocol,
                protocol_disabled_reason: protocol
                    .and_then(|protocol| protocol.disabled_reason.clone())
                    .or_else(|| (!local_protocol).then(|| "session_unavailable".to_string())),
                roles,
                state: if drain_intent {
                    "draining".to_string()
                } else {
                    report.state.as_str().to_string()
                },
                routing_eligible: report.state.routing_eligible() && !drain_intent,
                capacity_units: report.capacity_units,
                active_workers: report.active_workers,
                runnable_actors: report.runnable_actors,
                inflight: report.inflight,
                continuity_active_work: continuity_safety.active_owned_records,
                continuity_replica_responsibilities: continuity_safety
                    .required_replica_responsibilities,
                continuity_active_ownership_transfers:
                    super::node::continuity_active_ownership_transfers(&report.node_id),
                continuity_only_active_copy: continuity_safety.only_active_copy,
                queued_items: report.queued_items,
                queued_bytes: report.queued_bytes,
                reservations: report.outstanding_reservations,
                pressure: report.decision_pressure_ewma,
                dominant_signal: pressure.dominant_signal.to_string(),
                report_sequence: report.sequence,
                control_term: report.control_term,
                membership_generation: report.membership_generation,
                failure_domain: report.failure_domain.clone(),
                handlers: report.handlers.iter().cloned().collect(),
            }
        })
        .collect();
    let ready_capacity = nodes
        .iter()
        .filter(|node| node.state == "ready")
        .count()
        .try_into()
        .unwrap_or(u16::MAX);
    let draining_capacity = nodes
        .iter()
        .filter(|node| node.state == "draining")
        .count()
        .try_into()
        .unwrap_or(u16::MAX);
    let (scheduler_min_workers, scheduler_max_workers, scheduler_active_workers) =
        crate::actor::GLOBAL_SCHEDULER
            .get()
            .map_or((0, 0, 0), |scheduler| {
                let (minimum, maximum) = scheduler.worker_bounds();
                (
                    minimum.try_into().unwrap_or(u16::MAX),
                    maximum.try_into().unwrap_or(u16::MAX),
                    scheduler.active_workers().try_into().unwrap_or(u16::MAX),
                )
            });
    let observed_capacity = membership.len().try_into().unwrap_or(u16::MAX);
    let control = operator_control_state().lock();
    let desired_capacity = control.desired_capacity_override.unwrap_or_else(|| {
        std::env::var("MESH_DESIRED_CAPACITY")
            .ok()
            .and_then(|raw| raw.parse::<u16>().ok())
            .unwrap_or(observed_capacity)
    });
    let autoscaler_paused = control.autoscaler_paused;
    drop(control);
    super::routing::refresh_local_routing_telemetry();
    let local_peer_sessions = super::node::local_peer_session_telemetry();
    let local_telemetry = super::telemetry::runtime_telemetry().snapshot();
    let (local_continuity_store, local_continuity_store_error) =
        match super::continuity_store::configured_continuity_store().map(|store| store.stats()) {
            Some(Ok(stats)) => (Some(stats), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
    OperatorRuntimeSnapshot {
        schema_version: 6,
        local_node: state.name.clone(),
        telemetry_complete,
        desired_capacity,
        observed_capacity,
        ready_capacity,
        draining_capacity,
        autoscaler_paused,
        scheduler_min_workers,
        scheduler_max_workers,
        scheduler_active_workers,
        local_readiness: super::readiness::local_readiness_status(),
        consensus: super::consensus::consensus_runtime_snapshot(),
        autonomous: super::autonomous::autonomous_controller_status(),
        local_telemetry,
        local_peer_sessions,
        local_continuity_store,
        local_continuity_store_error,
        nodes,
    }
}

fn control_signature_payload(request: &OperatorControlRequest) -> Result<Vec<u8>, String> {
    let action = serde_json::to_vec(&request.action)
        .map_err(|error| format!("operator_control_action_encode_failed:{error}"))?;
    let schema_version = request.schema_version.to_string();
    let sequence = request.sequence.to_string();
    let expires_at = request.expires_at_unix_millis.to_string();
    let mut payload = Vec::new();
    for component in [
        schema_version.as_bytes(),
        request.cluster_id.as_bytes(),
        request.actor.as_bytes(),
        sequence.as_bytes(),
        expires_at.as_bytes(),
        request.reason.as_bytes(),
        action.as_slice(),
    ] {
        let length: u64 = component
            .len()
            .try_into()
            .map_err(|_| "operator_control_component_too_large".to_string())?;
        payload.extend_from_slice(&length.to_be_bytes());
        payload.extend_from_slice(component);
    }
    Ok(payload)
}

pub fn sign_operator_control_request(
    mut request: OperatorControlRequest,
    operator_key: &str,
) -> Result<OperatorControlRequest, String> {
    let signing_key = operator_control_keys(operator_key)
        .next()
        .ok_or_else(|| "operator_control_key_missing".to_string())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(signing_key.as_bytes())
        .map_err(|_| "operator_control_key_invalid".to_string())?;
    mac.update(&control_signature_payload(&request)?);
    request.signature = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(request)
}

fn operator_control_keys(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(',').map(str::trim).filter(|key| key.len() >= 32)
}

fn operator_control_signature_matches(raw_keys: &str, payload: &[u8], signature: &[u8]) -> bool {
    operator_control_keys(raw_keys).any(|candidate| {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(candidate.as_bytes()) else {
            return false;
        };
        mac.update(payload);
        mac.verify_slice(signature).is_ok()
    })
}

fn decode_hex_signature(signature: &str) -> Result<[u8; 32], String> {
    if signature.len() != 64 {
        return Err("operator_control_signature_invalid".to_string());
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&signature[index * 2..index * 2 + 2], 16)
            .map_err(|_| "operator_control_signature_invalid".to_string())?;
    }
    Ok(bytes)
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn validate_internal_control_caller(
    request: &OperatorControlRequest,
    authenticated_controller: bool,
) -> Result<(), String> {
    if request.actor == "mesh-drain-propagator" && !authenticated_controller {
        Err("operator_internal_control_requires_controller_identity".to_string())
    } else {
        Ok(())
    }
}

fn apply_operator_control(
    request: &OperatorControlRequest,
    authenticated_controller: bool,
) -> Result<OperatorControlOutcome, String> {
    if request.schema_version != 1
        || request.cluster_id.is_empty()
        || request.cluster_id.len() > 128
        || request.actor.is_empty()
        || request.actor.len() > 256
        || request.sequence == 0
        || request.reason.trim().is_empty()
        || request.reason.len() > 2_048
    {
        return Err("operator_control_request_invalid".to_string());
    }
    let expected_cluster = std::env::var("MESH_CLUSTER_ID").unwrap_or_else(|_| "mesh".to_string());
    if request.cluster_id != expected_cluster {
        return Err("operator_control_cluster_mismatch".to_string());
    }
    let now = unix_millis();
    if request.expires_at_unix_millis < now
        || request.expires_at_unix_millis > now.saturating_add(300_000)
    {
        return Err("operator_control_expired_or_too_far_future".to_string());
    }
    let key = std::env::var("MESH_OPERATOR_KEY")
        .map_err(|_| "operator_control_not_configured".to_string())?;
    let signature = decode_hex_signature(&request.signature)?;
    let payload = control_signature_payload(request)?;
    let authorized = operator_control_signature_matches(&key, &payload, &signature);
    if !authorized {
        return Err("operator_control_unauthorized".to_string());
    }

    refresh_operator_control_from_consensus();
    let mut state = operator_control_state().lock();
    let mut drain_to_prepare = None;
    let mut drain_to_cancel = None;
    if state
        .actor_sequences
        .get(&request.actor)
        .is_some_and(|sequence| request.sequence <= *sequence)
    {
        return Err("operator_control_replay_rejected".to_string());
    }
    if request.actor == "mesh-drain-propagator" {
        validate_internal_control_caller(request, authenticated_controller)?;
        let (node_id, draining) = match &request.action {
            OperatorControlAction::DrainNode { node_id } => (node_id, true),
            OperatorControlAction::CancelDrain { node_id } => (node_id, false),
            _ => return Err("operator_internal_control_action_invalid".to_string()),
        };
        let runtime_node_id = super::node::resolve_runtime_node_id(node_id)?;
        if node_state().is_none_or(|local| local.name != runtime_node_id) {
            return Err("operator_internal_control_target_mismatch".to_string());
        }
        if draining {
            state.drain_intents.insert(runtime_node_id.clone());
        } else {
            state.drain_intents.remove(&runtime_node_id);
        }
        state
            .actor_sequences
            .insert(request.actor.clone(), request.sequence);
        state.control_sequence = state.control_sequence.saturating_add(1);
        let outcome = OperatorControlOutcome {
            schema_version: 1,
            accepted: true,
            control_sequence: state.control_sequence,
            autoscaler_paused: state.autoscaler_paused,
            desired_capacity_override: state.desired_capacity_override,
            drain_intents: state.drain_intents.iter().cloned().collect(),
            consensus: None,
        };
        drop(state);
        super::telemetry::global_admission_controller().set_draining(draining);
        audit_operator_control(request, &outcome)?;
        return Ok(outcome);
    }
    let (command_id, mutation) = match &request.action {
        OperatorControlAction::PauseAutoscaler => (
            format!(
                "operator:{}:{}:{}",
                request.cluster_id, request.actor, request.sequence
            ),
            super::scaling::ControlMutation::PauseAutoscaler { paused: true },
        ),
        OperatorControlAction::ResumeAutoscaler => (
            format!(
                "operator:{}:{}:{}",
                request.cluster_id, request.actor, request.sequence
            ),
            super::scaling::ControlMutation::PauseAutoscaler { paused: false },
        ),
        OperatorControlAction::SetDesiredCapacity { worker_nodes } => {
            if *worker_nodes == 0 {
                return Err("operator_control_desired_capacity_invalid".to_string());
            }
            (
                format!(
                    "operator:{}:{}:{}",
                    request.cluster_id, request.actor, request.sequence
                ),
                super::scaling::ControlMutation::ManualOverride {
                    worker_nodes: *worker_nodes,
                },
            )
        }
        OperatorControlAction::DrainNode { node_id } => {
            if node_id.trim().is_empty() {
                return Err("operator_control_drain_node_invalid".to_string());
            }
            let runtime_node_id =
                super::node::resolve_runtime_node_id(node_id).unwrap_or_else(|_| node_id.clone());
            (
                format!(
                    "operator:{}:{}:{}",
                    request.cluster_id, request.actor, request.sequence
                ),
                super::scaling::ControlMutation::DrainIntent {
                    node_id: runtime_node_id,
                    cancelled: false,
                },
            )
        }
        OperatorControlAction::CancelDrain { node_id } => {
            if node_id.trim().is_empty() {
                return Err("operator_control_drain_node_invalid".to_string());
            }
            let runtime_node_id =
                super::node::resolve_runtime_node_id(node_id).unwrap_or_else(|_| node_id.clone());
            (
                format!(
                    "operator:{}:{}:{}",
                    request.cluster_id, request.actor, request.sequence
                ),
                super::scaling::ControlMutation::DrainIntent {
                    node_id: runtime_node_id,
                    cancelled: true,
                },
            )
        }
        OperatorControlAction::CommitControlMutation {
            command_id,
            mutation,
        } => (command_id.clone(), mutation.clone()),
    };
    validate_control_mutation(&mutation)?;
    let response = super::consensus::commit_consensus_command(
        super::consensus::ConsensusCommand {
            command_id,
            actor: request.actor.clone(),
            reason: request.reason.clone(),
            timestamp_unix_millis: now,
            actor_sequence: request.sequence,
            mutation: mutation.clone(),
        },
        Duration::from_secs(10),
    )?;
    apply_committed_control_mutation(&mut state, &mutation)?;
    if let super::scaling::ControlMutation::DrainIntent { node_id, cancelled } = &mutation {
        if *cancelled {
            drain_to_cancel = Some(node_id.clone());
        } else {
            drain_to_prepare = Some(node_id.clone());
        }
    }
    state.last_consensus_log_index = state.last_consensus_log_index.max(response.log_index);
    state
        .actor_sequences
        .insert(request.actor.clone(), request.sequence);
    state.control_sequence = state.control_sequence.saturating_add(1);
    let outcome = OperatorControlOutcome {
        schema_version: 1,
        accepted: true,
        control_sequence: state.control_sequence,
        autoscaler_paused: state.autoscaler_paused,
        desired_capacity_override: state.desired_capacity_override,
        drain_intents: state.drain_intents.iter().cloned().collect(),
        consensus: Some(response),
    };
    drop(state);
    if let Some(node_id) = drain_to_prepare.as_deref() {
        prepare_committed_drain(node_id);
    }
    if let Some(node_id) = drain_to_cancel.as_deref() {
        set_runtime_drain_intent(node_id, false);
    }
    audit_operator_control(request, &outcome)?;
    Ok(outcome)
}

fn apply_committed_control_mutation(
    state: &mut OperatorControlState,
    mutation: &super::scaling::ControlMutation,
) -> Result<(), String> {
    match mutation {
        super::scaling::ControlMutation::DesiredCapacity(desired) => {
            if desired.worker_nodes == 0 || desired.revision.0 == 0 {
                return Err("operator_control_desired_capacity_invalid".to_string());
            }
            state.desired_capacity_override = Some(desired.worker_nodes);
        }
        super::scaling::ControlMutation::ManualOverride { worker_nodes } => {
            if *worker_nodes == 0 {
                return Err("operator_control_desired_capacity_invalid".to_string());
            }
            state.desired_capacity_override = Some(*worker_nodes);
        }
        super::scaling::ControlMutation::PauseAutoscaler { paused } => {
            state.autoscaler_paused = *paused;
        }
        super::scaling::ControlMutation::DrainIntent { node_id, cancelled } => {
            if node_id.trim().is_empty() {
                return Err("operator_control_drain_node_invalid".to_string());
            }
            if *cancelled {
                state.drain_intents.remove(node_id);
                if let Ok(runtime_node_id) = super::node::resolve_runtime_node_id(node_id) {
                    state.drain_intents.remove(&runtime_node_id);
                }
            } else {
                state.drain_intents.insert(
                    super::node::resolve_runtime_node_id(node_id)
                        .unwrap_or_else(|_| node_id.clone()),
                );
            }
        }
        super::scaling::ControlMutation::DriverOperation(_)
        | super::scaling::ControlMutation::PolicyRevision { .. }
        | super::scaling::ControlMutation::MembershipIntent { .. } => {}
    }
    Ok(())
}

fn validate_control_mutation(mutation: &super::scaling::ControlMutation) -> Result<(), String> {
    match mutation {
        super::scaling::ControlMutation::DesiredCapacity(desired)
            if desired.worker_nodes == 0
                || desired.revision.0 == 0
                || desired.template_revision.trim().is_empty() =>
        {
            Err("operator_control_desired_capacity_invalid".to_string())
        }
        super::scaling::ControlMutation::ManualOverride { worker_nodes } if *worker_nodes == 0 => {
            Err("operator_control_desired_capacity_invalid".to_string())
        }
        super::scaling::ControlMutation::DrainIntent { node_id, .. }
            if node_id.trim().is_empty() =>
        {
            Err("operator_control_drain_node_invalid".to_string())
        }
        super::scaling::ControlMutation::PolicyRevision {
            revision,
            policy_json,
            policy_sha256,
        } if *revision == 0 || policy_json.trim().is_empty() || policy_sha256.len() != 64 => {
            Err("operator_control_policy_revision_invalid".to_string())
        }
        super::scaling::ControlMutation::MembershipIntent { generation, nodes }
            if *generation == 0
                || nodes.is_empty()
                || nodes.iter().any(|node| node.trim().is_empty()) =>
        {
            Err("operator_control_membership_intent_invalid".to_string())
        }
        _ => Ok(()),
    }
}

fn refresh_operator_control_from_consensus() {
    let Some(snapshot) = super::consensus::consensus_runtime_snapshot() else {
        return;
    };
    let mut state = operator_control_state().lock();
    for entry in &snapshot.entries {
        if entry.index <= state.last_consensus_log_index {
            continue;
        }
        if apply_committed_control_mutation(&mut state, &entry.mutation).is_ok() {
            if entry.actor_sequence > 0 {
                state
                    .actor_sequences
                    .entry(entry.actor.clone())
                    .and_modify(|sequence| *sequence = (*sequence).max(entry.actor_sequence))
                    .or_insert(entry.actor_sequence);
            }
            state.last_consensus_log_index = entry.index;
            state.control_sequence = state.control_sequence.saturating_add(1);
        }
    }
}

fn audit_operator_control(
    request: &OperatorControlRequest,
    outcome: &OperatorControlOutcome,
) -> Result<(), String> {
    let action = serde_json::to_string(&request.action)
        .map_err(|error| format!("operator_audit_encode_failed:{error}"))?;
    record_diagnostic(OperatorDiagnosticRecord {
        transition: "operator_control_committed".to_string(),
        reason: Some(request.reason.clone()),
        metadata: vec![
            ("actor".to_string(), request.actor.clone()),
            ("action".to_string(), action.clone()),
            ("sequence".to_string(), request.sequence.to_string()),
            (
                "control_sequence".to_string(),
                outcome.control_sequence.to_string(),
            ),
        ],
        ..OperatorDiagnosticRecord::default()
    });
    append_operator_audit_entry(&serde_json::json!({
        "schema_version": 1,
        "timestamp_unix_millis": unix_millis(),
        "cluster_id": request.cluster_id,
        "actor": request.actor,
        "sequence": request.sequence,
        "reason": request.reason,
        "action": request.action,
        "outcome": "committed",
        "control_sequence": outcome.control_sequence,
    }))
}

fn operator_action_name(action: &OperatorControlAction) -> &'static str {
    match action {
        OperatorControlAction::PauseAutoscaler => "pause_autoscaler",
        OperatorControlAction::ResumeAutoscaler => "resume_autoscaler",
        OperatorControlAction::SetDesiredCapacity { .. } => "set_desired_capacity",
        OperatorControlAction::DrainNode { .. } => "drain_node",
        OperatorControlAction::CancelDrain { .. } => "cancel_drain",
        OperatorControlAction::CommitControlMutation { .. } => "commit_control_mutation",
    }
}

fn bounded_audit_value(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn audit_operator_control_rejection(request: &OperatorControlRequest, rejection: &str) {
    let actor = bounded_audit_value(&request.actor, 256);
    let rejection = bounded_audit_value(rejection, 512);
    let action = operator_action_name(&request.action);
    record_diagnostic(OperatorDiagnosticRecord {
        transition: "operator_control_rejected".to_string(),
        reason: Some(rejection.clone()),
        metadata: vec![
            ("actor".to_string(), actor.clone()),
            ("action".to_string(), action.to_string()),
            ("sequence".to_string(), request.sequence.to_string()),
        ],
        ..OperatorDiagnosticRecord::default()
    });
    if let Err(error) = append_operator_audit_entry(&serde_json::json!({
        "schema_version": 1,
        "timestamp_unix_millis": unix_millis(),
        "cluster_id": bounded_audit_value(&request.cluster_id, 128),
        "actor": actor,
        "sequence": request.sequence,
        "action": action,
        "outcome": "rejected",
        "rejection": rejection,
    })) {
        record_diagnostic(OperatorDiagnosticRecord {
            transition: "operator_control_rejection_audit_failed".to_string(),
            reason: Some(error),
            ..OperatorDiagnosticRecord::default()
        });
    }
}

fn append_operator_audit_entry(entry: &serde_json::Value) -> Result<(), String> {
    let Some(path) = std::env::var("MESH_OPERATOR_AUDIT_LOG")
        .ok()
        .filter(|path| !path.trim().is_empty())
    else {
        return Ok(());
    };
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("operator_audit_directory_failed:{error}"))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("operator_audit_open_failed:{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("operator_audit_permissions_failed:{error}"))?;
    }
    writeln!(file, "{entry}").map_err(|error| format!("operator_audit_write_failed:{error}"))?;
    file.sync_data()
        .map_err(|error| format!("operator_audit_sync_failed:{error}"))
}

fn execute_transient_query(
    target: &str,
    cookie: &str,
    query: OperatorQuery,
    timeout: Duration,
) -> Result<OperatorReply, OperatorQueryError> {
    let query_kind = query.kind();
    let payload = encode_query_frame(
        OPERATOR_QUERY_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
        &query,
    )
    .map_err(|reason| OperatorQueryError::InvalidRequest {
        query: query_kind,
        reason,
    })?;
    let reply =
        execute_transient_operator_query(target, cookie, &payload, timeout).map_err(|reason| {
            OperatorQueryError::TargetUnavailable {
                target: target.to_string(),
                query: query_kind,
                reason,
            }
        })?;
    let (_request_id, result) =
        decode_query_reply_frame(&reply).map_err(|reason| OperatorQueryError::Decode {
            target: target.to_string(),
            query: query_kind,
            reason,
        })?;
    match result {
        Ok(bytes) => decode_query_reply_payload(query_kind, &bytes).map_err(|reason| {
            OperatorQueryError::Decode {
                target: target.to_string(),
                query: query_kind,
                reason,
            }
        }),
        Err(reason) => Err(OperatorQueryError::RemoteRejected {
            target: target.to_string(),
            query: query_kind,
            reason,
        }),
    }
}

pub fn query_operator_status_remote(
    target: &str,
    cookie: &str,
    timeout: Duration,
) -> Result<OperatorStatusSnapshot, OperatorQueryError> {
    match execute_transient_query(target, cookie, OperatorQuery::Status, timeout)? {
        OperatorReply::Status(snapshot) => Ok(snapshot),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Status,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_runtime_remote(
    target: &str,
    cookie: &str,
    timeout: Duration,
) -> Result<OperatorRuntimeSnapshot, OperatorQueryError> {
    match execute_transient_query(target, cookie, OperatorQuery::Runtime, timeout)? {
        OperatorReply::Runtime(snapshot) => Ok(snapshot),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Runtime,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_control_remote(
    target: &str,
    cookie: &str,
    request: OperatorControlRequest,
    timeout: Duration,
) -> Result<OperatorControlOutcome, OperatorQueryError> {
    match execute_transient_query(target, cookie, OperatorQuery::Control(request), timeout)? {
        OperatorReply::Control(outcome) => Ok(outcome),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Control,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_continuity_status_remote(
    target: &str,
    cookie: &str,
    request_key: &str,
    timeout: Duration,
) -> Result<ContinuityRecord, OperatorQueryError> {
    if request_key.is_empty() {
        return Err(OperatorQueryError::InvalidRequest {
            query: OperatorQueryKind::ContinuityLookup,
            reason: "request_key_missing".to_string(),
        });
    }
    match execute_transient_query(
        target,
        cookie,
        OperatorQuery::ContinuityLookup {
            request_key: request_key.to_string(),
        },
        timeout,
    )? {
        OperatorReply::ContinuityRecord(record) => Ok(record),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::ContinuityLookup,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_continuity_list_remote(
    target: &str,
    cookie: &str,
    limit: Option<usize>,
    timeout: Duration,
) -> Result<OperatorContinuityList, OperatorQueryError> {
    match execute_transient_query(
        target,
        cookie,
        OperatorQuery::ContinuityList { limit },
        timeout,
    )? {
        OperatorReply::ContinuityList(list) => Ok(list),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::ContinuityList,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_diagnostics_remote(
    target: &str,
    cookie: &str,
    limit: Option<usize>,
    timeout: Duration,
) -> Result<OperatorDiagnosticsSnapshot, OperatorQueryError> {
    match execute_transient_query(
        target,
        cookie,
        OperatorQuery::Diagnostics { limit },
        timeout,
    )? {
        OperatorReply::Diagnostics(snapshot) => Ok(snapshot),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Diagnostics,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_status(
    target: &str,
    timeout: Duration,
) -> Result<OperatorStatusSnapshot, OperatorQueryError> {
    match execute_query(target, OperatorQuery::Status, timeout)? {
        OperatorReply::Status(snapshot) => Ok(snapshot),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Status,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_runtime(
    target: &str,
    timeout: Duration,
) -> Result<OperatorRuntimeSnapshot, OperatorQueryError> {
    match execute_query(target, OperatorQuery::Runtime, timeout)? {
        OperatorReply::Runtime(snapshot) => Ok(snapshot),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Runtime,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_control(
    target: &str,
    request: OperatorControlRequest,
    timeout: Duration,
) -> Result<OperatorControlOutcome, OperatorQueryError> {
    match execute_query(target, OperatorQuery::Control(request), timeout)? {
        OperatorReply::Control(outcome) => Ok(outcome),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Control,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_continuity_status(
    target: &str,
    request_key: &str,
    timeout: Duration,
) -> Result<ContinuityRecord, OperatorQueryError> {
    if request_key.is_empty() {
        return Err(OperatorQueryError::InvalidRequest {
            query: OperatorQueryKind::ContinuityLookup,
            reason: "request_key_missing".to_string(),
        });
    }
    match execute_query(
        target,
        OperatorQuery::ContinuityLookup {
            request_key: request_key.to_string(),
        },
        timeout,
    )? {
        OperatorReply::ContinuityRecord(record) => Ok(record),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::ContinuityLookup,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_continuity_list(
    target: &str,
    limit: Option<usize>,
    timeout: Duration,
) -> Result<OperatorContinuityList, OperatorQueryError> {
    match execute_query(target, OperatorQuery::ContinuityList { limit }, timeout)? {
        OperatorReply::ContinuityList(list) => Ok(list),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::ContinuityList,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub fn query_operator_diagnostics(
    target: &str,
    limit: Option<usize>,
    timeout: Duration,
) -> Result<OperatorDiagnosticsSnapshot, OperatorQueryError> {
    match execute_query(target, OperatorQuery::Diagnostics { limit }, timeout)? {
        OperatorReply::Diagnostics(snapshot) => Ok(snapshot),
        _ => Err(OperatorQueryError::Decode {
            target: target.to_string(),
            query: OperatorQueryKind::Diagnostics,
            reason: "operator reply kind mismatch".to_string(),
        }),
    }
}

pub(crate) fn handle_operator_query_message(session: &Arc<NodeSession>, msg: &[u8]) {
    let authenticated_controller = session.negotiated_protocol.autonomous_enabled
        && session
            .remote_identity
            .as_ref()
            .is_some_and(|identity| identity.roles.iter().any(|role| role == "controller"));
    match build_query_reply_frame(
        msg,
        continuity_registry(),
        diagnostics_buffer(),
        authenticated_controller,
    ) {
        Ok(reply) => {
            if let Err(error) = session.send(super::node::OutboundClass::Control, reply) {
                eprintln!(
                    "mesh operator query: remote={} error=reply_write_failed:{}",
                    session.remote_name, error
                );
            }
        }
        Err(error) => {
            eprintln!(
                "mesh operator query: remote={} error={}",
                session.remote_name, error
            );
        }
    }
}

pub(crate) fn handle_operator_reply_message(session: &Arc<NodeSession>, msg: &[u8]) {
    match decode_query_reply_frame(msg) {
        Ok((request_id, result)) => {
            if let Some(sender) = session
                .pending_operator_queries
                .lock()
                .unwrap()
                .remove(&request_id)
            {
                let _ = sender.send(result);
            }
        }
        Err(error) => {
            eprintln!(
                "mesh operator query: remote={} error=reply_malformed:{}",
                session.remote_name, error
            );
        }
    }
}

fn execute_query(
    target: &str,
    query: OperatorQuery,
    timeout: Duration,
) -> Result<OperatorReply, OperatorQueryError> {
    let query_kind = query.kind();
    let state = node_state().ok_or_else(|| OperatorQueryError::TargetUnavailable {
        target: target.to_string(),
        query: query_kind,
        reason: "node_not_started".to_string(),
    })?;

    if state.name == target {
        return execute_local_query(
            Some(&state.name),
            &peer_names(state),
            continuity_registry(),
            diagnostics_buffer(),
            query,
            false,
        )
        .map_err(|reason| OperatorQueryError::LocalRejected {
            query: query_kind,
            reason,
        });
    }

    let session = {
        let sessions = state.sessions.read();
        sessions.get(target).cloned()
    }
    .ok_or_else(|| OperatorQueryError::TargetUnavailable {
        target: target.to_string(),
        query: query_kind,
        reason: "target_not_connected".to_string(),
    })?;

    let request_id = OPERATOR_QUERY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let payload = encode_query_frame(request_id, &query).map_err(|reason| {
        OperatorQueryError::InvalidRequest {
            query: query_kind,
            reason,
        }
    })?;
    let (tx, rx) = mpsc::channel();
    session
        .pending_operator_queries
        .lock()
        .unwrap()
        .insert(request_id, tx);

    {
        if let Err(error) = session.send(super::node::OutboundClass::Control, payload) {
            session
                .pending_operator_queries
                .lock()
                .unwrap()
                .remove(&request_id);
            return Err(OperatorQueryError::TargetUnavailable {
                target: target.to_string(),
                query: query_kind,
                reason: format!("query_write_failed:{error}"),
            });
        }
    }

    match rx.recv_timeout(timeout) {
        Ok(Ok(bytes)) => decode_query_reply_payload(query_kind, &bytes).map_err(|reason| {
            OperatorQueryError::Decode {
                target: target.to_string(),
                query: query_kind,
                reason,
            }
        }),
        Ok(Err(reason)) => Err(OperatorQueryError::RemoteRejected {
            target: target.to_string(),
            query: query_kind,
            reason,
        }),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            session
                .pending_operator_queries
                .lock()
                .unwrap()
                .remove(&request_id);
            Err(OperatorQueryError::Timeout {
                target: target.to_string(),
                query: query_kind,
                timeout,
            })
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            session
                .pending_operator_queries
                .lock()
                .unwrap()
                .remove(&request_id);
            Err(OperatorQueryError::TargetUnavailable {
                target: target.to_string(),
                query: query_kind,
                reason: "query_disconnected".to_string(),
            })
        }
    }
}

fn peer_names(state: &super::node::NodeState) -> Vec<String> {
    state.sessions.read().keys().cloned().collect()
}

fn normalized_membership(local_node: &str, peer_nodes: &[String]) -> OperatorMembershipSnapshot {
    let mut peer_nodes: Vec<String> = peer_nodes
        .iter()
        .filter(|peer| peer.as_str() != local_node)
        .cloned()
        .collect();
    peer_nodes.sort();
    peer_nodes.dedup();

    let mut nodes = Vec::with_capacity(peer_nodes.len() + 1);
    nodes.push(local_node.to_string());
    nodes.extend(peer_nodes.iter().cloned());
    nodes.sort();
    nodes.dedup();

    OperatorMembershipSnapshot {
        local_node: local_node.to_string(),
        peer_nodes,
        nodes,
    }
}

fn status_snapshot_from_parts(
    local_node: &str,
    peer_nodes: &[String],
    authority: ContinuityAuthorityStatus,
) -> OperatorStatusSnapshot {
    OperatorStatusSnapshot {
        membership: normalized_membership(local_node, peer_nodes),
        authority: authority.into(),
    }
}

fn continuity_list_from_registry(
    registry: &ContinuityRegistry,
    limit: Option<usize>,
) -> OperatorContinuityList {
    let mut records = registry.snapshot().records;
    records.sort_by(|left, right| {
        left.request_key
            .cmp(&right.request_key)
            .then_with(|| left.attempt_id.cmp(&right.attempt_id))
    });
    let total_records = records.len();
    let take = limit
        .unwrap_or(MAX_CONTINUITY_LIST_RECORDS)
        .min(MAX_CONTINUITY_LIST_RECORDS)
        .min(total_records);
    let truncated = take < total_records;
    records.truncate(take);
    OperatorContinuityList {
        records,
        total_records,
        truncated,
    }
}

fn execute_local_query(
    local_node: Option<&str>,
    peer_nodes: &[String],
    registry: &ContinuityRegistry,
    diagnostics: &OperatorDiagnosticsBuffer,
    query: OperatorQuery,
    authenticated_controller: bool,
) -> Result<OperatorReply, String> {
    match query {
        OperatorQuery::Status => {
            let local_node = local_node.ok_or_else(|| "node_not_started".to_string())?;
            Ok(OperatorReply::Status(status_snapshot_from_parts(
                local_node,
                peer_nodes,
                registry.authority_status(),
            )))
        }
        OperatorQuery::ContinuityLookup { request_key } => {
            if request_key.is_empty() {
                return Err("request_key_missing".to_string());
            }
            registry
                .record(&request_key)
                .map(OperatorReply::ContinuityRecord)
                .ok_or_else(|| "request_key_not_found".to_string())
        }
        OperatorQuery::ContinuityList { limit } => Ok(OperatorReply::ContinuityList(
            continuity_list_from_registry(registry, limit),
        )),
        OperatorQuery::Diagnostics { limit } => {
            Ok(OperatorReply::Diagnostics(diagnostics.snapshot(limit)))
        }
        OperatorQuery::Runtime => {
            let state = node_state().ok_or_else(|| "node_not_started".to_string())?;
            Ok(OperatorReply::Runtime(runtime_snapshot_from_state(state)))
        }
        OperatorQuery::Control(request) => {
            match apply_operator_control(&request, authenticated_controller) {
                Ok(outcome) => Ok(OperatorReply::Control(outcome)),
                Err(reason) => {
                    audit_operator_control_rejection(&request, &reason);
                    Err(reason)
                }
            }
        }
    }
}

fn encode_query_frame(request_id: u64, query: &OperatorQuery) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    match query {
        OperatorQuery::Status | OperatorQuery::Runtime => {}
        OperatorQuery::Control(request) => {
            payload = serde_json::to_vec(request)
                .map_err(|error| format!("operator control encode failed: {error}"))?;
        }
        OperatorQuery::ContinuityLookup { request_key } => {
            encode_string(&mut payload, request_key)?;
        }
        OperatorQuery::ContinuityList { limit } | OperatorQuery::Diagnostics { limit } => {
            encode_optional_limit(&mut payload, *limit)?;
        }
    }

    let mut frame = Vec::with_capacity(1 + 8 + 1 + 4 + payload.len());
    frame.push(DIST_OPERATOR_QUERY);
    frame.extend_from_slice(&request_id.to_le_bytes());
    frame.push(query.kind().to_wire());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_query_header(data: &[u8]) -> Result<(u64, u8, &[u8]), String> {
    if data.len() < 14 {
        return Err("operator query payload too short".to_string());
    }
    if data[0] != DIST_OPERATOR_QUERY {
        return Err(format!("operator query tag mismatch {}", data[0]));
    }
    let request_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let kind = data[9];
    let payload_len = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    if data.len() != 14 + payload_len {
        return Err("operator query payload length mismatch".to_string());
    }
    Ok((request_id, kind, &data[14..]))
}

fn decode_query(kind: OperatorQueryKind, payload: &[u8]) -> Result<OperatorQuery, String> {
    let mut pos = 0;
    let query = match kind {
        OperatorQueryKind::Status => OperatorQuery::Status,
        OperatorQueryKind::ContinuityLookup => OperatorQuery::ContinuityLookup {
            request_key: decode_string(payload, &mut pos)?,
        },
        OperatorQueryKind::ContinuityList => OperatorQuery::ContinuityList {
            limit: decode_optional_limit(payload, &mut pos)?,
        },
        OperatorQueryKind::Diagnostics => OperatorQuery::Diagnostics {
            limit: decode_optional_limit(payload, &mut pos)?,
        },
        OperatorQueryKind::Runtime => OperatorQuery::Runtime,
        OperatorQueryKind::Control => {
            let request = serde_json::from_slice(payload)
                .map_err(|error| format!("operator control decode failed: {error}"))?;
            pos = payload.len();
            OperatorQuery::Control(request)
        }
    };
    if pos != payload.len() {
        return Err("operator query payload trailing bytes".to_string());
    }
    Ok(query)
}

fn encode_query_reply_frame(
    request_id: u64,
    result: Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let (status, payload) = match result {
        Ok(payload) => (REPLY_STATUS_OK, payload),
        Err(reason) => {
            let mut payload = Vec::new();
            encode_string(&mut payload, &reason)?;
            (REPLY_STATUS_ERR, payload)
        }
    };

    let mut frame = Vec::with_capacity(1 + 8 + 1 + 4 + payload.len());
    frame.push(DIST_OPERATOR_REPLY);
    frame.extend_from_slice(&request_id.to_le_bytes());
    frame.push(status);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_query_reply_frame(data: &[u8]) -> Result<(u64, Result<Vec<u8>, String>), String> {
    if data.len() < 14 {
        return Err("operator reply payload too short".to_string());
    }
    if data[0] != DIST_OPERATOR_REPLY {
        return Err(format!("operator reply tag mismatch {}", data[0]));
    }
    let request_id = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let status = data[9];
    let payload_len = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    if data.len() != 14 + payload_len {
        return Err("operator reply payload length mismatch".to_string());
    }
    let payload = &data[14..];
    match status {
        REPLY_STATUS_OK => Ok((request_id, Ok(payload.to_vec()))),
        REPLY_STATUS_ERR => {
            let mut pos = 0;
            let reason = decode_string(payload, &mut pos)?;
            if pos != payload.len() {
                return Err("operator reply error payload trailing bytes".to_string());
            }
            Ok((request_id, Err(reason)))
        }
        other => Err(format!("invalid operator reply status {other}")),
    }
}

fn build_query_reply_frame(
    msg: &[u8],
    registry: &ContinuityRegistry,
    diagnostics: &OperatorDiagnosticsBuffer,
    authenticated_controller: bool,
) -> Result<Vec<u8>, String> {
    let (request_id, kind, payload) = decode_query_header(msg)?;
    let kind = match OperatorQueryKind::from_wire(kind) {
        Ok(kind) => kind,
        Err(reason) => {
            return encode_query_reply_frame(request_id, Err(reason));
        }
    };
    let query = match decode_query(kind, payload) {
        Ok(query) => query,
        Err(reason) => {
            return encode_query_reply_frame(request_id, Err(reason));
        }
    };

    let local_node = node_state().map(|state| state.name.clone());
    let peer_nodes = node_state().map(peer_names).unwrap_or_default();
    let result = execute_local_query(
        local_node.as_deref(),
        &peer_nodes,
        registry,
        diagnostics,
        query,
        authenticated_controller,
    )
    .and_then(|reply| encode_query_reply_payload(&reply));

    encode_query_reply_frame(request_id, result)
}

fn encode_query_reply_payload(reply: &OperatorReply) -> Result<Vec<u8>, String> {
    match reply {
        OperatorReply::Status(snapshot) => encode_status_snapshot(snapshot),
        OperatorReply::ContinuityRecord(record) => encode_record_payload(record),
        OperatorReply::ContinuityList(list) => encode_continuity_list(list),
        OperatorReply::Diagnostics(snapshot) => encode_diagnostics_snapshot(snapshot),
        OperatorReply::Runtime(snapshot) => serde_json::to_vec(snapshot)
            .map_err(|error| format!("operator runtime encode failed: {error}")),
        OperatorReply::Control(outcome) => serde_json::to_vec(outcome)
            .map_err(|error| format!("operator control outcome encode failed: {error}")),
    }
}

fn decode_query_reply_payload(
    kind: OperatorQueryKind,
    payload: &[u8],
) -> Result<OperatorReply, String> {
    match kind {
        OperatorQueryKind::Status => decode_status_snapshot(payload).map(OperatorReply::Status),
        OperatorQueryKind::ContinuityLookup => {
            decode_record_payload(payload).map(OperatorReply::ContinuityRecord)
        }
        OperatorQueryKind::ContinuityList => {
            decode_continuity_list(payload).map(OperatorReply::ContinuityList)
        }
        OperatorQueryKind::Diagnostics => {
            decode_diagnostics_snapshot(payload).map(OperatorReply::Diagnostics)
        }
        OperatorQueryKind::Runtime => serde_json::from_slice(payload)
            .map(OperatorReply::Runtime)
            .map_err(|error| format!("operator runtime decode failed: {error}")),
        OperatorQueryKind::Control => serde_json::from_slice(payload)
            .map(OperatorReply::Control)
            .map_err(|error| format!("operator control outcome decode failed: {error}")),
    }
}

fn encode_status_snapshot(snapshot: &OperatorStatusSnapshot) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    encode_string(&mut payload, &snapshot.membership.local_node)?;
    encode_string_list(&mut payload, &snapshot.membership.peer_nodes)?;
    encode_string(&mut payload, &snapshot.authority.cluster_role)?;
    payload.extend_from_slice(&snapshot.authority.promotion_epoch.to_le_bytes());
    encode_string(&mut payload, &snapshot.authority.replication_health)?;
    Ok(payload)
}

fn decode_status_snapshot(payload: &[u8]) -> Result<OperatorStatusSnapshot, String> {
    let mut pos = 0;
    let local_node = decode_string(payload, &mut pos)?;
    let peer_nodes = decode_string_list(payload, &mut pos)?;
    let cluster_role = decode_string(payload, &mut pos)?;
    let promotion_epoch = decode_u64(payload, &mut pos)?;
    let replication_health = decode_string(payload, &mut pos)?;
    if pos != payload.len() {
        return Err("operator status payload trailing bytes".to_string());
    }
    Ok(OperatorStatusSnapshot {
        membership: normalized_membership(&local_node, &peer_nodes),
        authority: OperatorAuthoritySnapshot {
            cluster_role,
            promotion_epoch,
            replication_health,
        },
    })
}

fn encode_continuity_list(list: &OperatorContinuityList) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    payload.extend_from_slice(
        &u32_from_usize(list.total_records, "operator continuity total records")?.to_le_bytes(),
    );
    payload.push(list.truncated as u8);
    payload.extend_from_slice(
        &u32_from_usize(list.records.len(), "operator continuity record count")?.to_le_bytes(),
    );
    for record in &list.records {
        let encoded = encode_record_payload(record)?;
        payload.extend_from_slice(
            &u32_from_usize(encoded.len(), "operator continuity record payload")?.to_le_bytes(),
        );
        payload.extend_from_slice(&encoded);
    }
    Ok(payload)
}

fn decode_continuity_list(payload: &[u8]) -> Result<OperatorContinuityList, String> {
    let mut pos = 0;
    let total_records = decode_u32(payload, &mut pos)? as usize;
    let truncated = decode_bool(payload, &mut pos)?;
    let count = decode_u32(payload, &mut pos)? as usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let record_len = decode_u32(payload, &mut pos)? as usize;
        if pos + record_len > payload.len() {
            return Err("operator continuity record payload truncated".to_string());
        }
        let record = decode_record_payload(&payload[pos..pos + record_len])?;
        pos += record_len;
        records.push(record);
    }
    if pos != payload.len() {
        return Err("operator continuity payload trailing bytes".to_string());
    }
    Ok(OperatorContinuityList {
        records,
        total_records,
        truncated,
    })
}

fn encode_diagnostics_snapshot(snapshot: &OperatorDiagnosticsSnapshot) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    payload.extend_from_slice(
        &u32_from_usize(snapshot.total_entries, "operator diagnostics total entries")?
            .to_le_bytes(),
    );
    payload.extend_from_slice(&snapshot.dropped_entries.to_le_bytes());
    payload.extend_from_slice(
        &u32_from_usize(
            snapshot.buffer_capacity,
            "operator diagnostics buffer capacity",
        )?
        .to_le_bytes(),
    );
    payload.push(snapshot.truncated as u8);
    payload.extend_from_slice(
        &u32_from_usize(snapshot.entries.len(), "operator diagnostics entry count")?.to_le_bytes(),
    );
    for entry in &snapshot.entries {
        encode_diagnostic_entry(&mut payload, entry)?;
    }
    Ok(payload)
}

fn decode_diagnostics_snapshot(payload: &[u8]) -> Result<OperatorDiagnosticsSnapshot, String> {
    let mut pos = 0;
    let total_entries = decode_u32(payload, &mut pos)? as usize;
    let dropped_entries = decode_u64(payload, &mut pos)?;
    let buffer_capacity = decode_u32(payload, &mut pos)? as usize;
    let truncated = decode_bool(payload, &mut pos)?;
    let count = decode_u32(payload, &mut pos)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(decode_diagnostic_entry(payload, &mut pos)?);
    }
    if pos != payload.len() {
        return Err("operator diagnostics payload trailing bytes".to_string());
    }
    Ok(OperatorDiagnosticsSnapshot {
        entries,
        total_entries,
        dropped_entries,
        buffer_capacity,
        truncated,
    })
}

fn encode_diagnostic_entry(
    payload: &mut Vec<u8>,
    entry: &OperatorDiagnosticEntry,
) -> Result<(), String> {
    payload.extend_from_slice(&entry.sequence.to_le_bytes());
    encode_string(payload, &entry.transition)?;
    encode_optional_string(payload, entry.request_key.as_deref())?;
    encode_optional_string(payload, entry.attempt_id.as_deref())?;
    encode_optional_string(payload, entry.owner_node.as_deref())?;
    encode_optional_string(payload, entry.replica_node.as_deref())?;
    encode_optional_string(payload, entry.execution_node.as_deref())?;
    encode_optional_string(payload, entry.cluster_role.as_deref())?;
    encode_optional_u64(payload, entry.promotion_epoch)?;
    encode_optional_string(payload, entry.replication_health.as_deref())?;
    encode_optional_string(payload, entry.replica_status.as_deref())?;
    encode_optional_string(payload, entry.reason.as_deref())?;
    payload.extend_from_slice(
        &u32_from_usize(entry.metadata.len(), "operator diagnostic metadata entries")?
            .to_le_bytes(),
    );
    for (key, value) in &entry.metadata {
        encode_string(payload, key)?;
        encode_string(payload, value)?;
    }
    Ok(())
}

fn decode_diagnostic_entry(
    payload: &[u8],
    pos: &mut usize,
) -> Result<OperatorDiagnosticEntry, String> {
    let sequence = decode_u64(payload, pos)?;
    let transition = decode_string(payload, pos)?;
    let request_key = decode_optional_string(payload, pos)?;
    let attempt_id = decode_optional_string(payload, pos)?;
    let owner_node = decode_optional_string(payload, pos)?;
    let replica_node = decode_optional_string(payload, pos)?;
    let execution_node = decode_optional_string(payload, pos)?;
    let cluster_role = decode_optional_string(payload, pos)?;
    let promotion_epoch = decode_optional_u64(payload, pos)?;
    let replication_health = decode_optional_string(payload, pos)?;
    let replica_status = decode_optional_string(payload, pos)?;
    let reason = decode_optional_string(payload, pos)?;
    let metadata_count = decode_u32(payload, pos)? as usize;
    let mut metadata = Vec::with_capacity(metadata_count);
    for _ in 0..metadata_count {
        let key = decode_string(payload, pos)?;
        let value = decode_string(payload, pos)?;
        metadata.push((key, value));
    }
    Ok(OperatorDiagnosticEntry {
        sequence,
        transition,
        request_key,
        attempt_id,
        owner_node,
        replica_node,
        execution_node,
        cluster_role,
        promotion_epoch,
        replication_health,
        replica_status,
        reason,
        metadata,
    })
}

fn encode_optional_limit(payload: &mut Vec<u8>, limit: Option<usize>) -> Result<(), String> {
    match limit {
        Some(limit) => {
            payload.push(1);
            payload
                .extend_from_slice(&u32_from_usize(limit, "operator query limit")?.to_le_bytes());
        }
        None => payload.push(0),
    }
    Ok(())
}

fn decode_optional_limit(payload: &[u8], pos: &mut usize) -> Result<Option<usize>, String> {
    match decode_byte(payload, pos)? {
        0 => Ok(None),
        1 => Ok(Some(decode_u32(payload, pos)? as usize)),
        other => Err(format!("invalid operator query limit flag {other}")),
    }
}

fn encode_string_list(payload: &mut Vec<u8>, values: &[String]) -> Result<(), String> {
    payload.extend_from_slice(
        &u32_from_usize(values.len(), "operator string list length")?.to_le_bytes(),
    );
    for value in values {
        encode_string(payload, value)?;
    }
    Ok(())
}

fn decode_string_list(payload: &[u8], pos: &mut usize) -> Result<Vec<String>, String> {
    let count = decode_u32(payload, pos)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(decode_string(payload, pos)?);
    }
    Ok(values)
}

fn encode_string(payload: &mut Vec<u8>, value: &str) -> Result<(), String> {
    payload
        .extend_from_slice(&u32_from_usize(value.len(), "operator string length")?.to_le_bytes());
    payload.extend_from_slice(value.as_bytes());
    Ok(())
}

fn decode_string(payload: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = decode_u32(payload, pos)? as usize;
    if *pos + len > payload.len() {
        return Err("operator string payload truncated".to_string());
    }
    let value = std::str::from_utf8(&payload[*pos..*pos + len])
        .map_err(|_| "operator string payload invalid UTF-8".to_string())?
        .to_string();
    *pos += len;
    Ok(value)
}

fn encode_optional_string(payload: &mut Vec<u8>, value: Option<&str>) -> Result<(), String> {
    match value {
        Some(value) => {
            payload.push(1);
            encode_string(payload, value)
        }
        None => {
            payload.push(0);
            Ok(())
        }
    }
}

fn decode_optional_string(payload: &[u8], pos: &mut usize) -> Result<Option<String>, String> {
    match decode_byte(payload, pos)? {
        0 => Ok(None),
        1 => decode_string(payload, pos).map(Some),
        other => Err(format!("invalid operator optional string flag {other}")),
    }
}

fn encode_optional_u64(payload: &mut Vec<u8>, value: Option<u64>) -> Result<(), String> {
    match value {
        Some(value) => {
            payload.push(1);
            payload.extend_from_slice(&value.to_le_bytes());
            Ok(())
        }
        None => {
            payload.push(0);
            Ok(())
        }
    }
}

fn decode_optional_u64(payload: &[u8], pos: &mut usize) -> Result<Option<u64>, String> {
    match decode_byte(payload, pos)? {
        0 => Ok(None),
        1 => decode_u64(payload, pos).map(Some),
        other => Err(format!("invalid operator optional u64 flag {other}")),
    }
}

fn decode_bool(payload: &[u8], pos: &mut usize) -> Result<bool, String> {
    match decode_byte(payload, pos)? {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(format!("invalid operator bool flag {other}")),
    }
}

fn decode_byte(payload: &[u8], pos: &mut usize) -> Result<u8, String> {
    if *pos >= payload.len() {
        return Err("operator payload truncated".to_string());
    }
    let value = payload[*pos];
    *pos += 1;
    Ok(value)
}

fn decode_u32(payload: &[u8], pos: &mut usize) -> Result<u32, String> {
    if *pos + 4 > payload.len() {
        return Err("operator u32 payload truncated".to_string());
    }
    let value = u32::from_le_bytes(payload[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(value)
}

fn decode_u64(payload: &[u8], pos: &mut usize) -> Result<u64, String> {
    if *pos + 8 > payload.len() {
        return Err("operator u64 payload truncated".to_string());
    }
    let value = u64::from_le_bytes(payload[*pos..*pos + 8].try_into().unwrap());
    *pos += 8;
    Ok(value)
}

fn u32_from_usize(value: usize, label: &str) -> Result<u32, String> {
    value
        .try_into()
        .map_err(|_| format!("{label} exceeds u32 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    static OPERATOR_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static OPERATOR_QUERY_TEST_INIT: std::sync::Once = std::sync::Once::new();
    const OPERATOR_QUERY_TEST_COOKIE: &str = "mesh-operator-query-test-cookie";

    fn fresh_registry() -> ContinuityRegistry {
        ContinuityRegistry::new()
    }

    #[test]
    fn operator_hmac_keyring_allows_rolling_rotation() {
        let old = "old-operator-key-0123456789abcdef";
        let new = "new-operator-key-0123456789abcdef";
        let request = OperatorControlRequest {
            schema_version: 1,
            cluster_id: "cluster-a".to_string(),
            actor: "operator-a".to_string(),
            sequence: 1,
            expires_at_unix_millis: 100,
            reason: "rotation test".to_string(),
            action: OperatorControlAction::PauseAutoscaler,
            signature: String::new(),
        };
        assert_eq!(
            sign_operator_control_request(request.clone(), "short-key"),
            Err("operator_control_key_missing".to_string())
        );
        let signed = sign_operator_control_request(request, old).expect("signed request");
        let signature = decode_hex_signature(&signed.signature).expect("signature");
        let payload = control_signature_payload(&signed).expect("payload");

        assert!(operator_control_signature_matches(
            &format!("{new},{old}"),
            &payload,
            &signature,
        ));
        assert!(!operator_control_signature_matches(
            new, &payload, &signature,
        ));
    }

    fn unused_loopback_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("bind ephemeral operator-query test port")
            .local_addr()
            .expect("operator-query test local_addr")
            .port()
    }

    fn ensure_operator_query_test_node() -> String {
        OPERATOR_QUERY_TEST_INIT.call_once(|| {
            let port = unused_loopback_port();
            let target = format!("operator-query-test@127.0.0.1:{port}");
            let start_code = crate::dist::node::mesh_node_start(
                target.as_ptr(),
                target.len() as u64,
                OPERATOR_QUERY_TEST_COOKIE.as_ptr(),
                OPERATOR_QUERY_TEST_COOKIE.len() as u64,
            );
            assert!(
                start_code == 0 || start_code == -1,
                "mesh_node_start should succeed or reuse the process test node"
            );
            std::thread::sleep(Duration::from_millis(150));
        });
        crate::dist::node::start_one_shot_test_listener()
            .expect("start a fresh operator query listener")
    }

    #[test]
    fn operator_query_status_includes_self_when_zero_records() {
        let registry = fresh_registry();
        let snapshot = execute_local_query(
            Some("alpha@127.0.0.1:9000"),
            &[],
            &registry,
            &OperatorDiagnosticsBuffer::new(8),
            OperatorQuery::Status,
            false,
        )
        .expect("status query should succeed");

        let OperatorReply::Status(snapshot) = snapshot else {
            panic!("expected status reply");
        };

        assert_eq!(snapshot.membership.local_node, "alpha@127.0.0.1:9000");
        assert!(snapshot.membership.peer_nodes.is_empty());
        assert_eq!(
            snapshot.membership.nodes,
            vec!["alpha@127.0.0.1:9000".to_string()]
        );
        assert_eq!(snapshot.authority.cluster_role, "primary");
        assert_eq!(snapshot.authority.promotion_epoch, 0);
        assert_eq!(snapshot.authority.replication_health, "local_only");
    }

    #[test]
    fn continuity_list_supports_repeated_runtime_names_without_order_assumptions() {
        let registry = fresh_registry();
        let runtime_name = "Api.Todos.handle_list_todos";
        let base_record = ContinuityRecord {
            request_key: String::new(),
            payload_hash: String::new(),
            record_version: 1,
            request_payload: Vec::new(),
            attempt_id: String::new(),
            phase: crate::dist::continuity::ContinuityPhase::Completed,
            result: crate::dist::continuity::ContinuityResult::Succeeded,
            ingress_node: "ingress@host".to_string(),
            owner_node: "owner@host".to_string(),
            replica_nodes: vec!["replica@host".to_string()],
            acknowledged_replica_nodes: vec!["replica@host".to_string()],
            replica_node: "replica@host".to_string(),
            replication_count: 2,
            replica_status: crate::dist::continuity::ReplicaStatus::Mirrored,
            cluster_role: crate::dist::continuity::ContinuityClusterRole::Primary,
            promotion_epoch: 0,
            replication_health: crate::dist::continuity::ReplicationHealth::Healthy,
            execution_node: "owner@host".to_string(),
            routed_remotely: false,
            fell_back_locally: true,
            error: String::new(),
            declared_handler_runtime_name: runtime_name.to_string(),
        };

        let first = ContinuityRecord {
            request_key: "http-route::Api.Todos.handle_list_todos::2".to_string(),
            payload_hash: "hash-2".to_string(),
            attempt_id: "attempt-2".to_string(),
            ..base_record.clone()
        };
        let second = ContinuityRecord {
            request_key: "http-route::Api.Todos.handle_list_todos::1".to_string(),
            payload_hash: "hash-1".to_string(),
            attempt_id: "attempt-1".to_string(),
            ..base_record
        };

        registry
            .merge_remote_record(3, first.clone())
            .expect("merge first repeated-runtime record");
        registry
            .merge_remote_record(4, second.clone())
            .expect("merge second repeated-runtime record");

        let list = continuity_list_from_registry(&registry, None);
        assert_eq!(list.total_records, 2);
        assert_eq!(list.records.len(), 2);
        assert!(list.records.iter().all(|record| {
            record.declared_handler_runtime_name() == runtime_name && record.replication_count == 2
        }));

        let first_lookup = list
            .records
            .iter()
            .find(|record| record.request_key == first.request_key)
            .expect("find first request key regardless of list order");
        assert_eq!(first_lookup.attempt_id, "attempt-2");

        let second_lookup = list
            .records
            .iter()
            .find(|record| record.request_key == second.request_key)
            .expect("find second request key regardless of list order");
        assert_eq!(second_lookup.attempt_id, "attempt-1");
    }

    #[test]
    fn operator_query_transient_status_does_not_register_peer() {
        let _guard = OPERATOR_TEST_GUARD.lock().unwrap();
        let target = ensure_operator_query_test_node();
        let state = node_state().expect("operator query test node should be started");
        let sessions_before: Vec<String> = state.sessions.read().keys().cloned().collect();

        let snapshot = query_operator_status_remote(&target, &state.cookie, Duration::from_secs(2))
            .expect("transient operator query should succeed");

        assert_eq!(snapshot.membership.local_node, state.name);
        let mut expected_nodes = sessions_before.clone();
        expected_nodes.push(snapshot.membership.local_node.clone());
        expected_nodes.sort();
        expected_nodes.dedup();
        assert_eq!(snapshot.membership.nodes, expected_nodes);
        assert_eq!(snapshot.authority.cluster_role, "primary");
        assert_eq!(snapshot.authority.promotion_epoch, 0);
        assert_eq!(snapshot.authority.replication_health, "local_only");
        assert_eq!(
            state.sessions.read().keys().cloned().collect::<Vec<_>>(),
            sessions_before,
            "transient operator query must not register a visible peer session"
        );
    }

    #[test]
    fn operator_query_invalid_kind_returns_error_reply() {
        let registry = fresh_registry();
        let diagnostics = OperatorDiagnosticsBuffer::new(8);
        let mut frame = Vec::new();
        frame.push(DIST_OPERATOR_QUERY);
        frame.extend_from_slice(&7u64.to_le_bytes());
        frame.push(0xFF);
        frame.extend_from_slice(&0u32.to_le_bytes());

        let reply = build_query_reply_frame(&frame, &registry, &diagnostics, false)
            .expect("rejectable malformed query should still produce reply frame");
        let (request_id, result) = decode_query_reply_frame(&reply).expect("decode error reply");
        assert_eq!(request_id, 7);
        let reason = result.expect_err("invalid query kind should reject");
        assert!(
            reason.contains("invalid operator query kind"),
            "unexpected reason: {reason}"
        );
    }

    #[test]
    fn operator_query_missing_request_key_returns_error_reply() {
        let registry = fresh_registry();
        let diagnostics = OperatorDiagnosticsBuffer::new(8);
        let frame = encode_query_frame(
            9,
            &OperatorQuery::ContinuityLookup {
                request_key: String::new(),
            },
        )
        .expect("encode continuity lookup frame");

        let reply = build_query_reply_frame(&frame, &registry, &diagnostics, false)
            .expect("missing request key should produce error reply");
        let (request_id, result) = decode_query_reply_frame(&reply).expect("decode error reply");
        assert_eq!(request_id, 9);
        assert_eq!(
            result.expect_err("empty request key should reject"),
            "request_key_missing"
        );
    }

    #[test]
    fn operator_control_query_consumes_the_json_payload() {
        let request = OperatorControlRequest {
            schema_version: 1,
            cluster_id: "proof-cluster".to_string(),
            actor: "proof-controller".to_string(),
            sequence: 7,
            expires_at_unix_millis: 42,
            reason: "scale decision".to_string(),
            action: OperatorControlAction::SetDesiredCapacity { worker_nodes: 4 },
            signature: "test-signature".to_string(),
        };
        let frame = encode_query_frame(11, &OperatorQuery::Control(request.clone()))
            .expect("encode control query");
        let (request_id, kind, payload) = decode_query_header(&frame).expect("decode query header");
        assert_eq!(request_id, 11);
        assert_eq!(
            decode_query(
                OperatorQueryKind::from_wire(kind).expect("known query kind"),
                payload
            )
            .expect("decode control query"),
            OperatorQuery::Control(request)
        );
    }

    #[test]
    fn internal_drain_actor_requires_authenticated_controller_session() {
        let request = OperatorControlRequest {
            schema_version: 1,
            cluster_id: "proof-cluster".to_string(),
            actor: "mesh-drain-propagator".to_string(),
            sequence: 7,
            expires_at_unix_millis: 42,
            reason: "propagate committed drain".to_string(),
            action: OperatorControlAction::DrainNode {
                node_id: "worker-a".to_string(),
            },
            signature: String::new(),
        };

        assert_eq!(
            validate_internal_control_caller(&request, false),
            Err("operator_internal_control_requires_controller_identity".to_string())
        );
        assert_eq!(validate_internal_control_caller(&request, true), Ok(()));
    }

    #[test]
    fn rejected_operator_control_is_retained_without_signature_material() {
        let request = OperatorControlRequest {
            schema_version: 1,
            cluster_id: "proof-cluster".to_string(),
            actor: "untrusted-operator".to_string(),
            sequence: 9,
            expires_at_unix_millis: 42,
            reason: "attempted action".to_string(),
            action: OperatorControlAction::PauseAutoscaler,
            signature: "must-never-be-audited".to_string(),
        };

        audit_operator_control_rejection(&request, "operator_control_unauthorized");

        let snapshot = diagnostics_buffer().snapshot(None);
        let entry = snapshot
            .entries
            .iter()
            .rev()
            .find(|entry| {
                entry.transition == "operator_control_rejected"
                    && entry
                        .metadata
                        .iter()
                        .any(|(name, value)| name == "actor" && value == "untrusted-operator")
            })
            .expect("rejected control diagnostic");
        assert_eq!(
            entry.reason.as_deref(),
            Some("operator_control_unauthorized")
        );
        assert!(!format!("{entry:?}").contains("must-never-be-audited"));
    }

    #[test]
    fn operator_query_status_decode_rejects_truncated_payload() {
        let mut payload = Vec::new();
        encode_string(&mut payload, "node@127.0.0.1:9000").expect("encode local node");
        encode_string_list(&mut payload, &[]).expect("encode peers");
        encode_string(&mut payload, "primary").expect("encode role");
        // Intentionally omit promotion_epoch and replication_health.

        let err = decode_query_reply_payload(OperatorQueryKind::Status, &payload)
            .expect_err("truncated status payload should fail decode");
        assert!(
            err.contains("truncated"),
            "expected truncated payload error, got: {err}"
        );
    }

    #[test]
    fn operator_diagnostics_ring_buffer_tracks_truncation() {
        let buffer = OperatorDiagnosticsBuffer::new(2);
        buffer.record(OperatorDiagnosticRecord {
            transition: "submit".to_string(),
            request_key: Some("req-1".to_string()),
            ..OperatorDiagnosticRecord::default()
        });
        buffer.record(OperatorDiagnosticRecord {
            transition: "owner_lost".to_string(),
            request_key: Some("req-2".to_string()),
            ..OperatorDiagnosticRecord::default()
        });
        buffer.record(OperatorDiagnosticRecord {
            transition: "degraded".to_string(),
            request_key: Some("req-3".to_string()),
            ..OperatorDiagnosticRecord::default()
        });

        let snapshot = buffer.snapshot(None);
        assert_eq!(snapshot.total_entries, 2);
        assert_eq!(snapshot.dropped_entries, 1);
        assert!(snapshot.truncated);
        assert_eq!(snapshot.buffer_capacity, 2);
        assert_eq!(snapshot.entries.len(), 2);
        assert_eq!(snapshot.entries[0].transition, "owner_lost");
        assert_eq!(snapshot.entries[1].transition, "degraded");
    }

    #[test]
    fn operator_diagnostics_recent_snapshot_keeps_reason_and_metadata() {
        let _guard = OPERATOR_TEST_GUARD.lock().unwrap();
        diagnostics_buffer().clear();
        record_diagnostic(OperatorDiagnosticRecord {
            transition: "prepare_timeout".to_string(),
            request_key: Some("req-9".to_string()),
            attempt_id: Some("attempt-9".to_string()),
            replica_node: Some("replica@127.0.0.1:9001".to_string()),
            reason: Some("replica_prepare_timeout".to_string()),
            metadata: vec![("query_kind".to_string(), "diagnostics".to_string())],
            ..OperatorDiagnosticRecord::default()
        });

        let snapshot = operator_recent_diagnostics(None);
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].transition, "prepare_timeout");
        assert_eq!(
            snapshot.entries[0].reason.as_deref(),
            Some("replica_prepare_timeout")
        );
        assert_eq!(
            snapshot.entries[0].metadata,
            vec![("query_kind".to_string(), "diagnostics".to_string())]
        );
        diagnostics_buffer().clear();
    }

    #[test]
    fn runtime_schema_six_preserves_schema_five_read_compatibility() {
        let snapshot: OperatorRuntimeSnapshot = serde_json::from_value(serde_json::json!({
            "schema_version": 5,
            "local_node": "node-a",
            "telemetry_complete": true,
            "desired_capacity": 1,
            "observed_capacity": 1,
            "ready_capacity": 1,
            "draining_capacity": 0,
            "autoscaler_paused": false,
            "scheduler_min_workers": 1,
            "scheduler_max_workers": 2,
            "scheduler_active_workers": 1,
            "nodes": []
        }))
        .expect("schema-five runtime snapshot");

        assert_eq!(snapshot.schema_version, 5);
        assert_eq!(snapshot.local_telemetry, Default::default());
        assert!(snapshot.local_peer_sessions.is_empty());
        assert!(snapshot.local_continuity_store.is_none());
    }
}
