//! Every Mesh example on the documentation site parses and passes the linter,
//! so the docs keep teaching the style `meshc lint` asks for.

#[path = "support/docs_examples.rs"]
mod docs_examples;

use mesh_diagnostics::SourceFile;

#[test]
fn website_examples_parse_and_pass_the_linter() {
    let mut problems = Vec::new();
    for (file, line, source) in docs_examples::examples() {
        let parse = mesh_parser::parse(&source);
        if let Some(error) = parse.errors().first() {
            problems.push(format!("{file}:{line}: does not parse: {error}"));
            continue;
        }
        let at = |found: u32| line + found as usize - 1;
        problems.extend(
            mesh_lint::lint(&parse, &SourceFile::new(&file, source.as_str()))
                .iter()
                .map(|lint| {
                    let found = lint.line().expect("a finding has a line");
                    format!(
                        "{file}:{}: {}: {}",
                        at(found),
                        lint.code().id(),
                        lint.message()
                    )
                }),
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
