//! CST-to-FormatIR walker for Snow source code.
//!
//! This module walks the rowan CST produced by `snow-parser` and converts it
//! into a `FormatIR` document tree. The walker processes all tokens including
//! trivia (comments, newlines) to preserve them in the formatted output.
//!
//! The walker dispatches on `SyntaxKind` for each CST node, producing
//! appropriate `FormatIR` structures for indentation, grouping, and line
//! breaking.
//!
//! NOTE: `ir::space()` means "space in flat mode, newline+indent in break mode".
//! Since the root context is always break mode, we use `sp()` (literal text " ")
//! for unconditional spaces, and reserve `ir::space()` for inside `Group` nodes.

use rowan::NodeOrToken;
use snow_parser::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::ir::{self, FormatIR};

/// Literal space text -- always emits " " regardless of mode.
/// Use this for unconditional spaces (e.g., between `fn` and name).
/// Use `ir::space()` only inside `Group` nodes where break behavior is desired.
fn sp() -> FormatIR {
    ir::text(" ")
}

/// Walk a CST node and produce a FormatIR document tree.
///
/// This is the main entry point for converting a parsed Snow syntax tree
/// into the format IR that the printer can render.
pub fn walk_node(node: &SyntaxNode) -> FormatIR {
    let kind = node.kind();
    match kind {
        SyntaxKind::SOURCE_FILE => walk_source_file(node),
        SyntaxKind::FN_DEF => walk_fn_def(node),
        SyntaxKind::LET_BINDING => walk_let_binding(node),
        SyntaxKind::IF_EXPR => walk_if_expr(node),
        SyntaxKind::CASE_EXPR => walk_case_expr(node),
        SyntaxKind::MATCH_ARM => walk_match_arm(node),
        SyntaxKind::BINARY_EXPR => walk_binary_expr(node),
        SyntaxKind::UNARY_EXPR => walk_unary_expr(node),
        SyntaxKind::CALL_EXPR => walk_call_expr(node),
        SyntaxKind::PIPE_EXPR => walk_pipe_expr(node),
        SyntaxKind::BLOCK => walk_block(node),
        SyntaxKind::PARAM_LIST => walk_paren_list(node),
        SyntaxKind::ARG_LIST => walk_paren_list(node),
        SyntaxKind::MODULE_DEF => walk_block_def(node),
        SyntaxKind::STRUCT_DEF => walk_struct_def(node),
        SyntaxKind::STRUCT_FIELD => walk_struct_field(node),
        SyntaxKind::CLOSURE_EXPR => walk_closure_expr(node),
        SyntaxKind::RETURN_EXPR => walk_return_expr(node),
        SyntaxKind::IMPORT_DECL => walk_import_decl(node),
        SyntaxKind::FROM_IMPORT_DECL => walk_from_import_decl(node),
        SyntaxKind::STRING_EXPR => walk_string_expr(node),
        SyntaxKind::TUPLE_EXPR => walk_paren_list(node),
        SyntaxKind::FIELD_ACCESS => walk_field_access(node),
        SyntaxKind::INDEX_EXPR => walk_index_expr(node),
        SyntaxKind::ELSE_BRANCH => walk_else_branch(node),
        SyntaxKind::INTERFACE_DEF => walk_block_def(node),
        SyntaxKind::IMPL_DEF => walk_impl_def(node),
        SyntaxKind::TYPE_ALIAS_DEF => walk_type_alias_def(node),
        SyntaxKind::SUM_TYPE_DEF => walk_block_def(node),
        SyntaxKind::VARIANT_DEF => walk_variant_def(node),
        SyntaxKind::ACTOR_DEF => walk_block_def(node),
        SyntaxKind::SERVICE_DEF => walk_block_def(node),
        SyntaxKind::SUPERVISOR_DEF => walk_block_def(node),
        SyntaxKind::RECEIVE_EXPR => walk_receive_expr(node),
        SyntaxKind::RECEIVE_ARM => walk_match_arm(node),
        SyntaxKind::SPAWN_EXPR => walk_spawn_send_link(node),
        SyntaxKind::SEND_EXPR => walk_spawn_send_link(node),
        SyntaxKind::LINK_EXPR => walk_spawn_send_link(node),
        SyntaxKind::SELF_EXPR => walk_self_expr(node),
        SyntaxKind::CALL_HANDLER => walk_call_handler(node),
        SyntaxKind::CAST_HANDLER => walk_cast_handler(node),
        SyntaxKind::TERMINATE_CLAUSE => walk_terminate_clause(node),
        SyntaxKind::CHILD_SPEC_DEF => walk_block_def(node),
        SyntaxKind::STRUCT_LITERAL => walk_struct_literal(node),
        // Simple leaf-like nodes: just emit their tokens inline.
        SyntaxKind::LITERAL
        | SyntaxKind::NAME
        | SyntaxKind::NAME_REF
        | SyntaxKind::PATH
        | SyntaxKind::TYPE_ANNOTATION
        | SyntaxKind::VISIBILITY
        | SyntaxKind::WILDCARD_PAT
        | SyntaxKind::IDENT_PAT
        | SyntaxKind::LITERAL_PAT
        | SyntaxKind::TUPLE_PAT
        | SyntaxKind::STRUCT_PAT
        | SyntaxKind::CONSTRUCTOR_PAT
        | SyntaxKind::OR_PAT
        | SyntaxKind::AS_PAT
        | SyntaxKind::GUARD_CLAUSE
        | SyntaxKind::FN_EXPR_BODY
        | SyntaxKind::INTERPOLATION
        | SyntaxKind::TRAILING_CLOSURE
        | SyntaxKind::IMPORT_LIST
        | SyntaxKind::TYPE_PARAM_LIST
        | SyntaxKind::GENERIC_PARAM_LIST
        | SyntaxKind::GENERIC_ARG_LIST
        | SyntaxKind::WHERE_CLAUSE
        | SyntaxKind::TRAIT_BOUND
        | SyntaxKind::OPTION_TYPE
        | SyntaxKind::RESULT_TYPE
        | SyntaxKind::INTERFACE_METHOD
        | SyntaxKind::VARIANT_FIELD
        | SyntaxKind::AFTER_CLAUSE
        | SyntaxKind::STRATEGY_CLAUSE
        | SyntaxKind::RESTART_LIMIT
        | SyntaxKind::SECONDS_LIMIT
        | SyntaxKind::STRUCT_LITERAL_FIELD
        | SyntaxKind::PARAM => walk_tokens_inline(node),
        // Fallback: emit tokens with spaces.
        _ => walk_tokens_inline(node),
    }
}

