//! Pratt expression parser for Snow.
//!
//! Implements operator precedence parsing using binding power tables.
//! Handles all Snow expression forms: literals, identifiers, binary/unary
//! operators, function calls, field access, indexing, pipe, grouping,
//! string interpolation, compound expressions (if/else, case/match,
//! closures, blocks), and basic statements (let bindings, return).

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

        // ── Postfix: function call (possibly with trailing closure) ──
        if current == SyntaxKind::L_PAREN && POSTFIX_BP >= min_bp {
            let m = p.open_before(lhs);
            parse_arg_list(p);
            // Check for trailing closure: `foo() do ... end`
            if p.at(SyntaxKind::DO_KW) {
                parse_trailing_closure(p);
            }
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

        // Compound expression atoms
        SyntaxKind::IF_KW => Some(parse_if_expr(p)),
        SyntaxKind::CASE_KW | SyntaxKind::MATCH_KW => Some(parse_case_expr(p)),
        SyntaxKind::FN_KW => Some(parse_closure(p)),

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

// ── Block Parsing ─────────────────────────────────────────────────────

/// Parse a block body: a sequence of statements separated by newlines
/// or semicolons.
///
/// A block is parsed until END_KW, ELSE_KW, or EOF is encountered.
/// Each statement is either a let binding, return expression, or an
/// expression-statement.
pub(crate) fn parse_block_body(p: &mut Parser) {
    let m = p.open();

    loop {
        // Eat leading newlines/semicolons between statements.
        p.eat_newlines();
        while p.eat(SyntaxKind::SEMICOLON) {
            p.eat_newlines();
        }

        // Check if we've reached a block terminator.
        match p.current() {
            SyntaxKind::END_KW | SyntaxKind::ELSE_KW | SyntaxKind::EOF => break,
            _ => {}
        }

        // Parse a statement or item.
        super::parse_item_or_stmt(p);

        if p.has_error() {
            break;
        }

        // After a statement, expect a separator or block terminator.
        match p.current() {
            SyntaxKind::NEWLINE => {
                p.eat_newlines();
            }
            SyntaxKind::SEMICOLON => {
                // Will be eaten at top of loop.
            }
            SyntaxKind::END_KW | SyntaxKind::ELSE_KW | SyntaxKind::EOF => {
                // Block terminator -- stop.
            }
            _ => {
                // If we're not at a separator or terminator, that's ok --
                // the next iteration will try to parse another statement
                // or hit an error.
            }
        }
    }

    p.close(m, SyntaxKind::BLOCK);
}

// ── Let Binding ───────────────────────────────────────────────────────

/// Parse a let binding: `let pattern [:: Type] = expr`
pub(crate) fn parse_let_binding(p: &mut Parser) {
    let m = p.open();
    p.advance(); // LET_KW

    // Parse the pattern: identifier, tuple, wildcard, etc.
    if p.at(SyntaxKind::L_PAREN) {
        // Tuple pattern: let (a, b) = expr
        super::patterns::parse_pattern(p);
    } else if p.at(SyntaxKind::IDENT) {
        let name = p.open();
        p.advance(); // identifier
        p.close(name, SyntaxKind::NAME);
    } else {
        p.error("expected identifier or pattern after `let`");
    }

    // Optional type annotation: `:: Type`
    if p.at(SyntaxKind::COLON_COLON) {
        let ann = p.open();
        p.advance(); // ::
        super::items::parse_type(p);
        p.close(ann, SyntaxKind::TYPE_ANNOTATION);
    }

    // Expect `=` and initializer.
    p.expect(SyntaxKind::EQ);
    if !p.has_error() {
        expr(p);
    }

    p.close(m, SyntaxKind::LET_BINDING);
}

// ── Return Expression ─────────────────────────────────────────────────

/// Parse a return expression: `return [expr]`
pub(crate) fn parse_return_expr(p: &mut Parser) {
    let m = p.open();
    p.advance(); // RETURN_KW

    // If next token looks like an expression start, parse the value.
    if looks_like_expr_start(p) {
        expr(p);
    }

    p.close(m, SyntaxKind::RETURN_EXPR);
}

/// Whether the current token could start an expression.
/// Used to determine if `return` has a value.
fn looks_like_expr_start(p: &Parser) -> bool {
    match p.current() {
        SyntaxKind::NEWLINE
        | SyntaxKind::END_KW
        | SyntaxKind::ELSE_KW
        | SyntaxKind::EOF
        | SyntaxKind::SEMICOLON => false,
        _ => true,
    }
}

// ── If/Else Expression ────────────────────────────────────────────────

/// Parse an if expression: `if cond do body [else [if ...] body] end`
fn parse_if_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // IF_KW

    // Parse condition.
    expr(p);

    // Expect `do`.
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse then-body.
    parse_block_body(p);

    // Check for else.
    if p.at(SyntaxKind::ELSE_KW) {
        let else_m = p.open();
        p.advance(); // ELSE_KW

        if p.at(SyntaxKind::IF_KW) {
            // else-if chain: parse nested if expression.
            parse_if_expr(p);
        } else {
            // else block.
            parse_block_body(p);
            p.expect(SyntaxKind::END_KW);
        }

        p.close(else_m, SyntaxKind::ELSE_BRANCH);
    } else {
        // No else -- expect `end`.
        if !p.at(SyntaxKind::END_KW) {
            p.error_with_related(
                "expected `end` to close `do` block",
                do_span,
                "`do` block started here",
            );
        } else {
            p.advance(); // END_KW
        }
    }

    p.close(m, SyntaxKind::IF_EXPR)
}

