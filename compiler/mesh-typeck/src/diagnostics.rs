//! Ariadne-based diagnostic rendering for type errors.
//!
//! Renders `TypeError` variants into formatted, labeled error messages
//! using the ariadne library. Output is terse (Go-minimal tone) with
//! dual-span labels showing expected vs found and fix suggestions when
//! a plausible fix exists (Elm-level thoroughness).
//!
//! Supports two output modes:
//! - **Human-readable** (default): colorized ariadne reports with multi-span labels
//! - **JSON** (via `--json`): one JSON object per line for editor/CI integration

use std::ops::Range;

use ariadne::{Color, Config, Report, ReportKind};
use serde::Serialize;

use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::Ty;

// ── Diagnostic Options ───────────────────────────────────────────────

/// Configuration for diagnostic rendering.
///
/// Controls color output (for terminal vs test/CI contexts) and output
/// format (human-readable vs machine-readable JSON).
#[derive(Clone, Debug)]
pub struct DiagnosticOptions {
    /// Whether to use ANSI color codes in output. Default: true.
    pub color: bool,
    /// Whether to output JSON format instead of human-readable. Default: false.
    pub json: bool,
    /// Paths to name in place of others, `(from, to)`: a file, or a
    /// directory for the files under it. `meshc test` builds a copy of the
    /// project in a temporary directory, and names the user's files.
    pub display_paths: Vec<(std::path::PathBuf, std::path::PathBuf)>,
}

impl Default for DiagnosticOptions {
    fn default() -> Self {
        Self {
            color: true,
            json: false,
            display_paths: Vec::new(),
        }
    }
}

impl DiagnosticOptions {
    /// The terminal report's settings. Spans are byte offsets into the
    /// source (ariadne's default is chars, which moves every report after a
    /// non-ASCII character).
    pub fn report_config(&self) -> Config {
        Config::default()
            .with_color(self.color)
            .with_index_type(ariadne::IndexType::Byte)
    }

    /// `path` as diagnostics name it (see `display_paths`).
    pub fn display_path(&self, path: &std::path::Path) -> String {
        for (from, to) in &self.display_paths {
            if let Ok(rest) = path.strip_prefix(from) {
                // `join("")` would add a separator.
                let to = if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                };
                return to.display().to_string();
            }
        }
        path.display().to_string()
    }

    /// Create options for colorless output (used in tests for deterministic snapshots).
    pub fn colorless() -> Self {
        Self {
            color: false,
            json: false,
            display_paths: Vec::new(),
        }
    }

    /// Create options for JSON output mode.
    pub fn json_mode() -> Self {
        Self {
            color: false,
            json: true,
            display_paths: Vec::new(),
        }
    }
}

// ── JSON Diagnostic Types ────────────────────────────────────────────

/// A single source span in a JSON diagnostic.
#[derive(Clone, Debug, Serialize)]
pub struct JsonSpan {
    pub start: usize,
    pub end: usize,
    pub label: String,
}

/// A machine-readable diagnostic in JSON format.
///
/// Produced one-per-line when `--json` flag is set. Designed for editor
/// integration and CI tooling.
#[derive(Clone, Debug, Serialize)]
pub struct JsonDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub file: String,
    pub spans: Vec<JsonSpan>,
    pub fix: Option<String>,
}

// ── Error Codes ────────────────────────────────────────────────────────

/// Assign a unique error code to each TypeError variant.
fn error_code(err: &TypeError) -> &'static str {
    match err {
        TypeError::Mismatch { .. } => "E0001",
        TypeError::InfiniteType { .. } => "E0002",
        TypeError::ArityMismatch { .. } | TypeError::TupleParameterSplit { .. } => "E0003",
        TypeError::UnboundVariable { .. } => "E0004",
        TypeError::NotAFunction { .. } => "E0005",
        TypeError::TraitNotSatisfied { .. } => "E0006",
        TypeError::MissingTraitMethod { .. } => "E0007",
        TypeError::TraitMethodSignatureMismatch { .. } => "E0008",
        TypeError::MissingField { .. }
        | TypeError::UnknownField { .. }
        | TypeError::NoSuchField { .. } => "E0009",
        TypeError::NoSuchMethod { .. } => "E0030",
        TypeError::ManualContinuityPromotionDisabled { .. } => "E0046",
        TypeError::UnknownVariant { .. } => "E0010",
        TypeError::OrPatternBindingMismatch { .. } => "E0011",
        TypeError::NonExhaustiveMatch { .. } => "E0012",
        TypeError::RedundantArm { .. } => "W0001",
        TypeError::NonExhaustiveClauses { .. } => "W0003",
        TypeError::SendTypeMismatch { .. } => "E0014",
        TypeError::SelfOutsideActor { .. } => "E0015",
        TypeError::MonitorOutsideActor { .. } => "E0086",
        TypeError::SpawnNonFunction { .. } => "E0016",
        TypeError::ReceiveOutsideActor { .. } => "E0017",
        TypeError::InvalidChildStart { .. } => "E0018",
        TypeError::InvalidStrategy { .. } => "E0019",
        TypeError::InvalidRestartType { .. } => "E0020",
        TypeError::InvalidShutdownValue { .. } => "E0021",
        TypeError::CatchAllNotLast { .. } => "E0022",
        TypeError::NonConsecutiveClauses { .. } => "E0023",
        TypeError::NonFirstClauseAnnotation { .. } => "W0002",
        TypeError::DuplicateImpl { .. } => "E0026",
        TypeError::AmbiguousMethod { .. } => "E0027",
        TypeError::UnsupportedDerive { .. } => "E0028",
        TypeError::MissingDerivePrerequisite { .. } => "E0029",
        TypeError::BreakOutsideLoop { .. } => "E0032",
        TypeError::ContinueOutsideLoop { .. } => "E0033",
        TypeError::ImportModuleNotFound { .. } => "E0031",
        TypeError::ImportNameNotFound { .. } => "E0034",
        TypeError::PrivateItem { .. } => "E0035",
        TypeError::HttpClusteredInvalidArguments { .. } => "E0047",
        TypeError::HttpClusteredPrivateHandler { .. } => "E0048",
        TypeError::HttpClusteredOutsideRouteHandlerPosition { .. } => "E0049",
        TypeError::HttpClusteredConflictingReplicationCount { .. } => "E0050",
        TypeError::HttpClusteredImportedOriginMissing { .. } => "E0051",
        TypeError::TryIncompatibleReturn { .. } => "E0036",
        TypeError::TryOnNonResultOption { .. } => "E0037",
        TypeError::NonSerializableField { .. } => "E0038",
        TypeError::NonMappableField { .. } => "E0039",
        TypeError::MissingAssocType { .. } => "E0040",
        TypeError::ExtraAssocType { .. } => "E0041",
        TypeError::UnresolvedAssocType { .. } => "E0042",
        TypeError::SlotPipeOutOfRange { .. } => "E0044",
        TypeError::UndefinedType { .. } => "E0045",
        TypeError::NativeDeclarationInvalid { .. } => "E0052",
        TypeError::ExportDeclarationInvalid { .. } => "E0055",
        TypeError::ResourceViolation { .. } => "E0053",
        TypeError::InvalidLetPattern { .. } => "E0054",
        TypeError::InvalidPassThroughArm { .. } => "E0056",
        TypeError::DuplicateBinding { .. } => "E0057",
        TypeError::DuplicateField { .. } => "E0058",
        TypeError::NotAStruct { .. } => "E0059",
        TypeError::UnderivableField { .. } => "E0060",
        TypeError::DuplicateVariant { .. } => "E0061",
        TypeError::CyclicAlias { .. } => "E0062",
        TypeError::UnboundedTypeParam { .. } => "E0063",
        TypeError::AmbiguousDefault { .. } => "E0064",
        TypeError::AmbiguousImplMethod { .. } => "E0065",
        TypeError::AmbiguousStaticMethod { .. } => "E0066",
        TypeError::RigidTypeParam { .. } => "E0067",
        TypeError::DuplicateDefinition { .. } => "E0068",
        TypeError::UnknownType { .. } => "E0069",
        TypeError::UnknownFieldOwner { .. } => "E0070",
        TypeError::UnknownInterface { .. } => "E0071",
        TypeError::InvalidLiteral { .. } => "E0072",
        TypeError::InvalidConcat { .. } => "E0073",
        TypeError::NoSuchModuleFunction { .. } => "E0074",
        TypeError::OverloadedFunctionValue { .. } => "E0075",
        TypeError::GenericImplTarget { .. } => "E0076",
        TypeError::AssertReceiveOutsideTest { .. } => "E0077",
        TypeError::IndexingUnsupported { .. } => "E0078",
        TypeError::ActorMessageTypeUnknown { .. } => "E0079",
        TypeError::TopLevelLet { .. } => "E0080",
        TypeError::ModuleNotImported { .. } => "E0081",
        TypeError::UntypedMethodParam { .. } => "E0082",
        TypeError::TypeNotValue { .. } => "E0083",
        TypeError::NestedDefinition { .. } => "E0084",
        TypeError::TypeArgumentCount { .. } => "E0085",
        TypeError::TopLevelStatement { .. } => "E0087",
        TypeError::MethodReturnUnknown { .. } => "E0088",
    }
}

