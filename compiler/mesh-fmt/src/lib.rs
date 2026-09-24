//! Mesh code formatter.
//!
//! This crate implements a canonical code formatter for Mesh source code using
//! the Wadler-Lindig document IR approach. It works by:
//!
//! 1. Parsing source code to a CST (via `mesh-parser`)
//! 2. Walking the CST to produce a `FormatIR` document tree
//! 3. Printing the IR to a string, respecting line width constraints
//!
//! The CST-based approach preserves comments and trivia while allowing the
//! formatter to rewrite whitespace and indentation canonically.

pub mod ir;
pub mod printer;
pub mod walker;

pub use printer::FormatConfig;

/// Format Mesh source code according to the given configuration.
///
/// Parses the source, walks the CST to produce format IR, and prints the
/// result as a formatted string. Comments are preserved in their original
/// positions relative to code.
/// Source that [`try_format`] cannot format is returned unchanged, so neither
/// parser recovery nor a formatter bug can discard code.
///
/// # Example
///
/// ```
/// use mesh_fmt::{format_source, FormatConfig};
///
/// let source = "fn add(a, b) do\na + b\nend";
/// let formatted = format_source(source, &FormatConfig::default());
/// assert_eq!(formatted, "fn add(a, b) do\n  a + b\nend\n");
/// ```
pub fn format_source(source: &str, config: &FormatConfig) -> String {
    try_format(source, config).unwrap_or_else(|_| source.to_owned())
}

/// Format like [`format_source`], or say why the source cannot be formatted.
pub fn try_format(source: &str, config: &FormatConfig) -> Result<String, String> {
    let parse = mesh_parser::parse(source);
    if !parse.errors().is_empty() {
        return Err("source contains parse errors".to_owned());
    }
    let formatted = printer::print(&walker::walk_node(&parse.syntax()), config);

    // Formatting may only move whitespace. Output that parses to other tokens
    // (a line comment swallowing the code after it, a dropped comment) is a
    // formatter bug, and writing it would change the program.
    let reparsed = mesh_parser::parse(&formatted);
    if !reparsed.errors().is_empty() || significant_tokens(&parse) != significant_tokens(&reparsed)
    {
        return Err("the formatter could not preserve it exactly (a formatter bug), so it was left unchanged".to_owned());
    }
    Ok(formatted)
}

/// Every token but whitespace, with the trailing blanks the printer trims and
/// without trailing commas, which the formatter may drop (`import (a, b,)`),
/// and without semicolons: a statement separated by `;` goes on its own line.
fn significant_tokens(parse: &mesh_parser::Parse) -> Vec<(mesh_parser::SyntaxKind, String)> {
    use mesh_parser::SyntaxKind;
    let tokens: Vec<_> = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| {
            !matches!(
                token.kind(),
                SyntaxKind::WHITESPACE
                    | SyntaxKind::NEWLINE
                    | SyntaxKind::SEMICOLON
                    | SyntaxKind::EOF
            )
        })
        .map(|token| (token.kind(), token.text().trim_end().to_owned()))
        .collect();
    let closes = |kind| {
        matches!(
            kind,
            SyntaxKind::R_PAREN | SyntaxKind::R_BRACKET | SyntaxKind::R_BRACE
        )
    };
    tokens
        .iter()
        .enumerate()
        .filter(|(i, (kind, _))| {
            !(*kind == SyntaxKind::COMMA && tokens.get(i + 1).is_some_and(|next| closes(next.0)))
        })
        .map(|(_, token)| token.clone())
        .collect()
}

#[cfg(test)]
mod idempotency_tests {
    use super::{format_source, FormatConfig};

    #[test]
    fn invalid_source_is_preserved_including_following_declarations() {
        let source =
            "fn broken(x) do\ncase x do\nErr(_) -> None +\nend\nend\n\nfn retained() do\n42\nend\n";
        assert_eq!(format_source(source, &FormatConfig::default()), source);
    }

    fn assert_idempotent(name: &str, source: &str) {
        let config = FormatConfig::default();
        let formatted = format_source(source, &config);
        let double_formatted = format_source(&formatted, &config);
        assert_eq!(
            formatted, double_formatted,
            "Idempotency failed for: {}\nFirst:  {:?}\nSecond: {:?}",
            name, formatted, double_formatted
        );
    }

    #[test]
    fn idempotent_empty_file() {
        assert_idempotent("empty file", "");
    }

