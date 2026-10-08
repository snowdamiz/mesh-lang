//! CST-to-FormatIR walker for Mesh source code.
//!
//! This module walks the rowan CST produced by `mesh-parser` and converts it
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

use mesh_parser::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use rowan::NodeOrToken;

use crate::ir::{self, FormatIR};

/// The CST keeps WHITESPACE and NEWLINE tokens so its offsets match the
/// source. The formatter derives all spacing and line breaks itself, so it
/// walks every element but those (the source file alone counts new lines,
/// for the blank lines it keeps).
trait Elements {
    fn elements(&self) -> impl Iterator<Item = SyntaxElement>;
}

impl Elements for SyntaxNode {
    fn elements(&self) -> impl Iterator<Item = SyntaxElement> {
        self.children_with_tokens().filter(|element| {
            !matches!(element.kind(), SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
        })
    }
}

/// Literal space text -- always emits " " regardless of mode.
/// Use this for unconditional spaces (e.g., between `fn` and name).
/// Use `ir::space()` only inside `Group` nodes where break behavior is desired.
fn sp() -> FormatIR {
    ir::text(" ")
}

/// Walk a CST node and produce a FormatIR document tree.
///
/// This is the main entry point for converting a parsed Mesh syntax tree
/// into the format IR that the printer can render.
pub fn walk_node(node: &SyntaxNode) -> FormatIR {
    let kind = node.kind();
    match kind {
        SyntaxKind::SOURCE_FILE => walk_source_file(node),
        SyntaxKind::FN_DEF | SyntaxKind::INTERFACE_METHOD => walk_fn_def(node),
        SyntaxKind::LITERAL_PAT => walk_literal_pat(node),
        SyntaxKind::STRUCT_UPDATE_EXPR => walk_struct_update(node),
        SyntaxKind::LET_BINDING => walk_let_binding(node),
        SyntaxKind::IF_EXPR => walk_if_expr(node),
        SyntaxKind::CASE_EXPR | SyntaxKind::RECEIVE_EXPR => walk_arms_expr(node),
        SyntaxKind::MATCH_ARM => walk_match_arm(node),
        SyntaxKind::BINARY_EXPR => walk_binary_expr(node),
        SyntaxKind::UNARY_EXPR => walk_unary_expr(node),
        SyntaxKind::CALL_EXPR => walk_concat(node),
        SyntaxKind::PIPE_EXPR => walk_pipe_expr(node),
        SyntaxKind::PARAM_LIST => walk_paren_list(node),
        SyntaxKind::ARG_LIST => walk_paren_list(node),
        SyntaxKind::MODULE_DEF => walk_block_def(node),
        SyntaxKind::STRUCT_DEF => walk_struct_def(node),
        SyntaxKind::STRUCT_FIELD => walk_struct_field(node),
        SyntaxKind::CLOSURE_EXPR => walk_closure_expr(node),
        SyntaxKind::TRAILING_CLOSURE => walk_trailing_closure(node),
        SyntaxKind::RETURN_EXPR => walk_return_expr(node),
        SyntaxKind::IMPORT_DECL => walk_import_decl(node),
        SyntaxKind::FROM_IMPORT_DECL => walk_from_import_decl(node),
        SyntaxKind::IMPORT_LIST => walk_import_list(node),
        SyntaxKind::STRING_EXPR | SyntaxKind::INTERPOLATION => walk_concat(node),
        SyntaxKind::TUPLE_EXPR => walk_paren_list(node),
        SyntaxKind::FIELD_ACCESS | SyntaxKind::INDEX_EXPR => walk_concat(node),
        SyntaxKind::ELSE_BRANCH => walk_else_branch(node),
        SyntaxKind::INTERFACE_DEF => walk_block_def(node),
        SyntaxKind::IMPL_DEF => walk_impl_def(node),
        SyntaxKind::TYPE_ALIAS_DEF => walk_type_alias_def(node),
        SyntaxKind::SUM_TYPE_DEF => walk_block_def(node),
        SyntaxKind::VARIANT_DEF => walk_variant_def(node),
        SyntaxKind::ACTOR_DEF => walk_block_def(node),
        SyntaxKind::SERVICE_DEF => walk_block_def(node),
        SyntaxKind::SUPERVISOR_DEF => walk_block_def(node),
        SyntaxKind::RECEIVE_ARM => walk_match_arm(node),
        SyntaxKind::SPAWN_EXPR | SyntaxKind::SEND_EXPR | SyntaxKind::LINK_EXPR => walk_concat(node),
        SyntaxKind::WHILE_EXPR => walk_while_expr(node),
        SyntaxKind::FOR_IN_EXPR => walk_for_in_expr(node),
        SyntaxKind::BREAK_EXPR => walk_break_expr(node),
        SyntaxKind::CONTINUE_EXPR => walk_continue_expr(node),
        SyntaxKind::SELF_EXPR => walk_self_expr(node),
        SyntaxKind::CALL_HANDLER | SyntaxKind::CAST_HANDLER | SyntaxKind::TERMINATE_CLAUSE => {
            walk_handler(node)
        }
        SyntaxKind::CHILD_SPEC_DEF => walk_child_spec_def(node),
        SyntaxKind::DESTRUCTURE_BINDING => walk_destructure_binding(node),
        SyntaxKind::DERIVING_CLAUSE => walk_deriving_clause(node),
        SyntaxKind::STRUCT_LITERAL | SyntaxKind::STRUCT_PAT | SyntaxKind::JSON_EXPR => {
            walk_braced_fields(node)
        }
        SyntaxKind::MAP_LITERAL => walk_map_literal(node),
        SyntaxKind::MAP_ENTRY => walk_map_entry(node),
        SyntaxKind::LIST_LITERAL => walk_list_literal(node),
        SyntaxKind::ASSOC_TYPE_BINDING => walk_assoc_type_binding(node),
        SyntaxKind::SCHEMA_OPTION => walk_tokens_inline(node),
        SyntaxKind::TRY_EXPR | SyntaxKind::ASSERT_RECEIVE_EXPR => walk_tokens_inline(node),
        SyntaxKind::PATH => walk_concat(node),
        // Simple leaf-like nodes: just emit their tokens inline.
        SyntaxKind::LITERAL
        | SyntaxKind::NAME
        | SyntaxKind::NAME_REF
        | SyntaxKind::TYPE_ANNOTATION
        | SyntaxKind::VISIBILITY
        | SyntaxKind::WILDCARD_PAT
        | SyntaxKind::IDENT_PAT
        | SyntaxKind::TUPLE_PAT
        | SyntaxKind::STRUCT_PAT_FIELD
        | SyntaxKind::CONSTRUCTOR_PAT
        | SyntaxKind::OR_PAT
        | SyntaxKind::AS_PAT
        | SyntaxKind::GUARD_CLAUSE
        | SyntaxKind::FN_EXPR_BODY
        | SyntaxKind::TYPE_PARAM_LIST
        | SyntaxKind::GENERIC_PARAM_LIST
        | SyntaxKind::GENERIC_ARG_LIST
        | SyntaxKind::WHERE_CLAUSE
        | SyntaxKind::TRAIT_BOUND
        | SyntaxKind::OPTION_TYPE
        | SyntaxKind::RESULT_TYPE
        | SyntaxKind::VARIANT_FIELD
        | SyntaxKind::AFTER_CLAUSE
        | SyntaxKind::STRATEGY_CLAUSE
        | SyntaxKind::RESTART_LIMIT
        | SyntaxKind::SECONDS_LIMIT
        | SyntaxKind::STRUCT_LITERAL_FIELD
        | SyntaxKind::ASSOC_TYPE_DEF
        | SyntaxKind::FUN_TYPE
        | SyntaxKind::CONS_PAT
        | SyntaxKind::LIST_PAT
        | SyntaxKind::PARAM => walk_tokens_inline(node),
        // Fallback: emit tokens with spaces.
        _ => walk_tokens_inline(node),
    }
}

// ── Source file (top-level) ────────────────────────────────────────────

struct SourceFileItem {
    ir: FormatIR,
    /// On the line after the previous item, with no blank line between.
    joins_previous: bool,
}

/// Whether `next` goes on the line after `prev` with no blank line between:
/// imports are one block, and so are the clauses of a function the source
/// wrote together.
fn items_join(prev: &SyntaxNode, next: &SyntaxNode, blank_line: bool) -> bool {
    let import = |n: &SyntaxNode| {
        matches!(
            n.kind(),
            SyntaxKind::IMPORT_DECL | SyntaxKind::FROM_IMPORT_DECL
        )
    };
    let fn_name = |n: &SyntaxNode| {
        (n.kind() == SyntaxKind::FN_DEF)
            .then(|| n.children().find(|c| c.kind() == SyntaxKind::NAME))
            .flatten()
            .map(|name| name.text().to_string())
    };
    (import(prev) && import(next))
        || (!blank_line && fn_name(prev).is_some() && fn_name(prev) == fn_name(next))
}

fn flush_pending_source_comments(
    pending_comments: &mut Vec<FormatIR>,
    items: &mut Vec<SourceFileItem>,
) {
    if pending_comments.is_empty() {
        return;
    }

    let mut parts = Vec::new();
    for (i, comment) in pending_comments.drain(..).enumerate() {
        if i > 0 {
            parts.push(ir::hardline());
        }
        parts.push(comment);
    }

    items.push(SourceFileItem {
        ir: ir::concat(parts),
        joins_previous: false,
    });
}

fn walk_source_file(node: &SyntaxNode) -> FormatIR {
    let mut items: Vec<SourceFileItem> = Vec::new();
    let mut pending_comments: Vec<FormatIR> = Vec::new();
    // The declaration just before, when no comment block came after it.
    let mut prev_node: Option<SyntaxNode> = None;
    // Line breaks since the last comment or item: two or more is a blank line.
    let mut newlines = 0;

    for child in node
        .children_with_tokens()
        .filter(|element| element.kind() != SyntaxKind::WHITESPACE)
    {
        match child {
            NodeOrToken::Token(tok) => {
                let kind = tok.kind();
                match kind {
                    SyntaxKind::EOF => {}
                    SyntaxKind::NEWLINE => newlines += 1,
                    SyntaxKind::COMMENT
                    | SyntaxKind::DOC_COMMENT
                    | SyntaxKind::MODULE_DOC_COMMENT => {
                        match items.last_mut() {
                            Some(last)
                                if pending_comments.is_empty() && ends_a_line_of_code(&tok) =>
                            {
                                append_comment(&mut last.ir, &tok)
                            }
                            _ => {
                                // A blank line ends a comment block.
                                if newlines > 1 {
                                    flush_pending_source_comments(
                                        &mut pending_comments,
                                        &mut items,
                                    );
                                }
                                pending_comments.push(ir::text(tok.text()));
                                prev_node = None;
                            }
                        }
                        newlines = 0;
                    }
                    _ => {}
                }
            }
            NodeOrToken::Node(n) => {
                // A comment block directly above a declaration stays attached to it.
                let joins_previous = if pending_comments.is_empty() {
                    prev_node
                        .as_ref()
                        .is_some_and(|prev| items_join(prev, &n, newlines > 1))
                } else {
                    newlines <= 1
                };
                flush_pending_source_comments(&mut pending_comments, &mut items);
                items.push(SourceFileItem {
                    ir: walk_node(&n),
                    joins_previous,
                });
                prev_node = Some(n);
                newlines = 0;
            }
        }
    }

    flush_pending_source_comments(&mut pending_comments, &mut items);

    let mut parts = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            parts.push(ir::hardline());
            if !item.joins_previous {
                parts.push(ir::hardline());
            }
        }
        parts.push(item.ir);
    }
    ir::concat(parts)
}

