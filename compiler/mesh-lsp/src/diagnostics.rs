//! Diagnostics as an editor shows them.

use std::collections::HashMap;
use std::path::Path;

use mesh_common::span::Span;
use mesh_diagnostics::{Diagnostic, Severity, SourceFile};
use serde_json::json;
use tower_lsp::lsp_types::{
    self, CodeDescription, DiagnosticRelatedInformation, DiagnosticSeverity, Location,
    NumberOrString, Position, Range, Url,
};

/// `d`, found while analyzing `document`, as an LSP diagnostic. Nothing is
/// dropped:
/// - `range`: the primary span. Without one, the whole first line of the
///   diagnostic's file, or of `document` for a diagnostic about no file.
/// - `code` and `code_description`: the catalog ID and its docs page.
/// - `message`: the observation, then `help:` and the repair, then a
///   `note:` line per note.
/// - `related_information`: every secondary label, in its own file.
/// - `data`: the edits that make the repair, for code actions.
///
/// `uri_for` names a file's document; a file it cannot name is `document`'s.
pub(crate) fn to_lsp(
    d: &Diagnostic,
    document: &SourceFile,
    uri_for: &dyn Fn(&Path) -> Option<Url>,
) -> lsp_types::Diagnostic {
    let file = d.file().unwrap_or(document);
    let uri = |file: &SourceFile| uri_for(file.path()).or_else(|| uri_for(document.path()));
    let shown = |file: &SourceFile, span: Span| {
        range(file, file.clamp(span)).unwrap_or_else(|| first_line(file, document))
    };

    let mut message = format!("{}\nhelp: {}", d.message(), d.repair());
    for note in d.notes() {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    let related: Vec<_> = d
        .labels()
        .iter()
        .filter_map(|label| {
            let file = label.file().unwrap_or(file);
            Some(DiagnosticRelatedInformation {
                location: Location {
                    uri: uri(file)?,
                    range: shown(file, label.span()),
                },
                message: label.message().to_string(),
            })
        })
        .collect();
    let edits: Vec<_> = d
        .edits()
        .iter()
        .filter_map(|edit| {
            let file = edit.file().unwrap_or(file);
            Some(json!({
                "uri": uri(file)?,
                "range": range(file, edit.span())?,
                "newText": edit.replacement(),
                "applicability": edit.applicability().as_str(),
            }))
        })
        .collect();

    lsp_types::Diagnostic {
        range: d
            .span()
            .map_or_else(|| first_line(file, document), |span| shown(file, span)),
        severity: Some(match d.severity() {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(d.code().id().to_string())),
        code_description: Some(CodeDescription {
            href: Url::parse(&d.code().docs_url()).expect("a catalog docs URL parses"),
        }),
        source: Some("mesh".to_string()),
        message,
        related_information: (!related.is_empty()).then_some(related),
        tags: None,
        data: (!edits.is_empty()).then(|| json!({ "edits": edits })),
    }
}

/// `ds` as LSP diagnostics, by the document each is about: one about
/// `util.mpl` found while analyzing `main.mpl` is `util.mpl`'s.
pub(crate) fn by_uri(
    ds: &[Diagnostic],
    document: &SourceFile,
    uri_for: &dyn Fn(&Path) -> Option<Url>,
) -> HashMap<Url, Vec<lsp_types::Diagnostic>> {
    let mut grouped: HashMap<Url, Vec<lsp_types::Diagnostic>> = HashMap::new();
    for d in ds {
        let file = d.file().unwrap_or(document);
        if let Some(uri) = uri_for(file.path()).or_else(|| uri_for(document.path())) {
            grouped
                .entry(uri)
                .or_default()
                .push(to_lsp(d, document, uri_for));
        }
    }
    grouped
}

/// `span` of `file` in UTF-16 positions, when the file's text is at hand.
fn range(file: &SourceFile, span: Span) -> Option<Range> {
    let position = |offset| {
        file.utf16_position(offset)
            .map(|(line, character)| Position::new(line, character))
    };
    Some(Range::new(position(span.start)?, position(span.end)?))
}

/// The first line of `file`, or of `document` when the file's text is not
/// at hand: where a diagnostic about a whole file is shown.
fn first_line(file: &SourceFile, document: &SourceFile) -> Range {
    let text = file.text().or(document.text()).unwrap_or_default();
    let line = text.lines().next().unwrap_or_default();
    let end = line.chars().map(|c| c.len_utf16() as u32).sum();
    Range::new(Position::new(0, 0), Position::new(0, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_diagnostics::{Applicability, Code};

    const MAIN: &str = "fn main() do\n  println(cuont)\nend\n";
    const UTIL: &str = "pub fn área() = 1\n";

    fn main_mpl() -> SourceFile {
        SourceFile::new("/p/main.mpl", MAIN)
    }

    fn util_mpl() -> SourceFile {
        SourceFile::new("/p/util.mpl", UTIL)
    }

    fn span_of(text: &str, needle: &str) -> Span {
        let start = text.find(needle).unwrap() as u32;
        Span::new(start, start + needle.len() as u32)
    }

    fn uri_for(path: &Path) -> Option<Url> {
        Url::from_file_path(path).ok()
    }

    fn uri(path: &str) -> Url {
        Url::from_file_path(path).unwrap()
    }

    #[test]
    fn a_diagnostic_keeps_its_code_repair_notes_and_docs() {
        let d = Diagnostic::in_file(Code::UnboundVariable, &main_mpl(), "undefined `cuont`")
            .at(span_of(MAIN, "cuont"), "not found")
            .with_repair("did you mean `count`?")
            .with_note("first note")
            .with_note("second note");
        let lsp = to_lsp(&d, &main_mpl(), &uri_for);
        assert_eq!(lsp.code, Some(NumberOrString::String("E0004".to_string())));
        assert_eq!(
            lsp.code_description.unwrap().href.as_str(),
            Code::UnboundVariable.docs_url()
        );
        assert_eq!(lsp.source.as_deref(), Some("mesh"));
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(
            lsp.message,
            "undefined `cuont`\nhelp: did you mean `count`?\nnote: first note\nnote: second note"
        );
        assert_eq!(
            lsp.range,
            Range::new(Position::new(1, 10), Position::new(1, 15))
        );
        assert_eq!(lsp.related_information, None);
        assert_eq!(lsp.data, None);
    }

    #[test]
    fn a_warning_is_a_warning() {
        let d = Diagnostic::in_file(Code::RedundantArm, &main_mpl(), "unreachable")
            .at(Span::new(0, 2), "");
        let lsp = to_lsp(&d, &main_mpl(), &uri_for);
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn a_diagnostic_without_a_span_covers_the_first_line() {
        let about_util = Diagnostic::in_file(Code::TypeMismatch, &util_mpl(), "whole file");
        assert_eq!(
            to_lsp(&about_util, &main_mpl(), &uri_for).range,
            Range::new(Position::new(0, 0), Position::new(0, 17)),
            "`á` is one UTF-16 unit"
        );
        let about_no_file = Diagnostic::new(Code::Uncategorized, "bad manifest");
        assert_eq!(
            to_lsp(&about_no_file, &main_mpl(), &uri_for).range,
            Range::new(Position::new(0, 0), Position::new(0, 12))
        );
    }

    #[test]
    fn an_empty_span_is_shown_one_character_wide() {
        let d = Diagnostic::in_file(Code::UnexpectedToken, &main_mpl(), "expected `end`")
            .at(Span::new(MAIN.len() as u32, MAIN.len() as u32), "");
        let lsp = to_lsp(&d, &main_mpl(), &uri_for);
        assert_ne!(lsp.range.start, lsp.range.end, "{:?}", lsp.range);
    }

    #[test]
    fn labels_and_edits_point_into_their_own_files() {
        let d = Diagnostic::in_file(Code::DuplicatePubFnAcrossModules, &main_mpl(), "twice")
            .at(span_of(MAIN, "main"), "here")
            .with_label(span_of(MAIN, "println"), "and here")
            .with_label_in(&util_mpl(), span_of(UTIL, "área"), "also here")
            .with_edit_in(
                &util_mpl(),
                span_of(UTIL, "área"),
                "size",
                Applicability::MachineApplicable,
            )
            .with_edit(
                Span::new(MAIN.len() as u32, MAIN.len() as u32),
                "\n",
                Applicability::MaybeIncorrect,
            );
        let lsp = to_lsp(&d, &main_mpl(), &uri_for);
        let related = lsp.related_information.unwrap();
        assert_eq!(related.len(), 2);
        assert_eq!(related[0].location.uri, uri("/p/main.mpl"));
        assert_eq!(related[0].message, "and here");
        assert_eq!(related[1].location.uri, uri("/p/util.mpl"));
        assert_eq!(
            related[1].location.range,
            Range::new(Position::new(0, 7), Position::new(0, 11))
        );
        let data = lsp.data.unwrap();
        let edits = data["edits"].as_array().unwrap();
        assert_eq!(edits[0]["uri"], uri("/p/util.mpl").as_str());
        assert_eq!(edits[0]["newText"], "size");
        assert_eq!(edits[0]["applicability"], "machine-applicable");
        assert_eq!(edits[1]["applicability"], "maybe-incorrect");
        assert_eq!(
            edits[1]["range"],
            json!({"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 0}}),
            "an insertion stays empty"
        );
    }

    #[test]
    fn diagnostics_are_grouped_by_the_document_they_are_about() {
        let ds = [
            Diagnostic::in_file(Code::TypeMismatch, &main_mpl(), "in main").at(Span::new(0, 2), ""),
            Diagnostic::in_file(Code::TypeMismatch, &util_mpl(), "in util").at(Span::new(0, 3), ""),
            Diagnostic::new(Code::Uncategorized, "about the project"),
        ];
        let grouped = by_uri(&ds, &main_mpl(), &uri_for);
        let messages = |path| -> Vec<String> {
            grouped[&uri(path)]
                .iter()
                .map(|d| d.message.lines().next().unwrap().to_string())
                .collect()
        };
        assert_eq!(messages("/p/main.mpl"), ["in main", "about the project"]);
        assert_eq!(messages("/p/util.mpl"), ["in util"]);
    }

    #[test]
    fn a_file_without_a_document_is_shown_on_the_one_analyzed() {
        let untitled = Url::parse("untitled:Untitled-1").unwrap();
        let document = SourceFile::new("main.mpl", MAIN);
        let uri_for = |path: &Path| (path == document.path()).then(|| untitled.clone());
        let d = Diagnostic::in_file(Code::TypeMismatch, &document, "m").at(Span::new(0, 2), "");
        let grouped = by_uri(&[d], &document, &uri_for);
        assert_eq!(grouped.keys().collect::<Vec<_>>(), [&untitled]);
    }
}