/// Determine severity string for JSON output.
fn severity(err: &TypeError) -> &'static str {
    match err {
        TypeError::RedundantArm { .. }
        | TypeError::NonFirstClauseAnnotation { .. }
        | TypeError::NonExhaustiveClauses { .. } => "warning",
        _ => "error",
    }
}

// ── Span Helpers ───────────────────────────────────────────────────────

/// Convert a rowan TextRange to a Rust Range<usize> for ariadne.
/// A type named with no type arguments, as a struct or `Int` is: one an
/// `impl` can be written for.
fn is_named_type(ty: &Ty) -> bool {
    match ty {
        Ty::Con(_) => true,
        Ty::App(con, args) => args.is_empty() && matches!(con.as_ref(), Ty::Con(_)),
        _ => false,
    }
}

fn text_range_to_range(range: rowan::TextRange) -> Range<usize> {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    start..end
}

/// Extract a primary span from a ConstraintOrigin.
fn origin_span(origin: &ConstraintOrigin) -> Option<Range<usize>> {
    origin.span().map(text_range_to_range)
}

// ── Fix Suggestions ────────────────────────────────────────────────────

/// How to turn a value of type `found` into the `expected` one, both as a
/// diagnostic shows them (`Ty::with_holes`).
fn fix_suggestion(expected: &Ty, found: &Ty) -> Option<&'static str> {
    let first_arg_of = |name: &str| match expected {
        Ty::App(con, args) if matches!(con.as_ref(), Ty::Con(c) if c.name == name) => args.first(),
        _ => None,
    };
    if first_arg_of("Option").is_some_and(|inner| could_be(inner, found)) {
        return Some("wrap in Some(...)");
    }
    if first_arg_of("Result").is_some_and(|ok| could_be(ok, found)) {
        return Some("wrap in Ok(...)");
    }
    match (expected.to_string().as_str(), found.to_string().as_str()) {
        ("Int", "Float") => Some("convert it with `Float.to_int(...)`"),
        ("Float", "Int") => Some("convert it with `Int.to_float(...)`"),
        ("String", "Int" | "Float") => Some("use to_string()"),
        // An argument converts on its own (`json_as_text`); nothing else does.
        ("String", "Json") => Some("encode it: `Json.encode(...)` is its JSON text"),
        ("Json", "String") => Some("parse it: `Json.parse(...)` reads JSON text into a Json"),
        ("Bool", _) => Some("expected a boolean expression"),
        _ => None,
    }
}

/// Whether `a` and `b`, as a diagnostic shows them, could be one type: the
/// same where neither has a hole (`_`, a part inference did not settle).
fn could_be(a: &Ty, b: &Ty) -> bool {
    let hole = |ty: &Ty| matches!(ty, Ty::Con(c) if c.name == "_");
    match (a, b) {
        _ if hole(a) || hole(b) => true,
        (Ty::App(..), Ty::App(..)) | (Ty::Fun(..), Ty::Fun(..)) | (Ty::Tuple(_), Ty::Tuple(_)) => {
            a.parts().count() == b.parts().count()
                && a.parts().zip(b.parts()).all(|(a, b)| could_be(a, b))
        }
        // `Point` is `Point` with no type arguments.
        _ => a.to_string() == b.to_string(),
    }
}

/// "did you mean `x`?" for an unknown `name`: the checker's `suggestion`, or
/// else the closest of `suggestions`, if one is close enough.
fn closest_name_help(
    name: &str,
    suggestion: &Option<String>,
    suggestions: Option<&[String]>,
) -> Option<String> {
    suggestion
        .clone()
        .or_else(|| find_closest_name(name, suggestions?, 2))
        .map(|closest| format!("did you mean `{closest}`?"))
}

/// Find the closest name in a list using Levenshtein distance.
pub(crate) fn find_closest_name(
    target: &str,
    candidates: &[String],
    max_distance: usize,
) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let dist = levenshtein_distance(target, candidate);
        if dist <= max_distance {
            if let Some((best_dist, _)) = best {
                if dist < best_dist {
                    best = Some((dist, candidate));
                }
            } else {
                best = Some((dist, candidate));
            }
        }
    }
    best.map(|(_, name)| name.to_string())
}

/// Compute Levenshtein edit distance between two strings.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

// ── JSON Rendering ───────────────────────────────────────────────────

