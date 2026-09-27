//! Maintained embedded Raft integration for Mesh control-plane state.
//!
//! OpenRaft owns election, quorum, replication, and fencing semantics.
//! Production nodes attach the Raft API to Mesh's authenticated
//! protocol-two control channel; `consensus_testing` holds an in-process
//! conformance transport for tests.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock as StdRwLock};
use std::time::Duration;

use openraft::error::{InstallSnapshotError, RPCError, RaftError, RemoteError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::{BasicNode, Config, Entry, EntryPayload, LogId, StoredMembership};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use super::consensus_store::{
    open_durable_consensus_store, DurableConsensusLogStore, DurableConsensusStateMachine,
};
use super::scaling::{ControlLogEntry, ControlMutation, ControlTerm};

pub type ConsensusNodeId = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsensusCommand {
    pub command_id: String,
    pub actor: String,
    pub reason: String,
    pub timestamp_unix_millis: u64,
    #[serde(default)]
    pub actor_sequence: u64,
    pub mutation: ControlMutation,
}

impl ConsensusCommand {
    pub fn validate(&self) -> Result<(), String> {
        if self.command_id.trim().is_empty()
            || self.command_id.len() > 512
            || self.actor.trim().is_empty()
            || self.actor.len() > 256
            || self.reason.trim().is_empty()
            || self.reason.len() > 2_048
        {
            return Err("consensus_command_invalid".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsensusResponse {
    pub log_index: u64,
    pub control_term: u64,
    pub applied: bool,
}

openraft::declare_raft_types!(
    pub MeshRaftConfig:
        D = ConsensusCommand,
        R = ConsensusResponse,
);

pub type MeshRaft = openraft::Raft<MeshRaftConfig>;

type MeshRaftError<E = openraft::error::Infallible> = RaftError<ConsensusNodeId, E>;
type MeshRpcError<E = openraft::error::Infallible> =
    RPCError<ConsensusNodeId, BasicNode, MeshRaftError<E>>;

#[derive(Debug, Serialize, Deserialize)]
enum MeshConsensusRpc {
    Append(AppendEntriesRequest<MeshRaftConfig>),
    InstallSnapshot(InstallSnapshotRequest<MeshRaftConfig>),
    Vote(VoteRequest<ConsensusNodeId>),
}

#[derive(Debug, Serialize, Deserialize)]
enum MeshConsensusRpcReply {
    Append(Result<AppendEntriesResponse<ConsensusNodeId>, MeshRaftError>),
    InstallSnapshot(
        Result<InstallSnapshotResponse<ConsensusNodeId>, MeshRaftError<InstallSnapshotError>>,
    ),
    Vote(Result<VoteResponse<ConsensusNodeId>, MeshRaftError>),
    TransportError(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct MeshConsensusRpcEnvelope {
    cluster_name: String,
    source_id: ConsensusNodeId,
    source_name: String,
    target_id: ConsensusNodeId,
    rpc: MeshConsensusRpc,
}

#[derive(Clone)]
struct MeshConsensusRpcServer {
    cluster_name: String,
    node_id: ConsensusNodeId,
    node_name: String,
    raft: MeshRaft,
    state_machine: DurableConsensusStateMachine,
    runtime: tokio::runtime::Handle,
}

static MESH_CONSENSUS_RPC_SERVER: OnceLock<StdRwLock<Option<MeshConsensusRpcServer>>> =
    OnceLock::new();
static MESH_CONSENSUS_RUNTIME_STARTED: AtomicBool = AtomicBool::new(false);

fn consensus_rpc_server() -> &'static StdRwLock<Option<MeshConsensusRpcServer>> {
    MESH_CONSENSUS_RPC_SERVER.get_or_init(|| StdRwLock::new(None))
}

fn register_mesh_consensus_rpc_server(
    cluster_name: &str,
    node_id: ConsensusNodeId,
    node_name: &str,
    raft: MeshRaft,
    state_machine: DurableConsensusStateMachine,
) -> Result<(), String> {
    if cluster_name.trim().is_empty()
        || cluster_name.len() > 256
        || node_id == 0
        || node_name.trim().is_empty()
        || node_name.len() > 512
    {
        return Err("consensus_rpc_server_configuration_invalid".to_string());
    }
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| "consensus_rpc_runtime_unavailable".to_string())?;
    *consensus_rpc_server()
        .write()
        .map_err(|_| "consensus_rpc_server_lock_poisoned".to_string())? =
        Some(MeshConsensusRpcServer {
            cluster_name: cluster_name.to_string(),
            node_id,
            node_name: node_name.to_string(),
            raft,
            state_machine,
            runtime,
        });
    Ok(())
}

fn encode_consensus_rpc_reply(reply: MeshConsensusRpcReply) -> Vec<u8> {
    serde_json::to_vec(&reply).unwrap_or_else(|error| {
        serde_json::to_vec(&MeshConsensusRpcReply::TransportError(format!(
            "consensus_rpc_reply_encode_failed:{error}"
        )))
        .unwrap_or_else(|_| b"{\"TransportError\":\"consensus_rpc_reply_encode_failed\"}".to_vec())
    })
}

fn send_consensus_transport_error(
    session: &Arc<super::node::NodeSession>,
    correlation_id: u64,
    reason: impl Into<String>,
) {
    let payload = encode_consensus_rpc_reply(MeshConsensusRpcReply::TransportError(reason.into()));
    let _ = super::node::send_mesh_consensus_rpc_reply(session, correlation_id, &payload);
}

/// Dispatch an incoming Raft request away from the distribution reader thread.
/// The authenticated peer name must match the source name in the signed TLS
/// session, and cluster/target identity must match the registered local node.
pub(crate) fn handle_mesh_consensus_rpc(
    session: Arc<super::node::NodeSession>,
    correlation_id: u64,
    payload: Vec<u8>,
) {
    if !session.negotiated_protocol.autonomous_enabled {
        send_consensus_transport_error(
            &session,
            correlation_id,
            "consensus_rpc_capability_unavailable",
        );
        return;
    }
    let request: MeshConsensusRpcEnvelope = match serde_json::from_slice(&payload) {
        Ok(request) => request,
        Err(error) => {
            send_consensus_transport_error(
                &session,
                correlation_id,
                format!("consensus_rpc_request_decode_failed:{error}"),
            );
            return;
        }
    };
    let server = match consensus_rpc_server().read() {
        Ok(server) => server.clone(),
        Err(_) => None,
    };
    let Some(server) = server else {
        send_consensus_transport_error(
            &session,
            correlation_id,
            "consensus_rpc_server_unavailable",
        );
        return;
    };
    if request.cluster_name != server.cluster_name
        || request.target_id != server.node_id
        || request.source_id == 0
        || request.source_name != session.remote_name
        || server.node_name != super::node::node_state().map_or("", |state| state.name.as_str())
    {
        send_consensus_transport_error(&session, correlation_id, "consensus_rpc_identity_mismatch");
        return;
    }

    server.runtime.spawn(async move {
        let reply = match request.rpc {
            MeshConsensusRpc::Append(request) => {
                MeshConsensusRpcReply::Append(server.raft.append_entries(request).await)
            }
            MeshConsensusRpc::InstallSnapshot(request) => {
                MeshConsensusRpcReply::InstallSnapshot(server.raft.install_snapshot(request).await)
            }
            MeshConsensusRpc::Vote(request) => {
                MeshConsensusRpcReply::Vote(server.raft.vote(request).await)
            }
        };
        let payload = encode_consensus_rpc_reply(reply);
        if let Err(error) =
            super::node::send_mesh_consensus_rpc_reply(&session, correlation_id, &payload)
        {
            eprintln!(
                "mesh consensus: transition=rpc_reply_write_failed remote={} reason={}",
                session.remote_name, error
            );
        }
    });
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsensusStateMachineData {
    pub last_applied_log: Option<LogId<ConsensusNodeId>>,
    pub last_membership: StoredMembership<ConsensusNodeId, BasicNode>,
    pub entries: Vec<ControlLogEntry>,
    pub command_results: BTreeMap<String, ConsensusResponse>,
}

pub(crate) fn apply_consensus_entries<I>(
    state: &mut ConsensusStateMachineData,
    entries: I,
) -> Vec<ConsensusResponse>
where
    I: IntoIterator<Item = Entry<MeshRaftConfig>>,
{
    let mut responses = Vec::new();
    for entry in entries {
        state.last_applied_log = Some(entry.log_id);
        match entry.payload {
            EntryPayload::Blank => responses.push(ConsensusResponse {
                log_index: entry.log_id.index,
                control_term: entry.log_id.leader_id.term,
                applied: false,
            }),
            EntryPayload::Membership(membership) => {
                state.last_membership = StoredMembership::new(Some(entry.log_id), membership);
                responses.push(ConsensusResponse {
                    log_index: entry.log_id.index,
                    control_term: entry.log_id.leader_id.term,
                    applied: false,
                });
            }
            EntryPayload::Normal(command) => {
                if let Some(existing) = state.command_results.get(&command.command_id) {
                    responses.push(existing.clone());
                    continue;
                }
                let response = ConsensusResponse {
                    log_index: entry.log_id.index,
                    control_term: entry.log_id.leader_id.term,
                    applied: true,
                };
                state.entries.push(ControlLogEntry {
                    index: entry.log_id.index,
                    term: ControlTerm(entry.log_id.leader_id.term),
                    actor: command.actor,
                    reason: command.reason,
                    timestamp_unix_millis: command.timestamp_unix_millis,
                    actor_sequence: command.actor_sequence,
                    mutation: command.mutation,
                });
                state
                    .command_results
                    .insert(command.command_id, response.clone());
                responses.push(response);
            }
        }
    }
    responses
}

/// Production OpenRaft transport over Mesh's persistent, authenticated,
/// protocol-two peer sessions.
#[derive(Clone, Debug)]
pub struct MeshConsensusNetwork {
    cluster_name: String,
    source_id: ConsensusNodeId,
    source_name: String,
}

impl MeshConsensusNetwork {
    pub fn new(
        cluster_name: &str,
        source_id: ConsensusNodeId,
        source_name: &str,
    ) -> Result<Self, String> {
        if cluster_name.trim().is_empty()
            || cluster_name.len() > 256
            || source_id == 0
            || source_name.trim().is_empty()
            || source_name.len() > 512
        {
            return Err("consensus_network_configuration_invalid".to_string());
        }
        Ok(Self {
            cluster_name: cluster_name.to_string(),
            source_id,
            source_name: source_name.to_string(),
        })
    }
}

impl RaftNetworkFactory<MeshRaftConfig> for MeshConsensusNetwork {
    type Network = MeshConsensusConnection;

    async fn new_client(&mut self, target: ConsensusNodeId, node: &BasicNode) -> Self::Network {
        MeshConsensusConnection {
            cluster_name: self.cluster_name.clone(),
            source_id: self.source_id,
            source_name: self.source_name.clone(),
            target,
            target_name: node.addr.clone(),
        }
    }
}

pub struct MeshConsensusConnection {
    cluster_name: String,
    source_id: ConsensusNodeId,
    source_name: String,
    target: ConsensusNodeId,
    target_name: String,
}

impl MeshConsensusConnection {
    async fn round_trip(
        &self,
        rpc: MeshConsensusRpc,
        snapshot: bool,
        option: RPCOption,
    ) -> Result<MeshConsensusRpcReply, String> {
        if self.target == 0 || self.target_name.trim().is_empty() {
            return Err("consensus_rpc_target_invalid".to_string());
        }
        let payload = serde_json::to_vec(&MeshConsensusRpcEnvelope {
            cluster_name: self.cluster_name.clone(),
            source_id: self.source_id,
            source_name: self.source_name.clone(),
            target_id: self.target,
            rpc,
        })
        .map_err(|error| format!("consensus_rpc_request_encode_failed:{error}"))?;
        let reply = super::node::execute_mesh_consensus_rpc(
            &self.target_name,
            payload,
            snapshot,
            option.hard_ttl(),
        )
        .await?;
        serde_json::from_slice(&reply)
            .map_err(|error| format!("consensus_rpc_reply_decode_failed:{error}"))
    }

    fn unreachable<E>(&self, error: E) -> MeshRpcError
    where
        E: std::fmt::Display,
    {
        RPCError::Unreachable(Unreachable::new(&std::io::Error::other(error.to_string())))
    }

    fn unreachable_snapshot<E>(&self, error: E) -> MeshRpcError<InstallSnapshotError>
    where
        E: std::fmt::Display,
    {
        RPCError::Unreachable(Unreachable::new(&std::io::Error::other(error.to_string())))
    }
}

impl RaftNetwork<MeshRaftConfig> for MeshConsensusConnection {
    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<MeshRaftConfig>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<ConsensusNodeId>, MeshRpcError> {
        match self
            .round_trip(MeshConsensusRpc::Append(request), false, option)
            .await
            .map_err(|error| self.unreachable(error))?
        {
            MeshConsensusRpcReply::Append(Ok(response)) => Ok(response),
            MeshConsensusRpcReply::Append(Err(error)) => {
                Err(RPCError::RemoteError(RemoteError::new(self.target, error)))
            }
            MeshConsensusRpcReply::TransportError(error) => Err(self.unreachable(error)),
            _ => Err(self.unreachable("consensus_rpc_reply_kind_mismatch")),
        }
    }

    async fn install_snapshot(
        &mut self,
        request: InstallSnapshotRequest<MeshRaftConfig>,
        option: RPCOption,
    ) -> Result<InstallSnapshotResponse<ConsensusNodeId>, MeshRpcError<InstallSnapshotError>> {
        match self
            .round_trip(MeshConsensusRpc::InstallSnapshot(request), true, option)
            .await
            .map_err(|error| self.unreachable_snapshot(error))?
        {
            MeshConsensusRpcReply::InstallSnapshot(Ok(response)) => Ok(response),
            MeshConsensusRpcReply::InstallSnapshot(Err(error)) => {
                Err(RPCError::RemoteError(RemoteError::new(self.target, error)))
            }
            MeshConsensusRpcReply::TransportError(error) => Err(self.unreachable_snapshot(error)),
            _ => Err(self.unreachable_snapshot("consensus_rpc_reply_kind_mismatch")),
        }
    }

    async fn vote(
        &mut self,
        request: VoteRequest<ConsensusNodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<ConsensusNodeId>, MeshRpcError> {
        match self
            .round_trip(MeshConsensusRpc::Vote(request), false, option)
            .await
            .map_err(|error| self.unreachable(error))?
        {
            MeshConsensusRpcReply::Vote(Ok(response)) => Ok(response),
            MeshConsensusRpcReply::Vote(Err(error)) => {
                Err(RPCError::RemoteError(RemoteError::new(self.target, error)))
            }
            MeshConsensusRpcReply::TransportError(error) => Err(self.unreachable(error)),
            _ => Err(self.unreachable("consensus_rpc_reply_kind_mismatch")),
        }
    }
}

#[derive(Clone)]
pub struct DurableEmbeddedConsensusNode {
    pub node_id: ConsensusNodeId,
    pub raft: MeshRaft,
    pub log_store: DurableConsensusLogStore,
    pub state_machine: DurableConsensusStateMachine,
}

pub(crate) fn validated_consensus_config(cluster_name: &str) -> Result<Arc<Config>, String> {
    if cluster_name.trim().is_empty() || cluster_name.len() > 256 {
        return Err("consensus_node_configuration_invalid".to_string());
    }
    Config {
        cluster_name: cluster_name.to_string(),
        // Distribution currently multiplexes reads and writes through one
        // rustls stream lock with a 100ms framing read timeout. Leave enough
        // room for a request and response to cross that boundary without
        // triggering false elections.
        heartbeat_interval: 500,
        election_timeout_min: 1_500,
        election_timeout_max: 3_000,
        max_payload_entries: 32,
        snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(1_000),
        // JSON encoding expands raw snapshot bytes. Keep each OpenRaft chunk
        // comfortably below Mesh's default negotiated 1 MiB frame ceiling.
        snapshot_max_chunk_size: 512 * 1_024,
        max_in_snapshot_log_to_keep: 100,
        ..Default::default()
    }
    .validate()
    .map(Arc::new)
    .map_err(|error| format!("consensus_configuration_invalid:{error}"))
}

/// Start the crash-durable production controller using Mesh's authenticated
/// peer transport. `BasicNode::addr` entries in the initialized membership
/// must contain the corresponding full Mesh node names.
pub async fn start_mesh_durable_consensus_node(
    node_id: ConsensusNodeId,
    node_name: &str,
    cluster_name: &str,
    path: &Path,
) -> Result<DurableEmbeddedConsensusNode, String> {
    if node_id == 0 || node_name.trim().is_empty() || node_name.len() > 512 {
        return Err("consensus_node_configuration_invalid".to_string());
    }
    let state =
        super::node::node_state().ok_or_else(|| "consensus_mesh_node_not_started".to_string())?;
    if state.name != node_name {
        return Err("consensus_mesh_node_identity_mismatch".to_string());
    }
    let config = validated_consensus_config(cluster_name)?;
    let network = MeshConsensusNetwork::new(cluster_name, node_id, node_name)?;
    let (log_store, state_machine) = open_durable_consensus_store(path)?;
    let raft = MeshRaft::new(
        node_id,
        config,
        network,
        log_store.clone(),
        state_machine.clone(),
    )
    .await
    .map_err(|error| format!("consensus_start_failed:{error}"))?;
    register_mesh_consensus_rpc_server(
        cluster_name,
        node_id,
        node_name,
        raft.clone(),
        state_machine.clone(),
    )?;
    Ok(DurableEmbeddedConsensusNode {
        node_id,
        raft,
        log_store,
        state_machine,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsensusRuntimeSnapshot {
    pub node_id: ConsensusNodeId,
    pub node_name: String,
    pub state: String,
    pub current_term: u64,
    pub current_leader: Option<ConsensusNodeId>,
    pub last_applied_log: Option<u64>,
    pub voter_ids: Vec<ConsensusNodeId>,
    pub entries: Vec<ControlLogEntry>,
}

pub fn consensus_runtime_snapshot() -> Option<ConsensusRuntimeSnapshot> {
    let server = consensus_rpc_server().read().ok()?.clone()?;
    let metrics = server.raft.metrics();
    let metrics = metrics.borrow();
    let state = server.state_machine.state().ok()?;
    Some(ConsensusRuntimeSnapshot {
        node_id: server.node_id,
        node_name: server.node_name,
        state: format!("{:?}", metrics.state).to_ascii_lowercase(),
        current_term: metrics.current_term,
        current_leader: metrics.current_leader,
        last_applied_log: state.last_applied_log.map(|log_id| log_id.index),
        voter_ids: state.last_membership.membership().voter_ids().collect(),
        entries: state.entries,
    })
}

pub fn commit_consensus_command(
    command: ConsensusCommand,
    timeout: Duration,
) -> Result<ConsensusResponse, String> {
    command.validate()?;
    if timeout.is_zero() {
        return Err("consensus_commit_timeout_invalid".to_string());
    }
    let server = consensus_rpc_server()
        .read()
        .map_err(|_| "consensus_rpc_server_lock_poisoned".to_string())?
        .clone()
        .ok_or_else(|| "consensus_rpc_server_unavailable".to_string())?;
    let (sender, receiver) = std::sync::mpsc::channel();
    server.runtime.spawn(async move {
        let result = server
            .raft
            .client_write(command)
            .await
            .map(|response| response.data)
            .map_err(|error| format!("consensus_commit_rejected:{error}"));
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(timeout)
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => "consensus_commit_timeout".to_string(),
            std::sync::mpsc::RecvTimeoutError::Disconnected => {
                "consensus_commit_disconnected".to_string()
            }
        })?
}

#[derive(Debug)]
struct MeshConsensusEnvironment {
    cluster_name: String,
    local_id: ConsensusNodeId,
    local_name: String,
    bootstrap_id: ConsensusNodeId,
    voters: BTreeMap<ConsensusNodeId, BasicNode>,
    store_path: std::path::PathBuf,
}

pub fn consensus_node_id_for_stable_id(stable_id: &str) -> Result<ConsensusNodeId, String> {
    let stable_id = stable_id.trim();
    if stable_id.is_empty() || stable_id.len() > 512 {
        return Err("consensus_stable_node_id_invalid".to_string());
    }
    let digest = sha2::Sha256::digest(stable_id.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let node_id = u64::from_be_bytes(bytes);
    Ok(if node_id == 0 { 1 } else { node_id })
}

fn consensus_environment(node_name: &str) -> Result<Option<MeshConsensusEnvironment>, String> {
    // The same mode and roles the rest of the runtime reads: a controller
    // enabled by its embedded manifest alone never started consensus.
    let autonomous = super::node::autonomous_mode_requested();
    let controller =
        super::readiness::local_roles().contains(super::telemetry::NodeRoles::CONTROLLER);
    if !autonomous || !controller {
        return Ok(None);
    }

    let cluster_name =
        std::env::var("MESH_CLUSTER_ID").map_err(|_| "consensus_cluster_id_missing".to_string())?;
    if cluster_name.trim().is_empty() || cluster_name.len() > 256 {
        return Err("consensus_cluster_id_invalid".to_string());
    }
    let local_stable_id = std::env::var("MESH_STABLE_NODE_ID")
        .map_err(|_| "consensus_stable_node_id_missing".to_string())?;
    let local_id = consensus_node_id_for_stable_id(&local_stable_id)?;
    let encoded_voters = std::env::var("MESH_CONTROLLER_VOTERS")
        .map_err(|_| "consensus_controller_voters_missing".to_string())?;
    let mut voters = BTreeMap::new();
    let mut bootstrap_id = None;
    for encoded in encoded_voters.split(',') {
        let (stable_id, address) = encoded
            .trim()
            .split_once('|')
            .ok_or_else(|| "consensus_controller_voter_invalid".to_string())?;
        let id = consensus_node_id_for_stable_id(stable_id)?;
        let address = address.trim();
        if address.is_empty() || address.len() > 512 || voters.contains_key(&id) {
            return Err("consensus_controller_voter_invalid".to_string());
        }
        bootstrap_id.get_or_insert(id);
        voters.insert(id, BasicNode::new(address));
    }
    if voters.is_empty() || (voters.len() > 1 && voters.len().is_multiple_of(2)) {
        return Err("consensus_controller_voter_count_invalid".to_string());
    }
    if voters.get(&local_id).map(|node| node.addr.as_str()) != Some(node_name) {
        return Err("consensus_local_voter_identity_mismatch".to_string());
    }
    let store_path = std::env::var_os("MESH_CONSENSUS_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/mesh-control-plane.redb"));
    Ok(Some(MeshConsensusEnvironment {
        cluster_name,
        local_id,
        local_name: node_name.to_string(),
        bootstrap_id: bootstrap_id.expect("validated non-empty voter set"),
        voters,
        store_path,
    }))
}

/// Start the controller runtime from the deployment environment. The runtime
/// owns a dedicated Tokio executor and durable store, so consensus I/O cannot
/// block Mesh actor schedulers or distribution reader threads.
pub fn start_mesh_consensus_from_env(node_name: &str) -> Result<bool, String> {
    let Some(environment) = consensus_environment(node_name)? else {
        return Ok(false);
    };
    MESH_CONSENSUS_RUNTIME_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "consensus_runtime_already_started".to_string())?;
    let thread = std::thread::Builder::new()
        .name("mesh-control-plane".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("mesh-control-plane-worker")
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("mesh consensus: transition=runtime_start_failed reason={error}");
                    return;
                }
            };
            runtime.block_on(async move {
                let node = match start_mesh_durable_consensus_node(
                    environment.local_id,
                    &environment.local_name,
                    &environment.cluster_name,
                    &environment.store_path,
                )
                .await
                {
                    Ok(node) => node,
                    Err(error) => {
                        eprintln!("mesh consensus: transition=node_start_failed reason={error}");
                        return;
                    }
                };
                let already_initialized = node
                    .state_machine
                    .state()
                    .map(|state| {
                        state
                            .last_membership
                            .membership()
                            .voter_ids()
                            .next()
                            .is_some()
                    })
                    .unwrap_or(false);
                if environment.local_id == environment.bootstrap_id && !already_initialized {
                    if let Err(error) = node.raft.initialize(environment.voters).await {
                        eprintln!(
                            "mesh consensus: transition=cluster_initialize_failed reason={error}"
                        );
                    }
                }
                while super::node::node_state()
                    .is_some_and(|state| !state.listener_shutdown.load(Ordering::Acquire))
                {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                if let Err(error) = node.raft.shutdown().await {
                    eprintln!("mesh consensus: transition=shutdown_failed reason={error}");
                }
            });
        })
        .map_err(|error| format!("consensus_runtime_thread_failed:{error}"));
    if let Err(error) = thread {
        MESH_CONSENSUS_RUNTIME_STARTED.store(false, Ordering::Release);
        return Err(error);
    }
    Ok(true)
}
