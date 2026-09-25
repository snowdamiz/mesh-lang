//! Every Mesh example on the documentation site parses and passes the linter,
//! so the docs keep teaching the style `meshc lint` asks for.

#[path = "support/docs_examples.rs"]
mod docs_examples;

#[test]
fn website_examples_parse_and_pass_the_linter() {
    let mut problems = Vec::new();
    for (file, line, source) in docs_examples::examples() {
        match mesh_lint::lint(&source) {
            Err(error) => problems.push(format!("{file}:{line}: does not parse: {error}")),
            Ok(lints) => problems.extend(lints.into_iter().map(|lint| {
                let at = line + source[..lint.offset as usize].matches('\n').count();
                format!("{file}:{at}: {}: {}", lint.rule, lint.message)
            })),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
