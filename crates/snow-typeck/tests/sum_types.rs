//! Integration tests for sum type (ADT) type checking.
//!
//! These tests exercise:
//! - Sum type definitions and variant constructor registration
//! - Qualified variant construction (Shape.Circle(5.0))
//! - Generic sum types (Option<T>, custom generics)
//! - Nullary constructors (Shape.Point)
//! - Constructor patterns in match arms
//! - Or-patterns and as-patterns
//! - Error cases: unknown variants, binding mismatches

use snow_typeck::error::TypeError;
use snow_typeck::ty::{Ty, TyCon};
use snow_typeck::TypeckResult;

// ── Helpers ────────────────────────────────────────────────────────────

/// Parse Snow source and run the type checker.
fn check_source(src: &str) -> TypeckResult {
    let parse = snow_parser::parse(src);
    snow_typeck::check(&parse)
}

/// Assert that the result has no errors and the final expression type
/// matches the expected type.
fn assert_result_type(result: &TypeckResult, expected: Ty) {
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
    let actual = result
        .result_type
        .as_ref()
        .expect("expected a result type from inference");
    let actual_str = format!("{}", actual);
    let expected_str = format!("{}", expected);
    assert_eq!(
        actual_str, expected_str,
        "expected type `{}`, got `{}`",
        expected_str, actual_str
    );
}

/// Assert that the result contains an error matching the given predicate.
#[allow(dead_code)]
fn assert_has_error<F: Fn(&TypeError) -> bool>(result: &TypeckResult, pred: F, desc: &str) {
    assert!(
        result.errors.iter().any(|e| pred(e)),
        "expected error matching `{}`, got errors: {:?}",
        desc,
        result.errors
    );
}

// ── Sum Type Registration ────────────────────────────────────────────

/// Test 1: Simple sum type definition registers and nullary constructor
/// resolves to the sum type.
#[test]
fn test_sum_type_nullary_constructor() {
    let result = check_source(
        "type Color do\n  Red\n  Green\n  Blue\nend\nColor.Red",
    );
    assert_result_type(
        &result,
        Ty::App(Box::new(Ty::Con(TyCon::new("Color"))), vec![]),
    );
}

/// Test 2: Sum type with positional field constructor.
/// Shape.Circle(5.0) should type-check to Shape.
#[test]
fn test_sum_type_positional_constructor() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\n  Point\nend\nShape.Circle(5.0)",
    );
    assert_result_type(
        &result,
        Ty::App(Box::new(Ty::Con(TyCon::new("Shape"))), vec![]),
    );
}

/// Test 3: Generic sum type -- Option.Some(42) should infer Option<Int>.
#[test]
fn test_generic_sum_type_constructor() {
    let result = check_source(
        "type MyOption<T> do\n  MySome(T)\n  MyNone\nend\nMyOption.MySome(42)",
    );
    assert_result_type(&result, Ty::option_like("MyOption", Ty::int()));
}

/// Test 4: Nullary generic constructor -- MyOption.MyNone with annotation.
#[test]
fn test_generic_sum_type_nullary_with_annotation() {
    let result = check_source(
        "type MyOption<T> do\n  MySome(T)\n  MyNone\nend\nlet x :: MyOption<Int> = MyOption.MyNone\nx",
    );
    assert_result_type(&result, Ty::option_like("MyOption", Ty::int()));
}

/// Test 5: Unqualified constructor access (e.g. Some(42) from builtins still works).
#[test]
fn test_builtin_option_still_works() {
    let result = check_source("Some(42)");
    assert_result_type(&result, Ty::option(Ty::int()));
}

/// Test 6: Wrong argument type to variant constructor produces error.
#[test]
fn test_sum_type_wrong_arg_type() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\nend\nShape.Circle(\"bad\")",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch (wrong constructor arg type)",
    );
}

