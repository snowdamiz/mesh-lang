//! Parser integration tests using insta snapshots.
//!
//! Each test parses a Snow expression/declaration/program, builds the CST,
//! and snapshots the debug tree output to verify correct structure.

use insta::assert_snapshot;
use snow_parser::ast::expr::{BinaryExpr, IfExpr, Literal};
use snow_parser::ast::item::{FnDef, LetBinding, SourceFile, StructDef};
use snow_parser::{debug_tree, parse, parse_block, parse_expr, AstNode};

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
    assert_snapshot!(source_and_debug("pub struct Pair<A, B> do\n  first :: A\n  second :: B\nend"));
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

// ═══════════════════════════════════════════════════════════════════════
// Plan 02-05: Typed AST Wrappers and Comprehensive Tests
// ═══════════════════════════════════════════════════════════════════════

// ── Full Program Snapshot Tests ──────────────────────────────────────

#[test]
fn full_complete_module() {
    let source = "\
module StringUtils do
  pub fn upcase(s :: String) -> String do
    s
  end

  pub fn downcase(s :: String) -> String do
    s
  end

  fn helper(x) do
    x
  end
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_program_with_imports_pipes_closures() {
    let source = "\
import IO
from List import map, filter

fn main() do
  let nums = (1, 2, 3, 4, 5)
  nums |> filter(fn (x) -> x > 2 end) |> map(fn (x) -> x * 2 end)
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_nested_if_else_with_case() {
    let source = "\
fn classify(x) do
  if x > 0 do
    case x do
      1 -> \"one\"
      2 -> \"two\"
      _ -> \"many\"
    end
  else if x == 0 do
    \"zero\"
  else
    \"negative\"
  end
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_struct_definition_and_usage() {
    let source = "\
pub struct Point<T> do
  x :: T
  y :: T
end

fn origin() -> Point do
  let p = Point
  p
end

fn translate(p :: Point, dx :: Float, dy :: Float) -> Point do
  p
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_multiple_imports_and_nested_modules() {
    let source = "\
import IO
import Math
from String import split, join, trim

module App do
  module Config do
    fn default() do
      42
    end
  end

  pub fn run() do
    let cfg = Config.default()
    cfg |> IO.inspect()
  end
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_pattern_matching_program() {
    let source = "\
fn fibonacci(n) do
  case n do
    0 -> 0
    1 -> 1
    _ -> fibonacci(n - 1) + fibonacci(n - 2)
  end
end

fn classify_pair(pair) do
  case pair do
    (0, 0) -> \"origin\"
    (x, 0) -> \"x-axis\"
    (0, y) -> \"y-axis\"
    (x, y) -> \"general\"
  end
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_closure_and_higher_order() {
    let source = "\
fn apply(f, x) do
  f(x)
end

fn compose(f, g) do
  fn (x) -> f(g(x)) end
end

fn main() do
  let double = fn (x) -> x * 2 end
  let inc = fn (x) -> x + 1 end
  let double_then_inc = compose(inc, double)
  apply(double_then_inc, 5)
end";
    assert_snapshot!(source_and_debug(source));
}

#[test]
fn full_chained_pipes_and_field_access() {
    let source = "\
fn process(data) do
  data.items
    |> filter(fn (x) -> x.active end)
    |> map(fn (x) -> x.value end)
end";
    assert_snapshot!(source_and_debug(source));
}

// ── AST Accessor Tests ──────────────────────────────────────────────

#[test]
fn ast_fn_def_accessors() {
    let p = parse("pub fn add(x, y) do\n  x + y\nend");
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let tree = p.tree();
    let fn_def: FnDef = tree.fn_defs().next().expect("should have fn def");

    // Visibility
    assert!(fn_def.visibility().is_some(), "should have pub visibility");

    // Name
    let name = fn_def.name().expect("should have name");
    assert_eq!(name.text().unwrap(), "add");

    // Param list
    let param_list = fn_def.param_list().expect("should have param list");
    let params: Vec<_> = param_list.params().collect();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name().unwrap().text(), "x");
    assert_eq!(params[1].name().unwrap().text(), "y");

    // Body
    assert!(fn_def.body().is_some(), "should have body block");
}

#[test]
fn ast_fn_def_with_return_type() {
    let p = parse("fn typed(x :: Int) -> Int do\n  x\nend");
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let tree = p.tree();
    let fn_def: FnDef = tree.fn_defs().next().expect("should have fn def");

    // No visibility
    assert!(fn_def.visibility().is_none());

    // Return type
    let ret_type = fn_def.return_type().expect("should have return type");
    assert_eq!(ret_type.type_name().unwrap().text(), "Int");

    // Param with type annotation
    let param_list = fn_def.param_list().unwrap();
    let param = param_list.params().next().unwrap();
    assert_eq!(param.name().unwrap().text(), "x");
    let ann = param.type_annotation().expect("should have type annotation");
    assert_eq!(ann.type_name().unwrap().text(), "Int");
}

#[test]
fn ast_let_binding_accessors() {
    let p = parse("let x :: Int = 5");
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let tree = p.tree();

    let let_binding: LetBinding = tree
        .syntax()
        .children()
        .find_map(LetBinding::cast)
        .expect("should have let binding");

    // Name
    let name = let_binding.name().expect("should have name");
    assert_eq!(name.text().unwrap(), "x");

    // Type annotation
    let ann = let_binding.type_annotation().expect("should have type annotation");
    assert_eq!(ann.type_name().unwrap().text(), "Int");

    // Initializer
    let init = let_binding.initializer().expect("should have initializer");
    match init {
        snow_parser::ast::expr::Expr::Literal(_) => {} // expected
        other => panic!("expected Literal, got {:?}", other),
    }
}

#[test]
fn ast_if_expr_accessors() {
    let p = parse_expr("if true do 1 else 2 end");
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let root = p.syntax();

    let if_expr: IfExpr = root
        .children()
        .find_map(IfExpr::cast)
        .expect("should have if expr");

    // Condition
    let cond = if_expr.condition().expect("should have condition");
    match cond {
        snow_parser::ast::expr::Expr::Literal(_) => {} // true
        other => panic!("expected Literal condition, got {:?}", other),
    }

    // Then branch
    assert!(if_expr.then_branch().is_some(), "should have then branch");

    // Else branch
    let else_br = if_expr.else_branch().expect("should have else branch");
    assert!(else_br.block().is_some(), "else should have a block");
}

#[test]
fn ast_struct_def_accessors() {
    let p = parse("pub struct Point do\n  x :: Float\n  y :: Float\nend");
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let tree = p.tree();

    let struct_def: StructDef = tree
        .syntax()
        .children()
        .find_map(StructDef::cast)
        .expect("should have struct def");

    // Visibility
    assert!(struct_def.visibility().is_some());

    // Name
    assert_eq!(struct_def.name().unwrap().text().unwrap(), "Point");

    // Fields
    let fields: Vec<_> = struct_def.fields().collect();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name().unwrap().text().unwrap(), "x");
    assert_eq!(
        fields[0].type_annotation().unwrap().type_name().unwrap().text(),
        "Float"
    );
    assert_eq!(fields[1].name().unwrap().text().unwrap(), "y");
}

#[test]
fn ast_source_file_items() {
    let source = "\
fn foo() do 1 end
fn bar() do 2 end";
    let p = parse(source);
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let tree = p.tree();

    let fns: Vec<_> = tree.fn_defs().collect();
    assert_eq!(fns.len(), 2);
    assert_eq!(fns[0].name().unwrap().text().unwrap(), "foo");
    assert_eq!(fns[1].name().unwrap().text().unwrap(), "bar");
}

#[test]
fn ast_binary_expr_accessors() {
    let p = parse_expr("1 + 2");
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let root = p.syntax();

    let binary: BinaryExpr = root
        .children()
        .find_map(BinaryExpr::cast)
        .expect("should have binary expr");

    // LHS
    let lhs = binary.lhs().expect("should have lhs");
    match lhs {
        snow_parser::ast::expr::Expr::Literal(_) => {}
        other => panic!("expected Literal lhs, got {:?}", other),
    }

    // RHS
    let rhs = binary.rhs().expect("should have rhs");
    match rhs {
        snow_parser::ast::expr::Expr::Literal(_) => {}
        other => panic!("expected Literal rhs, got {:?}", other),
    }

    // Operator
    let op = binary.op().expect("should have op token");
    assert_eq!(op.text(), "+");
}

#[test]
fn ast_literal_token() {
    let p = parse_expr("42");
    assert!(p.ok());
    let root = p.syntax();

    let lit: Literal = root
        .children()
        .find_map(Literal::cast)
        .expect("should have literal");

    let token = lit.token().expect("should have token");
    assert_eq!(token.text(), "42");
}

#[test]
fn ast_import_accessors() {
    let p = parse("from Foo.Bar import baz, qux");
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let tree = p.tree();

    let from_import: snow_parser::ast::item::FromImportDecl = tree
        .syntax()
        .children()
        .find_map(snow_parser::ast::item::FromImportDecl::cast)
        .expect("should have from import");

    let path = from_import.module_path().expect("should have module path");
    assert_eq!(path.segments(), vec!["Foo", "Bar"]);

    let import_list = from_import.import_list().expect("should have import list");
    let names: Vec<_> = import_list
        .names()
        .map(|n| n.text().unwrap())
        .collect();
    assert_eq!(names, vec!["baz", "qux"]);
}

#[test]
fn ast_module_accessors() {
    let source = "\
pub module Math do
  pub fn add(x, y) do
    x + y
  end
end";
    let p = parse(source);
    assert!(p.ok(), "parse errors: {:?}", p.errors());
    let tree = p.tree();

    let module: snow_parser::ast::item::ModuleDef = tree
        .modules()
        .next()
        .expect("should have module");

    assert!(module.visibility().is_some());
    assert_eq!(module.name().unwrap().text().unwrap(), "Math");

    let items: Vec<_> = module.items().collect();
    assert_eq!(items.len(), 1);
    match &items[0] {
        snow_parser::ast::item::Item::FnDef(_) => {}
        other => panic!("expected FnDef, got {:?}", other),
    }
}

#[test]
fn ast_parse_tree_convenience() {
    let p = parse("fn hello() do 42 end");
    assert!(p.ok());
    let tree: SourceFile = p.tree();
    let fn_def = tree.fn_defs().next().unwrap();
    assert_eq!(fn_def.name().unwrap().text().unwrap(), "hello");
}

// ── Error Message Quality Tests ─────────────────────────────────────

#[test]
fn error_fn_missing_end_references_do_span() {
    // fn foo() do 1 (missing end) -> error should reference the do span
    let p = parse("fn foo() do\n  1\n");
    assert!(!p.ok());
    let err = &p.errors()[0];
    assert!(
        err.message.contains("end"),
        "error should mention `end`: {}",
        err.message
    );
    assert!(
        err.related.is_some(),
        "error should have related span referencing `do`"
    );
    let (related_msg, _) = err.related.as_ref().unwrap();
    assert!(
        related_msg.contains("do"),
        "related message should mention `do`: {}",
        related_msg
    );
}

#[test]
fn error_glob_import_message() {
    // from Math import * -> error about glob imports
    let p = parse("from Math import *");
    assert!(!p.ok());
    let err = &p.errors()[0];
    assert!(
        err.message.contains("glob"),
        "error should mention glob: {}",
        err.message
    );
}

#[test]
fn error_let_missing_identifier_message() {
    // let = 5 -> error about missing identifier
    let p = parse("let = 5");
    assert!(!p.ok());
    let err = &p.errors()[0];
    assert!(
        err.message.contains("identifier") || err.message.contains("pattern"),
        "error should mention identifier or pattern: {}",
        err.message
    );
}

#[test]
fn error_if_missing_do() {
    // if true 1 end -> error about missing do
    let p = parse_expr("if true 1 end");
    assert!(!p.ok());
    let err = &p.errors()[0];
    assert!(
        err.message.contains("DO_KW") || err.message.contains("do") || err.message.contains("expected"),
        "error should mention do: {}",
        err.message
    );
}

#[test]
fn error_struct_missing_name() {
    let p = parse("struct do end");
    assert!(!p.ok());
    let err = &p.errors()[0];
    assert!(
        err.message.contains("name") || err.message.contains("struct"),
        "error should mention struct name: {}",
        err.message
    );
}

#[test]
fn error_module_missing_end() {
    let p = parse("module Foo do\n  fn bar() do 1 end\n");
    assert!(!p.ok());
    let err = &p.errors()[0];
    assert!(
        err.message.contains("end"),
        "error should mention `end`: {}",
        err.message
    );
}

// ── Lossless Round-Trip Tests ───────────────────────────────────────
//
// The lexer strips whitespace (by design -- whitespace tokens are not emitted).
// Newlines ARE preserved as tokens. The CST round-trip preserves all token text:
// stripping spaces from the source should match the CST text exactly.

fn strip_spaces(s: &str) -> String {
    // Remove spaces and indentation but preserve newlines and all other chars
    s.chars().filter(|c| *c != ' ').collect()
}

fn assert_lossless_roundtrip(source: &str) {
    let p = parse(source);
    let tree_text = p.syntax().text().to_string();
    let expected = strip_spaces(source);
    assert_eq!(
        tree_text, expected,
        "round-trip failed: CST text does not match source (modulo whitespace)"
    );
}

#[test]
fn lossless_simple_let() {
    assert_lossless_roundtrip("let x = 5");
}

#[test]
fn lossless_fn_def() {
    assert_lossless_roundtrip("fn add(x, y) do\n  x + y\nend");
}

#[test]
fn lossless_if_else() {
    assert_lossless_roundtrip("if true do\n  1\nelse\n  2\nend");
}

#[test]
fn lossless_case_expr() {
    assert_lossless_roundtrip("case x do\n  1 -> \"one\"\n  _ -> \"other\"\nend");
}

#[test]
fn lossless_struct_def() {
    assert_lossless_roundtrip("struct Point do\n  x :: Float\n  y :: Float\nend");
}

#[test]
fn lossless_import() {
    assert_lossless_roundtrip("import Foo.Bar");
}

#[test]
fn lossless_from_import() {
    assert_lossless_roundtrip("from Math import sqrt, pow");
}

#[test]
fn lossless_pipe_chain() {
    assert_lossless_roundtrip("fn main() do\n  x |> foo() |> bar()\nend");
}

#[test]
fn lossless_closure() {
    assert_lossless_roundtrip("fn (x) -> x + 1 end");
}

// ═══════════════════════════════════════════════════════════════════════
// Plan 03-01: Phase 3 Syntax (interface, impl, type alias, generics)
// ═══════════════════════════════════════════════════════════════════════

// ── Angle Bracket Generics ──────────────────────────────────────────

#[test]
fn struct_angle_bracket_generics() {
    assert_snapshot!(source_and_debug("struct Foo<T> do\n  x :: T\nend"));
}

// ── Interface Definition ────────────────────────────────────────────

#[test]
fn interface_simple() {
    assert_snapshot!(source_and_debug(
        "interface Printable do\n  fn to_string(self) -> String\nend"
    ));
}

#[test]
fn interface_with_generic() {
    assert_snapshot!(source_and_debug(
        "interface Container<T> do\n  fn get(self) -> T\n  fn set(self, value :: T)\nend"
    ));
}

// ── Impl Block ──────────────────────────────────────────────────────

#[test]
fn impl_simple() {
    assert_snapshot!(source_and_debug(
        "impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend"
    ));
}

// ── Type Alias ──────────────────────────────────────────────────────

#[test]
fn type_alias_simple() {
    assert_snapshot!(source_and_debug("type Alias = Int"));
}

#[test]
fn type_alias_generic() {
    assert_snapshot!(source_and_debug("type StringResult<T> = Result<T, String>"));
}

// ── Option Sugar ────────────────────────────────────────────────────

#[test]
fn option_sugar_in_type() {
    // Int? should produce a QUESTION token after IDENT "Int"
    assert_snapshot!(source_and_debug("fn foo(x :: Int?) do\n  x\nend"));
}

// ── Result Sugar ────────────────────────────────────────────────────

#[test]
fn result_sugar_in_type() {
    // T!E should produce BANG token between T and E types
    assert_snapshot!(source_and_debug("fn bar(x :: String!Error) do\n  x\nend"));
}

// ── Function with Where Clause ──────────────────────────────────────

#[test]
fn fn_with_where_clause() {
    assert_snapshot!(source_and_debug(
        "fn print<T>(x :: T) where T: Printable do\n  x\nend"
    ));
}

// ── Function with Generic Params ────────────────────────────────────

#[test]
fn fn_with_generic_params() {
    assert_snapshot!(source_and_debug("fn identity<T>(x :: T) -> T do\n  x\nend"));
}

#[test]
fn lossless_full_program() {
    let source = "\
module Math do
  pub fn add(x, y) do
    x + y
  end
end";
    assert_lossless_roundtrip(source);
}

// ═══════════════════════════════════════════════════════════════════════
// Plan 04-01: Sum Types, Constructor Patterns, Or/As Patterns
// ═══════════════════════════════════════════════════════════════════════

// ── Sum Type Definitions ────────────────────────────────────────────

#[test]
fn sum_type_simple() {
    assert_snapshot!(source_and_debug(
        "type Shape do\n  Circle(Float)\n  Rectangle(width :: Float, height :: Float)\n  Point\nend"
    ));
}

#[test]
fn sum_type_generic() {
    assert_snapshot!(source_and_debug(
        "type Option<T> do\n  Some(T)\n  None\nend"
    ));
}

#[test]
fn sum_type_multiple_positional() {
    assert_snapshot!(source_and_debug(
        "type Pair do\n  Pair(Int, Int)\nend"
    ));
}

// ── Constructor Patterns ────────────────────────────────────────────

#[test]
fn pattern_constructor_qualified() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  Shape.Circle(r) -> r\n  _ -> 0\nend"
    ));
}

#[test]
fn pattern_constructor_unqualified() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  Some(v) -> v\n  _ -> 0\nend"
    ));
}

#[test]
fn pattern_constructor_nested() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  Some(Circle(r)) -> r\n  _ -> 0\nend"
    ));
}

#[test]
fn pattern_constructor_qualified_no_args() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  Shape.Point -> 1\n  _ -> 0\nend"
    ));
}

// ── Or Patterns ─────────────────────────────────────────────────────

#[test]
fn pattern_or_simple() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  Circle(_) | Point -> 1\n  _ -> 0\nend"
    ));
}

#[test]
fn pattern_or_triple() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  1 | 2 | 3 -> true\n  _ -> false\nend"
    ));
}

// ── As Patterns ─────────────────────────────────────────────────────

#[test]
fn pattern_as_simple() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  Circle(r) as c -> c\n  _ -> x\nend"
    ));
}

// ── Combined Patterns ───────────────────────────────────────────────

#[test]
fn pattern_or_with_constructors() {
    assert_snapshot!(parse_and_debug(
        "case x do\n  Some(1) | Some(2) -> true\n  _ -> false\nend"
    ));
}

// ── Lossless Round-Trip for Sum Types ───────────────────────────────

#[test]
fn lossless_sum_type() {
    assert_lossless_roundtrip("type Shape do\n  Circle(Float)\n  Point\nend");
}