// ── Case/Match Expression ─────────────────────────────────────────────

/// Parse a case/match expression: `case expr do pattern -> body ... end`
fn parse_case_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // CASE_KW or MATCH_KW

    // Parse scrutinee.
    expr(p);

    // Expect `do`.
    let do_span = p.current_span();
    p.expect(SyntaxKind::DO_KW);

    // Parse match arms until END_KW.
    loop {
        p.eat_newlines();

        if p.at(SyntaxKind::END_KW) || p.at(SyntaxKind::EOF) {
            break;
        }

        parse_match_arm(p);

        if p.has_error() {
            break;
        }
    }

    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close `case` block",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance(); // END_KW
    }

    p.close(m, SyntaxKind::CASE_EXPR)
}

/// Parse a single match arm: `pattern [when guard] -> body`
fn parse_match_arm(p: &mut Parser) {
    let m = p.open();

    // Parse pattern.
    super::patterns::parse_pattern(p);

    // Optional `when` guard.
    if p.at(SyntaxKind::WHEN_KW) {
        p.advance(); // WHEN_KW
        expr(p);
    }

    // Expect `->`.
    p.expect(SyntaxKind::ARROW);

    // Parse arm body: a single expression.
    if !p.has_error() {
        expr(p);
    }

    p.close(m, SyntaxKind::MATCH_ARM);
}

// ── Closure Expression ────────────────────────────────────────────────

/// Parse a closure: `fn (params) -> body end` or `fn () -> body end`
fn parse_closure(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // FN_KW

    // Parse parameter list if present.
    if p.at(SyntaxKind::L_PAREN) {
        parse_param_list(p);
    }

    // Expect `->`.
    p.expect(SyntaxKind::ARROW);

    // Parse body: block or single expression.
    // Closures use `fn (params) -> body end`
    // The body may be a multi-statement block ending in `end`.
    if !p.has_error() {
        parse_block_body(p);
    }

    p.expect(SyntaxKind::END_KW);

    p.close(m, SyntaxKind::CLOSURE_EXPR)
}

// ── Parameter List ────────────────────────────────────────────────────

/// Parse a parameter list: `(param, param, ...)`
///
/// Each parameter is `name [:: Type]`.
pub(crate) fn parse_param_list(p: &mut Parser) {
    let m = p.open();
    p.advance(); // L_PAREN

    if !p.at(SyntaxKind::R_PAREN) {
        parse_param(p);
        while p.eat(SyntaxKind::COMMA) {
            if p.at(SyntaxKind::R_PAREN) {
                break; // trailing comma
            }
            parse_param(p);
        }
    }

    p.expect(SyntaxKind::R_PAREN);
    p.close(m, SyntaxKind::PARAM_LIST);
}

/// Parse a single parameter: `name [:: Type]` or `self`
fn parse_param(p: &mut Parser) {
    let m = p.open();

    // Parameter name: IDENT or self keyword.
    if p.at(SyntaxKind::IDENT) || p.at(SyntaxKind::SELF_KW) {
        p.advance();
    } else {
        p.error("expected parameter name");
    }

    // Optional type annotation: `:: Type`
    if p.at(SyntaxKind::COLON_COLON) {
        let ann = p.open();
        p.advance(); // ::
        super::items::parse_type(p);
        p.close(ann, SyntaxKind::TYPE_ANNOTATION);
    }

    p.close(m, SyntaxKind::PARAM);
}

// ── Trailing Closure ──────────────────────────────────────────────────

/// Parse a trailing closure: `do [|params|] body end`
///
/// Attached to the preceding CALL_EXPR as a TRAILING_CLOSURE child.
fn parse_trailing_closure(p: &mut Parser) {
    let m = p.open();
    let do_span = p.current_span();
    p.advance(); // DO_KW

    // Optional closure params: `do |x, y| ... end`
    // The lexer emits bare `|` as Error tokens, so we check for that.
    if p.at(SyntaxKind::ERROR) && p.current_text() == "|" {
        p.advance(); // opening |
        // Parse params between pipes.
        let params = p.open();
        if !(p.at(SyntaxKind::ERROR) && p.current_text() == "|") {
            parse_param(p);
            while p.eat(SyntaxKind::COMMA) {
                if p.at(SyntaxKind::ERROR) && p.current_text() == "|" {
                    break;
                }
                parse_param(p);
            }
        }
        p.close(params, SyntaxKind::PARAM_LIST);
        // Expect closing |
        if p.at(SyntaxKind::ERROR) && p.current_text() == "|" {
            p.advance(); // closing |
        } else {
            p.error("expected `|` to close trailing closure parameters");
        }
    }

    // Parse block body.
    parse_block_body(p);

    if !p.at(SyntaxKind::END_KW) {
        p.error_with_related(
            "expected `end` to close `do` block",
            do_span,
            "`do` block started here",
        );
    } else {
        p.advance(); // END_KW
    }

    p.close(m, SyntaxKind::TRAILING_CLOSURE);
}
