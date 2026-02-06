//! Pratt expression parser for Snow.
//!
//! Implements operator precedence parsing using binding power tables.
//! Handles all Snow expression forms: literals, identifiers, binary/unary
//! operators, function calls, field access, indexing, pipe, grouping,
//! and string interpolation.
//!
//! Compound expressions (if/else, case/match, closures, blocks) are NOT
//! handled here -- they will be added in Plan 03.

use crate::syntax_kind::SyntaxKind;

use super::{MarkClosed, Parser};

// ── Binding Power Tables ───────────────────────────────────────────────

/// Returns (left_bp, right_bp) for infix operators.
///
/// Left < right means left-associative (the usual case).
/// Left > right would mean right-associative.
/// Returns `None` if the token is not an infix operator.
fn infix_binding_power(op: SyntaxKind) -> Option<(u8, u8)> {
    match op {
        // Pipe: lowest expression precedence, left-associative
        SyntaxKind::PIPE => Some((3, 4)),

        // Logical OR: left-associative
        SyntaxKind::OR_KW | SyntaxKind::PIPE_PIPE => Some((5, 6)),

        // Logical AND: left-associative
        SyntaxKind::AND_KW | SyntaxKind::AMP_AMP => Some((7, 8)),

        // Equality: left-associative
        SyntaxKind::EQ_EQ | SyntaxKind::NOT_EQ => Some((9, 10)),

        // Comparison: left-associative
        SyntaxKind::LT | SyntaxKind::GT | SyntaxKind::LT_EQ | SyntaxKind::GT_EQ => Some((11, 12)),

        // Range: left-associative
        SyntaxKind::DOT_DOT => Some((13, 14)),

        // Concatenation: left-associative
        SyntaxKind::DIAMOND | SyntaxKind::PLUS_PLUS => Some((15, 16)),

        // Additive: left-associative
        SyntaxKind::PLUS | SyntaxKind::MINUS => Some((17, 18)),

        // Multiplicative: left-associative
        SyntaxKind::STAR | SyntaxKind::SLASH | SyntaxKind::PERCENT => Some((19, 20)),

        _ => None,
    }
}

/// Returns ((), right_bp) for prefix operators.
///
/// Returns `None` if the token is not a prefix operator.
fn prefix_binding_power(op: SyntaxKind) -> Option<((), u8)> {
    match op {
        SyntaxKind::MINUS => Some(((), 23)),
        SyntaxKind::BANG => Some(((), 23)),
        SyntaxKind::NOT_KW => Some(((), 23)),
        _ => None,
    }
}

/// Postfix operations (call, field access, indexing) have implicit binding
/// power of 25, tighter than all prefix and infix operators.
const POSTFIX_BP: u8 = 25;

// ── Expression Entry Point ─────────────────────────────────────────────

/// Parse an expression at the default (lowest) binding power.
pub(crate) fn expr(p: &mut Parser) {
    expr_bp(p, 0);
}

/// Parse an expression with the given minimum binding power.
///
/// This is the core Pratt parsing loop. It first parses an atom or prefix
/// expression (the LHS), then loops over postfix and infix operators,
/// consuming them as long as their binding power exceeds `min_bp`.
fn expr_bp(p: &mut Parser, min_bp: u8) -> Option<MarkClosed> {
    // Parse the left-hand side (atom or prefix).
    let mut lhs = lhs(p)?;

    // Postfix and infix loop.
    loop {
        if p.has_error() {
            break;
        }

        let current = p.current();

        // ── Postfix: function call ──
        if current == SyntaxKind::L_PAREN && POSTFIX_BP >= min_bp {
            let m = p.open_before(lhs);
            parse_arg_list(p);
            lhs = p.close(m, SyntaxKind::CALL_EXPR);
            continue;
        }

        // ── Postfix: field access ──
        if current == SyntaxKind::DOT && POSTFIX_BP >= min_bp {
            let m = p.open_before(lhs);
            p.advance(); // .
            p.expect(SyntaxKind::IDENT);
            lhs = p.close(m, SyntaxKind::FIELD_ACCESS);
            continue;
        }

        // ── Postfix: index access ──
        if current == SyntaxKind::L_BRACKET && POSTFIX_BP >= min_bp {
            let m = p.open_before(lhs);
            p.advance(); // [
            expr_bp(p, 0);
            p.expect(SyntaxKind::R_BRACKET);
            lhs = p.close(m, SyntaxKind::INDEX_EXPR);
            continue;
        }

        // ── Infix operators ──
        if let Some((l_bp, r_bp)) = infix_binding_power(current) {
            if l_bp < min_bp {
                break;
            }

            let m = p.open_before(lhs);
            p.advance(); // operator

            expr_bp(p, r_bp);

            let kind = if current == SyntaxKind::PIPE {
                SyntaxKind::PIPE_EXPR
            } else {
                SyntaxKind::BINARY_EXPR
            };
            lhs = p.close(m, kind);
            continue;
        }

        // Nothing matched -- exit the loop.
        break;
    }

    Some(lhs)
}

