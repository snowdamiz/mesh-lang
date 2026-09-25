//! LSP textDocument/signatureHelp implementation for the Mesh language.
//!
//! Provides parameter information and active parameter highlighting when the
//! cursor is inside function call parentheses. Detects the enclosing CALL_EXPR,
//! counts commas for the active parameter index, resolves the callee's type
//! from the TypeckResult, and extracts parameter names from the CST.

use tower_lsp::lsp_types::*;

use mesh_parser::SyntaxKind;
use mesh_parser::SyntaxNode;
use mesh_typeck::ty::Ty;

use crate::analysis::AnalysisResult;
use crate::syntax::{
    field_access_parts, first_ident_text, module_items, name_child_text, param_name,
};

/// Compute signature help at the given LSP position.
///
/// Returns `Some(SignatureHelp)` when the cursor is inside function call
/// parentheses, with the active parameter index set based on comma counting.
/// Returns `None` if the cursor is not inside a call expression or the
/// callee's type cannot be resolved.
pub fn compute_signature_help(
    source: &str,
    analysis: &AnalysisResult,
    position: &Position,
) -> Option<SignatureHelp> {
    // Step 1: Position conversion.
    let source_offset = crate::analysis::position_to_offset_pub(source, position)?;
    let tree_offset = crate::definition::source_to_tree_offset(source, source_offset)?;
    let target = rowan::TextSize::from(tree_offset as u32);

    let root = analysis.parse.syntax();

    // Step 2: Find enclosing CALL_EXPR via ARG_LIST walk.
    let token = root.token_at_offset(target).right_biased()?;
    let start_node = token.parent()?;
    let (call_expr, arg_list, active_parameter) = find_enclosing_call(&start_node, target)?;

    // Step 4: Extract callee name.
    let callee_name = extract_callee_name(&call_expr, &arg_list)?;

    // Step 5: Look up function type from TypeckResult.
    let fn_type = resolve_callee_type(&call_expr, &analysis.typeck)?;

    // Steps 6-7: Build SignatureInformation.
    let sig_info = build_signature_info(&root, &callee_name, &fn_type)?;

    Some(SignatureHelp {
        signatures: vec![sig_info],
        active_signature: Some(0),
        active_parameter: Some(active_parameter),
    })
}

/// Walk upward from a node to find the innermost enclosing CALL_EXPR.
///
/// Returns the CALL_EXPR node, the ARG_LIST node, and the active parameter
/// index (number of commas before the cursor in the ARG_LIST).
fn find_enclosing_call(
    start: &SyntaxNode,
    cursor_offset: rowan::TextSize,
) -> Option<(SyntaxNode, SyntaxNode, u32)> {
    let mut node = start.clone();

    loop {
        if node.kind() == SyntaxKind::ARG_LIST {
            // Found an arg list -- check that parent is a CALL_EXPR.
            if let Some(parent) = node.parent() {
                if parent.kind() == SyntaxKind::CALL_EXPR {
                    // Count commas before the cursor within this ARG_LIST.
                    let mut comma_count = 0u32;
                    for child_or_tok in node.children_with_tokens() {
                        if let rowan::NodeOrToken::Token(t) = child_or_tok {
                            if t.kind() == SyntaxKind::COMMA
                                && t.text_range().end() <= cursor_offset
                            {
                                comma_count += 1;
                            }
                        }
                    }

                    return Some((parent, node, comma_count));
                }
            }
        }

        node = node.parent()?;
    }
}

