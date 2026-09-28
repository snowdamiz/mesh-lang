//! The local proof gates `meshc proof` runs without Docker: a short
//! continuity soak, the performance gate and the chaos gate's arguments.

#[path = "support/test_artifacts.rs"]
mod test_artifacts;

use std::process::{Command, Output};

use serde_json::Value;
use test_artifacts::{command_output_text, meshc_bin, repo_root};

fn proof(args: &[&str]) -> Output {
    Command::new(meshc_bin())
        .arg("proof")
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("meshc runs")
}

fn summary(evidence: &std::path::Path) -> Value {
    let text = std::fs::read_to_string(evidence.join("summary.json")).expect("summary.json");
    serde_json::from_str(&text).expect("summary.json is JSON")
}

#[test]
fn a_short_continuity_soak_passes_as_a_smoke_run() {
    let evidence = tempfile::tempdir().unwrap();
    // Long enough for a disk plateau judged over four samples (one a
    // second); a snapshot is resumed from the first cycle.
    let output = proof(&[
        "continuity-soak",
        "--duration-seconds",
        "5",
        "--cycle-millis",
        "1",
        "--allow-short",
        "--evidence-dir",
        evidence.path().to_str().unwrap(),
    ]);
    let text = command_output_text(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("continuity_soak: SMOKE PASS"), "{text}");
    let summary = summary(evidence.path());
    assert_eq!(summary["safety_pass"], true, "{summary}");
    assert_eq!(summary["release_24h_pass"], false, "{summary}");
    assert!(summary["writes"].as_u64().unwrap() > 0, "{summary}");
    assert!(
        summary["interrupted_snapshots_resumed"].as_u64().unwrap() > 0,
        "{summary}"
    );
}

/// A soak too short for four samples cannot judge a disk plateau, so it does
/// not hold one against the run; it still checks every cycle's safety.
#[test]
fn a_soak_too_short_to_judge_a_plateau_still_checks_safety() {
    let evidence = tempfile::tempdir().unwrap();
    let output = proof(&[
        "continuity-soak",
        "--duration-seconds",
        "1",
        "--cycle-millis",
        "5",
        "--allow-short",
        "--evidence-dir",
        evidence.path().to_str().unwrap(),
    ]);
    let summary = summary(evidence.path());
    assert!(summary["writes"].as_u64().unwrap() > 0, "{summary}");
    assert_eq!(summary["release_24h_pass"], false, "{summary}");
    assert!(
        command_output_text(&output).contains("continuity_soak:"),
        "{}",
        command_output_text(&output)
    );
}

#[test]
fn a_short_soak_is_not_a_release_run() {
    for (args, error) in [
        (
            vec!["continuity-soak", "--duration-seconds", "2"],
            "continuity_soak_release_requires_86400_seconds",
        ),
        (
            vec![
                "continuity-soak",
                "--duration-seconds",
                "0",
                "--allow-short",
            ],
            "continuity_soak_duration_or_cycle_zero",
        ),
        (
            vec!["autonomous-performance", "--iterations", "10"],
            "autonomous_performance_requires_at_least_100_iterations",
        ),
        (
            vec!["autonomous-chaos", "--rounds", "0"],
            "autonomous_chaos_rounds_must_be_1_to_100",
        ),
    ] {
        let output = proof(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            command_output_text(&output).contains(error),
            "{args:?}: {}",
            command_output_text(&output)
        );
    }
}

