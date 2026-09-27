//! LSP textDocument/signatureHelp implementation for the Mesh language.
//!
//! Provides parameter information and active parameter highlighting when the
//! cursor is inside function call parentheses. Detects the enclosing CALL_EXPR,
//! counts commas for the active parameter index, resolves the callee's type
//! from the TypeckResult, and extracts parameter names from the CST.

use tower_lsp::lsp_types::*;

use mesh_parser::SyntaxKind;
use mesh_parser::{SyntaxNode, SyntaxToken};
use mesh_typeck::ty::Ty;

use crate::analysis::AnalysisResult;
use crate::syntax::{
    field_access_parts, first_ident_text, module_items, name_child_text, param_names,
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
    // Tree offsets are source offsets.
    let target =
        rowan::TextSize::from(crate::analysis::position_to_offset(source, position)? as u32);
    let root = analysis.parse.syntax();
    let tokens = root.token_at_offset(target);
    // The innermost call whose argument list holds the token after the
    // cursor, or an argument list not closed yet that the cursor follows:
    // the whitespace after it is outside it.
    let (call_expr, arg_list) = tokens
        .clone()
        .right_biased()
        .and_then(|token| enclosing_call(&token))
        .or_else(|| {
            let mut token = tokens.left_biased()?;
            while token.kind().is_trivia() {
                token = token.prev_token()?;
            }
            enclosing_call(&token).filter(|(_, arg_list)| {
                !arg_list
                    .children_with_tokens()
                    .any(|element| element.kind() == SyntaxKind::R_PAREN)
            })
        })?;
    // The argument the cursor is in: the commas before it.
    let active_parameter = arg_list
        .children_with_tokens()
        .filter(|element| {
            element.kind() == SyntaxKind::COMMA && element.text_range().end() <= target
        })
        .count() as u32;
    let callee = call_expr
        .children()
        .find(|child| child.kind() != SyntaxKind::ARG_LIST)?;
    let callee_name = extract_callee_name(&callee)?;
    let (params, ret) = resolve_callee_type(&callee, &analysis.typeck)?;

    Some(SignatureHelp {
        signatures: vec![build_signature_info(&root, &callee_name, params, ret)],
        active_signature: Some(0),
        active_parameter: Some(active_parameter),
    })
}

/// The innermost call whose argument list a token is in, and the list.
fn enclosing_call(token: &SyntaxToken) -> Option<(SyntaxNode, SyntaxNode)> {
    token.parent_ancestors().find_map(|node| {
        let parent = node.parent()?;
        (node.kind() == SyntaxKind::ARG_LIST && parent.kind() == SyntaxKind::CALL_EXPR)
            .then_some((parent, node))
    })
}

/// The callee's name: `add`, `Module.func`, or for a method call on an
/// expression, `method`.
fn extract_callee_name(callee: &SyntaxNode) -> Option<String> {
    match callee.kind() {
        SyntaxKind::FIELD_ACCESS => match field_access_parts(callee) {
            (Some(base), Some(field)) => Some(format!("{base}.{field}")),
            (_, field) => field,
        },
        _ => first_ident_text(callee),
    }
}

/// Resolve the callee's function type from the TypeckResult, as its
/// parameter and return types: the first function type recorded for the
/// callee or a node inside it, in tree order. Only the callee: an argument
/// can be a closure, whose type is a function too.
fn resolve_callee_type<'a>(
    callee: &SyntaxNode,
    typeck: &'a mesh_typeck::TypeckResult,
) -> Option<(&'a [Ty], &'a Ty)> {
    callee
        .descendants()
        .find_map(|node| match typeck.types.get(&node.text_range()) {
            Some(Ty::Fun(params, ret)) => Some((params.as_slice(), &**ret)),
            _ => None,
        })
}

