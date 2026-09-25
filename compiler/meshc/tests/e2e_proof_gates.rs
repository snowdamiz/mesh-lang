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
    let output = proof(&[
        "continuity-soak",
        "--duration-seconds",
        "2",
        "--cycle-millis",
        "20",
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
    let summary = summary(evidence.path());
    assert_eq!(summary["schema_version"], 2, "{summary}");
    assert_eq!(summary["iterations"], 200, "{summary}");
    assert_eq!(summary["pass"], output.status.success(), "{summary}");

    let budget = evidence.path().join("budget.json");
    std::fs::write(&budget, r#"{"schema_version": 1}"#).unwrap();
    let output = proof(&[
        "autonomous-performance",
        "--budget",
        budget.to_str().unwrap(),
    ]);
    assert!(
        command_output_text(&output).contains("performance_budget_"),
        "{}",
        command_output_text(&output)
    );
}
