//! An in-process transport and in-memory stores for OpenRaft conformance
//! tests of Mesh's consensus types. Production controllers run the durable
//! store over Mesh's authenticated peer sessions instead.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use openraft::error::{InstallSnapshotError, RPCError, RemoteError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::storage::{LogFlushed, RaftLogStorage, RaftStateMachine, Snapshot};
use openraft::{
    BasicNode, Entry, LogId, LogState, RaftLogId, RaftLogReader, RaftSnapshotBuilder,
    RaftTypeConfig, SnapshotMeta, StorageError, StorageIOError, StoredMembership, Vote,
};
use tokio::sync::{Mutex, RwLock};

use super::consensus::{
    apply_consensus_entries, validated_consensus_config, ConsensusNodeId, ConsensusResponse,
    ConsensusStateMachineData, DurableEmbeddedConsensusNode, MeshRaft, MeshRaftConfig,
};
use super::consensus_store::open_durable_consensus_store;

type MeshRpcError<E = openraft::error::Infallible> =
    RPCError<ConsensusNodeId, BasicNode, openraft::error::RaftError<ConsensusNodeId, E>>;

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

#[derive(Clone)]
pub struct EmbeddedConsensusNode {
    pub node_id: ConsensusNodeId,
    pub raft: MeshRaft,
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

impl ConsensusNodeHandle for DurableEmbeddedConsensusNode {
    fn consensus_node_id(&self) -> ConsensusNodeId {
        self.node_id
    }

    fn consensus_raft(&self) -> &MeshRaft {
        &self.raft
    }
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
    let state_machine = Arc::new(MemoryConsensusStateMachine::default());
    let raft = MeshRaft::new(
        node_id,
        config,
        network,
        MemoryRaftLogStore::default(),
        Arc::clone(&state_machine),
    )
    .await
    .map_err(|error| format!("consensus_start_failed:{error}"))?;
    Ok(EmbeddedConsensusNode {
        node_id,
        raft,
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
    use super::super::consensus::ConsensusCommand;
    use super::super::scaling::{ControlMutation, DesiredCapacity};
    use super::*;
    use openraft::EntryPayload;

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
