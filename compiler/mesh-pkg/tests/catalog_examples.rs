//! Catalog examples that the frontend can diagnose.

use std::path::Path;

use mesh_diagnostics::{Code, Family};
use mesh_pkg::frontend::{check, CheckOptions, Target};

/// The codes the frontend reports for `source` as a lone file, as `meshc
/// check main.mpl` checks it, sorted, one per diagnostic.
fn codes(source: &str) -> Vec<&'static str> {
    let target = Target::Source {
        path: Path::new("main.mpl"),
        text: source,
    };
    let mut codes: Vec<_> = check(target, &CheckOptions::cli())
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code().id())
        .collect();
    codes.sort();
    codes
}

#[test]
fn catalog_examples_get_exactly_their_code() {
    let mut failures = Vec::new();
    for code in
        Code::all().filter(|code| matches!(code.family(), Family::Syntax | Family::Semantic))
    {
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
