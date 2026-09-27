//! LSP textDocument/completion implementation for the Mesh language.
//!
//! Provides four tiers of code completions:
//! 1. **Keywords** -- Mesh keywords and contextual syntax, filtered by typed prefix
//! 2. **Built-in types** -- common types (Int, Float, String, etc.)
//! 3. **Snippets** -- template expansions for common patterns (fn, let, struct, etc.)
//! 4. **Scope-aware names** -- variables, functions, and types visible at the cursor

use tower_lsp::lsp_types::*;

use mesh_parser::SyntaxKind;
use mesh_parser::SyntaxNode;

use crate::analysis::AnalysisResult;
use crate::syntax::{defined_names, param_names};

/// Mesh keywords and contextual syntax.
const KEYWORDS: &[&str] = &[
    "actor",
    "after",
    "and",
    "break",
    "borrow",
    "call",
    "case",
    "cast",
    "continue",
    "consume",
    "def",
    "do",
    "else",
    "end",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "import",
    "in",
    "interface",
    "json",
    "let",
    "link",
    "match",
    "module",
    "nil",
    "not",
    "or",
    "pub",
    "receive",
    "return",
    "resource",
    "self",
    "send",
    "service",
    "spawn",
    "struct",
    "supervisor",
    "terminate",
    "true",
    "type",
    "when",
    "where",
    "while",
];

/// Built-in type names commonly used in Mesh.
const BUILTIN_TYPES: &[&str] = &[
    "Int",
    "Float",
    "String",
    "Bool",
    "List",
    "Map",
    "Set",
    "Option",
    "Result",
    "Queue",
    "Range",
    "Pid",
    "SecretBytes",
    "CryptoError",
    "X25519PrivateKey",
    "X25519PublicKey",
    "X25519KeyPair",
    "SigningPrivateKey",
    "SigningPublicKey",
    "SigningKeyPair",
    "Signature",
    "AeadKey",
];

/// Snippet definitions: (label, snippet_body).
const SNIPPETS: &[(&str, &str)] = &[
    ("fn", "fn ${1:name}(${2:params}) do\n  ${0}\nend"),
    ("let", "let ${1:name} = ${0}"),
    ("struct", "struct ${1:Name} do\n  ${0}\nend"),
    ("case", "case ${1:expr} do\n  ${2:pattern} -> ${0}\nend"),
    ("for", "for ${1:item} in ${2:collection} do\n  ${0}\nend"),
    ("while", "while ${1:condition} do\n  ${0}\nend"),
    ("actor", "actor ${1:Name}(${2:state}) do\n  ${0}\nend"),
    ("interface", "interface ${1:Name} do\n  ${0}\nend"),
    ("impl", "impl ${1:Trait} for ${2:Type} do\n  ${0}\nend"),
    ("type", "type ${1:Alias} = ${0:ExistingType}"),
    ("json", "json {\n  ${1:key}: ${0:value}\n}"),
];

/// Compute completion items at the given position.
///
/// Combines all four completion tiers: keywords, built-in types, snippets,
/// and scope-aware names from CST traversal. Results are filtered by the
/// prefix the user has typed so far.
pub fn compute_completions(
    source: &str,
    analysis: &AnalysisResult,
    position: &Position,
) -> Vec<CompletionItem> {
    // Convert LSP position to source byte offset.
    let source_offset = match crate::analysis::position_to_offset(source, position) {
        Some(o) => o,
        None => return Vec::new(),
    };

    // Extract the prefix by scanning backward from cursor to the last
    // non-identifier character. This avoids tree offset issues when cursor
    // is in whitespace.
    let prefix = extract_prefix(source, source_offset);

    let mut items = Vec::new();

    // Tier 1: Keyword completions.
    for &kw in KEYWORDS {
        if prefix.is_empty() || kw.starts_with(&prefix) {
            items.push(CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                sort_text: Some(format!("2_{}", kw)),
                ..Default::default()
            });
        }
    }

    // Tier 2: Built-in type completions.
    for &ty in BUILTIN_TYPES {
        if prefix.is_empty() || ty.starts_with(&prefix) {
            items.push(CompletionItem {
                label: ty.to_string(),
                kind: Some(CompletionItemKind::STRUCT),
                sort_text: Some(format!("1_{}", ty)),
                ..Default::default()
            });
        }
    }

    // Tier 3: Snippet completions.
    for &(label, body) in SNIPPETS {
        if prefix.is_empty() || label.starts_with(&prefix) {
            items.push(CompletionItem {
                label: label.to_string(),
                kind: Some(CompletionItemKind::SNIPPET),
                insert_text: Some(body.to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                sort_text: Some(format!("3_{}", label)),
                ..Default::default()
            });
        }
    }

    // Tier 4: Scope-aware name completions from CST walk.
    let root = analysis.parse.syntax();
    let scope_names = collect_in_scope_names(&root, source_offset);
    for (name, kind) in scope_names {
        if prefix.is_empty() || name.starts_with(&prefix) {
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(kind),
                sort_text: Some(format!("0_{}", name)),
                ..Default::default()
            });
        }
    }

    items
}

