//! Crash-durable OpenRaft log and state-machine storage backed by redb.

use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openraft::storage::{LogFlushed, RaftLogStorage, RaftStateMachine, Snapshot};
use openraft::{
    BasicNode, Entry, LogId, LogState, RaftLogId, RaftLogReader, RaftSnapshotBuilder,
    RaftTypeConfig, SnapshotMeta, StorageError, StorageIOError, StoredMembership, Vote,
};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use super::consensus::{
    apply_consensus_entries, ConsensusNodeId, ConsensusResponse, ConsensusStateMachineData,
    MeshRaftConfig,
};

const META_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("mesh_raft_meta_v1");
const LOG_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("mesh_raft_log_v1");
const META_LAST_PURGED: &str = "last_purged";
const META_COMMITTED: &str = "committed";
const META_VOTE: &str = "vote";
const META_STATE_MACHINE: &str = "state_machine";
const META_SNAPSHOT_META: &str = "snapshot_meta";
const META_SNAPSHOT_DATA: &str = "snapshot_data";
const META_SNAPSHOT_INDEX: &str = "snapshot_index";

#[derive(Clone)]
struct DurableConsensusDatabase {
    database: Arc<Database>,
    path: PathBuf,
}

impl std::fmt::Debug for DurableConsensusDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableConsensusDatabase")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl DurableConsensusDatabase {
    fn open(path: &Path) -> Result<Self, String> {
        if path.as_os_str().is_empty() {
            return Err("consensus_store_path_missing".to_string());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("consensus_store_directory_failed:{error}"))?;
        }
        let database = Database::create(path)
            .map_err(|error| format!("consensus_store_open_failed:{error}"))?;
        let write = database
            .begin_write()
            .map_err(|error| format!("consensus_store_initialize_failed:{error}"))?;
        {
            write
                .open_table(META_TABLE)
                .map_err(|error| format!("consensus_store_meta_table_failed:{error}"))?;
            write
                .open_table(LOG_TABLE)
                .map_err(|error| format!("consensus_store_log_table_failed:{error}"))?;
        }
        write
            .commit()
            .map_err(|error| format!("consensus_store_initialize_commit_failed:{error}"))?;
        secure_consensus_store(path)?;
        Ok(Self {
            database: Arc::new(database),
            path: path.to_path_buf(),
        })
    }

    fn read_meta<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, StorageError<ConsensusNodeId>> {
        let read = self.database.begin_read().map_err(storage_read_error)?;
        let table = read.open_table(META_TABLE).map_err(storage_read_error)?;
        let bytes = table
            .get(key)
            .map_err(storage_read_error)?
            .map(|value| value.value().to_vec());
        bytes
            .map(|bytes| serde_json::from_slice(&bytes).map_err(storage_read_error))
            .transpose()
    }

    fn read_raw_meta(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError<ConsensusNodeId>> {
        let read = self.database.begin_read().map_err(storage_read_error)?;
        let table = read.open_table(META_TABLE).map_err(storage_read_error)?;
        Ok(table
            .get(key)
            .map_err(storage_read_error)?
            .map(|value| value.value().to_vec()))
    }

    fn write_meta<T: serde::Serialize>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        let encoded = serde_json::to_vec(value).map_err(storage_write_error)?;
        let write = self.database.begin_write().map_err(storage_write_error)?;
        {
            let mut table = write.open_table(META_TABLE).map_err(storage_write_error)?;
            table
                .insert(key, encoded.as_slice())
                .map_err(storage_write_error)?;
        }
        write.commit().map_err(storage_write_error)
    }

    fn read_state(&self) -> Result<ConsensusStateMachineData, StorageError<ConsensusNodeId>> {
        Ok(self.read_meta(META_STATE_MACHINE)?.unwrap_or_default())
    }

    fn write_state(
        &self,
        state: &ConsensusStateMachineData,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        self.write_meta(META_STATE_MACHINE, state)
    }
}

fn storage_read_error(error: impl std::fmt::Display) -> StorageError<ConsensusNodeId> {
    let error = std::io::Error::other(error.to_string());
    StorageIOError::read(&error).into()
}

fn storage_write_error(error: impl std::fmt::Display) -> StorageError<ConsensusNodeId> {
    let error = std::io::Error::other(error.to_string());
    StorageIOError::write(&error).into()
}

#[cfg(unix)]
fn secure_consensus_store(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("consensus_store_permissions_failed:{error}"))
}

