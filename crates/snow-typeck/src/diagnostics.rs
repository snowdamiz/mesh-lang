//! Ariadne-based diagnostic rendering for type errors.
//!
//! Renders `TypeError` variants into formatted, labeled error messages
//! using the ariadne library. Output is terse (Go-minimal tone) with
//! dual-span labels showing expected vs found and fix suggestions when
//! a plausible fix exists (Elm-level thoroughness).

use std::ops::Range;

use ariadne::{Color, Config, Label, Report, ReportKind, Source};

use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::Ty;

// ── Error Codes ────────────────────────────────────────────────────────

/// Assign a unique error code to each TypeError variant.
fn error_code(err: &TypeError) -> &'static str {
    match err {
        TypeError::Mismatch { .. } => "E0001",
        TypeError::InfiniteType { .. } => "E0002",
        TypeError::ArityMismatch { .. } => "E0003",
        TypeError::UnboundVariable { .. } => "E0004",
        TypeError::NotAFunction { .. } => "E0005",
        TypeError::TraitNotSatisfied { .. } => "E0006",
        TypeError::MissingTraitMethod { .. } => "E0007",
        TypeError::TraitMethodSignatureMismatch { .. } => "E0008",
        TypeError::MissingField { .. } | TypeError::UnknownField { .. } | TypeError::NoSuchField { .. } => "E0009",
        TypeError::UnknownVariant { .. } => "E0010",
        TypeError::OrPatternBindingMismatch { .. } => "E0011",
        TypeError::NonExhaustiveMatch { .. } => "E0012",
        TypeError::RedundantArm { .. } => "W0001",
        TypeError::InvalidGuardExpression { .. } => "E0013",
        TypeError::SendTypeMismatch { .. } => "E0014",
        TypeError::SelfOutsideActor { .. } => "E0015",
        TypeError::SpawnNonFunction { .. } => "E0016",
        TypeError::ReceiveOutsideActor { .. } => "E0017",
        TypeError::InvalidChildStart { .. } => "E0018",
        TypeError::InvalidStrategy { .. } => "E0019",
        TypeError::InvalidRestartType { .. } => "E0020",
        TypeError::InvalidShutdownValue { .. } => "E0021",
    }
}

// ── Span Helpers ───────────────────────────────────────────────────────

/// Convert a rowan TextRange to a Rust Range<usize> for ariadne.
fn text_range_to_range(range: rowan::TextRange) -> Range<usize> {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    start..end
}

/// Extract a primary span from a ConstraintOrigin.
fn origin_span(origin: &ConstraintOrigin) -> Option<Range<usize>> {
    match origin {
        ConstraintOrigin::FnArg { call_site, .. } => Some(text_range_to_range(*call_site)),
        ConstraintOrigin::BinOp { op_span } => Some(text_range_to_range(*op_span)),
        ConstraintOrigin::IfBranches { if_span, .. } => Some(text_range_to_range(*if_span)),
        ConstraintOrigin::Annotation { annotation_span } => {
            Some(text_range_to_range(*annotation_span))
        }
        ConstraintOrigin::Return { return_span, .. } => Some(text_range_to_range(*return_span)),
        ConstraintOrigin::LetBinding { binding_span } => {
            Some(text_range_to_range(*binding_span))
        }
        ConstraintOrigin::Assignment { lhs_span, .. } => Some(text_range_to_range(*lhs_span)),
        ConstraintOrigin::Builtin => None,
    }
}

// ── Fix Suggestions ────────────────────────────────────────────────────

/// Generate a fix suggestion based on expected and found types.
fn fix_suggestion(expected: &Ty, found: &Ty) -> Option<String> {
    let exp_str = format!("{}", expected);
    let found_str = format!("{}", found);

    // Option<T> expected, T found -> "wrap in Some(...)"
    if exp_str.starts_with("Option<") {
        let inner = &exp_str[7..exp_str.len() - 1];
        if found_str == inner {
            return Some("wrap in Some(...)".to_string());
        }
    }

    // Result<T, E> expected, T found -> "wrap in Ok(...)"
    if exp_str.starts_with("Result<") {
        // Extract first type arg
        if let Some(comma_pos) = exp_str.find(',') {
            let inner = &exp_str[7..comma_pos];
            if found_str == inner.trim() {
                return Some("wrap in Ok(...)".to_string());
            }
        }
    }

    // Int expected, Float found -> "use Int conversion"
    if exp_str == "Int" && found_str == "Float" {
        return Some("use Int conversion".to_string());
    }

    // Float expected, Int found -> "use Float conversion"
    if exp_str == "Float" && found_str == "Int" {
        return Some("use Float conversion".to_string());
    }

    // String expected, Int found -> "use to_string()"
    if exp_str == "String" && found_str == "Int" {
        return Some("use to_string()".to_string());
    }

    // String expected, Float found -> "use to_string()"
    if exp_str == "String" && found_str == "Float" {
        return Some("use to_string()".to_string());
    }

    // Bool expected, non-Bool found -> "use a boolean expression"
    if exp_str == "Bool" && found_str != "Bool" {
        return Some("expected a boolean expression".to_string());
    }

    None
}