/// Render a type error as a JSON diagnostic string (one line).
///
/// Produces machine-readable output for editor/CI integration.
/// `span` as a range ariadne can underline in `source`: inside the source,
/// on character boundaries (spans are byte offsets), and at least one
/// character wide. A span at the end of the file (an unclosed string)
/// covers the last character, and an empty file gives `0..0`.
pub fn report_span(source: &str, span: Range<usize>) -> Range<usize> {
    let len = source.len();
    let floor = |mut i: usize| {
        i = i.min(len);
        while !source.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let mut start = floor(span.start);
    let mut end = floor(span.end).max(start);
    if start == end {
        if start < len {
            end = (start + 1..=len)
                .find(|&i| source.is_char_boundary(i))
                .unwrap_or(len);
        } else if start > 0 {
            start = floor(start - 1);
        }
    }
    start..end
}

/// The diagnostic as one JSON line: the error's message, and the spans,
/// labels and first help the terminal report shows.
pub fn render_json_diagnostic(
    error: &TypeError,
    source: &str,
    filename: &str,
    suggestions: Option<&[String]>,
) -> String {
    let description = describe(error, source, suggestions);
    // The first span is where the error is, as the report is; it carries
    // the label there, or the report's headline.
    let span = |range: &Range<usize>, label: String| JsonSpan {
        start: range.start,
        end: range.end,
        label,
    };
    let mut labels = description.labels;
    let at_error = labels
        .iter()
        .position(|label| label.span == description.span)
        .map(|index| labels.remove(index));
    let mut spans = vec![span(
        &description.span,
        at_error
            .and_then(|label| label.message)
            .unwrap_or(description.message),
    )];
    spans.extend(
        labels
            .into_iter()
            .map(|label| span(&label.span, label.message.unwrap_or_default())),
    );
    let diag = JsonDiagnostic {
        code: error_code(error).to_string(),
        severity: severity(error).to_string(),
        message: error.to_string(),
        file: filename.to_string(),
        spans,
        fix: description.helps.into_iter().next(),
    };
    serde_json::to_string(&diag).unwrap_or_else(|_| "{}".to_string())
}

/// What a diagnostic shows, whether in a terminal report or a JSON line:
/// its kind, headline, labelled spans (the first is where the error is),
/// help and notes. Built as ariadne's reports are.
struct Description {
    kind: ReportKind<'static>,
    span: Range<usize>,
    message: String,
    labels: Vec<Label>,
    helps: Vec<String>,
    notes: Vec<String>,
}

/// A labelled span of a [`Description`].
struct Label {
    span: Range<usize>,
    message: Option<String>,
    color: Option<Color>,
}

impl Label {
    fn new(span: Range<usize>) -> Self {
        Label {
            span,
            message: None,
            color: None,
        }
    }

    fn with_message(mut self, message: impl ToString) -> Self {
        self.message = Some(message.to_string());
        self
    }

    fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl Description {
    fn build(kind: ReportKind<'static>, span: Range<usize>) -> Self {
        Description {
            kind,
            span,
            message: String::new(),
            labels: Vec::new(),
            helps: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn with_label(mut self, label: Label) -> Self {
        self.add_label(label);
        self
    }

    fn add_label(&mut self, label: Label) {
        self.labels.push(label);
    }

    /// Adds a help, as ariadne's `with_help` does.
    fn with_help(mut self, help: impl ToString) -> Self {
        self.helps.push(help.to_string());
        self
    }

    /// Replaces the helps, as ariadne's `set_help` does.
    fn set_help(&mut self, help: impl ToString) {
        self.helps = vec![help.to_string()];
    }

    fn with_note(mut self, note: impl ToString) -> Self {
        self.notes.push(note.to_string());
        self
    }

    /// An error at `span`, labelled there.
    fn error(span: Range<usize>, label: impl ToString) -> Self {
        Description::build(ReportKind::Error, span.clone())
            .with_label(Label::new(span).with_message(label).with_color(Color::Red))
    }

    /// The terminal report.
    fn report(
        self,
        fname: &str,
        code: &str,
        config: Config,
    ) -> Report<'static, (String, Range<usize>)> {
        let mut report = Report::build(self.kind, (fname.to_string(), self.span))
            .with_code(code)
            .with_message(self.message)
            .with_config(config);
        for label in self.labels {
            let mut ariadne_label = ariadne::Label::new((fname.to_string(), label.span));
            if let Some(message) = label.message {
                ariadne_label = ariadne_label.with_message(message);
            }
            if let Some(color) = label.color {
                ariadne_label = ariadne_label.with_color(color);
            }
            report.add_label(ariadne_label);
        }
        for help in self.helps {
            report.add_help(help);
        }
        for note in self.notes {
            report.add_note(note);
        }
        report.finish()
    }
}

/// What `error` shows: see [`Description`].
/// How an error is reported: its headline, the error's `Display` text,
/// which JSON diagnostics and the language server show too, and the labels,
/// helps and notes at its spans.
fn describe(error: &TypeError, source: &str, suggestions: Option<&[String]>) -> Description {
    let mut description = describe_spans(error, source, suggestions);
    description.message = error.to_string();
    description
}

/// Everything `describe` reports about `error` but its headline.
fn describe_spans(error: &TypeError, source: &str, suggestions: Option<&[String]>) -> Description {
    let source_len = source.len();
    // Spans are byte offsets into the source.
    let clamp = |r: Range<usize>| report_span(source, r);

    match error {
        TypeError::Mismatch {
            expected,
            found,
            origin,
        } => {
            let (expected, found) = (&expected.with_holes(), &found.with_holes());
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            let mut builder = Description::build(ReportKind::Error, span.clone());

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
                ConstraintOrigin::FnArg {
                    call_site,
                    param_idx,
                } => {
                    let call_range = clamp(text_range_to_range(*call_site));
                    builder.add_label(
                        Label::new(call_range)
                            .with_message(format!(
                                "argument {} has type {}, expected {}",
                                param_idx + 1,
                                found,
                                expected
                            ))
                            .with_color(Color::Red),
                    );
                }
                ConstraintOrigin::Return {
                    return_span,
                    fn_span,
                } => {
                    let ret_range = clamp(text_range_to_range(*return_span));
                    let fn_range = clamp(text_range_to_range(*fn_span));
                    builder.add_label(
                        Label::new(ret_range)
                            .with_message(format!("returns {}", found))
                            .with_color(Color::Red),
                    );
                    builder.add_label(
                        Label::new(fn_range)
                            .with_message(format!("return type declared as {}", expected))
                            .with_color(Color::Blue),
                    );
                }
                ConstraintOrigin::Assignment { lhs_span, rhs_span } => {
                    let lhs_range = clamp(text_range_to_range(*lhs_span));
                    let rhs_range = clamp(text_range_to_range(*rhs_span));
                    builder.add_label(
                        Label::new(lhs_range)
                            .with_message(format!("expected {}", expected))
                            .with_color(Color::Red),
                    );
                    builder.add_label(
                        Label::new(rhs_range)
                            .with_message(format!("found {}", found))
                            .with_color(Color::Blue),
                    );
                }
                ConstraintOrigin::Pattern { pattern_span } => {
                    let range = clamp(text_range_to_range(*pattern_span));
                    builder.add_label(
                        Label::new(range)
                            .with_message(format!(
                                "this pattern matches {found}, but the value is {expected}"
                            ))
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

            let is_pattern = matches!(origin, ConstraintOrigin::Pattern { .. });
            if let Some(fix) = fix_suggestion(expected, found).filter(|_| !is_pattern) {
                builder.set_help(fix);
            }

            builder
        }

        TypeError::InfiniteType { origin, .. } => {
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            Description::error(span, "recursive type here")
                .with_help("a value cannot have a type that refers to itself")
        }

        TypeError::ArityMismatch {
            expected,
            found,
            origin,
        } => {
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            let mut builder =
                Description::error(span, format!("expected {} argument(s)", expected));

            if *expected > *found {
                builder.set_help(format!("missing {} argument(s)", expected - found));
            } else {
                builder.set_help(format!("{} extra argument(s)", found - expected));
            }

            builder
        }

        TypeError::TupleParameterSplit { elements, origin } => {
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let names: Vec<String> = (0..*elements)
                .map(|i| ((b'a' + (i % 26) as u8) as char).to_string())
                .collect();
            Description::error(
                clamp(span),
                format!("takes {elements} arguments, not a tuple"),
            )
            .with_help(format!(
                "`fn ({}) -> ...` takes {elements} arguments; to take the tuple apart, \
                     put its pattern in parentheses: `fn (({})) -> ... end`",
                names.join(", "),
                names.join(", ")
            ))
        }

        TypeError::UnboundVariable {
            name,
            span,
            suggestion,
        } => {
            let range = clamp(text_range_to_range(*span));
            let mut builder = Description::error(range, "not found in this scope");
            if matches!(name.as_str(), "null" | "undefined") {
                builder.set_help(format!(
                    "Mesh has no `{name}`: a value that may be absent is an `Option` \
                     (`None`, or `Some(value)`)"
                ));
            } else if let Some(fix) = closest_name_help(name, suggestion, suggestions) {
                builder.set_help(fix);
            }
            builder
        }

        TypeError::NotAFunction { ty, span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, format!("{} is not a function", ty.with_holes()))
                .with_help("only functions can be called; remove the parentheses to use the value")
        }

        TypeError::TraitNotSatisfied {
            ty,
            trait_name,
            origin,
        } => {
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);

            Description::error(
                span,
                format!("{} does not satisfy {}", ty.with_holes(), trait_name),
            )
            .with_help(if trait_name == "Json" {
                "JSON holds Int, Float, Bool, String, tuples, and Option, List and \
                 Map<String, _> of them; a struct or sum type gets it with `deriving(Json)`"
                    .to_string()
            } else if is_named_type(ty)
                && matches!(
                    trait_name.as_str(),
                    "Eq" | "Ord" | "Display" | "Debug" | "Hash"
                )
            {
                format!(
                    "add `deriving({trait_name})` to the definition of `{ty}`, \
                     or `impl {trait_name} for {ty} do ... end`"
                )
            } else if trait_name == "Add" && matches!(ty.con_name(), Some("String" | "List")) {
                "join strings with `<>`, and lists with `++`".to_string()
            } else if is_named_type(ty) {
                format!("add `impl {} for {} do ... end`", trait_name, ty)
            } else {
                "only a named type without type parameters can have an `impl`".to_string()
            })
        }

        TypeError::UnboundedTypeParam {
            param,
            trait_name,
            origin,
        } => {
            let span = origin_span(origin).unwrap_or(0..source_len.max(1).min(source_len));
            let span = clamp(span);
            Description::error(span, format!("this needs {param}: {trait_name}"))
                .with_help(format!(
                    "add `where {param}: {trait_name}` to the function, so every call is checked for it"
                ))
        }

        TypeError::MissingTraitMethod {
            trait_name,
            method_name,
            span,
            ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(span, format!("missing `{}`", method_name)).with_help(format!(
                "add `fn {}` as `{}` declares it to the impl block",
                method_name, trait_name
            ))
        }

        TypeError::TraitMethodSignatureMismatch {
            trait_name,
            expected,
            found,
            span,
            ..
        } => {
            let expected = &expected.with_holes();
            let found = &found.with_holes();
            let span = clamp(text_range_to_range(*span));

            Description::error(
                span,
                format!("`{}` declares {}, this is {}", trait_name, expected, found),
            )
        }

        TypeError::MissingField {
            field_name, span, ..
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("field `{}` is required", field_name))
                .with_help(format!("add `{}: <value>`", field_name))
        }

        TypeError::UnknownField {
            struct_name,
            field_name,
            span,
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                format!("`{}` has no field `{}`", struct_name, field_name),
            )
        }

        TypeError::NoSuchField {
            field_name, span, ..
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("no field `{}`", field_name))
        }

        TypeError::NoSuchMethod {
            ty,
            method_name,
            span,
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("method `{}` not found", method_name)).with_help(
                format!(
                    "type `{}` has no trait impl providing `{}`",
                    ty, method_name
                ),
            )
        }

        TypeError::ManualContinuityPromotionDisabled { span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "manual authority changes are no longer part of the Mesh surface")
                .with_help(
                    "failover is automatic-only now; use `Continuity.authority_status()` to inspect authority state",
                )
        }

        TypeError::UnknownVariant {
            name,
            span,
            suggestion,
            cons_tail,
        } => {
            let range = clamp(text_range_to_range(*span));
            let mut builder = Description::error(range, "not a known variant");
            if *cons_tail {
                builder.set_help(format!(
                    "`::` in a pattern splits a list into its head and tail, so `{name}` was read \
                     as a variant; a pattern takes no type annotation"
                ));
            } else if let Some(fix) = closest_name_help(name, suggestion, suggestions) {
                builder.set_help(fix);
            }
            builder
        }

        TypeError::OrPatternBindingMismatch {
            list_tail, span, ..
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "alternatives must bind the same variables").with_help(
                if *list_tail {
                    "`|` in a list pattern separates alternatives; a list's tail is written \
                     `first :: rest` (`a :: b :: rest` after two elements)"
                } else {
                    "all alternatives in an or-pattern must bind the same set of variable names"
                },
            )
        }

        TypeError::NonExhaustiveMatch {
            missing_patterns,
            span,
            ..
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("missing: {}", missing_patterns.join(", ")))
                .with_help("add the missing patterns or a wildcard `_` arm")
        }

        TypeError::NonExhaustiveClauses {
            missing_patterns,
            span,
            ..
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::build(ReportKind::Warning, range.clone())
                .with_label(
                    Label::new(range)
                        .with_message(format!("missing: {}", missing_patterns.join(", ")))
                        .with_color(Color::Yellow),
                )
                .with_help("add a clause for the missing patterns: a call no clause matches panics")
        }

        TypeError::RedundantArm { span, .. } => {
            let range = clamp(text_range_to_range(*span));

            Description::build(ReportKind::Warning, range.clone())
                .with_label(
                    Label::new(range)
                        .with_message("this arm is unreachable")
                        .with_color(Color::Yellow),
                )
                .with_help("remove this arm or reorder the match")
        }

        TypeError::SendTypeMismatch {
            expected,
            found,
            span,
        } => {
            let expected = &expected.with_holes();
            let found = &found.with_holes();
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("expected {}, found {}", expected, found))
                .with_help(format!("this Pid accepts messages of type {}", expected))
        }

        TypeError::SelfOutsideActor { span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "self() is only available inside an actor block")
        }

        TypeError::MonitorOutsideActor { span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                "a monitor delivers its message to the actor that sets it up",
            )
            .with_help("call `Process.monitor` or `Node.monitor` inside an actor block")
        }

        TypeError::SpawnNonFunction { found, span } => {
            let found = &found.with_holes();
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("expected a function, found {}", found))
        }

        TypeError::ReceiveOutsideActor { span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "receive is only available inside an actor block")
                .with_help("move this receive expression into an actor block")
        }

        TypeError::InvalidChildStart { found, span, .. } => {
            let found = &found.with_holes();
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("expected Pid<M>, found {}", found))
                .with_help("the start function must call spawn() and return a Pid")
        }

        TypeError::InvalidStrategy { span, .. } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                "expected one_for_one, one_for_all, rest_for_one, or simple_one_for_one",
            )
        }

        TypeError::InvalidRestartType { span, .. } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "expected permanent, transient, or temporary")
        }

        TypeError::InvalidShutdownValue { span, .. } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "expected a positive integer or brutal_kill")
        }

        // ── Multi-clause function diagnostics (11-02) ──────────────────
        TypeError::CatchAllNotLast { span, .. } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "clauses after a catch-all are unreachable")
        }
        TypeError::NonConsecutiveClauses {
            first_span,
            second_span,
            ..
        } => {
            let range = clamp(text_range_to_range(*second_span));
            let first_range = clamp(text_range_to_range(*first_span));

            Description::build(ReportKind::Error, range.clone())
                .with_label(
                    Label::new(first_range)
                        .with_message("first definition here")
                        .with_color(Color::Blue),
                )
                .with_label(
                    Label::new(range)
                        .with_message("non-consecutive redefinition here")
                        .with_color(Color::Red),
                )
        }
        TypeError::NonFirstClauseAnnotation { span, .. } => {
            let range = clamp(text_range_to_range(*span));

            Description::build(ReportKind::Warning, range.clone()).with_label(
                Label::new(range)
                    .with_message("only the first clause should have this annotation")
                    .with_color(Color::Yellow),
            )
        }
        TypeError::DuplicateImpl {
            first_impl, span, ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(span, first_impl.to_string())
                .with_help("remove one of the conflicting impl blocks")
        }

        TypeError::AmbiguousMethod {
            method_name,
            candidate_traits,
            span,
            ..
        } => {
            let range = clamp(text_range_to_range(*span));

            let suggestions: Vec<String> = candidate_traits
                .iter()
                .map(|t| format!("{}.{}(value)", t, method_name))
                .collect();
            let help = format!("use qualified syntax: {}", suggestions.join(" or "));

            Description::error(range, format!("multiple traits provide `{}`", method_name))
                .with_help(help)
        }

        TypeError::UnsupportedDerive {
            trait_name, span, ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(span, format!("`{}` cannot be derived here", trait_name))
                .with_help(
                    "structs derive Eq, Ord, Display, Debug, Hash, Json, Row, and Schema; sum types all but Row and Schema",
                )
        }

        TypeError::MissingDerivePrerequisite {
            trait_name,
            requires,
            span,
            ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(
                span,
                format!(
                    "`{}` requires `{}` for its implementation",
                    trait_name, requires
                ),
            )
            .with_help(format!(
                "add `{}` to the deriving list: deriving({}, {})",
                requires, requires, trait_name
            ))
        }

        TypeError::BreakOutsideLoop { span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                "`break` can only be used inside a `while` or `for` loop",
            )
            .with_help("move this `break` inside a loop body")
        }

        TypeError::ContinueOutsideLoop { span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                "`continue` can only be used inside a `while` or `for` loop",
            )
            .with_help("move this `continue` inside a loop body")
        }

        // The headline names the module and any near miss, or the name and
        // what the module exports.
        TypeError::ImportModuleNotFound { span, .. } => {
            Description::error(clamp(text_range_to_range(*span)), "no module has this name")
        }

        TypeError::ImportNameNotFound { span, .. } => {
            Description::error(clamp(text_range_to_range(*span)), "not exported")
        }

        TypeError::PrivateItem {
            module_name,
            name,
            span,
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "private to its module").with_help(format!(
                "add `pub` to `{}` in module `{}` to make it accessible",
                name, module_name
            ))
        }

        TypeError::HttpClusteredInvalidArguments { span, .. } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "this wrapper")
                .with_help(
                    "use `HTTP.clustered(handler)` or `HTTP.clustered(<int>, handler)` with a public top-level route handler reference",
                )
        }

        TypeError::HttpClusteredPrivateHandler { handler_name, span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                format!(
                    "`{}` is private and cannot cross the clustered route boundary",
                    handler_name
                ),
            )
            .with_help(format!(
                "add `pub` to `{}` or remove `HTTP.clustered(...)`",
                handler_name
            ))
        }

        TypeError::HttpClusteredOutsideRouteHandlerPosition { span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                "use this wrapper directly as the handler argument to `HTTP.route(...)` or `HTTP.on_*(...)`")
        }

        TypeError::HttpClusteredConflictingReplicationCount {
            runtime_name,
            first_count,
            current_count,
            first_span,
            span,
        } => {
            let first_range = clamp(text_range_to_range(*first_span));
            let current_range = clamp(text_range_to_range(*span));

            Description::build(ReportKind::Error, current_range.clone())
                .with_label(
                    Label::new(first_range)
                        .with_message(format!(
                            "`{}` was first declared here with replication count {}",
                            runtime_name, first_count
                        ))
                        .with_color(Color::Blue),
                )
                .with_label(
                    Label::new(current_range)
                        .with_message(format!(
                            "conflicting replication count {} for `{}`",
                            current_count, runtime_name
                        ))
                        .with_color(Color::Red),
                )
                .with_help("keep one replication count per clustered route handler runtime name")
        }

        TypeError::HttpClusteredImportedOriginMissing { handler_name, span } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!(
                            "cannot determine the defining module for imported handler `{}`",
                            handler_name
                        ))
                .with_note(
                    "imported bare handlers must preserve their defining module so clustered route lowering can keep the real runtime name",
                )
        }

        TypeError::TryIncompatibleReturn {
            operand_ty,
            fn_return_ty,
            span,
        } => {
            let operand_ty = &operand_ty.with_holes();
            let fn_return_ty = &fn_return_ty.with_holes();
            let range = clamp(text_range_to_range(*span));

            Description::error(range, "cannot use `?` here")
                .with_note(format!(
                    "cannot propagate `{}` from a function returning `{}`; Result errors must match or have a From conversion, and Option requires an Option return type",
                    operand_ty, fn_return_ty
                ))
        }

        TypeError::TryOnNonResultOption { operand_ty, span } => {
            let operand_ty = &operand_ty.with_holes();
            let range = clamp(text_range_to_range(*span));

            Description::error(
                range,
                format!("type `{}` is not `Result` or `Option`", operand_ty),
            )
            .with_help("the `?` operator can only be used on `Result<T, E>` or `Option<T>` values")
        }

        TypeError::NonSerializableField {
            struct_name: _,
            field_type,
            span,
            ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(span, format!("`{}` is not serializable", field_type))
                .with_help(format!(
                    "type `{}` does not derive Json; add `deriving(Json)` to its definition, or use a serializable type (Int, Float, Bool, String, a tuple, Option<T>, List<T>, Map<String, V>)",
                    field_type
                ))
        }

        TypeError::NonMappableField {
            struct_name: _,
            field_type,
            span,
            ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(span, format!("`{}` is not row-mappable", field_type))
                .with_help("only Int, Float, Bool, String, and Option<T> fields are supported for deriving(Row)")
        }

        TypeError::MissingAssocType {
            assoc_name, span, ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(span, format!("missing `type {} = ...`", assoc_name)).with_help(
                format!(
                    "add `type {} = <ConcreteType>` to the impl block",
                    assoc_name
                ),
            )
        }

        TypeError::ExtraAssocType {
            trait_name,
            assoc_name,
            span,
            ..
        } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(
                span,
                format!("`{}` is not declared by `{}`", assoc_name, trait_name),
            )
        }

        TypeError::UnresolvedAssocType { assoc_name, span } => {
            let span = clamp(text_range_to_range(*span));

            Description::error(span, "not declared").with_help(format!(
                "declare it in the interface with `type {assoc_name}`, and bind it in each impl"
            ))
        }

        TypeError::SlotPipeOutOfRange {
            slot, arity, span, ..
        } => {
            let range = clamp(text_range_to_range(*span));

            let mut builder = Description::error(
                range,
                format!("slot {} exceeds function arity {}", slot, arity),
            );

            if *arity <= 1 {
                builder.set_help("use |> to pipe as the first argument");
            } else {
                builder.set_help(format!(
                    "use |> to pipe as the first argument, or |2>...|{}> for other positions",
                    arity
                ));
            }

            builder
        }

        TypeError::UndefinedType {
            target_name, span, ..
        } => {
            let range = clamp(text_range_to_range(*span));

            Description::error(range, format!("`{}` is not defined", target_name)).with_help(
                format!(
                    "define `{}` as a struct, sum type, or type alias before using it here",
                    target_name
                ),
            )
        }
        TypeError::NativeDeclarationInvalid { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "this declaration").with_help(
                "use a public, fully annotated, non-generic signature and a C identifier symbol",
            )
        }
        TypeError::ExportDeclarationInvalid { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "this declaration").with_help(
                "use `@export(\"c_symbol\") pub fn name(request :: Bytes) -> Bytes!String`",
            )
        }
        TypeError::InvalidLetPattern { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "this pattern")
                .with_help(
                    "use only lowercase names, `_`, and tuple and struct patterns; use `case` for refutable patterns",
                )
        }
        TypeError::DuplicateField { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "given again here").with_help("give each field one value")
        }
        TypeError::UnderivableField {
            trait_name, span, ..
        } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "functions cannot be compared, hashed or shown")
                .with_help(format!("remove `{trait_name}` from the deriving list"))
        }
        TypeError::DuplicateVariant { variant, span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(
                range,
                format!("`{variant}` would name either type's variant"),
            )
            .with_help("give one of the variants another name")
        }
        TypeError::AmbiguousImplMethod {
            method,
            candidates,
            found,
            span,
            by_argument,
            ..
        } => {
            let range = clamp(text_range_to_range(*span));
            let types = candidates
                .iter()
                .map(|ty| format!("`{ty}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let first = candidates
                .first()
                .map_or_else(|| "Int".to_string(), |ty| ty.to_string());
            let (label, help) = match (found, by_argument) {
                (Some(_), false) => (
                    format!("they return {types}"),
                    format!("give the result a type: `let x :: {first} = value.{method}()`"),
                ),
                (None, false) => (
                    format!("its impls return {types}"),
                    format!("give the result a type: `let x :: {first} = value.{method}()`"),
                ),
                (Some(_), true) => (
                    format!("they take {types}"),
                    "convert the argument to one of those types first".to_string(),
                ),
                (None, true) => (
                    format!("its impls take {types}"),
                    format!("give the argument a type, such as `{first}`"),
                ),
            };
            Description::error(range, label).with_help(help)
        }
        TypeError::DuplicateDefinition { name, span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "defined again here")
                .with_help(format!("rename or remove one of the two `{name}`s"))
        }
        TypeError::UnknownType { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "no type has this name")
                .with_help("check the spelling, or define or import the type")
        }
        TypeError::NoSuchModuleFunction {
            module,
            available,
            span,
            ..
        } => {
            let range = clamp(text_range_to_range(*span));
            let mut report = Description::error(range, format!("not a function of `{module}`"));
            if !available.is_empty() {
                report = report.with_help(format!("`{module}` has {}", available.join(", ")));
            }
            report
        }
        TypeError::AssertReceiveOutsideTest { span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "not in a test file")
                .with_help("elsewhere, write the `receive ... after TIMEOUT -> ...` it stands for")
        }
        TypeError::GenericImplTarget { name, span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, format!("`{name}` takes type parameters")).with_help(format!(
                "implement it for a type without type parameters, such as a struct \
                     holding the `{name}` you mean"
            ))
        }
        TypeError::OverloadedFunctionValue {
            name,
            arities,
            span,
        } => {
            let range = clamp(text_range_to_range(*span));
            let counts = crate::error::arity_list(arities, " or ");
            let shown = arities.iter().copied().find(|&a| a > 0).unwrap_or(0);
            let params: Vec<String> = (0..shown).map(|i| format!("a{i}")).collect();
            let params = params.join(", ");
            Description::error(range, "each arity is its own function").with_help(format!(
                "call it with {counts} arguments; as a value, use a closure \
                     that calls one: `fn {params} -> {name}({params}) end`"
            ))
        }
        TypeError::InvalidConcat { ty, span, .. } => {
            let ty = &ty.with_holes();
            let range = clamp(text_range_to_range(*span));
            Description::error(range, format!("these operands are `{ty}`"))
                .with_help("convert the values to strings first, e.g. with `\"${a}${b}\"`")
        }
        TypeError::TopLevelLet { name, span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "a module has no global bindings")
                .with_help(format!(
                    "move it into the function that uses it, or make it a function: `fn {name}() do ... end`"
                ))
        }
        TypeError::TopLevelStatement { span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "nothing runs this").with_help(
                "a program runs `main` and what it calls: move this into `main` or the function \
                 that needs it",
            )
        }
        TypeError::ModuleNotImported { module, span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "a module of this project")
                .with_help(format!("add `import {module}` at the top of the file"))
        }
        TypeError::ActorMessageTypeUnknown { actor, span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "no typed `Pid` reaches this actor")
                .with_help(format!(
                    "give the pid a message type where it is spawned, as `let pid :: Pid<Int> = spawn({actor})`, or use the messages in the actor as the type they are"
                ))
        }
        TypeError::IndexingUnsupported { span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "no indexing syntax").with_help(
                "use `List.get(list, index)`, `Map.get(map, key)`, or `Tuple.nth(tuple, index)`",
            )
        }
        TypeError::InvalidLiteral { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "invalid literal")
        }
        TypeError::UnknownInterface { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "no interface has this name")
                .with_help("check the spelling, or declare the interface or import its module")
        }
        TypeError::UnknownFieldOwner { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "nothing here fixes the type of this value")
                .with_help("annotate the value's type, such as a parameter `p :: Point`")
        }
        TypeError::NestedDefinition { keyword, span } => {
            let range = clamp(text_range_to_range(*span));
            let help = if *keyword == "fn" {
                "move it to the top level, or bind a closure: `let name = fn x -> ... end`"
            } else {
                "move it to the top level of the module"
            };
            Description::error(range, "inside a function").with_help(help)
        }
        TypeError::TypeArgumentCount { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "wrong number of type arguments")
        }
        TypeError::TypeNotValue { name, span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(
                range,
                "a type names no value")
            .with_help(format!(
                "build a value of it (`{name} {{ ... }}` for a struct, a variant for a sum type), or call one of its methods, `{name}.method(...)`"
            ))
        }
        TypeError::UntypedMethodParam { param, span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(
                range,
                "nothing fixes the type of this parameter")
            .with_help(format!(
                "annotate it, `{param} :: Type`: a method is compiled once, for the type it is implemented for, not for each call"
            ))
        }
        TypeError::MethodReturnUnknown { method, span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "nothing fixes what this method returns").with_help(format!(
                "annotate its return type in the `impl` or the interface, `fn {method}(self) -> Type`"
            ))
        }
        TypeError::RigidTypeParam { param, found, span } => {
            let found = &found.with_holes();
            let range = clamp(text_range_to_range(*span));
            Description::error(range, format!("`{param}` is declared here"))
                .with_help(format!(
                    "a generic function must work for every `{param}`: use `{found}` in its signature instead, or keep `{param}` values as they are"
                ))
        }
        TypeError::AmbiguousStaticMethod {
            method,
            types,
            span,
        } => {
            let range = clamp(text_range_to_range(*span));
            let example = types.first().cloned().unwrap_or_else(|| "Type".to_string());
            Description::error(range, "which type's is meant?")
                .with_help(format!("call it on the type: `{example}.{method}()`"))
        }
        TypeError::AmbiguousDefault { span } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "nothing fixes this value's type")
                .with_help("annotate it: `let x :: Int = default()`")
        }
        TypeError::CyclicAlias { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "expanding it never ends").with_help(
                "an alias names an existing type; use a struct or sum type for a recursive type",
            )
        }
        TypeError::NotAStruct { ty, span } => {
            let range = clamp(text_range_to_range(*span));
            let label = if matches!(ty, Ty::Var(_)) {
                "the type of this value is not known here"
            } else {
                "not a struct"
            };
            Description::error(range, label)
                .with_help("`Name { field: value }` and `%{value | field: new}` work on structs; annotate the value with its struct type if it has one")
        }
        TypeError::DuplicateBinding { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "each name in a pattern binds one value")
                .with_help("rename one of them, or compare the values in a `when` guard")
        }
        TypeError::InvalidPassThroughArm { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "this arm")
                .with_help("write the value after `->`: `pattern -> value`")
        }
        TypeError::ResourceViolation { span, .. } => {
            let range = clamp(text_range_to_range(*span));
            Description::error(range, "here")
                .with_help("move each resource once, or pass it to a direct `borrow` parameter")
        }
    }
}

