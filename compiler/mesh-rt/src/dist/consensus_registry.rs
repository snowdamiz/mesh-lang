//! Internal Raft group registration. Session authorization stays separate from wire selectors.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use openraft::RaftTypeConfig;
use serde::{Deserialize, Serialize};

use super::{ConsensusNodeId, DurableConsensusStateMachine, MeshConsensusRpcServer, MeshRaft};

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum RaftGroup {
    #[default]
    Control,
    Jobs(String),
}

impl RaftGroup {
    fn validate(&self) -> Result<(), String> {
        if let Self::Jobs(deployment) = self {
            if deployment.trim().is_empty() || deployment.len() > 256 {
                return Err("consensus_rpc_deployment_invalid".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct RaftRpcIdentity {
    pub(crate) cluster_name: String,
    pub(crate) node_id: ConsensusNodeId,
    pub(crate) node_name: String,
}

/// Fields come from an authenticated session, never from the request envelope.
/// Jobs has no production constructor until its session capability is negotiated.
pub(crate) struct AuthorizedRaftPeer<'a> {
    group: RaftGroup,
    remote_name: &'a str,
    local_name: &'a str,
}

impl<'a> AuthorizedRaftPeer<'a> {
    pub(super) fn control(
        autonomous_enabled: bool,
        remote_name: &'a str,
        local_name: &'a str,
    ) -> Result<Self, String> {
        if !autonomous_enabled {
            return Err("consensus_rpc_capability_unavailable".to_string());
        }
        Ok(Self {
            group: RaftGroup::Control,
            remote_name,
            local_name,
        })
    }

    pub(super) fn validate(
        &self,
        group: &RaftGroup,
        identity: &RaftRpcIdentity,
        cluster_name: &str,
        source_id: ConsensusNodeId,
        source_name: &str,
        target_id: ConsensusNodeId,
    ) -> Result<(), String> {
        group.validate()?;
        if group != &self.group {
            return Err("consensus_rpc_group_mismatch".to_string());
        }
        if cluster_name != identity.cluster_name
            || target_id != identity.node_id
            || source_id == 0
            || source_name != self.remote_name
            || identity.node_name != self.local_name
        {
            return Err("consensus_rpc_identity_mismatch".to_string());
        }
        Ok(())
    }
}

struct RegisteredRaft {
    incarnation: Arc<()>,
    identity: RaftRpcIdentity,
    raft: Arc<dyn Any + Send + Sync>,
    runtime: tokio::runtime::Handle,
    control_state_machine: Option<DurableConsensusStateMachine>,
}

/// Removal is conditional on this exact registration, including across registries.
#[derive(Clone)]
pub(crate) struct RaftRegistration {
    group: RaftGroup,
    incarnation: Arc<()>,
}

#[derive(Default)]
pub(crate) struct RaftRpcRegistry {
    groups: RwLock<BTreeMap<RaftGroup, RegisteredRaft>>,
}

impl RaftRpcRegistry {
    fn insert<C: RaftTypeConfig<NodeId = ConsensusNodeId>>(
        &self,
        group: RaftGroup,
        identity: RaftRpcIdentity,
        raft: openraft::Raft<C>,
        runtime: tokio::runtime::Handle,
        control_state_machine: Option<DurableConsensusStateMachine>,
    ) -> Result<RaftRegistration, String> {
        group.validate()?;
        if identity.cluster_name.trim().is_empty()
            || identity.cluster_name.len() > 256
            || identity.node_id == 0
            || identity.node_name.trim().is_empty()
            || identity.node_name.len() > 512
            || identity.node_id != raft.metrics().borrow().id
            || identity.cluster_name != raft.config().cluster_name
        {
            return Err("consensus_node_configuration_invalid".to_string());
        }
        let mut groups = self
            .groups
            .write()
            .map_err(|_| "consensus_rpc_server_lock_poisoned".to_string())?;
        if groups.contains_key(&group) {
            return Err("consensus_rpc_group_already_registered".to_string());
        }
        let incarnation = Arc::new(());
        groups.insert(
            group.clone(),
            RegisteredRaft {
                incarnation: incarnation.clone(),
                identity,
                raft: Arc::new(raft),
                runtime,
                control_state_machine,
            },
        );
        Ok(RaftRegistration { group, incarnation })
    }

    pub(super) fn register_control(
        &self,
        server: MeshConsensusRpcServer,
    ) -> Result<RaftRegistration, String> {
        self.insert(
            RaftGroup::Control,
            server.identity(),
            server.raft.clone(),
            server.runtime.clone(),
            Some(server.state_machine),
        )
    }

    pub(crate) fn register_jobs<C: RaftTypeConfig<NodeId = ConsensusNodeId>>(
        &self,
        deployment: &str,
        identity: RaftRpcIdentity,
        raft: openraft::Raft<C>,
        runtime: tokio::runtime::Handle,
    ) -> Result<RaftRegistration, String> {
        self.insert(
            RaftGroup::Jobs(deployment.to_string()),
            identity,
            raft,
            runtime,
            None,
        )
    }

    pub(crate) fn unregister(&self, registration: &RaftRegistration) -> Result<bool, String> {
        let mut groups = self
            .groups
            .write()
            .map_err(|_| "consensus_rpc_server_lock_poisoned".to_string())?;
        let current = groups
            .get(&registration.group)
            .is_some_and(|entry| Arc::ptr_eq(&entry.incarnation, &registration.incarnation));
        if current {
            groups.remove(&registration.group);
        }
        Ok(current)
    }

    pub(super) fn control(&self) -> Result<Option<MeshConsensusRpcServer>, String> {
        let groups = self
            .groups
            .read()
            .map_err(|_| "consensus_rpc_server_lock_poisoned".to_string())?;
        Ok(groups
            .get(&RaftGroup::Control)
            .map(|entry| MeshConsensusRpcServer {
                cluster_name: entry.identity.cluster_name.clone(),
                node_id: entry.identity.node_id,
                node_name: entry.identity.node_name.clone(),
                raft: entry
                    .raft
                    .downcast_ref::<MeshRaft>()
                    .expect("Control has MeshRaftConfig")
                    .clone(),
                state_machine: entry
                    .control_state_machine
                    .clone()
                    .expect("Control has a state machine"),
                runtime: entry.runtime.clone(),
            }))
    }

    /// Resolves only a real Raft handle of the expected config after identity validation.
    /// A request resolved before unregister may finish on that old handle.
    pub(crate) fn resolve<C: RaftTypeConfig<NodeId = ConsensusNodeId>>(
        &self,
        peer: &AuthorizedRaftPeer<'_>,
        group: &RaftGroup,
        cluster_name: &str,
        source_id: ConsensusNodeId,
        source_name: &str,
        target_id: ConsensusNodeId,
    ) -> Result<(openraft::Raft<C>, tokio::runtime::Handle), String> {
        if group != &peer.group {
            return Err("consensus_rpc_group_mismatch".to_string());
        }
        let groups = self
            .groups
            .read()
            .map_err(|_| "consensus_rpc_server_lock_poisoned".to_string())?;
        let entry = groups
            .get(group)
            .ok_or_else(|| "consensus_rpc_server_unavailable".to_string())?;
        peer.validate(
            group,
            &entry.identity,
            cluster_name,
            source_id,
            source_name,
            target_id,
        )?;
        let raft = entry
            .raft
            .downcast_ref::<openraft::Raft<C>>()
            .ok_or_else(|| "consensus_rpc_config_mismatch".to_string())?;
        Ok((raft.clone(), entry.runtime.clone()))
    }
}

pub(crate) fn raft_rpc_registry() -> &'static RaftRpcRegistry {
    static REGISTRY: OnceLock<RaftRpcRegistry> = OnceLock::new();
    REGISTRY.get_or_init(RaftRpcRegistry::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dist::consensus::{ConsensusCommand, DurableEmbeddedConsensusNode, MeshRaftConfig};
    use crate::dist::consensus_testing::{start_durable_consensus_node, InProcessConsensusNetwork};
    use crate::dist::scaling::{ControlMutation, DesiredCapacity, DesiredRevision};
    use openraft::raft::AppendEntriesRequest;
    use openraft::storage::RaftLogStorage;
    use openraft::{CommittedLeaderId, Entry, EntryPayload, LogId, Vote};
    use std::io::Cursor;
    use std::time::Duration;

    openraft::declare_raft_types!(OtherConfig: D = String, R = ());

    fn identity() -> RaftRpcIdentity {
        RaftRpcIdentity {
            cluster_name: "cluster".to_string(),
            node_id: 9,
            node_name: "local@host:4370".to_string(),
        }
    }

    fn peer(group: RaftGroup) -> AuthorizedRaftPeer<'static> {
        AuthorizedRaftPeer {
            group,
            remote_name: "peer@host:4370",
            local_name: "local@host:4370",
        }
    }

    async fn node(path: &std::path::Path) -> DurableEmbeddedConsensusNode {
        start_durable_consensus_node(9, InProcessConsensusNetwork::default(), "cluster", path)
            .await
            .unwrap()
    }

    fn resolve(registry: &RaftRpcRegistry, group: &RaftGroup) -> super::super::MeshRaft {
        registry
            .resolve::<MeshRaftConfig>(
                &peer(group.clone()),
                group,
                "cluster",
                3,
                "peer@host:4370",
                9,
            )
            .unwrap()
            .0
    }

    async fn append(raft: &super::super::MeshRaft, id: &str, term: u64) {
        let log_id = LogId::new(CommittedLeaderId::new(term, 3), 0);
        let result = raft
            .append_entries(AppendEntriesRequest {
                vote: Vote::new_committed(term, 3),
                prev_log_id: None,
                entries: vec![Entry {
                    log_id,
                    payload: EntryPayload::Normal(ConsensusCommand {
                        command_id: id.to_string(),
                        actor: "registry-test".to_string(),
                        reason: "check group isolation".to_string(),
                        timestamp_unix_millis: 1,
                        actor_sequence: 0,
                        mutation: ControlMutation::DesiredCapacity(DesiredCapacity {
                            revision: DesiredRevision(1),
                            worker_nodes: 1,
                            gateway_nodes: 0,
                            template_revision: "v1".to_string(),
                        }),
                    }),
                }],
                leader_commit: Some(log_id),
            })
            .await
            .unwrap();
        assert!(matches!(
            result,
            openraft::raft::AppendEntriesResponse::Success
        ));
        raft.wait(Some(Duration::from_secs(5)))
            .applied_index(Some(0), "apply routed command")
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn groups_route_to_distinct_rafts_and_validate_before_lookup() {
        let directory = tempfile::tempdir().unwrap();
        let registry = RaftRpcRegistry::default();
        let control = node(&directory.path().join("control.redb")).await;
        let mut a = node(&directory.path().join("a.redb")).await;
        let b = node(&directory.path().join("b.redb")).await;
        let control_server = MeshConsensusRpcServer {
            cluster_name: "cluster".to_string(),
            node_id: 9,
            node_name: "local@host:4370".to_string(),
            raft: control.raft.clone(),
            state_machine: control.state_machine.clone(),
            runtime: tokio::runtime::Handle::current(),
        };
        let control_token = registry.register_control(control_server.clone()).unwrap();
        assert_eq!(
            registry.register_control(control_server).err(),
            Some("consensus_rpc_group_already_registered".to_string())
        );
        for mismatched in [
            RaftRpcIdentity {
                node_id: 10,
                ..identity()
            },
            RaftRpcIdentity {
                cluster_name: "other".to_string(),
                ..identity()
            },
        ] {
            assert_eq!(
                registry
                    .register_jobs(
                        "invalid",
                        mismatched,
                        a.raft.clone(),
                        tokio::runtime::Handle::current()
                    )
                    .err(),
                Some("consensus_node_configuration_invalid".to_string())
            );
        }
        let group_a = RaftGroup::Jobs("a".to_string());
        let group_b = RaftGroup::Jobs("b".to_string());
        let a_token = registry
            .register_jobs(
                "a",
                identity(),
                a.raft.clone(),
                tokio::runtime::Handle::current(),
            )
            .unwrap();
        let b_token = registry
            .register_jobs(
                "b",
                identity(),
                b.raft.clone(),
                tokio::runtime::Handle::current(),
            )
            .unwrap();
        assert_eq!(
            registry
                .register_jobs(
                    "a",
                    identity(),
                    b.raft.clone(),
                    tokio::runtime::Handle::current()
                )
                .err(),
            Some("consensus_rpc_group_already_registered".to_string())
        );
        for deployment in ["".to_string(), " ".to_string(), "x".repeat(257)] {
            assert_eq!(
                registry
                    .register_jobs(
                        &deployment,
                        identity(),
                        a.raft.clone(),
                        tokio::runtime::Handle::current()
                    )
                    .err(),
                Some("consensus_rpc_deployment_invalid".to_string())
            );
        }
        assert_eq!(
            AuthorizedRaftPeer::control(false, "peer@host:4370", "local@host:4370").err(),
            Some("consensus_rpc_capability_unavailable".to_string())
        );
        for unauthorized in [RaftGroup::Control, group_b.clone()] {
            assert_eq!(
                registry
                    .resolve::<MeshRaftConfig>(
                        &peer(unauthorized),
                        &group_a,
                        "cluster",
                        3,
                        "peer@host:4370",
                        9
                    )
                    .err(),
                Some("consensus_rpc_group_mismatch".to_string())
            );
        }
        for (cluster, source, source_name, target) in [
            ("other", 3, "peer@host:4370", 9),
            ("cluster", 0, "peer@host:4370", 9),
            ("cluster", 3, "impostor@host:4370", 9),
            ("cluster", 3, "peer@host:4370", 8),
        ] {
            assert_eq!(
                registry
                    .resolve::<MeshRaftConfig>(
                        &peer(group_a.clone()),
                        &group_a,
                        cluster,
                        source,
                        source_name,
                        target
                    )
                    .err(),
                Some("consensus_rpc_identity_mismatch".to_string())
            );
        }
        let mut renamed = peer(group_a.clone());
        renamed.local_name = "renamed@host:4370";
        assert_eq!(
            registry
                .resolve::<MeshRaftConfig>(&renamed, &group_a, "cluster", 3, "peer@host:4370", 9)
                .err(),
            Some("consensus_rpc_identity_mismatch".to_string())
        );
        assert_eq!(
            registry
                .resolve::<OtherConfig>(
                    &peer(group_a.clone()),
                    &group_a,
                    "cluster",
                    3,
                    "peer@host:4370",
                    9
                )
                .err(),
            Some("consensus_rpc_config_mismatch".to_string())
        );
        assert!(a.log_store.read_vote().await.unwrap().is_none());
        assert!(a
            .log_store
            .get_log_state()
            .await
            .unwrap()
            .last_log_id
            .is_none());

        append(
            &resolve(&registry, &RaftGroup::Control),
            "control-command",
            7,
        )
        .await;
        assert!(a.state_machine.state().unwrap().entries.is_empty());
        assert!(b.state_machine.state().unwrap().entries.is_empty());
        append(&resolve(&registry, &group_a), "a-command", 8).await;
        append(&resolve(&registry, &group_b), "b-command", 9).await;
        for (node, expected) in [
            (&control, "control-command"),
            (&a, "a-command"),
            (&b, "b-command"),
        ] {
            let state = node.state_machine.state().unwrap();
            assert_eq!(
                state
                    .command_results
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                [expected]
            );
            assert_eq!(state.entries.len(), 1);
        }
        for token in [control_token, a_token, b_token] {
            assert!(registry.unregister(&token).unwrap());
        }
        for node in [control, a, b] {
            node.raft.shutdown().await.unwrap();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_unregister_and_inflight_dispatch_cannot_reach_a_successor() {
        let directory = tempfile::tempdir().unwrap();
        let registry = RaftRpcRegistry::default();
        let old = node(&directory.path().join("old.redb")).await;
        let mut new = node(&directory.path().join("new.redb")).await;
        let group = RaftGroup::Jobs("deployment".to_string());
        let old_token = registry
            .register_jobs(
                "deployment",
                identity(),
                old.raft.clone(),
                tokio::runtime::Handle::current(),
            )
            .unwrap();
        let old_route = resolve(&registry, &group);
        let (release, held) = tokio::sync::oneshot::channel();
        let in_flight = tokio::spawn(async move {
            held.await.unwrap();
            append(&old_route, "old-command", 7).await;
        });
        assert!(registry.unregister(&old_token).unwrap());
        assert_eq!(
            registry
                .resolve::<MeshRaftConfig>(
                    &peer(group.clone()),
                    &group,
                    "cluster",
                    3,
                    "peer@host:4370",
                    9
                )
                .err(),
            Some("consensus_rpc_server_unavailable".to_string())
        );
        let new_token = registry
            .register_jobs(
                "deployment",
                identity(),
                new.raft.clone(),
                tokio::runtime::Handle::current(),
            )
            .unwrap();
        assert!(!registry.unregister(&old_token).unwrap());
        let other_registry = RaftRpcRegistry::default();
        let foreign = other_registry
            .register_jobs(
                "deployment",
                identity(),
                old.raft.clone(),
                tokio::runtime::Handle::current(),
            )
            .unwrap();
        assert!(!registry.unregister(&foreign).unwrap());
        release.send(()).unwrap();
        in_flight.await.unwrap();
        assert!(old
            .state_machine
            .state()
            .unwrap()
            .command_results
            .contains_key("old-command"));
        assert!(new.state_machine.state().unwrap().entries.is_empty());
        assert!(new.log_store.read_vote().await.unwrap().is_none());
        append(&resolve(&registry, &group), "new-command", 8).await;
        let new_state = new.state_machine.state().unwrap();
        assert_eq!(
            new_state
                .command_results
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["new-command"]
        );
        assert!(registry.unregister(&new_token).unwrap());
        assert!(!registry.unregister(&old_token).unwrap());
        assert!(other_registry.unregister(&foreign).unwrap());
        old.raft.shutdown().await.unwrap();
        new.raft.shutdown().await.unwrap();
    }
}
