//! Parser integration tests using insta snapshots.
//!
//! Each test parses a Snow expression/declaration/program, builds the CST,
//! and snapshots the debug tree output to verify correct structure.

use insta::assert_snapshot;
use snow_parser::{debug_tree, parse, parse_block, parse_expr};

fn parse_and_debug(source: &str) -> String {
    let parse = parse_expr(source);
    format_parse(&parse)
}

fn block_and_debug(source: &str) -> String {
    let parse = parse_block(source);
    format_parse(&parse)
}

fn source_and_debug(source: &str) -> String {
    let parse = parse(source);
    format_parse(&parse)
}

fn format_parse(parse: &snow_parser::Parse) -> String {
    let tree = debug_tree(&parse.syntax());
    if !parse.errors().is_empty() {
        format!(
            "{}\nerrors:\n{}",
            tree,
            parse
                .errors()
                .iter()
                .map(|e| format!("  - {} @{}..{}", e.message, e.span.start, e.span.end))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        tree
    }
}

// ── Literals ───────────────────────────────────────────────────────────

#[test]
fn literal_int() {
    assert_snapshot!(parse_and_debug("42"));
}

#[test]
fn literal_float() {
    assert_snapshot!(parse_and_debug("3.14"));
}

#[test]
fn literal_true() {
    assert_snapshot!(parse_and_debug("true"));
}

#[test]
fn literal_false() {
    assert_snapshot!(parse_and_debug("false"));
}

#[test]
fn literal_nil() {
    assert_snapshot!(parse_and_debug("nil"));
}

#[test]
fn literal_string() {
    assert_snapshot!(parse_and_debug("\"hello\""));
}

// ── Simple Binary Expressions ──────────────────────────────────────────

#[test]
fn binary_add() {
    assert_snapshot!(parse_and_debug("1 + 2"));
}

#[test]
fn binary_mul_add_precedence() {
    // * binds tighter than +, so: a * b + c => (a * b) + c
    assert_snapshot!(parse_and_debug("a * b + c"));
}

// ── Precedence Chain ───────────────────────────────────────────────────

#[test]
fn precedence_chain() {
    // 1 + 2 * 3 - 4 / 2 => (1 + (2 * 3)) - (4 / 2)
    assert_snapshot!(parse_and_debug("1 + 2 * 3 - 4 / 2"));
}

// ── Unary Prefix ───────────────────────────────────────────────────────

#[test]
fn unary_negate() {
    assert_snapshot!(parse_and_debug("-x"));
}

#[test]
fn unary_bang() {
    assert_snapshot!(parse_and_debug("!flag"));
}

#[test]
fn unary_not_keyword() {
    assert_snapshot!(parse_and_debug("not done"));
}

// ── Unary with Binary ──────────────────────────────────────────────────

#[test]
fn unary_with_binary() {
    // -x + y => (-x) + y (unary binds tighter)
    assert_snapshot!(parse_and_debug("-x + y"));
}

// ── Comparison ─────────────────────────────────────────────────────────

#[test]
fn comparison_eq() {
    assert_snapshot!(parse_and_debug("a == b"));
}

#[test]
fn comparison_lt_with_arithmetic() {
    // x < y + 1 => x < (y + 1) (+ binds tighter than <)
    assert_snapshot!(parse_and_debug("x < y + 1"));
}

// ── Logical ────────────────────────────────────────────────────────────

#[test]
fn logical_and_or() {
    // a and b or c => (a and b) or c (and binds tighter than or)
    assert_snapshot!(parse_and_debug("a and b or c"));
}

// ── Pipe ───────────────────────────────────────────────────────────────

#[test]
fn pipe_simple() {
    assert_snapshot!(parse_and_debug("x |> foo()"));
}

#[test]
fn pipe_chain() {
    // x |> foo() |> bar() => ((x |> foo()) |> bar()) -- left-associative
    assert_snapshot!(parse_and_debug("x |> foo() |> bar()"));
}

// ── Function Calls ─────────────────────────────────────────────────────

#[test]
fn call_no_args() {
    assert_snapshot!(parse_and_debug("foo()"));
}

#[test]
fn call_with_args() {
    assert_snapshot!(parse_and_debug("foo(1, 2, 3)"));
}

#[test]
fn call_with_expr_arg() {
    assert_snapshot!(parse_and_debug("foo(a, b + c)"));
}

// ── Nested Calls ───────────────────────────────────────────────────────

#[test]
fn nested_calls() {
    assert_snapshot!(parse_and_debug("foo(bar(x))"));
}

// ── Field Access ───────────────────────────────────────────────────────

#[test]
fn field_access_single() {
    assert_snapshot!(parse_and_debug("a.b"));
}

#[test]
fn field_access_chain() {
    // a.b.c => (a.b).c -- left-to-right
    assert_snapshot!(parse_and_debug("a.b.c"));
}

// ── Index Access ───────────────────────────────────────────────────────

#[test]
fn index_access() {
    assert_snapshot!(parse_and_debug("a[0]"));
}

#[test]
fn index_with_expr() {
    assert_snapshot!(parse_and_debug("a[i + 1]"));
}

// ── Mixed Postfix ──────────────────────────────────────────────────────

#[test]
fn mixed_postfix() {
    // a.b(c)[d] => ((a.b)(c))[d]
    assert_snapshot!(parse_and_debug("a.b(c)[d]"));
}

// ── Grouped Expression ─────────────────────────────────────────────────

#[test]
fn grouped_expression() {
    // (a + b) * c => (group(a + b)) * c
    assert_snapshot!(parse_and_debug("(a + b) * c"));
}

// ── String Interpolation ───────────────────────────────────────────────

#[test]
fn string_interpolation() {
    assert_snapshot!(parse_and_debug("\"hello ${name} world\""));
}

// ── Pipe with Calls ────────────────────────────────────────────────────

#[test]
fn pipe_with_calls() {
    assert_snapshot!(parse_and_debug("data |> map(f) |> filter(g)"));
}

// ── Error Cases ────────────────────────────────────────────────────────

#[test]
fn error_missing_lhs() {
    // + by itself should produce an error
    assert_snapshot!(parse_and_debug("+"));
}

// ── Range ──────────────────────────────────────────────────────────────

#[test]
fn range_operator() {
    assert_snapshot!(parse_and_debug("1..10"));
}

// ── Concatenation ──────────────────────────────────────────────────────

#[test]
fn concat_diamond() {
    assert_snapshot!(parse_and_debug("a <> b"));
}

#[test]
fn concat_plus_plus() {
    assert_snapshot!(parse_and_debug("a ++ b"));
}

// ── Tuple ──────────────────────────────────────────────────────────────

#[test]
fn tuple_expression() {
    assert_snapshot!(parse_and_debug("(1, 2, 3)"));
}

#[test]
fn empty_tuple() {
    assert_snapshot!(parse_and_debug("()"));
}

// ── Modulo ─────────────────────────────────────────────────────────────

#[test]
fn modulo_operator() {
    assert_snapshot!(parse_and_debug("a % b"));
}

// ── Let Bindings ──────────────────────────────────────────────────────

#[test]
fn let_simple() {
    assert_snapshot!(block_and_debug("let x = 5"));
}

#[test]
fn let_with_type_annotation() {
    assert_snapshot!(block_and_debug("let name :: String = \"hello\""));
}

#[test]
fn let_multiple_statements() {
    assert_snapshot!(block_and_debug("let x = 1\nlet y = 2"));
}

// ── Return ────────────────────────────────────────────────────────────

#[test]
fn return_with_value() {
    assert_snapshot!(block_and_debug("return x"));
}

#[test]
fn return_with_expr() {
    assert_snapshot!(block_and_debug("return x + 1"));
}

// ── If/Else ───────────────────────────────────────────────────────────

#[test]
fn if_simple() {
    assert_snapshot!(parse_and_debug("if true do\n  1\nend"));
}

#[test]
fn if_else() {
    assert_snapshot!(parse_and_debug("if x > 0 do\n  x\nelse\n  -x\nend"));
}

#[test]
fn if_else_if_else() {
    assert_snapshot!(parse_and_debug("if a do\n  1\nelse if b do\n  2\nelse\n  3\nend"));
}

#[test]
fn if_single_line() {
    assert_snapshot!(parse_and_debug("if true do 1 end"));
}

// ── Case/Match ────────────────────────────────────────────────────────

#[test]
fn case_simple() {
    assert_snapshot!(parse_and_debug("case x do\n  1 -> \"one\"\n  2 -> \"two\"\nend"));
}

#[test]
fn match_boolean() {
    assert_snapshot!(parse_and_debug("match value do\n  true -> 1\n  false -> 0\nend"));
}

// ── Closures ──────────────────────────────────────────────────────────

#[test]
fn closure_single_param() {
    assert_snapshot!(parse_and_debug("fn (x) -> x + 1 end"));
}

#[test]
fn closure_two_params() {
    assert_snapshot!(parse_and_debug("fn (x, y) -> x + y end"));
}

#[test]
fn closure_no_params() {
    assert_snapshot!(parse_and_debug("fn () -> 42 end"));
}

// ── Blocks ────────────────────────────────────────────────────────────

#[test]
fn block_multi_statement() {
    assert_snapshot!(block_and_debug("let x = 1\nx + 1"));
}

// ── Trailing Closures ────────────────────────────────────────────────

#[test]
fn trailing_closure_basic() {
    assert_snapshot!(parse_and_debug("run() do\n  42\nend"));
}

// ── Error Cases (compound) ───────────────────────────────────────────

#[test]
fn error_if_missing_end() {
    assert_snapshot!(parse_and_debug("if x do\n  1\n"));
}

#[test]
fn error_let_missing_ident() {
    assert_snapshot!(block_and_debug("let = 5"));
}

// ── Newline Significance ─────────────────────────────────────────────

#[test]
fn newlines_inside_parens_ignored() {
    assert_snapshot!(parse_and_debug("foo(\n  1,\n  2\n)"));
}

// ── Return bare (no value) ───────────────────────────────────────────

#[test]
fn return_bare() {
    assert_snapshot!(block_and_debug("return"));
}

// ── Case with when guard ─────────────────────────────────────────────

#[test]
fn case_with_when_guard() {
    assert_snapshot!(parse_and_debug("case x do\n  n when n > 0 -> n\n  _ -> 0\nend"));
}

// ═══════════════════════════════════════════════════════════════════════
// Plan 02-04: Declarations, Patterns, and Types
// ═══════════════════════════════════════════════════════════════════════

// ── Function Definitions ─────────────────────────────────────────────

#[test]
fn fn_def_simple() {
    assert_snapshot!(source_and_debug("fn greet(name) do\n  \"hello\"\nend"));
}

#[test]
fn fn_def_pub() {
    assert_snapshot!(source_and_debug("pub fn add(x, y) do\n  x + y\nend"));
}

#[test]
fn fn_def_typed_params_and_return() {
    assert_snapshot!(source_and_debug("fn typed(x :: Int, y :: Int) -> Int do\n  x + y\nend"));
}

#[test]
fn def_keyword() {
    assert_snapshot!(source_and_debug("def greet(name) do\n  \"hello\"\nend"));
}

#[test]
fn fn_def_no_params() {
    assert_snapshot!(source_and_debug("fn hello() do\n  \"world\"\nend"));
}

// ── Module Definitions ──────────────────────────────────────────────

#[test]
fn module_simple() {
    assert_snapshot!(source_and_debug("module Math do\n  pub fn add(x, y) do\n    x + y\n  end\nend"));
}

#[test]
fn module_nested() {
    assert_snapshot!(source_and_debug("module Outer do\n  module Inner do\n  end\nend"));
}

// ── Import Declarations ─────────────────────────────────────────────

#[test]
fn import_simple() {
    assert_snapshot!(source_and_debug("import Math"));
}

#[test]
fn import_dotted_path() {
    assert_snapshot!(source_and_debug("import Foo.Bar.Baz"));
}

#[test]
fn from_import() {
    assert_snapshot!(source_and_debug("from Math import sqrt, pow"));
}

// ── Struct Definitions ──────────────────────────────────────────────

#[test]
fn struct_simple() {
    assert_snapshot!(source_and_debug("struct Point do\n  x :: Float\n  y :: Float\nend"));
}

#[test]
fn struct_pub_with_generics() {
    assert_snapshot!(source_and_debug("pub struct Pair[A, B] do\n  first :: A\n  second :: B\nend"));
}

// ── Pattern Matching ────────────────────────────────────────────────

#[test]
fn case_with_literal_patterns() {
    assert_snapshot!(parse_and_debug("case x do\n  0 -> \"zero\"\n  _ -> \"other\"\nend"));
}

#[test]
fn let_tuple_destructure() {
    assert_snapshot!(source_and_debug("let (a, b) = pair"));
}

#[test]
fn case_with_tuple_patterns() {
    assert_snapshot!(parse_and_debug("case point do\n  (0, 0) -> \"origin\"\n  (x, y) -> \"other\"\nend"));
}

#[test]
fn case_with_negative_literal() {
    assert_snapshot!(parse_and_debug("case x do\n  -1 -> \"neg\"\n  0 -> \"zero\"\n  1 -> \"pos\"\nend"));
}

#[test]
fn case_with_string_pattern() {
    assert_snapshot!(parse_and_debug("case s do\n  \"hello\" -> 1\n  _ -> 0\nend"));
}

// ── Full Programs (integration) ─────────────────────────────────────

#[test]
fn full_program_module_with_fns() {
    let source = "\
module Math do
  pub fn add(x, y) do
    x + y
  end

  pub fn sub(x, y) do
    x - y
  end
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_program_imports_and_pipes() {
    let source = "\
import IO
from Math import sqrt

fn main() do
  42 |> sqrt() |> IO.puts()
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_program_struct_and_fn() {
    let source = "\
struct Point do
  x :: Float
  y :: Float
end

fn distance(p :: Point) -> Float do
  p.x + p.y
end";
    assert_snapshot!(source_and_debug(source));
}

// ── Error Cases (declarations) ──────────────────────────────────────

#[test]
fn error_fn_missing_name() {
    assert_snapshot!(source_and_debug("fn do end"));
}

#[test]
fn error_glob_import() {
    assert_snapshot!(source_and_debug("from Math import *"));
}

// ── Public parse() API end-to-end ───────────────────────────────────

#[test]
fn parse_api_simple_expression() {
    // parse() should work with simple expressions
    assert_snapshot!(source_and_debug("1 + 2"));
}

#[test]
fn parse_api_let_binding() {
    assert_snapshot!(source_and_debug("let x = 42"));
}
