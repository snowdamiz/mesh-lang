//! One node's control plane end to end: a single-voter embedded consensus,
//! operator controls and queries over the node's own listener, and then the
//! autonomous controller leading that consensus. The node, the consensus
//! server, and the embedded config are process-wide, so this binary holds
//! one test that runs the phases in order.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mesh_rt::dist::autonomous::start_autonomous_controller;
use mesh_rt::dist::consensus::start_mesh_durable_consensus_node;
use mesh_rt::{
    autonomous_controller_status, commit_consensus_command, configured_continuity_store,
    consensus_runtime_snapshot, mesh_node_start, mesh_register_autonomous_config_json,
    query_operator_continuity_list_remote, query_operator_continuity_status_remote,
    query_operator_control_remote, query_operator_diagnostics_remote,
    query_operator_runtime_remote, query_operator_status_remote, sign_operator_control_request,
    ConsensusCommand, ControlMutation, OperatorControlAction, OperatorControlOutcome,
    OperatorControlRequest, OperatorQueryError, RuntimeAutonomousConfig,
    RuntimeCapacityDriverConfig, RuntimeFeatureGates, ScalingPolicy,
    AUTONOMOUS_CONFIG_SCHEMA_VERSION,
};

const COOKIE: &str = "control-plane-test-cookie";
const OPERATOR_KEY: &str = "control-plane-operator-key-0123456789";
const TIMEOUT: Duration = Duration::from_secs(10);

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:?}",
            autonomous_controller_status()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Operator {
    target: String,
    sequence: u64,
}

impl Operator {
    fn request(&mut self, actor: &str, action: OperatorControlAction) -> OperatorControlRequest {
        self.sequence += 1;
        sign_operator_control_request(
            OperatorControlRequest {
                schema_version: 1,
                cluster_id: "mesh".to_string(),
                actor: actor.to_string(),
                sequence: self.sequence,
                expires_at_unix_millis: unix_millis() + 60_000,
                reason: "control plane test".to_string(),
                action,
                signature: String::new(),
            },
            OPERATOR_KEY,
        )
        .expect("signed request")
    }

    fn send(
        &self,
        request: OperatorControlRequest,
    ) -> Result<OperatorControlOutcome, OperatorQueryError> {
        query_operator_control_remote(&self.target, COOKIE, request, TIMEOUT)
    }

    fn control(&mut self, action: OperatorControlAction) -> OperatorControlOutcome {
        let request = self.request("control-plane-operator", action);
        self.send(request).expect("control accepted")
    }

