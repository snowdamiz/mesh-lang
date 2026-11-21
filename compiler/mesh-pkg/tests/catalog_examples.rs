//! Catalog examples that the frontend and the linter can diagnose.

use std::{fs, path::Path};

use mesh_diagnostics::{Code, Family};
use mesh_pkg::frontend::{check, CheckOptions, Target};

/// The codes `meshc check main.mpl` and `meshc lint main.mpl` report for
/// `source` as a lone file, sorted, one per diagnostic.
fn codes(source: &str) -> Vec<&'static str> {
    let target = Target::Source {
        path: Path::new("main.mpl"),
        text: source,
    };
    target_codes(target)
}

fn target_codes(target: Target<'_>) -> Vec<&'static str> {
    let frontend = check(target, &CheckOptions::cli());
    let lints = frontend
        .lintable()
        .flat_map(|(parse, file)| mesh_lint::lint(parse, file));
    let mut codes: Vec<_> = frontend
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code().id())
        .chain(lints.map(|lint| lint.code().id()))
        .collect();
    codes.sort();
    codes
}

#[test]
fn catalog_examples_get_exactly_their_code() {
    let mut failures = Vec::new();
    for code in Code::all().filter(|code| {
        matches!(
            code.family(),
            Family::Syntax | Family::Semantic | Family::Lint
        ) && *code != Code::InvalidNativeDeclaration
    }) {
        let Some(example) = code.info().example else {
            continue;
        };
        let bad = codes(example.bad);
        if bad != [code.id()] {
            failures.push(format!("{}: the bad example gets {bad:?}", code.id()));
        }
        let fixed = codes(example.fixed);
        if !fixed.is_empty() {
            failures.push(format!("{}: the fixed example gets {fixed:?}", code.id()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn native_declaration_example_in_a_manifest_declared_binding() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("mesh.toml"),
        "[package]\nname = \"example\"\nversion = \"0.1.0\"\n\n[native]\nabi = 1\nbindings = [\"binding.mpl\"]\n",
    )
    .unwrap();
    fs::write(root.join("main.mpl"), "fn main() do\nend\n").unwrap();
    let binding = root.join("binding.mpl");
    let example = Code::InvalidNativeDeclaration.info().example.unwrap();

    fs::write(&binding, example.bad).unwrap();
    assert_eq!(target_codes(Target::Project { root }), ["E0052"]);

    fs::write(&binding, example.fixed).unwrap();
    assert!(target_codes(Target::Project { root }).is_empty());
}