// ── Main Rendering Function ────────────────────────────────────────────

/// Render a type error into a formatted diagnostic string using ariadne.
///
/// Accepts `DiagnosticOptions` to control color and output format.
/// When `options.json` is true, delegates to `render_json_diagnostic`.
/// When `options.color` is false, uses colorless config for deterministic
/// test snapshots.
///
/// The optional `suggestions` parameter provides names in scope for
/// "did you mean X?" suggestions on E0004/E0010 errors.
pub fn render_diagnostic(
    error: &TypeError,
    source: &str,
    filename: &str,
    options: &DiagnosticOptions,
    suggestions: Option<&[String]>,
) -> String {
    if options.json {
        // One object per line: the caller prints the rendering verbatim.
        let mut line = render_json_diagnostic(error, source, filename, suggestions);
        line.push('\n');
        return line;
    }

    let code = error_code(error);

    let fname = filename.to_string();

    let report = describe(error, source, suggestions).report(&fname, code, options.report_config());

    let mut buf = Vec::new();
    let cache = ariadne::sources([(fname, source.to_string())]);
    report
        .write(cache, &mut buf)
        .expect("failed to write diagnostic");
    let rendered = String::from_utf8(buf).expect("diagnostic output should be valid UTF-8");
    // ariadne pads some lines with trailing spaces. Nothing reads them, and the
    // repository's whitespace guard strips them from committed snapshots.
    let mut trimmed = rendered
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    if rendered.ends_with('\n') {
        trimmed.push('\n');
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Errors with a place of their own point there in JSON too; some were
    /// reported as covering the whole file.
    #[test]
    fn json_diagnostics_point_at_the_error_not_the_whole_file() {
        let source = "fn f(0) = 0\nfn g() = 1\nfn f(n) = n\n";
        let second = source.rfind("fn f").unwrap();
        let range = |start: usize, end: usize| {
            rowan::TextRange::new((start as u32).into(), (end as u32).into())
        };
        let error = TypeError::NonConsecutiveClauses {
            fn_name: "f".to_string(),
            arity: 1,
            first_span: range(0, 11),
            second_span: range(second, second + 11),
        };
        let json: serde_json::Value =
            serde_json::from_str(&render_json_diagnostic(&error, source, "m.mpl", None)).unwrap();
        assert_eq!(json["spans"][0]["start"], second, "{json}");
        assert_eq!(json["spans"][0]["end"], second + 11, "{json}");
        // An error about no one place still covers the whole source.
        let error = TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Builtin,
        };
        let json: serde_json::Value =
            serde_json::from_str(&render_json_diagnostic(&error, source, "m.mpl", None)).unwrap();
        assert_eq!(json["spans"][0]["end"], source.len(), "{json}");
    }

    /// A mismatch suggests how to turn the value found into the one
    /// expected. `Ok(...)` was not suggested for a `Result` whose value type
    /// has a comma in it (`Map<String, Int>`) or a part not settled yet.
    #[test]
    fn mismatches_suggest_the_conversion_they_need() {
        let con = |name: &str| Ty::Con(crate::ty::TyCon::new(name));
        let map = |k: Ty, v: Ty| Ty::map(k, v);
        let cases = [
            (Ty::option(Ty::int()), Ty::int(), Some("wrap in Some(...)")),
            (Ty::option(Ty::int()), Ty::string(), None),
            (
                Ty::result(map(Ty::string(), Ty::int()), Ty::string()),
                map(con("_"), con("_")),
                Some("wrap in Ok(...)"),
            ),
            (
                Ty::result(Ty::Tuple(vec![Ty::int()]), Ty::string()),
                Ty::Tuple(vec![Ty::int(), Ty::int()]),
                None,
            ),
            (
                Ty::result(Ty::struct_ty("Point", vec![]), Ty::string()),
                con("Point"),
                Some("wrap in Ok(...)"),
            ),
            (
                Ty::int(),
                Ty::float(),
                Some("convert it with `Float.to_int(...)`"),
            ),
            (
                Ty::float(),
                Ty::int(),
                Some("convert it with `Int.to_float(...)`"),
            ),
            (Ty::string(), Ty::int(), Some("use to_string()")),
            (Ty::string(), Ty::float(), Some("use to_string()")),
            (Ty::bool(), Ty::int(), Some("expected a boolean expression")),
            (Ty::string(), Ty::bool(), None),
            (
                Ty::string(),
                Ty::json(),
                Some("encode it: `Json.encode(...)` is its JSON text"),
            ),
            (
                Ty::json(),
                Ty::string(),
                Some("parse it: `Json.parse(...)` reads JSON text into a Json"),
            ),
        ];
        for (expected, found, suggestion) in cases {
            assert_eq!(
                fix_suggestion(&expected, &found),
                suggestion,
                "{expected} / {found}"
            );
        }
    }

    /// A type that lacks a trait is told how to get it: JSON by what JSON
    /// holds, a named type by a derive or an impl, and a type with
    /// parameters that it can have no impl.
    #[test]
    fn missing_traits_are_explained_by_the_type() {
        let help = |ty: Ty, trait_name: &str| {
            let error = TypeError::TraitNotSatisfied {
                ty,
                trait_name: trait_name.to_string(),
                origin: ConstraintOrigin::Builtin,
            };
            let json: serde_json::Value =
                serde_json::from_str(&render_json_diagnostic(&error, "x", "m.mpl", None)).unwrap();
            json["fix"].as_str().unwrap_or_default().to_string()
        };
        let point = || Ty::struct_ty("Point", vec![]);
        assert!(help(Ty::int(), "Json").starts_with("JSON holds Int, Float"));
        assert!(help(point(), "Eq").starts_with("add `deriving(Eq)`"));
        assert_eq!(
            help(point(), "Named"),
            "add `impl Named for Point do ... end`"
        );
        assert_eq!(
            help(Ty::list(Ty::int()), "Named"),
            "only a named type without type parameters can have an `impl`"
        );
    }

    /// A span ariadne can underline: on character boundaries, inside the
    /// source, and at least one character wide.
    #[test]
    fn report_spans_are_whole_characters_inside_the_source() {
        let source = "é = 1\n";
        // Inside the two bytes of `é`: the character.
        assert_eq!(report_span(source, 1..1), 0..2);
        assert_eq!(report_span(source, 1..4), 0..4);
        // Empty: the character there, or the last one at the end.
        assert_eq!(report_span(source, 3..3), 3..4);
        assert_eq!(report_span(source, 7..9), 6..7);
        assert_eq!(report_span("", 0..0), 0..0);
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "abd"), 1);
        assert_eq!(levenshtein_distance("count", "cound"), 1);
    }

    #[test]
    fn test_find_closest_name() {
        let candidates = vec![
            "count".to_string(),
            "counter".to_string(),
            "amount".to_string(),
        ];
        assert_eq!(
            find_closest_name("cont", &candidates, 2),
            Some("count".to_string())
        );
        assert_eq!(find_closest_name("xyz", &candidates, 2), None);
        assert_eq!(
            find_closest_name("count", &candidates, 2),
            Some("count".to_string())
        );
    }

    #[test]
    fn test_diagnostic_options_default() {
        let opts = DiagnosticOptions::default();
        assert!(opts.color);
        assert!(!opts.json);
    }

    #[test]
    fn test_diagnostic_options_colorless() {
        let opts = DiagnosticOptions::colorless();
        assert!(!opts.color);
        assert!(!opts.json);
    }

    #[test]
    fn test_diagnostic_options_json() {
        let opts = DiagnosticOptions::json_mode();
        assert!(!opts.color);
        assert!(opts.json);
    }

    #[test]
    fn test_json_diagnostic_serialization() {
        let diag = JsonDiagnostic {
            code: "E0001".to_string(),
            severity: "error".to_string(),
            message: "expected Int, found String".to_string(),
            file: "main.mpl".to_string(),
            spans: vec![JsonSpan {
                start: 10,
                end: 15,
                label: "expected Int, found String".to_string(),
            }],
            fix: None,
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("E0001"));
        assert!(json.contains("expected Int, found String"));
        assert!(json.contains("\"fix\":null"));
    }

    #[test]
    fn test_json_diagnostic_with_fix() {
        let diag = JsonDiagnostic {
            code: "E0001".to_string(),
            severity: "error".to_string(),
            message: "expected Option<Int>, found Int".to_string(),
            file: "test.mpl".to_string(),
            spans: vec![],
            fix: Some("wrap in Some(...)".to_string()),
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("\"fix\":\"wrap in Some(...)\""));
    }
}
