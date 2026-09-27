//! A controller starts its embedded consensus from the deployment
//! environment: the bootstrap voter initializes the cluster, leads it, and
//! shuts it down with the node, and the autonomous controller waits for that
//! consensus before it ticks. The consensus runtime starts once per process,
//! so this binary holds one test.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mesh_rt::dist::autonomous::start_autonomous_controller;
use mesh_rt::dist::consensus::start_mesh_consensus_from_env;
use mesh_rt::{
    autonomous_controller_status, consensus_runtime_snapshot, mesh_node_start,
    mesh_register_autonomous_config_json, RuntimeAutonomousConfig, RuntimeCapacityDriverConfig,
    ScalingPolicy, AUTONOMOUS_CONFIG_SCHEMA_VERSION,
};

#[test]
fn the_bootstrap_controller_initializes_and_leads_its_consensus() {
    let directory = tempfile::tempdir().expect("tempdir");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let name = format!("controller@127.0.0.1:{port}");
    let cookie = "consensus-from-env-cookie";
    // The node starts before autonomous mode is set: it would otherwise
    // need a signed identity and mutual TLS, which this test has no use for.
    assert_eq!(
        mesh_node_start(
            name.as_ptr(),
            name.len() as u64,
            cookie.as_ptr(),
            cookie.len() as u64
        ),
        0
    );
    std::env::set_var("MESH_CLUSTER_MODE", "autonomous");
    std::env::set_var("MESH_ROLES", "controller");
    std::env::set_var("MESH_CLUSTER_ID", "env-cluster");
    std::env::set_var("MESH_STABLE_NODE_ID", "env-cluster/controller/a");
    std::env::set_var(
        "MESH_CONTROLLER_VOTERS",
        format!("env-cluster/controller/a|{name}"),
    );
    std::env::set_var(
        "MESH_CONSENSUS_DB",
        directory.path().join("control-plane.redb"),
    );

    // A controller started before its consensus waits for one: no ticks.
    // Its config enables durable continuity, kept in this test's directory.
    std::env::set_var("MESH_CONTINUITY_DB", directory.path().join("continuity.db"));
    let config = RuntimeAutonomousConfig {
        schema_version: AUTONOMOUS_CONFIG_SCHEMA_VERSION,
        enabled: true,
        features: Default::default(),
        policy_revision: 1,
        policy: ScalingPolicy::default(),
        managed_roles: vec!["worker".to_string()],
        gateway_nodes: 0,
        template_revision: "v1".to_string(),
        reconcile_interval_millis: 10,
        startup_timeout_millis: 1_000,
        drain_timeout_millis: 1_000,
        termination_timeout_millis: 1_000,
        force_termination_after_drain_timeout: false,
        scheduler: Default::default(),
        routing: Default::default(),
        continuity: Default::default(),
        driver: RuntimeCapacityDriverConfig::Process {
            command: vec!["sh".to_string(), "-c".to_string(), "true".to_string()],
            working_directory: directory.path().to_path_buf(),
        },
    };
    let json = serde_json::to_vec(&config).unwrap();
    assert_eq!(
        mesh_register_autonomous_config_json(json.as_ptr(), json.len() as u64),
        0
    );
    assert_eq!(start_autonomous_controller(), Ok(true));
    std::thread::sleep(Duration::from_millis(100));
    let waiting = autonomous_controller_status();
    assert_eq!(waiting.state, "starting");
    assert_eq!(waiting.tick_sequence, 0);

    // A voter list without this node's address is refused before anything
    // starts, and the runtime can still start once it is right.
    let voters = std::env::var("MESH_CONTROLLER_VOTERS").unwrap();
    std::env::set_var("MESH_CONTROLLER_VOTERS", "env-cluster/controller/a");
    assert_eq!(
        start_mesh_consensus_from_env(&name),
        Err("consensus_controller_voter_invalid".to_string())
    );
    std::env::set_var("MESH_CONTROLLER_VOTERS", voters);
    assert_eq!(start_mesh_consensus_from_env(&name), Ok(true));
    assert_eq!(
        start_mesh_consensus_from_env(&name),
        Err("consensus_runtime_already_started".to_string())
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = consensus_runtime_snapshot();
        if snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.state == "leader" && snapshot.voter_ids.len() == 1)
            && autonomous_controller_status().leader
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{snapshot:?} {:?}",
            autonomous_controller_status()
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    mesh_rt::dist::node::node_state()
        .unwrap()
        .listener_shutdown
        .store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(20);
    while consensus_runtime_snapshot().is_some_and(|snapshot| snapshot.state == "leader") {
        assert!(
            Instant::now() < deadline,
            "consensus still leads after shutdown"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
