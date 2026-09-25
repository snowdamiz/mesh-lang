//! Go-to-definition resolution via CST traversal.
//!
//! Resolves identifier references to their definition sites by walking the
//! concrete syntax tree. Supports:
//! - Variable references -> let binding NAME
//! - Function calls -> fn definition NAME
//! - Type names -> struct/type/sum type definition NAME
//! - Module-qualified names -> function within user-defined module
//!
//! ## Coordinate system
//!
//! The CST keeps whitespace as WHITESPACE tokens, so rowan `TextRange` offsets
//! are source byte offsets. The two conversion functions below only bounds-check.

use crate::syntax::{field_access_parts, first_ident_text, module_items};
use mesh_parser::SyntaxKind;
use mesh_parser::SyntaxNode;
use rowan::TextRange;

/// Convert a source byte offset to a rowan tree offset.
///
/// The CST is lossless (whitespace is kept as WHITESPACE tokens), so tree
/// offsets are source offsets. This only rejects offsets past the end.
pub fn source_to_tree_offset(source: &str, source_offset: usize) -> Option<usize> {
    (source_offset < source.len()).then_some(source_offset)
}

/// Convert a rowan tree offset to a source byte offset.
///
/// Identity for the same reason as [`source_to_tree_offset`].
pub fn tree_to_source_offset(source: &str, tree_offset: usize) -> Option<usize> {
    (tree_offset <= source.len()).then_some(tree_offset)
}

/// Find the definition site of the identifier at the given source byte offset.
///
/// Converts the source offset to a rowan tree offset, traverses the CST,
/// and returns the `TextRange` of the definition's NAME node (in rowan
/// coordinates). The caller must convert back to source coordinates using
/// `tree_to_source_offset` if needed for LSP position computation.
pub fn find_definition(source: &str, root: &SyntaxNode, source_offset: usize) -> Option<TextRange> {
    let tree_offset = source_to_tree_offset(source, source_offset)?;
    let target_offset = rowan::TextSize::from(tree_offset as u32);

    // Find the token at the given offset.
    let token = root.token_at_offset(target_offset).right_biased()?;

    // Only resolve IDENT tokens that are inside NAME_REF nodes or type annotation contexts.
    let parent = token.parent()?;
    let parent_kind = parent.kind();

    match parent_kind {
        SyntaxKind::NAME_REF => {
            // A standard library module has no definition in the source, so
            // none is found for it.
            find_variable_or_function_def(&parent, token.text())
        }
        // `Module.function`: the function in that module's body.
        SyntaxKind::FIELD_ACCESS if token.kind() == SyntaxKind::IDENT => {
            let (Some(module), Some(function)) = field_access_parts(&parent) else {
                return None;
            };
            find_in_module(root, &module, &function)
        }
        _ => {
            // The token might be an IDENT inside a TYPE_ANNOTATION or other context.
            // Walk up to see if we're in a type annotation context.
            if token.kind() == SyntaxKind::IDENT {
                let name_text = token.text().to_string();
                // Check if this is a type name reference in a type annotation.
                if is_in_type_context(&parent) {
                    return find_type_def(root, &name_text);
                }
            }
            None
        }
    }
}

/// Check whether a node is in a type annotation context.
fn is_in_type_context(node: &SyntaxNode) -> bool {
    node.ancestors()
        .any(|ancestor| ancestor.kind() == SyntaxKind::TYPE_ANNOTATION)
}

/// Find a variable or function definition for a NAME_REF node.
///
/// Walks upward from the reference through enclosing blocks, searching for:
/// - LET_BINDING with a matching NAME
/// - FN_DEF with a matching NAME
/// - PARAM with a matching NAME/IDENT
///
/// At the top level (SOURCE_FILE), searches all definitions.
fn find_variable_or_function_def(name_ref_node: &SyntaxNode, name: &str) -> Option<TextRange> {
    // Walk up the tree from the reference.
    let mut current = name_ref_node.parent()?;

    loop {
        match current.kind() {
            SyntaxKind::BLOCK | SyntaxKind::SOURCE_FILE => {
                // Search earlier siblings in this block/source for definitions.
                if let Some(range) = search_block_for_def(&current, name_ref_node, name) {
                    return Some(range);
                }
                // If this is SOURCE_FILE, we're done searching.
                if current.kind() == SyntaxKind::SOURCE_FILE {
                    return None;
                }
            }
            // A function's or a closure's parameters.
            SyntaxKind::FN_DEF | SyntaxKind::CLOSURE_EXPR => {
                if let Some(range) = search_params_for_name(&current, name) {
                    return Some(range);
                }
            }
            _ => {}
        }

        current = current.parent()?;
    }
}

