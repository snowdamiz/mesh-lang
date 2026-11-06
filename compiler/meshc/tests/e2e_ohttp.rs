//! Public OHTTP API opt0/opt2 execution and independent-wire conformance.
use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "support/test_artifacts.rs"]
mod artifacts;

#[test]
fn ohttp_public_api_executes_both_names_reference_request_and_wire_bounds_at_opt0_and_opt2() {
    for optimization in ["0", "2"] {
        let evidence = artifacts::artifact_dir("ohttp-public-api", &format!("opt{optimization}"));
        let project = evidence.join("ohttp-public-api");
        fs::create_dir(&project).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e/ohttp.mpl"),
            project.join("main.mpl"),
        )
        .unwrap();
        let build = Command::new(artifacts::meshc_bin())
            .args(["build", "--opt-level", optimization])
            .arg(&project)
            .output()
            .unwrap();
        fs::write(evidence.join("build.stdout"), &build.stdout).unwrap();
        fs::write(evidence.join("build.stderr"), &build.stderr).unwrap();
        assert!(
            build.status.success(),
            "opt{optimization}: {}\n{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let binary = if cfg!(windows) {
            "ohttp-public-api.exe"
        } else {
            "ohttp-public-api"
        };
        let run = Command::new(project.join(binary)).output().unwrap();
        fs::write(evidence.join("run.stdout"), &run.stdout).unwrap();
        fs::write(evidence.join("run.stderr"), &run.stderr).unwrap();
        assert!(
            run.status.success(),
            "opt{optimization}: {}\n{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(
            run.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8(run.stdout).unwrap(), "ohttp:qualified:ok\nohttp:flat:ok\nohttp:bounds:ok\nohttp:reference-request:ok\nohttp:wire-caps:ok\n");
    }
}
