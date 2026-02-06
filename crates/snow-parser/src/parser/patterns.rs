//! Pattern parser for Snow.
//!
//! Parses patterns used in match arms, let bindings, and destructuring.
//! Patterns include: wildcard (`_`), identifier, literal, tuple, and struct patterns.

use crate::syntax_kind::SyntaxKind;

use super::{MarkClosed, Parser};

/// Parse a pattern.
///
/// Patterns:
/// - `_` -> WILDCARD_PAT
/// - `42`, `"hello"`, `true`, `false`, `nil` -> LITERAL_PAT
/// - `ident` -> IDENT_PAT
/// - `(p1, p2, ...)` -> TUPLE_PAT
/// - `-42` (negative literal) -> LITERAL_PAT
pub(crate) fn parse_pattern(p: &mut Parser) -> Option<MarkClosed> {
    match p.current() {
        // Wildcard: _
        // The lexer emits `_` as an Ident token, so check the text.
        SyntaxKind::IDENT if p.current_text() == "_" => {
            let m = p.open();
            p.advance(); // _
            Some(p.close(m, SyntaxKind::WILDCARD_PAT))
        }

        // Literal patterns: numbers, booleans, nil
        SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL => {
            let m = p.open();
            p.advance();
            Some(p.close(m, SyntaxKind::LITERAL_PAT))
        }

        SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW | SyntaxKind::NIL_KW => {
            let m = p.open();
            p.advance();
            Some(p.close(m, SyntaxKind::LITERAL_PAT))
        }

        // String literal pattern
        SyntaxKind::STRING_START => {
            let m = p.open();
            // Consume the whole string (STRING_START...STRING_END)
            p.advance(); // STRING_START
            loop {
                match p.current() {
                    SyntaxKind::STRING_CONTENT => p.advance(),
                    SyntaxKind::STRING_END => {
                        p.advance();
                        break;
                    }
                    SyntaxKind::EOF => {
                        p.error("unterminated string in pattern");
                        break;
                    }
                    _ => {
                        // Interpolated strings are not valid patterns
                        p.error("string interpolation not allowed in patterns");
                        break;
                    }
                }
            }
            Some(p.close(m, SyntaxKind::LITERAL_PAT))
        }

        // Negative number literal: -42
        SyntaxKind::MINUS
            if matches!(p.nth(1), SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL) =>
        {
            let m = p.open();
            p.advance(); // -
            p.advance(); // number
            Some(p.close(m, SyntaxKind::LITERAL_PAT))
        }

        // Tuple pattern: (p1, p2, ...)
        SyntaxKind::L_PAREN => {
            let m = p.open();
            p.advance(); // (

            if !p.at(SyntaxKind::R_PAREN) {
                parse_pattern(p);
                while p.eat(SyntaxKind::COMMA) {
                    if p.at(SyntaxKind::R_PAREN) {
                        break; // trailing comma
                    }
                    parse_pattern(p);
                }
            }

            p.expect(SyntaxKind::R_PAREN);
            Some(p.close(m, SyntaxKind::TUPLE_PAT))
        }

        // Identifier pattern
        SyntaxKind::IDENT => {
            let m = p.open();
            p.advance(); // ident
            Some(p.close(m, SyntaxKind::IDENT_PAT))
        }

        _ => {
            p.error("expected pattern");
            None
        }
    }
}
