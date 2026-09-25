//! Tower-lsp Backend implementation for the Mesh language server.
//!
//! Implements the LSP `LanguageServer` trait with support for:
//! - textDocument/didOpen, didChange, didClose (diagnostics)
//! - textDocument/hover (type information)
//! - textDocument/definition (go-to-definition)
//! - textDocument/documentSymbol (Outline, Breadcrumbs, Go-to-Symbol)
//! - textDocument/completion (keyword, type, snippet, scope-aware completions)
//! - textDocument/signatureHelp (parameter info and active parameter tracking)
//! - Server capabilities advertisement

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use mesh_parser::SyntaxKind;
use mesh_parser::SyntaxNode;

use crate::analysis::{self, AnalysisResult};

/// Per-document state stored in the server.
struct DocumentState {
    /// The latest source text.
    source: String,
    /// The latest analysis result.
    analysis: AnalysisResult,
}

/// The Mesh LSP server backend.
///
/// Holds a reference to the LSP client (for sending notifications like
/// diagnostics) and an in-memory document store keyed by URI.
pub struct MeshBackend {
    /// The LSP client used to send notifications (e.g., publishDiagnostics).
    client: Client,
    /// Document store: URI -> (source, analysis result).
    documents: Mutex<HashMap<String, DocumentState>>,
}

impl MeshBackend {
    /// Create a new Mesh LSP backend.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
        }
    }

    /// Analyze a document and publish diagnostics.
    async fn analyze_and_publish(&self, uri: Url, source: String) {
        let uri_str = uri.to_string();
        let open_documents = {
            let docs = self.documents.lock().unwrap();
            docs.iter()
                .map(|(doc_uri, state)| (doc_uri.clone(), state.source.clone()))
                .collect::<Vec<_>>()
        };
        let result = analysis::analyze_document(&uri_str, &source, &open_documents);
        let diagnostics = result.diagnostics.clone();

        // Store document state for hover queries.
        {
            let mut docs = self.documents.lock().unwrap();
            docs.insert(
                uri_str,
                DocumentState {
                    source,
                    analysis: result,
                },
            );
        }

        // Publish diagnostics to the client.
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for MeshBackend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: None,
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: None,
                    work_done_progress_options: Default::default(),
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Mesh LSP server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let source = params.text_document.text;
        self.analyze_and_publish(uri, source).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        // We use TextDocumentSyncKind::FULL, so the first content change
        // contains the entire document.
        if let Some(change) = params.content_changes.into_iter().next() {
            self.analyze_and_publish(uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri_str = params.text_document.uri.to_string();

        // Remove document from store.
        {
            let mut docs = self.documents.lock().unwrap();
            docs.remove(&uri_str);
        }

        // Clear diagnostics for the closed document.
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri_str = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(&uri_str) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        // Positions are converted against the analyzed text (see
        // `AnalysisResult::source`).
        let type_info =
            analysis::type_at_position(&doc.analysis.source, &doc.analysis.typeck, &position);

        match type_info {
            Some(ty_str) => Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```mesh\n{}\n```", ty_str),
                }),
                range: None,
            })),
            None => Ok(None),
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let uri_str = uri.to_string();
        let position = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(&uri_str) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        // Convert LSP position to byte offset.
        let offset = match analysis::position_to_offset_pub(&doc.analysis.source, &position) {
            Some(o) => o,
            None => return Ok(None),
        };

        // Traverse the CST to find the definition.
        let root = doc.analysis.parse.syntax();
        let def_range =
            match crate::definition::find_definition(&doc.analysis.source, &root, offset) {
                Some(r) => r,
                None => return Ok(None),
            };

        // Convert the definition range (in rowan tree coordinates) back to
        // source byte offsets, then to LSP positions.
        let start_tree: usize = def_range.start().into();
        let end_tree: usize = def_range.end().into();
        let start_source =
            crate::definition::tree_to_source_offset(&doc.analysis.source, start_tree)
                .unwrap_or(start_tree);
        let end_source = crate::definition::tree_to_source_offset(&doc.analysis.source, end_tree)
            .unwrap_or(end_tree);
        let start = analysis::offset_to_position(&doc.analysis.source, start_source);
        let end = analysis::offset_to_position(&doc.analysis.source, end_source);

        let location = Location {
            uri,
            range: Range::new(start, end),
        };

        Ok(Some(GotoDefinitionResponse::Scalar(location)))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri_str = params.text_document.uri.to_string();

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(&uri_str) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let root = doc.analysis.parse.syntax();
        let symbols = collect_symbols(&doc.analysis.source, &root);

        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri_str = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(&uri_str) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let items =
            crate::completion::compute_completions(&doc.analysis.source, &doc.analysis, &position);

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri_str = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(&uri_str) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::signature_help::compute_signature_help(
            &doc.analysis.source,
            &doc.analysis,
            &position,
        ))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri_str = params.text_document.uri.to_string();
        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(&uri_str) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let config = mesh_fmt::FormatConfig {
            indent_size: params.options.tab_size as usize,
            ..Default::default()
        };
        let formatted = mesh_fmt::format_source(&doc.source, &config);

        if formatted == doc.source {
            return Ok(None);
        }

        // Full-document replacement: single TextEdit covering entire document.
        let end = analysis::offset_to_position(&doc.source, doc.source.len());
        Ok(Some(vec![TextEdit {
            range: Range::new(Position::new(0, 0), end),
            new_text: formatted,
        }]))
    }
}