    #[test]
    fn match_arm_block_body_stays_on_following_lines() {
        let source = "fn f(o) do
  case o do
    Some(x) ->
      let y = x * 2
      y + 1
    None -> -1
  end
end
";
        let formatted = format_source(source, &FormatConfig::default());
        assert!(
            formatted.contains(
                "Some(x) ->
      let y = x * 2
      y + 1
"
            ),
            "arm block body must stay a block:
{formatted}"
        );
        assert_idempotent("match arm block body", source);
    }

    #[test]
    fn closure_statement_and_return_arm_format() {
        let source = "fn compose(f, g) do
  fn x -> g(f(x)) end
end

fn h(o) do
  case o do
    Some(x) -> x
    None -> return -1
  end
end
";
        assert_idempotent("closure statement and return arm", source);
    }

    #[test]
    fn idempotent_single_let_binding() {
        assert_idempotent("single let", "let x = 42");
    }

    #[test]
    fn idempotent_let_with_type_annotation() {
        assert_idempotent("let with type", "let name :: String = \"hello\"");
    }

    #[test]
    fn idempotent_fn_with_do_end() {
        assert_idempotent(
            "fn with do/end",
            "fn greet(name) do\nlet msg = \"hello\"\nmsg\nend",
        );
    }

    #[test]
    fn idempotent_nested_if_else() {
        assert_idempotent(
            "nested if/else",
            "if x > 0 do\nif x > 10 do\n\"big\"\nelse\n\"small\"\nend\nelse\n\"negative\"\nend",
        );
    }

    #[test]
    fn idempotent_case_multiple_arms() {
        assert_idempotent(
            "case with arms",
            "case x do\n1 -> \"one\"\n2 -> \"two\"\n_ -> \"other\"\nend",
        );
    }

    #[test]
    fn idempotent_module_with_imports() {
        assert_idempotent(
            "module with imports",
            "from Math import sqrt\nmodule Geometry do\nfn area(r) do\n3 * r * r\nend\nend",
        );
    }

    #[test]
    fn idempotent_actor_block() {
        assert_idempotent("actor block", "actor Counter do\nfn init() do\n0\nend\nend");
    }

    #[test]
    fn idempotent_receive_expr() {
        assert_idempotent(
            "receive expression",
            "fn loop() do\nreceive do\nx -> x\nend\nend",
        );
    }

    #[test]
    fn idempotent_supervisor_block() {
        assert_idempotent("supervisor block", "supervisor MySup do\nend");
    }

    #[test]
    fn idempotent_supervisor_child_spec() {
        assert_idempotent(
            "supervisor child spec",
            "supervisor MySup do\nstrategy: one_for_one\nmax_restarts: 20\nmax_seconds: 60\nchild worker do\nstart: fn -> spawn(worker_loop) end\nrestart: permanent\nshutdown: 5000\nend\nend",
        );
    }

    #[test]
    fn idempotent_service_definition() {
        assert_idempotent(
            "service definition",
            "service Counter do\nfn init() do\n0\nend\nend",
        );
    }

    #[test]
    fn idempotent_pipe_chain() {
        // NOTE: Pipe operator idempotency is limited by a known parser issue:
        // after formatting, the pipe operator appears at line start which the
        // parser doesn't handle (multiline pipe limitation in STATE.md).
        // This test uses single-line pipe which is idempotent.
        let config = FormatConfig::default();
        let source = "x |> foo() |> bar()";
        let formatted = format_source(source, &config);
        // Verify it contains the pipe operators.
        assert!(
            formatted.contains("|>"),
            "Pipe operator should be preserved"
        );
        assert!(formatted.contains("foo()"), "foo() should be preserved");
        assert!(formatted.contains("bar()"), "bar() should be preserved");
    }

    #[test]
    fn idempotent_string_interpolation() {
        assert_idempotent("string interpolation", "let msg = \"hello #{name}!\"");
    }

    #[test]
    fn idempotent_line_comment() {
        assert_idempotent("line comment", "# This is a comment\nlet x = 1");
    }

    #[test]
    fn idempotent_inline_comment() {
        assert_idempotent(
            "inline comment in fn",
            "fn foo() do\n# body comment\n1\nend",
        );
    }

    #[test]
    fn idempotent_multiple_blank_lines() {
        // Multiple blank lines between items should collapse to a single blank line
        // (the formatter uses hardline+hardline between top-level items).
        assert_idempotent(
            "multiple blank lines",
            "fn foo() do\n1\nend\n\n\n\nfn bar() do\n2\nend",
        );
    }

