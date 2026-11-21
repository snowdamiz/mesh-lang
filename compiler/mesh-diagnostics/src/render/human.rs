//! The format for a person: an Ariadne report where the text is at hand,
//! and a few plain lines where it is not.

use std::fmt::Write as _;

use super::{Paths, Summary};
use crate::{Code, Diagnostic, Severity};

const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub(super) fn render(diagnostic: &Diagnostic, color: bool, paths: &Paths) -> String {
    #[cfg(feature = "terminal")]
    if let Some(report) = ariadne_report::render(diagnostic, color, paths) {
        return report;
    }
    plain(diagnostic, color, paths)
}

/// `[E0001] Error: message`, where the diagnostic is, and its repair and notes.
fn plain(diagnostic: &Diagnostic, color: bool, paths: &Paths) -> String {
    let code = diagnostic.code();
    let kind = match diagnostic.severity() {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
    };
    let headline = format!("[{}] {kind}:", code.id());
    let mut text = format!(
        "{} {}\n",
        paint(&headline, severity_color(diagnostic.severity()), color),
        paths.remap(diagnostic.message())
    );
    if let Some(file) = diagnostic.file() {
        let mut location = paths.display(file.path());
        match diagnostic.span().and_then(|span| file.line_col(span.start)) {
            Some(at) => {
                let _ = write!(location, ":{}:{}", at.line, at.col);
            }
            None => {
                if let Some(line) = diagnostic.line() {
                    let _ = write!(location, ":{line}");
                }
            }
        }
        let _ = writeln!(text, "   --> {location}");
    }
    let _ = writeln!(text, "  Help: {}", diagnostic.repair());
    for note in diagnostic.notes() {
        let _ = writeln!(text, "  Note: {note}");
    }
    text
}

/// Where to read more about `code`, shown under its first diagnostic.
pub(super) fn footer(code: Code, color: bool) -> String {
    let line = format!(
        "  = {}: {} · skill diagnostics.md#{} · {}",
        code.id(),
        code.explain_command(),
        code.anchor(),
        code.docs_url()
    );
    format!("{}\n", paint(&line, DIM, color))
}

/// `error: 3 errors (E0004 ×2, E0001)`; `None` when nothing was reported.
pub(super) fn tally(summary: Summary, codes: &[(Code, u32)], color: bool) -> Option<String> {
    let count = |n: u32, what: &str| match n {
        1 => format!("1 {what}"),
        n => format!("{n} {what}s"),
    };
    let (severity, counts) = match (summary.errors, summary.warnings) {
        (0, 0) => return None,
        (0, warnings) => (Severity::Warning, count(warnings, "warning")),
        (errors, 0) => (Severity::Error, count(errors, "error")),
        (errors, warnings) => (
            Severity::Error,
            format!("{}, {}", count(errors, "error"), count(warnings, "warning")),
        ),
    };
    let codes = codes
        .iter()
        .map(|&(code, n)| match n {
            1 => code.id().to_string(),
            n => format!("{} ×{n}", code.id()),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = format!("{}:", severity.as_str());
    Some(format!(
        "{} {counts} ({codes})\n",
        paint(&prefix, severity_color(severity), color)
    ))
}

fn severity_color(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => RED,
        Severity::Warning => YELLOW,
    }
}

fn paint(text: &str, ansi: &str, color: bool) -> String {
    if color {
        format!("{ansi}{text}{RESET}")
    } else {
        text.to_string()
    }
}

#[cfg(feature = "terminal")]
mod ariadne_report {
    use std::ops::Range;

    use ariadne::{Color, Config, IndexType, Label, Report, ReportKind};

    use super::Paths;
    use crate::{Diagnostic, Severity, SourceFile};

    /// The Ariadne report, when the primary span's text is at hand. Labels
    /// in a file whose text is not are left out: there is nothing to draw.
    pub(super) fn render(diagnostic: &Diagnostic, color: bool, paths: &Paths) -> Option<String> {
        let file = diagnostic.file().filter(|file| file.text().is_some())?;
        let span = file.clamp(diagnostic.span()?);
        let (kind, primary_color) = match diagnostic.severity() {
            Severity::Error => (ReportKind::Error, Color::Red),
            Severity::Warning => (ReportKind::Warning, Color::Yellow),
        };

        let mut labels = Vec::new();
        for label in diagnostic.labels() {
            let label_file = label.file().unwrap_or(file);
            if label_file.text().is_some() {
                labels.push((
                    label_file,
                    label_file.clamp(label.span()),
                    Some(label.message()),
                    Color::Blue,
                ));
            }
        }
        // A primary span without a label is underlined only when no other
        // label is in its file: those show where the error is, and a line
        // through all of them would hide theirs.
        let primary_label = diagnostic.primary_label();
        if primary_label.is_some() || !labels.iter().any(|(f, ..)| *f == file) {
            labels.insert(0, (file, span, primary_label, primary_color));
        }
        // Ariadne draws labels of one file in one box only while their
        // lines ascend, so each file's go in order of where they end, the
        // diagnostic's file first.
        let mut files: Vec<&SourceFile> = vec![file];
        for &(label_file, ..) in &labels {
            if !files.contains(&label_file) {
                files.push(label_file);
            }
        }
        labels.sort_by_key(|(label_file, span, ..)| {
            let rank = files.iter().position(|f| f == label_file);
            (rank, span.end)
        });

        let range = |span: mesh_common::span::Span| span.start as usize..span.end as usize;
        let mut report = Report::build(kind, (paths.display(file.path()), range(span)))
            .with_code(diagnostic.code().id())
            .with_message(paths.remap(diagnostic.message()))
            .with_config(
                Config::default()
                    .with_color(color)
                    .with_index_type(IndexType::Byte),
            );
        for (label_file, span, message, label_color) in labels {
            let mut label: Label<(String, Range<usize>)> =
                Label::new((paths.display(label_file.path()), range(span))).with_color(label_color);
            if let Some(message) = message {
                label = label.with_message(message);
            }
            report.add_label(label);
        }
        report.add_help(diagnostic.repair());
        for note in diagnostic.notes() {
            report.add_note(note);
        }

        let sources = files
            .iter()
            .map(|f| (paths.display(f.path()), f.text().unwrap_or_default()));
        let mut out = Vec::new();
        report
            .finish()
            .write(ariadne::sources(sources), &mut out)
            .ok()?;
        let rendered = String::from_utf8(out).ok()?;
        // Ariadne pads some lines with trailing spaces; nothing reads them.
        let mut trimmed = rendered
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        trimmed.push('\n');
        Some(trimmed)
    }
}
