//! Document analysis: parse, type-check, and produce LSP diagnostics.
//!
//! This module bridges the Snow compiler frontend (parser + typeck) with the
//! LSP protocol. It converts byte-offset spans into LSP line/character
//! positions (0-based, UTF-16 code units per the LSP spec) and translates
//! parse errors and type errors into `lsp_types::Diagnostic`.

use rowan::TextRange;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

use snow_typeck::error::{ConstraintOrigin, TypeError};
use snow_typeck::ty::Ty;
use snow_typeck::TypeckResult;

/// The result of analyzing a Snow document.
pub struct AnalysisResult {
    /// LSP diagnostics (parse errors + type errors + warnings).
    pub diagnostics: Vec<Diagnostic>,
    /// The parse result, kept for further queries.
    pub parse: snow_parser::Parse,
    /// The type-check result, kept for hover queries.
    pub typeck: TypeckResult,
}

/// Analyze a Snow document: parse, type-check, and produce diagnostics.
///
/// This is the main entry point called by the LSP server on didOpen/didChange.
pub fn analyze_document(_uri: &str, source: &str) -> AnalysisResult {
    let parse = snow_parser::parse(source);
    let typeck = snow_typeck::check(&parse);

    let mut diagnostics = Vec::new();

    // Convert parse errors to LSP diagnostics.
    for error in parse.errors() {
        let start = offset_to_position(source, error.span.start as usize);
        let end = offset_to_position(source, error.span.end as usize);
        diagnostics.push(Diagnostic {
            range: Range::new(start, end),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("snow".to_string()),
            message: error.message.clone(),
            ..Default::default()
        });
    }

    // Convert type errors to LSP diagnostics.
    for error in &typeck.errors {
        if let Some(diag) = type_error_to_diagnostic(source, error, DiagnosticSeverity::ERROR) {
            diagnostics.push(diag);
        }
    }

    // Convert warnings to LSP diagnostics.
    for warning in &typeck.warnings {
        if let Some(diag) = type_error_to_diagnostic(source, warning, DiagnosticSeverity::WARNING) {
            diagnostics.push(diag);
        }
    }

    AnalysisResult {
        diagnostics,
        parse,
        typeck,
    }
}

/// Convert a byte offset to an LSP Position (0-based line, 0-based UTF-16 character offset).
///
/// The LSP specification requires positions in UTF-16 code units. For ASCII-only
/// sources, UTF-16 offset == byte offset within the line. For non-ASCII sources,
/// we count UTF-16 code units properly.
pub fn offset_to_position(source: &str, offset: usize) -> Position {
    let offset = offset.min(source.len());
    let before = &source[..offset];

    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_text = &source[line_start..offset];

    // Count UTF-16 code units for LSP spec compliance.
    let character: u32 = line_text
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum();

    Position { line, character }
}

/// Look up the inferred type at a given LSP position.
///
/// Searches the typeck result's type map for the smallest range that contains
/// the given byte offset. Returns the type formatted as a string.
pub fn type_at_position(
    source: &str,
    typeck: &TypeckResult,
    position: &Position,
) -> Option<String> {
    let offset = position_to_offset(source, position)?;
    let target_offset = rowan::TextSize::from(offset as u32);

    // Find the smallest range containing this offset.
    let mut best: Option<(TextRange, &Ty)> = None;
    for (range, ty) in &typeck.types {
        if range.contains(target_offset) || range.start() == target_offset {
            match &best {
                Some((best_range, _)) if range.len() < best_range.len() => {
                    best = Some((*range, ty));
                }
                None => {
                    best = Some((*range, ty));
                }
                _ => {}
            }
        }
    }

    best.map(|(_, ty)| format!("{}", ty))
}

/// Convert an LSP Position back to a byte offset in the source.
///
/// Public wrapper for go-to-definition support.
pub fn position_to_offset_pub(source: &str, position: &Position) -> Option<usize> {
    position_to_offset(source, position)
}

/// Convert an LSP Position back to a byte offset in the source.
fn position_to_offset(source: &str, position: &Position) -> Option<usize> {
    let mut current_line = 0u32;
    let mut line_start = 0usize;

    for (i, ch) in source.char_indices() {
        if current_line == position.line {
            // Count UTF-16 code units from line_start to find character offset.
            let line_text = &source[line_start..];
            let mut utf16_offset = 0u32;
            for (byte_idx, c) in line_text.char_indices() {
                if utf16_offset >= position.character {
                    return Some(line_start + byte_idx);
                }
                utf16_offset += c.len_utf16() as u32;
            }
            // Position is at or past end of line.
            return Some(line_start + line_text.find('\n').unwrap_or(line_text.len()));
        }
        if ch == '\n' {
            current_line += 1;
            line_start = i + 1;
        }
    }

    // If we're looking for a position on the last line (no trailing newline).
    if current_line == position.line {
        let line_text = &source[line_start..];
        let mut utf16_offset = 0u32;
        for (byte_idx, c) in line_text.char_indices() {
            if utf16_offset >= position.character {
                return Some(line_start + byte_idx);
            }
            utf16_offset += c.len_utf16() as u32;
        }
        return Some(source.len());
    }

    None
}

