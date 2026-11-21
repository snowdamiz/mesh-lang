//! Parse error types for the Mesh parser.

use std::borrow::Cow;
use std::fmt;

use mesh_common::span::Span;
use mesh_diagnostics::{Code, Diagnostic, SourceFile};

/// A parse error with location information and optional related span.
///
/// Parse errors carry their catalog code, the primary span where the problem
/// was detected, a human-readable message, an optional related span for
/// context (e.g., "opened here" for unclosed delimiters), and an optional
/// repair for this instance in place of the catalog's.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub code: Code,
    /// Human-readable description of what went wrong.
    pub message: String,
    /// Primary source location where the error was detected.
    pub span: Span,
    /// Optional related location with context message (e.g., "block opened here").
    pub related: Option<(String, Span)>,
    /// `None`: the catalog's repair for `code`.
    pub repair: Option<Cow<'static, str>>,
}

impl ParseError {
    pub fn new(code: Code, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            span,
            related: None,
            repair: None,
        }
    }

    pub fn with_related(mut self, message: impl Into<String>, span: Span) -> Self {
        self.related = Some((message.into(), span));
        self
    }

    pub fn with_repair(mut self, repair: impl Into<Cow<'static, str>>) -> Self {
        self.repair = Some(repair.into());
        self
    }

    /// The diagnostic every tool reports for this error in `file`.
    pub fn to_diagnostic(&self, file: &SourceFile) -> Diagnostic {
        let primary_label = match self.code {
            Code::UnclosedDelimiter => "closing delimiter expected here",
            Code::MissingEnd => "expected `end` here",
            _ => "",
        };
        let mut diagnostic =
            Diagnostic::in_file(self.code, file, &self.message).at(self.span, primary_label);
        if let Some((message, span)) = &self.related {
            diagnostic = diagnostic.with_label(*span, message);
        }
        if let Some(repair) = &self.repair {
            diagnostic = diagnostic.with_repair(repair.clone());
        }
        diagnostic
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parse_error_lowers_with_its_code_related_span_and_repair() {
        let file = SourceFile::new("main.mpl", "fn main() do\n  1\n");
        let error = ParseError::new(
            Code::MissingEnd,
            "expected `end` to close the function body, found the end of the file",
            Span::new(17, 17),
        )
        .with_related("block opened here", Span::new(10, 12))
        .with_repair("Add `end` after the body.");
        let diagnostic = error.to_diagnostic(&file);
        assert_eq!(diagnostic.code(), Code::MissingEnd);
        assert_eq!(diagnostic.message(), error.message);
        assert_eq!(diagnostic.span(), Some(Span::new(17, 17)));
        assert_eq!(diagnostic.primary_label(), Some("expected `end` here"));
        let [label] = diagnostic.labels() else {
            panic!("{diagnostic:?}");
        };
        assert_eq!(label.message(), "block opened here");
        assert_eq!(label.span(), Span::new(10, 12));
        assert_eq!(diagnostic.repair(), "Add `end` after the body.");
    }

    #[test]
    fn without_a_repair_the_catalog_repair_applies() {
        let file = SourceFile::new("main.mpl", "let = 1");
        let diagnostic = ParseError::new(Code::UnexpectedToken, "expected a name", Span::new(4, 5))
            .to_diagnostic(&file);
        assert_eq!(diagnostic.repair(), Code::UnexpectedToken.info().repair);
        assert!(diagnostic.labels().is_empty());
    }

    #[test]
    fn parse_error_display() {
        let err = ParseError::new(Code::UnexpectedToken, "unexpected token", Span::new(0, 1));
        assert_eq!(err.to_string(), "unexpected token");
    }
}
