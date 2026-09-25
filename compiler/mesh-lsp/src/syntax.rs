//! Reading names out of syntax nodes, for completion, definitions and
//! signature help.

use mesh_parser::ast::item::{LetBinding, Param};
use mesh_parser::ast::AstNode;
use mesh_parser::{SyntaxKind, SyntaxNode, SyntaxToken};

/// A node's first IDENT token.
fn first_ident(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::IDENT)
}

/// The text of a node's first IDENT token.
pub(crate) fn first_ident_text(node: &SyntaxNode) -> Option<String> {
    first_ident(node).map(|token| token.text().to_string())
}

/// The names a definition binds: its NAME's, or for a `let` with a
/// pattern each name the pattern binds (`a` and `b` in `let (a, b) = pair`).
pub(crate) fn defined_names(definition: &SyntaxNode) -> Vec<SyntaxToken> {
    if let Some(name) = definition
        .children()
        .find(|child| child.kind() == SyntaxKind::NAME)
    {
        return first_ident(&name).into_iter().collect();
    }
    LetBinding::cast(definition.clone())
        .and_then(|binding| binding.pattern())
        .into_iter()
        .flat_map(|pattern| pattern.syntax().descendants())
        .filter(|node| node.kind() == SyntaxKind::IDENT_PAT)
        .filter_map(|node| first_ident(&node))
        .collect()
}

/// The name in a node's NAME child.
pub(crate) fn name_child_text(node: &SyntaxNode) -> Option<String> {
    node.children()
        .find(|child| child.kind() == SyntaxKind::NAME)
        .and_then(|name| first_ident_text(&name))
}

/// A function's or closure's parameters, each with its name: none for a
/// pattern or `self`.
pub(crate) fn param_names(fn_node: &SyntaxNode) -> impl Iterator<Item = Option<SyntaxToken>> {
    fn_node
        .children()
        .filter(|child| child.kind() == SyntaxKind::PARAM_LIST)
        .flat_map(|list| list.children())
        .filter_map(Param::cast)
        .map(|param| param.name())
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
