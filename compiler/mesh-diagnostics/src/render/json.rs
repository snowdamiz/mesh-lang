//! The format for a tool: one JSON object per line, keys in a fixed order.
//! The first six keys are the original contract; the rest are additive.

use serde::Serialize;

use super::Paths;
use crate::{Applicability, Diagnostic, SourceFile};

#[derive(Serialize)]
struct Wire<'a> {
    code: &'static str,
    severity: &'static str,
    message: &'a str,
    file: String,
    spans: Vec<WireSpan<'a>>,
    fix: &'a str,
    line: Option<u32>,
    col: Option<u32>,
    name: &'static str,
    category: &'static str,
    notes: &'a [String],
    guide: Guide,
    docs_url: String,
    edits: Vec<WireEdit<'a>>,
}

#[derive(Serialize)]
struct WireSpan<'a> {
    start: u32,
    end: u32,
    label: &'a str,
    file: String,
    line: Option<u32>,
    col: Option<u32>,
    end_line: Option<u32>,
    end_col: Option<u32>,
}

#[derive(Serialize)]
struct Guide {
    explain: String,
    skill: String,
    topic: Option<&'static str>,
}

#[derive(Serialize)]
struct WireEdit<'a> {
    file: String,
    start: u32,
    end: u32,
    replacement: &'a str,
    applicability: &'static str,
}

pub(super) fn render<'a>(diagnostic: &'a Diagnostic, paths: &Paths) -> String {
    let code = diagnostic.code();
    let info = code.info();
    let own = diagnostic.file();
    let display =
        |file: Option<&SourceFile>| file.map_or_else(String::new, |f| paths.display(f.path()));

    let span = |file: Option<&SourceFile>, span: mesh_common::span::Span, label: &'a str| {
        let span = file.map_or(span, |f| f.clamp(span));
        let at = |offset| file.and_then(|f| f.line_col(offset));
        let (start, end) = (at(span.start), at(span.end));
        WireSpan {
            start: span.start,
            end: span.end,
            label,
            file: display(file),
            line: start.map(|at| at.line),
            col: start.map(|at| at.col),
            end_line: end.map(|at| at.line),
            end_col: end.map(|at| at.col),
        }
    };
    // `spans[0]` is the primary span; a diagnostic without one has none.
    let mut spans = Vec::new();
    let (line, col) = match diagnostic.span() {
        Some(primary) => {
            let label = diagnostic.primary_label().unwrap_or(diagnostic.message());
            let primary = span(own, primary, label);
            let at = (primary.line, primary.col);
            spans.push(primary);
            for label in diagnostic.labels() {
                spans.push(span(label.file().or(own), label.span(), label.message()));
            }
            at
        }
        None => (diagnostic.line(), None),
    };

    let wire = Wire {
        code: code.id(),
        severity: diagnostic.severity().as_str(),
        message: diagnostic.message(),
        file: display(own),
        spans,
        fix: diagnostic.repair(),
        line,
        col,
        name: info.name,
        category: code.family().as_str(),
        notes: diagnostic.notes(),
        guide: Guide {
            explain: code.explain_command(),
            skill: code.skill_ref(),
            topic: info.topic,
        },
        docs_url: code.docs_url(),
        edits: diagnostic
            .edits()
            .iter()
            .map(|edit| WireEdit {
                file: display(edit.file().or(own)),
                start: edit.span().start,
                end: edit.span().end,
                replacement: edit.replacement(),
                applicability: match edit.applicability() {
                    Applicability::MachineApplicable => "machine-applicable",
                    Applicability::MaybeIncorrect => "maybe-incorrect",
                },
            })
            .collect(),
    };
    let mut line = serde_json::to_string(&wire).expect("a diagnostic serializes");
    line.push('\n');
    line
}
