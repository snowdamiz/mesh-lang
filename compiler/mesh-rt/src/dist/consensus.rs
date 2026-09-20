//! Maintained embedded Raft integration for Mesh control-plane state.
//!
//! OpenRaft owns election, quorum, replication, and fencing semantics. The
//! in-process network below is a deterministic conformance transport used by
//! tests; production nodes attach the same Raft API to Mesh's authenticated
//! protocol-two control channel.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock as StdRwLock};
use std::time::Duration;

use openraft::error::{InstallSnapshotError, RPCError, RaftError, RemoteError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::storage::{LogFlushed, RaftLogStorage, RaftStateMachine, Snapshot};
use openraft::{
    BasicNode, Config, Entry, EntryPayload, LogId, LogState, RaftLogId, RaftLogReader,
    RaftSnapshotBuilder, RaftTypeConfig, SnapshotMeta, StorageError, StorageIOError,
    StoredMembership, Vote,
};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use tokio::sync::{Mutex, RwLock};

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

#[derive(Clone, Debug, Default)]
pub struct MemoryRaftLogStore {
    inner: Arc<Mutex<MemoryRaftLogState>>,
}

#[derive(Debug, Default)]
struct MemoryRaftLogState {
    last_purged_log_id: Option<LogId<ConsensusNodeId>>,
    log: BTreeMap<u64, Entry<MeshRaftConfig>>,
    committed: Option<LogId<ConsensusNodeId>>,
    vote: Option<Vote<ConsensusNodeId>>,
}

impl RaftLogReader<MeshRaftConfig> for MemoryRaftLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<MeshRaftConfig>>, StorageError<ConsensusNodeId>> {
        let state = self.inner.lock().await;
        Ok(state
            .log
            .range(range)
            .map(|(_, entry)| entry.clone())
            .collect())
    }
}

impl RaftLogStorage<MeshRaftConfig> for MemoryRaftLogStore {
    type LogReader = Self;

    async fn get_log_state(
        &mut self,
    ) -> Result<LogState<MeshRaftConfig>, StorageError<ConsensusNodeId>> {
        let state = self.inner.lock().await;
        let last_log_id = state
            .log
            .iter()
            .next_back()
            .map(|(_, entry)| *entry.get_log_id())
            .or(state.last_purged_log_id);
        Ok(LogState {
            last_purged_log_id: state.last_purged_log_id,
            last_log_id,
        })
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<ConsensusNodeId>>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        self.inner.lock().await.committed = committed;
        Ok(())
    }

    async fn read_committed(
        &mut self,
    ) -> Result<Option<LogId<ConsensusNodeId>>, StorageError<ConsensusNodeId>> {
        Ok(self.inner.lock().await.committed)
    }

    async fn save_vote(
        &mut self,
        vote: &Vote<ConsensusNodeId>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        self.inner.lock().await.vote = Some(*vote);
        Ok(())
    }

