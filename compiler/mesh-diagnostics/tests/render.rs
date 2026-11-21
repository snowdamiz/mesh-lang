//! What a `Reporter` writes: for a person (Ariadne where the text is at
//! hand, plain lines where it is not, a footer once per code, a tally) and
//! for a tool (one JSON object per line).

use std::path::PathBuf;

use mesh_common::span::Span;
use mesh_diagnostics::{
    Applicability, Code, Diagnostic, Failed, Format, Reporter, SourceFile, Summary,
};

const MAIN: &str = "fn main() do\n  let count = 1\n  println(cuont)\nend\n";
const GEO: &str = "pub fn area() = 1\n";

const HUMAN: Format = Format::Human { color: false };

fn main_mpl() -> SourceFile {
    SourceFile::new("src/main.mpl", MAIN)
}

fn span_of(text: &str, needle: &str) -> Span {
    let start = text.find(needle).unwrap() as u32;
    Span::new(start, start + needle.len() as u32)
}

fn unbound() -> Diagnostic {
    let at = span_of(MAIN, "cuont");
    Diagnostic::in_file(
        Code::UnboundVariable,
        &main_mpl(),
        "undefined variable `cuont`",
    )
    .at(at, "not found in this scope")
    .with_label(span_of(MAIN, "count"), "`count` is defined here")
    .with_repair("did you mean `count`?")
    .with_edit(at, "count", Applicability::MaybeIncorrect)
}

fn duplicate_across_files() -> Diagnostic {
    let geo = SourceFile::new("src/geo.mpl", GEO);
    let main = SourceFile::new("src/main.mpl", "fn area() = 2\n");
    Diagnostic::in_file(Code::DuplicateDefinition, &main, "`area` is defined twice")
        .at(Span::new(3, 7), "defined again here")
        .with_label_in(&geo, span_of(GEO, "area"), "first defined here")
        .with_edit(Span::new(0, 14), "", Applicability::MachineApplicable)
        .with_note("each module's names are one namespace")
}

fn file_level() -> Diagnostic {
    Diagnostic::in_file(
        Code::TypeMismatch,
        &main_mpl(),
        "type mismatch: expected `Int`, found `String`",
    )
}

fn spanless() -> Diagnostic {
    Diagnostic::internal("unexpected MIR node `Foo`")
}

fn line_only() -> Diagnostic {
    Diagnostic::at_line(
        Code::TypeMismatch,
        SourceFile::unread("src/main.mpl"),
        12,
        "type mismatch: expected `Int`, found `String`",
    )
}

fn redundant() -> Diagnostic {
    Diagnostic::in_file(Code::RedundantArm, &main_mpl(), "this arm is unreachable")
}

/// The reporter's whole output for `diagnostics`, tally included.
fn report(format: Format, diagnostics: impl IntoIterator<Item = Diagnostic>) -> String {
    let mut out = Vec::new();
    let mut reporter = Reporter::new(format, &mut out);
    for diagnostic in diagnostics {
        reporter.emit(diagnostic);
    }
    reporter.finish();
    String::from_utf8(out).unwrap()
}

#[cfg(feature = "terminal")]
#[test]
fn a_spanned_diagnostic_is_an_ariadne_report() {
    insta::assert_snapshot!(report(HUMAN, [unbound()]));
}

#[cfg(feature = "terminal")]
#[test]
fn an_unlabeled_primary_span_has_a_marker() {
    let text = "fn main() = \"unfinished";
    let file = SourceFile::new("main.mpl", text);
    let quote = span_of(text, "\"");
    let diagnostic =
        Diagnostic::in_file(Code::UnterminatedString, &file, "unterminated string").at(quote, "");

    let output = report(HUMAN, [diagnostic]);
    assert!(output.contains("main.mpl:1:13"), "{output}");
    assert!(output.contains('┬'), "{output}");
}

#[cfg(feature = "terminal")]
#[test]
fn an_explicit_primary_marker_is_drawn_with_a_related_span() {
    let text = "fn main() do\n  println(1\nend\n";
    let file = SourceFile::new("main.mpl", text);
    let diagnostic = Diagnostic::in_file(Code::UnclosedDelimiter, &file, "expected `)`")
        .at(span_of(text, "end"), "closing delimiter expected here")
        .with_label(span_of(text, "(1"), "opened here");

    let output = report(HUMAN, [diagnostic]);
    assert!(output.contains("opened here"), "{output}");
    assert!(
        output.contains("closing delimiter expected here"),
        "{output}"
    );
    assert_eq!(output.matches('┬').count(), 2, "{output}");
}

