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

type RpcAnswer<T, E = openraft::error::Infallible> = Result<T, MeshRaftError<E>>;

impl MeshConsensusRpcReply {
    fn into_append(self) -> Result<RpcAnswer<AppendEntriesResponse<ConsensusNodeId>>, Self> {
        match self {
            Self::Append(answer) => Ok(answer),
            other => Err(other),
        }
    }

    fn into_install_snapshot(
        self,
    ) -> Result<RpcAnswer<InstallSnapshotResponse<ConsensusNodeId>, InstallSnapshotError>, Self>
    {
        match self {
            Self::InstallSnapshot(answer) => Ok(answer),
            other => Err(other),
        }
    }

    fn into_vote(self) -> Result<RpcAnswer<VoteResponse<ConsensusNodeId>>, Self> {
        match self {
            Self::Vote(answer) => Ok(answer),
            other => Err(other),
        }
    }
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

/// Serves this node's Raft to its peers' RPCs. Only a durable node's start
/// registers one, after it checked the node's identity and the cluster's
/// name, and from inside the Tokio runtime its Raft runs on.
fn register_mesh_consensus_rpc_server(
    cluster_name: &str,
    node_id: ConsensusNodeId,
    node_name: &str,
    raft: MeshRaft,
    state_machine: DurableConsensusStateMachine,
) {
    *consensus_rpc_server().write().unwrap() = Some(MeshConsensusRpcServer {
        cluster_name: cluster_name.to_string(),
        node_id,
        node_name: node_name.to_string(),
        raft,
        state_machine,
        runtime: tokio::runtime::Handle::current(),
    });
}

fn encode_consensus_rpc_reply(reply: MeshConsensusRpcReply) -> Vec<u8> {
    serde_json::to_vec(&reply).expect("a consensus RPC reply encodes")
}

/// The local node's Raft and the RPC in `payload`, if the request is for
/// it: the session negotiated autonomous mode, and the envelope names this
/// cluster, this node, and the authenticated peer as its source.
fn accepted_consensus_rpc(
    autonomous_enabled: bool,
    remote_name: &str,
    payload: &[u8],
    server: Option<MeshConsensusRpcServer>,
    local_node: &str,
) -> Result<(MeshConsensusRpcServer, MeshConsensusRpc), String> {
    if !autonomous_enabled {
        return Err("consensus_rpc_capability_unavailable".to_string());
    }
    let request: MeshConsensusRpcEnvelope = serde_json::from_slice(payload)
        .map_err(|error| format!("consensus_rpc_request_decode_failed:{error}"))?;
    let server = server.ok_or_else(|| "consensus_rpc_server_unavailable".to_string())?;
    if request.cluster_name != server.cluster_name
        || request.target_id != server.node_id
        || request.source_id == 0
        || request.source_name != remote_name
        || server.node_name != local_node
    {
        return Err("consensus_rpc_identity_mismatch".to_string());
    }
    Ok((server, request.rpc))
}

/// The local Raft's answer to an accepted RPC, encoded for the reply frame.
async fn answer_consensus_rpc(raft: &MeshRaft, rpc: MeshConsensusRpc) -> Vec<u8> {
    encode_consensus_rpc_reply(match rpc {
        MeshConsensusRpc::Append(request) => {
            MeshConsensusRpcReply::Append(raft.append_entries(request).await)
        }
        MeshConsensusRpc::InstallSnapshot(request) => {
            MeshConsensusRpcReply::InstallSnapshot(raft.install_snapshot(request).await)
        }
        MeshConsensusRpc::Vote(request) => MeshConsensusRpcReply::Vote(raft.vote(request).await),
    })
}

/// Dispatch an incoming Raft request away from the distribution reader thread.
/// The authenticated peer name must match the source name in the signed TLS
/// session, and cluster/target identity must match the registered local node.
pub(crate) fn handle_mesh_consensus_rpc(
    session: Arc<super::node::NodeSession>,
    correlation_id: u64,
    payload: Vec<u8>,
) {
    let server = consensus_rpc_server()
        .read()
        .ok()
        .and_then(|server| server.clone());
    let accepted = accepted_consensus_rpc(
        session.negotiated_protocol.autonomous_enabled,
        &session.remote_name,
        &payload,
        server,
        super::node::node_state().map_or("", |state| state.name.as_str()),
    );
    let (server, rpc) = match accepted {
        Ok(accepted) => accepted,
        Err(reason) => {
            let reply = encode_consensus_rpc_reply(MeshConsensusRpcReply::TransportError(reason));
            let _ = super::node::send_mesh_consensus_rpc_reply(&session, correlation_id, &reply);
            return;
        }
    };
    server.runtime.spawn(async move {
        let payload = answer_consensus_rpc(&server.raft, rpc).await;
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
        .expect("a consensus RPC encodes");
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
}

/// A peer that could not answer: its RPC is retried like a lost message.
fn unreachable<E: std::error::Error>(error: impl std::fmt::Display) -> MeshRpcError<E> {
    RPCError::Unreachable(Unreachable::new(&std::io::Error::other(error.to_string())))
}

/// The response `reply` carries for an RPC whose own reply variant `pick`
/// takes out: the target's answer, its refusal, or why it could not answer.
fn rpc_reply<T, E: std::error::Error>(
    target: ConsensusNodeId,
    reply: Result<MeshConsensusRpcReply, String>,
    pick: fn(MeshConsensusRpcReply) -> Result<RpcAnswer<T, E>, MeshConsensusRpcReply>,
) -> Result<T, MeshRpcError<E>> {
    match reply.map(pick) {
        Ok(Ok(Ok(response))) => Ok(response),
        Ok(Ok(Err(error))) => Err(RPCError::RemoteError(RemoteError::new(target, error))),
        Ok(Err(MeshConsensusRpcReply::TransportError(error))) | Err(error) => {
            Err(unreachable(error))
        }
        Ok(Err(_)) => Err(unreachable("consensus_rpc_reply_kind_mismatch")),
    }
}

impl RaftNetwork<MeshRaftConfig> for MeshConsensusConnection {
    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<MeshRaftConfig>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<ConsensusNodeId>, MeshRpcError> {
        let rpc = MeshConsensusRpc::Append(request);
        let reply = self.round_trip(rpc, false, option).await;
        rpc_reply(self.target, reply, MeshConsensusRpcReply::into_append)
    }

    async fn install_snapshot(
        &mut self,
        request: InstallSnapshotRequest<MeshRaftConfig>,
        option: RPCOption,
    ) -> Result<InstallSnapshotResponse<ConsensusNodeId>, MeshRpcError<InstallSnapshotError>> {
        let rpc = MeshConsensusRpc::InstallSnapshot(request);
        let reply = self.round_trip(rpc, true, option).await;
        rpc_reply(
            self.target,
            reply,
            MeshConsensusRpcReply::into_install_snapshot,
        )
    }

    async fn vote(
        &mut self,
        request: VoteRequest<ConsensusNodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<ConsensusNodeId>, MeshRpcError> {
        let rpc = MeshConsensusRpc::Vote(request);
        let reply = self.round_trip(rpc, false, option).await;
        rpc_reply(self.target, reply, MeshConsensusRpcReply::into_vote)
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
    );
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

/// Reads one deployment environment variable: the runtime passes
/// `std::env::var_os`, tests a table of their own.
type EnvironmentLookup<'a> = &'a dyn Fn(&str) -> Option<std::ffi::OsString>;

/// This controller's consensus settings when `autonomous` mode runs it as a
/// `controller`, from the deployment environment.
fn consensus_environment(
    node_name: &str,
    autonomous: bool,
    controller: bool,
    env: EnvironmentLookup<'_>,
) -> Result<Option<MeshConsensusEnvironment>, String> {
    if !autonomous || !controller {
        return Ok(None);
    }
    let text = |name: &str, missing: &str| {
        env(name)
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| missing.to_string())
    };
    let cluster_name = text("MESH_CLUSTER_ID", "consensus_cluster_id_missing")?;
    if cluster_name.trim().is_empty() || cluster_name.len() > 256 {
        return Err("consensus_cluster_id_invalid".to_string());
    }
    let local_stable_id = text("MESH_STABLE_NODE_ID", "consensus_stable_node_id_missing")?;
    let local_id = consensus_node_id_for_stable_id(&local_stable_id)?;
    let encoded_voters = text(
        "MESH_CONTROLLER_VOTERS",
        "consensus_controller_voters_missing",
    )?;
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
    // Splitting yields at least one voter, or an error above.
    if voters.len() > 1 && voters.len().is_multiple_of(2) {
        return Err("consensus_controller_voter_count_invalid".to_string());
    }
    if voters.get(&local_id).map(|node| node.addr.as_str()) != Some(node_name) {
        return Err("consensus_local_voter_identity_mismatch".to_string());
    }
    let store_path = env("MESH_CONSENSUS_DB")
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
    // The same mode and roles the rest of the runtime reads: a controller
    // enabled by its embedded manifest alone never started consensus.
    let Some(environment) = consensus_environment(
        node_name,
        super::node::autonomous_mode_requested(),
        super::readiness::local_roles().contains(super::telemetry::NodeRoles::CONTROLLER),
        &|name| std::env::var_os(name),
    )?
    else {
        return Ok(false);
    };
    MESH_CONSENSUS_RUNTIME_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "consensus_runtime_already_started".to_string())?;
    // A node that cannot start the runtime fails to start; it cannot be
    // started again, so the flag stays taken.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("mesh-control-plane-worker")
        .enable_all()
        .build()
        .map_err(|error| format!("consensus_runtime_start_failed:{error}"))?;
    runtime.spawn(run_mesh_consensus(environment));
    // The consensus runs for as long as the process does.
    std::mem::forget(runtime);
    Ok(true)
}

/// Starts this controller's consensus node, which the bootstrap voter
/// initializes. Its Raft runs on the runtime for as long as the handle
/// registered for peers' RPCs lives: as long as the process.
async fn run_mesh_consensus(environment: MeshConsensusEnvironment) {
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
    if environment.local_id == environment.bootstrap_id {
        // Refused once the node holds a log or a vote, which openraft
        // documents as safe to ignore, or when the Raft has stopped, which
        // every later call reports.
        let _ = node.raft.initialize(environment.voters).await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::scaling::{DesiredCapacity, DesiredRevision};
    use super::*;
    use openraft::{SnapshotMeta, Vote};

    fn command() -> ConsensusCommand {
        ConsensusCommand {
            command_id: "command".to_string(),
            actor: "actor".to_string(),
            reason: "reason".to_string(),
            timestamp_unix_millis: 1,
            actor_sequence: 0,
            mutation: ControlMutation::DesiredCapacity(DesiredCapacity {
                revision: DesiredRevision(1),
                worker_nodes: 1,
                gateway_nodes: 0,
                template_revision: "v1".to_string(),
            }),
        }
    }

    #[test]
    fn commands_are_refused_for_empty_or_oversized_fields() {
        assert_eq!(command().validate(), Ok(()));
        let cases: [fn(&mut ConsensusCommand); 6] = [
            |command| command.command_id = " ".to_string(),
            |command| command.command_id = "x".repeat(513),
            |command| command.actor = String::new(),
            |command| command.actor = "x".repeat(257),
            |command| command.reason = " ".to_string(),
            |command| command.reason = "x".repeat(2_049),
        ];
        for change in cases {
            let mut invalid = command();
            change(&mut invalid);
            assert_eq!(
                invalid.validate(),
                Err("consensus_command_invalid".to_string())
            );
            assert_eq!(
                commit_consensus_command(invalid, Duration::from_secs(1)),
                Err("consensus_command_invalid".to_string())
            );
        }
        assert_eq!(
            commit_consensus_command(command(), Duration::ZERO),
            Err("consensus_commit_timeout_invalid".to_string())
        );
        // Nothing in this process registers the embedded consensus.
        assert_eq!(
            commit_consensus_command(command(), Duration::from_secs(1)),
            Err("consensus_rpc_server_unavailable".to_string())
        );
        assert_eq!(consensus_runtime_snapshot(), None);
        assert_eq!(start_mesh_consensus_from_env("node@host:4370"), Ok(false));
    }

    #[test]
    fn stable_node_ids_hash_to_consensus_ids() {
        let id = consensus_node_id_for_stable_id(" cluster/controller/a ").unwrap();
        assert_eq!(
            Ok(id),
            consensus_node_id_for_stable_id("cluster/controller/a")
        );
        assert_ne!(
            Ok(id),
            consensus_node_id_for_stable_id("cluster/controller/b")
        );
        for invalid in [" ".to_string(), "x".repeat(513)] {
            assert_eq!(
                consensus_node_id_for_stable_id(&invalid),
                Err("consensus_stable_node_id_invalid".to_string())
            );
        }
    }

    fn lookup<'a>(
        table: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |name| {
            table
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.into())
        }
    }

    #[test]
    fn a_controllers_consensus_settings_come_from_its_environment() {
        let voters = "c/controller/a|a@a:4370,c/controller/b|b@b:4370,c/controller/c|c@c:4370";
        let valid = [
            ("MESH_CLUSTER_ID", "c"),
            ("MESH_STABLE_NODE_ID", "c/controller/b"),
            ("MESH_CONTROLLER_VOTERS", voters),
        ];
        assert!(
            consensus_environment("b@b:4370", false, true, &lookup(&valid))
                .unwrap()
                .is_none()
        );
        assert!(
            consensus_environment("b@b:4370", true, false, &lookup(&valid))
                .unwrap()
                .is_none()
        );
        let environment = consensus_environment("b@b:4370", true, true, &lookup(&valid))
            .unwrap()
            .expect("controller environment");
        assert_eq!(environment.cluster_name, "c");
        assert_eq!(
            environment.local_id,
            consensus_node_id_for_stable_id("c/controller/b").unwrap()
        );
        assert_eq!(
            environment.bootstrap_id,
            consensus_node_id_for_stable_id("c/controller/a").unwrap()
        );
        assert_eq!(environment.voters.len(), 3);
        assert_eq!(
            environment.store_path,
            std::path::PathBuf::from("/tmp/mesh-control-plane.redb")
        );
        let mut stored = valid.to_vec();
        stored.push(("MESH_CONSENSUS_DB", "/data/consensus.redb"));
        assert_eq!(
            consensus_environment("b@b:4370", true, true, &lookup(&stored))
                .unwrap()
                .unwrap()
                .store_path,
            std::path::PathBuf::from("/data/consensus.redb")
        );

        let cluster_too_long = "x".repeat(257);
        let refusals: &[(&[(&str, &str)], &str)] = &[
            (&[], "consensus_cluster_id_missing"),
            (&[("MESH_CLUSTER_ID", " ")], "consensus_cluster_id_invalid"),
            (
                &[("MESH_CLUSTER_ID", &cluster_too_long)],
                "consensus_cluster_id_invalid",
            ),
            (
                &[("MESH_CLUSTER_ID", "c")],
                "consensus_stable_node_id_missing",
            ),
            (
                &[("MESH_CLUSTER_ID", "c"), ("MESH_STABLE_NODE_ID", " ")],
                "consensus_stable_node_id_invalid",
            ),
            (
                &[
                    ("MESH_CLUSTER_ID", "c"),
                    ("MESH_STABLE_NODE_ID", "c/controller/b"),
                ],
                "consensus_controller_voters_missing",
            ),
            (
                &[
                    ("MESH_CLUSTER_ID", "c"),
                    ("MESH_STABLE_NODE_ID", "c/controller/b"),
                    ("MESH_CONTROLLER_VOTERS", "c/controller/b"),
                ],
                "consensus_controller_voter_invalid",
            ),
            (
                &[
                    ("MESH_CLUSTER_ID", "c"),
                    ("MESH_STABLE_NODE_ID", "c/controller/b"),
                    ("MESH_CONTROLLER_VOTERS", "c/controller/b| "),
                ],
                "consensus_controller_voter_invalid",
            ),
            (
                &[
                    ("MESH_CLUSTER_ID", "c"),
                    ("MESH_STABLE_NODE_ID", "c/controller/b"),
                    (
                        "MESH_CONTROLLER_VOTERS",
                        "c/controller/b|b@b:4370,c/controller/b|b@b:4370",
                    ),
                ],
                "consensus_controller_voter_invalid",
            ),
            (
                &[
                    ("MESH_CLUSTER_ID", "c"),
                    ("MESH_STABLE_NODE_ID", "c/controller/b"),
                    (
                        "MESH_CONTROLLER_VOTERS",
                        "c/controller/a|a@a:4370,c/controller/b|b@b:4370",
                    ),
                ],
                "consensus_controller_voter_count_invalid",
            ),
            (
                &[
                    ("MESH_CLUSTER_ID", "c"),
                    ("MESH_STABLE_NODE_ID", "c/controller/b"),
                    ("MESH_CONTROLLER_VOTERS", "c/controller/b|elsewhere@b:4370"),
                ],
                "consensus_local_voter_identity_mismatch",
            ),
        ];
        for (table, expected) in refusals {
            assert_eq!(
                consensus_environment("b@b:4370", true, true, &lookup(table)).err(),
                Some(expected.to_string()),
                "{expected}"
            );
        }
    }

    #[test]
    fn consensus_configuration_needs_a_cluster_and_a_node() {
        for cluster in ["".to_string(), "x".repeat(257)] {
            assert_eq!(
                validated_consensus_config(&cluster).err(),
                Some("consensus_node_configuration_invalid".to_string())
            );
        }
        assert!(validated_consensus_config("cluster").is_ok());
        let long_name = "x".repeat(513);
        for (cluster, id, name) in [
            ("", 1, "node"),
            ("cluster", 0, "node"),
            ("cluster", 1, " "),
            ("cluster", 1, long_name.as_str()),
        ] {
            assert_eq!(
                MeshConsensusNetwork::new(cluster, id, name).err(),
                Some("consensus_network_configuration_invalid".to_string())
            );
        }
    }

    fn vote() -> Vote<ConsensusNodeId> {
        Vote::new(1, 1)
    }

    #[tokio::test]
    async fn replies_map_to_answers_refusals_and_unreachable_peers() {
        let target = 7;
        let pick = MeshConsensusRpcReply::into_vote;
        let answer = VoteResponse::new(vote(), None, true);
        assert_eq!(
            rpc_reply(
                target,
                Ok(MeshConsensusRpcReply::Vote(Ok(answer.clone()))),
                pick
            )
            .unwrap(),
            answer
        );
        let refused = rpc_reply(
            target,
            Ok(MeshConsensusRpcReply::Vote(Err(RaftError::Fatal(
                openraft::error::Fatal::Stopped,
            )))),
            pick,
        );
        assert!(matches!(refused, Err(RPCError::RemoteError(ref error)) if error.target == 7));
        for reply in [
            Ok(MeshConsensusRpcReply::TransportError("gone".to_string())),
            Err("no session".to_string()),
            Ok(MeshConsensusRpcReply::Append(Ok(
                AppendEntriesResponse::Success,
            ))),
        ] {
            assert!(matches!(
                rpc_reply(target, reply, pick),
                Err(RPCError::Unreachable(_))
            ));
        }
        let append = MeshConsensusRpcReply::Append(Ok(AppendEntriesResponse::Success));
        assert!(matches!(
            rpc_reply(target, Ok(append), MeshConsensusRpcReply::into_append),
            Ok(AppendEntriesResponse::Success)
        ));
        let snapshot =
            MeshConsensusRpcReply::InstallSnapshot(Ok(InstallSnapshotResponse { vote: vote() }));
        assert!(rpc_reply(
            target,
            Ok(snapshot),
            MeshConsensusRpcReply::into_install_snapshot
        )
        .is_ok());
        let vote_reply = MeshConsensusRpcReply::Vote(Ok(answer.clone()));
        for wrong in [
            rpc_reply(target, Ok(vote_reply), MeshConsensusRpcReply::into_append).map(|_| ()),
            rpc_reply(
                target,
                Ok(MeshConsensusRpcReply::TransportError("x".to_string())),
                MeshConsensusRpcReply::into_append,
            )
            .map(|_| ()),
        ] {
            assert!(matches!(wrong, Err(RPCError::Unreachable(_))));
        }
        assert!(matches!(
            rpc_reply(
                target,
                Ok(MeshConsensusRpcReply::Vote(Ok(answer.clone()))),
                MeshConsensusRpcReply::into_install_snapshot
            ),
            Err(RPCError::Unreachable(_))
        ));

        // A connection to no one is unreachable before anything is sent.
        let mut network = MeshConsensusNetwork::new("cluster", 1, "source@host:4370").unwrap();
        let mut connection = network.new_client(0, &BasicNode::new("")).await;
        let option = || RPCOption::new(Duration::from_secs(1));
        let append = AppendEntriesRequest {
            vote: vote(),
            prev_log_id: None,
            entries: Vec::new(),
            leader_commit: None,
        };
        assert!(matches!(
            connection.append_entries(append, option()).await,
            Err(RPCError::Unreachable(_))
        ));
        assert!(matches!(
            connection
                .vote(VoteRequest::new(vote(), None), option())
                .await,
            Err(RPCError::Unreachable(_))
        ));
        let snapshot = InstallSnapshotRequest {
            vote: vote(),
            meta: SnapshotMeta {
                last_log_id: None,
                last_membership: StoredMembership::default(),
                snapshot_id: "snapshot".to_string(),
            },
            offset: 0,
            data: Vec::new(),
            done: true,
        };
        assert!(matches!(
            connection.install_snapshot(snapshot, option()).await,
            Err(RPCError::Unreachable(_))
        ));
        // A named peer this node has no session with is unreachable too.
        let mut absent = network
            .new_client(2, &BasicNode::new("absent@127.0.0.1:1"))
            .await;
        assert!(matches!(
            absent.vote(VoteRequest::new(vote(), None), option()).await,
            Err(RPCError::Unreachable(_))
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_requests_for_this_node_from_their_peer_are_accepted() {
        let directory = tempfile::tempdir().expect("tempdir");
        let node = super::super::consensus_testing::start_durable_consensus_node(
            9,
            super::super::consensus_testing::InProcessConsensusNetwork::default(),
            "cluster",
            &directory.path().join("consensus.redb"),
        )
        .await
        .expect("durable node");
        let server = MeshConsensusRpcServer {
            cluster_name: "cluster".to_string(),
            node_id: 9,
            node_name: "local@host:4370".to_string(),
            raft: node.raft.clone(),
            state_machine: node.state_machine.clone(),
            runtime: tokio::runtime::Handle::current(),
        };
        let envelope = |cluster: &str, source_id, source: &str, target_id| {
            serde_json::to_vec(&MeshConsensusRpcEnvelope {
                cluster_name: cluster.to_string(),
                source_id,
                source_name: source.to_string(),
                target_id,
                rpc: MeshConsensusRpc::Vote(VoteRequest::new(vote(), None)),
            })
            .unwrap()
        };
        let accept = |autonomous, payload: &[u8], server: Option<MeshConsensusRpcServer>| {
            accepted_consensus_rpc(
                autonomous,
                "peer@host:4370",
                payload,
                server,
                "local@host:4370",
            )
            .map(|(_, rpc)| matches!(rpc, MeshConsensusRpc::Vote(_)))
        };
        let valid = envelope("cluster", 3, "peer@host:4370", 9);
        assert_eq!(accept(true, &valid, Some(server.clone())), Ok(true));
        assert_eq!(
            accept(false, &valid, Some(server.clone())),
            Err("consensus_rpc_capability_unavailable".to_string())
        );
        assert!(accept(true, b"{", Some(server.clone()))
            .unwrap_err()
            .starts_with("consensus_rpc_request_decode_failed:"));
        assert_eq!(
            accept(true, &valid, None),
            Err("consensus_rpc_server_unavailable".to_string())
        );
        for mismatched in [
            envelope("other", 3, "peer@host:4370", 9),
            envelope("cluster", 3, "peer@host:4370", 8),
            envelope("cluster", 0, "peer@host:4370", 9),
            envelope("cluster", 3, "impostor@host:4370", 9),
        ] {
            assert_eq!(
                accept(true, &mismatched, Some(server.clone())),
                Err("consensus_rpc_identity_mismatch".to_string())
            );
        }
        let elsewhere = accepted_consensus_rpc(
            true,
            "peer@host:4370",
            &valid,
            Some(server.clone()),
            "renamed@host:4370",
        );
        assert_eq!(
            elsewhere.map(|_| ()),
            Err("consensus_rpc_identity_mismatch".to_string())
        );
        assert!(
            encode_consensus_rpc_reply(MeshConsensusRpcReply::TransportError("reason".to_string()))
                .starts_with(b"{\"TransportError\"")
        );

        // An accepted RPC gets the local Raft's answer of the same kind.
        let answer = |payload: Vec<u8>| -> MeshConsensusRpcReply {
            serde_json::from_slice(&payload).unwrap()
        };
        let voted = answer(
            answer_consensus_rpc(
                &node.raft,
                MeshConsensusRpc::Vote(VoteRequest::new(vote(), None)),
            )
            .await,
        );
        assert!(voted.into_vote().is_ok());
        let appended = answer(
            answer_consensus_rpc(
                &node.raft,
                MeshConsensusRpc::Append(AppendEntriesRequest {
                    vote: vote(),
                    prev_log_id: None,
                    entries: Vec::new(),
                    leader_commit: None,
                }),
            )
            .await,
        );
        assert!(appended.into_append().is_ok());
        let installed = answer(
            answer_consensus_rpc(
                &node.raft,
                MeshConsensusRpc::InstallSnapshot(InstallSnapshotRequest {
                    vote: vote(),
                    meta: SnapshotMeta {
                        last_log_id: None,
                        last_membership: StoredMembership::default(),
                        snapshot_id: "snapshot".to_string(),
                    },
                    offset: 0,
                    data: Vec::new(),
                    done: true,
                }),
            )
            .await,
        );
        assert!(installed.into_install_snapshot().is_ok());

        // A durable node must be this process's started node.
        let long_name = "x".repeat(513);
        for (id, name) in [(0, "local@host:4370"), (9, " "), (9, long_name.as_str())] {
            assert_eq!(
                start_mesh_durable_consensus_node(id, name, "cluster", directory.path())
                    .await
                    .err(),
                Some("consensus_node_configuration_invalid".to_string())
            );
        }
        let not_this_node = start_mesh_durable_consensus_node(
            9,
            "not-this-node@host:4370",
            "cluster",
            directory.path(),
        )
        .await
        .err()
        .unwrap();
        assert!(
            not_this_node == "consensus_mesh_node_not_started"
                || not_this_node == "consensus_mesh_node_identity_mismatch",
            "{not_this_node}"
        );
        node.raft.shutdown().await.expect("shutdown");
    }
}