// ── Function definition ──────────────────────────────────────────────

fn walk_fn_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
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
                    }
                    // `end` closes the BLOCK below; the `=` of an expression
                    // body goes with FN_EXPR_BODY.
                    SyntaxKind::END_KW | SyntaxKind::EQ => {}
                    // A comment after a decorator on its own line: one that
                    // ends the decorator's line stays there.
                    SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT
                        if matches!(parts.last(), Some(FormatIR::Hardline)) =>
                    {
                        if ends_a_line_of_code(&tok) {
                            parts.pop();
                            parts.push(sp());
                        }
                        parts.push(ir::text(tok.text()));
                        parts.push(ir::hardline());
                    }
                    _ => {
                        add_token_with_context(&tok, &mut parts);
                    }
                }
            }
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::CLUSTER_DECORATOR_DECL
                    | SyntaxKind::NATIVE_DECORATOR_DECL
                    | SyntaxKind::EXPORT_DECORATOR_DECL
                    | SyntaxKind::DISPLAY_DECORATOR_DECL => {
                        parts.push(walk_decorator(&n));
                        // Keep the decorator on the fn's line or its own, as written.
                        let own_line = std::iter::successors(n.next_sibling_or_token(), |e| {
                            e.next_sibling_or_token()
                        })
                        .take_while(|e| e.kind().is_trivia())
                        .any(|e| e.kind() == SyntaxKind::NEWLINE);
                        parts.push(if own_line { ir::hardline() } else { sp() });
                    }
                    SyntaxKind::VISIBILITY => {
                        parts.push(walk_node(&n));
                        parts.push(sp());
                    }
                    SyntaxKind::TYPE_ANNOTATION
                    | SyntaxKind::WHERE_CLAUSE
                    | SyntaxKind::GUARD_CLAUSE => {
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
                    }
                    SyntaxKind::BLOCK => {
                        parts.push(body_then_end(&n));
                    }
                    // The name, generic parameters and parameters.
                    _ => parts.push(walk_node(&n)),
                }
            }
        }
    }

    ir::concat(parts)
}

/// `@cluster`, `@cluster(3)`, `@native("sym")`, `@export`: no spaces inside.
fn walk_decorator(node: &SyntaxNode) -> FormatIR {
    ir::concat(
        node.descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !matches!(t.kind(), SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE))
            .map(|t| {
                if t.kind().is_trivia() {
                    inline_comment(&t)
                } else {
                    ir::text(t.text())
                }
            })
            .collect(),
    )
}

// ── Let binding ────────────────────────────────────────────────────────

fn walk_let_binding(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::LET_KW => {
                    parts.push(ir::text("let"));
                    parts.push(sp());
                }
                SyntaxKind::EQ => {
                    parts.push(sp());
                    parts.push(ir::text("="));
                    parts.push(sp());
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => match n.kind() {
                SyntaxKind::TYPE_ANNOTATION => {
                    parts.push(sp());
                    parts.push(walk_node(&n));
                }
                _ => {
                    parts.push(walk_node(&n));
                }
            },
        }
    }

    ir::group(ir::concat(parts))
}

// ── If expression ────────────────────────────────────────────────────

fn walk_if_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
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
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        parts.push(indented_body(&n));
                    }
                    // The condition and the else branch.
                    _ => parts.push(walk_node(&n)),
                }
            }
        }
    }

    ir::concat(parts)
}

fn walk_else_branch(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::ELSE_KW => {
                    parts.push(ir::hardline());
                    parts.push(ir::text("else"));
                }
                SyntaxKind::END_KW => {
                    parts.push(ir::hardline());
                    parts.push(ir::text("end"));
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => match n.kind() {
                SyntaxKind::BLOCK => {
                    parts.push(indented_body(&n));
                }
                // `else if`.
                _ => {
                    parts.push(sp());
                    parts.push(walk_node(&n));
                }
            },
        }
    }

    ir::concat(parts)
}

// ── While expression ──────────────────────────────────────────────────

fn walk_while_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::WHILE_KW => {
                    parts.push(ir::text("while"));
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
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        parts.push(indented_body(&n));
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

// ── For-in expression ──────────────────────────────────────────────────

fn walk_for_in_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::FOR_KW => {
                    parts.push(ir::text("for"));
                    parts.push(sp());
                }
                SyntaxKind::IN_KW => {
                    parts.push(sp());
                    parts.push(ir::text("in"));
                    parts.push(sp());
                }
                SyntaxKind::WHEN_KW => {
                    parts.push(sp());
                    parts.push(ir::text("when"));
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
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => {
                match n.kind() {
                    SyntaxKind::BLOCK => {
                        parts.push(indented_body(&n));
                    }
                    // The binding, the iterable and the filter.
                    _ => parts.push(walk_node(&n)),
                }
            }
        }
    }

    ir::concat(parts)
}

/// `{k, v}`: a `for` loop's map entry binding.
fn walk_destructure_binding(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut names = 0;
    for child in node.elements() {
        match child {
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
                names += 1;
            }
            // `{k, v,}` loses its trailing comma.
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA => {
                if names < 2 {
                    parts.extend([ir::text(","), sp()]);
                }
            }
            NodeOrToken::Token(tok) => add_token_with_context(&tok, &mut parts),
        }
    }
    ir::concat(parts)
}

fn walk_break_expr(_node: &SyntaxNode) -> FormatIR {
    ir::text("break")
}

fn walk_continue_expr(_node: &SyntaxNode) -> FormatIR {
    ir::text("continue")
}

// ── Case/match expression ────────────────────────────────────────────

/// `case`/`match subject do <arms> end` and `receive do <arms> end`: each arm
/// (`after` included) on its own line, and a comment between arms kept where
/// it was, after an arm's line or on a line of its own.
fn walk_arms_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut arms: Vec<FormatIR> = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::CASE_KW | SyntaxKind::MATCH_KW | SyntaxKind::RECEIVE_KW => {
                    parts.push(ir::text(tok.text()));
                }
                SyntaxKind::DO_KW => {
                    parts.push(sp());
                    parts.push(ir::text("do"));
                }
                // Arms separated by `;` go on lines of their own.
                SyntaxKind::END_KW | SyntaxKind::SEMICOLON => {}
                // The other tokens are comments.
                _ => push_body_comment(&mut parts, &mut arms, &tok),
            },
            NodeOrToken::Node(n) => match n.kind() {
                SyntaxKind::MATCH_ARM | SyntaxKind::RECEIVE_ARM | SyntaxKind::AFTER_CLAUSE => {
                    arms.push(walk_node(&n));
                }
                // The scrutinee of a `case`.
                _ => {
                    parts.push(sp());
                    parts.push(walk_node(&n));
                }
            },
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
    let has_do = node
        .elements()
        .any(|child| child.kind() == SyntaxKind::DO_KW);

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::ARROW | SyntaxKind::FAT_ARROW => {
                    parts.push(sp());
                    parts.push(ir::text(tok.text()));
                    parts.push(sp());
                }
                SyntaxKind::DO_KW => {
                    parts.push(ir::text("do"));
                }
                SyntaxKind::END_KW => {}
                // A guard is `when` and its condition, no node of its own.
                SyntaxKind::WHEN_KW => {
                    parts.push(sp());
                    parts.push(ir::text("when"));
                    parts.push(sp());
                }
                _ => add_token_with_context(&tok, &mut parts),
            },
            NodeOrToken::Node(n) => match n.kind() {
                // A `do ... end` body, or statements on the lines after `->`:
                // those stay there, indented, so the arm stays a block when
                // re-parsed.
                SyntaxKind::BLOCK => {
                    parts.push(indented_body(&n));
                    if has_do {
                        parts.push(ir::hardline());
                        parts.push(ir::text("end"));
                    }
                }
                _ => {
                    parts.push(walk_node(&n));
                }
            },
        }
    }

    ir::concat(parts)
}

fn walk_trailing_closure(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut first_bar = true;

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::DO_KW => {
                    parts.push(sp());
                    parts.push(ir::text("do"));
                }
                SyntaxKind::BAR => {
                    if first_bar {
                        parts.push(sp());
                        first_bar = false;
                    }
                    parts.push(ir::text("|"));
                }
                SyntaxKind::END_KW => {}
                _ => add_token_with_context(&tok, &mut parts),
            },
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::PARAM_LIST => {
                parts.push(walk_bare_param_list(&n))
            }
            // The body.
            NodeOrToken::Node(n) => {
                parts.push(body_then_end(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Binary expression ────────────────────────────────────────────────

fn walk_binary_expr(node: &SyntaxNode) -> FormatIR {
    let (mut first, mut rest) = (Vec::new(), Vec::new());
    binary_chain(node, &mut first, &mut rest);
    ir::group(ir::concat(vec![
        ir::concat(first),
        ir::indent(ir::concat(rest)),
    ]))
}

/// The operands and operators of `node`, taking in operands that repeat its
/// operator, so each link of a long `a && b && c` gets a line of its own. The
/// chain breaks before its operators, the links one level in (a line starting
/// with an operator continues the last). Comparisons stay on the line: their
/// operands are what break, and `x\n  == 0` after a multi-line `x` reads badly.
/// `-` and `%` never start a line, since they also start statements.
fn binary_chain(node: &SyntaxNode, first: &mut Vec<FormatIR>, rest: &mut Vec<FormatIR>) {
    let operator = binary_operator(node);
    for child in node.elements() {
        // Everything after the chain's first operator goes one level in.
        let parts = if rest.is_empty() {
            &mut *first
        } else {
            &mut *rest
        };
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                // Range operator `..` has no surrounding spaces.
                SyntaxKind::DOT_DOT => parts.push(ir::text("..")),
                SyntaxKind::AMP_AMP
                | SyntaxKind::PIPE_PIPE
                | SyntaxKind::AND_KW
                | SyntaxKind::OR_KW
                | SyntaxKind::DIAMOND
                | SyntaxKind::PLUS_PLUS
                | SyntaxKind::PLUS
                | SyntaxKind::STAR
                | SyntaxKind::SLASH => rest.extend([ir::space(), ir::text(tok.text()), sp()]),
                kind if is_operator(kind) => rest.extend([sp(), ir::text(tok.text()), sp()]),
                _ => add_token_with_context(&tok, parts),
            },
            NodeOrToken::Node(n)
                if n.kind() == SyntaxKind::BINARY_EXPR && binary_operator(&n) == operator =>
            {
                binary_chain(&n, first, rest)
            }
            NodeOrToken::Node(n) => parts.push(walk_node(&n)),
        }
    }
}

fn binary_operator(node: &SyntaxNode) -> Option<SyntaxKind> {
    node.children_with_tokens()
        .map(|element| element.kind())
        .find(|&kind| kind == SyntaxKind::DOT_DOT || is_operator(kind))
}

// ── Unary expression ────────────────────────────────────────────────

fn walk_unary_expr(node: &SyntaxNode) -> FormatIR {
    // `not x`; `-x` and `!x` take no space.
    let mut parts = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => {
                add_token_with_context(&tok, &mut parts);
                if tok.kind() == SyntaxKind::NOT_KW {
                    parts.push(sp());
                }
            }
            NodeOrToken::Node(n) => parts.push(walk_node(&n)),
        }
    }
    ir::concat(parts)
}

