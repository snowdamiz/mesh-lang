#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot locate test executable")
        .parent()
        .expect("test executable has no parent")
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}

fn build() -> (tempfile::TempDir, PathBuf, Output) {
    artifacts::ensure_mesh_rt_staticlib();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("e2e")
        .join("postgres_connection_ownership.mpl");
    let temp = tempfile::tempdir().expect("failed to create temp directory");
    let project = temp.path().join("postgres-connection-ownership");
    std::fs::create_dir_all(&project).expect("failed to create project directory");
    std::fs::copy(fixture, project.join("main.mpl"))
        .expect("failed to copy PgConn ownership fixture");

    let output = Command::new(meshc_bin())
        .args(["build", project.to_str().expect("non-UTF-8 project path")])
        .output()
        .expect("failed to invoke meshc");
    (temp, project, output)
}

#[test]
fn direct_postgres_connection_ownership_compiles() {
    let (_temp, _project, output) = build();

    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL or the documented local mesh_test PostgreSQL"]
fn direct_postgres_connection_round_trip() {
    let (_temp, project, output) = build();
    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let run = Command::new(project.join("postgres-connection-ownership"))
        .output()
        .expect("failed to execute PgConn ownership fixture");
    assert!(
        run.status.success(),
        "fixture failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "direct:ok\ndone\n");
}