#[cfg(feature = "terminal")]
#[test]
fn labels_in_other_files_get_their_own_block() {
    insta::assert_snapshot!(report(HUMAN, [duplicate_across_files()]));
}

#[test]
fn a_diagnostic_about_a_whole_file_names_the_file() {
    insta::assert_snapshot!(report(HUMAN, [file_level()]));
}

#[test]
fn a_diagnostic_about_no_file_is_its_headline_and_repair() {
    insta::assert_snapshot!(report(HUMAN, [spanless()]));
}

#[test]
fn a_runtime_diagnostic_names_its_file_and_line() {
    insta::assert_snapshot!(report(HUMAN, [line_only()]));
}

#[test]
fn a_runtime_diagnostic_may_name_its_column() {
    let human = report(HUMAN, [line_only().at_column(5)]);
    assert!(human.contains("   --> src/main.mpl:12:5\n"), "{human}");
    let json: serde_json::Value =
        serde_json::from_str(&report(Format::Json, [line_only().at_column(5)])).unwrap();
    assert_eq!(
        (json["line"].as_u64(), json["col"].as_u64()),
        (Some(12), Some(5))
    );
}

/// Without the `terminal` feature (the runtime's build) a spanned
/// diagnostic is plain lines that name its line and column.
#[cfg(not(feature = "terminal"))]
#[test]
fn without_ariadne_a_spanned_diagnostic_names_its_line_and_column() {
    assert_eq!(
        report(HUMAN, [unbound()]),
        "[E0004] Error: undefined variable `cuont`\n   --> src/main.mpl:3:11\n  Help: did you mean `count`?\n  = E0004: meshc explain E0004 · skill diagnostics.md#e0004 · https://meshlang.dev/docs/errors/#e0004\nerror: 1 error (E0004)\n"
    );
}

/// The footer points at the explanation under the first diagnostic of each
/// code, and only there; the tally counts each code in first-seen order.
#[test]
fn the_footer_is_shown_once_per_code() {
    let output = report(
        HUMAN,
        [
            spanless(),
            file_level(),
            spanless(),
            redundant(),
            file_level(),
        ],
    );
    for id in ["I0001", "E0001", "W0001"] {
        assert_eq!(
            output
                .matches(&format!("  = {id}: meshc explain {id}"))
                .count(),
            1,
            "{output}"
        );
    }
    insta::assert_snapshot!(output);
}

/// A reporter told its footers were shown writes none.
#[test]
fn a_reporter_without_footers_writes_none() {
    let mut out = Vec::new();
    let mut reporter = Reporter::new(HUMAN, &mut out).without_footers();
    reporter.emit(unbound());
    reporter.finish();
    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("undefined variable `cuont`"), "{output}");
    assert!(!output.contains("meshc explain"), "{output}");
    assert!(output.ends_with("error: 1 error (E0004)\n"), "{output}");
}

#[test]
fn the_tally_counts_errors_and_warnings() {
    let tally = |diagnostics: Vec<Diagnostic>| {
        let output = report(HUMAN, diagnostics);
        output.lines().last().unwrap_or_default().to_string()
    };
    assert_eq!(
        tally(vec![spanless(), redundant(), spanless()]),
        "error: 2 errors, 1 warning (I0001 ×2, W0001)"
    );
    assert_eq!(tally(vec![redundant()]), "warning: 1 warning (W0001)");
    assert_eq!(
        tally(vec![redundant(), redundant()]),
        "warning: 2 warnings (W0001 ×2)"
    );
    assert_eq!(report(HUMAN, []), "", "a clean run prints nothing");
}

#[test]
fn json_is_one_object_per_line_with_no_tally() {
    let output = report(
        Format::Json,
        [
            unbound(),
            duplicate_across_files(),
            file_level(),
            spanless(),
            line_only(),
        ],
    );
    assert_eq!(output.lines().count(), 5, "{output}");
    for line in output.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    insta::assert_snapshot!(output);
}

fn alias(from: &str, to: &str) -> [(PathBuf, PathBuf); 1] {
    [(PathBuf::from(from), PathBuf::from(to))]
}

