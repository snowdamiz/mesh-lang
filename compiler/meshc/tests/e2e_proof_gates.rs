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
    // Long and fast enough for a resumed snapshot (every 1,000 cycles) and a
    // disk plateau judged over four samples (one a second).
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