/// Extract the callee name from a CALL_EXPR node.
///
/// Handles simple calls (`add(x, y)`), qualified calls (`Module.func(x)`),
/// and method-style calls (`expr.method(args)`).
fn extract_callee_name(call_expr: &SyntaxNode, arg_list: &SyntaxNode) -> Option<String> {
    let arg_list_range = arg_list.text_range();

    // The callee is the child of CALL_EXPR that is NOT the ARG_LIST.
    for child in call_expr.children() {
        if child.text_range() == arg_list_range {
            continue;
        }

        match child.kind() {
            SyntaxKind::NAME_REF => {
                // Simple call: `add(x, y)` -- NAME_REF contains the IDENT.
                return first_ident_text(&child);
            }
            // Qualified call: `Module.func(x)`, or a method call:
            // `expr.method(args)`.
            SyntaxKind::FIELD_ACCESS => {
                return match field_access_parts(&child) {
                    (Some(base), Some(field)) => Some(format!("{base}.{field}")),
                    (None, field) => field,
                    (Some(_), None) => None,
                };
            }
            _ => {
                // Try to extract an IDENT token directly from this node.
                if let Some(name) = first_ident_text(&child) {
                    return Some(name);
                }
            }
        }
    }

    None
}

/// Resolve the callee's function type from the TypeckResult: the first
/// function type recorded for the callee or a node inside it, in tree order.
/// Only the callee: an argument can be a closure, whose type is a function
/// too.
fn resolve_callee_type(call_expr: &SyntaxNode, typeck: &mesh_typeck::TypeckResult) -> Option<Ty> {
    let callee = call_expr
        .children()
        .find(|child| child.kind() != SyntaxKind::ARG_LIST)?;
    callee
        .descendants()
        .find_map(|node| match typeck.types.get(&node.text_range()) {
            Some(ty @ Ty::Fun(..)) => Some(ty.clone()),
            _ => None,
        })
}

/// Find parameter names for a user-defined function from the CST: a
/// top-level `fn`, or for `Module.f` the `fn f` in that module's body.
fn find_fn_def_param_names(root: &SyntaxNode, callee_name: &str) -> Option<Vec<String>> {
    let fn_name = callee_name.rsplit('.').next().unwrap_or(callee_name);
    let is_named = |item: &SyntaxNode| {
        item.kind() == SyntaxKind::FN_DEF && name_child_text(item).as_deref() == Some(fn_name)
    };
    let function = match callee_name.rsplit_once('.') {
        Some((module, _)) => module_items(root, module).find(is_named),
        None => root.children().find(is_named),
    }?;
    Some(
        function
            .children()
            .filter(|child| child.kind() == SyntaxKind::PARAM_LIST)
            .flat_map(|list| list.children())
            .filter(|param| param.kind() == SyntaxKind::PARAM)
            .filter_map(|param| param_name(&param))
            .collect(),
    )
}