#[test]
fn shown_paths_apply_to_every_file_named() {
    let copy = SourceFile::new("/tmp/copy/src/main.mpl", MAIN);
    let geo = SourceFile::new("/tmp/copy/src/geo.mpl", GEO);
    let diagnostic =
        Diagnostic::in_file(Code::UnboundVariable, &copy, "undefined variable `cuont`")
            .at(span_of(MAIN, "cuont"), "")
            .with_label_in(&geo, span_of(GEO, "area"), "here")
            .with_edit_in(
                &geo,
                span_of(GEO, "area"),
                "size",
                Applicability::MaybeIncorrect,
            );

    let mut out = Vec::new();
    let mut reporter = Reporter::new(Format::Json, &mut out);
    reporter.with_path_aliases(alias("/tmp/copy", "project"), |reporter| {
        reporter.emit(diagnostic.clone())
    });
    reporter.finish();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["file"], "project/src/main.mpl");
    assert_eq!(json["spans"][0]["file"], "project/src/main.mpl");
    assert_eq!(json["spans"][1]["file"], "project/src/geo.mpl");
    assert_eq!(json["edits"][0]["file"], "project/src/geo.mpl");

    let mut out = Vec::new();
    let mut reporter = Reporter::new(HUMAN, &mut out);
    reporter.with_path_aliases(
        alias("/tmp/copy/src/main.mpl", "tests/main.test.mpl"),
        |reporter| reporter.emit(diagnostic),
    );
    reporter.finish();
    let human = String::from_utf8(out).unwrap();
    assert!(human.contains("tests/main.test.mpl"), "{human}");
    assert!(!human.contains("/tmp/copy/src/main.mpl"), "{human}");
}

#[test]
fn shown_paths_apply_to_paths_a_message_names() {
    let diagnostic = Diagnostic::new(
        Code::ClusterDeclarationInvalid,
        "Failed to read '/tmp/copy/src/geo.mpl'",
    );
    for format in [HUMAN, Format::Json] {
        let mut out = Vec::new();
        let mut reporter = Reporter::new(format, &mut out);
        reporter.with_path_aliases(alias("/tmp/copy", "project"), |reporter| {
            reporter.emit(diagnostic.clone())
        });
        reporter.finish();
        let written = String::from_utf8(out).unwrap();
        assert!(
            written.contains("Failed to read 'project/src/geo.mpl'"),
            "{written}"
        );
        assert!(!written.contains("/tmp/copy"), "{written}");
    }
}

#[test]
fn status_lines_are_for_people_only() {
    let mut out = Vec::new();
    let mut reporter = Reporter::new(HUMAN, &mut out);
    reporter.status(format_args!("  Compiled: {}", "target/app"));
    assert_eq!(
        reporter.finish(),
        Summary {
            errors: 0,
            warnings: 0
        }
    );
    assert_eq!(String::from_utf8(out).unwrap(), "  Compiled: target/app\n");

    let mut out = Vec::new();
    let mut reporter = Reporter::new(Format::Json, &mut out);
    reporter.status("  Compiled: target/app");
    reporter.finish();
    assert!(out.is_empty());
}

#[test]
fn only_an_error_fails_a_run() {
    let mut out = Vec::new();
    let mut reporter = Reporter::new(Format::Json, &mut out);
    assert!(reporter.emit_all([redundant(), redundant()]).is_ok());
    assert!(reporter.emit_all([redundant(), spanless()]).is_err());
    let _: Failed = reporter.fail(file_level());
    let summary = reporter.finish();
    assert_eq!(
        summary,
        Summary {
            errors: 2,
            warnings: 3
        }
    );
    assert_eq!(summary.exit_code(), 1);
    assert_eq!(
        Summary {
            errors: 0,
            warnings: 3
        }
        .exit_code(),
        0
    );
    assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 5);
}

#[test]
fn colour_marks_the_headline_and_dims_the_footer() {
    let output = report(Format::Human { color: true }, [spanless()]);
    assert!(
        output.contains("\x1b[31m[I0001] Error:\x1b[0m"),
        "{output:?}"
    );
    assert!(
        output.contains("\x1b[2m  = I0001: meshc explain I0001"),
        "{output:?}"
    );
    assert!(
        output.contains("\x1b[31merror:\x1b[0m 1 error (I0001)"),
        "{output:?}"
    );
}
