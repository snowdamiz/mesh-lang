//! A controller starts its embedded consensus from the deployment
//! environment: the bootstrap voter initializes the cluster, leads it, and
//! shuts it down with the node. The consensus runtime starts once per
//! process, so this binary holds one test.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mesh_rt::dist::consensus::start_mesh_consensus_from_env;
use mesh_rt::{consensus_runtime_snapshot, mesh_node_start};

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
        {
            break;
        }
        assert!(Instant::now() < deadline, "{snapshot:?}");
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