/// The elements as they are, spaced as `add_token_with_context` spaces
/// tokens: a call `f(x)`, a field access `a.b`, an index `a[0]`, `spawn(...)`.
fn walk_concat(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => add_token_with_context(&tok, &mut parts),
            NodeOrToken::Node(n) => parts.push(walk_node(&n)),
        }
    }
    ir::concat(parts)
}

// ── Pipe expression ────────────────────────────────────────────────

enum PipePiece {
    Code(FormatIR),
    Comment(SyntaxToken),
}

fn collect_pipe_segments(node: &SyntaxNode, pieces: &mut Vec<PipePiece>) {
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => {
                if matches!(tok.kind(), SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT) {
                    pieces.push(PipePiece::Comment(tok));
                }
            }
            NodeOrToken::Node(n) => {
                if n.kind() == SyntaxKind::PIPE_EXPR {
                    collect_pipe_segments(&n, pieces);
                } else {
                    pieces.push(PipePiece::Code(walk_node(&n)));
                }
            }
        }
    }
}

/// One step of a pipeline per line; a comment stays at the end of the line
/// it ends, or on a line of its own.
fn walk_pipe_expr(node: &SyntaxNode) -> FormatIR {
    let mut pieces = Vec::new();
    collect_pipe_segments(node, &mut pieces);

    let mut lines: Vec<FormatIR> = Vec::new();
    let mut seen_code = false;
    for piece in pieces {
        match piece {
            PipePiece::Comment(tok) => match lines.last_mut() {
                Some(last) if ends_a_line_of_code(&tok) => append_comment(last, &tok),
                _ => lines.push(inline_comment(&tok)),
            },
            PipePiece::Code(segment) => {
                lines.push(if seen_code {
                    ir::concat(vec![ir::text("|>"), sp(), segment])
                } else {
                    segment
                });
                seen_code = true;
            }
        }
    }

    // The first line, then the others indented under it.
    let rest: Vec<FormatIR> = lines
        .split_off(lines.len().min(1))
        .into_iter()
        .flat_map(|line| [ir::hardline(), line])
        .collect();
    ir::concat(vec![ir::concat(lines), ir::indent(ir::concat(rest))])
}

// ── Block ─────────────────────────────────────────────────────────

/// A block's statements on the lines below, one level in.
fn indented_body(block: &SyntaxNode) -> FormatIR {
    ir::indent(ir::concat(vec![ir::hardline(), walk_block_body(block)]))
}

/// A block's statements on the lines below, one level in, then `end`.
fn body_then_end(block: &SyntaxNode) -> FormatIR {
    ir::concat(vec![indented_body(block), ir::hardline(), ir::text("end")])
}

/// Walk the children of a BLOCK node, producing statements separated by hardlines.
fn walk_block_body(node: &SyntaxNode) -> FormatIR {
    let mut stmts: Vec<FormatIR> = Vec::new();

    for child in node.elements() {
        match child {
            // Statements separated by `;` go on lines of their own.
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::SEMICOLON => {}
            // The other tokens in a block are comments.
            NodeOrToken::Token(tok) => push_comment(&mut stmts, &tok),
            NodeOrToken::Node(n) => stmts.push(walk_node(&n)),
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
    let mut outer = Vec::new();
    // The elements of a list that has to break sit one level in rather than at
    // the enclosing statement's indent. A single element has no separators to
    // break at and keeps its own layout, so `Ok(rows |> ..)` does not drift.
    let mut inner = Vec::new();
    let indented = node.children().count() > 1;
    let mut open = false;

    for child in node.elements() {
        let parts = if open { &mut inner } else { &mut outer };
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::L_PAREN => {
                    parts.push(ir::text("("));
                    open = true;
                }
                SyntaxKind::R_PAREN => {
                    let elements = ir::concat(std::mem::take(&mut inner));
                    outer.push(if indented {
                        ir::indent(elements)
                    } else {
                        elements
                    });
                    outer.push(ir::text(")"));
                    open = false;
                }
                SyntaxKind::COMMA => {
                    parts.push(ir::text(","));
                    if !is_trailing_comma(&tok) {
                        parts.push(ir::space());
                    }
                }
                _ => {
                    add_token_with_context(&tok, parts);
                }
            },
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    // A list the parser left unclosed keeps whatever followed its paren.
    outer.extend(inner);
    ir::group(ir::concat(outer))
}

// ── Block-structured definitions (module, actor, service, etc.) ──────

fn walk_block_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut past_do = false;
    let mut inner_items: Vec<FormatIR> = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
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
                // Items separated by `;` go on lines of their own.
                SyntaxKind::END_KW | SyntaxKind::SEMICOLON => {}
                // After `do`, the only other tokens are comments.
                _ if past_do => push_body_comment(&mut parts, &mut inner_items, &tok),
                _ => add_token_with_context(&tok, &mut parts),
            },
            NodeOrToken::Node(n) => {
                if n.kind() == SyntaxKind::DERIVING_CLAUSE {
                    // Handled after "end" is emitted
                } else if !past_do {
                    parts.push(walk_node(&n));
                    if n.kind() == SyntaxKind::VISIBILITY {
                        parts.push(sp());
                    }
                } else if n.kind() == SyntaxKind::BLOCK && node.kind() == SyntaxKind::ACTOR_DEF {
                    // An actor's body is statements, one per line, as in a
                    // function; the other bodies hold definitions.
                    let body = walk_block_body(&n);
                    if !matches!(body, FormatIR::Empty) {
                        inner_items.push(body);
                    }
                } else if n.kind() == SyntaxKind::BLOCK {
                    for block_child in n.elements() {
                        match block_child {
                            NodeOrToken::Token(t) => match t.kind() {
                                SyntaxKind::COMMENT
                                | SyntaxKind::DOC_COMMENT
                                | SyntaxKind::MODULE_DOC_COMMENT => {
                                    push_comment(&mut inner_items, &t);
                                }
                                _ => {}
                            },
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
        // Definitions are separated by a blank line; a sum type's variants
        // are listed one per line.
        let blank_between = node.kind() != SyntaxKind::SUM_TYPE_DEF;
        let mut body_parts = Vec::new();
        for (i, item) in inner_items.into_iter().enumerate() {
            if i > 0 && blank_between {
                body_parts.push(ir::hardline());
            }
            body_parts.push(ir::hardline());
            body_parts.push(item);
        }
        parts.push(ir::indent(ir::concat(body_parts)));
    }
    parts.push(ir::hardline());
    parts.push(ir::text("end"));

    // `deriving(...)` follows `end`.
    if let Some(deriving) = node
        .children()
        .find(|n| n.kind() == SyntaxKind::DERIVING_CLAUSE)
    {
        parts.push(sp());
        parts.push(walk_node(&deriving));
    }

    ir::concat(parts)
}

/// `child name do`, then one `key: value` per line.
fn walk_child_spec_def(node: &SyntaxNode) -> FormatIR {
    let mut header = Vec::new();
    let mut lines: Vec<FormatIR> = Vec::new();
    let mut past_do = false;
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::DO_KW => {
                    header.extend([sp(), ir::text("do")]);
                    past_do = true;
                }
                SyntaxKind::END_KW => {}
                _ if past_do => push_body_comment(&mut header, &mut lines, &tok),
                // `child`.
                SyntaxKind::IDENT => header.extend([ir::text(tok.text()), sp()]),
                _ => add_token_with_context(&tok, &mut header),
            },
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::BLOCK => {
                walk_child_spec_body(&n, &mut header, &mut lines)
            }
            // The name.
            NodeOrToken::Node(n) => header.push(walk_node(&n)),
        }
    }
    let body: Vec<FormatIR> = lines
        .into_iter()
        .flat_map(|line| [ir::hardline(), line])
        .collect();
    ir::concat(vec![
        ir::concat(header),
        ir::indent(ir::concat(body)),
        ir::hardline(),
        ir::text("end"),
    ])
}

/// A child spec's `key: value` pairs, a line each; `;` between them goes.
fn walk_child_spec_body(block: &SyntaxNode, header: &mut Vec<FormatIR>, lines: &mut Vec<FormatIR>) {
    // A pair is a key, which starts its line, then `:`, then the value.
    let mut at_key = true;
    for element in block.elements() {
        let (piece, colon) = match element {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::SEMICOLON => continue,
            NodeOrToken::Token(tok) if tok.kind().is_trivia() => {
                push_body_comment(header, lines, &tok);
                continue;
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COLON => {
                (ir::concat(vec![ir::text(":"), sp()]), true)
            }
            NodeOrToken::Token(tok) => (ir::text(tok.text()), false),
            NodeOrToken::Node(n) => (walk_node(&n), false),
        };
        match lines.last_mut() {
            Some(line) if !at_key => append(line, piece),
            _ => lines.push(piece),
        }
        // After a key or `:` the pair goes on; a value ends it.
        at_key = !at_key && !colon;
    }
}

// ── Struct definition ─────────────────────────────────────────────────

fn walk_struct_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let mut fields: Vec<FormatIR> = Vec::new();
    let mut in_body = false;
    let is_opaque_resource = node
        .children()
        .any(|child| child.kind() == SyntaxKind::RESOURCE_MODIFIER)
        && !node
            .elements()
            .any(|child| child.kind() == SyntaxKind::STRUCT_KW);

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::STRUCT_KW => {
                    parts.push(ir::text("struct"));
                    parts.push(sp());
                }
                SyntaxKind::DO_KW => {
                    parts.push(sp());
                    parts.push(ir::text("do"));
                    in_body = true;
                }
                // Fields separated by `;` go on lines of their own.
                SyntaxKind::END_KW | SyntaxKind::SEMICOLON => {}
                // After `do`, the only other tokens are comments.
                _ if in_body => push_body_comment(&mut parts, &mut fields, &tok),
                _ => add_token_with_context(&tok, &mut parts),
            },
            // Emitted after `end`.
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::DERIVING_CLAUSE => {}
            NodeOrToken::Node(n) if in_body || n.kind() == SyntaxKind::STRUCT_FIELD => {
                fields.push(walk_node(&n));
            }
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
                if matches!(
                    n.kind(),
                    SyntaxKind::VISIBILITY | SyntaxKind::RESOURCE_MODIFIER
                ) {
                    parts.push(sp());
                }
            }
        }
    }

    if is_opaque_resource {
        return ir::concat(parts);
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

    // `deriving(...)` follows `end`.
    if let Some(deriving) = node
        .children()
        .find(|n| n.kind() == SyntaxKind::DERIVING_CLAUSE)
    {
        parts.push(sp());
        parts.push(walk_node(&deriving));
    }

    ir::concat(parts)
}

/// `deriving(Eq, Show)`; a trailing comma goes.
fn walk_deriving_clause(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for tok in node.elements().filter_map(|e| e.into_token()) {
        match tok.kind() {
            SyntaxKind::COMMA if is_trailing_comma(&tok) => {}
            SyntaxKind::COMMA => parts.extend([ir::text(","), sp()]),
            _ => add_token_with_context(&tok, &mut parts),
        }
    }
    ir::concat(parts)
}

fn walk_struct_field(node: &SyntaxNode) -> FormatIR {
    walk_tokens_inline(node)
}

