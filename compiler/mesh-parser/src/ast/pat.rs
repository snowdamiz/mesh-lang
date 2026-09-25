//! Typed AST nodes for patterns.
//!
//! Covers: WildcardPat, IdentPat, LiteralPat, TuplePat, ConstructorPat, StructPat,
//! OrPat, AsPat, ConsPat, ListPat.

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
    Constructor(ConstructorPat),
    Struct(StructPat),
    Or(OrPat),
    As(AsPat),
    Cons(ConsPat),
    List(ListPat),
}

impl Pattern {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::WILDCARD_PAT => Some(Pattern::Wildcard(WildcardPat { syntax: node })),
            SyntaxKind::IDENT_PAT => Some(Pattern::Ident(IdentPat { syntax: node })),
            SyntaxKind::LITERAL_PAT => Some(Pattern::Literal(LiteralPat { syntax: node })),
            SyntaxKind::TUPLE_PAT => {
                // `(pattern)` with no comma is a grouping, as in `h :: (h2 :: t)`;
                // there are no one-element tuples.
                let has_comma = node
                    .children_with_tokens()
                    .any(|element| element.kind() == SyntaxKind::COMMA);
                let mut inner = node.children().filter_map(Pattern::cast);
                match (inner.next(), has_comma) {
                    (Some(only), false) if inner.next().is_none() => Some(only),
                    _ => Some(Pattern::Tuple(TuplePat { syntax: node })),
                }
            }
            SyntaxKind::CONSTRUCTOR_PAT => {
                Some(Pattern::Constructor(ConstructorPat { syntax: node }))
            }
            SyntaxKind::STRUCT_PAT => Some(Pattern::Struct(StructPat { syntax: node })),
            SyntaxKind::OR_PAT => Some(Pattern::Or(OrPat { syntax: node })),
            SyntaxKind::AS_PAT => Some(Pattern::As(AsPat { syntax: node })),
            SyntaxKind::CONS_PAT => Some(Pattern::Cons(ConsPat { syntax: node })),
            SyntaxKind::LIST_PAT => Some(Pattern::List(ListPat { syntax: node })),
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
            Pattern::Constructor(n) => &n.syntax,
            Pattern::Struct(n) => &n.syntax,
            Pattern::Or(n) => &n.syntax,
            Pattern::As(n) => &n.syntax,
            Pattern::Cons(n) => &n.syntax,
            Pattern::List(n) => &n.syntax,
        }
    }

    /// The patterns directly inside this one: tuple and list elements,
    /// constructor arguments, struct field patterns, every alternative of an
    /// or-pattern, a cons pattern's head and tail, an as-pattern's inner
    /// pattern (not its name).
    pub fn sub_patterns(&self) -> Vec<Pattern> {
        match self {
            Pattern::Wildcard(_) | Pattern::Ident(_) | Pattern::Literal(_) => Vec::new(),
            Pattern::Struct(s) => s.fields().filter_map(|f| f.pattern()).collect(),
            Pattern::As(a) => a.pattern().into_iter().collect(),
            _ => self.syntax().children().filter_map(Pattern::cast).collect(),
        }
    }

    /// The name tokens this pattern binds, in order: lowercase identifier
    /// patterns (an uppercase one is a constructor) and `as` names. The
    /// alternatives of an or-pattern bind the same names; the first one's
    /// are returned.
    pub fn binders(&self) -> Vec<SyntaxToken> {
        let mut out = Vec::new();
        self.collect_binders(&mut out);
        out
    }

    fn collect_binders(&self, out: &mut Vec<SyntaxToken>) {
        match self {
            Pattern::Ident(ident) => out.extend(
                ident
                    .name()
                    .filter(|name| !name.text().starts_with(|c: char| c.is_uppercase())),
            ),
            Pattern::Or(or) => {
                if let Some(first) = or.alternatives().next() {
                    first.collect_binders(out);
                }
            }
            Pattern::As(as_pat) => {
                if let Some(inner) = as_pat.pattern() {
                    inner.collect_binders(out);
                }
                out.extend(as_pat.binding_name());
            }
            _ => self
                .sub_patterns()
                .iter()
                .for_each(|sub| sub.collect_binders(out)),
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
                        | SyntaxKind::ATOM_LITERAL
                )
            })
    }

    /// Whether the literal is negated: `-1` is a MINUS and then the `1`
    /// that [`token`](Self::token) returns.
    pub fn is_negative(&self) -> bool {
        self.syntax
            .children_with_tokens()
            .any(|it| it.kind() == SyntaxKind::MINUS)
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

// ── Constructor Pattern ──────────────────────────────────────────────────

ast_node!(ConstructorPat, CONSTRUCTOR_PAT);

impl ConstructorPat {
    /// Whether this is a qualified constructor (e.g., `Shape.Circle` vs `Some`).
    ///
    /// Qualified constructors have a DOT token between type name and variant name.
    pub fn is_qualified(&self) -> bool {
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .any(|t| t.kind() == SyntaxKind::DOT)
    }

    /// The type qualifier name (e.g., "Shape" in `Shape.Circle`).
    ///
    /// Returns `None` for unqualified constructors like `Some(x)`.
    pub fn type_name(&self) -> Option<SyntaxToken> {
        if self.is_qualified() {
            // First IDENT is the type name
            self.syntax
                .children_with_tokens()
                .filter_map(|it| it.into_token())
                .find(|t| t.kind() == SyntaxKind::IDENT)
        } else {
            None
        }
    }

    /// The variant name (e.g., "Circle" in `Shape.Circle` or `Circle` in `Circle(r)`).
    ///
    /// For qualified constructors, this is the IDENT after the DOT.
    /// For unqualified constructors, this is the first (and only) IDENT.
    pub fn variant_name(&self) -> Option<SyntaxToken> {
        let idents: Vec<_> = self
            .syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .filter(|t| t.kind() == SyntaxKind::IDENT)
            .collect();

        if self.is_qualified() {
            // Second IDENT is the variant name
            idents.into_iter().nth(1)
        } else {
            // First IDENT is the variant name
            idents.into_iter().next()
        }
    }

    /// The sub-patterns inside the constructor's parentheses.
    ///
    /// For `Circle(r)` this yields the `r` pattern.
    /// For nullary constructors like `Shape.Point`, this is empty.
    pub fn fields(&self) -> impl Iterator<Item = Pattern> + '_ {
        self.syntax.children().filter_map(Pattern::cast)
    }
}