// ── Main Rendering Function ────────────────────────────────────────────

/// Render a type error into a formatted diagnostic string using ariadne.
///
/// The output is colorless for consistent test snapshots. Each diagnostic
/// includes an error code, terse message, labeled source spans, and a
/// fix suggestion when applicable.
pub fn render_diagnostic(error: &TypeError, source: &str, _filename: &str) -> String {
    let config = Config::default().with_color(false);
    let source_len = source.len();

    // Clamp a range to be valid within source bounds.
    let clamp = |r: Range<usize>| -> Range<usize> {
        let s = r.start.min(source_len);
        let e = r.end.min(source_len).max(s);
        // Ensure non-empty span for ariadne (it needs at least 1-char span).
        if s == e {
            s..e.saturating_add(1).min(source_len)
        } else {
            s..e
        }
    };

    let code = error_code(error);

    let report = match error {
        TypeError::Mismatch {
            expected,
            found,
            origin,
        } => {
            let msg = format!("expected {}, found {}", expected, found);
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            let mut builder = Report::build(ReportKind::Error, span.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config);

            // Add dual-span labels based on origin.
            match origin {
                ConstraintOrigin::IfBranches {
                    then_span,
                    else_span,
                    ..
                } => {
                    let then_range = clamp(text_range_to_range(*then_span));
                    let else_range = clamp(text_range_to_range(*else_span));
                    builder.add_label(
                        Label::new(then_range)
                            .with_message(format!("expected {}", expected))
                            .with_color(Color::Red),
                    );
                    builder.add_label(
                        Label::new(else_range)
                            .with_message(format!("found {}", found))
                            .with_color(Color::Blue),
                    );
                }
                ConstraintOrigin::Annotation { annotation_span } => {
                    let ann_range = clamp(text_range_to_range(*annotation_span));
                    builder.add_label(
                        Label::new(ann_range)
                            .with_message(format!("expected {} from annotation", expected))
                            .with_color(Color::Red),
                    );
                }
                _ => {
                    builder.add_label(
                        Label::new(span.clone())
                            .with_message(format!("expected {}, found {}", expected, found))
                            .with_color(Color::Red),
                    );
                }
            }

            // Add fix suggestion.
            if let Some(fix) = fix_suggestion(expected, found) {
                builder.set_help(fix);
            }

            builder.finish()
        }

        TypeError::InfiniteType { var, ty, origin } => {
            let msg = format!("infinite type: ?{} occurs in {}", var.0, ty);
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            Report::build(ReportKind::Error, span.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(span)
                        .with_message("recursive type here")
                        .with_color(Color::Red),
                )
                .with_help("a value cannot have a type that refers to itself")
                .finish()
        }

        TypeError::ArityMismatch {
            expected,
            found,
            origin,
        } => {
            let msg = format!("expected {} argument(s), found {}", expected, found);
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            let mut builder = Report::build(ReportKind::Error, span.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(span)
                        .with_message(format!("expected {} argument(s)", expected))
                        .with_color(Color::Red),
                );

            if *expected > *found {
                builder.set_help(format!("missing {} argument(s)", expected - found));
            } else {
                builder.set_help(format!("{} extra argument(s)", found - expected));
            }

            builder.finish()
        }

        TypeError::UnboundVariable { name, span } => {
            let msg = format!("undefined variable: {}", name);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("not found in this scope")
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::NotAFunction { ty, span } => {
            let msg = format!("type {} is not callable", ty);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("{} is not a function", ty))
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::TraitNotSatisfied {
            ty,
            trait_name,
            origin,
        } => {
            let msg = format!("{} does not implement {}", ty, trait_name);
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            Report::build(ReportKind::Error, span.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(span)
                        .with_message(format!("{} does not satisfy {}", ty, trait_name))
                        .with_color(Color::Red),
                )
                .with_help(format!("add `impl {} for {} do ... end`", trait_name, ty))
                .finish()
        }

        TypeError::MissingTraitMethod {
            trait_name,
            method_name,
            impl_ty,
        } => {
            let msg = format!(
                "impl {} for {} is missing method {}",
                trait_name, impl_ty, method_name
            );
            let span = clamp(0..source_len.max(1).min(source_len));

            Report::build(ReportKind::Error, span.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(span)
                        .with_message(format!("missing `{}`", method_name))
                        .with_color(Color::Red),
                )
                .with_help(format!(
                    "add `fn {}(self) ... end` to the impl block",
                    method_name
                ))
                .finish()
        }

        TypeError::TraitMethodSignatureMismatch {
            trait_name,
            method_name,
            expected,
            found,
        } => {
            let msg = format!(
                "method {} in impl {} has wrong signature: expected {}, found {}",
                method_name, trait_name, expected, found
            );
            let span = clamp(0..source_len.max(1).min(source_len));

            Report::build(ReportKind::Error, span.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(span)
                        .with_message(format!(
                            "expected return type {}, found {}",
                            expected, found
                        ))
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::MissingField {
            struct_name,
            field_name,
            span,
        } => {
            let msg = format!("missing field {} in struct {}", field_name, struct_name);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("field `{}` is required", field_name))
                        .with_color(Color::Red),
                )
                .with_help(format!("add `{}: <value>`", field_name))
                .finish()
        }

        TypeError::UnknownField {
            struct_name,
            field_name,
            span,
        } => {
            let msg = format!("unknown field {} in struct {}", field_name, struct_name);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("`{}` has no field `{}`", struct_name, field_name))
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::NoSuchField {
            ty,
            field_name,
            span,
        } => {
            let msg = format!("type {} has no field {}", ty, field_name);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("no field `{}`", field_name))
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::UnknownVariant { name, span } => {
            let msg = format!("unknown variant: {}", name);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("not a known variant")
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::OrPatternBindingMismatch {
            expected_bindings,
            found_bindings,
            span,
        } => {
            let msg = format!(
                "or-pattern alternatives bind different variables: [{}] vs [{}]",
                expected_bindings.join(", "),
                found_bindings.join(", ")
            );
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("alternatives must bind the same variables")
                        .with_color(Color::Red),
                )
                .with_help("all alternatives in an or-pattern must bind the same set of variable names")
                .finish()
        }

        TypeError::NonExhaustiveMatch {
            scrutinee_type,
            missing_patterns,
            span,
        } => {
            let msg = format!(
                "non-exhaustive match on `{}`",
                scrutinee_type
            );
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("missing: {}", missing_patterns.join(", ")))
                        .with_color(Color::Red),
                )
                .with_help("add the missing patterns or a wildcard `_` arm")
                .finish()
        }

        TypeError::RedundantArm { arm_index, span } => {
            let msg = format!("redundant match arm (arm {})", arm_index + 1);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Warning, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("this arm is unreachable")
                        .with_color(Color::Yellow),
                )
                .with_help("remove this arm or reorder the match")
                .finish()
        }

        TypeError::InvalidGuardExpression { reason, span } => {
            let msg = format!("invalid guard: {}", reason);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("only comparisons, boolean ops, literals, and names allowed")
                        .with_color(Color::Red),
                )
                .with_help("guards must be simple boolean expressions")
                .finish()
        }

        TypeError::SendTypeMismatch {
            expected,
            found,
            span,
        } => {
            let msg = format!("message type mismatch: expected {}, found {}", expected, found);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("expected {}, found {}", expected, found))
                        .with_color(Color::Red),
                )
                .with_help(format!("this Pid accepts messages of type {}", expected))
                .finish()
        }

        TypeError::SelfOutsideActor { span } => {
            let msg = "self() used outside actor block";
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("self() is only available inside an actor block")
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::SpawnNonFunction { found, span } => {
            let msg = format!("cannot spawn non-function: found {}", found);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("expected a function, found {}", found))
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::ReceiveOutsideActor { span } => {
            let msg = "receive used outside actor block";
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("receive is only available inside an actor block")
                        .with_color(Color::Red),
                )
                .with_help("move this receive expression into an actor block")
                .finish()
        }

        TypeError::InvalidChildStart {
            child_name,
            found,
            span,
        } => {
            let msg = format!(
                "child `{}` start function must return Pid, found `{}`",
                child_name, found
            );
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message(format!("expected Pid<M>, found {}", found))
                        .with_color(Color::Red),
                )
                .with_help("the start function must call spawn() and return a Pid")
                .finish()
        }

        TypeError::InvalidStrategy { found, span } => {
            let msg = format!("unknown supervision strategy `{}`", found);
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("expected one_for_one, one_for_all, rest_for_one, or simple_one_for_one")
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::InvalidRestartType {
            found,
            child_name,
            span,
        } => {
            let msg = format!(
                "invalid restart type `{}` for child `{}`",
                found, child_name
            );
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("expected permanent, transient, or temporary")
                        .with_color(Color::Red),
                )
                .finish()
        }

        TypeError::InvalidShutdownValue {
            found,
            child_name,
            span,
        } => {
            let msg = format!(
                "invalid shutdown value `{}` for child `{}`",
                found, child_name
            );
            let range = clamp(text_range_to_range(*span));

            Report::build(ReportKind::Error, range.clone())
                .with_code(code)
                .with_message(&msg)
                .with_config(config)
                .with_label(
                    Label::new(range)
                        .with_message("expected a positive integer or brutal_kill")
                        .with_color(Color::Red),
                )
                .finish()
        }
    };

    // Render to buffer without colors.
    let mut buf = Vec::new();
    let cache = Source::from(source);
    report.write(cache, &mut buf).expect("failed to write diagnostic");
    String::from_utf8(buf).expect("diagnostic output should be valid UTF-8")
}