/// Find parameter names for a user-defined function from the CST: a
/// top-level `fn`, or for `Module.f` the `fn f` in that module's body. A
/// pattern parameter has none.
fn find_fn_def_param_names(root: &SyntaxNode, callee_name: &str) -> Option<Vec<Option<String>>> {
    let fn_name = callee_name.rsplit('.').next().unwrap_or(callee_name);
    let is_named = |item: &SyntaxNode| {
        item.kind() == SyntaxKind::FN_DEF && name_child_text(item).as_deref() == Some(fn_name)
    };
    let function = match callee_name.rsplit_once('.') {
        Some((module, _)) => module_items(root, module).find(is_named),
        None => root.children().find(is_named),
    }?;
    Some(
        param_names(&function)
            .map(|name| name.map(|name| name.text().to_string()))
            .collect(),
    )
}

/// Build the SignatureInformation from the function's types and, for a
/// function defined in the source, its parameter names.
fn build_signature_info(
    root: &SyntaxNode,
    callee_name: &str,
    params: &[Ty],
    ret: &Ty,
) -> SignatureInformation {
    let param_names = find_fn_def_param_names(root, callee_name);
    // The variables named across the whole signature, so one keeps its name.
    let (params, ret) = match Ty::Fun(params.to_vec(), Box::new(ret.clone())).with_named_vars() {
        Ty::Fun(params, ret) => (params, *ret),
        _ => unreachable!("naming variables keeps a function type"),
    };
    let param_labels: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            match param_names
                .as_ref()
                .and_then(|names| names.get(i)?.as_ref())
            {
                Some(name) => format!("{name}: {ty}"),
                None => format!("{ty}"),
            }
        })
        .collect();
    let label = format!("{}({}) -> {}", callee_name, param_labels.join(", "), ret);
    let param_infos = param_labels
        .into_iter()
        .map(|label| ParameterInformation {
            label: ParameterLabel::Simple(label),
            documentation: None,
        })
        .collect();

    SignatureInformation {
        label,
        documentation: None,
        parameters: Some(param_infos),
        active_parameter: None,
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

    /// Typing a call with no closing parenthesis yet: at the end of the
    /// document, or before the next line.
    #[test]
    fn an_unclosed_call_has_signature_help() {
        let definition = "fn add(a :: Int, b :: Int) -> Int do\n  a + b\nend\n\nfn main() do\n";
        for rest in ["  add(1, ", "  add(1, \n  add(2, 3)\nend\n"] {
            let source = format!("{definition}{rest}");
            let help = sig_help_after(&source, "  add(1, ")
                .unwrap_or_else(|| panic!("no help in {source:?}"));
            assert_eq!(label(&help), "add(a: Int, b: Int) -> Int");
            assert_eq!(help.active_parameter, Some(1));
        }
        // After a closed call there is none.
        let source = format!("{definition}  add(1, 2) \nend\n");
        assert!(sig_help_after(&source, "add(1, 2) ").is_none());
    }

    /// A pattern parameter has no name, and the names after it stay with
    /// their parameters.
    #[test]
    fn a_pattern_parameter_leaves_the_other_names_in_place() {
        let source =
            "fn pick(0, y :: Int) -> Int do\n  y\nend\n\nfn main() do\n  pick(0, 1)\nend\n";
        let help = sig_help_after(source, "  pick(").unwrap();
        assert_eq!(label(&help), "pick(Int, y: Int) -> Int");
    }

    /// A function with no definition in the source is labelled by its types.
    #[test]
    fn a_standard_library_function_is_labelled_by_its_types() {
        let source = "fn main() do\n  String.length(\"abc\")\nend\n";
        let help = sig_help_after(source, "String.length(").unwrap();
        assert_eq!(label(&help), "String.length(String) -> Int");
    }

    /// A callee with no name, or no type, has no signature to show.
    #[test]
    fn a_callee_without_a_name_or_type_has_no_help() {
        let source = "fn main() do\n  (fn(x :: Int) -> x end)(1)\n  let r :: Result<Int, String> = (-5).try_into()\nend\n";
        assert!(sig_help_after(source, "end)(").is_none());
        assert!(sig_help_after(source, "try_into(").is_none());
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
