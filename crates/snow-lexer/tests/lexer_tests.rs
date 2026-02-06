use insta::assert_yaml_snapshot;
use serde::Serialize;
use snow_lexer::Lexer;

/// A human-readable representation of a token for snapshot testing.
#[derive(Serialize)]
struct TokenSnapshot {
    kind: String,
    text: String,
    span: (u32, u32),
}

/// Tokenize source and return a list of snapshot-friendly token representations.
fn tokenize_snapshot(source: &str) -> Vec<TokenSnapshot> {
    Lexer::tokenize(source)
        .into_iter()
        .map(|tok| {
            let text = if tok.span.start < tok.span.end {
                source[tok.span.start as usize..tok.span.end as usize].to_string()
            } else {
                String::new()
            };
            TokenSnapshot {
                kind: format!("{:?}", tok.kind),
                text,
                span: (tok.span.start, tok.span.end),
            }
        })
        .collect()
}

// ── Fixture-based tests ──────────────────────────────────────────────────

#[test]
fn test_keywords() {
    let source = include_str!("../../../tests/fixtures/keywords.snow");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_operators() {
    let source = include_str!("../../../tests/fixtures/operators.snow");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_numbers() {
    let source = include_str!("../../../tests/fixtures/numbers.snow");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_identifiers() {
    let source = include_str!("../../../tests/fixtures/identifiers.snow");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

// ── Inline tests ─────────────────────────────────────────────────────────

#[test]
fn test_simple_string() {
    let tokens = tokenize_snapshot(r#""hello world""#);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_line_comment() {
    let tokens = tokenize_snapshot("# this is a comment");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_doc_comment() {
    let tokens = tokenize_snapshot("## this is a doc comment");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_module_doc_comment() {
    let tokens = tokenize_snapshot("##! module doc");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_mixed_expression() {
    let tokens = tokenize_snapshot("let result = add(1, 2) |> multiply(3)");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_spans_accurate() {
    let tokens = tokenize_snapshot("let x = 42");
    // Verify exact span values via snapshot
    assert_yaml_snapshot!(tokens);
}
