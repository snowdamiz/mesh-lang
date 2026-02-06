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

// Case match with constructor patterns is tested after Task 2
// (constructor pattern inference). See test_sum_type_case_match below.

// ── Helper for generic non-builtin types ──────────────────────────────

trait TyExt {
    fn option_like(name: &str, inner: Ty) -> Ty;
}

impl TyExt for Ty {
    fn option_like(name: &str, inner: Ty) -> Ty {
        Ty::App(Box::new(Ty::Con(TyCon::new(name))), vec![inner])
    }
}