// ── Closure expression ────────────────────────────────────────────────

fn walk_closure_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let has_do = node.elements().any(|c| c.kind() == SyntaxKind::DO_KW);
    // A comment after `->` ends its line: the body goes on the next one, and
    // `end` on its own.
    let mut broken = false;

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::FN_KW => {
                    parts.push(ir::text("fn"));
                    // `fn(x)`, spelled like the function type `fn(Int)`;
                    // `fn x ->`, `fn ->` and `fn do` keep their space.
                    let parens = node
                        .children()
                        .next()
                        .is_some_and(|first| has_parens(&first));
                    if !parens {
                        parts.push(sp());
                    }
                }
                SyntaxKind::ARROW => parts.extend([ir::text("->"), sp()]),
                SyntaxKind::DO_KW => parts.push(ir::text("do")),
                SyntaxKind::END_KW => {
                    if broken {
                        parts.push(ir::hardline());
                    } else if !has_do {
                        // A `do` body places its `end` itself.
                        parts.push(sp());
                    }
                    parts.push(ir::text("end"));
                }
                _ => {
                    broken |= !has_do && tok.kind() == SyntaxKind::COMMENT;
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => match n.kind() {
                SyntaxKind::PARAM_LIST => parts.extend([walk_closure_params(&n), sp()]),
                SyntaxKind::GUARD_CLAUSE => parts.extend([walk_node(&n), sp()]),
                SyntaxKind::CLOSURE_CLAUSE => parts.extend([sp(), walk_closure_clause(&n)]),
                _ if has_do => parts.push(closure_do_body(&n)),
                _ if broken => {
                    parts.push(indented_body(&n));
                }
                // An arrow body, on the arrow's line.
                _ => parts.push(walk_block_body(&n)),
            },
        }
    }

    ir::concat(parts)
}

fn has_parens(params: &SyntaxNode) -> bool {
    params.kind() == SyntaxKind::PARAM_LIST
        && params.elements().any(|c| c.kind() == SyntaxKind::L_PAREN)
}

/// A closure clause's parameters, `(x, y)` or bare `x, y`.
fn walk_closure_params(params: &SyntaxNode) -> FormatIR {
    if has_parens(params) {
        walk_paren_list(params)
    } else {
        walk_bare_param_list(params)
    }
}

/// A closure clause's `do` body up to its `end`: on the `do` line when it is
/// one short expression (`fn() do 1 end`), indented on the lines below
/// otherwise.
fn closure_do_body(block: &SyntaxNode) -> FormatIR {
    let multiline = block.children().count() > 1
        || block.children().next().is_some_and(|stmt| {
            matches!(
                stmt.kind(),
                SyntaxKind::STRUCT_LITERAL
                    | SyntaxKind::MAP_LITERAL
                    | SyntaxKind::PIPE_EXPR
                    // Block constructs keep their lines.
                    | SyntaxKind::IF_EXPR
                    | SyntaxKind::CASE_EXPR
                    | SyntaxKind::FOR_IN_EXPR
                    | SyntaxKind::WHILE_EXPR
                    | SyntaxKind::RECEIVE_EXPR
            )
        });
    if multiline {
        ir::concat(vec![indented_body(block), ir::hardline()])
    } else {
        ir::concat(vec![sp(), walk_block_body(block), sp()])
    }
}

/// `| pattern -> body` or `| pattern do body end`: a closure's clauses after
/// its first.
fn walk_closure_clause(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    let has_do = node.elements().any(|c| c.kind() == SyntaxKind::DO_KW);
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::BAR | SyntaxKind::ARROW => parts.extend([ir::text(tok.text()), sp()]),
                SyntaxKind::DO_KW | SyntaxKind::END_KW => parts.push(ir::text(tok.text())),
                _ => add_token_with_context(&tok, &mut parts),
            },
            NodeOrToken::Node(n) => match n.kind() {
                SyntaxKind::PARAM_LIST => parts.extend([walk_closure_params(&n), sp()]),
                SyntaxKind::GUARD_CLAUSE => parts.extend([walk_node(&n), sp()]),
                _ if has_do => parts.push(closure_do_body(&n)),
                _ => parts.push(walk_block_body(&n)),
            },
        }
    }
    ir::concat(parts)
}

/// Walk a bare (unparenthesized) PARAM_LIST, formatting params with ", " separators.
fn walk_bare_param_list(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::COMMA => {
                    parts.push(ir::text(","));
                    parts.push(sp());
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── Return expression ────────────────────────────────────────────────

fn walk_return_expr(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::RETURN_KW => {
                    parts.push(ir::text("return"));
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
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

fn walk_import_list(node: &SyntaxNode) -> FormatIR {
    // Check whether this import list is wrapped in parens.
    let has_parens = node.elements().any(
        |child| matches!(child, NodeOrToken::Token(ref tok) if tok.kind() == SyntaxKind::L_PAREN),
    );

    let commented = node.children_with_tokens().any(|child| {
        child.kind().is_trivia()
            && !matches!(child.kind(), SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
    });
    if !has_parens && commented {
        return walk_tokens_inline(node);
    }
    if !has_parens {
        // `sqrt, pow` on the line when it fits, parenthesized when not.
        return ir::group(ir::if_break(
            walk_tokens_inline(node),
            parenthesized_imports(node),
        ));
    }
    parenthesized_imports(node)
}

/// `(`, one name per indented line, `)`. A comment that ends a line stays at
/// the end of that line, after `(` or after the name it follows; one on a line
/// of its own stays on its own line, before the next name.
fn parenthesized_imports(node: &SyntaxNode) -> FormatIR {
    // (comments on lines of their own before, name, comments after)
    let mut names: Vec<(Vec<FormatIR>, FormatIR, Vec<FormatIR>)> = Vec::new();
    let mut header = Vec::new();
    let mut before = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Node(name) => {
                names.push((std::mem::take(&mut before), walk_node(&name), Vec::new()))
            }
            NodeOrToken::Token(tok) if tok.kind().is_trivia() => {
                let comment = inline_comment(&tok);
                if !ends_a_line_of_code(&tok) {
                    before.push(comment);
                } else if let Some((_, _, after)) = names.last_mut() {
                    after.push(comment);
                } else {
                    header.push(comment);
                }
            }
            // The parens and the commas.
            NodeOrToken::Token(_) => {}
        }
    }

    let mut inner_parts = Vec::new();
    for (i, (before, name, after)) in names.iter().enumerate() {
        inner_parts.push(ir::hardline());
        for comment in before {
            inner_parts.push(comment.clone());
            inner_parts.push(ir::hardline());
        }
        inner_parts.push(name.clone());
        if i < names.len() - 1 {
            inner_parts.push(ir::text(","));
        }
        for comment in after {
            inner_parts.push(sp());
            inner_parts.push(comment.clone());
        }
    }
    // Comments on lines of their own after the last name.
    for comment in before {
        inner_parts.push(ir::hardline());
        inner_parts.push(comment);
    }

    let mut parts = vec![ir::text("(")];
    for comment in header {
        parts.push(sp());
        parts.push(comment);
    }
    parts.push(ir::indent(ir::concat(inner_parts)));
    parts.push(ir::hardline());
    parts.push(ir::text(")"));

    ir::concat(parts)
}

fn walk_from_import_decl(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                // `from`; the module is a PATH node.
                SyntaxKind::IDENT => {
                    parts.push(ir::text("from"));
                    parts.push(sp());
                }
                SyntaxKind::IMPORT_KW => {
                    parts.push(sp());
                    parts.push(ir::text("import"));
                    parts.push(sp());
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }

    ir::concat(parts)
}

// ── String expression ────────────────────────────────────────────────

// ── Field access ────────────────────────────────────────────────────

// ── Index expression ────────────────────────────────────────────────

// ── Impl definition ──────────────────────────────────────────────────

fn walk_impl_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::IMPL_KW => parts.extend([ir::text("impl"), sp()]),
                SyntaxKind::FOR_KW => parts.extend([sp(), ir::text("for"), sp()]),
                SyntaxKind::DO_KW => parts.extend([sp(), ir::text("do")]),
                SyntaxKind::END_KW => {}
                _ => add_token_with_context(&tok, &mut parts),
            },
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::BLOCK => {
                parts.push(ir::indent(walk_block_inner_items(&n)));
                parts.push(ir::hardline());
                parts.push(ir::text("end"));
            }
            // The interface, its type arguments and the type.
            NodeOrToken::Node(n) => parts.push(walk_node(&n)),
        }
    }
    ir::concat(parts)
}

// ── Type alias ──────────────────────────────────────────────────────

fn walk_type_alias_def(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::TYPE_KW => {
                    parts.push(ir::text("type"));
                    parts.push(sp());
                }
                SyntaxKind::EQ => {
                    parts.push(sp());
                    parts.push(ir::text("="));
                    parts.push(sp());
                }
                // A tuple type's commas: `(A, B)`.
                SyntaxKind::COMMA => {
                    parts.push(ir::text(","));
                    parts.push(sp());
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => match n.kind() {
                SyntaxKind::VISIBILITY => {
                    parts.push(walk_node(&n));
                    parts.push(sp());
                }
                _ => {
                    parts.push(walk_node(&n));
                }
            },
        }
    }

    ir::concat(parts)
}

// ── Literal pattern and struct update ──────────────────────────────

/// A literal pattern, as one token: `-1`, not `- 1`.
fn walk_literal_pat(node: &SyntaxNode) -> FormatIR {
    ir::concat(
        node.children_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| !token.kind().is_trivia())
            .map(|token| ir::text(token.text()))
            .collect(),
    )
}

/// `%{base | field: value, other: value}`, or the fields one per line under
/// `%{base |` when they do not fit or carry comments. A comment that ends the
/// `|` line or a field's line stays at its end; one on a line of its own stays
/// on its own line.
fn walk_struct_update(node: &SyntaxNode) -> FormatIR {
    let mut head = vec![ir::text("%{")];
    let mut header: Option<SyntaxToken> = None;
    let mut lines: Vec<(Option<FormatIR>, Option<SyntaxToken>)> = Vec::new();
    let mut past_bar = false;
    for element in node.children_with_tokens() {
        match element {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::BAR => past_bar = true,
                SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT if !past_bar => {
                    head.extend([inline_comment(&tok), sp()])
                }
                SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT => match lines.last_mut() {
                    Some((Some(_), comment @ None)) if ends_a_line_of_code(&tok) => {
                        *comment = Some(tok)
                    }
                    None if header.is_none() && ends_a_line_of_code(&tok) => header = Some(tok),
                    _ => lines.push((None, Some(tok))),
                },
                _ => {}
            },
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::STRUCT_LITERAL_FIELD => {
                lines.push((Some(walk_node(&n)), None))
            }
            NodeOrToken::Node(n) => head.push(walk_node(&n)),
        }
    }
    head.push(ir::text(" |"));

    let fields = lines.iter().filter(|(field, _)| field.is_some()).count();
    if header.is_none() && lines.iter().all(|(_, comment)| comment.is_none()) {
        let mut inner = Vec::new();
        for (i, (field, _)) in lines.into_iter().enumerate() {
            if i > 0 {
                inner.push(ir::text(","));
            }
            inner.push(ir::space());
            inner.extend(field);
        }
        head.extend([
            ir::indent(ir::concat(inner)),
            ir::if_break(FormatIR::Empty, ir::hardline()),
            ir::text("}"),
        ]);
        return ir::group(ir::concat(head));
    }

    if let Some(comment) = header {
        head.extend([sp(), ir::text(comment.text())]);
    }
    let mut inner = Vec::new();
    let mut seen = 0;
    for (field, comment) in lines {
        inner.push(ir::hardline());
        if let Some(field) = field {
            inner.push(field);
            seen += 1;
            if seen < fields {
                inner.push(ir::text(","));
            }
        }
        if let Some(comment) = comment {
            if !matches!(inner.last(), Some(FormatIR::Hardline)) {
                inner.push(sp());
            }
            inner.push(inline_comment(&comment));
        }
    }
    head.extend([ir::indent(ir::concat(inner)), ir::hardline(), ir::text("}")]);
    ir::concat(head)
}

