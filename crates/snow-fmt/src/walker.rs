//! CST-to-FormatIR walker for Snow source code.
//!
//! This module walks the rowan CST produced by `snow-parser` and converts it
//! into a `FormatIR` document tree. The walker processes all tokens including
//! trivia (comments, newlines) to preserve them in the formatted output.

use snow_parser::SyntaxNode;

use crate::ir::FormatIR;

/// Walk a CST node and produce a FormatIR document tree.
///
/// This is the main entry point for converting a parsed Snow syntax tree
/// into the format IR that the printer can render.
pub fn walk_node(node: &SyntaxNode) -> FormatIR {
    // Stub: will be fully implemented in Task 2.
    // For now, emit all token text separated by spaces to make the crate compile.
    let mut parts = Vec::new();
    collect_tokens(node, &mut parts);
    if parts.is_empty() {
        FormatIR::Empty
    } else {
        FormatIR::Concat(parts)
    }
}

fn collect_tokens(node: &SyntaxNode, parts: &mut Vec<FormatIR>) {
    use rowan::NodeOrToken;
    use snow_parser::SyntaxKind;

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(tok) => {
                let kind = tok.kind();
                if kind == SyntaxKind::EOF {
                    continue;
                }
                if kind == SyntaxKind::NEWLINE {
                    // Skip newlines; formatter controls line breaks.
                    continue;
                }
                let txt = tok.text().to_string();
                if !txt.is_empty() {
                    if !parts.is_empty() {
                        parts.push(FormatIR::Space);
                    }
                    parts.push(FormatIR::Text(txt));
                }
            }
            NodeOrToken::Node(n) => {
                collect_tokens(&n, parts);
            }
        }
    }
}
