//! Integration tests for Mesh trait system: interface definitions, impl blocks,
//! where clause constraints, compiler-known operator traits, and trait method dispatch.

use mesh_typeck::error::TypeError;
use mesh_typeck::ty::Ty;
use mesh_typeck::TypeckResult;

// ── Helpers ────────────────────────────────────────────────────────────

/// Parse Mesh source and run the type checker.
fn check_source(src: &str) -> TypeckResult {
    let parse = mesh_parser::parse(src);
    mesh_typeck::check(&parse)
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
fn assert_has_error<F: Fn(&TypeError) -> bool>(result: &TypeckResult, pred: F, desc: &str) {
    assert!(
        result.errors.iter().any(|e| pred(e)),
        "expected error matching `{}`, got errors: {:?}",
        desc,
        result.errors
    );
}

// ── Interface definition and impl ────────────────────────────────────

/// 1. Parse `interface Printable do fn to_string(self) -> String end`.
///    Check it registers without errors.
#[test]
fn test_interface_definition() {
    let result = check_source("interface Printable do\n  fn to_string(self) -> String\nend");
    assert!(
        result.errors.is_empty(),
        "interface definition should register without errors, got: {:?}",
        result.errors
    );
}

/// 2. Parse interface + impl for Int. Check it type-checks without errors.
#[test]
fn test_impl_block() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend",
    );
    assert!(
        result.errors.is_empty(),
        "impl block should type-check without errors, got: {:?}",
        result.errors
    );
}

/// 3. Impl block missing a required method. Check for error.
#[test]
fn test_impl_missing_method() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\nend",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::MissingTraitMethod { .. }),
        "MissingTraitMethod",
    );
}

/// 4. Impl with wrong return type. Check for mismatch.
#[test]
fn test_impl_wrong_method_signature() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> Int do\n    42\n  end\nend",
    );
    assert_has_error(
        &result,
        |e| {
            matches!(
                e,
                TypeError::TraitMethodSignatureMismatch { .. } | TypeError::Mismatch { .. }
            )
        },
        "TraitMethodSignatureMismatch or Mismatch",
    );
}

// ── Where clauses ────────────────────────────────────────────────────

/// 5. Function with where clause called with satisfying type.
///    Note: Mesh uses `::` for type annotations in params (not `:`).
#[test]
fn test_where_clause_satisfied() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend\n\
         fn show<T>(x :: T) -> String where T: Printable do\n  to_string(x)\nend\n\
         show(42)",
    );
    assert_result_type(&result, Ty::string());
}

/// 6. Call with type lacking required impl.
#[test]
fn test_where_clause_unsatisfied() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         fn show<T>(x :: T) -> String where T: Printable do\n  to_string(x)\nend\n\
         show(true)",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::TraitNotSatisfied { .. }),
        "TraitNotSatisfied",
    );
}

/// 7. Multiple constraints on same type param.
#[test]
fn test_multiple_where_constraints() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         interface Debuggable do\n  fn debug(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend\n\
         impl Debuggable for Int do\n  fn debug(self) -> String do\n    \"dbg:int\"\n  end\nend\n\
         fn show_debug<T>(x :: T) -> String where T: Printable, T: Debuggable do\n  to_string(x)\nend\n\
         show_debug(42)",
    );
    assert_result_type(&result, Ty::string());
}

// ── Compiler-known traits for operators ──────────────────────────────

/// 8. `1 + 2` -> Int (via Add trait for Int).
#[test]
fn test_add_trait_int() {
    let result = check_source("1 + 2");
    assert_result_type(&result, Ty::int());
}

/// 9. `1.0 + 2.0` -> Float (via Add trait for Float).
#[test]
fn test_add_trait_float() {
    let result = check_source("1.0 + 2.0");
    assert_result_type(&result, Ty::float());
}

/// 10. `"a" + "b"` fails (no Add for String).
#[test]
fn test_add_trait_string_fails() {
    let result = check_source("\"a\" + \"b\"");
    assert_has_error(
        &result,
        |e| {
            matches!(
                e,
                TypeError::TraitNotSatisfied { .. } | TypeError::Mismatch { .. }
            )
        },
        "TraitNotSatisfied or Mismatch (no Add for String)",
    );
}

/// 11. `1 == 2` -> Bool (via Eq trait).
#[test]
fn test_eq_trait() {
    let result = check_source("1 == 2");
    assert_result_type(&result, Ty::bool());
}

/// 12. `1 < 2` -> Bool (via Ord trait).
#[test]
fn test_ord_trait() {
    let result = check_source("1 < 2");
    assert_result_type(&result, Ty::bool());
}

// ── Trait method dispatch ────────────────────────────────────────────

/// 13. Call trait method on concrete type with registered impl.
#[test]
fn test_trait_method_call() {
    let interface = "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend\n";
    // Display and Printable both give Int a `to_string`: a bare call, like
    // `42.to_string()`, is ambiguous, and naming the interface resolves it.
    let result = check_source(&format!("{interface}to_string(42)"));
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::AmbiguousMethod { .. }),
        "AmbiguousMethod",
    );
    let result = check_source(&format!("{interface}Printable.to_string(42)"));
    assert_result_type(&result, Ty::string());
}

/// In an impl, `-> Self` is the implementing type: it was left as `Self`,
/// so the body (`Size { ... }`) mismatched and callers saw a type `Self`
/// with no fields.
#[test]
fn impl_method_self_is_the_implementing_type() {
    let result = check_source(
        "interface Growable do\n  fn grow(self) -> Self\nend\n\nstruct Size do\n  n :: Int\nend\n\nimpl Growable for Size do\n  fn grow(self) -> Self do\n    Size { n: self.n + 1 }\n  end\nend\n\nfn main() do\n  let s = Size { n: 1 }\n  s.grow().n\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// `Iter<T>` is the pipeline type `Iter.*` returns; naming it in an
/// annotation was E0069 "unknown type".
#[test]
fn iter_is_a_known_type_in_annotations() {
    let result = check_source(
        "fn evens(xs :: List<Int>) -> Iter<Int> do\n  Iter.filter(Iter.from(xs), fn x -> x % 2 == 0 end)\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// `describe`, `test`, `setup` and `teardown` are test-DSL builtins in every
/// file; a bare call of an interface method with one of those names reached
/// the builtin ("expected 2 argument(s), found 1").
#[test]
fn interface_methods_may_share_a_test_dsl_name() {
    let result = check_source(
        "interface Describe do\n  fn describe(self) -> String\nend\n\nstruct P do\n  x :: Int\nend\n\nimpl Describe for P do\n  fn describe(self) -> String do\n    \"p\"\n  end\nend\n\nfn main() do\n  let p = P { x: 1 }\n  describe(p)\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}
