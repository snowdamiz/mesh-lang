//! Existing opaque public APIs exercise approved typed SPQR roles at opt0/opt2.
use std::{fs, process::Command};
#[path = "support/test_artifacts.rs"]
mod artifacts;

#[test]
fn spqr_typed_storage_public_api_opt0_and_opt2() {
    let compiler = artifacts::meshc_bin();
    for opt in ["0", "2"] {
        let evidence = artifacts::artifact_dir("spqr-typed-storage", &format!("opt{opt}"));
        let project = evidence.join("spqr-storage-proof");
        fs::create_dir(&project).unwrap();
        fs::write(
            project.join("mesh.toml"),
            "[package]\nname = \"spqr-storage-proof\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            project.join("main.mpl"),
            include_str!("../../../tests/e2e/spqr_storage.mpl"),
        )
        .unwrap();
        let build = Command::new(&compiler)
            .arg("build")
            .arg(&project)
            .args(["--opt-level", opt])
            .output()
            .unwrap();
        fs::write(evidence.join("build.stdout"), &build.stdout).unwrap();
        fs::write(evidence.join("build.stderr"), &build.stderr).unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let binary = project.join(if cfg!(windows) {
            "spqr-storage-proof.exe"
        } else {
            "spqr-storage-proof"
        });
        let run = Command::new(binary).output().unwrap();
        fs::write(evidence.join("run.stdout"), &run.stdout).unwrap();
        fs::write(evidence.join("run.stderr"), &run.stderr).unwrap();
        assert!(run.status.success());
        assert!(run.stderr.is_empty());
        assert_eq!(run.stdout, b"SPQR_STORAGE_PUBLIC_API_EXECUTED\n");
    }
}
