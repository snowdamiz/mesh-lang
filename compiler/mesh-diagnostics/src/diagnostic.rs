use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU32;
use std::path::Path;

use mesh_common::span::Span;

use crate::{Code, Origin, Severity, SourceFile, SourceMap};

/// One diagnostic: an observation at a place, with a repair.
///
/// Invariants, kept by the constructors (the fields are private):
/// - The severity is the code's catalog severity, so an `E0001` warning
///   cannot exist.
/// - The repair is never empty. It starts as the catalog's default, and
///   [`with_repair`](Self::with_repair) replaces it with a specific one.
/// - A span is attached only together with the file it indexes, so every
///   diagnostic renders without side tables.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    code: Code,
    message: String,
    /// `None`: about the invocation, not a file.
    file: Option<SourceFile>,
    /// `None` with a file: about the whole file.
    primary: Option<Primary>,
    labels: Vec<Label>,
    repair: Cow<'static, str>,
    notes: Vec<String>,
    /// Applying every edit performs the repair.
    edits: Vec<Edit>,
}

#[derive(Clone, Debug, PartialEq)]
enum Primary {
    /// Byte offsets into the diagnostic's file.
    Span { span: Span, label: Option<String> },
    /// The runtime's: a line, and maybe a column, is known; the text is not.
    Line {
        line: NonZeroU32,
        col: Option<NonZeroU32>,
    },
}

/// A secondary span, possibly in another file.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// `None`: the diagnostic's file.
    file: Option<SourceFile>,
    span: Span,
    message: String,
}

/// How safely a tool may apply an [`Edit`] without a person reading it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Applicability {
    /// The edit is the repair; applying it unseen is safe.
    MachineApplicable,
    /// The edit is a guess, such as a "did you mean" name.
    MaybeIncorrect,
}

impl Applicability {
    /// The name tools read: `machine-applicable` or `maybe-incorrect`.
    pub fn as_str(self) -> &'static str {
        match self {
            Applicability::MachineApplicable => "machine-applicable",
            Applicability::MaybeIncorrect => "maybe-incorrect",
        }
    }
}

/// A replacement of the text at a span.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    /// `None`: the diagnostic's file.
    file: Option<SourceFile>,
    span: Span,
    replacement: String,
    applicability: Applicability,
}