/// Build the SignatureInformation from the function type and optional param names.
fn build_signature_info(
    root: &SyntaxNode,
    callee_name: &str,
    fn_type: &Ty,
) -> Option<SignatureInformation> {
    match fn_type {
        Ty::Fun(params, ret) => {
            let param_names = find_fn_def_param_names(root, callee_name);

            let param_labels: Vec<String> = params
                .iter()
                .enumerate()
                .map(
                    |(i, ty)| match param_names.as_ref().and_then(|names| names.get(i)) {
                        Some(name) => format!("{name}: {ty}"),
                        None => format!("{ty}"),
                    },
                )
                .collect();
            let label = format!("{}({}) -> {}", callee_name, param_labels.join(", "), ret);
            let param_infos = param_labels
                .into_iter()
                .map(|label| ParameterInformation {
                    label: ParameterLabel::Simple(label),
                    documentation: None,
                })
                .collect();

            Some(SignatureInformation {
                label,
                documentation: None,
                parameters: Some(param_infos),
                active_parameter: None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    /// Helper: compute signature help for source at the given position.
    fn sig_help_at(source: &str, line: u32, character: u32) -> Option<SignatureHelp> {
        let analysis = crate::analysis::analyze_document("file:///test.mpl", source, &[]);
        let position = Position { line, character };
        compute_signature_help(source, &analysis, &position)
    }

    /// Signature help at the end of `line`'s `marker`.
    fn sig_help_after(source: &str, marker: &str) -> Option<SignatureHelp> {
        let at = source.find(marker).unwrap() + marker.len();
        let line = source[..at].matches('\n').count() as u32;
        let character = (at - source[..at].rfind('\n').map_or(0, |nl| nl + 1)) as u32;
        sig_help_at(source, line, character)
    }

    fn label(help: &SignatureHelp) -> &str {
        &help.signatures[0].label
    }

    #[test]
    fn signature_help_is_the_callees_not_a_closure_arguments() {
        let source = "fn apply(f, x :: Int) -> Int do\n  f(x)\nend\n\nfn main() do\n  apply(fn(y) -> y + 1 end, 2)\nend\n";
        let help = sig_help_after(source, "apply(fn(y) -> y + 1 end, ").unwrap();
        assert!(label(&help).starts_with("apply(f: "), "{}", label(&help));
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn signature_help_names_a_module_functions_parameters() {
        let source = "import Geo\n\nmodule Geo do\n  pub fn area(w :: Int, h :: Int) -> Int do\n    w * h\n  end\nend\n\nfn area(a :: Int) -> Int do\n  a\nend\n\nfn main() do\n  Geo.area(1, 2)\nend\n";
        let help = sig_help_after(source, "Geo.area(").unwrap();
        assert_eq!(label(&help), "Geo.area(w: Int, h: Int) -> Int");
    }

    #[test]
    fn signature_help_simple_call() {
        // fn add(a, b) do a + b end\nlet x = add(1, 2)
        let source = "fn add(a, b) do\na + b\nend\nlet x = add(1, 2)";
        // Cursor inside add(1, 2) -- after the opening paren.
        // "let x = add(" starts at line 3, "add(" -> character 8+4 = 12.
        // Position: line 3, character 12 -> inside the call at the `1`.
        let result = sig_help_at(source, 3, 12);
        assert!(
            result.is_some(),
            "Should return signature help inside add(1, 2)"
        );
        let help = result.unwrap();
        assert_eq!(help.signatures.len(), 1);
        let sig = &help.signatures[0];
        assert!(sig.parameters.is_some());
        let params = sig.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2, "add has 2 parameters");
    }

    #[test]
    fn signature_help_active_parameter_after_comma() {
        // fn add(a, b) do a + b end\nlet x = add(1, )
        let source = "fn add(a, b) do\na + b\nend\nlet x = add(1, )";
        // Cursor after the comma, at the space before ')'.
        // "let x = add(1, )" -- comma is at character 14, cursor at 15.
        let result = sig_help_at(source, 3, 15);
        assert!(result.is_some(), "Should return signature help after comma");
        let help = result.unwrap();
        assert_eq!(
            help.active_parameter,
            Some(1),
            "Active parameter should be 1 after first comma"
        );
    }

    #[test]
    fn signature_help_no_call() {
        // No function call -- cursor at a simple let binding.
        let source = "let x = 42";
        let result = sig_help_at(source, 0, 5);
        assert!(
            result.is_none(),
            "Should return None when not inside a function call"
        );
    }

    #[test]
    fn signature_help_first_parameter() {
        // fn greet(name) do name end\nlet x = greet()
        let source = "fn greet(name) do\nname\nend\nlet x = greet()";
        // Cursor right after '(' in greet() -- character 13.
        let result = sig_help_at(source, 3, 14);
        assert!(
            result.is_some(),
            "Should return signature help inside greet()"
        );
        let help = result.unwrap();
        assert_eq!(
            help.active_parameter,
            Some(0),
            "Active parameter should be 0 right after opening paren"
        );
        let sig = &help.signatures[0];
        let params = sig.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1, "greet has 1 parameter");
    }

    #[test]
    fn signature_help_has_parameter_names() {
        // Verify that parameter labels include the name from the FN_DEF.
        let source = "fn add(a, b) do\na + b\nend\nlet x = add(1, 2)";
        let result = sig_help_at(source, 3, 12);
        assert!(result.is_some());
        let help = result.unwrap();
        let sig = &help.signatures[0];
        let params = sig.parameters.as_ref().unwrap();
        // The first parameter label should contain "a".
        let first_label = match &params[0].label {
            ParameterLabel::Simple(s) => s.clone(),
            _ => String::new(),
        };
        assert!(
            first_label.contains("a"),
            "First parameter label should contain 'a', got: {}",
            first_label
        );
    }
}