// ── Source file (top-level) ────────────────────────────────────────────

fn walk_source_file(node: &SyntaxNode) -> FormatIR {
    let mut items: Vec<FormatIR> = Vec::new();
    let mut pending_comment: Option<FormatIR> = None;

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                let kind = tok.kind();
                match kind {
                    SyntaxKind::EOF => {}
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT | SyntaxKind::MODULE_DOC_COMMENT => {
                        if let Some(pc) = pending_comment.take() {
                            items.push(pc);
                        }
                        pending_comment = Some(ir::text(tok.text()));
                    }
                    _ => {}
                }
            }
            NodeOrToken::Node(n) => {
                if let Some(pc) = pending_comment.take() {
                    items.push(pc);
                }
                items.push(walk_node(&n));
            }
        }
    }

    if let Some(pc) = pending_comment.take() {
        items.push(pc);
    }

    // Separate top-level items with blank lines (hardline + hardline).
    if items.is_empty() {
        FormatIR::Empty
    } else {
        let mut parts = Vec::new();
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                parts.push(ir::hardline());
                parts.push(ir::hardline());
            }
            parts.push(item);
        }
        ir::concat(parts)
    }
}

// ── Function definition ──────────────────────────────────────────────

fn walk_fn_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut has_block = false;
    let mut has_expr_body = false;

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::FN_KW | SyntaxKind::DEF_KW => {
                        parts.push(ir::text(tok.text()));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                        has_block = true;
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::EQ if !has_block => {
                        // `= expr` body form -- the EQ token before FN_EXPR_BODY.
                        // Don't emit here; it's handled with the FN_EXPR_BODY node.
                    }
                    SyntaxKind::WHEN_KW => {
                        parts.push(sp());
                        parts.push(ir::text("when"));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT => {
                        parts.push(sp());
                        parts.push(ir::text(tok.text()));
                    }
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::VISIBILITY => {
                        parts.push(walk_node(&n));
                        parts.push(sp());
                    }
                    SyntaxKind::NAME => {
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::PARAM_LIST => {
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::TYPE_ANNOTATION => {
                        parts.push(sp());
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::WHERE_CLAUSE => {
                        parts.push(sp());
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::GENERIC_PARAM_LIST => {
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::GUARD_CLAUSE => {
                        // Guard clause: emit space + walk tokens inline.
                        parts.push(sp());
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::FN_EXPR_BODY => {
                        // `= expr` body form.
                        parts.push(sp());
                        parts.push(ir::text("="));
                        parts.push(sp());
                        // Walk the expression child of FN_EXPR_BODY.
                        for body_child in n.children() {
                            parts.push(walk_node(&body_child));
                        }
                        has_expr_body = true;
                    }
                    SyntaxKind::BLOCK if has_block => {
                        let body = walk_block_body(&n);
                        parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    _ => {
                        if !has_expr_body {
                            parts.push(walk_node(&n));
                        }
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

// ── Let binding ────────────────────────────────────────────────────────

fn walk_let_binding(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::LET_KW => {
                        parts.push(ir::text("let"));
                        parts.push(sp());
                    }
                    SyntaxKind::EQ => {
                        parts.push(sp());
                        parts.push(ir::text("="));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::TYPE_ANNOTATION => {
                        parts.push(sp());
                        parts.push(walk_node(&n));
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::group(ir::concat(parts))
}

// ── If expression ────────────────────────────────────────────────────

fn walk_if_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::IF_KW => {
                        parts.push(ir::text("if"));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::END_KW => {
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        let body = walk_block_body(&n);
                        parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                    }
                    SyntaxKind::ELSE_BRANCH => {
                        parts.push(walk_node(&n));
                    }
                    _ => {
                        // Condition expression.
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

fn walk_else_branch(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::ELSE_KW => {
                        parts.push(ir::hardline());
                        parts.push(ir::text("else"));
                    }
                    SyntaxKind::END_KW => {
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        let body = walk_block_body(&n);
                        parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                    }
                    SyntaxKind::IF_EXPR => {
                        parts.push(sp());
                        parts.push(walk_node(&n));
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

// ── Case/match expression ────────────────────────────────────────────

fn walk_case_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut arms: Vec<FormatIR> = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::CASE_KW => {
                        parts.push(ir::text("case"));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT => {
                        arms.push(ir::text(tok.text()));
                    }
                    _ => {}
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::MATCH_ARM => {
                        arms.push(walk_node(&n));
                    }
                    _ => {
                        // Scrutinee expression.
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    if !arms.is_empty() {
        let mut arm_parts = Vec::new();
        for arm in arms {
            arm_parts.push(ir::hardline());
            arm_parts.push(arm);
        }
        parts.push(ir::indent(ir::concat(arm_parts)));
    }

    parts.push(ir::hardline());
    parts.push(ir::text("end"));

    ir::concat(parts)
}

fn walk_match_arm(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::ARROW | SyntaxKind::FAT_ARROW => {
                        parts.push(sp());
                        parts.push(ir::text(tok.text()));
                        parts.push(sp());
                    }
                    SyntaxKind::WHEN_KW => {
                        parts.push(sp());
                        parts.push(ir::text("when"));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT => {
                        parts.push(sp());
                        parts.push(ir::text(tok.text()));
                    }
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::GUARD_CLAUSE => {
                        parts.push(sp());
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::BLOCK => {
                        let body = walk_block_body(&n);
                        parts.push(body);
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

// ── Binary expression ────────────────────────────────────────────────

fn walk_binary_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::NEWLINE => {}
                    _ if is_operator(tok.kind()) => {
                        parts.push(sp());
                        parts.push(ir::text(tok.text()));
                        parts.push(sp());
                    }
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Unary expression ────────────────────────────────────────────────

fn walk_unary_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::NOT_KW => {
                        parts.push(ir::text("not"));
                        parts.push(sp());
                    }
                    SyntaxKind::MINUS => {
                        parts.push(ir::text("-"));
                    }
                    SyntaxKind::BANG => {
                        parts.push(ir::text("!"));
                    }
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Pipe expression ────────────────────────────────────────────────

fn walk_pipe_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::PIPE => {
                        parts.push(ir::hardline());
                        parts.push(ir::text("|>"));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::indent(ir::concat(parts))
}

// ── Call expression ──────────────────────────────────────────────────

fn walk_call_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Block ─────────────────────────────────────────────────────────

fn walk_block(node: &SyntaxNode) -> FormatIR {
    walk_block_body(node)
}

/// Walk the children of a BLOCK node, producing statements separated by hardlines.
fn walk_block_body(node: &SyntaxNode) -> FormatIR {
    let mut stmts: Vec<FormatIR> = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT | SyntaxKind::MODULE_DOC_COMMENT => {
                        stmts.push(ir::text(tok.text()));
                    }
                    _ => {
                        stmts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                stmts.push(walk_node(&n));
            }
        }
    }

    if stmts.is_empty() {
        FormatIR::Empty
    } else {
        let mut parts = Vec::new();
        for (i, stmt) in stmts.into_iter().enumerate() {
            if i > 0 {
                parts.push(ir::hardline());
            }
            parts.push(stmt);
        }
        ir::concat(parts)
    }
}

// ── Parenthesized lists (param_list, arg_list, tuple_expr) ───────────

fn walk_paren_list(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::L_PAREN => {
                        parts.push(ir::text("("));
                    }
                    SyntaxKind::R_PAREN => {
                        parts.push(ir::text(")"));
                    }
                    SyntaxKind::COMMA => {
                        parts.push(ir::text(","));
                        parts.push(ir::space());
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::group(ir::concat(parts))
}

// ── Block-structured definitions (module, actor, service, etc.) ──────

fn walk_block_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut past_do = false;
    let mut inner_items: Vec<FormatIR> = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::MODULE_KW
                    | SyntaxKind::ACTOR_KW
                    | SyntaxKind::SERVICE_KW
                    | SyntaxKind::SUPERVISOR_KW
                    | SyntaxKind::INTERFACE_KW
                    | SyntaxKind::TYPE_KW => {
                        parts.push(ir::text(tok.text()));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                        past_do = true;
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT | SyntaxKind::MODULE_DOC_COMMENT => {
                        if past_do {
                            inner_items.push(ir::text(tok.text()));
                        } else {
                            parts.push(sp());
                            parts.push(ir::text(tok.text()));
                        }
                    }
                    _ => {
                        if past_do {
                            inner_items.push(ir::text(tok.text()));
                        } else {
                            add_token_with_context(&tok, &mut parts);
                        }
                    }
                }
            }
            NodeOrToken::Node(n) => {
                if !past_do {
                    match n.kind() {
                        SyntaxKind::NAME | SyntaxKind::GENERIC_PARAM_LIST => {
                            parts.push(walk_node(&n));
                        }
                        _ => {
                            parts.push(walk_node(&n));
                        }
                    }
                } else if n.kind() == SyntaxKind::BLOCK {
                    for block_child in n.children_with_tokens() {
                        match block_child {
                            NodeOrToken::Token(t) => {
                                match t.kind() {
                                    SyntaxKind::NEWLINE => {}
                                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT | SyntaxKind::MODULE_DOC_COMMENT => {
                                        inner_items.push(ir::text(t.text()));
                                    }
                                    _ => {}
                                }
                            }
                            NodeOrToken::Node(bn) => {
                                inner_items.push(walk_node(&bn));
                            }
                        }
                    }
                } else {
                    inner_items.push(walk_node(&n));
                }
            }
        }
    }

    if !inner_items.is_empty() {
        let mut body_parts = Vec::new();
        for (i, item) in inner_items.into_iter().enumerate() {
            if i > 0 {
                body_parts.push(ir::hardline());
            }
            body_parts.push(ir::hardline());
            body_parts.push(item);
        }
        parts.push(ir::indent(ir::concat(body_parts)));
    }
    parts.push(ir::hardline());
    parts.push(ir::text("end"));

    ir::concat(parts)
}

// ── Struct definition ─────────────────────────────────────────────────

fn walk_struct_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut fields: Vec<FormatIR> = Vec::new();
    let mut in_body = false;

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::STRUCT_KW => {
                        parts.push(ir::text("struct"));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                        in_body = true;
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT => {
                        if in_body {
                            fields.push(ir::text(tok.text()));
                        } else {
                            parts.push(ir::text(tok.text()));
                        }
                    }
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                if in_body || n.kind() == SyntaxKind::STRUCT_FIELD {
                    fields.push(walk_node(&n));
                } else {
                    match n.kind() {
                        SyntaxKind::VISIBILITY => {
                            parts.push(walk_node(&n));
                            parts.push(sp());
                        }
                        SyntaxKind::NAME | SyntaxKind::GENERIC_PARAM_LIST => {
                            parts.push(walk_node(&n));
                        }
                        _ => {
                            parts.push(walk_node(&n));
                        }
                    }
                }
            }
        }
    }

    if !fields.is_empty() {
        let mut field_parts = Vec::new();
        for field in fields {
            field_parts.push(ir::hardline());
            field_parts.push(field);
        }
        parts.push(ir::indent(ir::concat(field_parts)));
    }
    parts.push(ir::hardline());
    parts.push(ir::text("end"));

    ir::concat(parts)
}

fn walk_struct_field(node: &SyntaxNode) -> FormatIR {
    walk_tokens_inline(node)
}

// ── Closure expression ────────────────────────────────────────────────

fn walk_closure_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::FN_KW => {
                        parts.push(ir::text("fn"));
                        parts.push(sp());
                    }
                    SyntaxKind::ARROW => {
                        parts.push(sp());
                        parts.push(ir::text("->"));
                        parts.push(sp());
                    }
                    SyntaxKind::END_KW => {
                        parts.push(sp());
                        parts.push(ir::text("end"));
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        let stmt_count = count_block_stmts(&n);
                        if stmt_count > 1 {
                            let body = walk_block_body(&n);
                            parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                            parts.push(ir::hardline());
                        } else {
                            let body = walk_block_body(&n);
                            parts.push(body);
                        }
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

// ── Return expression ────────────────────────────────────────────────

fn walk_return_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::RETURN_KW => {
                        parts.push(ir::text("return"));
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(sp());
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Import declarations ──────────────────────────────────────────────

fn walk_import_decl(node: &SyntaxNode) -> FormatIR {
    walk_tokens_inline(node)
}

fn walk_from_import_decl(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::IDENT => {
                        if tok.text() == "from" {
                            parts.push(ir::text("from"));
                            parts.push(sp());
                        } else {
                            parts.push(ir::text(tok.text()));
                        }
                    }
                    SyntaxKind::IMPORT_KW => {
                        parts.push(sp());
                        parts.push(ir::text("import"));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── String expression ────────────────────────────────────────────────

fn walk_string_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        parts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_string_interpolation(&n));
            }
        }
    }

    ir::concat(parts)
}

fn walk_string_interpolation(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        parts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Field access ────────────────────────────────────────────────────

fn walk_field_access(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::DOT => {
                        parts.push(ir::text("."));
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        parts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Index expression ────────────────────────────────────────────────

fn walk_index_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::L_BRACKET => parts.push(ir::text("[")),
                    SyntaxKind::R_BRACKET => parts.push(ir::text("]")),
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        parts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Impl definition ──────────────────────────────────────────────────

fn walk_impl_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut has_block = false;

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::IMPL_KW => {
                        parts.push(ir::text("impl"));
                        parts.push(sp());
                    }
                    SyntaxKind::FOR_KW => {
                        parts.push(sp());
                        parts.push(ir::text("for"));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                        has_block = true;
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::IDENT => {
                        parts.push(ir::text(tok.text()));
                    }
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK if has_block => {
                        let body = walk_block_inner_items(&n);
                        parts.push(ir::indent(body));
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    SyntaxKind::NAME => {
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::GENERIC_PARAM_LIST | SyntaxKind::GENERIC_ARG_LIST => {
                        parts.push(walk_node(&n));
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    if !has_block {
        parts.push(ir::hardline());
        parts.push(ir::text("end"));
    }

    ir::concat(parts)
}

// ── Type alias ──────────────────────────────────────────────────────

fn walk_type_alias_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::TYPE_KW => {
                        parts.push(ir::text("type"));
                        parts.push(sp());
                    }
                    SyntaxKind::EQ => {
                        parts.push(sp());
                        parts.push(ir::text("="));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        parts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Variant definition ──────────────────────────────────────────────

fn walk_variant_def(node: &SyntaxNode) -> FormatIR {
    walk_tokens_inline(node)
}

// ── Receive expression ──────────────────────────────────────────────

fn walk_receive_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut arms: Vec<FormatIR> = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::RECEIVE_KW => {
                        parts.push(ir::text("receive"));
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::RECEIVE_ARM => {
                        arms.push(walk_node(&n));
                    }
                    SyntaxKind::AFTER_CLAUSE => {
                        arms.push(walk_node(&n));
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    if !arms.is_empty() {
        let mut arm_parts = Vec::new();
        for arm in arms {
            arm_parts.push(ir::hardline());
            arm_parts.push(arm);
        }
        parts.push(ir::indent(ir::concat(arm_parts)));
    }

    parts.push(ir::hardline());
    parts.push(ir::text("end"));

    ir::concat(parts)
}

// ── Spawn/Send/Link expressions ──────────────────────────────────────

fn walk_spawn_send_link(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::SPAWN_KW | SyntaxKind::SEND_KW | SyntaxKind::LINK_KW => {
                        parts.push(ir::text(tok.text()));
                    }
                    SyntaxKind::L_PAREN => parts.push(ir::text("(")),
                    SyntaxKind::R_PAREN => parts.push(ir::text(")")),
                    SyntaxKind::COMMA => {
                        parts.push(ir::text(","));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        parts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Self expression ──────────────────────────────────────────────────

fn walk_self_expr(node: &SyntaxNode) -> FormatIR {
    walk_tokens_inline(node)
}

// ── Call handler ──────────────────────────────────────────────────────

fn walk_call_handler(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::CALL_KW => {
                        parts.push(ir::text("call"));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        let body = walk_block_body(&n);
                        parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    SyntaxKind::PARAM_LIST => {
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::TYPE_ANNOTATION => {
                        parts.push(sp());
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::NAME => {
                        parts.push(walk_node(&n));
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

// ── Cast handler ──────────────────────────────────────────────────────

fn walk_cast_handler(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::CAST_KW => {
                        parts.push(ir::text("cast"));
                        parts.push(sp());
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        let body = walk_block_body(&n);
                        parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    SyntaxKind::PARAM_LIST => {
                        parts.push(walk_node(&n));
                    }
                    SyntaxKind::NAME => {
                        parts.push(walk_node(&n));
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

// ── Terminate clause ──────────────────────────────────────────────────

fn walk_terminate_clause(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::TERMINATE_KW => {
                        parts.push(ir::text("terminate"));
                    }
                    SyntaxKind::DO_KW => {
                        parts.push(sp());
                        parts.push(ir::text("do"));
                    }
                    SyntaxKind::END_KW => {}
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        let body = walk_block_body(&n);
                        parts.push(ir::indent(ir::concat(vec![ir::hardline(), body])));
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                    _ => {
                        parts.push(walk_node(&n));
                    }
                }
            }
        }
    }

    ir::concat(parts)
}

// ── Struct literal ──────────────────────────────────────────────────

fn walk_struct_literal(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::L_BRACE => {
                        parts.push(ir::text(" {"));
                        parts.push(sp());
                    }
                    SyntaxKind::R_BRACE => {
                        parts.push(sp());
                        parts.push(ir::text("}"));
                    }
                    SyntaxKind::COMMA => {
                        parts.push(ir::text(","));
                        parts.push(sp());
                    }
                    SyntaxKind::NEWLINE => {}
                    _ => {
                        parts.push(ir::text(tok.text()));
                    }
                }
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::group(ir::concat(parts))
}

// ── Walk block inner items ──────────────────────────────────────────

/// Walk the children of a BLOCK that contains items (fns, fields, etc.)
/// inside a module/actor/service/etc definition.
fn walk_block_inner_items(node: &SyntaxNode) -> FormatIR {
    let mut items: Vec<FormatIR> = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                match tok.kind() {
                    SyntaxKind::NEWLINE => {}
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT | SyntaxKind::MODULE_DOC_COMMENT => {
                        items.push(ir::text(tok.text()));
                    }
                    _ => {}
                }
            }
            NodeOrToken::Node(n) => {
                items.push(walk_node(&n));
            }
        }
    }

    if items.is_empty() {
        FormatIR::Empty
    } else {
        let mut parts = Vec::new();
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                parts.push(ir::hardline());
            }
            parts.push(ir::hardline());
            parts.push(item);
        }
        ir::concat(parts)
    }
}

// ── Helper: walk tokens inline with smart spacing ────────────────────

/// Walk all tokens in a node, emitting them with appropriate spacing.
fn walk_tokens_inline(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                let kind = tok.kind();
                if kind == SyntaxKind::EOF || kind == SyntaxKind::NEWLINE {
                    continue;
                }
                if kind == SyntaxKind::COMMENT
                    || kind == SyntaxKind::DOC_COMMENT
                    || kind == SyntaxKind::MODULE_DOC_COMMENT
                {
                    if !parts.is_empty() {
                        parts.push(sp());
                    }
                    parts.push(ir::text(tok.text()));
                    continue;
                }
                if !parts.is_empty() && needs_space_before(tok.kind()) {
                    parts.push(sp());
                }
                parts.push(ir::text(tok.text()));
            }
            NodeOrToken::Node(n) => {
                if !parts.is_empty() && needs_space_before_node(n.kind()) {
                    parts.push(sp());
                }
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Spacing helpers ──────────────────────────────────────────────────

/// Check if a token kind should be preceded by a space when following another token.
fn needs_space_before(kind: SyntaxKind) -> bool {
    !matches!(
        kind,
        SyntaxKind::L_PAREN
            | SyntaxKind::R_PAREN
            | SyntaxKind::L_BRACKET
            | SyntaxKind::R_BRACKET
            | SyntaxKind::COMMA
            | SyntaxKind::DOT
            | SyntaxKind::COLON_COLON
            | SyntaxKind::STRING_START
            | SyntaxKind::STRING_END
            | SyntaxKind::STRING_CONTENT
            | SyntaxKind::INTERPOLATION_START
            | SyntaxKind::INTERPOLATION_END
    )
}

/// Check if a node kind should be preceded by a space.
fn needs_space_before_node(kind: SyntaxKind) -> bool {
    !matches!(
        kind,
        SyntaxKind::PARAM_LIST
            | SyntaxKind::ARG_LIST
            | SyntaxKind::GENERIC_PARAM_LIST
    )
}

/// Check if a SyntaxKind is an operator token.
fn is_operator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
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
            | SyntaxKind::AMP_AMP
            | SyntaxKind::PIPE_PIPE
            | SyntaxKind::PIPE
            | SyntaxKind::DOT_DOT
            | SyntaxKind::DIAMOND
            | SyntaxKind::PLUS_PLUS
            | SyntaxKind::AND_KW
            | SyntaxKind::OR_KW
    )
}

/// Add a token to parts.
fn add_token_with_context(tok: &SyntaxToken, parts: &mut Vec<FormatIR>) {
    let kind = tok.kind();
    if kind == SyntaxKind::EOF || kind == SyntaxKind::NEWLINE {
        return;
    }
    if kind == SyntaxKind::COMMENT
        || kind == SyntaxKind::DOC_COMMENT
        || kind == SyntaxKind::MODULE_DOC_COMMENT
    {
        if !parts.is_empty() {
            parts.push(sp());
        }
        parts.push(ir::text(tok.text()));
        return;
    }
    parts.push(ir::text(tok.text()));
}

/// Count non-trivia children (statements) in a block.
fn count_block_stmts(node: &SyntaxNode) -> usize {
    let mut count = 0;
    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                if !tok.kind().is_trivia() && tok.kind() != SyntaxKind::EOF {
                    count += 1;
                }
            }
            NodeOrToken::Node(_) => {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use crate::format_source;
    use crate::printer::FormatConfig;

    fn fmt(source: &str) -> String {
        format_source(source, &FormatConfig::default())
    }

    #[test]
    fn simple_let_binding() {
        let result = fmt("let x = 1");
        assert_eq!(result, "let x = 1\n");
    }

    #[test]
    fn fn_def_with_body() {
        let result = fmt("fn add(a, b) do\na + b\nend");
        assert_eq!(result, "fn add(a, b) do\n  a + b\nend\n");
    }

    #[test]
    fn fn_def_multiple_statements() {
        let result = fmt("fn foo(x) do\nlet y = x + 1\ny\nend");
        assert_eq!(result, "fn foo(x) do\n  let y = x + 1\n  y\nend\n");
    }

    #[test]
    fn if_else_expression() {
        let result = fmt("if x > 0 do\nx\nelse\n-x\nend");
        assert_eq!(result, "if x > 0 do\n  x\nelse\n  -x\nend\n");
    }

    #[test]
    fn case_expression() {
        let result = fmt("case x do\n1 -> \"one\"\n2 -> \"two\"\nend");
        assert_eq!(result, "case x do\n  1 -> \"one\"\n  2 -> \"two\"\nend\n");
    }

    #[test]
    fn comment_preserved() {
        let result = fmt("# This is a comment\nfn foo() do\n1\nend");
        assert!(result.contains("# This is a comment"));
    }

    #[test]
    fn idempotent_let() {
        let src = "let x = 1";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn idempotent_fn_def() {
        let src = "fn add(a, b) do\na + b\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn idempotent_if_else() {
        let src = "if x > 0 do\nx\nelse\n-x\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn idempotent_case() {
        let src = "case x do\n1 -> \"one\"\n2 -> \"two\"\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn idempotent_module() {
        let src = "module Math do\nfn add(a, b) do\na + b\nend\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn struct_definition() {
        let result = fmt("struct Point do\nx :: Float\ny :: Float\nend");
        assert_eq!(result, "struct Point do\n  x :: Float\n  y :: Float\nend\n");
    }

    #[test]
    fn blank_line_between_top_level_items() {
        let result = fmt("fn foo() do\n1\nend\nfn bar() do\n2\nend");
        assert_eq!(result, "fn foo() do\n  1\nend\n\nfn bar() do\n  2\nend\n");
    }

    #[test]
    fn pipe_expression() {
        let result = fmt("x |> foo() |> bar()");
        assert!(result.contains("|>"));
        assert!(result.contains("foo()"));
        assert!(result.contains("bar()"));
    }

    #[test]
    fn call_with_args() {
        let result = fmt("foo(1, 2, 3)");
        assert_eq!(result, "foo(1, 2, 3)\n");
    }

    #[test]
    fn binary_expression() {
        let result = fmt("a + b");
        assert_eq!(result, "a + b\n");
    }

    #[test]
    fn from_import() {
        let result = fmt("from Math import sqrt, pow");
        assert_eq!(result, "from Math import sqrt, pow\n");
    }

    #[test]
    fn let_with_type_annotation() {
        let result = fmt("let name :: String = \"hello\"");
        assert_eq!(result, "let name :: String = \"hello\"\n");
    }

    #[test]
    fn idempotent_struct() {
        let src = "struct Point do\nx :: Float\ny :: Float\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn typed_fn_def() {
        let result = fmt("fn typed(x :: Int, y :: Int) -> Int do\nx + y\nend");
        assert_eq!(result, "fn typed(x :: Int, y :: Int) -> Int do\n  x + y\nend\n");
    }

    #[test]
    fn fn_expr_body_form() {
        let result = fmt("fn double(x) = x * 2");
        assert_eq!(result, "fn double(x) = x * 2\n");
    }

    #[test]
    fn fn_expr_body_literal_pattern() {
        let result = fmt("fn fib(0) = 0");
        assert_eq!(result, "fn fib(0) = 0\n");
    }

    #[test]
    fn fn_expr_body_with_guard() {
        let result = fmt("fn abs(n) when n < 0 = -n");
        assert_eq!(result, "fn abs(n) when n < 0 = -n\n");
    }

    #[test]
    fn fn_expr_body_idempotent() {
        let src = "fn fib(0) = 0";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn fn_expr_body_guard_idempotent() {
        let src = "fn abs(n) when n < 0 = -n";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(first, second, "Idempotency failed.\nFirst: {:?}\nSecond: {:?}", first, second);
    }

    #[test]
    fn multi_clause_fn_formatted() {
        let src = "fn fib(0) = 0\nfn fib(1) = 1\nfn fib(n) = fib(n - 1) + fib(n - 2)";
        let result = fmt(src);
        assert!(result.contains("fn fib(0) = 0"), "Result: {:?}", result);
        assert!(result.contains("fn fib(1) = 1"), "Result: {:?}", result);
        assert!(result.contains("fn fib(n) = fib(n - 1) + fib(n - 2)"), "Result: {:?}", result);
    }
}