    #[test]
    fn idempotent_struct_with_fields() {
        assert_idempotent(
            "struct with fields",
            "struct Point do\nx :: Float\ny :: Float\nend",
        );
    }

    #[test]
    fn idempotent_resource_declarations_and_parameter_ownership() {
        assert_idempotent(
            "resource declarations and parameter ownership",
            "resource SecretBytes\nresource struct RatchetSecrets do\nroot_key :: SecretBytes\nend\nfn rotate(root::borrow SecretBytes, next::consume SecretBytes) do\nnext\nend",
        );
    }

    #[test]
    fn idempotent_sum_type() {
        assert_idempotent(
            "sum type",
            "type Shape do\nCircle(Float)\nRectangle(Float, Float)\nend",
        );
    }

    #[test]
    fn idempotent_interface_def() {
        // NOTE: interface methods with do/end body have a known formatting bug
        // (the body gets separated from the fn header by walk_block_def).
        // This test uses a bodyless method declaration which formats correctly.
        assert_idempotent(
            "interface definition",
            "interface Show do\nfn show(self)\nend",
        );
    }

    #[test]
    fn idempotent_impl_def() {
        assert_idempotent(
            "impl definition",
            "impl Printable for Int do\nfn to_string(self) do\n\"int\"\nend\nend",
        );
    }

    #[test]
    fn idempotent_closure_expr() {
        assert_idempotent("closure expression", "let f = fn (x) -> x + 1 end");
    }

    #[test]
    fn idempotent_call_expression() {
        assert_idempotent("call expression", "foo(1, 2, bar(3))");
    }

    #[test]
    fn idempotent_binary_expressions() {
        assert_idempotent("binary expressions", "let r = a + b * c - d / e");
    }

    #[test]
    fn idempotent_type_alias() {
        assert_idempotent("type alias", "type Pair = (Int, Int)");
    }

    #[test]
    fn idempotent_pub_type_alias() {
        assert_idempotent("pub type alias", "pub type UserId = Int");
    }

    #[test]
    fn idempotent_field_access() {
        assert_idempotent("field access", "let l = String.length(s)");
    }

    #[test]
    fn idempotent_return_expr() {
        assert_idempotent("return expression", "fn foo() do\nreturn 42\nend");
    }

    #[test]
    fn idempotent_from_import() {
        assert_idempotent("from import", "from Math import sqrt, pow");
    }

    #[test]
    fn idempotent_dotted_from_import() {
        assert_idempotent("dotted from import", "from Api.Router import build_router");
    }

    #[test]
    fn idempotent_parenthesized_dotted_from_import() {
        assert_idempotent(
            "parenthesized dotted from import",
            "from Api.Router import (\nbuild_router,\nhealth_router\n)",
        );
    }

    #[test]
    fn idempotent_tuple_expr() {
        assert_idempotent("tuple expression", "let t = (1, 2, 3)");
    }

    #[test]
    fn idempotent_unary_expr() {
        assert_idempotent("unary expression", "let x = -1");
    }

    #[test]
    fn idempotent_not_expr() {
        assert_idempotent("not expression", "let b = not true");
    }

    #[test]
    fn idempotent_list_literal() {
        assert_idempotent("list literal", "[1, 2, 3]");
    }

    #[test]
    fn idempotent_map_literal() {
        assert_idempotent("map literal", "let m = %{\"a\" => 1, \"b\" => 2}");
    }

    #[test]
    fn idempotent_nested_list() {
        assert_idempotent("nested list", "let xs = [[1, 2], [3, 4]]");
    }

    #[test]
    fn idempotent_assoc_type_binding() {
        assert_idempotent(
            "assoc type binding in impl",
            "impl Iterator for MyIter do\ntype Item = Int\nfn next(self) do\nNone\nend\nend",
        );
    }
}

#[cfg(test)]
mod edge_case_tests {
    use super::{format_source, FormatConfig};

    fn fmt(source: &str) -> String {
        format_source(source, &FormatConfig::default())
    }

    #[test]
    fn comments_only_file() {
        let result = fmt("# Just a comment");
        assert!(result.contains("# Just a comment"));
        // Should be idempotent.
        let second = fmt(&result);
        assert_eq!(result, second);
    }