/// Its timings measure this machine, so a busy one may miss the budget; the
/// gate still has to run to the end, write its evidence and say which.
#[test]
fn the_performance_gate_measures_and_reports() {
    let evidence = tempfile::tempdir().unwrap();
    let output = proof(&[
        "autonomous-performance",
        "--iterations",
        "200",
        "--evidence-dir",
        evidence.path().to_str().unwrap(),
    ]);
    let text = command_output_text(&output);
    assert!(
        text.contains("autonomous_performance: PASS")
            || text.contains("autonomous_performance_gate_failed"),
        "{text}"
    );
    let measured = summary(evidence.path());
    assert_eq!(measured["schema_version"], 2, "{measured}");
    assert_eq!(measured["iterations"], 200, "{measured}");
    assert_eq!(measured["pass"], output.status.success(), "{measured}");

    // A budget no machine meets fails the gate, after writing its evidence.
    let mut budget: Value = serde_json::from_str(
        &std::fs::read_to_string(
            repo_root().join("proof/autonomous-gates/performance-budget.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for (name, limit) in budget.as_object_mut().unwrap() {
        if name.contains("_max_") {
            *limit = 0.into();
        } else if name.contains("_min_") {
            *limit = 1e12.into();
        }
    }
    let unmeetable = evidence.path().join("unmeetable.json");
    std::fs::write(&unmeetable, budget.to_string()).unwrap();
    let failed = tempfile::tempdir().unwrap();
    let output = proof(&[
        "autonomous-performance",
        "--iterations",
        "100",
        "--budget",
        unmeetable.to_str().unwrap(),
        "--evidence-dir",
        failed.path().to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        command_output_text(&output).contains("autonomous_performance_gate_failed"),
        "{}",
        command_output_text(&output)
    );
    assert_eq!(summary(failed.path())["pass"], false);

    // A budget it cannot read, or of another schema.
    budget["schema_version"] = 1.into();
    std::fs::write(&unmeetable, budget.to_string()).unwrap();
    let bad = evidence.path().join("bad.json");
    std::fs::write(&bad, r#"{"schema_version": 2}"#).unwrap();
    for (file, error) in [
        (&unmeetable, "performance_budget_schema_unsupported"),
        (&bad, "performance_budget_decode_failed"),
    ] {
        let output = proof(&["autonomous-performance", "--budget", file.to_str().unwrap()]);
        assert!(
            command_output_text(&output).contains(error),
            "{}",
            command_output_text(&output)
        );
    }
    let output = proof(&["autonomous-performance", "--budget", "no-such-budget.json"]);
    assert!(
        command_output_text(&output).contains("performance_budget_read_failed"),
        "{}",
        command_output_text(&output)
    );
}

/// One round of the chaos gate runs each fault suite of the runtime once and
/// records it. The suites build with cargo: the coverage run's compiler
/// wrapper must not reach them (it would instrument the runtime programs
/// link against).
#[test]
fn one_chaos_round_runs_every_suite() {
    let evidence = tempfile::tempdir().unwrap();
    let mut command = Command::new(meshc_bin());
    command
        .args([
            "proof",
            "autonomous-chaos",
            "--rounds",
            "1",
            "--evidence-dir",
        ])
        .arg(evidence.path())
        .current_dir(repo_root());
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if name.contains("RUSTFLAGS") || name.starts_with("RUSTC_") || name.contains("LLVM_COV") {
            command.env_remove(name.as_ref());
        }
    }
    let output = command.output().expect("meshc runs");
    let text = command_output_text(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("autonomous_chaos: PASS (1 rounds"), "{text}");
    let summary = summary(evidence.path());
    assert_eq!(summary["rounds_completed"], 1, "{summary}");
    assert_eq!(summary["passed"], true, "{summary}");
}

/// `meshc proof docker-autoscaling` with a fake `docker` first on PATH that
/// logs its calls, fails those whose arguments contain one of `failures`,
/// and otherwise answers as an empty Docker holding one container would.
/// Returns meshc's output, the evidence's summary (if written) and the calls.
fn docker_proof_with_failures(
    failures: &[&str],
    extra: &[&str],
) -> (Output, Option<Value>, String) {
    use std::os::unix::fs::PermissionsExt as _;

    let fake = tempfile::tempdir().unwrap();
    let calls = fake.path().join("calls.log");
    let failing = failures
        .iter()
        .map(|failure| format!("*\"{failure}\"*"))
        .collect::<Vec<_>>()
        .join("|");
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> '{calls}'
case "$*" in {failing}) echo "injected failure" >&2; exit 1;; esac
case "$*" in
  "version --format"*) echo '{{"Client":{{}}}}';;
  "compose version --short") echo 2.29.0;;
  "ps -aq"*) echo 0123456789abcdef;;
  "inspect 0123456789abcdef") echo '[]';;
  "logs 0123456789abcdef") echo 'a managed container log';;
