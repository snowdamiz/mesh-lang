//! Standard repository artifacts by default; explicit immutable
//! compiler/runtime override only for isolated qualification. Keep opt0/2 output.
use std::{fs, path::PathBuf, process::Command};
#[path = "support/test_artifacts.rs"]
mod artifacts;
fn compiler() -> PathBuf {
    match std::env::var_os("MESH_SECRET_PRIMITIVES_MESHC") {
        Some(value) => {
            let compiler = PathBuf::from(value);
            assert!(compiler.is_file(), "explicit compiler must exist");
            let runtime = PathBuf::from(
                std::env::var_os("MESH_RT_LIB_PATH")
                    .expect("an explicit isolated compiler requires explicit runtime"),
            );
            assert!(runtime.is_file(), "explicit runtime must exist");
            compiler
        }
        None => artifacts::meshc_bin(),
    }
}
#[test]
fn secret_primitives_compile_and_execute_opt0_and_opt2() {
    let compiler = compiler();
    for opt in ["0", "2"] {
        let evidence =
            artifacts::artifact_dir("secret-primitives-public-api", &format!("opt{opt}"));
        let project = evidence.join("secret-primitives");
        fs::create_dir(&project).unwrap();
        fs::write(
            project.join("mesh.toml"),
            "[package]\nname = \"secret-primitives\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            project.join("main.mpl"),
            include_str!("../../../tests/e2e/secret_primitives.mpl"),
        )
        .unwrap();
        fs::write(
            evidence.join("compiler-path.txt"),
            compiler.to_string_lossy().as_bytes(),
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
            "opt{opt}: {}\n{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let executable = project.join(if cfg!(windows) {
            "secret-primitives.exe"
        } else {
            "secret-primitives"
        });
        // Successful `meshc build` emits exactly this progress line. Reject
        // warnings, diagnostics and every other unexpected stderr byte.
        assert_eq!(
            build.stderr,
            format!("  Compiled: {}\n", executable.display()).into_bytes(),
            "unexpected build stderr at opt{opt}"
        );
        let run = Command::new(&executable).output().unwrap();
        fs::write(evidence.join("run.stdout"), &run.stdout).unwrap();
        fs::write(evidence.join("run.stderr"), &run.stderr).unwrap();
        assert!(
            run.status.success(),
            "opt{opt}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(
            run.stderr.is_empty(),
            "unexpected run stderr at opt{opt}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "C1_SECRET_PRIMITIVES_EXECUTED\n"
        );
    }
}