// ── Variant definition ──────────────────────────────────────────────

fn walk_variant_def(node: &SyntaxNode) -> FormatIR {
    walk_tokens_inline(node)
}

// ── Receive expression ──────────────────────────────────────────────

// ── Spawn/Send/Link expressions ──────────────────────────────────────

// ── Self expression ──────────────────────────────────────────────────

fn walk_self_expr(node: &SyntaxNode) -> FormatIR {
    walk_tokens_inline(node)
}

// ── Call handler ──────────────────────────────────────────────────────

/// A service's `call Name(params) :: T do |state| ... end`,
/// `cast Name(params) do |state| ... end` and `terminate do ... end`.
fn walk_handler(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::CALL_KW | SyntaxKind::CAST_KW => {
                    parts.extend([ir::text(tok.text()), sp()])
                }
                SyntaxKind::BAR => {
                    if opens_state_param(&tok) {
                        parts.push(sp());
                    }
                    parts.push(ir::text("|"));
                }
                SyntaxKind::DO_KW => parts.extend([sp(), ir::text("do")]),
                SyntaxKind::END_KW => {}
                // `terminate`, the state's name and comments.
                _ => add_token_with_context(&tok, &mut parts),
            },
            NodeOrToken::Node(n) => match n.kind() {
                SyntaxKind::BLOCK => parts.push(body_then_end(&n)),
                SyntaxKind::TYPE_ANNOTATION => parts.extend([sp(), walk_node(&n)]),
                // The name and the parameters.
                _ => parts.push(walk_node(&n)),
            },
        }
    }
    ir::concat(parts)
}

/// The bar that opens a handler's `do |state|` parameter follows `do`.
fn opens_state_param(bar: &SyntaxToken) -> bool {
    std::iter::successors(bar.prev_token(), |token| token.prev_token())
        .find(|token| !token.kind().is_trivia())
        .is_some_and(|token| token.kind() == SyntaxKind::DO_KW)
}

// ── Cast handler ──────────────────────────────────────────────────────

// ── Terminate clause ──────────────────────────────────────────────────

// ── Struct literals and patterns, json literals ─────────────────────

/// `Point { x: 1, y }`, the pattern `Point { x, y: 0 }` and `json { a: 1 }`:
/// on one line when it fits, one field per line when it does not or when a
/// comment sits among the fields.
fn walk_braced_fields(node: &SyntaxNode) -> FormatIR {
    let mut prefix_parts = Vec::new();
    // Each field and each comment on a line of its own is a line; a comment
    // that ends a field's line stays after that field and its comma.
    let mut lines: Vec<(Option<FormatIR>, Option<SyntaxToken>)> = Vec::new();
    let mut saw_l_brace = false;

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::L_BRACE => saw_l_brace = true,
                SyntaxKind::R_BRACE | SyntaxKind::COMMA => {}
                kind if kind.is_trivia() && saw_l_brace => match lines.last_mut() {
                    Some((Some(_), comment @ None)) if ends_a_line_of_code(&tok) => {
                        *comment = Some(tok)
                    }
                    _ => lines.push((None, Some(tok))),
                },
                _ => add_token_with_context(&tok, &mut prefix_parts),
            },
            NodeOrToken::Node(n)
                if matches!(
                    n.kind(),
                    SyntaxKind::STRUCT_LITERAL_FIELD
                        | SyntaxKind::STRUCT_PAT_FIELD
                        | SyntaxKind::JSON_FIELD
                ) =>
            {
                lines.push((Some(walk_node(&n)), None))
            }
            NodeOrToken::Node(n) => prefix_parts.push(walk_node(&n)),
        }
    }

    let fields = lines.iter().filter(|(field, _)| field.is_some()).count();
    let commented = lines.iter().any(|(_, comment)| comment.is_some());
    // Without comments: on one line when it fits, like a list or a map.
    if !commented {
        let mut inner = Vec::new();
        for (i, (field, _)) in lines.into_iter().enumerate() {
            if i > 0 {
                inner.push(ir::text(","));
            }
            inner.push(ir::space());
            inner.extend(field);
        }
        prefix_parts.push(ir::text(" {"));
        prefix_parts.push(ir::indent(ir::concat(inner)));
        prefix_parts.push(ir::space());
        prefix_parts.push(ir::text("}"));
        return ir::group(ir::concat(prefix_parts));
    }

    let mut parts = prefix_parts;
    parts.push(ir::text(" {"));
    let mut inner_parts = Vec::new();
    let mut seen = 0;
    for (field, comment) in lines {
        inner_parts.push(ir::hardline());
        if let Some(field) = field {
            inner_parts.push(field);
            seen += 1;
            if seen < fields {
                inner_parts.push(ir::text(","));
            }
        }
        if let Some(comment) = comment {
            if inner_parts
                .last()
                .is_some_and(|last| !matches!(last, FormatIR::Hardline))
            {
                inner_parts.push(sp());
            }
            inner_parts.push(inline_comment(&comment));
        }
    }
    parts.push(ir::indent(ir::concat(inner_parts)));
    parts.push(ir::hardline());
    parts.push(ir::text("}"));
    ir::concat(parts)
}

// ── Map literal ─────────────────────────────────────────────────────

fn walk_map_literal(node: &SyntaxNode) -> FormatIR {
    // Keyword arguments, `f(x, name: v)`, parse as a map literal without `%{}`.
    if !node
        .children_with_tokens()
        .any(|element| element.kind() == SyntaxKind::PERCENT)
    {
        let mut parts = Vec::new();
        for child in node.elements() {
            match child {
                NodeOrToken::Node(n) => parts.push(walk_node(&n)),
                NodeOrToken::Token(tok) => match tok.kind() {
                    SyntaxKind::COMMA => {
                        parts.push(ir::text(","));
                        parts.push(ir::space());
                    }
                    _ => add_token_with_context(&tok, &mut parts),
                },
            }
        }
        return ir::concat(parts);
    }
    walk_delimited_items(node, "%{", "}")
}

// ── Map entry ───────────────────────────────────────────────────────

fn walk_map_entry(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::FAT_ARROW => {
                    parts.push(sp());
                    parts.push(ir::text("=>"));
                    parts.push(sp());
                }
                // A keyword argument, `name: value`.
                SyntaxKind::COLON => {
                    parts.push(ir::text(":"));
                    parts.push(sp());
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }
    ir::concat(parts)
}

// ── List literal ────────────────────────────────────────────────────

fn walk_list_literal(node: &SyntaxNode) -> FormatIR {
    walk_delimited_items(node, "[", "]")
}

/// `[a, b]` or `%{k => v}`: on one line when it fits, otherwise one element
/// per line between the delimiters, each keeping its trailing comment.
fn walk_delimited_items(node: &SyntaxNode, open: &str, close: &str) -> FormatIR {
    let mut items: Vec<FormatIR> = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Node(n) => items.push(walk_node(&n)),
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::COMMA => {
                    if let Some(last) = items.last_mut() {
                        append(last, ir::text(","));
                    }
                }
                SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT => match items.last_mut() {
                    Some(last) if ends_a_line_of_code(&tok) => append_comment(last, &tok),
                    _ => items.push(inline_comment(&tok)),
                },
                // The delimiters, whitespace and newlines are re-emitted.
                _ => {}
            },
        }
    }
    if items.is_empty() {
        return ir::text(format!("{open}{close}"));
    }
    let softline = || ir::if_break(FormatIR::Empty, ir::hardline());
    let mut inner = vec![softline()];
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            inner.push(ir::space());
        }
        inner.push(item);
    }
    ir::group(ir::concat(vec![
        ir::text(open),
        ir::indent(ir::concat(inner)),
        softline(),
        ir::text(close),
    ]))
}

// ── Associated type binding ─────────────────────────────────────────

fn walk_assoc_type_binding(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => match tok.kind() {
                SyntaxKind::TYPE_KW => {
                    parts.push(ir::text("type"));
                    parts.push(sp());
                }
                SyntaxKind::EQ => {
                    parts.push(sp());
                    parts.push(ir::text("="));
                    parts.push(sp());
                }
                _ => {
                    add_token_with_context(&tok, &mut parts);
                }
            },
            NodeOrToken::Node(n) => {
                parts.push(walk_node(&n));
            }
        }
    }
    ir::concat(parts)
}

// ── Walk block inner items ──────────────────────────────────────────

/// Walk the children of a BLOCK that contains items (fns, fields, etc.)
/// inside a module/actor/service/etc definition.
fn walk_block_inner_items(node: &SyntaxNode) -> FormatIR {
    let mut items: Vec<FormatIR> = Vec::new();

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) if tok.kind().is_trivia() => push_comment(&mut items, &tok),
            // Items separated by `;` go on lines of their own.
            NodeOrToken::Token(_) => {}
            NodeOrToken::Node(n) => items.push(walk_node(&n)),
        }
    }

    // Each item on a line of its own, with a blank line between two.
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

// ── Helper: walk tokens inline with smart spacing ────────────────────