#[cfg(not(unix))]
fn secure_consensus_store(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[derive(Clone, Debug)]
pub struct DurableConsensusLogStore {
    inner: DurableConsensusDatabase,
}

#[derive(Clone, Debug)]
pub struct DurableConsensusStateMachine {
    inner: DurableConsensusDatabase,
}

pub fn open_durable_consensus_store(
    path: &Path,
) -> Result<(DurableConsensusLogStore, DurableConsensusStateMachine), String> {
    let inner = DurableConsensusDatabase::open(path)?;
    Ok((
        DurableConsensusLogStore {
            inner: inner.clone(),
        },
        DurableConsensusStateMachine { inner },
    ))
}

impl DurableConsensusStateMachine {
    pub fn state(&self) -> Result<ConsensusStateMachineData, String> {
        self.inner
            .read_state()
            .map_err(|error| format!("consensus_store_state_read_failed:{error}"))
    }
}

impl RaftLogReader<MeshRaftConfig> for DurableConsensusLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<MeshRaftConfig>>, StorageError<ConsensusNodeId>> {
        let read = self
            .inner
            .database
            .begin_read()
            .map_err(storage_read_error)?;
        let table = read.open_table(LOG_TABLE).map_err(storage_read_error)?;
        let mut entries = Vec::new();
        for item in table.range(range).map_err(storage_read_error)? {
            let (_, value) = item.map_err(storage_read_error)?;
            entries.push(serde_json::from_slice(value.value()).map_err(storage_read_error)?);
        }
        Ok(entries)
    }
}

impl RaftLogStorage<MeshRaftConfig> for DurableConsensusLogStore {
    type LogReader = Self;

    async fn get_log_state(
        &mut self,
    ) -> Result<LogState<MeshRaftConfig>, StorageError<ConsensusNodeId>> {
        let last_purged_log_id = self.inner.read_meta(META_LAST_PURGED)?;
        let read = self
            .inner
            .database
            .begin_read()
            .map_err(storage_read_error)?;
        let table = read.open_table(LOG_TABLE).map_err(storage_read_error)?;
        let last_log_id = table
            .last()
            .map_err(storage_read_error)?
            .map(|(_, value)| {
                serde_json::from_slice::<Entry<MeshRaftConfig>>(value.value())
                    .map(|entry| *entry.get_log_id())
                    .map_err(storage_read_error)
            })
            .transpose()?
            .or(last_purged_log_id);
        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<ConsensusNodeId>>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        self.inner.write_meta(META_COMMITTED, &committed)
    }

    async fn read_committed(
        &mut self,
    ) -> Result<Option<LogId<ConsensusNodeId>>, StorageError<ConsensusNodeId>> {
        Ok(self.inner.read_meta(META_COMMITTED)?.flatten())
    }

    async fn save_vote(
        &mut self,
        vote: &Vote<ConsensusNodeId>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        self.inner.write_meta(META_VOTE, vote)
    }

