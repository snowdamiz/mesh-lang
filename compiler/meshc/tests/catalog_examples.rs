//! The catalog examples of the codes only code generation reports. The
//! frontend and the editor never see them, so `meshc build` is what must
//! report exactly that code for the bad program and build the fixed one.

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::process::Command;

use mesh_diagnostics::Code;

/// The codes code generation reports for a program the frontend accepts.
/// `mesh-pkg`'s `catalog_examples` and `mesh-lsp`'s `parity` skip them.
const CODEGEN_ONLY: &[Code] = &[
    Code::RecursiveByValueType,
    Code::RuntimeSymbolCollision,
    Code::SpawnGroupedUnsafe,
    Code::NodeSpawnArgument,
    Code::ExportSymbolCollision,
];

/// Whether `meshc build --json` built `source` as a project's `main.mpl`,
/// and the code of each diagnostic it reported.
fn build(source: &str) -> (bool, Vec<String>) {
    let project = tempfile::tempdir().expect("temp project");
    std::fs::write(project.path().join("main.mpl"), source).expect("main.mpl");
    let output = Command::new(artifacts::meshc_bin())
        .args(["build", "--json"])
        .arg(project.path())
        .output()
        .expect("meshc build runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let codes = stderr
        .lines()
        .map(|line| {
            let diagnostic: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("not a JSON diagnostic ({error}): {line}"));
            diagnostic["code"].as_str().expect("a code").to_string()
        })
        .collect();
    (output.status.success(), codes)
}

#[test]
fn codegen_examples_get_exactly_their_code() {
    let failures: Vec<String> = std::thread::scope(|scope| {
        let runs: Vec<_> = CODEGEN_ONLY
            .iter()
            .map(|&code| {
                scope.spawn(move || {
                    let example = code
                        .info()
                        .example
                        .unwrap_or_else(|| panic!("{} has no example", code.id()));
                    let mut failures = Vec::new();
                    let (built, codes) = build(example.bad);
                    if built || codes != [code.id()] {
                        failures.push(format!(
                            "{}: the bad example built: {built}, reported {codes:?}",
                            code.id()
                        ));
                    }
                    let (built, codes) = build(example.fixed);
                    if !built || !codes.is_empty() {
                        failures.push(format!(
                            "{}: the fixed example built: {built}, reported {codes:?}",
                            code.id()
                        ));
                    }
                    failures
                })
            })
            .collect();
        runs.into_iter()
            .flat_map(|run| run.join().expect("an example run"))
            .collect()
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