impl Diagnostic {
    /// A diagnostic about the invocation rather than a file.
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            message: message.into(),
            file: None,
            primary: None,
            labels: Vec::new(),
            repair: Cow::Borrowed(code.info().repair),
            notes: Vec::new(),
            edits: Vec::new(),
        }
    }

    /// A diagnostic about `file`; [`at`](Self::at) places it in the text.
    pub fn in_file(code: Code, file: &SourceFile, message: impl Into<String>) -> Self {
        Diagnostic {
            file: Some(file.clone()),
            ..Diagnostic::new(code, message)
        }
    }

    /// A diagnostic at 1-based `line` of a file whose text is not at hand.
    pub fn at_line(code: Code, file: SourceFile, line: u32, message: impl Into<String>) -> Self {
        debug_assert!(line > 0, "lines are 1-based");
        Diagnostic {
            file: Some(file),
            primary: NonZeroU32::new(line).map(|line| Primary::Line { line, col: None }),
            ..Diagnostic::new(code, message)
        }
    }

    /// Adds the 1-based column to an [`at_line`](Self::at_line) diagnostic.
    pub fn at_column(mut self, col: u32) -> Self {
        match &mut self.primary {
            Some(Primary::Line { col: at, .. }) => *at = NonZeroU32::new(col),
            _ => debug_assert!(false, "a column needs the line it is on"),
        }
        self
    }

    /// An internal compiler error (`I0001`); `detail` becomes a note.
    pub fn internal(detail: impl fmt::Display) -> Self {
        let code = Code::InternalCompilerError;
        Diagnostic::new(code, code.info().title).with_note(detail.to_string())
    }

    /// Places the diagnostic at `span` of its file, labelled `label`.
    pub fn at(mut self, span: Span, label: impl Into<String>) -> Self {
        debug_assert!(self.file.is_some(), "a span needs the file it indexes");
        let label = label.into();
        self.primary = Some(Primary::Span {
            span,
            label: (!label.is_empty()).then_some(label),
        });
        self
    }

    /// A secondary span in the diagnostic's file.
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        debug_assert!(self.file.is_some(), "a span needs the file it indexes");
        self.labels.push(Label {
            file: None,
            span,
            message: message.into(),
        });
        self
    }

    /// A secondary span in another file.
    pub fn with_label_in(
        mut self,
        file: &SourceFile,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        self.labels.push(Label {
            file: Some(file.clone()),
            span,
            message: message.into(),
        });
        self
    }

    /// Replaces the catalog's default repair with one for this instance.
    pub fn with_repair(mut self, repair: impl Into<Cow<'static, str>>) -> Self {
        let repair = repair.into();
        debug_assert!(!repair.is_empty(), "a repair is never empty");
        if !repair.is_empty() {
            self.repair = repair;
        }
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// An edit in the diagnostic's file.
    pub fn with_edit(
        mut self,
        span: Span,
        replacement: impl Into<String>,
        applicability: Applicability,
    ) -> Self {
        debug_assert!(self.file.is_some(), "a span needs the file it indexes");
        self.edits.push(Edit {
            file: None,
            span,
            replacement: replacement.into(),
            applicability,
        });
        self
    }

    /// An edit in another file.
    pub fn with_edit_in(
        mut self,
        file: &SourceFile,
        span: Span,
        replacement: impl Into<String>,
        applicability: Applicability,
    ) -> Self {
        self.edits.push(Edit {
            file: Some(file.clone()),
            span,
            replacement: replacement.into(),
            applicability,
        });
        self
    }

    /// What the diagnostic says about the generated file at `generated`
    /// said about `source`, the file it was generated from, each span
    /// placed through `map`. Its edits stay only if each lands on text of
    /// `source` exactly: applied anywhere else, they would change text the
    /// diagnostic is not about.
    pub fn relocated(mut self, generated: &Path, source: &SourceFile, map: &SourceMap) -> Self {
        let is_generated = |file: &SourceFile| file.path() == generated;
        let own = self.file.as_ref().is_some_and(is_generated);
        let moves = |file: &Option<SourceFile>| file.as_ref().map_or(own, is_generated);
        if own {
            self.file = Some(source.clone());
            if let Some(Primary::Span { span, .. }) = &mut self.primary {
                *span = map.origin(*span).span();
            }
        }
        for label in &mut self.labels {
            if moves(&label.file) {
                label.span = map.origin(label.span).span();
                if label.file.is_some() {
                    label.file = Some(source.clone());
                }
            }
        }
        let mut exact = true;
        for edit in &mut self.edits {
            if moves(&edit.file) {
                match map.origin(edit.span) {
                    Origin::Exact(span) => edit.span = span,
                    Origin::Near(_) => exact = false,
                }
                if edit.file.is_some() {
                    edit.file = Some(source.clone());
                }
            }
        }
        if !exact {
            self.edits.clear();
        }
        self
    }

    pub fn code(&self) -> Code {
        self.code
    }

    pub fn severity(&self) -> Severity {
        self.code.severity()
    }

    /// The observation, the headline every format shows.
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn file(&self) -> Option<&SourceFile> {
        self.file.as_ref()
    }

    /// The primary span, in the diagnostic's file.
    pub fn span(&self) -> Option<Span> {
        match self.primary {
            Some(Primary::Span { span, .. }) => Some(span),
            _ => None,
        }
    }

    /// The 1-based line of the primary span or line.
    pub fn line(&self) -> Option<u32> {
        match self.primary.as_ref()? {
            Primary::Span { span, .. } => Some(self.file.as_ref()?.line_col(span.start)?.line),
            Primary::Line { line, .. } => Some(line.get()),
        }
    }

    /// The 1-based column of the primary span or line, where it is known.
    pub fn col(&self) -> Option<u32> {
        match self.primary.as_ref()? {
            Primary::Span { span, .. } => Some(self.file.as_ref()?.line_col(span.start)?.col),
            Primary::Line { col, .. } => col.map(NonZeroU32::get),
        }
    }

    pub fn primary_label(&self) -> Option<&str> {
        match &self.primary {
            Some(Primary::Span { label, .. }) => label.as_deref(),
            _ => None,
        }
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn repair(&self) -> &str {
        &self.repair
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }
}

impl Label {
    /// The label's file, or `None` for the diagnostic's own.
    pub fn file(&self) -> Option<&SourceFile> {
        self.file.as_ref()
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Edit {
    /// The edit's file, or `None` for the diagnostic's own.
    pub fn file(&self) -> Option<&SourceFile> {
        self.file.as_ref()
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn replacement(&self) -> &str {
        &self.replacement
    }

    pub fn applicability(&self) -> Applicability {
        self.applicability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_mpl() -> SourceFile {
        SourceFile::new("src/main.mpl", "fn main() do\n  println(cuont)\nend\n")
    }

    #[test]
    fn a_new_diagnostic_carries_its_catalog_repair_and_severity() {
        let d = Diagnostic::new(
            Code::TypeMismatch,
            "type mismatch: expected `Int`, found `String`",
        );
        assert_eq!(d.code(), Code::TypeMismatch);
        assert_eq!(d.severity(), Severity::Error);
        assert_eq!(d.repair(), Code::TypeMismatch.info().repair);
        assert_eq!(d.file(), None);
        assert_eq!(d.span(), None);
        assert_eq!(d.line(), None);
        assert_eq!(
            Diagnostic::new(Code::RedundantArm, "x").severity(),
            Severity::Warning
        );
    }

    #[test]
    fn a_placed_diagnostic_reports_its_span_line_and_label() {
        let file = main_mpl();
        let d = Diagnostic::in_file(Code::UnboundVariable, &file, "undefined variable `cuont`")
            .at(Span::new(23, 28), "not found in this scope")
            .with_repair("did you mean `count`?")
            .with_edit(Span::new(23, 28), "count", Applicability::MaybeIncorrect);
        assert_eq!(d.file(), Some(&file));
        assert_eq!(d.span(), Some(Span::new(23, 28)));
        assert_eq!(d.line(), Some(2));
        assert_eq!(d.primary_label(), Some("not found in this scope"));
        assert_eq!(d.repair(), "did you mean `count`?");
        let edit = &d.edits()[0];
        assert_eq!(edit.file(), None);
        assert_eq!(edit.span(), Span::new(23, 28));
        assert_eq!(edit.replacement(), "count");
        assert_eq!(edit.applicability(), Applicability::MaybeIncorrect);
    }

    #[test]
    fn an_empty_label_is_no_label() {
        let d = Diagnostic::in_file(Code::TypeMismatch, &main_mpl(), "m").at(Span::new(0, 2), "");
        assert_eq!(d.primary_label(), None);
        assert_eq!(d.span(), Some(Span::new(0, 2)));
    }

    #[test]
    fn labels_and_edits_may_point_into_other_files() {
        let other = SourceFile::new("src/geo.mpl", "pub fn area() = 1\n");
        let d = Diagnostic::in_file(Code::DuplicateDefinition, &main_mpl(), "m")
            .at(Span::new(0, 2), "here")
            .with_label(Span::new(3, 7), "and here")
            .with_label_in(&other, Span::new(7, 11), "first defined here")
            .with_edit_in(
                &other,
                Span::new(7, 11),
                "size",
                Applicability::MachineApplicable,
            )
            .with_note("a note");
        assert_eq!(d.labels()[0].file(), None);
        assert_eq!(d.labels()[0].message(), "and here");
        assert_eq!(d.labels()[1].file(), Some(&other));
        assert_eq!(d.labels()[1].span(), Span::new(7, 11));
        assert_eq!(d.edits()[0].file(), Some(&other));
        assert_eq!(
            d.edits()[0].applicability(),
            Applicability::MachineApplicable
        );
        assert_eq!(d.notes(), ["a note"]);
    }

    #[test]
    fn a_runtime_diagnostic_knows_only_its_line() {
        let d = Diagnostic::at_line(
            Code::TypeMismatch,
            SourceFile::unread("src/main.mpl"),
            12,
            "m",
        );
        assert_eq!(d.line(), Some(12));
        assert_eq!(d.col(), None);
        assert_eq!(d.span(), None);
        assert_eq!(d.primary_label(), None);
        assert_eq!(d.at_column(5).col(), Some(5));
    }

    #[test]
    fn a_span_in_an_unread_file_has_no_line() {
        let d = Diagnostic::in_file(Code::TypeMismatch, &SourceFile::unread("a.mpl"), "m")
            .at(Span::new(0, 1), "x");
        assert_eq!(d.line(), None);
    }

    #[test]
    fn internal_errors_keep_their_detail_as_a_note() {
        let d = Diagnostic::internal("unexpected MIR node `Foo`");
        assert_eq!(d.code(), Code::InternalCompilerError);
        assert_eq!(d.message(), Code::InternalCompilerError.info().title);
        assert_eq!(d.notes(), ["unexpected MIR node `Foo`"]);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "a repair is never empty"))]
    fn an_empty_repair_keeps_the_default() {
        let d = Diagnostic::new(Code::TypeMismatch, "m").with_repair("");
        assert_eq!(d.repair(), Code::TypeMismatch.info().repair);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a span needs the file it indexes")]
    fn a_span_without_a_file_is_a_bug() {
        let _ = Diagnostic::new(Code::TypeMismatch, "m").at(Span::new(0, 1), "x");
    }
}
