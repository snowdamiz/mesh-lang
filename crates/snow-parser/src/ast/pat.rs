//! Typed AST nodes for patterns.
//!
//! Covers: WildcardPat, IdentPat, LiteralPat, TuplePat.

use crate::ast::{ast_node, child_token, AstNode};
use crate::cst::{SyntaxNode, SyntaxToken};
use crate::syntax_kind::SyntaxKind;

// ── Pattern enum ─────────────────────────────────────────────────────────

/// Any pattern node.
#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard(WildcardPat),
    Ident(IdentPat),
    Literal(LiteralPat),
    Tuple(TuplePat),
}

impl Pattern {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::WILDCARD_PAT => Some(Pattern::Wildcard(WildcardPat { syntax: node })),
            SyntaxKind::IDENT_PAT => Some(Pattern::Ident(IdentPat { syntax: node })),
            SyntaxKind::LITERAL_PAT => Some(Pattern::Literal(LiteralPat { syntax: node })),
            SyntaxKind::TUPLE_PAT => Some(Pattern::Tuple(TuplePat { syntax: node })),
            _ => None,
        }
    }

    /// Access the underlying syntax node regardless of variant.
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Pattern::Wildcard(n) => &n.syntax,
            Pattern::Ident(n) => &n.syntax,
            Pattern::Literal(n) => &n.syntax,
            Pattern::Tuple(n) => &n.syntax,
        }
    }
}

// ── Wildcard Pattern ─────────────────────────────────────────────────────

ast_node!(WildcardPat, WILDCARD_PAT);

// ── Identifier Pattern ───────────────────────────────────────────────────

ast_node!(IdentPat, IDENT_PAT);

impl IdentPat {
    /// The identifier text.
    pub fn name(&self) -> Option<SyntaxToken> {
        child_token(&self.syntax, SyntaxKind::IDENT)
    }
}

// ── Literal Pattern ──────────────────────────────────────────────────────

ast_node!(LiteralPat, LITERAL_PAT);

impl LiteralPat {
    /// The literal value token.
    pub fn token(&self) -> Option<SyntaxToken> {
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .find(|t| {
                matches!(
                    t.kind(),
                    SyntaxKind::INT_LITERAL
                        | SyntaxKind::FLOAT_LITERAL
                        | SyntaxKind::TRUE_KW
                        | SyntaxKind::FALSE_KW
                        | SyntaxKind::NIL_KW
                        | SyntaxKind::STRING_START
                )
            })
    }
}

// ── Tuple Pattern ────────────────────────────────────────────────────────

ast_node!(TuplePat, TUPLE_PAT);

impl TuplePat {
    /// The sub-patterns in the tuple.
    pub fn patterns(&self) -> impl Iterator<Item = Pattern> + '_ {
        self.syntax.children().filter_map(Pattern::cast)
    }
}