/// Extract the identifier prefix being typed by scanning backward from
/// the cursor position.
///
/// Stops at the first character that is not alphanumeric or underscore.
/// Returns an empty string if the cursor is at the start of a line or
/// right after whitespace/punctuation.
fn extract_prefix(source: &str, offset: usize) -> String {
    let before = &source[..offset];
    let start = before
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    before[start..].to_string()
}

/// Collect all in-scope names visible at the given source byte offset.
///
/// Walks upward through the CST from the cursor position, collecting
/// names from let bindings, function definitions, parameters, and
/// top-level definitions. Inner-scope names shadow outer-scope names.
fn collect_in_scope_names(root: &SyntaxNode, offset: usize) -> Vec<(String, CompletionItemKind)> {
    let target = rowan::TextSize::from(offset as u32);
    let mut seen_names = std::collections::HashSet::new();
    let mut names = Vec::new();
    // Outward from the token at the cursor.
    let scopes = root
        .token_at_offset(target)
        .right_biased()
        .into_iter()
        .flat_map(|token| token.parent_ancestors());
    for current in scopes {
        match current.kind() {
            SyntaxKind::BLOCK | SyntaxKind::SOURCE_FILE => {
                let search_all = current.kind() == SyntaxKind::SOURCE_FILE;
                collect_block_names(&current, target, search_all, &mut seen_names, &mut names);
            }
            SyntaxKind::FN_DEF | SyntaxKind::CLOSURE_EXPR => {
                collect_param_names(&current, &mut seen_names, &mut names);
            }
            _ => {}
        }
    }

    names
}

/// Collect names from definitions in a block or source file.
///
/// For blocks, only includes definitions before the cursor position.
/// For SOURCE_FILE, includes all definitions (forward references allowed).
fn collect_block_names(
    block: &SyntaxNode,
    cursor_offset: rowan::TextSize,
    search_all: bool,
    seen: &mut std::collections::HashSet<String>,
    names: &mut Vec<(String, CompletionItemKind)>,
) {
    for child in block.children() {
        // Only consider definitions before the cursor (unless top-level).
        if !search_all && child.text_range().start() >= cursor_offset {
            break;
        }

        let kind = match child.kind() {
            SyntaxKind::LET_BINDING => CompletionItemKind::VARIABLE,
            SyntaxKind::FN_DEF | SyntaxKind::ACTOR_DEF | SyntaxKind::SERVICE_DEF => {
                CompletionItemKind::FUNCTION
            }
            SyntaxKind::MODULE_DEF => CompletionItemKind::MODULE,
            SyntaxKind::STRUCT_DEF => CompletionItemKind::STRUCT,
            SyntaxKind::SUM_TYPE_DEF => CompletionItemKind::ENUM,
            SyntaxKind::INTERFACE_DEF => CompletionItemKind::INTERFACE,
            _ => continue,
        };
        for name in defined_names(&child) {
            // Deduplicate: inner-scope names shadow outer-scope names.
            if seen.insert(name.text().to_string()) {
                names.push((name.text().to_string(), kind));
            }
        }
    }
}

