//! Snapshot tests for Snow type error diagnostics.
//!
//! Each test triggers a specific type error, renders it through the ariadne
//! diagnostic pipeline, and snapshots the output with insta. These verify
//! that error messages are terse, include dual-span labels, and show fix
//! suggestions when plausible.

use snow_typeck::diagnostics::render_diagnostic;
use snow_typeck::TypeckResult;

// ── Helpers ────────────────────────────────────────────────────────────

/// Parse Snow source and run the type checker.
fn check_source(src: &str) -> TypeckResult {
    let parse = snow_parser::parse(src);
    snow_typeck::check(&parse)
}

/// Render the first error from a type check result as a diagnostic string.
fn render_first_error(src: &str) -> String {
    let result = check_source(src);
    assert!(
        !result.errors.is_empty(),
        "expected at least one error for source: {:?}",
        src
    );
    render_diagnostic(&result.errors[0], src, "test.snow")
}

/// Render all errors from a type check result as diagnostic strings.
fn render_all_errors(src: &str) -> Vec<String> {
    let result = check_source(src);
    result.render_errors(src, "test.snow")
}

// ── Diagnostic Snapshot Tests ──────────────────────────────────────────

/// Type mismatch: annotation says Int but expression is a String.
#[test]
fn test_diag_type_mismatch() {
    let src = "let x :: Int = \"hello\"";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Type mismatch in if branches: then branch Int, else branch String.
#[test]
fn test_diag_if_branch_mismatch() {
    let src = "if true do 1 else \"hello\" end";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Arity mismatch: function expects 2 args, called with 1.
#[test]
fn test_diag_arity_mismatch() {
    let src = "let f = fn (x, y) -> x end\nf(1)";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Unbound variable: name not defined.
#[test]
fn test_diag_unbound_variable() {
    let src = "x + 1";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Not a function: trying to call an Int.
#[test]
fn test_diag_not_a_function() {
    let src = "let x = 42\nx(1)";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Missing field in struct literal.
#[test]
fn test_diag_missing_field() {
    let src = "struct Point do\n  x :: Int\n  y :: Int\nend\nPoint { x: 1 }";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Unknown field in struct literal.
#[test]
fn test_diag_unknown_field() {
    let src = "struct Point do\n  x :: Int\n  y :: Int\nend\nPoint { x: 1, z: 2 }";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Trait not satisfied: String used with +.
#[test]
fn test_diag_trait_not_satisfied() {
    let src = "\"hello\" + \"world\"";
    let output = render_first_error(src);
    insta::assert_snapshot!(output);
}

/// Render errors method returns Vec of rendered diagnostics.
#[test]
fn test_render_errors_multiple() {
    let src = "x + y";
    let errors = render_all_errors(src);
    // Should have at least one unbound variable error
    assert!(
        !errors.is_empty(),
        "expected at least one rendered error"
    );
    // Each rendered error should contain "Error" header
    for e in &errors {
        assert!(
            e.contains("Error"),
            "rendered error should contain 'Error': {}",
            e
        );
    }
}

/// Error code appears in diagnostic output.
#[test]
fn test_error_codes_present() {
    // Type mismatch -> E0001
    let src = "let x :: Int = \"hello\"";
    let output = render_first_error(src);
    assert!(
        output.contains("E0001"),
        "expected E0001 in mismatch diagnostic: {}",
        output
    );

    // Unbound variable -> E0004
    let src2 = "x + 1";
    let output2 = render_first_error(src2);
    assert!(
        output2.contains("E0004"),
        "expected E0004 in unbound var diagnostic: {}",
        output2
    );
}
