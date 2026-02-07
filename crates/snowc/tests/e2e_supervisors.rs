//! End-to-end integration tests for the Snow supervisor compiler pipeline.
//!
//! Each test compiles a .snow program that exercises supervisor features,
//! builds it into a native binary, and verifies expected behavior.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Helper: compile a Snow source file and assert it compiles successfully.
/// Returns the path to the compiled binary.
fn compile_snow(source: &str) -> (tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, source).expect("failed to write main.snow");

    // Build with snowc
    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke snowc");

    assert!(
        output.status.success(),
        "snowc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let binary = project_dir.join("project");
    assert!(
        binary.exists(),
        "compiled binary not found at {}",
        binary.display()
    );

    (temp_dir, binary)
}

/// Read a test fixture from the tests/e2e/ directory.
fn read_fixture(name: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("e2e")
        .join(name);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", fixture_path.display(), e))
}

/// Find the snowc binary in the target directory.
fn find_snowc() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot find current exe")
        .parent()
        .expect("cannot find parent dir")
        .to_path_buf();

    if path.file_name().map_or(false, |n| n == "deps") {
        path = path.parent().unwrap().to_path_buf();
    }

    let snowc = path.join("snowc");
    assert!(
        snowc.exists(),
        "snowc binary not found at {}. Run `cargo build -p snowc` first.",
        snowc.display()
    );
    snowc
}

// ── Supervisor E2E Tests ─────────────────────────────────────────────────

/// Test: Supervisor block compiles to a native binary successfully.
///
/// This validates the full compiler pipeline: parsing supervisor blocks,
/// type checking, MIR lowering, and LLVM codegen with supervisor intrinsics.
#[test]
fn supervisor_basic() {
    let source = read_fixture("supervisor_basic.snow");
    let (_temp_dir, binary) = compile_snow(&source);

    // Verify the binary exists and is executable.
    assert!(binary.exists(), "compiled supervisor binary should exist");

    // Run the binary with a short timeout -- it should print and exit.
    let mut child = Command::new(&binary)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn binary: {}", e));

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(10);
    let poll_interval = std::time::Duration::from_millis(50);

    let output = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    out.read_to_end(&mut stdout).ok();
                }
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    err.read_to_end(&mut stderr).ok();
                }
                break std::process::Output {
                    status,
                    stdout,
                    stderr,
                };
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("supervisor binary timed out after {} seconds", timeout.as_secs());
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => panic!("error waiting for process: {}", e),
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("supervisor started"),
        "Expected 'supervisor started' in output, got: {}",
        stdout
    );
}