/// Search within a block or source file for a definition of `name` that
/// appears before `name_ref_node`.
fn search_block_for_def(
    block: &SyntaxNode,
    name_ref_node: &SyntaxNode,
    name: &str,
) -> Option<TextRange> {
    let ref_offset = name_ref_node.text_range().start();

    // For SOURCE_FILE, search all children (not just earlier ones) to handle
    // forward references to top-level functions.
    let search_all = block.kind() == SyntaxKind::SOURCE_FILE;

    for child in block.children() {
        // Only consider definitions before the reference (unless top-level).
        if !search_all && child.text_range().start() >= ref_offset {
            break;
        }

        let defines = matches!(
            child.kind(),
            SyntaxKind::LET_BINDING
                | SyntaxKind::FN_DEF
                | SyntaxKind::ACTOR_DEF
                | SyntaxKind::SERVICE_DEF
                | SyntaxKind::MODULE_DEF
        );
        if let Some(range) = defines
            .then(|| name_child_if_matches(&child, name))
            .flatten()
        {
            return Some(range);
        }
    }

    None
}

/// Search parameter list of a FN_DEF or CLOSURE_EXPR for a matching name.
fn search_params_for_name(fn_node: &SyntaxNode, name: &str) -> Option<TextRange> {
    fn_node
        .children()
        .filter(|child| child.kind() == SyntaxKind::PARAM_LIST)
        .flat_map(|list| list.children())
        .filter(|param| param.kind() == SyntaxKind::PARAM)
        .find_map(|param| {
            // A PARAM holds its name as an IDENT token or a NAME node.
            param
                .children_with_tokens()
                .find_map(|element| match element {
                    rowan::NodeOrToken::Token(t)
                        if t.kind() == SyntaxKind::IDENT && t.text() == name =>
                    {
                        Some(t.text_range())
                    }
                    rowan::NodeOrToken::Node(n) if n.kind() == SyntaxKind::NAME => {
                        (first_ident_text(&n).as_deref() == Some(name)).then(|| n.text_range())
                    }
                    _ => None,
                })
        })
}

/// Find a type definition (struct, sum type, type alias) with a matching name.
fn find_type_def(root: &SyntaxNode, name: &str) -> Option<TextRange> {
    root.children()
        .filter(|child| {
            matches!(
                child.kind(),
                SyntaxKind::STRUCT_DEF | SyntaxKind::SUM_TYPE_DEF | SyntaxKind::TYPE_ALIAS_DEF
            )
        })
        .find_map(|definition| name_child_if_matches(&definition, name))
}

/// Find a function definition inside a MODULE_DEF with the given module name.
fn find_in_module(root: &SyntaxNode, module_name: &str, fn_name: &str) -> Option<TextRange> {
    module_items(root, module_name)
        .filter(|item| item.kind() == SyntaxKind::FN_DEF)
        .find_map(|function| name_child_if_matches(&function, fn_name))
}