    async fn read_vote(
        &mut self,
    ) -> Result<Option<Vote<ConsensusNodeId>>, StorageError<ConsensusNodeId>> {
        self.inner.read_meta(META_VOTE)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<MeshRaftConfig>,
    ) -> Result<(), StorageError<ConsensusNodeId>>
    where
        I: IntoIterator<Item = Entry<MeshRaftConfig>>,
    {
        let entries: Vec<_> = entries.into_iter().collect();
        let write = self
            .inner
            .database
            .begin_write()
            .map_err(storage_write_error)?;
        {
            let mut table = write.open_table(LOG_TABLE).map_err(storage_write_error)?;
            for entry in &entries {
                let encoded = serde_json::to_vec(entry).map_err(storage_write_error)?;
                table
                    .insert(&entry.log_id.index, encoded.as_slice())
                    .map_err(storage_write_error)?;
            }
        }
        write.commit().map_err(storage_write_error)?;
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(
        &mut self,
        log_id: LogId<ConsensusNodeId>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        let write = self
            .inner
            .database
            .begin_write()
            .map_err(storage_write_error)?;
        {
            let mut table = write.open_table(LOG_TABLE).map_err(storage_write_error)?;
            let keys: Vec<_> = table
                .range(log_id.index..)
                .map_err(storage_read_error)?
                .map(|item| item.map(|(key, _)| key.value()).map_err(storage_read_error))
                .collect::<Result<_, _>>()?;
            for key in keys {
                table.remove(&key).map_err(storage_write_error)?;
            }
        }
        write.commit().map_err(storage_write_error)
    }

    async fn purge(
        &mut self,
        log_id: LogId<ConsensusNodeId>,
    ) -> Result<(), StorageError<ConsensusNodeId>> {
        let encoded_log_id = serde_json::to_vec(&log_id).map_err(storage_write_error)?;
        let write = self
            .inner
            .database
            .begin_write()
            .map_err(storage_write_error)?;
        {
            let mut logs = write.open_table(LOG_TABLE).map_err(storage_write_error)?;
            let keys: Vec<_> = logs
                .range(..=log_id.index)
                .map_err(storage_read_error)?
                .map(|item| item.map(|(key, _)| key.value()).map_err(storage_read_error))
                .collect::<Result<_, _>>()?;
            for key in keys {
                logs.remove(&key).map_err(storage_write_error)?;
            }
        }
        {
            let mut meta = write.open_table(META_TABLE).map_err(storage_write_error)?;
            meta.insert(META_LAST_PURGED, encoded_log_id.as_slice())
                .map_err(storage_write_error)?;
        }
        write.commit().map_err(storage_write_error)
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }
}

impl RaftSnapshotBuilder<MeshRaftConfig> for DurableConsensusStateMachine {
    async fn build_snapshot(
        &mut self,
    ) -> Result<Snapshot<MeshRaftConfig>, StorageError<ConsensusNodeId>> {
        let state = self.inner.read_state()?;
        let data = serde_json::to_vec(&state).map_err(storage_read_error)?;
        let next_index = self
            .inner
            .read_meta::<u64>(META_SNAPSHOT_INDEX)?
            .unwrap_or(0)
            .saturating_add(1);
        let snapshot_id = state.last_applied_log.map_or_else(
            || format!("empty-{next_index}"),
            |log_id| format!("{}-{}-{next_index}", log_id.leader_id, log_id.index),
        );
        let meta = SnapshotMeta {
            last_log_id: state.last_applied_log,
            last_membership: state.last_membership,
            snapshot_id,
        };
        let encoded_meta = serde_json::to_vec(&meta).map_err(storage_write_error)?;
        let encoded_index = serde_json::to_vec(&next_index).map_err(storage_write_error)?;
        let write = self
            .inner
            .database
            .begin_write()
            .map_err(storage_write_error)?;
        {
            let mut table = write.open_table(META_TABLE).map_err(storage_write_error)?;
            table
                .insert(META_SNAPSHOT_META, encoded_meta.as_slice())
                .map_err(storage_write_error)?;
            table
                .insert(META_SNAPSHOT_DATA, data.as_slice())
                .map_err(storage_write_error)?;
            table
                .insert(META_SNAPSHOT_INDEX, encoded_index.as_slice())
                .map_err(storage_write_error)?;
        }
        write.commit().map_err(storage_write_error)?;
        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStateMachine<MeshRaftConfig> for DurableConsensusStateMachine {
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
        let state = self.inner.read_state()?;
        Ok((state.last_applied_log, state.last_membership))
    }

    async fn apply<I>(
        &mut self,
        entries: I,
    ) -> Result<Vec<ConsensusResponse>, StorageError<ConsensusNodeId>>
    where
        I: IntoIterator<Item = Entry<MeshRaftConfig>> + Send,
    {
        let mut state = self.inner.read_state()?;
        let responses = apply_consensus_entries(&mut state, entries);
        self.inner.write_state(&state)?;
        Ok(responses)
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
        let mut state: ConsensusStateMachineData =
            serde_json::from_slice(&data).map_err(storage_read_error)?;
        state.last_applied_log = meta.last_log_id;
        state.last_membership = meta.last_membership.clone();
        let encoded_state = serde_json::to_vec(&state).map_err(storage_write_error)?;
        let encoded_meta = serde_json::to_vec(meta).map_err(storage_write_error)?;
        let write = self
            .inner
            .database
            .begin_write()
            .map_err(storage_write_error)?;
        {
            let mut table = write.open_table(META_TABLE).map_err(storage_write_error)?;
            table
                .insert(META_STATE_MACHINE, encoded_state.as_slice())
                .map_err(storage_write_error)?;
            table
                .insert(META_SNAPSHOT_META, encoded_meta.as_slice())
                .map_err(storage_write_error)?;
            table
                .insert(META_SNAPSHOT_DATA, data.as_slice())
                .map_err(storage_write_error)?;
        }
        write.commit().map_err(storage_write_error)
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<MeshRaftConfig>>, StorageError<ConsensusNodeId>> {
        let meta: Option<SnapshotMeta<ConsensusNodeId, BasicNode>> =
            self.inner.read_meta(META_SNAPSHOT_META)?;
        let data = self.inner.read_raw_meta(META_SNAPSHOT_DATA)?;
        match (meta, data) {
            (None, None) => Ok(None),
            (Some(meta), Some(data)) => Ok(Some(Snapshot {
                meta,
                snapshot: Box::new(Cursor::new(data)),
            })),
            _ => Err(storage_read_error("consensus snapshot is incomplete")),
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dist::consensus::{
        start_durable_consensus_node, wait_for_consensus_leader, ConsensusCommand,
        InProcessConsensusNetwork,
    };
    use crate::dist::scaling::{ControlMutation, DesiredCapacity, DesiredRevision};
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn command(id: &str, workers: u16) -> ConsensusCommand {
        ConsensusCommand {
            command_id: id.to_string(),
            actor: "durability-test".to_string(),
            reason: "persist consensus state".to_string(),
            timestamp_unix_millis: 1,
            actor_sequence: 0,
            mutation: ControlMutation::DesiredCapacity(DesiredCapacity {
                revision: DesiredRevision(u64::from(workers)),
                worker_nodes: workers,
                gateway_nodes: 0,
                template_revision: "v1".to_string(),
            }),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn durable_consensus_recovers_vote_log_membership_and_state_machine() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("controller.redb");
        let network = InProcessConsensusNetwork::default();
        let node = start_durable_consensus_node(501, network.clone(), "durable-test", &path)
            .await
            .expect("start durable node");
        network.register(501, node.raft.clone()).await;
        node.raft
            .initialize(BTreeMap::from([(501, BasicNode::new("node-501"))]))
            .await
            .expect("initialize one voter");
        wait_for_consensus_leader(std::slice::from_ref(&node), Duration::from_secs(5))
            .await
            .expect("first leader");
        node.raft
            .client_write(command("durable-command-1", 2))
            .await
            .expect("first durable command");
        node.raft.shutdown().await.expect("shutdown first node");
        network.remove(501).await;
        drop(node);

        let restarted = start_durable_consensus_node(501, network.clone(), "durable-test", &path)
            .await
            .expect("restart durable node");
        network.register(501, restarted.raft.clone()).await;
        wait_for_consensus_leader(std::slice::from_ref(&restarted), Duration::from_secs(5))
            .await
            .expect("leader after restart");
        let recovered = restarted
            .state_machine
            .state()
            .expect("read recovered state");
        assert_eq!(recovered.entries.len(), 1);
        assert!(recovered.command_results.contains_key("durable-command-1"));
        restarted
            .raft
            .client_write(command("durable-command-2", 3))
            .await
            .expect("write after restart");
        assert_eq!(
            restarted
                .state_machine
                .state()
                .expect("state after restart write")
                .entries
                .len(),
            2
        );
        restarted.raft.shutdown().await.expect("shutdown restart");
    }

    #[tokio::test]
    async fn durable_snapshot_round_trips_raw_state_bytes() {
        let source_directory = tempfile::tempdir().expect("source tempdir");
        let (_, mut source) =
            open_durable_consensus_store(&source_directory.path().join("source.redb"))
                .expect("source store");
        let leader = openraft::CommittedLeaderId::new(3, 9);
        source
            .apply(vec![Entry {
                log_id: LogId::new(leader, 1),
                payload: openraft::EntryPayload::Normal(command("snapshot-command", 4)),
            }])
            .await
            .expect("apply source state");
        let built = source.build_snapshot().await.expect("build snapshot");
        let current = source
            .get_current_snapshot()
            .await
            .expect("read current snapshot")
            .expect("snapshot exists");
        assert_eq!(
            built.snapshot.into_inner(),
            current.snapshot.get_ref().clone()
        );

        let target_directory = tempfile::tempdir().expect("target tempdir");
        let (_, mut target) =
            open_durable_consensus_store(&target_directory.path().join("target.redb"))
                .expect("target store");
        target
            .install_snapshot(&current.meta, current.snapshot)
            .await
            .expect("install snapshot");

        let state = target.state().expect("installed state");
        assert_eq!(state.entries.len(), 1);
        assert!(state.command_results.contains_key("snapshot-command"));
    }
}
