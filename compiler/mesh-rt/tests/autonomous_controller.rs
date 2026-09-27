//! The embedded autonomous config and its controller are process-wide: a
//! registered config puts every part of the runtime in autonomous mode, so
//! the library's own tests never register one. This binary does, once.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mesh_rt::dist::autonomous::start_autonomous_controller;
use mesh_rt::dist::consensus::start_mesh_consensus_from_env;
use mesh_rt::{
    autonomous_controller_status, consensus_runtime_snapshot, embedded_autonomous_config,
    mesh_register_autonomous_config_json, RuntimeAutonomousConfig, RuntimeCapacityDriverConfig,
    RuntimeContinuityConfig, RuntimeFeatureGates, RuntimeRoutingConfig, RuntimeSchedulerConfig,
    ScalingPolicy, AUTONOMOUS_CONFIG_SCHEMA_VERSION,
};

fn register(config: &RuntimeAutonomousConfig) -> i32 {
    let json = serde_json::to_vec(config).expect("encode config");
    mesh_register_autonomous_config_json(json.as_ptr(), json.len() as u64)
}

#[test]
fn a_registered_config_starts_one_controller_that_stops_without_a_node() {
    let directory = tempfile::tempdir().expect("tempdir");
    let working_directory: PathBuf = directory.path().join("workers");
    let config = RuntimeAutonomousConfig {
        schema_version: AUTONOMOUS_CONFIG_SCHEMA_VERSION,
        enabled: true,
        features: RuntimeFeatureGates::default(),
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
        scheduler: RuntimeSchedulerConfig::default(),
        routing: RuntimeRoutingConfig::default(),
        continuity: RuntimeContinuityConfig::default(),
        driver: RuntimeCapacityDriverConfig::Process {
            command: vec!["sh".to_string(), "-c".to_string(), "true".to_string()],
            working_directory: working_directory.clone(),
        },
    };
    assert_eq!(register(&config), 0);
    assert_eq!(embedded_autonomous_config(), Some(&config));
    // One config per process.
    assert_eq!(register(&config), -1);

    // Only a controller runs the controller.
    std::env::remove_var("MESH_ROLES");
    assert_eq!(start_autonomous_controller(), Ok(false));
    std::env::set_var("MESH_ROLES", "controller");
    std::env::remove_var("MESH_CLUSTER_ID");
    assert_eq!(
        start_autonomous_controller(),
        Err("autonomous_cluster_id_missing".to_string())
    );
    std::env::set_var("MESH_CLUSTER_ID", "controller-test");
    // The driver refuses a working directory that does not exist, and the
    // refusal is start's error, not a thread's.
    assert_eq!(
        start_autonomous_controller(),
        Err("process_driver_working_directory_invalid".to_string())
    );
    assert!(!autonomous_controller_status().configured);

    std::fs::create_dir(&working_directory).expect("working directory");
    assert_eq!(start_autonomous_controller(), Ok(true));
    let status = autonomous_controller_status();
    assert!(status.configured);
    assert_eq!(status.policy_revision, 1);
    // With no node to serve, the controller stops at once.
    let deadline = Instant::now() + Duration::from_secs(10);
    while autonomous_controller_status().state != "stopped" {
        assert!(
            Instant::now() < deadline,
            "{:?}",
            autonomous_controller_status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!autonomous_controller_status().running);
    assert_eq!(
        start_autonomous_controller(),
        Err("autonomous_controller_already_started".to_string())
    );

    // Consensus needs this node started: its runtime thread reports that it
    // is not and leaves no consensus behind.
    std::env::set_var("MESH_STABLE_NODE_ID", "controller-test/controller/a");
    std::env::set_var(
        "MESH_CONTROLLER_VOTERS",
        "controller-test/controller/a|a@a:4370",
    );
    std::env::set_var("MESH_CONSENSUS_DB", directory.path().join("consensus.redb"));
    assert_eq!(start_mesh_consensus_from_env("a@a:4370"), Ok(true));
    std::thread::sleep(Duration::from_millis(200));
    assert!(consensus_runtime_snapshot().is_none());
}
