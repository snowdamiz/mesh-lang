//! Real generated ABI regression: local HTTP barriers, SQLite, and retained Host
//! storage callbacks. Native OS policy and asynchronous scheduling are not tested.
#![cfg(unix)]
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
#[path = "support/test_artifacts.rs"]
mod artifacts;

#[test]
fn generated_native_authority_gates_actual_effects() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library_authority");
    let directory = artifacts::artifact_dir("native-effect-authority", "generated-abi");
    let library = directory.join(if cfg!(target_os = "macos") {
        "libauthority.dylib"
    } else {
        "libauthority.so"
    });
    let output = Command::new(artifacts::meshc_bin())
        .args(["build"])
        .arg(&fixture)
        .args([
            "--artifact",
            "cdylib",
            "--opt-level",
            "0",
            "--no-color",
            "-o",
        ])
        .arg(&library)
        .output()
        .expect("compile fixture");
    artifacts::write_artifact(
        &directory.join("build.log"),
        artifacts::command_output_text(&output),
    );
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    let probe = directory.join("stack-probe.so");
    let output = Command::new("cc")
        .arg(fixture.join("stack_probe.c"))
        .arg("-I")
        .arg(&directory)
        .args(["-shared", "-fPIC", "-pthread", "-o"])
        .arg(&probe)
        .output()
        .expect("compile C ABI and stack probe");
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    for case in [
        "control",
        "write",
        "query-write",
        "schema",
        "commit",
        "raw-commit",
        "release",
        "accepted-commit",
        "http",
        "store",
        "accepted-store",
        "nonce",
        "accepted-nonce",
        "actor",
        "timer",
        "missing",
        "error",
        "unsupported",
        "generations",
        "registration",
        "hot-journal",
        "terminate",
        "v1-6",
        "v1-8",
        "v1-output",
        "v1-missing",
        "ancestry",
        "gc",
        "panic",
        "request-refusal",
        "stream-refusal",
    ] {
        let child = Command::new("python3")
            .arg(fixture.join("check.py"))
            .arg(&library)
            .arg(case)
            .arg(format!("--stack-probe={}", probe.display()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run native embedding");
        let output = artifacts::wait_with_timeout(child, Duration::from_secs(30))
            .expect("bounded native embedding");
        artifacts::write_artifact(
            &directory.join(format!("{case}.log")),
            artifacts::command_output_text(&output),
        );
        assert!(
            output.status.success(),
            "{case}: {}",
            artifacts::command_output_text(&output)
        );
    }
}