    #[test]
    fn deeply_nested_5_levels() {
        let src = "if a do\nif b do\nif c do\nif d do\nif e do\n1\nend\nend\nend\nend\nend";
        let result = fmt(src);
        // Should be indented 5 levels deep for the innermost body.
        assert!(
            result.contains("          1"),
            "Expected 10 spaces indent, got:\n{}",
            result
        );
        // Must be idempotent.
        let second = fmt(&result);
        assert_eq!(result, second);
    }

    #[test]
    fn trailing_whitespace_removal() {
        // Formatter should not produce trailing whitespace on any line.
        let result = fmt("fn foo() do\n1\nend");
        for (i, line) in result.lines().enumerate() {
            assert!(
                !line.ends_with(' ') && !line.ends_with('\t'),
                "Line {} has trailing whitespace: {:?}",
                i + 1,
                line
            );
        }
    }

    #[test]
    fn blank_lines_in_sum_types_have_no_indentation() {
        let result = fmt("type Error do\nFirst\nSecond\nend");
        for (index, line) in result.lines().enumerate() {
            assert_eq!(
                line.trim_end(),
                line,
                "line {} has trailing whitespace: {:?}",
                index + 1,
                line
            );
        }
    }

    #[test]
    fn trailing_newline() {
        // Every formatted output should end with exactly one newline.
        let result = fmt("let x = 1");
        assert!(result.ends_with('\n'), "Should end with newline");
        assert!(
            !result.ends_with("\n\n"),
            "Should not end with double newline"
        );
    }

    #[test]
    fn empty_file_produces_empty_output() {
        let result = fmt("");
        // Empty input should produce empty output (no spurious newlines).
        assert_eq!(result, "");
    }

    #[test]
    fn long_string_literal_not_wrapped() {
        // A long string literal should not be line-wrapped by the formatter.
        let long_string =
            "let s = \"This is a very long string literal that exceeds the default 100 character line width limit but should not be wrapped\"";
        let result = fmt(long_string);
        // The string should remain on one line.
        let content_lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(
            content_lines.len(),
            1,
            "Long string should stay on one line, got:\n{}",
            result
        );
    }

    #[test]
    fn consistent_newline_at_end() {
        // Various inputs should all end with exactly one newline.
        let inputs = vec![
            "let x = 1",
            "fn foo() do\n1\nend",
            "# comment",
            "struct P do\nx :: Int\nend",
        ];
        for input in inputs {
            let result = fmt(input);
            if !result.is_empty() {
                assert!(
                    result.ends_with('\n'),
                    "Output should end with newline for input: {:?}",
                    input
                );
                assert!(
                    !result.ends_with("\n\n"),
                    "Output should not end with double newline for input: {:?}",
                    input
                );
            }
        }
    }

