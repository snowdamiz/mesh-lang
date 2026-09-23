//! SC5: a 100-line program compiles in under 5 seconds at -O0.
//!
//! A test binary of its own: cargo runs test binaries one at a time, so the
//! build is timed alone rather than against the parallel builds of the other
//! end-to-end tests.

use std::path::{Path, PathBuf};
use std::process::Command;

fn find_meshc() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot find current exe")
        .parent()
        .expect("cannot find parent dir")
        .to_path_buf();
    if path.file_name().is_some_and(|n| n == "deps") {
        path = path.parent().unwrap().to_path_buf();
    }
    let meshc = path.join("meshc");
    assert!(
        meshc.exists(),
        "meshc binary not found at {}. Run `cargo build -p meshc` first.",
        meshc.display()
    );
    meshc
}

#[test]
fn e2e_performance() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e/comprehensive.mpl");
    let source = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", fixture.display(), e));
    std::fs::write(project_dir.join("main.mpl"), &source).expect("failed to write main.mpl");

    // The first run of a freshly built binary can pay for an OS scan of it
    // (macOS checks every new executable); that is not compile time.
    let meshc = find_meshc();
    Command::new(&meshc)
        .arg("--version")
        .output()
        .expect("failed to invoke meshc");

    let start = std::time::Instant::now();
    let output = Command::new(&meshc)
        .args(["build", project_dir.to_str().unwrap(), "--opt-level", "0"])
        .output()
        .expect("failed to invoke meshc");
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "Compilation failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed.as_secs() < 5,
        "Compilation took {:?} which exceeds 5 second limit",
        elapsed
    );
}
