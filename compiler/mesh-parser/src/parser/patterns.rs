//! Pattern parser for Mesh.
//!
//! Parses patterns used in match arms, let bindings, and destructuring.
//! Patterns include: wildcard (`_`), identifier, literal, tuple, struct,
//! constructor, or-pattern, as-pattern, and cons-pattern.
//!
//! Pattern grammar (precedence, lowest to highest):
//! ```text
//! pattern       = as_pattern
//! as_pattern    = cons_pattern ["as" IDENT]
//! cons_pattern  = or_pattern ("::" cons_pattern)?
//! or_pattern    = primary_pattern ("|" primary_pattern)*
//! primary_pattern = wildcard | literal | tuple | list | constructor | struct | ident
//! struct        = NAME ["." NAME] "{" [field ("," field)* [","]] "}"
//! field         = IDENT [":" pattern]
//! ```

use crate::syntax_kind::SyntaxKind;

use super::{MarkClosed, Parser};

/// Whether a `let` or `for` binding starts with a pattern rather than a
/// name: a tuple `(a, b)` or a struct pattern `Point { x }` / `Geo.Point { x }`.
pub(crate) fn at_destructuring_pattern(p: &Parser) -> bool {
    let struct_start = |at: usize| {
        p.nth(at) == SyntaxKind::L_BRACE
            || (p.nth(at) == SyntaxKind::DOT
                && p.nth(at + 1) == SyntaxKind::IDENT
                && p.nth(at + 2) == SyntaxKind::L_BRACE)
    };
    p.at(SyntaxKind::L_PAREN)
        || (p.at(SyntaxKind::IDENT)
            && p.current_text().starts_with(|c: char| c.is_uppercase())
            && struct_start(1))
}

/// Parse a pattern (top-level entry point).
///
/// Handles the full pattern grammar including or-patterns and as-patterns.
pub(crate) fn parse_pattern(p: &mut Parser) -> Option<MarkClosed> {
    parse_as_pattern(p, false)
}

/// Parse a parameter's pattern. A `::` after it starts the parameter's type
/// unless a list pattern follows (`h :: t`), as after a plain name.
pub(crate) fn parse_param_pattern(p: &mut Parser) -> Option<MarkClosed> {
    parse_as_pattern(p, true)
}

/// Parse an as-pattern: `pattern as name`
///
/// If the inner pattern is followed by an IDENT with text "as", wraps
/// the pattern in an AS_PAT node.
fn parse_as_pattern(p: &mut Parser, param: bool) -> Option<MarkClosed> {
    let inner = parse_cons_pattern(p, param)?;

    // Check for `as` binding (contextual keyword -- "as" is just an IDENT)
    if p.at(SyntaxKind::IDENT) && p.current_text() == "as" {
        let m = p.open_before(inner);
        p.advance(); // "as" ident

        // Parse the binding name
        if p.at(SyntaxKind::IDENT) {
            let name = p.open();
            p.advance();
            p.close(name, SyntaxKind::IDENT_PAT);
        } else {
            p.error("expected identifier after `as`");
        }

        Some(p.close(m, SyntaxKind::AS_PAT))
    } else {
        Some(inner)
    }
}

/// Parse a cons pattern: `head :: tail`
///
/// Cons patterns destructure lists into head element and tail list.
/// Right-associative: `a :: b :: c` parses as `a :: (b :: c)`.
/// Only valid in pattern position -- `::` is not used as an expression operator.
fn parse_cons_pattern(p: &mut Parser, param: bool) -> Option<MarkClosed> {
    let head = parse_or_pattern(p)?;

    if p.at(SyntaxKind::COLON_COLON) && (!param || super::expressions::at_cons_tail(p, 1)) {
        let m = p.open_before(head);
        p.advance(); // ::
        parse_cons_pattern(p, param); // right-associative: tail can be another cons
        Some(p.close(m, SyntaxKind::CONS_PAT))
    } else {
        Some(head)
    }
}

/// Parse an or-pattern: `pattern | pattern | ...`
///
/// If the primary pattern is followed by BAR tokens, wraps all alternatives
/// in an OR_PAT node.
fn parse_or_pattern(p: &mut Parser) -> Option<MarkClosed> {
    let first = parse_primary_pattern(p)?;

    if p.at(SyntaxKind::BAR) {
        let m = p.open_before(first);
        while p.eat(SyntaxKind::BAR) {
            parse_primary_pattern(p);
        }
        Some(p.close(m, SyntaxKind::OR_PAT))
    } else {
        Some(first)
    }
}