/// If a node has a NAME child whose text matches `name`, return the NAME's range.
fn name_child_if_matches(node: &SyntaxNode, name: &str) -> Option<TextRange> {
    node.children()
        .find(|child| {
            child.kind() == SyntaxKind::NAME && first_ident_text(child).as_deref() == Some(name)
        })
        .map(|name| name.text_range())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parse source, convert source offset to tree offset, and find definition.
    fn def_at(source: &str, source_offset: usize) -> Option<TextRange> {
        let parse = mesh_parser::parse(source);
        let root = parse.syntax();
        find_definition(source, &root, source_offset)
    }

    /// Helper: get the source byte offset of a tree TextRange start.
    fn tree_range_to_source(source: &str, range: TextRange) -> Option<usize> {
        tree_to_source_offset(source, range.start().into())
    }

    #[test]
    fn tree_offsets_are_source_offsets() {
        let source = "let x = 42";
        let parse = mesh_parser::parse(source);
        assert_eq!(parse.syntax().text().to_string(), source);
        assert_eq!(source_to_tree_offset(source, 4), Some(4)); // 'x'
        assert_eq!(source_to_tree_offset(source, source.len()), None);
        assert_eq!(tree_to_source_offset(source, 8), Some(8)); // '4'
        assert_eq!(
            tree_to_source_offset(source, source.len()),
            Some(source.len())
        );
        assert_eq!(tree_to_source_offset(source, source.len() + 1), None);
    }

    #[test]
    fn find_def_variable_let_binding() {
        let source = "let x = 42\nlet y = x";
        // "x" at the end (in `y = x`) should resolve to the NAME in `let x = 42`
        // Find the source offset of the second "x"
        let first_x = source.find('x').unwrap();
        let use_offset = source[first_x + 1..].find('x').unwrap() + first_x + 1;
        let result = def_at(source, use_offset);
        assert!(result.is_some(), "Should find definition of x");
        let range = result.unwrap();
        // Convert the result back to source offset to verify.
        let def_source_offset = tree_range_to_source(source, range).unwrap();
        assert_eq!(
            def_source_offset, 4,
            "Definition of x should be at source offset 4"
        );
    }

    /// Where the definition of the `occurrence`-th `name` in `source`
    /// (counting from 0) starts, as a source offset.
    fn def_of(source: &str, name: &str, occurrence: usize) -> Option<usize> {
        let at = source.match_indices(name).nth(occurrence)?.0;
        tree_range_to_source(source, def_at(source, at)?)
    }

    #[test]
    fn find_def_module_function_and_other_definitions() {
        // `Geo.area(...)` resolves into the module's body.
        let source =
            "module Geo do\n  pub fn area(r) = r * r\nend\n\nfn main() do\n  Geo.area(2)\nend\n";
        assert_eq!(def_of(source, "area", 1), source.find("area"));
        assert_eq!(def_of(source, "Geo", 1), source.find("Geo"));
        // A closure's parameter, an actor and a service.
        let source = "actor Pinger() do\n  1\nend\n\nservice Store do\n  fn init() -> Int do\n    0\n  end\nend\n\nfn main() do\n  let f = fn(x) -> x + 1 end\n  spawn(Pinger)\n  Store.start()\nend\n";
        assert_eq!(def_of(source, "x", 1), source.find("(x)").map(|at| at + 1));
        assert_eq!(def_of(source, "Pinger", 1), source.find("Pinger"));
        // An unknown module's function has no definition here.
        assert_eq!(def_of("fn main() do\n  Nowhere.go()\nend\n", "go", 0), None);
    }

    #[test]
    fn find_def_function_call() {
        let source = "fn add(a, b) do\na + b\nend\nlet result = add(1, 2)";
        // "add" in the call `add(1, 2)` should resolve to the NAME in `fn add`.
        let call_offset = source.rfind("add").unwrap();
        let result = def_at(source, call_offset);
        assert!(result.is_some(), "Should find definition of add");
        let range = result.unwrap();
        let def_source_offset = tree_range_to_source(source, range).unwrap();
        // "fn add" -- NAME for "add" starts at source offset 3.
        assert_eq!(
            def_source_offset, 3,
            "Definition of add should be at source offset 3"
        );
    }

    #[test]
    fn find_def_type_in_annotation() {
        let source =
            "struct Point do\nx :: Int\ny :: Int\nend\nlet p :: Point = Point { x: 1, y: 2 }";
        // Find "Point" in the type annotation `:: Point`.
        let after_let = source.find("let p").unwrap();
        let in_annotation = source[after_let..].find("Point").unwrap() + after_let;
        let result = def_at(source, in_annotation);
        // Type annotation context detection -- verify no panic.
        let _ = result;
    }

    #[test]
    fn find_def_returns_none_for_builtins() {
        let source = "let x = 42";
        // Offset 0 is 'l' of 'let', which is a keyword, not a NAME_REF.
        let result = def_at(source, 0);
        assert!(
            result.is_none(),
            "Keywords should not resolve to definitions"
        );
    }

    #[test]
    fn find_def_nested_scope_shadowing() {
        // Inner let shadows outer let.
        let source = "fn main() do\nlet x = 1\nfn inner() do\nlet x = 2\nlet y = x\nend\nend";
        // Find the "x" usage in `let y = x`.
        let y_binding = source.find("let y = x").unwrap();
        let x_use = y_binding + "let y = ".len();
        let result = def_at(source, x_use);
        assert!(result.is_some(), "Should find inner x definition");
        let range = result.unwrap();
        let def_source_offset = tree_range_to_source(source, range).unwrap();
        // The inner `let x = 2` NAME "x" should be at the source offset of that x.
        let inner_x_def = source.find("let x = 2").unwrap() + "let ".len();
        assert_eq!(def_source_offset, inner_x_def);
    }

    #[test]
    fn find_def_returns_none_for_unknown() {
        let source = "let x = unknown_var";
        // "unknown_var" starts at source offset 8.
        let result = def_at(source, 8);
        assert!(result.is_none(), "Unknown variables should return None");
    }

    #[test]
    fn find_def_function_param() {
        let source = "fn double(n) do\nlet result = n + n\nresult\nend";
        // "n" in `n + n` should resolve to the parameter.
        let n_use = source.find("n + n").unwrap();
        let result = def_at(source, n_use);
        assert!(result.is_some(), "Should find parameter definition of n");
    }
}
