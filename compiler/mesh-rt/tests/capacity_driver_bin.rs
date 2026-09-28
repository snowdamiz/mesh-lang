//! The capacity driver binary as an operator starts it.

/// Without its configuration the driver says why and exits 1.
#[test]
fn the_capacity_driver_refuses_to_start_unconfigured() {
    let mut driver = std::process::Command::new(env!("CARGO_BIN_EXE_mesh-capacity-driver"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("MESH_DOCKER_DRIVER_") {
            driver.env_remove(name);
        }
    }
    let output = driver.output().expect("the driver runs");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mesh capacity driver failed: driver_service_allowed_cluster_missing"),
        "{stderr}"
    );
}