/// Parse a primary pattern (no or/as wrapping).
///
/// Primary patterns:
/// - `_` -> WILDCARD_PAT
/// - `42`, `"hello"`, `true`, `false`, `nil` -> LITERAL_PAT
/// - `-42` (negative literal) -> LITERAL_PAT
/// - `(p1, p2, ...)` -> TUPLE_PAT
/// - `Name.Variant(args)` -> CONSTRUCTOR_PAT (qualified)
/// - `Variant(args)` -> CONSTRUCTOR_PAT (unqualified, starts with uppercase + parens)
/// - `Point { x, y: 0 }`, `Geo.Point { x }` -> STRUCT_PAT
/// - `ident` -> IDENT_PAT
fn parse_primary_pattern(p: &mut Parser) -> Option<MarkClosed> {
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

        SyntaxKind::TRUE_KW
        | SyntaxKind::FALSE_KW
        | SyntaxKind::NIL_KW
        | SyntaxKind::ATOM_LITERAL => {
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
                    SyntaxKind::INTERPOLATION_START => {
                        p.error("string interpolation not allowed in patterns");
                        break;
                    }
                    _ => {
                        p.error("unterminated string in pattern");
                        break;
                    }
                }
            }
            Some(p.close(m, SyntaxKind::LITERAL_PAT))
        }

        // Negative number literal: -42
        SyntaxKind::MINUS
            if matches!(
                p.nth(1),
                SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL
            ) =>
        {
            let m = p.open();
            p.advance(); // -
            p.advance(); // number
            Some(p.close(m, SyntaxKind::LITERAL_PAT))
        }

        // List pattern: [] or [p1, p2, ...]
        SyntaxKind::L_BRACKET => {
            let m = p.open();
            parse_pattern_list(p, SyntaxKind::R_BRACKET);
            Some(p.close(m, SyntaxKind::LIST_PAT))
        }

        // Tuple pattern: (p1, p2, ...)
        SyntaxKind::L_PAREN => {
            let m = p.open();
            parse_pattern_list(p, SyntaxKind::R_PAREN);
            Some(p.close(m, SyntaxKind::TUPLE_PAT))
        }

        // Identifier-starting patterns: plain ident, constructor, qualified
        // constructor, struct pattern.
        SyntaxKind::IDENT => {
            let starts_upper = p.current_text().starts_with(|c: char| c.is_uppercase());
            let qualified = p.nth(1) == SyntaxKind::DOT && p.nth(2) == SyntaxKind::IDENT;
            let m = p.open();
            p.advance(); // name
            if qualified {
                p.advance(); // .
                p.advance(); // variant or struct name
            }
            match p.nth(0) {
                // `Point { x, y: 0 }` or `Geo.Point { x }`
                SyntaxKind::L_BRACE if starts_upper || qualified => {
                    parse_struct_pattern_fields(p);
                    Some(p.close(m, SyntaxKind::STRUCT_PAT))
                }
                // `Variant(args)` or `Type.Variant(args)`
                SyntaxKind::L_PAREN if starts_upper || qualified => {
                    parse_pattern_list(p, SyntaxKind::R_PAREN);
                    Some(p.close(m, SyntaxKind::CONSTRUCTOR_PAT))
                }
                // `Type.Variant`
                _ if qualified => Some(p.close(m, SyntaxKind::CONSTRUCTOR_PAT)),
                // A name (a nullary constructor is resolved later).
                _ => Some(p.close(m, SyntaxKind::IDENT_PAT)),
            }
        }

        _ => {
            p.error("expected pattern");
            None
        }
    }
}

/// Parse `open pattern, pattern, ... close`, where `open` is the current
/// token; a trailing comma is allowed.
fn parse_pattern_list(p: &mut Parser, close: SyntaxKind) {
    p.advance(); // ( or [
    if !p.at(close) {
        parse_pattern(p);
        while p.eat(SyntaxKind::COMMA) {
            if p.at(close) {
                break; // trailing comma
            }
            parse_pattern(p);
        }
    }
    p.expect(close);
}

/// Parse a struct pattern's `{ field: pattern, field, ... }`. A field alone
/// binds a variable of its name; fields left out match anything.
fn parse_struct_pattern_fields(p: &mut Parser) {
    p.advance(); // {
    while !p.at(SyntaxKind::R_BRACE) && !p.at(SyntaxKind::EOF) {
        let field = p.open();
        if p.at(SyntaxKind::DOT_DOT) {
            p.error("`..` is not needed: the fields a struct pattern leaves out match anything");
            p.close(field, SyntaxKind::STRUCT_PAT_FIELD);
            return;
        }
        if !p.at(SyntaxKind::IDENT) {
            p.error("expected a field name in the struct pattern");
            p.close(field, SyntaxKind::STRUCT_PAT_FIELD);
            return;
        }
        if p.nth(1) == SyntaxKind::COLON {
            let name = p.open();
            p.advance(); // field name
            p.close(name, SyntaxKind::NAME);
            p.advance(); // :
            parse_pattern(p);
        } else {
            let binding = p.open();
            p.advance(); // field name, bound as a variable
            p.close(binding, SyntaxKind::IDENT_PAT);
        }
        p.close(field, SyntaxKind::STRUCT_PAT_FIELD);
        if p.has_error() {
            return;
        }
        // Fields are separated by commas or, as in a struct literal, by
        // new lines.
        p.eat(SyntaxKind::COMMA);
    }
    p.expect(SyntaxKind::R_BRACE);
}