/// Test 7: Multiple field constructor.
#[test]
fn test_sum_type_multiple_fields() {
    let result = check_source(
        "type Shape do\n  Rect(Float, Float)\n  Point\nend\nShape.Rect(3.0, 4.0)",
    );
    assert_result_type(
        &result,
        Ty::App(Box::new(Ty::Con(TyCon::new("Shape"))), vec![]),
    );
}

// ── Constructor Pattern Tests ─────────────────────────────────────────

/// Test 8: Case match on sum type with constructor patterns.
/// The constructor pattern `Circle(r)` binds `r` to the Float field.
#[test]
fn test_sum_type_case_match() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\n  Point\nend\n\
         let s = Shape.Circle(5.0)\n\
         case s do\n  Circle(r) -> r\n  Point -> 0.0\nend",
    );
    assert_result_type(&result, Ty::float());
}

/// Test 9: Constructor pattern with qualified name in case arm.
#[test]
fn test_sum_type_qualified_constructor_pattern() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\n  Point\nend\n\
         let s = Shape.Circle(5.0)\n\
         case s do\n  Shape.Circle(r) -> r\n  Shape.Point -> 0.0\nend",
    );
    assert_result_type(&result, Ty::float());
}

/// Test 10: Nested constructor patterns.
#[test]
fn test_nested_constructor_pattern() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\nend\n\
         let s = Some(Shape.Circle(3.14))\n\
         case s do\n  Some(Circle(r)) -> r\n  None -> 0.0\nend",
    );
    assert_result_type(&result, Ty::float());
}

/// Test 11: Constructor pattern with wildcard sub-pattern.
#[test]
fn test_constructor_pattern_with_wildcard() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\n  Point\nend\n\
         let s = Shape.Circle(5.0)\n\
         case s do\n  Circle(_) -> 1\n  Point -> 2\nend",
    );
    assert_result_type(&result, Ty::int());
}

/// Test 12: Unknown variant in constructor pattern produces error.
#[test]
fn test_unknown_variant_pattern() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\nend\n\
         let s = Shape.Circle(5.0)\n\
         case s do\n  Triangle(a) -> a\nend",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::UnknownVariant { .. }),
        "UnknownVariant",
    );
}

// ── As Pattern Tests ─────────────────────────────────────────────────

/// Test 13: As-pattern binds the whole matched value.
#[test]
fn test_as_pattern_binds_whole_value() {
    let result = check_source(
        "type Shape do\n  Circle(Float)\n  Point\nend\n\
         let s = Shape.Circle(5.0)\n\
         case s do\n  Circle(r) as shape -> r\n  Point -> 0.0\nend",
    );
    assert_result_type(&result, Ty::float());
}

/// Test 14: As-pattern with literal.
#[test]
fn test_as_pattern_with_literal() {
    let result = check_source(
        "let x = 42\n\
         case x do\n  n as val -> val\nend",
    );
    assert_result_type(&result, Ty::int());
}

// ── Or Pattern Tests ─────────────────────────────────────────────────

/// Test 15: Or-pattern with nullary constructors.
#[test]
fn test_or_pattern_nullary() {
    let result = check_source(
        "type Color do\n  Red\n  Green\n  Blue\nend\n\
         let c = Color.Red\n\
         case c do\n  Red | Green -> 1\n  Blue -> 2\nend",
    );
    assert_result_type(&result, Ty::int());
}

/// Test 16: Or-pattern with literal alternatives.
#[test]
fn test_or_pattern_literals() {
    let result = check_source(
        "let x = 1\n\
         case x do\n  1 | 2 | 3 -> true\n  _ -> false\nend",
    );
    assert_result_type(&result, Ty::bool());
}

// ── Helper for generic non-builtin types ──────────────────────────────

trait TyExt {
    fn option_like(name: &str, inner: Ty) -> Ty;
}

impl TyExt for Ty {
    fn option_like(name: &str, inner: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new(name))), vec![inner])
    }
}
