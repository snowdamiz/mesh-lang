use insta::assert_yaml_snapshot;
use mesh_lexer::Lexer;
use serde::Serialize;

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

// ── Fixture-based tests (from plan 01-02) ───────────────────────────────

#[test]
fn test_keywords() {
    let source = include_str!("../../../tests/fixtures/keywords.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_operators() {
    let source = include_str!("../../../tests/fixtures/operators.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_numbers() {
    let source = include_str!("../../../tests/fixtures/numbers.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_identifiers() {
    let source = include_str!("../../../tests/fixtures/identifiers.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

// ── Inline tests (from plan 01-02) ──────────────────────────────────────

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

// ── New fixture-based tests (plan 01-03) ────────────────────────────────

#[test]
fn test_simple_string_escapes() {
    let source = include_str!("../../../tests/fixtures/strings.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_string_interpolation() {
    let source = include_str!("../../../tests/fixtures/interpolation.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_comments() {
    let source = include_str!("../../../tests/fixtures/comments.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_newlines() {
    let source = include_str!("../../../tests/fixtures/newlines.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_error_recovery() {
    let source = include_str!("../../../tests/fixtures/error_recovery.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_full_program() {
    let source = include_str!("../../../tests/fixtures/full_program.mpl");
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

// ── New inline tests (plan 01-03) ───────────────────────────────────────

#[test]
fn test_adjacent_interpolations() {
    // Adjacent interpolations should NOT produce empty StringContent between them
    let tokens = tokenize_snapshot(r#""${a}${b}""#);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_interpolation_with_braces() {
    // Braces inside interpolation should be tracked correctly
    let tokens = tokenize_snapshot(r#""${map[key]}""#);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_triple_quoted_string() {
    let source = "\"\"\"hello\nworld\"\"\"";
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_triple_quoted_interpolation() {
    let source = "\"\"\"hello ${name}\nworld\"\"\"";
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_nested_block_comment() {
    let tokens = tokenize_snapshot("#= outer #= inner =# outer =#");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_empty_input() {
    let tokens = tokenize_snapshot("");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_whitespace_only() {
    let tokens = tokenize_snapshot("   \t  ");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_span_accuracy_interpolation() {
    // Verify spans are byte-accurate for interpolation
    let source = r#""hello ${name}""#;
    let tokens = tokenize_snapshot(source);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_unterminated_block_comment() {
    let tokens = tokenize_snapshot("#= no close");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_empty_string() {
    let tokens = tokenize_snapshot(r#""""#);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_interpolation_with_expression() {
    // Complex expression inside interpolation
    let tokens = tokenize_snapshot(r#""result: ${a + b * 2}""#);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_escaped_dollar_in_string() {
    // Escaped dollar should not start interpolation
    let tokens = tokenize_snapshot(r#""price: \$100""#);
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_crlf_newlines() {
    let tokens = tokenize_snapshot("let x = 1\r\nlet y = 2");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_consecutive_newlines() {
    let tokens = tokenize_snapshot("let x = 1\n\n\nlet y = 2");
    assert_yaml_snapshot!(tokens);
}

// ── Actor keyword tests (plan 06-02) ─────────────────────────────────

#[test]
fn test_actor_keyword() {
    let tokens = tokenize_snapshot("actor MyCounter do end");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_terminate_keyword() {
    let tokens = tokenize_snapshot("terminate do end");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_spawn_expr_tokens() {
    let tokens = tokenize_snapshot("spawn(func, args)");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_send_expr_tokens() {
    let tokens = tokenize_snapshot("send(pid, msg)");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_receive_block_tokens() {
    let tokens = tokenize_snapshot("receive do end");
    assert_yaml_snapshot!(tokens);
}

#[test]
fn test_self_expr_tokens() {
    let tokens = tokenize_snapshot("self()");
    assert_yaml_snapshot!(tokens);
}

/// `:name` is an atom, but a `:` right after a name or a closing bracket
/// separates a key from its value: `x:y` is `x`, `:`, `y`.
#[test]
fn colon_after_an_operand_is_not_an_atom() {
    let kinds = |source: &str| -> Vec<String> {
        Lexer::tokenize(source)
            .into_iter()
            .map(|tok| format!("{:?}", tok.kind))
            .filter(|kind| kind != "Eof")
            .collect()
    };
    assert_eq!(kinds("x:y"), ["Ident", "Colon", "Ident"]);
    assert_eq!(
        kinds("f():y"),
        ["Ident", "LParen", "RParen", "Colon", "Ident"]
    );
    assert_eq!(kinds("x: :ok"), ["Ident", "Colon", "Atom"]);
    assert_eq!(kinds("[:ok]"), ["LBracket", "Atom", "RBracket"]);
    assert_eq!(kinds("f(:ok)"), ["Ident", "LParen", "Atom", "RParen"]);
}

fn kinds(source: &str) -> Vec<String> {
    Lexer::tokenize(source)
        .into_iter()
        .map(|tok| format!("{:?}", tok.kind))
        .filter(|kind| kind != "Eof")
        .collect()
}

/// Malformed input becomes `Error` tokens, and lexing goes on after them.
#[test]
fn malformed_input_lexes_as_error_tokens() {
    assert_eq!(kinds("~x"), ["Error", "Ident"]);
    assert_eq!(kinds("a & b"), ["Ident", "Error", "Ident"]);
    assert_eq!(kinds("`"), ["Error"]);
    // A slot pipe needs its `>` and a position from 2.
    assert_eq!(kinds("|5 x"), ["Error", "Ident"]);
    assert_eq!(kinds("|1>"), ["Error"]);
    assert_eq!(kinds("|0>"), ["Error"]);
    assert_eq!(kinds("|2>"), ["SlotPipe(2)"]);
    // A regex needs `/`, an end, and only the flags i, m and s.
    assert_eq!(kinds("~rx"), ["Error", "Ident"]);
    assert_eq!(kinds("~r/abc"), ["Error"]);
    assert_eq!(kinds("~r/a/q"), ["Error"]);
    assert_eq!(
        kinds("~r/a\\/b/ims"),
        ["RegexLiteral(\"a\\\\/b\", \"ims\")"]
    );
}

/// Inside `#{...}` the lexer reads code as it does outside a string, and
/// counts braces to find the interpolation's end.
#[test]
fn interpolations_lex_as_code_until_their_closing_brace() {
    assert_eq!(
        kinds("\"#{f(@x; y) # c\r\n}\""),
        [
            "StringStart",
            "InterpolationStart",
            "Ident",
            "LParen",
            "At",
            "Ident",
            "Semicolon",
            "Ident",
            "RParen",
            "Comment",
            "Newline",
            "InterpolationEnd",
            "StringEnd"
        ]
    );
    assert_eq!(
        kinds("\"#{ %{a => 1}\n}\""),
        [
            "StringStart",
            "InterpolationStart",
            "Percent",
            "LBrace",
            "Ident",
            "FatArrow",
            "IntLiteral",
            "RBrace",
            "Newline",
            "InterpolationEnd",
            "StringEnd"
        ]
    );
    assert_eq!(
        kinds("\"#{`}\""),
        [
            "StringStart",
            "InterpolationStart",
            "Error",
            "InterpolationEnd",
            "StringEnd"
        ]
    );
    // An interpolation cut off by the end of the file: the interpolation and
    // then the string are unterminated.
    assert_eq!(
        kinds("\"#{x"),
        [
            "StringStart",
            "InterpolationStart",
            "Ident",
            "Error",
            "Error"
        ]
    );
}

/// Literal and comment forms the fixtures do not cover.
#[test]
fn exponents_block_comments_and_heredoc_quotes() {
    // An integer with an exponent is a float.
    assert_eq!(kinds("1e3 2.5e-1"), ["FloatLiteral", "FloatLiteral"]);
    // `=` inside a block comment does not end it unless `#` follows.
    assert_eq!(kinds("#= a = b =# x"), ["Comment", "Ident"]);
    // Quotes inside a heredoc are content, escaped or not, until `"""`.
    assert_eq!(
        kinds("\"\"\"say \"hi\" \\\"x\\\" \"\" done\"\"\""),
        ["StringStart", "StringContent", "StringEnd"]
    );
    assert_eq!(kinds("\"\"\"\"\"\""), ["StringStart", "StringEnd"]);
    // A string cut off by the end of the file.
    assert_eq!(kinds("\"abc"), ["StringStart", "StringContent", "Error"]);
}
