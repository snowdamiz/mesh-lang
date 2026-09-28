//! Distribution subsystem for Mesh.
//!
//! The node identity/connection layer for inter-node message transport, and
//! the cluster services built on it.

pub mod autonomous;
pub mod bootstrap;
pub mod cluster_api;
pub mod consensus;
pub mod consensus_store;
pub mod continuity;
pub mod continuity_store;
pub mod discovery;
pub mod driver_service;
pub mod global;
pub mod identity;
pub mod identity_claim;
pub mod node;
pub mod operator;
pub mod protocol;
pub mod readiness;
pub mod routing;
pub mod scaling;
pub mod telemetry;

/// The longest prefix of `text` at most `max_bytes` long that ends at a
/// character.
pub(crate) fn char_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Whether the test at `path`, as the test harness names it, runs its body
/// here: in a process of its own, which it starts from this binary when not
/// already in one, for a test that changes what every test in a process
/// shares.
#[cfg(test)]
pub(crate) fn in_own_process(path: &str) -> bool {
    /// Names the one test a child process of this binary runs.
    const OWN_PROCESS_TEST_ENV: &str = "MESH_RT_OWN_PROCESS_TEST";
    if std::env::var(OWN_PROCESS_TEST_ENV).is_ok_and(|test| test == path) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([path, "--exact", "--nocapture", "--test-threads=1"])
        .env(OWN_PROCESS_TEST_ENV, path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

#[cfg(test)]
mod autonomous_model_tests;
#[cfg(test)]
mod consensus_testing;