// ── Atom / Prefix Parsing (LHS) ───────────────────────────────────────

/// Parse the left-hand side of an expression: an atom or a prefix operator.
fn lhs(p: &mut Parser) -> Option<MarkClosed> {
    let current = p.current();

    // ── Prefix operators ──
    if let Some(((), r_bp)) = prefix_binding_power(current) {
        let m = p.open();
        p.advance(); // operator
        expr_bp(p, r_bp);
        return Some(p.close(m, SyntaxKind::UNARY_EXPR));
    }

    // ── Atoms ──
    match current {
        // Numeric literals
        SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL => {
            let m = p.open();
            p.advance();
            Some(p.close(m, SyntaxKind::LITERAL))
        }

        // Boolean and nil literals
        SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW | SyntaxKind::NIL_KW => {
            let m = p.open();
            p.advance();
            Some(p.close(m, SyntaxKind::LITERAL))
        }

        // Identifier
        SyntaxKind::IDENT => {
            let m = p.open();
            p.advance();
            Some(p.close(m, SyntaxKind::NAME_REF))
        }

        // String (may contain interpolation)
        SyntaxKind::STRING_START => Some(parse_string_expr(p)),

        // Grouped expression or tuple
        SyntaxKind::L_PAREN => {
            let m = p.open();
            p.advance(); // (

            // Empty parens: unit/empty tuple
            if p.at(SyntaxKind::R_PAREN) {
                p.advance(); // )
                return Some(p.close(m, SyntaxKind::TUPLE_EXPR));
            }

            // Parse the first expression.
            expr_bp(p, 0);

            // If followed by a comma, it's a tuple.
            if p.at(SyntaxKind::COMMA) {
                // Parse remaining tuple elements.
                while p.eat(SyntaxKind::COMMA) {
                    if p.at(SyntaxKind::R_PAREN) {
                        break; // trailing comma
                    }
                    expr_bp(p, 0);
                }
                p.expect(SyntaxKind::R_PAREN);
                Some(p.close(m, SyntaxKind::TUPLE_EXPR))
            } else {
                // Just a grouped expression -- no wrapper node needed,
                // but we keep the parens in the CST via the open/close.
                p.expect(SyntaxKind::R_PAREN);
                Some(p.close(m, SyntaxKind::TUPLE_EXPR))
            }
        }

        // L_BRACKET for list literals could go here in the future.

        _ => {
            p.error("expected expression");
            None
        }
    }
}

// ── Argument List ──────────────────────────────────────────────────────

/// Parse an argument list: `(expr, expr, ...)`.
fn parse_arg_list(p: &mut Parser) {
    let m = p.open();
    p.advance(); // (

    // Parse comma-separated arguments.
    if !p.at(SyntaxKind::R_PAREN) {
        expr_bp(p, 0);
        while p.eat(SyntaxKind::COMMA) {
            if p.at(SyntaxKind::R_PAREN) {
                break; // trailing comma
            }
            expr_bp(p, 0);
        }
    }

    p.expect(SyntaxKind::R_PAREN);
    p.close(m, SyntaxKind::ARG_LIST);
}

// ── String Expression ──────────────────────────────────────────────────

/// Parse a string expression, which may contain interpolation segments.
///
/// String tokens from the lexer look like:
///   STRING_START  STRING_CONTENT?  (INTERPOLATION_START expr INTERPOLATION_END STRING_CONTENT?)*  STRING_END
fn parse_string_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // STRING_START

    loop {
        match p.current() {
            SyntaxKind::STRING_CONTENT => {
                p.advance();
            }
            SyntaxKind::INTERPOLATION_START => {
                let interp = p.open();
                p.advance(); // ${
                expr_bp(p, 0);
                p.expect(SyntaxKind::INTERPOLATION_END);
                p.close(interp, SyntaxKind::INTERPOLATION);
            }
            SyntaxKind::STRING_END => {
                p.advance();
                break;
            }
            SyntaxKind::EOF => {
                p.error("unterminated string");
                break;
            }
            _ => {
                // Unexpected token inside string -- error recovery.
                p.error("unexpected token in string");
                break;
            }
        }
    }

    p.close(m, SyntaxKind::STRING_EXPR)
}
