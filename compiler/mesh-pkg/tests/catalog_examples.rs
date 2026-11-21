//! Catalog examples that the parser or type checker can diagnose.

use std::collections::BTreeSet;

use mesh_diagnostics::{Code, Family};
use mesh_pkg::project::{check_project, single_source_project};

/// The codes `source` gets: `P0001` for any parse error (every parse error
/// has that code, and a parse error stops the build), else the errors and
/// warnings of every module.
fn codes(source: &str) -> BTreeSet<&'static str> {
    let project = single_source_project(source).expect("a single source links");
    if project
        .module_parses
        .iter()
        .any(|parse| !parse.errors().is_empty())
    {
        return BTreeSet::from([Code::UnexpectedToken.id()]);
    }
    check_project(&project, false)
        .typeck
        .iter()
        .flat_map(|result| result.errors.iter().chain(&result.warnings))
        .map(|error| error.code().id())
        .collect()
}

#[test]
fn catalog_examples_get_exactly_their_code() {
    let mut failures = Vec::new();
    for code in
        Code::all().filter(|code| matches!(code.family(), Family::Syntax | Family::Semantic))
    {
        if matches!(
            code,
            Code::DuplicateTypeAcrossModules | Code::DuplicatePubFnAcrossModules
        ) {
            // These project checks run in meshc. The CLI suite runs their examples.
            continue;
        }
        let Some(example) = code.info().example else {
            continue;
        };
        let bad = codes(example.bad);
        if bad != BTreeSet::from([code.id()]) {
            failures.push(format!("{}: the bad example gets {bad:?}", code.id()));
        }
        let fixed = codes(example.fixed);
        if !fixed.is_empty() {
            failures.push(format!("{}: the fixed example gets {fixed:?}", code.id()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
