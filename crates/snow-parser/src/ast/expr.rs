//! Typed AST nodes for expressions.
//!
//! Covers all expression forms: literals, name references, binary/unary
//! operators, function calls, pipe expressions, field access, index access,
//! if/else, case/match, closures, blocks, strings, return, and tuples.

use crate::ast::item::{Block, GuardClause, ParamList};
use crate::ast::{ast_node, child_node, child_nodes, child_token, AstNode};
use crate::cst::{SyntaxNode, SyntaxToken};
use crate::syntax_kind::SyntaxKind;

// ── Expr enum ────────────────────────────────────────────────────────────

/// Any expression node.
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Literal),
    NameRef(NameRef),
    BinaryExpr(BinaryExpr),
    UnaryExpr(UnaryExpr),
    CallExpr(CallExpr),
    PipeExpr(PipeExpr),
    FieldAccess(FieldAccess),
    IndexExpr(IndexExpr),
    IfExpr(IfExpr),
    CaseExpr(CaseExpr),
    ClosureExpr(ClosureExpr),
    Block(Block),
    StringExpr(StringExpr),
    ReturnExpr(ReturnExpr),
    TupleExpr(TupleExpr),
    StructLiteral(StructLiteral),
    MapLiteral(MapLiteral),
    // Actor expression types
    SpawnExpr(SpawnExpr),
    SendExpr(SendExpr),
    ReceiveExpr(ReceiveExpr),
    SelfExpr(SelfExpr),
    LinkExpr(LinkExpr),
}

impl Expr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::LITERAL => Some(Expr::Literal(Literal { syntax: node })),
            SyntaxKind::NAME_REF => Some(Expr::NameRef(NameRef { syntax: node })),
            SyntaxKind::BINARY_EXPR => Some(Expr::BinaryExpr(BinaryExpr { syntax: node })),
            SyntaxKind::UNARY_EXPR => Some(Expr::UnaryExpr(UnaryExpr { syntax: node })),
            SyntaxKind::CALL_EXPR => Some(Expr::CallExpr(CallExpr { syntax: node })),
            SyntaxKind::PIPE_EXPR => Some(Expr::PipeExpr(PipeExpr { syntax: node })),
            SyntaxKind::FIELD_ACCESS => Some(Expr::FieldAccess(FieldAccess { syntax: node })),
            SyntaxKind::INDEX_EXPR => Some(Expr::IndexExpr(IndexExpr { syntax: node })),
            SyntaxKind::IF_EXPR => Some(Expr::IfExpr(IfExpr { syntax: node })),
            SyntaxKind::CASE_EXPR => Some(Expr::CaseExpr(CaseExpr { syntax: node })),
            SyntaxKind::CLOSURE_EXPR => Some(Expr::ClosureExpr(ClosureExpr { syntax: node })),
            SyntaxKind::BLOCK => Some(Expr::Block(Block { syntax: node })),
            SyntaxKind::STRING_EXPR => Some(Expr::StringExpr(StringExpr { syntax: node })),
            SyntaxKind::RETURN_EXPR => Some(Expr::ReturnExpr(ReturnExpr { syntax: node })),
            SyntaxKind::TUPLE_EXPR => Some(Expr::TupleExpr(TupleExpr { syntax: node })),
            SyntaxKind::STRUCT_LITERAL => {
                Some(Expr::StructLiteral(StructLiteral { syntax: node }))
            }
            SyntaxKind::MAP_LITERAL => {
                Some(Expr::MapLiteral(MapLiteral { syntax: node }))
            }
            // Actor expressions
            SyntaxKind::SPAWN_EXPR => Some(Expr::SpawnExpr(SpawnExpr { syntax: node })),
            SyntaxKind::SEND_EXPR => Some(Expr::SendExpr(SendExpr { syntax: node })),
            SyntaxKind::RECEIVE_EXPR => Some(Expr::ReceiveExpr(ReceiveExpr { syntax: node })),
            SyntaxKind::SELF_EXPR => Some(Expr::SelfExpr(SelfExpr { syntax: node })),
            SyntaxKind::LINK_EXPR => Some(Expr::LinkExpr(LinkExpr { syntax: node })),
            _ => None,
        }
    }

    /// Access the underlying syntax node regardless of variant.
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Expr::Literal(n) => &n.syntax,
            Expr::NameRef(n) => &n.syntax,
            Expr::BinaryExpr(n) => &n.syntax,
            Expr::UnaryExpr(n) => &n.syntax,
            Expr::CallExpr(n) => &n.syntax,
            Expr::PipeExpr(n) => &n.syntax,
            Expr::FieldAccess(n) => &n.syntax,
            Expr::IndexExpr(n) => &n.syntax,
            Expr::IfExpr(n) => &n.syntax,
            Expr::CaseExpr(n) => &n.syntax,
            Expr::ClosureExpr(n) => &n.syntax,
            Expr::Block(n) => AstNode::syntax(n),
            Expr::StringExpr(n) => &n.syntax,
            Expr::ReturnExpr(n) => &n.syntax,
            Expr::TupleExpr(n) => &n.syntax,
            Expr::StructLiteral(n) => &n.syntax,
            Expr::MapLiteral(n) => &n.syntax,
            Expr::SpawnExpr(n) => &n.syntax,
            Expr::SendExpr(n) => &n.syntax,
            Expr::ReceiveExpr(n) => &n.syntax,
            Expr::SelfExpr(n) => &n.syntax,
            Expr::LinkExpr(n) => &n.syntax,
        }
    }
}