// ── Struct Pattern ──────────────────────────────────────────────────

ast_node!(StructPat, STRUCT_PAT);

impl StructPat {
    /// The struct's name: `Point` in `Point { x }` and in `Geo.Point { x }`.
    pub fn type_name(&self) -> Option<SyntaxToken> {
        self.name_tokens().last()
    }

    /// The module qualifier: `Geo` in `Geo.Point { x }`.
    pub fn qualifier(&self) -> Option<SyntaxToken> {
        let names: Vec<_> = self.name_tokens().collect();
        (names.len() > 1).then(|| names[0].clone())
    }

    fn name_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .filter(|t| t.kind() == SyntaxKind::IDENT)
    }

    /// The fields the pattern names; the struct's other fields match anything.
    pub fn fields(&self) -> impl Iterator<Item = StructPatField> + '_ {
        self.syntax.children().filter_map(StructPatField::cast)
    }
}

ast_node!(StructPatField, STRUCT_PAT_FIELD);

impl StructPatField {
    /// The field's name: `y` in `y: 0`, and `x` in `x` alone.
    pub fn name(&self) -> Option<SyntaxToken> {
        match self
            .syntax
            .children()
            .find(|n| n.kind() == SyntaxKind::NAME)
        {
            Some(name) => child_token(&name, SyntaxKind::IDENT),
            None => self.pattern().and_then(|pattern| match pattern {
                Pattern::Ident(ident) => ident.name(),
                _ => None,
            }),
        }
    }

    /// The field's pattern: `0` in `y: 0`; `x` alone is the pattern `x`.
    pub fn pattern(&self) -> Option<Pattern> {
        self.syntax.children().find_map(Pattern::cast)
    }
}

// ── Or Pattern ──────────────────────────────────────────────────────────

ast_node!(OrPat, OR_PAT);

impl OrPat {
    /// The alternative patterns in this or-pattern.
    ///
    /// For `Circle(_) | Point` this yields both `Circle(_)` and `Point`.
    pub fn alternatives(&self) -> impl Iterator<Item = Pattern> + '_ {
        self.syntax.children().filter_map(Pattern::cast)
    }
}

// ── Cons Pattern ────────────────────────────────────────────────────────

// ── List Pattern ─────────────────────────────────────────────────────────

ast_node!(ListPat, LIST_PAT);

impl ListPat {
    /// The element patterns: `[]` has none, `[a, b]` has two. The pattern
    /// matches a list of exactly that many elements.
    pub fn patterns(&self) -> impl Iterator<Item = Pattern> + '_ {
        self.syntax.children().filter_map(Pattern::cast)
    }
}

ast_node!(ConsPat, CONS_PAT);

impl ConsPat {
    /// The head pattern (first element).
    ///
    /// For `h :: t`, this is `h`.
    pub fn head(&self) -> Option<Pattern> {
        self.syntax.children().find_map(Pattern::cast)
    }

    /// The tail pattern (remaining list).
    ///
    /// For `h :: t`, this is `t`.
    pub fn tail(&self) -> Option<Pattern> {
        self.syntax.children().filter_map(Pattern::cast).nth(1)
    }
}

// ── As Pattern ──────────────────────────────────────────────────────────

ast_node!(AsPat, AS_PAT);

impl AsPat {
    /// The inner pattern (before `as`).
    ///
    /// For `Circle(r) as c`, this is `Circle(r)`.
    pub fn pattern(&self) -> Option<Pattern> {
        self.syntax.children().find_map(Pattern::cast)
    }

    /// The binding name after `as`.
    ///
    /// For `Circle(r) as c`, this returns the token "c".
    /// The binding is stored as an IDENT_PAT child; we get its IDENT token.
    pub fn binding_name(&self) -> Option<SyntaxToken> {
        // The binding is the last IDENT_PAT child (after the inner pattern)
        let binding_pat = self.syntax.children().filter_map(Pattern::cast).last()?;
        match binding_pat {
            Pattern::Ident(ident_pat) => ident_pat.name(),
            _ => None,
        }
    }
}