/// Walk all tokens in a node, emitting them with appropriate spacing.
fn walk_tokens_inline(node: &SyntaxNode) -> FormatIR {
    let mut parts = Vec::new();
    // Nothing goes between an opening paren or bracket and what it encloses
    // (`Some(x)`, not `Some( x)`), nor inside the angle brackets of a generic
    // list (`Map<String, Int>`). Braces stay spaced on both sides, since a
    // space always precedes `}`: `json { id: 7 }`, not `json {id: 7 }`.
    let mut after_open = false;
    let angles = matches!(
        node.kind(),
        SyntaxKind::GENERIC_ARG_LIST | SyntaxKind::GENERIC_PARAM_LIST
    );
    // In a type, `(` opens a tuple type (`:: (Int, Int)`, `Map<K, (A, B)>`)
    // and is spaced like a name, except as `Fun(`'s parameter list.
    let in_type = matches!(
        node.kind(),
        SyntaxKind::TYPE_ANNOTATION | SyntaxKind::GENERIC_ARG_LIST | SyntaxKind::FUN_TYPE
    );
    let mut prev_kind = None;

    for child in node.elements() {
        match child {
            NodeOrToken::Token(tok) => {
                let kind = tok.kind();
                if kind.is_trivia() {
                    add_token_with_context(&tok, &mut parts);
                    continue;
                }
                let closes_angles = angles && kind == SyntaxKind::GT;
                // `head :: tail`; elsewhere `::` belongs to a type annotation,
                // whose caller spaces it.
                let spaced = needs_space_before(kind)
                    || (kind == SyntaxKind::COLON_COLON && node.kind() == SyntaxKind::CONS_PAT)
                    || (in_type
                        && kind == SyntaxKind::L_PAREN
                        && prev_kind != Some(SyntaxKind::IDENT));
                if !parts.is_empty() && !after_open && !closes_angles && spaced {
                    parts.push(sp());
                }
                parts.push(ir::text(tok.text()));
                prev_kind = Some(kind);
                // `!` here is only ever the result sugar, `Int!String`; a
                // name follows its qualifier's `.` directly (`Shape.Circle`).
                after_open = matches!(
                    kind,
                    SyntaxKind::L_PAREN
                        | SyntaxKind::L_BRACKET
                        | SyntaxKind::BANG
                        | SyntaxKind::DOT
                ) || (angles && kind == SyntaxKind::LT);
            }
            NodeOrToken::Node(n) => {
                if !parts.is_empty() && !after_open && needs_space_before_node(n.kind()) {
                    parts.push(sp());
                }
                parts.push(walk_node(&n));
                after_open = false;
                prev_kind = None;
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
            | SyntaxKind::COLON
            | SyntaxKind::COLON_COLON
            | SyntaxKind::QUESTION
            | SyntaxKind::BANG
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
            | SyntaxKind::GENERIC_ARG_LIST
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
/// A comment emitted inside a line of code. A line comment runs to the end of
/// its line, so the line must end after it or the code that follows would be
/// commented out; a `#= ... =#` block comment can stay inline.
fn inline_comment(tok: &SyntaxToken) -> FormatIR {
    if tok.text().starts_with("#=") {
        ir::text(tok.text())
    } else {
        ir::concat(vec![ir::text(tok.text()), ir::line_end()])
    }
}

/// Add a comment to a list of lines. One that ends a line of code
/// (`x :: Int # why`) stays at the end of that line; one on a line of its own
/// is a line of its own.
fn push_comment(items: &mut Vec<FormatIR>, tok: &SyntaxToken) {
    match items.last_mut() {
        Some(last) if ends_a_line_of_code(tok) => append_comment(last, tok),
        _ => items.push(ir::text(tok.text())),
    }
}

/// Add a comment in a construct's body. One that ends the header's line
/// (`do # why`) stays there; the others are lines of the body.
fn push_body_comment(header: &mut Vec<FormatIR>, lines: &mut Vec<FormatIR>, tok: &SyntaxToken) {
    if lines.is_empty() && ends_a_line_of_code(tok) {
        header.push(sp());
        header.push(inline_comment(tok));
    } else {
        push_comment(lines, tok);
    }
}

/// Whether a comma is the last thing before its list's closing bracket,
/// comments aside.
fn is_trailing_comma(comma: &SyntaxToken) -> bool {
    std::iter::successors(comma.next_sibling_or_token(), |e| e.next_sibling_or_token())
        .find(|e| !e.kind().is_trivia())
        .is_some_and(|e| {
            matches!(
                e.kind(),
                SyntaxKind::R_PAREN | SyntaxKind::R_BRACKET | SyntaxKind::R_BRACE
            )
        })
}

fn ends_a_line_of_code(comment: &SyntaxToken) -> bool {
    let mut prev = comment.prev_token();
    while prev
        .as_ref()
        .is_some_and(|t| t.kind() == SyntaxKind::WHITESPACE)
    {
        prev = prev.and_then(|t| t.prev_token());
    }
    prev.is_some_and(|t| t.kind() != SyntaxKind::NEWLINE)
}

fn append_comment(line: &mut FormatIR, comment: &SyntaxToken) {
    append(line, ir::concat(vec![sp(), inline_comment(comment)]));
}

/// Add `more` to the end of `line`.
fn append(line: &mut FormatIR, more: FormatIR) {
    let code = std::mem::replace(line, FormatIR::Empty);
    *line = ir::concat(vec![code, more]);
}

fn add_token_with_context(tok: &SyntaxToken, parts: &mut Vec<FormatIR>) {
    let kind = tok.kind();
    if kind == SyntaxKind::COMMENT
        || kind == SyntaxKind::DOC_COMMENT
        || kind == SyntaxKind::MODULE_DOC_COMMENT
    {
        // One that ends a line of code after a list's breakable space
        // (`a, # why`) stays on that line, before the break.
        if matches!(parts.last(), Some(FormatIR::Space)) && ends_a_line_of_code(tok) {
            parts.pop();
            parts.extend([sp(), inline_comment(tok), ir::space()]);
            return;
        }
        // Otherwise after a breakable space the comment starts the line the
        // space breaks into; after a space it needs no other.
        if !matches!(parts.last(), None | Some(FormatIR::Space))
            && !matches!(parts.last(), Some(FormatIR::Text(text)) if text == " ")
        {
            parts.push(sp());
        }
        parts.push(inline_comment(tok));
        return;
    }
    parts.push(ir::text(tok.text()));
}

#[cfg(test)]
mod tests {
    use crate::format_source;
    use crate::printer::FormatConfig;

    fn fmt(source: &str) -> String {
        format_source(source, &FormatConfig::default())
    }

    #[test]
    fn annotated_pattern_parameters() {
        assert_eq!(
            fmt("fn f(0::Int)=1\nfn f((s,n)  ::  borrow (SecretBytes,Int))=n\nfn f(h::t::List<Int>)=h"),
            "fn f(0 :: Int) = 1\nfn f((s, n) :: borrow (SecretBytes, Int)) = n\nfn f(h :: t :: List<Int>) = h\n"
        );
    }

    #[test]
    fn struct_patterns_fit_on_one_line_or_take_a_line_per_field() {
        assert_eq!(
            fmt("case p do\nPoint{x:0,y} -> y\nGeo.Point {  x ,  y: (a,_)  } -> x\nend"),
            "case p do\n  Point { x: 0, y } -> y\n  Geo.Point { x, y: (a, _) } -> x\nend\n"
        );
        let long = "let Account { identifier: identifier, display_name: display_name, created_at: created, flags: f, owner_id: o } = a";
        assert_eq!(
            fmt(long),
            "let Account {\n  identifier: identifier,\n  display_name: display_name,\n  created_at: created,\n  flags: f,\n  owner_id: o\n} = a\n"
        );
    }

    /// An actor's body is statements, one per line like a function's; it
    /// had a blank line after each, as the definitions in a module do.
    #[test]
    fn actor_bodies_have_a_statement_per_line() {
        assert_eq!(
            fmt("actor a() do\nlet x = 1\n\nprintln(\"x\")\nterminate do\nprintln(\"bye\")\nend\nend"),
            "actor a() do\n  let x = 1\n  println(\"x\")\n  terminate do\n    println(\"bye\")\n  end\nend\n"
        );
        assert_eq!(fmt("actor b() do\nend"), "actor b() do\nend\n");
    }

    #[test]
    fn qualified_constructor_patterns_keep_their_dot() {
        assert_eq!(
            fmt("case s do\nShape.Circle(r) -> r\nShape.Dot -> 0\nend"),
            "case s do\n  Shape.Circle(r) -> r\n  Shape.Dot -> 0\nend\n"
        );
    }

    #[test]
    fn a_struct_literal_is_on_one_line_when_it_fits() {
        assert_eq!(
            fmt("let p = Some(Point {\nx: 1,\ny: 2\n})"),
            "let p = Some(Point { x: 1, y: 2 })\n"
        );
        assert_eq!(fmt("let e = Empty {}"), "let e = Empty { }\n");
        let long = format!("let p = Point {{ name: \"{}\", y: 2 }}", "a".repeat(90));
        assert_eq!(
            fmt(&long),
            format!(
                "let p = Point {{\n  name: \"{}\",\n  y: 2\n}}\n",
                "a".repeat(90)
            )
        );
    }

    #[test]
    fn the_clauses_of_a_function_stay_together() {
        assert_eq!(
            fmt("fn fib(0) = 0\nfn fib(1) = 1\nfn fib(n) = fib(n - 1) + fib(n - 2)\nfn g() = 1\n"),
            "fn fib(0) = 0\nfn fib(1) = 1\nfn fib(n) = fib(n - 1) + fib(n - 2)\n\nfn g() = 1\n"
        );
        // Unless the source put a blank line between them.
        assert_eq!(
            fmt("fn h(0) = 0\n\nfn h(n) = n # n\nfn h2() = 1\n"),
            "fn h(0) = 0\n\nfn h(n) = n # n\n\nfn h2() = 1\n"
        );
    }

    #[test]
    fn a_map_entry_binding_keeps_comments_and_drops_a_trailing_comma() {
        assert_eq!(
            fmt("fn f(m) do\nfor {k,v,} in m do\nk\nend\nend"),
            "fn f(m) do\n  for {k, v} in m do\n    k\n  end\nend\n"
        );
        assert_eq!(
            fmt("fn f(m) do\nfor {k, # key\nv} in m do\nk\nend\nend"),
            "fn f(m) do\n  for {k, # key\n    v} in m do\n    k\n  end\nend\n"
        );
    }

    #[test]
    fn arms_separated_by_semicolons_go_on_lines_of_their_own() {
        assert_eq!(
            fmt("fn f(x) do\ncase x do 1 -> 2; _ -> 4 end\nend"),
            "fn f(x) do\n  case x do\n    1 -> 2\n    _ -> 4\n  end\nend\n"
        );
    }

    #[test]
    fn items_separated_by_semicolons_go_on_lines_of_their_own() {
        assert_eq!(
            fmt("type T do\nA; B\nend\n\nstruct S do\na :: Int; b :: Int\nend"),
            "type T do\n  A\n  B\nend\n\nstruct S do\n  a :: Int\n  b :: Int\nend\n"
        );
        assert_eq!(
            fmt("interface I do\nfn f(self) -> Int; fn g(self) -> Int\nend"),
            "interface I do\n  fn f(self) -> Int\n\n  fn g(self) -> Int\nend\n"
        );
        for header in ["module M", "impl I for Int"] {
            assert_eq!(
                fmt(&format!("{header} do\nfn f() = 1; fn g() = 2\nend")),
                format!("{header} do\n  fn f() = 1\n\n  fn g() = 2\nend\n")
            );
        }
    }

    #[test]
    fn a_deriving_clause_keeps_comments_and_drops_a_trailing_comma() {
        assert_eq!(
            fmt("type T do\nA\nend deriving( Eq , Show, )"),
            "type T do\n  A\nend deriving(Eq, Show)\n"
        );
        assert_eq!(
            fmt("struct S do\na :: Int\nend deriving(Eq, # c\nShow)"),
            "struct S do\n  a :: Int\nend deriving(Eq, # c\n  Show)\n"
        );
    }

    #[test]
    fn a_child_spec_has_a_pair_per_line() {
        assert_eq!(
            fmt("supervisor S do\nchild w do # w\nstart:   fn -> 1 end; restart: permanent #  note:  x\nshutdown: brutal_kill\nend\nend"),
            "supervisor S do\n  child w do # w\n    start: fn -> 1 end\n    restart: permanent #  note:  x\n    shutdown: brutal_kill\n  end\nend\n"
        );
        // A value over several lines keeps its own layout.
        assert_eq!(
            fmt("supervisor S do\nchild w do\nstart: fn -> case 1 do\n1 -> 2\nend end\nend\nend"),
            "supervisor S do\n  child w do\n    start: fn -> case 1 do\n      1 -> 2\n    end end\n  end\nend\n"
        );
    }

    #[test]
    fn closure_clauses_keep_guards_parens_and_do_bodies() {
        assert_eq!(
            fmt("fn main() do\nlet f = fn (x) when x > 0 -> x | (y) -> 0 - y end\nf\nend"),
            "fn main() do\n  let f = fn(x) when x > 0 -> x | (y) -> 0 - y end\n  f\nend\n"
        );
        assert_eq!(
            fmt("fn main() do\nlet f = fn 0 -> 1 | (n) do\nn\nend end\nlet g = fn 0 -> 1 | n do\nlet m = n\nm\nend end\ng\nend"),
            "fn main() do\n  let f = fn 0 -> 1 | (n) do n end end\n  let g = fn 0 -> 1 | n do\n    let m = n\n    m\n  end end\n  g\nend\n"
        );
    }

    #[test]
    fn a_closure_is_spelled_like_a_function_type() {
        assert_eq!(
            fmt("fn main() do\nlet f = fn (x, y) -> x end\nlet g = fn x -> x end\nfn () do 1 end\nend"),
            "fn main() do\n  let f = fn(x, y) -> x end\n  let g = fn x -> x end\n  fn() do 1 end\nend\n"
        );
    }

    #[test]
    fn a_comment_ending_a_header_or_list_line_stays_there() {
        for (source, formatted) in [
            (
                "case x do # c\n1 -> 2\nend",
                "case x do # c\n  1 -> 2\nend\n",
            ),
            (
                "struct S do # c\na :: Int\nend",
                "struct S do # c\n  a :: Int\nend\n",
            ),
            (
                "from Bar import ( # c\na, # a\n# own\nb\n# last\n)",
                "from Bar import ( # c\n  a, # a\n  # own\n  b\n  # last\n)\n",
            ),
            ("fn f(a, # a\nb) = a", "fn f(a, # a\n  b) = a\n"),
            ("let t = (1, # c\n2)", "let t = (1, # c\n  2)\n"),
            (
                "case p do\nP { x, # c\ny } -> x\nend",
                "case p do\n  P {\n    x, # c\n    y\n  } -> x\nend\n",
            ),
        ] {
            assert_eq!(fmt(source), formatted, "{source}");
        }
    }

    #[test]
    fn fields_separated_by_new_lines_get_commas() {
        assert_eq!(
            fmt("case p do\nPoint {\nx\ny: 0\n} -> x\nend"),
            "case p do\n  Point { x, y: 0 } -> x\nend\n"
        );
        assert_eq!(
            fmt("let p = Point {\nx: 1\ny: 2\n}"),
            "let p = Point { x: 1, y: 2 }\n"
        );
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
    fn case_arm_do_block() {
        let result = fmt(
            "case outcome do\nReject -> do\nlet value = recover(outcome)?\nOk(value)\nend\n_ -> Ok(outcome)\nend",
        );
        assert_eq!(
            result,
            "case outcome do\n  Reject -> do\n    let value = recover(outcome)?\n    Ok(value)\n  end\n  _ -> Ok(outcome)\nend\n"
        );
    }

    #[test]
    fn nested_trailing_closure_blocks() {
        let result = fmt("describe(\"paper\") do\ntest(\"fills\") do\nassert(true)\nend\nend");
        assert_eq!(
            result,
            "describe(\"paper\") do\n  test(\"fills\") do\n    assert(true)\n  end\nend\n"
        );
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
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn idempotent_fn_def() {
        let src = "fn add(a, b) do\na + b\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn idempotent_if_else() {
        let src = "if x > 0 do\nx\nelse\n-x\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn idempotent_case() {
        let src = "case x do\n1 -> \"one\"\n2 -> \"two\"\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn idempotent_module() {
        let src = "module Math do\nfn add(a, b) do\na + b\nend\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
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
    fn top_level_imports_stay_compact() {
        let result = fmt("from Foo import bar\nfrom Baz import qux\nfn main() do\n1\nend");
        assert_eq!(
            result,
            "from Foo import bar\nfrom Baz import qux\n\nfn main() do\n  1\nend\n"
        );
    }

    #[test]
    fn top_level_comment_block_stays_compact_before_imports() {
        let result = fmt("# one\n# two\nfrom Foo import bar\nfrom Baz import qux");
        assert_eq!(
            result,
            "# one\n# two\nfrom Foo import bar\nfrom Baz import qux\n"
        );
    }

    #[test]
    fn top_level_comments_keep_their_blank_lines() {
        // A comment written directly above a declaration stays attached to it,
        // and a blank line between two comment blocks stays.
        formats_to(
            "# header\n\n## About foo.\nfn foo() do\n1\nend\n# loose\n\nfn bar() do\n2\nend",
            "# header\n\n## About foo.\nfn foo() do\n  1\nend\n\n# loose\n\nfn bar() do\n  2\nend\n",
        );
    }

    #[test]
    fn pipe_expression() {
        let result = fmt("fn foo() do\nlet q = source() |> step_one() |> step_two()\nq\nend");
        assert_eq!(
            result,
            "fn foo() do\n  let q = source()\n    |> step_one()\n    |> step_two()\n  q\nend\n"
        );
    }

    #[test]
    fn pipe_with_closure_and_struct_literal_breaks_cleanly() {
        let result = fmt(
            "fn foo() do\nOk(rows |> List.map(fn (row) do Organization { id: Map.get(row, \"id\"), name: Map.get(row, \"name\"), slug: Map.get(row, \"slug\"), created_at: Map.get(row, \"created_at\") } end))\nend",
        );
        assert_eq!(
            result,
            "fn foo() do\n  Ok(rows\n    |> List.map(fn(row) do\n      Organization {\n        id: Map.get(row, \"id\"),\n        name: Map.get(row, \"name\"),\n        slug: Map.get(row, \"slug\"),\n        created_at: Map.get(row, \"created_at\")\n      }\n    end))\nend\n"
        );
    }

    #[test]
    fn call_with_args() {
        let result = fmt("foo(1, 2, 3)");
        assert_eq!(result, "foo(1, 2, 3)\n");
    }

    #[test]
    fn line_comments_inside_expressions_never_swallow_code() {
        // These comments used to be printed inline, commenting out the code after them.
        let result = fmt(
            "fn main() do\nlet xs = [\n1, # first\n2\n]\nlet s = add(\n1, # left\n2\n)\nlet j = json {\n# the id\nid: 7\n}\nxs\nend",
        );
        assert_eq!(
            result,
            "fn main() do\n  let xs = [\n    1, # first\n    2\n  ]\n  let s = add(1, # left\n    2)\n  let j = json {\n    # the id\n    id: 7\n  }\n  xs\nend\n"
        );
    }

    #[test]
    fn decorators_keep_their_spelling_and_line() {
        // `@cluster pub fn` used to become `@ clusterpub fn`, which does not parse.
        let source = "@cluster pub fn add() -> Int do\n  1\nend\n\n@cluster(3)\npub fn sync() -> Int do\n  3\nend\n\n@native(\"mesh_math_add\")\npub fn native_add(a :: Int, b :: Int) -> Int\n\n@display\n@export(\"mesh_show\")\npub fn show(request :: Bytes) -> Plaintext<Bytes>!String do\n  Ok(Plaintext.from(request))\nend\n";
        assert_eq!(fmt(source), source);
    }

    #[test]
    fn long_json_literals_break_one_field_per_line() {
        let result = fmt("fn health() do\njson { status: \"ok\", backend: \"postgres\", migrations: \"meshc migrate\", handler: \"Work.sync_todos\" }\nend");
        assert_eq!(
            result,
            "fn health() do\n  json {\n    status: \"ok\",\n    backend: \"postgres\",\n    migrations: \"meshc migrate\",\n    handler: \"Work.sync_todos\"\n  }\nend\n"
        );
    }

    #[test]
    fn types_operators_and_trailing_comments_keep_their_documented_spelling() {
        let source = "fn f<T>(r :: Map<String, List<Int?>>) -> Int!String where T: Display do\n  let v = g(r)? # checked\n  case v do\n    Ok(x) # passes through\n    Err(e) -> Err(e)\n  end\nend\n\ntype Step<T> do\n  Done(T)\n  Failed(String)\nend\n";
        assert_eq!(fmt(source), source);
        let pipeline = "fn f(x :: Int) -> Int do\n  x\n    |> add(1) # step one\n    # before two\n    |> add(2)\nend\n";
        assert_eq!(fmt(pipeline), pipeline);
        // What earlier versions printed is repaired.
        assert_eq!(
            fmt("fn f(x :: Option < Int >) -> Int ! String do\n  g(x) ?\nend"),
            "fn f(x :: Option<Int>) -> Int!String do\n  g(x)?\nend\n"
        );
    }

    #[test]
    fn a_list_that_does_not_fit_has_one_element_per_line() {
        let source = "fn f() do\n  let accounts = [\n    checked_account_meta(source, false, true)?,\n    checked_account_meta(mint, false, false)?, # the mint\n    checked_account_meta(destination, false, true)?\n  ]\n  let short = [1, 2, 3]\n  let m = %{\"a\" => 1}\n  accounts\nend\n";
        assert_eq!(fmt(source), source);
        assert_eq!(
            fmt("fn f() do\n  [checked_account_meta(source, false, true)?, checked_account_meta(mint, false, false)?, owner_of(source)]\nend"),
            "fn f() do\n  [\n    checked_account_meta(source, false, true)?,\n    checked_account_meta(mint, false, false)?,\n    owner_of(source)\n  ]\nend\n"
        );
    }

    #[test]
    fn wrapped_arguments_sit_one_level_in() {
        // Before, the broken lines of an argument list landed at the call's own
        // indent, level with the statement they continue.
        let result = fmt("fn f() do\n  let presented = present_message(input.database_path, input.group_id, input.body, input.attachment_list)?\nend");
        assert_eq!(
            result,
            "fn f() do\n  let presented = present_message(input.database_path,\n    input.group_id,\n    input.body,\n    input.attachment_list)?\nend\n"
        );
    }

    #[test]
    fn json_literal_braces_stay_balanced() {
        let result = fmt("fn foo() do\nlet j = json { id: 7, ok: Some(x) }\nj\nend");
        assert_eq!(
            result,
            "fn foo() do\n  let j = json { id: 7, ok: Some(x) }\n  j\nend\n"
        );
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
    fn walk_path_preserves_dotted_import_and_impl_paths() {
        let single_line_import = fmt("from Api.Router import build_router");
        assert_eq!(single_line_import, "from Api.Router import build_router\n");

        let multiline_import = fmt("from Api.Router import (\nbuild_router,\nhealth_router\n)");
        assert_eq!(
            multiline_import,
            "from Api.Router import (\n  build_router,\n  health_router\n)\n"
        );

        let qualified_impl = fmt("impl Foo.Bar for Baz.Qux do\nfn run(self) do\nself\nend\nend");
        assert_eq!(
            qualified_impl,
            "impl Foo.Bar for Baz.Qux do\n  fn run(self) do\n    self\n  end\nend\n"
        );
    }

    #[test]
    fn let_with_type_annotation() {
        let result = fmt("let name :: String = \"hello\"");
        assert_eq!(result, "let name :: String = \"hello\"\n");
    }

    #[test]
    fn pub_type_alias() {
        let result = fmt("pub type UserId = Int");
        assert_eq!(result, "pub type UserId = Int\n");
    }

    #[test]
    fn pub_sum_type_keeps_visibility_spacing() {
        let result = fmt("pub type Severity do\nFatal\nend");
        assert_eq!(result, "pub type Severity do\n  Fatal\nend\n");
    }

    #[test]
    fn schema_option_table_keeps_space_before_string_literal() {
        let result = fmt("pub struct Person do\ntable \"people\"\nend deriving(Schema)");
        assert_eq!(
            result,
            "pub struct Person do\n  table \"people\"\nend deriving(Schema)\n"
        );
    }

    #[test]
    fn idempotent_struct() {
        let src = "struct Point do\nx :: Float\ny :: Float\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn typed_fn_def() {
        let result = fmt("fn typed(x :: Int, y :: Int) -> Int do\nx + y\nend");
        assert_eq!(
            result,
            "fn typed(x :: Int, y :: Int) -> Int do\n  x + y\nend\n"
        );
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
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn fn_expr_body_guard_idempotent() {
        let src = "fn abs(n) when n < 0 = -n";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn multi_clause_fn_formatted() {
        let src = "fn fib(0) = 0\nfn fib(1) = 1\nfn fib(n) = fib(n - 1) + fib(n - 2)";
        let result = fmt(src);
        assert!(result.contains("fn fib(0) = 0"), "Result: {:?}", result);
        assert!(result.contains("fn fib(1) = 1"), "Result: {:?}", result);
        assert!(
            result.contains("fn fib(n) = fib(n - 1) + fib(n - 2)"),
            "Result: {:?}",
            result
        );
    }

    #[test]
    fn while_loop() {
        let result = fmt("while true do\nbreak\nend");
        assert_eq!(result, "while true do\n  break\nend\n");
    }

    #[test]
    fn while_loop_with_body() {
        let result = fmt("while x > 0 do\nprintln(x)\nend");
        assert_eq!(result, "while x > 0 do\n  println(x)\nend\n");
    }

    #[test]
    fn while_loop_idempotent() {
        let src = "while true do\nbreak\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn break_in_while() {
        let result = fmt("while true do\nbreak\nend");
        assert!(
            result.contains("break"),
            "Result should contain break: {:?}",
            result
        );
    }

    #[test]
    fn continue_in_while() {
        let result = fmt("while true do\ncontinue\nend");
        assert!(
            result.contains("continue"),
            "Result should contain continue: {:?}",
            result
        );
    }

    // ── For-in expression tests ─────────────────────────────────────

    #[test]
    fn for_in_range_basic() {
        let result = fmt("for i in 0..10 do\nprintln(i)\nend");
        assert_eq!(result, "for i in 0..10 do\n  println(i)\nend\n");
    }

    #[test]
    fn for_in_range_idempotent() {
        let src = "for i in 0..10 do\nprintln(i)\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn for_in_range_normalize_whitespace() {
        // Extra spaces should be normalized
        let result = fmt("for  i  in  0..10  do\nprintln(i)\nend");
        assert_eq!(result, "for i in 0..10 do\n  println(i)\nend\n");
    }

    #[test]
    fn for_in_destructure_binding() {
        // Map destructuring: for {k, v} in m do body end
        let result = fmt("for {k, v} in m do\nprintln(v)\nend");
        assert_eq!(result, "for {k, v} in m do\n  println(v)\nend\n");
    }

    #[test]
    fn for_in_destructure_binding_idempotent() {
        let src = "for {k, v} in m do\nprintln(v)\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    // ── For-in with when filter clause tests ──────────────────────────

    #[test]
    fn for_in_filter_basic() {
        let result = fmt("for x in list when x > 0 do\nx\nend");
        assert_eq!(result, "for x in list when x > 0 do\n  x\nend\n");
    }

    #[test]
    fn for_in_filter_basic_idempotent() {
        let src = "for x in list when x > 0 do\nx\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn for_in_filter_range() {
        let result = fmt("for i in 0..10 when i % 2 == 0 do\ni\nend");
        assert_eq!(result, "for i in 0..10 when i % 2 == 0 do\n  i\nend\n");
    }

    #[test]
    fn for_in_filter_range_idempotent() {
        let src = "for i in 0..10 when i % 2 == 0 do\ni\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn for_in_filter_destructure() {
        let result = fmt("for {k, v} in map when v > 0 do\nk\nend");
        assert_eq!(result, "for {k, v} in map when v > 0 do\n  k\nend\n");
    }

    #[test]
    fn for_in_filter_destructure_idempotent() {
        let src = "for {k, v} in map when v > 0 do\nk\nend";
        let first = fmt(src);
        let second = fmt(&first);
        assert_eq!(
            first, second,
            "Idempotency failed.\nFirst: {:?}\nSecond: {:?}",
            first, second
        );
    }

    #[test]
    fn map_literal_formatting() {
        let result = fmt("%{\"a\" => 1, \"b\" => 2}");
        assert!(result.contains("=>"), "Result: {:?}", result);
        assert!(result.contains("%{"), "Result: {:?}", result);
    }

    #[test]
    fn list_literal_formatting() {
        let result = fmt("[1, 2, 3]");
        assert_eq!(result, "[1, 2, 3]\n");
    }

    #[test]
    fn empty_list_literal() {
        let result = fmt("[]");
        assert_eq!(result, "[]\n");
    }

    #[test]
    fn empty_map_literal() {
        let result = fmt("%{}");
        assert_eq!(result, "%{}\n");
    }

    #[test]
    fn from_import_paren_single_line() {
        let result = fmt("from Math import (sqrt, pow)");
        assert_eq!(
            result, "from Math import (\n  sqrt,\n  pow\n)\n",
            "Parenthesized imports should format with one name per indented line"
        );
    }

    #[test]
    fn from_import_paren_multiline() {
        let result = fmt("from Math import (\n  sqrt,\n  pow\n)");
        assert_eq!(
            result, "from Math import (\n  sqrt,\n  pow\n)\n",
            "Multiline parenthesized imports should preserve structure"
        );
    }

    #[test]
    fn from_import_paren_trailing_comma() {
        let result = fmt("from Math import (\n  sqrt,\n  pow,\n)");
        assert_eq!(
            result, "from Math import (\n  sqrt,\n  pow\n)\n",
            "Trailing comma in parenthesized imports should be cleaned up"
        );
    }

    #[test]
    fn trailing_comma_arg_list() {
        let result = fmt("fn main() do\n  add(1, 2,)\nend");
        assert!(
            !result.contains(", )"),
            "Trailing comma before ) should not produce extra space. Got: {:?}",
            result
        );
        assert!(
            result.contains(",)"),
            "Trailing comma should be preserved but without space before ). Got: {:?}",
            result
        );
    }

    /// `try_format`, so a refusal fails the test instead of passing the input through.
    fn formats_to(source: &str, expected: &str) {
        let formatted = crate::try_format(source, &FormatConfig::default()).expect("formats");
        assert_eq!(formatted, expected);
    }

    #[test]
    fn match_keeps_its_keyword_and_cons_patterns_are_spaced() {
        formats_to(
            "fn f(x) do\nmatch x do\nSome(head::tail) -> head\n_ -> 0\nend\nend",
            "fn f(x) do\n  match x do\n    Some(head :: tail) -> head\n    _ -> 0\n  end\nend\n",
        );
    }

    #[test]
    fn keyword_arguments_stay_keyword_arguments() {
        formats_to(
            "fn main() do\nrequest(\"/events\", method:\"POST\", kind: :json)\nend",
            "fn main() do\n  request(\"/events\", method: \"POST\", kind: :json)\nend\n",
        );
    }

    #[test]
    fn semicolon_separated_statements_get_their_own_lines() {
        formats_to(
            "fn main() do\nlet a = 1; let b = 2\na + b;\nend",
            "fn main() do\n  let a = 1\n  let b = 2\n  a + b\nend\n",
        );
    }

    #[test]
    fn closure_clauses_are_spaced() {
        formats_to(
            "let c = fn 0 -> \"zero\"|n when n > 0 -> \"positive\"|_ -> \"other\" end",
            "let c = fn 0 -> \"zero\" | n when n > 0 -> \"positive\" | _ -> \"other\" end\n",
        );
    }

    #[test]
    fn handler_state_parameter_follows_do_with_a_space() {
        formats_to(
            "service S do\nfn init() -> Int do\n0\nend\ncall Get() :: Int do|s|\n(s, s)\nend\ncast Reset() do|_s|\n0\nend\nend",
            "service S do\n  fn init() -> Int do\n    0\n  end\n\n  call Get() :: Int do |s|\n    (s, s)\n  end\n\n  cast Reset() do |_s|\n    0\n  end\nend\n",
        );
    }

    #[test]
    fn tuple_types_are_spaced_like_other_types() {
        formats_to(
            "fn f(x :: (Int, Int), m :: Map<String, (Int, Int)>, cb :: Fun((Int, Int)) -> (Int, Int)) -> (Int, String)? do\nx\nend",
            // The return type counts toward the line: the parameters break.
            "fn f(x :: (Int, Int),\n  m :: Map<String, (Int, Int)>,\n  cb :: Fun((Int, Int)) -> (Int, Int)) -> (Int, String)? do\n  x\nend\n",
        );
        formats_to(
            "let p :: (Int, (Int, Int))!String = x",
            "let p :: (Int, (Int, Int))!String = x\n",
        );
    }

    #[test]
    fn long_operator_chains_break_before_each_operator() {
        formats_to(
            "fn f() do\nif first_condition_value_here == 1 && second_condition_value_here == 2 && third_condition_value_here - 3 == 4 do\n1\nelse\n2\nend\nend",
            "fn f() do\n  if first_condition_value_here == 1\n    && second_condition_value_here == 2\n    && third_condition_value_here - 3 == 4 do\n    1\n  else\n    2\n  end\nend\n",
        );
        formats_to("fn f() do\na && b\nend", "fn f() do\n  a && b\nend\n");
    }

    #[test]
    fn long_imports_and_struct_updates_break_one_item_per_line() {
        formats_to(
            "from Config import database_url_key, port_key, todo_rate_limit_max_requests_key, missing_required_env_key",
            "from Config import (\n  database_url_key,\n  port_key,\n  todo_rate_limit_max_requests_key,\n  missing_required_env_key\n)\n",
        );
        formats_to("from Math import sqrt, pow", "from Math import sqrt, pow\n");
        formats_to(
            "fn f(state) do\n%{state | version: 2, epoch: commit_epoch, transcript_hash: transcript_hash, previous_chain_length: previous, sent: 0}\nend",
            "fn f(state) do\n  %{state |\n    version: 2,\n    epoch: commit_epoch,\n    transcript_hash: transcript_hash,\n    previous_chain_length: previous,\n    sent: 0\n  }\nend\n",
        );
        formats_to(
            "fn f(s) do\n%{s | a: 1, b: 2}\nend",
            "fn f(s) do\n  %{s | a: 1, b: 2}\nend\n",
        );
        formats_to(
            "fn f(s) do\n%{s | # c\na: 1, # c\n# own line\nb: 2 # c\n}\nend",
            "fn f(s) do\n  %{s | # c\n    a: 1, # c\n    # own line\n    b: 2 # c\n  }\nend\n",
        );
    }

    #[test]
    fn receive_after_clause_is_an_arm_and_end_closes_the_block() {
        formats_to(
            "actor w() do\nreceive do\nm -> m\nafter 10 -> 0\nend\nend",
            "actor w() do\n  receive do\n    m -> m\n    after 10 -> 0\n  end\nend\n",
        );
    }
}