// ── Literal ──────────────────────────────────────────────────────────────

ast_node!(Literal, LITERAL);

impl Literal {
    /// The literal token (INT_LITERAL, FLOAT_LITERAL, TRUE_KW, FALSE_KW, NIL_KW).
    pub fn token(&self) -> Option<SyntaxToken> {
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .next()
    }
}

// ── Name Reference ───────────────────────────────────────────────────────

ast_node!(NameRef, NAME_REF);

impl NameRef {
    /// The identifier text.
    pub fn text(&self) -> Option<String> {
        child_token(&self.syntax, SyntaxKind::IDENT).map(|t| t.text().to_string())
    }
}

// ── Binary Expression ────────────────────────────────────────────────────

ast_node!(BinaryExpr, BINARY_EXPR);

impl BinaryExpr {
    /// The left-hand side expression.
    pub fn lhs(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The right-hand side expression.
    pub fn rhs(&self) -> Option<Expr> {
        self.syntax.children().filter_map(Expr::cast).nth(1)
    }

    /// The operator token.
    pub fn op(&self) -> Option<SyntaxToken> {
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .find(|t| {
                matches!(
                    t.kind(),
                    SyntaxKind::PLUS
                        | SyntaxKind::MINUS
                        | SyntaxKind::STAR
                        | SyntaxKind::SLASH
                        | SyntaxKind::PERCENT
                        | SyntaxKind::EQ_EQ
                        | SyntaxKind::NOT_EQ
                        | SyntaxKind::LT
                        | SyntaxKind::GT
                        | SyntaxKind::LT_EQ
                        | SyntaxKind::GT_EQ
                        | SyntaxKind::AND_KW
                        | SyntaxKind::OR_KW
                        | SyntaxKind::AMP_AMP
                        | SyntaxKind::PIPE_PIPE
                        | SyntaxKind::DOT_DOT
                        | SyntaxKind::DIAMOND
                        | SyntaxKind::PLUS_PLUS
                )
            })
    }
}

// ── Unary Expression ─────────────────────────────────────────────────────

ast_node!(UnaryExpr, UNARY_EXPR);

impl UnaryExpr {
    /// The operator token.
    pub fn op(&self) -> Option<SyntaxToken> {
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .find(|t| {
                matches!(
                    t.kind(),
                    SyntaxKind::MINUS | SyntaxKind::BANG | SyntaxKind::NOT_KW
                )
            })
    }

    /// The operand expression.
    pub fn operand(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }
}

// ── Call Expression ──────────────────────────────────────────────────────

ast_node!(CallExpr, CALL_EXPR);

impl CallExpr {
    /// The callee expression (function being called).
    pub fn callee(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The argument list.
    pub fn arg_list(&self) -> Option<ArgList> {
        child_node(&self.syntax)
    }
}

ast_node!(ArgList, ARG_LIST);

impl ArgList {
    /// All argument expressions.
    pub fn args(&self) -> impl Iterator<Item = Expr> + '_ {
        self.syntax.children().filter_map(Expr::cast)
    }
}

// ── Pipe Expression ──────────────────────────────────────────────────────

ast_node!(PipeExpr, PIPE_EXPR);

impl PipeExpr {
    /// The left-hand side (input to the pipe).
    pub fn lhs(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The right-hand side (function receiving the piped value).
    pub fn rhs(&self) -> Option<Expr> {
        self.syntax.children().filter_map(Expr::cast).nth(1)
    }
}

// ── Field Access ─────────────────────────────────────────────────────────

ast_node!(FieldAccess, FIELD_ACCESS);

impl FieldAccess {
    /// The expression being accessed.
    pub fn base(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The field name token.
    pub fn field(&self) -> Option<SyntaxToken> {
        // The field IDENT is after the DOT token; find the last IDENT.
        self.syntax
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .filter(|t| t.kind() == SyntaxKind::IDENT)
            .last()
    }
}

// ── Index Expression ─────────────────────────────────────────────────────

ast_node!(IndexExpr, INDEX_EXPR);

impl IndexExpr {
    /// The expression being indexed.
    pub fn base(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The index expression (inside brackets).
    pub fn index(&self) -> Option<Expr> {
        self.syntax.children().filter_map(Expr::cast).nth(1)
    }
}

// ── If Expression ────────────────────────────────────────────────────────

ast_node!(IfExpr, IF_EXPR);

impl IfExpr {
    /// The condition expression.
    pub fn condition(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The then-branch block.
    pub fn then_branch(&self) -> Option<Block> {
        child_node(&self.syntax)
    }

    /// The else branch, if present.
    pub fn else_branch(&self) -> Option<ElseBranch> {
        child_node(&self.syntax)
    }
}

ast_node!(ElseBranch, ELSE_BRANCH);

impl ElseBranch {
    /// The else block (for plain `else ... end`).
    pub fn block(&self) -> Option<Block> {
        child_node(&self.syntax)
    }

