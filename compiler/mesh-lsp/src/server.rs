//! Tower-lsp Backend implementation for the Mesh language server.
//!
//! Implements the LSP `LanguageServer` trait with support for:
//! - textDocument/didOpen, didChange, didClose (diagnostics, dropped
//!   when the document changes again before its analysis finishes)
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

use mesh_diagnostics::SourceFile;
use mesh_parser::ast::item::ImplDef;
use mesh_parser::ast::AstNode;
use mesh_parser::SyntaxKind;
use mesh_parser::SyntaxNode;

use crate::analysis::{self, AnalysisResult};
use crate::syntax::name_child_text;

/// Per-document state stored in the server.
struct DocumentState {
    /// The latest source text.
    source: String,
    /// Which edit `source` is; see [`Documents::edit`].
    revision: u64,
    /// The analysis of the latest text analyzed: of an older text while an
    /// analysis runs, and `None` until the first one is done.
    analysis: Option<AnalysisResult>,
}

/// The open documents, by URI.
#[derive(Default)]
struct Documents {
    open: HashMap<String, DocumentState>,
    last_revision: u64,
}

/// One text of a document to analyze, with the open documents as they
/// were when it was the latest.
struct Pending {
    uri: Url,
    /// The client's version of `source`.
    version: i32,
    revision: u64,
    source: String,
    open_documents: Vec<(String, String)>,
}

impl Documents {
    /// Records `source` as the document's latest text. Each edit gets a
    /// revision no edit had before, of any document and across closing and
    /// reopening, so an analysis of an older text is recognized.
    fn edit(&mut self, uri: &Url, version: i32, source: String) -> Pending {
        self.last_revision += 1;
        let revision = self.last_revision;
        let analysis = self
            .open
            .remove(uri.as_str())
            .and_then(|state| state.analysis);
        self.open.insert(
            uri.to_string(),
            DocumentState {
                source: source.clone(),
                revision,
                analysis,
            },
        );
        Pending {
            uri: uri.clone(),
            version,
            revision,
            source,
            open_documents: self
                .open
                .iter()
                .map(|(uri, state)| (uri.clone(), state.source.clone()))
                .collect(),
        }
    }

    fn close(&mut self, uri: &Url) {
        self.open.remove(uri.as_str());
    }

    /// Keeps `analysis` of `pending`'s text, and returns the diagnostics to
    /// publish. `None` when the document was edited or closed since: they
    /// are about a text the editor no longer shows.
    fn finish(
        &mut self,
        pending: &Pending,
        analysis: AnalysisResult,
    ) -> Option<Vec<tower_lsp::lsp_types::Diagnostic>> {
        let state = self
            .open
            .get_mut(pending.uri.as_str())
            .filter(|state| state.revision == pending.revision)?;
        let diagnostics = analysis.lsp_diagnostics(&pending.uri);
        state.analysis = Some(analysis);
        Some(diagnostics)
    }

    /// The latest analysis of the document at `uri`.
    fn analysis(&self, uri: &Url) -> Option<&AnalysisResult> {
        self.open.get(uri.as_str())?.analysis.as_ref()
    }
}

/// Type checking recurses as deep as the program nests, so analysis gets
/// the stack meshc's compiler thread has.
const ANALYSIS_STACK_BYTES: usize = 512 * 1024 * 1024;

/// Analyzes `pending` on a thread of its own, so the server goes on
/// reading the editor's messages, and an edit made meanwhile is seen.
async fn analyze(pending: Pending) -> Option<(Pending, AnalysisResult)> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("mesh-lsp-analysis".to_string())
        .stack_size(ANALYSIS_STACK_BYTES)
        .spawn(move || {
            let analysis = analysis::analyze_document(
                pending.uri.as_str(),
                &pending.source,
                &pending.open_documents,
            );
            let _ = sender.send((pending, analysis));
        })
        .ok()?;
    receiver.await.ok()
}

/// The Mesh LSP server backend.
///
/// Holds a reference to the LSP client (for sending notifications like
/// diagnostics) and an in-memory document store keyed by URI.
pub struct MeshBackend {
    /// The LSP client used to send notifications (e.g., publishDiagnostics).
    client: Client,
    documents: Mutex<Documents>,
    /// Held from finding an analysis current until its diagnostics are
    /// sent, so that an older text's cannot be sent after a newer one's.
    publishing: tokio::sync::Mutex<()>,
}