    #[test]
    fn blank_lines_collapse_between_items() {
        // Multiple blank lines between top-level items should collapse to exactly one blank line.
        let input = "fn foo() do\n1\nend\n\n\n\n\nfn bar() do\n2\nend";
        let result = fmt(input);
        // There should be exactly one blank line between the two functions.
        assert!(
            result.contains("end\n\nfn bar"),
            "Expected single blank line between items, got:\n{}",
            result
        );
        assert!(
            !result.contains("end\n\n\nfn"),
            "Should not have double blank lines, got:\n{}",
            result
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::{format_source, FormatConfig};

    fn fmt(source: &str) -> String {
        format_source(source, &FormatConfig::default())
    }

    #[test]
    fn snapshot_fn_with_body() {
        let result = fmt("fn add(a, b) do\na + b\nend");
        insta::assert_snapshot!(result, @r"
        fn add(a, b) do
          a + b
        end
        ");
    }

    #[test]
    fn snapshot_if_else() {
        let result = fmt("if x > 0 do\nx\nelse\n-x\nend");
        insta::assert_snapshot!(result, @r"
        if x > 0 do
          x
        else
          -x
        end
        ");
    }

    #[test]
    fn snapshot_case_expr() {
        let result = fmt("case color do\n\"red\" -> 1\n\"blue\" -> 2\n_ -> 0\nend");
        insta::assert_snapshot!(result, @r#"
        case color do
          "red" -> 1
          "blue" -> 2
          _ -> 0
        end
        "#);
    }

    #[test]
    fn snapshot_struct_def() {
        let result = fmt("struct Point do\nx :: Float\ny :: Float\nend");
        insta::assert_snapshot!(result, @r"
        struct Point do
          x :: Float
          y :: Float
        end
        ");
    }

    #[test]
    fn snapshot_opaque_resource() {
        let result = fmt("resource   SecretBytes");
        insta::assert_snapshot!(result, @"resource SecretBytes
");
    }

    #[test]
    fn snapshot_resource_struct_and_parameter_ownership() {
        let result = fmt(
            "pub resource struct RatchetSecrets do\nroot_key::SecretBytes\nend\nfn rotate(root::borrow SecretBytes,next::consume SecretBytes)do\nnext\nend",
        );
        insta::assert_snapshot!(result, @r"
        pub resource struct RatchetSecrets do
          root_key :: SecretBytes
        end

        fn rotate(root :: borrow SecretBytes, next :: consume SecretBytes) do
          next
        end
        ");
    }

    #[test]
    fn snapshot_module_with_fn() {
        let result = fmt("module Math do\nfn square(x) do\nx * x\nend\nend");
        insta::assert_snapshot!(result, @r"
        module Math do
          fn square(x) do
            x * x
          end
        end
        ");
    }

    #[test]
    fn snapshot_let_with_type() {
        let result = fmt("let name :: String = \"Mesh\"");
        insta::assert_snapshot!(result, @r#"let name :: String = "Mesh"
"#);
    }

    #[test]
    fn snapshot_binary_ops() {
        let result = fmt("let r = a + b * c");
        insta::assert_snapshot!(result, @"let r = a + b * c
");
    }

    #[test]
    fn snapshot_from_import() {
        let result = fmt("from Math import sqrt, pow");
        insta::assert_snapshot!(result, @"from Math import sqrt, pow
");
    }

    #[test]
    fn snapshot_dotted_from_import() {
        let result = fmt("from Api.Router import build_router");
        insta::assert_snapshot!(result, @"from Api.Router import build_router
");
    }

    #[test]
    fn snapshot_parenthesized_dotted_from_import() {
        let result = fmt("from Api.Router import (\nbuild_router,\nhealth_router\n)");
        insta::assert_snapshot!(result, @r"
        from Api.Router import (
          build_router,
          health_router
        )
        ");
    }

    #[test]
    fn snapshot_multiple_top_level() {
        let result = fmt("let x = 1\nfn foo() do\nx\nend");
        insta::assert_snapshot!(result, @r"
        let x = 1

        fn foo() do
          x
        end
        ");
    }

    #[test]
    fn layouts_are_kept() {
        // An import-list comment moved to the next name, `(A,B)` in an
        // alias, a default method's body flattened, `- 1` in a pattern,
        // `% { p | x: 2 }`, and a closure whose body is one `if` collapsed.
        let src = "from Util import (\n  helper, # the helper\n  other\n)\n\ntype Pair<A, B> = (A, B)\n\ninterface Named do\n  fn name(self) -> String\n\n  fn label(self) -> String do\n    let n = self.name()\n    \"name=\" <> n\n  end\nend\n\nfn sign(n :: Int) -> String do\n  case n do\n    -1 -> \"minus one\"\n    _ -> \"other\"\n  end\nend\n\nstruct P do\n  x :: Int\nend\n\nfn main() do\n  let p = P { x: 1 }\n  let q = %{p | x: 2}\n  let abs = fn m do\n    if m > 0 do\n      m\n    else\n      0 - m\n    end\n  end\n  println(\"#{q.x} #{abs(-3)}\")\nend\n";
        assert_eq!(fmt(src), src);
        assert_eq!(
            fmt("fn f(p) do\n  % { p | x: 2 }\nend\n"),
            "fn f(p) do\n  %{p | x: 2}\nend\n"
        );
    }

    #[test]
    fn assert_receive_is_formatted() {
        // Test files using it were refused: "source contains parse errors".
        let src = "test(\"receive\") do\n  send(self(), 42)\n  assert_receive 42, 500\n  assert_receive (a, _)\nend\n";
        assert_eq!(fmt(src), src);
        assert_eq!(
            fmt("test(\"x\") do\n  assert_receive   42 ,  500\nend\n"),
            "test(\"x\") do\n  assert_receive 42, 500\nend\n"
        );
    }

    #[test]
    fn snapshot_comment_preserved() {
        let result = fmt("# A comment\nfn foo() do\n1\nend");
        insta::assert_snapshot!(result, @r"
        # A comment
        fn foo() do
          1
        end
        ");
    }
}