    /// The chained `if` expression (for `else if ...`).
    pub fn if_expr(&self) -> Option<IfExpr> {
        child_node(&self.syntax)
    }
}

// ── Case/Match Expression ────────────────────────────────────────────────

ast_node!(CaseExpr, CASE_EXPR);

impl CaseExpr {
    /// The scrutinee expression being matched.
    pub fn scrutinee(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The match arms.
    pub fn arms(&self) -> impl Iterator<Item = MatchArm> + '_ {
        child_nodes(&self.syntax)
    }
}

ast_node!(MatchArm, MATCH_ARM);

impl MatchArm {
    /// The pattern being matched.
    pub fn pattern(&self) -> Option<super::pat::Pattern> {
        self.syntax
            .children()
            .find_map(super::pat::Pattern::cast)
    }

    /// The guard expression (after `when`), if present.
    pub fn guard(&self) -> Option<Expr> {
        // The guard is the second expression child (after the pattern
        // and before the arrow). If there's a when guard, there will be
        // expression nodes between the pattern and the body.
        // We look for the WHEN_KW token, then the next Expr child.
        let has_when = self
            .syntax
            .children_with_tokens()
            .any(|it| it.kind() == SyntaxKind::WHEN_KW);
        if has_when {
            // First expr child is the guard, second is the body.
            self.syntax.children().filter_map(Expr::cast).next()
        } else {
            None
        }
    }

    /// The body expression (after `->`).
    pub fn body(&self) -> Option<Expr> {
        let has_when = self
            .syntax
            .children_with_tokens()
            .any(|it| it.kind() == SyntaxKind::WHEN_KW);
        if has_when {
            // With guard: second expr is body
            self.syntax.children().filter_map(Expr::cast).nth(1)
        } else {
            // Without guard: first expr is body
            self.syntax.children().filter_map(Expr::cast).next()
        }
    }
}

// ── Closure Expression ───────────────────────────────────────────────────

ast_node!(ClosureExpr, CLOSURE_EXPR);

impl ClosureExpr {
    /// The parameter list, if present.
    ///
    /// For single-clause closures, returns the PARAM_LIST child.
    /// For multi-clause closures, returns the first clause's PARAM_LIST
    /// (direct child of CLOSURE_EXPR, not inside a CLOSURE_CLAUSE).
    pub fn param_list(&self) -> Option<ParamList> {
        child_node(&self.syntax)
    }

    /// The closure body block.
    ///
    /// Returns the BLOCK child for both arrow closures (`-> expr`) and
    /// do/end closures (`do ... end`).
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }

    /// The guard clause on the first/only clause, if present.
    pub fn guard(&self) -> Option<GuardClause> {
        child_node(&self.syntax)
    }

    /// Whether this is a multi-clause closure.
    ///
    /// Multi-clause closures have CLOSURE_CLAUSE children for the 2nd+ clauses.
    pub fn is_multi_clause(&self) -> bool {
        self.syntax
            .children()
            .any(|c| c.kind() == SyntaxKind::CLOSURE_CLAUSE)
    }

    /// Returns additional clauses (2nd, 3rd, ...) for multi-clause closures.
    ///
    /// The first clause's data is stored as direct children of CLOSURE_EXPR
    /// (param_list, guard, body). Additional clauses are CLOSURE_CLAUSE children.
    pub fn clauses(&self) -> impl Iterator<Item = ClosureClause> + '_ {
        self.syntax.children().filter_map(ClosureClause::cast)
    }
}

// ── Closure Clause (multi-clause closures) ──────────────────────────────

ast_node!(ClosureClause, CLOSURE_CLAUSE);

impl ClosureClause {
    /// The parameter list for this clause.
    pub fn param_list(&self) -> Option<ParamList> {
        child_node(&self.syntax)
    }