    async fn read_vote(
        &mut self,
    ) -> Result<Option<Vote<ConsensusNodeId>>, StorageError<ConsensusNodeId>> {
        Ok(self.inner.lock().await.vote)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<MeshRaftConfig>,
    ) -> Result<(), StorageError<ConsensusNodeId>>
    where
        I: IntoIterator<Item = Entry<MeshRaftConfig>>,
    {
        let mut state = self.inner.lock().await;
        for entry in entries {
            state.log.insert(entry.log_id.index, entry);
        }
        drop(state);
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(
        &mut self,
        log_id: LogId<ConsensusNodeId>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        let mut state = self.inner.lock().await;
        let keys: Vec<_> = state
            .log
            .range(log_id.index..)
            .map(|(index, _)| *index)
            .collect();
        for index in keys {
            state.log.remove(&index);
        }
        Ok(())
    }

    async fn purge(
        &mut self,
        log_id: LogId<ConsensusNodeId>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        let mut state = self.inner.lock().await;
        state.last_purged_log_id = Some(log_id);
        let keys: Vec<_> = state
            .log
            .range(..=log_id.index)
            .map(|(index, _)| *index)
            .collect();
        for index in keys {
            state.log.remove(&index);
        }
        Ok(())
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsensusStateMachineData {
    pub last_applied_log: Option<LogId<ConsensusNodeId>>,
    pub last_membership: StoredMembership<ConsensusNodeId, BasicNode>,
    pub entries: Vec<ControlLogEntry>,
    pub command_results: BTreeMap<String, ConsensusResponse>,
}

#[derive(Debug)]
struct StoredConsensusSnapshot {
    meta: SnapshotMeta<ConsensusNodeId, BasicNode>,
    data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct MemoryConsensusStateMachine {
    state: RwLock<ConsensusStateMachineData>,
    snapshot_index: AtomicU64,
    current_snapshot: RwLock<Option<StoredConsensusSnapshot>>,
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

impl MemoryConsensusStateMachine {
    pub async fn state(&self) -> ConsensusStateMachineData {
        self.state.read().await.clone()
    }
}

impl RaftSnapshotBuilder<MeshRaftConfig> for Arc<MemoryConsensusStateMachine> {
    async fn build_snapshot(
        &mut self,
    ) -> Result<Snapshot<MeshRaftConfig>, StorageError<ConsensusNodeId>> {
        let state = self.state.read().await;
        let data = serde_json::to_vec(&*state)
            .map_err(|error| StorageIOError::read_state_machine(&error))?;
        let last_log_id = state.last_applied_log;
        let last_membership = state.last_membership.clone();
        let mut current_snapshot = self.current_snapshot.write().await;
        drop(state);

        let snapshot_index = self.snapshot_index.fetch_add(1, Ordering::Relaxed) + 1;
        let snapshot_id = last_log_id.map_or_else(
            || format!("empty-{snapshot_index}"),
            |log_id| format!("{}-{}-{snapshot_index}", log_id.leader_id, log_id.index),
        );
        let meta = SnapshotMeta {
            last_log_id,
            last_membership,
            snapshot_id,
        };
        *current_snapshot = Some(StoredConsensusSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        });
        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStateMachine<MeshRaftConfig> for Arc<MemoryConsensusStateMachine> {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<
        (
            Option<LogId<ConsensusNodeId>>,
            StoredMembership<ConsensusNodeId, BasicNode>,
        ),
        StorageError<ConsensusNodeId>,
    > {
        let state = self.state.read().await;
        Ok((state.last_applied_log, state.last_membership.clone()))
    }

    async fn apply<I>(
        &mut self,
        entries: I,
    ) -> Result<Vec<ConsensusResponse>, StorageError<ConsensusNodeId>>
    where
        I: IntoIterator<Item = Entry<MeshRaftConfig>> + Send,
    {
        let mut state = self.state.write().await;
        Ok(apply_consensus_entries(&mut state, entries))
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<MeshRaftConfig as RaftTypeConfig>::SnapshotData>, StorageError<ConsensusNodeId>>
    {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<ConsensusNodeId, BasicNode>,
        snapshot: Box<<MeshRaftConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        let data = snapshot.into_inner();
        let state: ConsensusStateMachineData = serde_json::from_slice(&data)
            .map_err(|error| StorageIOError::read_snapshot(Some(meta.signature()), &error))?;
        *self.state.write().await = state;
        *self.current_snapshot.write().await = Some(StoredConsensusSnapshot {
            meta: meta.clone(),
            data,
        });
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<MeshRaftConfig>>, StorageError<ConsensusNodeId>> {
        Ok(self
            .current_snapshot
            .read()
            .await
            .as_ref()
            .map(|snapshot| Snapshot {
                meta: snapshot.meta.clone(),
                snapshot: Box::new(Cursor::new(snapshot.data.clone())),
            }))
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

#[derive(Clone, Default)]
pub struct InProcessConsensusNetwork {
    peers: Arc<RwLock<BTreeMap<ConsensusNodeId, MeshRaft>>>,
}

impl InProcessConsensusNetwork {
    pub async fn register(&self, node_id: ConsensusNodeId, raft: MeshRaft) {
        self.peers.write().await.insert(node_id, raft);
    }

    pub async fn remove(&self, node_id: ConsensusNodeId) {
        self.peers.write().await.remove(&node_id);
    }
}

impl RaftNetworkFactory<MeshRaftConfig> for InProcessConsensusNetwork {
    type Network = InProcessConsensusConnection;

    async fn new_client(&mut self, target: ConsensusNodeId, _node: &BasicNode) -> Self::Network {
        InProcessConsensusConnection {
            target,
            peers: Arc::clone(&self.peers),
        }
    }
}

pub struct InProcessConsensusConnection {
    target: ConsensusNodeId,
    peers: Arc<RwLock<BTreeMap<ConsensusNodeId, MeshRaft>>>,
}

impl InProcessConsensusConnection {
    async fn target(&self) -> Result<MeshRaft, MeshRpcError> {
        self.peers
            .read()
            .await
            .get(&self.target)
            .cloned()
            .ok_or_else(|| {
                RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "consensus target unavailable",
                )))
            })
    }
}

impl RaftNetwork<MeshRaftConfig> for InProcessConsensusConnection {
    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<MeshRaftConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<ConsensusNodeId>, MeshRpcError> {
        self.target()
            .await?
            .append_entries(request)
            .await
            .map_err(|error| RPCError::RemoteError(RemoteError::new(self.target, error)))
    }

    async fn install_snapshot(
        &mut self,
        request: InstallSnapshotRequest<MeshRaftConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<ConsensusNodeId>, MeshRpcError<InstallSnapshotError>> {
        let target = self
            .peers
            .read()
            .await
            .get(&self.target)
            .cloned()
            .ok_or_else(|| {
                RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "consensus target unavailable",
                )))
            })?;
        target
            .install_snapshot(request)
            .await
            .map_err(|error| RPCError::RemoteError(RemoteError::new(self.target, error)))
    }

    async fn vote(
        &mut self,
        request: VoteRequest<ConsensusNodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<ConsensusNodeId>, MeshRpcError> {
        self.target()
            .await?
            .vote(request)
            .await
            .map_err(|error| RPCError::RemoteError(RemoteError::new(self.target, error)))
    }
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
pub struct EmbeddedConsensusNode {
    pub node_id: ConsensusNodeId,
    pub raft: MeshRaft,
    pub log_store: MemoryRaftLogStore,
    pub state_machine: Arc<MemoryConsensusStateMachine>,
}

pub trait ConsensusNodeHandle {
    fn consensus_node_id(&self) -> ConsensusNodeId;
    fn consensus_raft(&self) -> &MeshRaft;
}

impl ConsensusNodeHandle for EmbeddedConsensusNode {
    fn consensus_node_id(&self) -> ConsensusNodeId {
        self.node_id
    }

    fn consensus_raft(&self) -> &MeshRaft {
        &self.raft
    }
}

#[derive(Clone)]
pub struct DurableEmbeddedConsensusNode {
    pub node_id: ConsensusNodeId,
    pub raft: MeshRaft,
    pub log_store: DurableConsensusLogStore,
    pub state_machine: DurableConsensusStateMachine,
}

impl ConsensusNodeHandle for DurableEmbeddedConsensusNode {
    fn consensus_node_id(&self) -> ConsensusNodeId {
        self.node_id
    }

    fn consensus_raft(&self) -> &MeshRaft {
        &self.raft
    }
}

fn validated_consensus_config(cluster_name: &str) -> Result<Arc<Config>, String> {
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

pub async fn start_in_process_consensus_node(
    node_id: ConsensusNodeId,
    network: InProcessConsensusNetwork,
    cluster_name: &str,
) -> Result<EmbeddedConsensusNode, String> {
    if node_id == 0 || cluster_name.trim().is_empty() {
        return Err("consensus_node_configuration_invalid".to_string());
    }
    let config = validated_consensus_config(cluster_name)?;
    let log_store = MemoryRaftLogStore::default();
    let state_machine = Arc::new(MemoryConsensusStateMachine::default());
    let raft = MeshRaft::new(
        node_id,
        config,
        network,
        log_store.clone(),
        Arc::clone(&state_machine),
    )
    .await
    .map_err(|error| format!("consensus_start_failed:{error}"))?;
    Ok(EmbeddedConsensusNode {
        node_id,
        raft,
        log_store,
        state_machine,
    })
}

pub async fn start_durable_consensus_node(
    node_id: ConsensusNodeId,
    network: InProcessConsensusNetwork,
    cluster_name: &str,
    path: &Path,
) -> Result<DurableEmbeddedConsensusNode, String> {
    if node_id == 0 || cluster_name.trim().is_empty() {
        return Err("consensus_node_configuration_invalid".to_string());
    }
    let config = validated_consensus_config(cluster_name)?;
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
    Ok(DurableEmbeddedConsensusNode {
        node_id,
        raft,
        log_store,
        state_machine,
    })
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
    let autonomous = std::env::var("MESH_CLUSTER_MODE")
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("autonomous"));
    let controller = std::env::var("MESH_ROLES")
        .unwrap_or_default()
        .split(',')
        .any(|role| role.trim().eq_ignore_ascii_case("controller"));
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

pub async fn wait_for_consensus_leader<N: ConsensusNodeHandle>(
    nodes: &[N],
    timeout: Duration,
) -> Result<ConsensusNodeId, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let leaders: BTreeSet<_> = nodes
            .iter()
            .filter_map(|node| {
                let metrics = node.consensus_raft().metrics();
                let metrics = metrics.borrow();
                (metrics.state == openraft::ServerState::Leader
                    && metrics.current_leader == Some(node.consensus_node_id()))
                .then_some(node.consensus_node_id())
            })
            .collect();
        if leaders.len() == 1 {
            return Ok(*leaders.iter().next().expect("one leader"));
        }
        if tokio::time::Instant::now() >= deadline {
            let states = nodes
                .iter()
                .map(|node| {
                    let metrics = node.consensus_raft().metrics();
                    let metrics = metrics.borrow();
                    format!(
                        "{}:{:?}:term={}:leader={:?}:membership={:?}",
                        node.consensus_node_id(),
                        metrics.state,
                        metrics.current_term,
                        metrics.current_leader,
                        metrics.membership_config.membership()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            return Err(format!("consensus_leader_timeout:{states}"));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::scaling::DesiredCapacity;
    use super::*;

    fn command(command_id: &str, workers: u16) -> ConsensusCommand {
        ConsensusCommand {
            command_id: command_id.to_string(),
            actor: "autoscaler".to_string(),
            reason: "test desired capacity".to_string(),
            timestamp_unix_millis: 1,
            actor_sequence: 0,
            mutation: ControlMutation::DesiredCapacity(DesiredCapacity {
                revision: super::super::scaling::DesiredRevision(u64::from(workers)),
                worker_nodes: workers,
                gateway_nodes: 0,
                template_revision: "v1".to_string(),
            }),
        }
    }

    async fn wait_for_entries(
        nodes: &[&EmbeddedConsensusNode],
        count: usize,
    ) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let mut complete = true;
            for node in nodes {
                complete &= node.state_machine.state().await.entries.len() >= count;
            }
            if complete {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("consensus_replication_timeout".to_string());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn three_voter_consensus_replicates_and_survives_leader_failure() {
        let network = InProcessConsensusNetwork::default();
        let mut nodes = Vec::new();
        for node_id in 101..=103 {
            let node =
                start_in_process_consensus_node(node_id, network.clone(), "mesh-consensus-test")
                    .await
                    .expect("start consensus node");
            network.register(node_id, node.raft.clone()).await;
            nodes.push(node);
        }
        let members = BTreeMap::from([
            (101, BasicNode::new("node-101")),
            (102, BasicNode::new("node-102")),
            (103, BasicNode::new("node-103")),
        ]);
        nodes[0]
            .raft
            .initialize(members)
            .await
            .expect("initialize three voters");

        let first_leader = wait_for_consensus_leader(&nodes, Duration::from_secs(5))
            .await
            .expect("first leader");
        let first = nodes
            .iter()
            .find(|node| node.node_id == first_leader)
            .expect("leader node")
            .raft
            .client_write(command("command-1", 3))
            .await
            .expect("first majority write");
        assert!(first.data.applied);
        wait_for_entries(&nodes.iter().collect::<Vec<_>>(), 1)
            .await
            .expect("replicate first command");

        let failed_index = nodes
            .iter()
            .position(|node| node.node_id == first_leader)
            .expect("failed leader index");
        nodes[failed_index]
            .raft
            .shutdown()
            .await
            .expect("shutdown leader");
        network.remove(first_leader).await;
        let live: Vec<_> = nodes
            .iter()
            .filter(|node| node.node_id != first_leader)
            .cloned()
            .collect();
        // A follower with a committed leader vote waits the configured leader
        // lease (3s) plus its randomized election timeout (up to 3s), then the
        // next 750ms tick. Keep the assertion above that real upper bound.
        let next_leader = wait_for_consensus_leader(&live, Duration::from_secs(8))
            .await
            .expect("replacement leader");
        assert_ne!(next_leader, first_leader);
        let second = live
            .iter()
            .find(|node| node.node_id == next_leader)
            .expect("replacement leader node")
            .raft
            .client_write(command("command-2", 2))
            .await
            .expect("second majority write");
        assert!(second.data.applied);
        assert!(second.data.control_term > first.data.control_term);
        wait_for_entries(&live.iter().collect::<Vec<_>>(), 2)
            .await
            .expect("replicate second command");

        for node in &live {
            let state = node.state_machine.state().await;
            assert_eq!(state.entries.len(), 2);
            assert_eq!(
                state
                    .command_results
                    .keys()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["command-1".to_string(), "command-2".to_string()])
            );
        }
        for node in live {
            node.raft.shutdown().await.expect("shutdown live node");
        }
    }

    #[tokio::test]
    async fn state_machine_deduplicates_a_retried_command_id() {
        let mut state_machine = Arc::new(MemoryConsensusStateMachine::default());
        let leader_id = openraft::CommittedLeaderId::new(7, 1);
        let entries = vec![
            Entry {
                log_id: LogId::new(leader_id, 1),
                payload: EntryPayload::Normal(command("same-command", 2)),
            },
            Entry {
                log_id: LogId::new(leader_id, 2),
                payload: EntryPayload::Normal(command("same-command", 2)),
            },
        ];

        let responses = state_machine.apply(entries).await.expect("apply entries");

        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0], responses[1]);
        assert_eq!(state_machine.state().await.entries.len(), 1);
    }
}