esac
exit 0
"#,
        calls = calls.display(),
    );
    // openssl answers for real unless a failure names it: "openssl" fails,
    // "openssl-silent" succeeds writing nothing, "no-openssl" leaves it off
    // PATH (with everything else), "no-tmp" gives no temporary directory.
    let fake_openssl = if failures.contains(&"openssl") {
        Some("#!/bin/sh\necho 'injected failure' >&2\nexit 1\n")
    } else if failures.contains(&"openssl-silent") {
        Some("#!/bin/sh\nexit 0\n")
    } else {
        None
    };
    for (tool, body) in [("docker", Some(script.as_str())), ("openssl", fake_openssl)] {
        if let Some(body) = body {
            let path = fake.path().join(tool);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let evidence = tempfile::tempdir().unwrap();
    // "unwritable-summary" leaves no room for the summary.
    if failures.contains(&"unwritable-summary") {
        std::fs::create_dir(evidence.path().join("summary.json")).unwrap();
    }
    let path = if failures.contains(&"no-openssl") {
        fake.path().display().to_string()
    } else {
        format!(
            "{}:{}",
            fake.path().display(),
            std::env::var("PATH").unwrap()
        )
    };
    let mut command = Command::new(meshc_bin());
    command
        .args(["proof", "docker-autoscaling", "--evidence-dir"])
        .arg(evidence.path())
        .args(extra)
        .env("PATH", path)
        .current_dir(repo_root());
    if failures.contains(&"no-tmp") {
        command.env("TMPDIR", fake.path().join("missing"));
    }
    let output = command.output().expect("meshc runs");
    let summary = evidence
        .path()
        .join("summary.json")
        .is_file()
        .then(|| summary(evidence.path()));
    (
        output,
        summary,
        std::fs::read_to_string(calls).unwrap_or_default(),
    )
}

#[test]
fn the_docker_proof_stops_at_the_step_that_fails_and_still_cleans_up() {
    for (failure, extra, step) in [
        ("version --format", &["--no-build"][..], "docker version"),
        (
            "compose version",
            &["--no-build"][..],
            "docker compose version",
        ),
        (" config", &["--no-build"][..], "config"),
        ("image inspect", &["--no-build"][..], "docker image inspect"),
        ("--target runtime", &[][..], "--target runtime"),
        (
            "mesh-autoscaling-proof:local",
            &[][..],
            "docker tag mesh-autoscaling-proof",
        ),
        ("--target driver", &[][..], "--target driver"),
        (
            "mesh-autoscaling-driver:local",
            &[][..],
            "docker tag mesh-autoscaling-driver",
        ),
    ] {
        let (output, summary, calls) = docker_proof_with_failures(&[failure], extra);
        let text = command_output_text(&output);
        assert!(!output.status.success(), "{failure}: {text}");
        assert!(
            text.contains("docker_autoscaling_proof: FAIL"),
            "{failure}: {text}"
        );
        let summary = summary.expect("summary.json");
        assert_eq!(summary["passed"], false, "{failure}: {summary}");
        let error = summary["error"].as_str().unwrap_or_default();
        assert!(
            error.starts_with("proof_command_failed:") && error.contains(step),
            "{failure}: {error}"
        );
        assert!(error.contains("injected failure"), "{failure}: {error}");
        // Cleanup stopped the topology and removed its managed container.
        assert!(
            calls.contains("rm -f 0123456789abcdef"),
            "{failure}: {calls}"
        );
        assert!(calls.contains(" down --volumes"), "{failure}: {calls}");
        assert_eq!(
            summary["cleanup_error"],
            Value::Null,
            "{failure}: {summary}"
        );
    }
}

#[test]
fn a_failed_cleanup_is_reported_beside_the_failure() {
    for cleanup_step in [
        " stop --timeout",
        "--filter label=mesh.managed=true",
        " down --volumes",
    ] {
        let (output, summary, _) =
            docker_proof_with_failures(&["image inspect", cleanup_step], &["--no-build"]);
        assert!(!output.status.success());
        let summary = summary.expect("summary.json");
        let cleanup_error = summary["cleanup_error"].as_str().unwrap_or_default();
        assert!(
            cleanup_error.starts_with("proof_command_failed:"),
            "{cleanup_step}: {summary}"
        );
        assert_eq!(
            summary["assertions"]["cleanup_completed"], false,
            "{summary}"
        );
    }
}

#[test]
fn the_docker_proof_needs_its_certificates_before_anything_else() {
    for (failure, error) in [
        ("openssl", "proof_openssl_failed:injected failure"),
        ("no-openssl", "proof_openssl_start_failed:"),
        ("openssl-silent", "proof_mtls_read_failed:"),
        ("no-tmp", "proof_mtls_directory_failed:"),
    ] {
        let (output, summary, calls) = docker_proof_with_failures(&[failure], &["--no-build"]);
        let text = command_output_text(&output);
        assert!(!output.status.success(), "{failure}: {text}");
        assert!(text.contains(error), "{failure}: {text}");
        assert!(
            summary.is_none(),
            "{failure}: evidence before the certificates: {summary:?}"
        );
        assert!(
            calls.is_empty(),
            "{failure}: Docker before the certificates: {calls}"
        );
    }
}

#[test]
fn a_summary_that_cannot_be_written_fails_the_proof() {
    let (output, summary, calls) =
        docker_proof_with_failures(&["image inspect", "unwritable-summary"], &["--no-build"]);
    let text = command_output_text(&output);
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("proof_evidence_write_failed:summary.json:"),
        "{text}"
    );
    assert!(summary.is_none(), "{summary:?}");
    // The topology was cleaned up before the summary.
    assert!(calls.contains(" down --volumes"), "{calls}");
}