/// Walk CST children and collect document symbols for the Outline panel.
///
/// Recursively descends into container nodes (modules, actors, services,
/// interfaces, impls) to produce a hierarchical symbol tree.
fn collect_symbols(source: &str, node: &SyntaxNode) -> Vec<DocumentSymbol> {
    node.children()
        .filter_map(|child| {
            let kind = match child.kind() {
                SyntaxKind::FN_DEF | SyntaxKind::CALL_HANDLER | SyntaxKind::CAST_HANDLER => {
                    SymbolKind::FUNCTION
                }
                SyntaxKind::STRUCT_DEF => SymbolKind::STRUCT,
                SyntaxKind::MODULE_DEF => SymbolKind::MODULE,
                SyntaxKind::ACTOR_DEF | SyntaxKind::SERVICE_DEF | SyntaxKind::SUPERVISOR_DEF => {
                    SymbolKind::CLASS
                }
                SyntaxKind::INTERFACE_DEF => SymbolKind::INTERFACE,
                SyntaxKind::IMPL_DEF => SymbolKind::OBJECT,
                SyntaxKind::LET_BINDING => SymbolKind::VARIABLE,
                SyntaxKind::SUM_TYPE_DEF => SymbolKind::ENUM,
                SyntaxKind::TYPE_ALIAS_DEF => SymbolKind::TYPE_PARAMETER,
                _ => return None,
            };
            let mut symbol = if child.kind() == SyntaxKind::IMPL_DEF {
                // An impl has no NAME child: it is named for its interface and type.
                let name = extract_impl_name(&child);
                let path = child.children().find(|n| n.kind() == SyntaxKind::PATH);
                make_symbol(source, &child, kind, Some((&name, path.as_ref())))
            } else {
                make_symbol(source, &child, kind, None)
            }?;
            // An interface's members are its methods; a module's, an actor's,
            // a service's and an impl's are the definitions in their bodies.
            let members: Vec<DocumentSymbol> = match child.kind() {
                SyntaxKind::INTERFACE_DEF => child
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::INTERFACE_METHOD)
                    .filter_map(|method| make_symbol(source, &method, SymbolKind::FUNCTION, None))
                    .collect(),
                SyntaxKind::MODULE_DEF
                | SyntaxKind::ACTOR_DEF
                | SyntaxKind::SERVICE_DEF
                | SyntaxKind::IMPL_DEF => child
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::BLOCK)
                    .flat_map(|block| collect_symbols(source, &block))
                    .collect(),
                _ => Vec::new(),
            };
            if !members.is_empty() {
                symbol.children = Some(members);
            }
            Some(symbol)
        })
        .collect()
}

/// Extract a display name for an IMPL_DEF node: `impl Show for Point`.
fn extract_impl_name(node: &SyntaxNode) -> String {
    use mesh_parser::ast::AstNode;
    let Some(impl_def) = mesh_parser::ast::item::ImplDef::cast(node.clone()) else {
        return "impl".to_string();
    };
    match (impl_def.interface_name(), impl_def.type_name()) {
        (Some(interface), Some(ty)) => format!("impl {} for {}", interface.text(), ty.text()),
        (Some(interface), None) => format!("impl {}", interface.text()),
        _ => "impl".to_string(),
    }
}

