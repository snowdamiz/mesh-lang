//! Expression parser integration tests using insta snapshots.
//!
//! Each test parses a Snow expression, builds the CST, and snapshots the
//! debug tree output to verify correct precedence, associativity, and
//! tree structure.

use insta::assert_snapshot;
use snow_parser::{debug_tree, parse_expr};

fn parse_and_debug(source: &str) -> String {
    let parse = parse_expr(source);
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