    /// The guard clause, if present.
    pub fn guard(&self) -> Option<GuardClause> {
        child_node(&self.syntax)
    }

    /// The body block.
    pub fn body(&self) -> Option<Block> {
        child_node(&self.syntax)
    }

    /// The body as an expression (first Expr child).
    pub fn body_expr(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }
}

// ── String Expression ────────────────────────────────────────────────────

ast_node!(StringExpr, STRING_EXPR);

// ── Return Expression ────────────────────────────────────────────────────

ast_node!(ReturnExpr, RETURN_EXPR);

impl ReturnExpr {
    /// The return value expression, if present.
    pub fn value(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }
}

// ── Tuple Expression ─────────────────────────────────────────────────────

ast_node!(TupleExpr, TUPLE_EXPR);

impl TupleExpr {
    /// The elements of the tuple.
    pub fn elements(&self) -> impl Iterator<Item = Expr> + '_ {
        self.syntax.children().filter_map(Expr::cast)
    }
}

// ── Struct Literal Expression ───────────────────────────────────────────

ast_node!(StructLiteral, STRUCT_LITERAL);

impl StructLiteral {
    /// The struct name (NAME_REF child).
    pub fn name_ref(&self) -> Option<NameRef> {
        child_node(&self.syntax)
    }

    /// The struct literal fields.
    pub fn fields(&self) -> impl Iterator<Item = StructLiteralField> + '_ {
        child_nodes(&self.syntax)
    }
}

ast_node!(StructLiteralField, STRUCT_LITERAL_FIELD);

impl StructLiteralField {
    /// The field name.
    pub fn name(&self) -> Option<super::item::Name> {
        child_node(&self.syntax)
    }

    /// The field value expression.
    pub fn value(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }
}

// ── Map Literal Expression ───────────────────────────────────────────────

ast_node!(MapLiteral, MAP_LITERAL);

impl MapLiteral {
    /// The map entries.
    pub fn entries(&self) -> impl Iterator<Item = MapEntry> + '_ {
        child_nodes(&self.syntax)
    }
}

ast_node!(MapEntry, MAP_ENTRY);

impl MapEntry {
    /// The key expression (first child expression).
    pub fn key(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The value expression (second child expression, after `=>`).
    pub fn value(&self) -> Option<Expr> {
        self.syntax.children().filter_map(Expr::cast).nth(1)
    }
}

// ── Actor Expression Types ──────────────────────────────────────────────

ast_node!(SpawnExpr, SPAWN_EXPR);

impl SpawnExpr {
    /// The argument list (function reference + initial state args).
    pub fn arg_list(&self) -> Option<ArgList> {
        child_node(&self.syntax)
    }
}

ast_node!(SendExpr, SEND_EXPR);

impl SendExpr {
    /// The argument list (target pid + message).
    pub fn arg_list(&self) -> Option<ArgList> {
        child_node(&self.syntax)
    }
}

ast_node!(ReceiveExpr, RECEIVE_EXPR);

impl ReceiveExpr {
    /// The receive arms.
    pub fn arms(&self) -> impl Iterator<Item = ReceiveArm> + '_ {
        child_nodes(&self.syntax)
    }

    /// The optional after (timeout) clause.
    pub fn after_clause(&self) -> Option<AfterClause> {
        child_node(&self.syntax)
    }
}

ast_node!(ReceiveArm, RECEIVE_ARM);

impl ReceiveArm {
    /// The pattern being matched.
    pub fn pattern(&self) -> Option<super::pat::Pattern> {
        self.syntax
            .children()
            .find_map(super::pat::Pattern::cast)
    }

    /// The body expression (after `->`).
    pub fn body(&self) -> Option<Expr> {
        let has_when = self
            .syntax
            .children_with_tokens()
            .any(|it| it.kind() == SyntaxKind::WHEN_KW);
        if has_when {
            self.syntax.children().filter_map(Expr::cast).nth(1)
        } else {
            self.syntax.children().filter_map(Expr::cast).next()
        }
    }
}

ast_node!(AfterClause, AFTER_CLAUSE);

impl AfterClause {
    /// The timeout expression.
    pub fn timeout(&self) -> Option<Expr> {
        self.syntax.children().find_map(Expr::cast)
    }

    /// The timeout body expression.
    pub fn body(&self) -> Option<Expr> {
        self.syntax.children().filter_map(Expr::cast).nth(1)
    }
}

ast_node!(SelfExpr, SELF_EXPR);

ast_node!(LinkExpr, LINK_EXPR);

impl LinkExpr {
    /// The argument list (target pid).
    pub fn arg_list(&self) -> Option<ArgList> {
        child_node(&self.syntax)
    }
}