/// Extract a TextRange span from a TypeError for diagnostic positioning.
fn type_error_span(error: &TypeError) -> Option<TextRange> {
    match error {
        TypeError::Mismatch { origin, .. } => origin_to_range(origin),
        TypeError::InfiniteType { origin, .. } => origin_to_range(origin),
        TypeError::ArityMismatch { origin, .. } => origin_to_range(origin),
        TypeError::UnboundVariable { span, .. } => Some(*span),
        TypeError::NotAFunction { span, .. } => Some(*span),
        TypeError::TraitNotSatisfied { origin, .. } => origin_to_range(origin),
        TypeError::MissingTraitMethod { .. } => None,
        TypeError::TraitMethodSignatureMismatch { .. } => None,
        TypeError::MissingField { span, .. } => Some(*span),
        TypeError::UnknownField { span, .. } => Some(*span),
        TypeError::NoSuchField { span, .. } => Some(*span),
        TypeError::UnknownVariant { span, .. } => Some(*span),
        TypeError::OrPatternBindingMismatch { span, .. } => Some(*span),
        TypeError::NonExhaustiveMatch { span, .. } => Some(*span),
        TypeError::RedundantArm { span, .. } => Some(*span),
        TypeError::InvalidGuardExpression { span, .. } => Some(*span),
        TypeError::SendTypeMismatch { span, .. } => Some(*span),
        TypeError::SelfOutsideActor { span, .. } => Some(*span),
        TypeError::SpawnNonFunction { span, .. } => Some(*span),
        TypeError::ReceiveOutsideActor { span, .. } => Some(*span),
        TypeError::InvalidChildStart { span, .. } => Some(*span),
        TypeError::InvalidStrategy { span, .. } => Some(*span),
        TypeError::InvalidRestartType { span, .. } => Some(*span),
        TypeError::InvalidShutdownValue { span, .. } => Some(*span),
    }
}

/// Extract a TextRange from a ConstraintOrigin.
fn origin_to_range(origin: &ConstraintOrigin) -> Option<TextRange> {
    match origin {
        ConstraintOrigin::FnArg { call_site, .. } => Some(*call_site),
        ConstraintOrigin::BinOp { op_span } => Some(*op_span),
        ConstraintOrigin::IfBranches { if_span, .. } => Some(*if_span),
        ConstraintOrigin::Annotation { annotation_span } => Some(*annotation_span),
        ConstraintOrigin::Return { return_span, .. } => Some(*return_span),
        ConstraintOrigin::LetBinding { binding_span } => Some(*binding_span),
        ConstraintOrigin::Assignment { lhs_span, .. } => Some(*lhs_span),
        ConstraintOrigin::Builtin => None,
    }
}

/// Convert a TypeError into an LSP Diagnostic.
fn type_error_to_diagnostic(
    source: &str,
    error: &TypeError,
    severity: DiagnosticSeverity,
) -> Option<Diagnostic> {
    let range = type_error_span(error)?;
    let start_offset: usize = range.start().into();
    let end_offset: usize = range.end().into();

    let start = offset_to_position(source, start_offset);
    let end = offset_to_position(source, end_offset);

    Some(Diagnostic {
        range: Range::new(start, end),
        severity: Some(severity),
        source: Some("snow".to_string()),
        message: format!("{}", error),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_valid_source_no_diagnostics() {
        let source = "let x = 42";
        let result = analyze_document("file:///test.snow", source);
        assert!(
            result.diagnostics.is_empty(),
            "Valid source should produce no diagnostics, got: {:?}",
            result.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn analyze_type_error_produces_diagnostic() {
        // Using an undefined variable should produce a type error diagnostic.
        let source = "let x = undefined_var";
        let result = analyze_document("file:///test.snow", source);
        assert!(
            !result.diagnostics.is_empty(),
            "Type error should produce at least one diagnostic"
        );
        let diag = &result.diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diag.source.as_deref(), Some("snow"));
    }

    #[test]
    fn offset_to_position_first_line() {
        let source = "hello world";
        let pos = offset_to_position(source, 0);
        assert_eq!(pos, Position { line: 0, character: 0 });

        let pos = offset_to_position(source, 5);
        assert_eq!(pos, Position { line: 0, character: 5 });
    }

    #[test]
    fn offset_to_position_multiline() {
        let source = "line1\nline2\nline3";
        // 'l' of line2 is at offset 6
        let pos = offset_to_position(source, 6);
        assert_eq!(pos, Position { line: 1, character: 0 });

        // 'l' of line3 is at offset 12
        let pos = offset_to_position(source, 12);
        assert_eq!(pos, Position { line: 2, character: 0 });

        // 'i' of line2 is at offset 7
        let pos = offset_to_position(source, 7);
        assert_eq!(pos, Position { line: 1, character: 1 });
    }

    #[test]
    fn offset_to_position_at_end() {
        let source = "ab\ncd";
        let pos = offset_to_position(source, 5);
        assert_eq!(pos, Position { line: 1, character: 2 });
    }

    #[test]
    fn position_to_offset_roundtrip() {
        let source = "hello\nworld\nfoo";
        for offset in 0..source.len() {
            let pos = offset_to_position(source, offset);
            let back = position_to_offset(source, &pos);
            assert_eq!(
                back,
                Some(offset),
                "Roundtrip failed for offset {} (pos {:?})",
                offset,
                pos
            );
        }
    }
}