    fn refused(&mut self, request: OperatorControlRequest) -> String {
        match self.send(request) {
            Err(OperatorQueryError::RemoteRejected { reason, .. }) => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn a_node_serves_consensus_operator_controls_and_its_controller() {
    let directory = tempfile::tempdir().expect("tempdir");
    // The node keeps its continuity in a store of this test's: settled
    // before any embedded config could ask for a default one.
    std::env::set_var("MESH_CONTINUITY_DB", directory.path().join("continuity.db"));
    assert!(configured_continuity_store().is_some());
    // Until an operator sets one, the desired capacity is the deployment's.
    std::env::set_var("MESH_DESIRED_CAPACITY", "4");
    mesh_rt::actor::mesh_rt_init_actor(2);
    std::env::set_var("MESH_OPERATOR_KEY", OPERATOR_KEY);
    let audit_log = directory.path().join("audit").join("operator.log");
    std::env::set_var("MESH_OPERATOR_AUDIT_LOG", &audit_log);

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let name = format!("control@127.0.0.1:{port}");
    assert_eq!(
        mesh_node_start(
            name.as_ptr(),
            name.len() as u64,
            COOKIE.as_ptr(),
            COOKIE.len() as u64
        ),
        0
    );

    // A single-voter consensus on this node leads at once.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let consensus = runtime.block_on(async {
        let node = start_mesh_durable_consensus_node(
            1,
            &name,
            "control-plane",
            &directory.path().join("consensus.redb"),
        )
        .await
        .expect("consensus node");
        node.raft
            .initialize(BTreeMap::from([(1, openraft::BasicNode::new(&name))]))
            .await
            .expect("initialize");
        node
    });
    wait_until("a consensus leader", || {
        consensus_runtime_snapshot().is_some_and(|snapshot| {
            snapshot.state == "leader" && snapshot.current_leader == Some(1)
        })
    });
    let committed = commit_consensus_command(
        ConsensusCommand {
            command_id: "direct-command".to_string(),
            actor: "control-plane-test".to_string(),
            reason: "direct commit".to_string(),
            timestamp_unix_millis: unix_millis(),
            actor_sequence: 0,
            mutation: ControlMutation::PauseAutoscaler { paused: false },
        },
        TIMEOUT,
    )
    .expect("commit");
    assert!(committed.applied);
    assert_eq!(
        commit_consensus_command(
            ConsensusCommand {
                command_id: "hurried-command".to_string(),
                actor: "control-plane-test".to_string(),
                reason: "no time to commit".to_string(),
                timestamp_unix_millis: unix_millis(),
                actor_sequence: 0,
                mutation: ControlMutation::PauseAutoscaler { paused: false },
            },
            Duration::from_nanos(1),
        ),
        Err("consensus_commit_timeout".to_string())
    );

    let before = mesh_rt::operator_runtime_snapshot().expect("local runtime snapshot");
    assert_eq!(before.desired_capacity, 4);
    assert_eq!(before.scheduler_min_workers, 2);
    assert!(before.local_continuity_store.is_some());

    // Operator controls commit through the consensus and apply here.
    let mut operator = Operator {
        target: name.clone(),
        sequence: 0,
    };
    let paused = operator.control(OperatorControlAction::PauseAutoscaler);
    assert!(paused.autoscaler_paused);
    assert!(paused.consensus.is_some_and(|response| response.applied));
    assert!(
        !operator
            .control(OperatorControlAction::ResumeAutoscaler)
            .autoscaler_paused
    );
    assert_eq!(
        operator
            .control(OperatorControlAction::SetDesiredCapacity { worker_nodes: 3 })
            .desired_capacity_override,
        Some(3)
    );
    let remote = "remote-worker@127.0.0.1:1".to_string();
    let drained = operator.control(OperatorControlAction::DrainNode {
        node_id: remote.clone(),
    });
    assert!(drained.drain_intents.contains(&remote));
    let cancelled = operator.control(OperatorControlAction::CancelDrain {
        node_id: remote.clone(),
    });
    assert!(!cancelled.drain_intents.contains(&remote));
    assert!(operator
        .control(OperatorControlAction::DrainNode {
            node_id: name.clone()
        })
        .drain_intents
        .contains(&name));
    let draining = mesh_rt::operator_runtime_snapshot().expect("draining snapshot");
    let local = draining
        .nodes
        .iter()
        .find(|node| node.node_id == name)
        .expect("local node reported");
    assert_eq!(local.state, "draining");
    assert!(!local.routing_eligible);
    assert!(operator
        .control(OperatorControlAction::CancelDrain {
            node_id: name.clone()
        })
        .drain_intents
        .is_empty());
    operator.control(OperatorControlAction::CommitControlMutation {
        command_id: "operator-policy".to_string(),
        mutation: ControlMutation::PolicyRevision {
            revision: 9,
            policy_json: "{}".to_string(),
            policy_sha256: "0".repeat(64),
        },
    });

    // Refusals: a replay, an invalid target, the propagator's identity, a
    // bad signature, and an expired request.
    let mut replay = operator.request(
        "control-plane-operator",
        OperatorControlAction::PauseAutoscaler,
    );
    replay.sequence = 1;
    let replay = sign_operator_control_request(replay, OPERATOR_KEY).unwrap();
    assert_eq!(operator.refused(replay), "operator_control_replay_rejected");
    let zero = operator.request(
        "control-plane-operator",
        OperatorControlAction::SetDesiredCapacity { worker_nodes: 0 },
    );
    assert_eq!(
        operator.refused(zero),
        "operator_control_desired_capacity_invalid"
    );
    let internal = operator.request(
        "mesh-drain-propagator",
        OperatorControlAction::DrainNode {
            node_id: name.clone(),
        },
    );
    assert_eq!(
        operator.refused(internal),
        "operator_internal_control_requires_controller_identity"
    );
    let mut forged = operator.request(
        "control-plane-operator",
        OperatorControlAction::PauseAutoscaler,
    );
    forged.signature = "0".repeat(64);
    assert_eq!(operator.refused(forged), "operator_control_unauthorized");
    let mut expired = operator.request(
        "control-plane-operator",
        OperatorControlAction::PauseAutoscaler,
    );
    expired.expires_at_unix_millis = 1;
    let expired = sign_operator_control_request(expired, OPERATOR_KEY).unwrap();
    assert_eq!(
        operator.refused(expired),
        "operator_control_expired_or_too_far_future"
    );
    let audit = std::fs::read_to_string(&audit_log).expect("audit log");
    assert!(audit.contains("\"outcome\":\"committed\""), "{audit}");
    assert!(audit.contains("\"outcome\":\"rejected\""), "{audit}");

    // The node answers every read-only query over the same listener.
    let runtime_snapshot =
        query_operator_runtime_remote(&name, COOKIE, TIMEOUT).expect("runtime snapshot");
    assert_eq!(runtime_snapshot.local_node, name);
    assert_eq!(runtime_snapshot.desired_capacity, 3);
    assert!(runtime_snapshot
        .consensus
        .is_some_and(|consensus| consensus.state == "leader"));
    let status = query_operator_status_remote(&name, COOKIE, TIMEOUT).expect("status");
    assert_eq!(status.membership.local_node, name);
    let list = query_operator_continuity_list_remote(&name, COOKIE, Some(10), TIMEOUT)
        .expect("continuity list");
    assert_eq!(list.total_records, 0);
    assert!(matches!(
        query_operator_continuity_status_remote(&name, COOKIE, "unknown-request", TIMEOUT),
        Err(OperatorQueryError::RemoteRejected { ref reason, .. }) if reason == "request_key_not_found"
    ));
    assert!(matches!(
        query_operator_continuity_status_remote(&name, COOKIE, "", TIMEOUT),
        Err(OperatorQueryError::InvalidRequest { .. })
    ));
    let diagnostics =
        query_operator_diagnostics_remote(&name, COOKIE, Some(500), TIMEOUT).expect("diagnostics");
    assert!(diagnostics
        .entries
        .iter()
        .any(|entry| entry.transition == "drain_target_propagation_failed"));
    assert!(matches!(
        query_operator_diagnostics_remote(&name, COOKIE, Some(usize::MAX), TIMEOUT),
        Err(OperatorQueryError::InvalidRequest { .. })
    ));
    let unreachable = query_operator_status_remote(&name, "wrong-cookie", TIMEOUT)
        .expect_err("a wrong cookie is refused");
    assert!(
        unreachable.to_string().contains("unavailable"),
        "{unreachable}"
    );

    // The autonomous controller leads this consensus: it registers its
    // policy, records membership, sets the minimum capacity, and creates
    // workers through its driver.
    let workers = directory.path().join("workers");
    std::fs::create_dir(&workers).unwrap();
    let config = RuntimeAutonomousConfig {
        schema_version: AUTONOMOUS_CONFIG_SCHEMA_VERSION,
        enabled: true,
        features: RuntimeFeatureGates::default(),
        policy_revision: 1,
        policy: ScalingPolicy::default(),
        managed_roles: vec!["worker".to_string()],
        gateway_nodes: 0,
        template_revision: "v1".to_string(),
        reconcile_interval_millis: 50,
        startup_timeout_millis: 5_000,
        drain_timeout_millis: 60_000,
        termination_timeout_millis: 5_000,
        force_termination_after_drain_timeout: false,
        scheduler: Default::default(),
        routing: Default::default(),
        continuity: Default::default(),
        driver: RuntimeCapacityDriverConfig::Process {
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "exec sleep 5".to_string(),
            ],
            working_directory: workers.clone(),
        },
    };
    let json = serde_json::to_vec(&config).unwrap();
    assert_eq!(
        mesh_register_autonomous_config_json(json.as_ptr(), json.len() as u64),
        0
    );
    std::env::set_var("MESH_ROLES", "controller");
    std::env::set_var("MESH_CLUSTER_ID", "control-plane");
    assert_eq!(start_autonomous_controller(), Ok(true));
    wait_until("the controller's first ticks", || {
        let status = autonomous_controller_status();
        status.leader
            && status.desired_workers == 3
            && status
                .last_reconcile
                .is_some_and(|reconcile| reconcile.observed_workers == 3)
    });
    let status = autonomous_controller_status();
    assert_eq!(status.state, "leader");
    assert_eq!(status.membership_generation, 1);
    let entries = consensus_runtime_snapshot().unwrap().entries;
    assert!(entries
        .iter()
        .any(|entry| entry.reason == "register embedded scaling policy"));
    assert!(entries
        .iter()
        .any(|entry| entry.reason == "record observed runtime membership"));

    // A driver that can no longer create workers fails the tick that asks
    // for one more, and the controller reports why.
    std::fs::remove_dir(&workers).unwrap();
    commit_consensus_command(
        ConsensusCommand {
            command_id: "more-workers".to_string(),
            actor: "control-plane-test".to_string(),
            reason: "one more worker".to_string(),
            timestamp_unix_millis: unix_millis(),
            actor_sequence: 0,
            mutation: ControlMutation::ManualOverride { worker_nodes: 4 },
        },
        TIMEOUT,
    )
    .expect("override committed");
    wait_until("a failed tick", || {
        autonomous_controller_status()
            .last_error
            .is_some_and(|error| error.contains("process_driver_working_directory_invalid"))
    });

    // Without a leading consensus the controller stands by.
    runtime.block_on(async { consensus.raft.shutdown().await.expect("shutdown") });
    wait_until("the controller to stand by", || {
        let status = autonomous_controller_status();
        status.state == "standby" && !status.leader
    });

    // A stopping node stops its controller.
    mesh_rt::dist::node::node_state()
        .unwrap()
        .listener_shutdown
        .store(true, Ordering::Release);
    wait_until("the controller to stop", || {
        autonomous_controller_status().state == "stopped"
    });
}