/// Construct a `DocumentSymbol` from a CST node.
///
/// Computes the full range (entire definition) and selection range (name only)
/// using the rowan-to-source offset conversion chain.
///
/// The `override_name` parameter allows callers (e.g., for IMPL_DEF) to provide
/// a custom name and an alternative node for the selection range.
fn make_symbol(
    source: &str,
    node: &SyntaxNode,
    kind: SymbolKind,
    override_name: Option<(&str, Option<&SyntaxNode>)>,
) -> Option<DocumentSymbol> {
    let (name, sel_range_node) = match override_name {
        Some((n, sel_node)) => (n.to_string(), sel_node),
        None => {
            // Find the NAME child and extract the IDENT token text.
            let name_text = node
                .children()
                .find(|n| n.kind() == SyntaxKind::NAME)
                .and_then(|name_node| {
                    name_node
                        .children_with_tokens()
                        .filter_map(|it| it.into_token())
                        .find(|t| t.kind() == SyntaxKind::IDENT)
                        .map(|t| t.text().to_string())
                })?;
            (name_text, None)
        }
    };

    // Compute the full range of the node.
    let node_range = node.text_range();
    let range_start_tree: usize = node_range.start().into();
    let range_end_tree: usize = node_range.end().into();
    let range_start_source = crate::definition::tree_to_source_offset(source, range_start_tree)?;
    let range_end_source = crate::definition::tree_to_source_offset(source, range_end_tree)?;

    let range = Range::new(
        analysis::offset_to_position(source, range_start_source),
        analysis::offset_to_position(source, range_end_source),
    );

    // Compute the selection range (name identifier only).
    let selection_range = if let Some(sel_node) = sel_range_node {
        // Use the provided node (e.g., PATH for IMPL_DEF).
        let sel_text_range = sel_node.text_range();
        let sel_start_tree: usize = sel_text_range.start().into();
        let sel_end_tree: usize = sel_text_range.end().into();
        let sel_start_source = crate::definition::tree_to_source_offset(source, sel_start_tree)?;
        let sel_end_source = crate::definition::tree_to_source_offset(source, sel_end_tree)?;
        Range::new(
            analysis::offset_to_position(source, sel_start_source),
            analysis::offset_to_position(source, sel_end_source),
        )
    } else {
        // Find the NAME child for selection range.
        let name_node = node.children().find(|n| n.kind() == SyntaxKind::NAME)?;
        let name_text_range = name_node.text_range();
        let sel_start_tree: usize = name_text_range.start().into();
        let sel_end_tree: usize = name_text_range.end().into();
        let sel_start_source = crate::definition::tree_to_source_offset(source, sel_start_tree)?;
        let sel_end_source = crate::definition::tree_to_source_offset(source, sel_end_tree)?;
        Range::new(
            analysis::offset_to_position(source, sel_start_source),
            analysis::offset_to_position(source, sel_end_source),
        )
    };

    #[allow(deprecated)] // `deprecated` field is deprecated but required by the struct
    Some(DocumentSymbol {
        name,
        detail: None,
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the server advertises the expected capabilities.
    /// Every kind of definition is a symbol, with its members nested.
    #[test]
    fn every_definition_kind_is_a_document_symbol() {
        let source = "module Geo do\n  pub fn area(r) = r * r\nend\n\nstruct Point do\n  x :: Int\nend\n\ntype Shape do\n  Dot\nend\n\ntype Id = Int\n\ninterface Named do\n  fn name(self) -> String\nend\n\nimpl Named for Point do\n  fn name(self) -> String do\n    \"p\"\n  end\nend\n\nactor Pinger() do\n  1\nend\n\nservice Counter do\n  fn init() -> Int do\n    0\n  end\n  call Get() :: Int do |n|\n    (n, n)\n  end\n  cast Add(k :: Int) do |n|\n    n + k\n  end\nend\n\nsupervisor Sup do\n  strategy: one_for_one\nend\n\nlet limit = 3\n";
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let symbols = collect_symbols(source, &parse.syntax());
        let described: Vec<(String, SymbolKind, Vec<String>)> = symbols
            .iter()
            .map(|symbol| {
                let members = symbol
                    .children
                    .iter()
                    .flatten()
                    .map(|member| member.name.clone())
                    .collect();
                (symbol.name.clone(), symbol.kind, members)
            })
            .collect();
        let expected = [
            ("Geo", SymbolKind::MODULE, vec!["area"]),
            ("Point", SymbolKind::STRUCT, vec![]),
            ("Shape", SymbolKind::ENUM, vec![]),
            ("Id", SymbolKind::TYPE_PARAMETER, vec![]),
            ("Named", SymbolKind::INTERFACE, vec!["name"]),
            ("impl Named for Point", SymbolKind::OBJECT, vec!["name"]),
            ("Pinger", SymbolKind::CLASS, vec![]),
            ("Counter", SymbolKind::CLASS, vec!["init", "Get", "Add"]),
            ("Sup", SymbolKind::CLASS, vec![]),
            ("limit", SymbolKind::VARIABLE, vec![]),
        ];
        assert_eq!(described.len(), expected.len(), "{described:?}");
        for ((name, kind, members), (want_name, want_kind, want_members)) in
            described.iter().zip(expected)
        {
            assert_eq!((name.as_str(), *kind), (want_name, want_kind));
            assert_eq!(members, &want_members);
        }
    }

    #[tokio::test]
    async fn server_capabilities() {
        let (service, _) = tower_lsp::LspService::new(MeshBackend::new);
        let server = service.inner();
        let result = server
            .initialize(InitializeParams::default())
            .await
            .unwrap();

        let caps = result.capabilities;
        assert!(caps.hover_provider.is_some());
        assert!(caps.text_document_sync.is_some());
        assert!(caps.document_symbol_provider.is_some());
        assert!(caps.completion_provider.is_some());
        assert!(caps.signature_help_provider.is_some());
        assert!(caps.document_formatting_provider.is_some());
    }
}
