//! Reading names out of syntax nodes, for completion, definitions and
//! signature help.

use mesh_parser::{SyntaxKind, SyntaxNode};
use rowan::NodeOrToken;

/// The text of a node's first IDENT token.
pub(crate) fn first_ident_text(node: &SyntaxNode) -> Option<String> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::IDENT)
        .map(|token| token.text().to_string())
}

/// The name in a node's NAME child.
pub(crate) fn name_child_text(node: &SyntaxNode) -> Option<String> {
    node.children()
        .find(|child| child.kind() == SyntaxKind::NAME)
        .and_then(|name| first_ident_text(&name))
}

/// A parameter's name: its IDENT, or the name in its NAME child, whichever
/// comes first.
pub(crate) fn param_name(param: &SyntaxNode) -> Option<String> {
    param
        .children_with_tokens()
        .find_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::IDENT => {
                Some(Some(token.text().to_string()))
            }
            NodeOrToken::Node(node) if node.kind() == SyntaxKind::NAME => {
                Some(first_ident_text(&node))
            }
            _ => None,
        })
        .flatten()
}

/// The items in the body of the top-level `module <name> do ... end`.
pub(crate) fn module_items<'a>(
    root: &'a SyntaxNode,
    module_name: &'a str,
) -> impl Iterator<Item = SyntaxNode> + 'a {
    root.children()
        .filter(move |child| {
            child.kind() == SyntaxKind::MODULE_DEF
                && name_child_text(child).as_deref() == Some(module_name)
        })
        .flat_map(|module| module.children().filter(|c| c.kind() == SyntaxKind::BLOCK))
        .flat_map(|body| body.children())
}

/// A field access's base when it is a plain name, `Geo` in `Geo.area`, and
/// its field, `area`: an IDENT token of the FIELD_ACCESS itself.
pub(crate) fn field_access_parts(field_access: &SyntaxNode) -> (Option<String>, Option<String>) {
    let base = field_access
        .children()
        .next()
        .filter(|base| base.kind() == SyntaxKind::NAME_REF)
        .and_then(|base| first_ident_text(&base));
    let field = field_access
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::IDENT)
        .last()
        .map(|token| token.text().to_string());
    (base, field)
}