/// Collect parameter names from a FN_DEF or CLOSURE_EXPR node.
fn collect_param_names(
    fn_node: &SyntaxNode,
    seen: &mut std::collections::HashSet<String>,
    names: &mut Vec<(String, CompletionItemKind)>,
) {
    for name in param_names(fn_node).flatten() {
        if seen.insert(name.text().to_string()) {
            names.push((name.text().to_string(), CompletionItemKind::VARIABLE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: compute completions for source at the given position.
    fn completions_at(source: &str, line: u32, character: u32) -> Vec<CompletionItem> {
        let analysis = crate::analysis::analyze_document("file:///test.mpl", source, &[]);
        let position = Position { line, character };
        compute_completions(source, &analysis, &position)
    }

    /// The scope names at the cursor, with their kinds.
    fn scope_names_at(
        source: &str,
        line: u32,
        character: u32,
    ) -> Vec<(String, CompletionItemKind)> {
        completions_at(source, line, character)
            .into_iter()
            .filter(|item| {
                item.sort_text
                    .as_deref()
                    .is_some_and(|sort| sort.starts_with("0_"))
            })
            .map(|item| (item.label, item.kind.unwrap()))
            .collect()
    }

    #[test]
    fn every_kind_of_definition_is_offered_as_what_it_is() {
        let source = "module Geometry do\nend\n\nstruct Point do\n  x :: Int\nend\n\ntype Shape do\n  Circle\nend\n\ninterface Area do\n  fn area(self) -> Int\nend\n\nfn main() do\n  1\nend\n";
        let names = scope_names_at(source, 16, 2);
        for expected in [
            ("Geometry", CompletionItemKind::MODULE),
            ("Point", CompletionItemKind::STRUCT),
            ("Shape", CompletionItemKind::ENUM),
            ("Area", CompletionItemKind::INTERFACE),
            ("main", CompletionItemKind::FUNCTION),
        ] {
            assert!(
                names.contains(&(expected.0.to_string(), expected.1)),
                "{expected:?} in {names:?}"
            );
        }
    }

    #[test]
    fn a_binding_shadowing_a_parameter_is_offered_once() {
        let source = "fn f(x :: Int) do\n  let x = 2\n  x\nend\n";
        let names = scope_names_at(source, 2, 3);
        assert_eq!(
            names.iter().filter(|(name, _)| name == "x").count(),
            1,
            "{names:?}"
        );
    }

    #[test]
    fn a_definition_still_being_typed_offers_no_name() {
        // `fn` with no name yet, before a function that has one.
        let source = "fn\n\nfn main() do\n  1\nend\n";
        let names = scope_names_at(source, 3, 2);
        assert!(names.iter().all(|(name, _)| name == "main"), "{names:?}");
    }

    #[test]
    fn a_position_past_the_document_offers_nothing() {
        assert!(completions_at("fn main() do\n  1\nend\n", 40, 0).is_empty());
    }

    /// The names a `let` pattern binds are offered, and not the fields it
    /// matches them against.
    #[test]
    fn names_a_let_pattern_binds_are_offered() {
        let source = "struct Point do\n  x :: Int\n  y :: Int\nend\n\nfn main() do\n  let (a, (b, _)) = (1, (2, 3))\n  let Point { x, y: py } = Point { x: 1, y: 2 }\n  a\nend\n";
        let names: Vec<String> = scope_names_at(source, 8, 2)
            .into_iter()
            .filter(|(_, kind)| *kind == CompletionItemKind::VARIABLE)
            .map(|(name, _)| name)
            .collect();
        for bound in ["a", "b", "x", "py"] {
            assert!(
                names.iter().any(|name| name == bound),
                "{bound} in {names:?}"
            );
        }
        assert!(
            !names.iter().any(|name| name == "y" || name == "_"),
            "{names:?}"
        );
    }

    #[test]
    fn keyword_completion_prefix_filter() {
        // Typing "wh" should match "when", "where", "while" but not "fn" or "let".
        let source = "wh";
        let items = completions_at(source, 0, 2);

        let keyword_labels: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::KEYWORD))
            .map(|i| i.label.as_str())
            .collect();

        assert!(
            keyword_labels.contains(&"when"),
            "should contain 'when', got: {:?}",
            keyword_labels
        );
        assert!(
            keyword_labels.contains(&"where"),
            "should contain 'where', got: {:?}",
            keyword_labels
        );
        assert!(
            keyword_labels.contains(&"while"),
            "should contain 'while', got: {:?}",
            keyword_labels
        );
        assert!(!keyword_labels.contains(&"fn"), "should not contain 'fn'");
        assert!(!keyword_labels.contains(&"let"), "should not contain 'let'");
    }

    #[test]
    fn builtin_type_completion() {
        // Typing "St" should match "String" but not "Int".
        let source = "St";
        let items = completions_at(source, 0, 2);

        let type_labels: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::STRUCT))
            .map(|i| i.label.as_str())
            .collect();

        assert!(
            type_labels.contains(&"String"),
            "should contain 'String', got: {:?}",
            type_labels
        );
        assert!(
            !type_labels.contains(&"Int"),
            "should not contain 'Int', got: {:?}",
            type_labels
        );
    }

    #[test]
    fn ownership_syntax_and_resource_types_are_completed() {
        let items = completions_at("", 0, 0);
        let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();

        for expected in [
            "resource",
            "borrow",
            "consume",
            "SecretBytes",
            "X25519PrivateKey",
            "SigningPrivateKey",
            "AeadKey",
        ] {
            assert!(
                labels.contains(&expected),
                "missing completion `{expected}`"
            );
        }
    }

    #[test]
    fn scope_completion_finds_let_bindings() {
        // Parse "let x = 1\nlet y = 2\n" and verify both appear in scope completions at the end.
        let source = "let x = 1\nlet y = 2\n";
        // Position at end of the second line (line 2, char 0 -- empty line).
        let items = completions_at(source, 2, 0);

        let scope_labels: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::VARIABLE))
            .map(|i| i.label.as_str())
            .collect();

        assert!(
            scope_labels.contains(&"x"),
            "should contain 'x', got: {:?}",
            scope_labels
        );
        assert!(
            scope_labels.contains(&"y"),
            "should contain 'y', got: {:?}",
            scope_labels
        );
    }

    #[test]
    fn scope_completion_finds_fn_params() {
        // Parse "fn add(a, b) do\n\nend" and verify "a" and "b" appear at line 1.
        let source = "fn add(a, b) do\n\nend";
        // Position inside the function body (line 1, char 0).
        let items = completions_at(source, 1, 0);

        let scope_labels: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::VARIABLE))
            .map(|i| i.label.as_str())
            .collect();

        assert!(
            scope_labels.contains(&"a"),
            "should contain 'a', got: {:?}",
            scope_labels
        );
        assert!(
            scope_labels.contains(&"b"),
            "should contain 'b', got: {:?}",
            scope_labels
        );
    }

    #[test]
    fn snippet_completions_filtered_by_prefix() {
        // Typing "fo" should match the "for" snippet but not "while" or "fn".
        let source = "fo";
        let items = completions_at(source, 0, 2);

        let snippet_labels: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::SNIPPET))
            .map(|i| i.label.as_str())
            .collect();

        assert!(
            snippet_labels.contains(&"for"),
            "should contain 'for' snippet, got: {:?}",
            snippet_labels
        );
        assert!(
            !snippet_labels.contains(&"while"),
            "should not contain 'while' snippet"
        );
    }

    #[test]
    fn empty_prefix_returns_all_completions() {
        // At the start of an empty document, all keywords, types, and snippets should appear.
        let source = "";
        let items = completions_at(source, 0, 0);

        // Keep a coarse floor so accidental table loss is visible without
        // coupling this test to every future completion.
        assert!(
            items.len() >= 72,
            "expected at least 72 completions for empty prefix, got {}",
            items.len()
        );
    }

    #[test]
    fn scope_completion_includes_fn_defs() {
        // Functions defined in the file should appear as FUNCTION completions.
        let source = "fn greet(name) do\nname\nend\n";
        let items = completions_at(source, 3, 0);

        let fn_labels: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::FUNCTION))
            .map(|i| i.label.as_str())
            .collect();

        assert!(
            fn_labels.contains(&"greet"),
            "should contain 'greet' function, got: {:?}",
            fn_labels
        );
    }
}