impl MeshBackend {
    /// Create a new Mesh LSP backend.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(Documents::default()),
            publishing: tokio::sync::Mutex::new(()),
        }
    }

    /// Analyze the document's new text and publish its diagnostics, unless
    /// it changes again first.
    async fn analyze_and_publish(&self, uri: Url, version: i32, source: String) {
        let pending = self.documents.lock().unwrap().edit(&uri, version, source);
        let Some((pending, analysis)) = analyze(pending).await else {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("The analysis of {uri} did not finish"),
                )
                .await;
            return;
        };
        let _publishing = self.publishing.lock().await;
        let diagnostics = self.documents.lock().unwrap().finish(&pending, analysis);
        if let Some(diagnostics) = diagnostics {
            self.client
                .publish_diagnostics(pending.uri, diagnostics, Some(pending.version))
                .await;
        }
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
        let document = params.text_document;
        self.analyze_and_publish(document.uri, document.version, document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let document = params.text_document;
        // We use TextDocumentSyncKind::FULL, so the first content change
        // contains the entire document.
        if let Some(change) = params.content_changes.into_iter().next() {
            self.analyze_and_publish(document.uri, document.version, change.text)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.lock().unwrap().close(&uri);
        // Clear diagnostics for the closed document.
        let _publishing = self.publishing.lock().await;
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let Some(analysis) = docs.analysis(uri) else {
            return Ok(None);
        };

        // Positions are converted against the analyzed text (see
        // `AnalysisResult::file`).
        let type_info = analysis::type_at_position(&analysis.file, &analysis.typeck, &position);

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
        let position = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let Some(analysis) = docs.analysis(&uri) else {
            return Ok(None);
        };

        // Convert LSP position to byte offset.
        let offset = match analysis::position_to_offset(&analysis.file, &position) {
            Some(o) => o,
            None => return Ok(None),
        };

        // Traverse the CST to find the definition.
        let root = analysis.parse.syntax();
        let def_range = match crate::definition::find_definition(&root, offset) {
            Some(r) => r,
            None => return Ok(None),
        };

        let location = Location {
            range: analysis::text_range_to_range(&analysis.file, def_range),
            uri,
        };

        Ok(Some(GotoDefinitionResponse::Scalar(location)))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let docs = self.documents.lock().unwrap();
        let Some(analysis) = docs.analysis(&params.text_document.uri) else {
            return Ok(None);
        };

        let root = analysis.parse.syntax();
        let symbols = collect_symbols(&analysis.file, &root);

        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position.position;

        let docs = self.documents.lock().unwrap();
        let Some(analysis) = docs.analysis(&params.text_document_position.text_document.uri) else {
            return Ok(None);
        };

        let items = crate::completion::compute_completions(analysis, &position);

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let Some(analysis) = docs.analysis(uri) else {
            return Ok(None);
        };

        Ok(crate::signature_help::compute_signature_help(
            analysis, &position,
        ))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let docs = self.documents.lock().unwrap();
        let Some(doc) = docs.open.get(params.text_document.uri.as_str()) else {
            return Ok(None);
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
        let end = analysis::offset_to_position(
            &SourceFile::new("", doc.source.as_str()),
            doc.source.len(),
        );
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
fn collect_symbols(file: &SourceFile, node: &SyntaxNode) -> Vec<DocumentSymbol> {
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
            // An impl has no NAME child: it is named for its interface and
            // type, and selected at its interface.
            let override_name = ImplDef::cast(child.clone()).map(|impl_def| {
                let path = child.children().find(|n| n.kind() == SyntaxKind::PATH);
                (impl_name(&impl_def), path)
            });
            let mut symbol = make_symbol(file, &child, kind, override_name)?;
            // An interface's members are its methods; a module's, an actor's,
            // a service's and an impl's are the definitions in their bodies.
            let members: Vec<DocumentSymbol> = match child.kind() {
                SyntaxKind::INTERFACE_DEF => child
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::INTERFACE_METHOD)
                    .filter_map(|method| make_symbol(file, &method, SymbolKind::FUNCTION, None))
                    .collect(),
                SyntaxKind::MODULE_DEF
                | SyntaxKind::ACTOR_DEF
                | SyntaxKind::SERVICE_DEF
                | SyntaxKind::IMPL_DEF => child
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::BLOCK)
                    .flat_map(|block| collect_symbols(file, &block))
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

/// An impl's display name: `impl Show for Point`.
fn impl_name(impl_def: &ImplDef) -> String {
    match (impl_def.interface_name(), impl_def.type_name()) {
        (Some(interface), Some(ty)) => format!("impl {} for {}", interface.text(), ty.text()),
        (Some(interface), None) => format!("impl {}", interface.text()),
        _ => "impl".to_string(),
    }
}

/// Construct a `DocumentSymbol` from a CST node: its whole range, and its
/// name's for the selection.
///
/// The `override_name` parameter allows callers (e.g., for IMPL_DEF) to provide
/// a custom name and an alternative node for the selection range; without
/// one, the selection is the whole node.
fn make_symbol(
    file: &SourceFile,
    node: &SyntaxNode,
    kind: SymbolKind,
    override_name: Option<(String, Option<SyntaxNode>)>,
) -> Option<DocumentSymbol> {
    let (name, selection) = match override_name {
        Some(name_and_selection) => name_and_selection,
        None => (
            name_child_text(node)?,
            node.children().find(|n| n.kind() == SyntaxKind::NAME),
        ),
    };
    let range = analysis::text_range_to_range(file, node.text_range());
    let selection_range = selection.map_or(range, |selection| {
        analysis::text_range_to_range(file, selection.text_range())
    });

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

    /// Every kind of definition is a symbol, with its members nested.
    #[test]
    fn every_definition_kind_is_a_document_symbol() {
        let source = "module Geo do\n  pub fn area(r) = r * r\nend\n\nstruct Point do\n  x :: Int\nend\n\ntype Shape do\n  Dot\nend\n\ntype Id = Int\n\ninterface Named do\n  fn name(self) -> String\nend\n\nimpl Named for Point do\n  fn name(self) -> String do\n    \"p\"\n  end\nend\n\nactor Pinger() do\n  1\nend\n\nservice Counter do\n  fn init() -> Int do\n    0\n  end\n  call Get() :: Int do |n|\n    (n, n)\n  end\n  cast Add(k :: Int) do |n|\n    n + k\n  end\nend\n\nsupervisor Sup do\n  strategy: one_for_one\nend\n\nlet limit = 3\n";
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let symbols = collect_symbols(&SourceFile::new("", source), &parse.syntax());
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

    /// An impl still being typed is named for as much of it as there is.
    #[test]
    fn an_incomplete_impl_is_named_for_what_it_has() {
        for (source, expected) in [
            ("impl Named do\nend\n", "impl Named"),
            ("impl do\nend\n", "impl"),
        ] {
            let parse = mesh_parser::parse(source);
            let names: Vec<_> = collect_symbols(&SourceFile::new("", source), &parse.syntax())
                .into_iter()
                .map(|symbol| symbol.name)
                .collect();
            assert_eq!(names, [expected], "{source:?}");
        }
    }

    fn analyzed(pending: &Pending) -> AnalysisResult {
        analysis::analyze_document(
            pending.uri.as_str(),
            &pending.source,
            &pending.open_documents,
        )
    }

    /// An analysis that finishes after the document changed or closed is
    /// about a text the editor no longer shows: it is not published, and
    /// the newer text's analysis stays the one queries read.
    #[test]
    fn an_analysis_of_a_text_since_replaced_is_dropped() {
        let uri = Url::parse("file:///nowhere/main.mpl").unwrap();
        let broken = "fn main() do\n  let n :: Int = \"one\"\nend\n";
        let fixed = "fn main() do\n  let n :: Int = 1\nend\n";
        let mut documents = Documents::default();

        let first = documents.edit(&uri, 1, broken.to_string());
        let second = documents.edit(&uri, 2, fixed.to_string());
        let (first_analysis, second_analysis) = (analyzed(&first), analyzed(&second));
        assert_eq!(documents.finish(&second, second_analysis), Some(vec![]));
        assert_eq!(documents.finish(&first, first_analysis), None);
        assert_eq!(documents.analysis(&uri).unwrap().source(), fixed);

        let third = documents.edit(&uri, 3, broken.to_string());
        documents.close(&uri);
        let third_analysis = analyzed(&third);
        assert_eq!(documents.finish(&third, third_analysis), None);

        // Reopened at the version it had before it closed, it is a new text.
        let reopened = documents.edit(&uri, 3, fixed.to_string());
        assert_eq!(documents.finish(&third, analyzed(&third)), None);
        let published = documents.finish(&reopened, analyzed(&reopened));
        assert_eq!(published, Some(vec![]));
    }

    /// Verify that the server advertises the expected capabilities, and
    /// shuts down when asked.
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
        assert!(server.shutdown().await.is_ok());
    }
}
