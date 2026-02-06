//! Snapshot tests for Snow type error diagnostics.
//!
//! Each test triggers a specific type error, renders it through the ariadne
//! diagnostic pipeline, and snapshots the output with insta. These verify
//! that error messages are terse, include dual-span labels, and show fix
//! suggestions when plausible.

use snow_typeck::diagnostics::render_diagnostic;
use snow_typeck::error::TypeError;
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

// ── Phase 4 Diagnostic Tests ────────────────────────────────────────

/// Non-exhaustive match renders with missing patterns and E0012 code.
#[test]
fn test_diag_non_exhaustive_match() {
    let src = "case x do Some(v) -> v end";
    let err = TypeError::NonExhaustiveMatch {
        scrutinee_type: "Option<Int>".to_string(),
        missing_patterns: vec!["None".to_string()],
        span: rowan::TextRange::new(0.into(), 26.into()),
    };
    let output = render_diagnostic(&err, src, "test.snow");
    insta::assert_snapshot!(output);
}

/// Non-exhaustive match with multiple missing variants.
#[test]
fn test_diag_non_exhaustive_match_multiple() {
    let src = "case s do Circle(r) -> r end";
    let err = TypeError::NonExhaustiveMatch {
        scrutinee_type: "Shape".to_string(),
        missing_patterns: vec!["Rect".to_string(), "Point".to_string()],
        span: rowan::TextRange::new(0.into(), 28.into()),
    };
    let output = render_diagnostic(&err, src, "test.snow");
    assert!(output.contains("E0012"), "expected E0012 code: {}", output);
    assert!(output.contains("Rect"), "expected Rect in missing: {}", output);
    assert!(output.contains("Point"), "expected Point in missing: {}", output);
    assert!(output.contains("non-exhaustive"), "expected non-exhaustive message: {}", output);
}

/// Redundant arm renders as warning (not error) with W0001 code.
#[test]
fn test_diag_redundant_arm() {
    let src = "case x do _ -> 1\n  _ -> 2 end";
    let err = TypeError::RedundantArm {
        arm_index: 1,
        span: rowan::TextRange::new(18.into(), 24.into()),
    };
    let output = render_diagnostic(&err, src, "test.snow");
    insta::assert_snapshot!(output);
}

/// Redundant arm diagnostic uses Warning report kind, not Error.
#[test]
fn test_diag_redundant_arm_is_warning() {
    let src = "case x do _ -> 1\n  true -> 2 end";
    let err = TypeError::RedundantArm {
        arm_index: 1,
        span: rowan::TextRange::new(18.into(), 27.into()),
    };
    let output = render_diagnostic(&err, src, "test.snow");
    assert!(output.contains("Warning"), "expected Warning kind: {}", output);
    assert!(output.contains("W0001"), "expected W0001 code: {}", output);
    assert!(output.contains("unreachable"), "expected 'unreachable' label: {}", output);
}

/// Invalid guard expression renders with E0013 code.
#[test]
fn test_diag_invalid_guard_expression() {
    let src = "case x do n when f(n) -> n end";
    let err = TypeError::InvalidGuardExpression {
        reason: "function calls not allowed in guards".to_string(),
        span: rowan::TextRange::new(16.into(), 20.into()),
    };
    let output = render_diagnostic(&err, src, "test.snow");
    insta::assert_snapshot!(output);
}

/// Unknown variant diagnostic renders with E0010 code.
#[test]
fn test_diag_unknown_variant() {
    let src = "type Shape do Circle(Float) end\ncase s do Triangle(a) -> a end";
    let err = TypeError::UnknownVariant {
        name: "Triangle".to_string(),
        span: rowan::TextRange::new(42.into(), 54.into()),
    };
    let output = render_diagnostic(&err, src, "test.snow");
    assert!(output.contains("E0010"), "expected E0010 code: {}", output);
    assert!(output.contains("Triangle"), "expected 'Triangle' in output: {}", output);
}

/// Or-pattern binding mismatch diagnostic renders with E0011 code.
#[test]
fn test_diag_or_pattern_binding_mismatch() {
    let src = "case x do a | (b, c) -> a end";
    let err = TypeError::OrPatternBindingMismatch {
        expected_bindings: vec!["a".to_string()],
        found_bindings: vec!["b".to_string(), "c".to_string()],
        span: rowan::TextRange::new(10.into(), 20.into()),
    };
    let output = render_diagnostic(&err, src, "test.snow");
    assert!(output.contains("E0011"), "expected E0011 code: {}", output);
    assert!(output.contains("bind"), "expected binding-related message: {}", output);
}
