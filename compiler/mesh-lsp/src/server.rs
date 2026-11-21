//! Tower-lsp Backend implementation for the Mesh language server.
//!
//! Implements the LSP `LanguageServer` trait with support for:
//! - textDocument/didOpen, didChange, didClose (diagnostics, dropped
//!   when the project changes again before its analysis finishes)
//! - textDocument/codeAction (quick fixes for diagnostic edits)
//! - textDocument/hover (type information)
//! - textDocument/definition (go-to-definition)
//! - textDocument/documentSymbol (Outline, Breadcrumbs, Go-to-Symbol)
//! - textDocument/completion (keyword, type, snippet, scope-aware completions)
//! - textDocument/signatureHelp (parameter info and active parameter tracking)
//! - Server capabilities advertisement

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
use crate::diagnostics::{self, PublishedDiagnostic};
use crate::syntax::name_child_text;

/// Per-document state stored in the server.
struct DocumentState {
    /// The latest source text.
    source: String,
    /// Which edit `source` is; see [`Documents::edit`].
    revision: u64,
    version: i32,
    path: Option<PathBuf>,
    scope: DiagnosticScope,
    owner: AnalysisOwner,
    /// The analysis of the latest text analyzed: of an older text while an
    /// analysis runs, and `None` until the first one is done.
    analysis: Option<AnalysisResult>,
}

/// The open documents, by URI.
#[derive(Default)]
struct Documents {
    open: HashMap<Url, DocumentState>,
    diagnostics: HashMap<DiagnosticScope, ScopeDiagnostics>,
    last_revision: u64,
}

#[derive(Clone, Eq, Hash, PartialEq)]
enum DiagnosticScope {
    Project(PathBuf),
    Loose(Url),
}

/// Which analysis owns a set of published diagnostics inside a scope.
#[derive(Clone, Eq, Hash, PartialEq)]
enum AnalysisOwner {
    Project,
    TestHelpers,
    TestFile(Url),
    Loose(Url),
}

#[derive(Default)]
struct OwnerDiagnostics {
    revision: u64,
    published_revision: u64,
    by_uri: HashMap<Url, Vec<PublishedDiagnostic>>,
}

#[derive(Default)]
struct ScopeDiagnostics {
    owners: HashMap<AnalysisOwner, OwnerDiagnostics>,
    by_uri: HashMap<Url, Vec<PublishedDiagnostic>>,
}

impl ScopeDiagnostics {
    fn recompute_effective(&mut self) -> HashMap<Url, Vec<PublishedDiagnostic>> {
        let mut uris = HashSet::new();
        for owner in self.owners.values() {
            uris.extend(owner.by_uri.keys().cloned());
        }
        let mut next = HashMap::with_capacity(uris.len());
        for uri in uris {
            let diagnostics = self
                .owners
                .get(&AnalysisOwner::TestFile(uri.clone()))
                .and_then(|owner| owner.by_uri.get(&uri))
                .or_else(|| {
                    self.owners
                        .get(&AnalysisOwner::Project)
                        .and_then(|owner| owner.by_uri.get(&uri))
                })
                .or_else(|| {
                    self.owners
                        .get(&AnalysisOwner::TestHelpers)
                        .and_then(|owner| owner.by_uri.get(&uri))
                })
                .or_else(|| {
                    self.owners
                        .values()
                        .filter_map(|owner| {
                            owner
                                .by_uri
                                .get(&uri)
                                .map(|diags| (owner.published_revision, diags))
                        })
                        .max_by_key(|(revision, _)| *revision)
                        .map(|(_, diags)| diags)
                });
            if let Some(diagnostics) = diagnostics {
                next.insert(uri, diagnostics.clone());
            }
        }
        std::mem::replace(&mut self.by_uri, next)
    }
}

#[derive(Debug, PartialEq)]
struct Publication {
    uri: Url,
    version: Option<i32>,
    diagnostics: Vec<Diagnostic>,
}

/// One text of a document to analyze, with the open documents as they
/// were when it was the latest.
struct Pending {
    uri: Url,
    revision: u64,
    source: String,
    open_documents: Vec<(String, String)>,
}

impl Documents {
    /// Records `source` as the document's latest text. Each edit gets a
    /// revision no edit had before, of any document and across closing and
    /// reopening, so an analysis of an older text is recognized.
    fn edit(&mut self, uri: &Url, version: i32, source: String) -> Vec<Pending> {
        let state = self.open.entry(uri.clone()).or_insert_with(|| {
            let path = analysis::canonical_file_path(uri.as_str());
            let (scope, owner) = match path
                .as_deref()
                .and_then(|path| analysis::find_project_root(path).map(|root| (root, path)))
            {
                Some((root, path)) => {
                    let owner = match analysis::project_role(&root, path) {
                        analysis::ProjectRole::Project => AnalysisOwner::Project,
                        analysis::ProjectRole::TestHelper => AnalysisOwner::TestHelpers,
                        analysis::ProjectRole::TestFile => AnalysisOwner::TestFile(uri.clone()),
                    };
                    (DiagnosticScope::Project(root), owner)
                }
                None => (
                    DiagnosticScope::Loose(uri.clone()),
                    AnalysisOwner::Loose(uri.clone()),
                ),
            };
            DocumentState {
                source: String::new(),
                revision: 0,
                version,
                path,
                scope,
                owner,
                analysis: None,
            }
        });
        state.source = source;
        state.version = version;
        let scope = state.scope.clone();
        let owner = state.owner.clone();
        let mut pending = vec![self.start_analysis(uri)];
        pending.extend(self.dependent_pending(&scope, &owner));
        pending
    }

    fn start_analysis(&mut self, uri: &Url) -> Pending {
        self.last_revision += 1;
        let revision = self.last_revision;
        let state = self.open.get_mut(uri).expect("analyzing an open document");
        state.revision = revision;
        self.diagnostics
            .entry(state.scope.clone())
            .or_default()
            .owners
            .entry(state.owner.clone())
            .or_default()
            .revision = revision;
        let source = state.source.clone();
        let mut open: Vec<_> = self.open.iter().collect();
        open.sort_unstable_by_key(|(_, state)| state.revision);
        Pending {
            uri: uri.clone(),
            revision,
            source,
            open_documents: open
                .into_iter()
                .map(|(uri, state)| (uri.to_string(), state.source.clone()))
                .collect(),
        }
    }

    /// Other analyses in `scope` whose results depend on files owned by
    /// `changed_owner`. Editing a project module refreshes open helpers and
    /// test files; editing a test file affects no other analysis.
    fn dependent_pending(
        &mut self,
        scope: &DiagnosticScope,
        changed_owner: &AnalysisOwner,
    ) -> Vec<Pending> {
        let mut targets = Vec::new();
        match changed_owner {
            AnalysisOwner::Project => {
                if let Some(helper) = self
                    .open
                    .iter()
                    .filter(|(_, state)| {
                        &state.scope == scope && state.owner == AnalysisOwner::TestHelpers
                    })
                    .max_by_key(|(_, state)| state.revision)
                    .map(|(uri, _)| uri.clone())
                {
                    targets.push(helper);
                }
                let mut tests: Vec<Url> = self
                    .open
                    .iter()
                    .filter(|(_, state)| {
                        &state.scope == scope && matches!(state.owner, AnalysisOwner::TestFile(_))
                    })
                    .map(|(uri, _)| uri.clone())
                    .collect();
                tests.sort_by(|a, b| a.as_str().cmp(b.as_str()));
                targets.extend(tests);
            }
            AnalysisOwner::TestHelpers => {
                let mut tests: Vec<Url> = self
                    .open
                    .iter()
                    .filter(|(_, state)| {
                        &state.scope == scope && matches!(state.owner, AnalysisOwner::TestFile(_))
                    })
                    .map(|(uri, _)| uri.clone())
                    .collect();
                tests.sort_by(|a, b| a.as_str().cmp(b.as_str()));
                targets.extend(tests);
            }
            AnalysisOwner::TestFile(_) | AnalysisOwner::Loose(_) => {}
        }
        targets
            .into_iter()
            .map(|uri| self.start_analysis(&uri))
            .collect()
    }

    fn close(&mut self, uri: &Url) -> (Vec<Pending>, Vec<Publication>) {
        let Some(state) = self.open.remove(uri) else {
            return (vec![], vec![]);
        };
        let has_remaining_in_scope = self.open.values().any(|other| other.scope == state.scope);
        if !has_remaining_in_scope {
            let mut by_uri = self.diagnostics.remove(&state.scope).unwrap().by_uri;
            by_uri.entry(uri.clone()).or_default();
            let clears = by_uri
                .into_keys()
                .map(|uri| self.publication(uri, vec![]))
                .collect();
            return (vec![], clears);
        }

        let same_owner = self
            .open
            .iter()
            .filter(|(_, other)| other.scope == state.scope && other.owner == state.owner)
            .max_by_key(|(_, other)| other.revision)
            .map(|(uri, _)| uri.clone());
        let mut pending = Vec::new();
        let mut clears = Vec::new();
        if let Some(remaining) = same_owner {
            pending.push(self.start_analysis(&remaining));
        } else if let Some(scope_diagnostics) = self.diagnostics.get_mut(&state.scope) {
            scope_diagnostics.owners.remove(&state.owner);
            let previous = scope_diagnostics.recompute_effective();
            let cleared_uris: Vec<Url> = previous
                .into_keys()
                .filter(|old_uri| !scope_diagnostics.by_uri.contains_key(old_uri))
                .collect();
            for old_uri in cleared_uris {
                clears.push(self.publication(old_uri, vec![]));
            }
        }
        pending.extend(self.dependent_pending(&state.scope, &state.owner));
        (pending, clears)
    }

    /// Keeps the active document's analysis for queries. Each analysis owns
    /// the diagnostics it reports: a file disappears only when every owner
    /// that reported it stops reporting it or closes.
    fn finish(&mut self, pending: &Pending, analysis: AnalysisResult) -> Option<Vec<Publication>> {
        let state = self
            .open
            .get_mut(&pending.uri)
            .filter(|state| state.revision == pending.revision)?;
        let scope = state.scope.clone();
        let owner = state.owner.clone();
        state.analysis = Some(analysis);
        if self.diagnostics.get(&scope)?.owners.get(&owner)?.revision != pending.revision {
            return None;
        }

        let analysis = self.analysis(&pending.uri)?;
        let mut open_by_path = HashMap::new();
        for (uri, _) in &pending.open_documents {
            let uri = Url::parse(uri).expect("the URI of an open document");
            if let Some(path) = self.open.get(&uri).and_then(|state| state.path.as_deref()) {
                open_by_path.insert(path, uri);
            }
        }
        let uri_for = |path: &Path| {
            if path == analysis.file.path() {
                Some(pending.uri.clone())
            } else {
                open_by_path
                    .get(path)
                    .cloned()
                    .or_else(|| Url::from_file_path(path).ok())
            }
        };
        let owner_by_uri = diagnostics::by_uri(&analysis.diagnostics, &analysis.file, &uri_for);
        let prev_effective = {
            let scope_diagnostics = self.diagnostics.get_mut(&scope)?;
            let owner_diagnostics = scope_diagnostics.owners.get_mut(&owner)?;
            owner_diagnostics.published_revision = pending.revision;
            owner_diagnostics.by_uri = owner_by_uri;
            scope_diagnostics.recompute_effective()
        };

        let current_effective = &self.diagnostics[&scope].by_uri;
        let mut updates = Vec::new();
        for (uri, diagnostics) in current_effective {
            let changed = prev_effective
                .get(uri)
                .map(|prev| prev.iter().map(|d| &d.lsp).collect::<Vec<_>>())
                != Some(diagnostics.iter().map(|d| &d.lsp).collect::<Vec<_>>());
            if uri == &pending.uri || changed {
                updates.push(self.publication(
                    uri.clone(),
                    diagnostics.iter().map(|d| d.lsp.clone()).collect(),
                ));
            }
        }
        if !current_effective.contains_key(&pending.uri) {
            updates.push(self.publication(pending.uri.clone(), vec![]));
        }
        for old_uri in prev_effective.keys() {
            if !current_effective.contains_key(old_uri) && old_uri != &pending.uri {
                updates.push(self.publication(old_uri.clone(), vec![]));
            }
        }
        Some(updates)
    }

    fn publication(&self, uri: Url, diagnostics: Vec<Diagnostic>) -> Publication {
        Publication {
            version: self.open.get(&uri).map(|state| state.version),
            uri,
            diagnostics,
        }
    }

    fn published_diagnostics(&self, uri: &Url) -> Option<&[PublishedDiagnostic]> {
        if let Some(open) = self.open.get(uri) {
            let scope = self.diagnostics.get(&open.scope)?;
            let owner = scope.owners.get(&open.owner)?;
            return (owner.published_revision == owner.revision)
                .then(|| scope.by_uri.get(uri).map(Vec::as_slice))
                .flatten();
        }
        self.diagnostics
            .values()
            .filter_map(|scope| {
                let diagnostics = scope.by_uri.get(uri)?;
                let revision = scope
                    .owners
                    .values()
                    .filter(|owner| {
                        owner.published_revision == owner.revision && owner.by_uri.contains_key(uri)
                    })
                    .map(|owner| owner.published_revision)
                    .max()?;
                Some((revision, diagnostics.as_slice()))
            })
            .max_by_key(|(revision, _)| *revision)
            .map(|(_, diagnostics)| diagnostics)
    }

    /// The latest analysis of the document at `uri`.
    fn analysis(&self, uri: &Url) -> Option<&AnalysisResult> {
        self.open.get(uri)?.analysis.as_ref()
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
        let pending = {
            let _publishing = self.publishing.lock().await;
            self.documents.lock().unwrap().edit(&uri, version, source)
        };
        for item in pending {
            self.publish_analysis(item).await;
        }
    }

    async fn publish_analysis(&self, pending: Pending) {
        let uri = pending.uri.clone();
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
        let updates = self.documents.lock().unwrap().finish(&pending, analysis);
        if let Some(updates) = updates {
            self.publish(updates).await;
        }
    }

    async fn publish(&self, updates: Vec<Publication>) {
        for update in updates {
            self.client
                .publish_diagnostics(update.uri, update.diagnostics, update.version)
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
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        ..Default::default()
                    },
                )),
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
        let pending = {
            let _publishing = self.publishing.lock().await;
            let (pending, clears) = self
                .documents
                .lock()
                .unwrap()
                .close(&params.text_document.uri);
            self.publish(clears).await;
            pending
        };
        for item in pending {
            self.publish_analysis(item).await;
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        if params.context.only.as_ref().is_some_and(|kinds| {
            !kinds
                .iter()
                .any(|kind| kind == &CodeActionKind::QUICKFIX || kind.as_str().is_empty())
        }) {
            return Ok(None);
        }
        let docs = self.documents.lock().unwrap();
        let Some(diagnostics) = docs.published_diagnostics(&params.text_document.uri) else {
            return Ok(None);
        };
        let actions: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                ranges_overlap(d.lsp.range, params.range)
                    || params.context.diagnostics.iter().any(|provided| {
                        provided.range == d.lsp.range
                            && provided.code == d.lsp.code
                            && provided.message == d.lsp.message
                    })
            })
            .filter_map(PublishedDiagnostic::quick_fix)
            .map(CodeActionOrCommand::CodeAction)
            .collect();
        Ok((!actions.is_empty()).then_some(actions))
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
        let Some(doc) = docs.open.get(&params.text_document.uri) else {
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

fn ranges_overlap(first: Range, second: Range) -> bool {
    if first.start == first.end || second.start == second.end {
        first.start <= second.end && second.start <= first.end
    } else {
        first.start < second.end && second.start < first.end
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

        let first = documents.edit(&uri, 1, broken.to_string()).remove(0);
        let second = documents.edit(&uri, 2, fixed.to_string()).remove(0);
        let (first_analysis, second_analysis) = (analyzed(&first), analyzed(&second));
        assert_eq!(
            documents.finish(&second, second_analysis),
            Some(vec![Publication {
                uri: uri.clone(),
                version: Some(2),
                diagnostics: vec![],
            }])
        );
        assert_eq!(documents.finish(&first, first_analysis), None);
        assert_eq!(documents.analysis(&uri).unwrap().source(), fixed);

        let third = documents.edit(&uri, 3, broken.to_string()).remove(0);
        documents.close(&uri);
        let third_analysis = analyzed(&third);
        assert_eq!(documents.finish(&third, third_analysis), None);

        // Reopened at the version it had before it closed, it is a new text.
        let reopened = documents.edit(&uri, 3, fixed.to_string()).remove(0);
        assert_eq!(documents.finish(&third, analyzed(&third)), None);
        let published = documents.finish(&reopened, analyzed(&reopened));
        assert_eq!(
            published,
            Some(vec![Publication {
                uri,
                version: Some(3),
                diagnostics: vec![],
            }])
        );
    }

    #[test]
    fn a_newer_sibling_overlay_invalidates_project_publication() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(
            root.join("mesh.toml"),
            "[package]\nname = \"project\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let main = "from Util import size\nfn main() -> Int do\n  size()\nend\n";
        let util = "pub fn size() -> Int do\n  1\nend\n";
        std::fs::write(root.join("main.mpl"), main).unwrap();
        std::fs::write(root.join("util.mpl"), util).unwrap();
        let main_uri = Url::from_file_path(root.join("main.mpl")).unwrap();
        let util_uri = Url::from_file_path(root.join("util.mpl")).unwrap();

        let mut documents = Documents::default();
        let old = documents.edit(&main_uri, 4, main.to_string()).remove(0);
        let newer = documents.edit(&util_uri, 1, util.to_string()).remove(0);
        assert!(documents.finish(&newer, analyzed(&newer)).is_some());
        assert_eq!(documents.finish(&old, analyzed(&old)), None);
        assert_eq!(documents.analysis(&main_uri).unwrap().source(), main);

        let pending = documents.edit(&main_uri, 5, main.to_string()).remove(0);
        let (mut refreshed, clears) = documents.close(&util_uri);
        assert!(clears.is_empty());
        assert_eq!(documents.finish(&pending, analyzed(&pending)), None);
        let refreshed = refreshed.remove(0);
        assert!(documents.finish(&refreshed, analyzed(&refreshed)).is_some());
    }

    /// A project in a temporary directory: its canonical root, and a URI
    /// for each file written.
    fn test_project(files: &[(&str, &str)]) -> (tempfile::TempDir, Vec<Url>) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(
            root.join("mesh.toml"),
            "[package]\nname = \"project\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let uris = files
            .iter()
            .map(|(path, text)| {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, text).unwrap();
                Url::from_file_path(path).unwrap()
            })
            .collect();
        (temp, uris)
    }

    /// Edits or opens `uri` with `text`, runs every resulting analysis, and
    /// returns every publication emitted.
    fn edit_and_finish(
        documents: &mut Documents,
        uri: &Url,
        version: i32,
        text: &str,
    ) -> Vec<Publication> {
        let mut updates = Vec::new();
        for pending in documents.edit(uri, version, text.to_string()) {
            updates.extend(
                documents
                    .finish(&pending, analyzed(&pending))
                    .expect("the latest analysis is published"),
            );
        }
        updates
    }

    /// Opens `uri` with `text`, and returns what its analysis publishes.
    fn open(documents: &mut Documents, uri: &Url, text: &str) -> Vec<Publication> {
        edit_and_finish(documents, uri, 1, text)
    }

    fn codes(publication: &Publication) -> Vec<String> {
        publication
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match &diagnostic.code {
                Some(NumberOrString::String(code)) => Some(code.clone()),
                _ => None,
            })
            .collect()
    }

    fn published_codes(documents: &Documents, uri: &Url) -> Vec<String> {
        documents
            .published_diagnostics(uri)
            .unwrap_or_default()
            .iter()
            .filter_map(|diagnostic| match &diagnostic.lsp.code {
                Some(NumberOrString::String(code)) => Some(code.clone()),
                _ => None,
            })
            .collect()
    }

    const MAIN: &str = "fn main() do\nend\n";
    const BROKEN_TEST: &str = "test(\"a\") do\n  assert(cuont == 1)\nend\n";

    /// A project analysis does not check test files or the helpers under
    /// `tests/`: what their analyses published stays.
    #[test]
    fn a_project_analysis_keeps_a_test_files_diagnostics() {
        let helper = "pub fn help() -> Int do\n  undefined_name\nend\n";
        let (_temp, uris) = test_project(&[
            ("main.mpl", MAIN),
            ("tests/a.test.mpl", BROKEN_TEST),
            ("tests/helper.mpl", helper),
        ]);
        let [main, test, helper_uri] = &uris[..] else {
            unreachable!()
        };
        let mut documents = Documents::default();

        let published = open(&mut documents, test, BROKEN_TEST);
        let ours = published.iter().find(|p| &p.uri == test).unwrap();
        assert_eq!(codes(ours), ["E0004"], "{published:?}");
        let helper_codes = open(&mut documents, helper_uri, helper)
            .iter()
            .filter(|p| &p.uri == helper_uri)
            .flat_map(codes)
            .collect::<Vec<_>>();
        assert_eq!(helper_codes, ["E0004"]);

        let published = open(&mut documents, main, MAIN);
        assert!(
            published
                .iter()
                .all(|p| !((&p.uri == test || &p.uri == helper_uri) && p.diagnostics.is_empty())),
            "the project analysis cleared a file it did not check: {published:?}"
        );
        assert_eq!(published_codes(&documents, test), ["E0004"]);
        assert_eq!(published_codes(&documents, helper_uri), ["E0004"]);

        // Closed while the project stays open, the test file's diagnostics
        // go with it: no remaining analysis checks it.
        let (_, clears) = documents.close(test);
        assert_eq!(
            clears,
            [Publication {
                uri: test.clone(),
                version: None,
                diagnostics: vec![]
            }]
        );
    }

    /// Each test file is checked as a program of its own, so one test
    /// file's analysis leaves another's diagnostics.
    #[test]
    fn test_files_keep_each_others_diagnostics() {
        let other = "test(\"b\") do\n  assert(totl == 1)\nend\n";
        let (_temp, uris) = test_project(&[
            ("main.mpl", MAIN),
            ("tests/a.test.mpl", BROKEN_TEST),
            ("tests/b.test.mpl", other),
        ]);
        let [_, a, b] = &uris[..] else { unreachable!() };
        let mut documents = Documents::default();

        open(&mut documents, a, BROKEN_TEST);
        let published = open(&mut documents, b, other);
        assert!(published.iter().all(|p| &p.uri != a), "{published:?}");
        assert_eq!(published_codes(&documents, a), ["E0004"]);
        assert_eq!(published_codes(&documents, b), ["E0004"]);
    }

    /// The test program replaces the project's entry, as `meshc test`
    /// builds it: a type both define is no `E0102`.
    #[test]
    fn a_test_file_replaces_the_entry_it_shares_names_with() {
        let main = "struct Point do\n  x :: Int\nend\n\nfn main() do\nend\n";
        let test = "struct Point do\n  y :: Int\nend\n\ntest(\"point\") do\n  let p = Point { y: 1 }\n  assert(p.y == 1)\nend\n";
        let (_temp, uris) = test_project(&[("main.mpl", main), ("tests/point.test.mpl", test)]);
        let [main_uri, test_uri] = &uris[..] else {
            unreachable!()
        };
        let mut documents = Documents::default();

        open(&mut documents, main_uri, main);
        let published = open(&mut documents, test_uri, test);
        let codes: Vec<_> = published.iter().flat_map(codes).collect();
        assert!(codes.is_empty(), "{published:?}");
    }

    #[test]
    fn fixing_a_manifest_clears_its_diagnostic() {
        let (temp, uris) = test_project(&[("main.mpl", MAIN)]);
        let root = temp.path().canonicalize().unwrap();
        let manifest_path = root.join("mesh.toml");
        let manifest_uri = Url::from_file_path(&manifest_path).unwrap();
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"project\"\nversion = 5\n",
        )
        .unwrap();
        let [main] = &uris[..] else { unreachable!() };
        let mut documents = Documents::default();

        let published = open(&mut documents, main, MAIN);
        let manifest_pub = published.iter().find(|p| p.uri == manifest_uri).unwrap();
        assert_eq!(codes(manifest_pub), ["M0002"], "{published:?}");

        std::fs::write(
            &manifest_path,
            "[package]\nname = \"project\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let published = edit_and_finish(&mut documents, main, 2, MAIN);
        let cleared = published.iter().find(|p| p.uri == manifest_uri).unwrap();
        assert!(cleared.diagnostics.is_empty(), "{published:?}");
        assert!(published_codes(&documents, &manifest_uri).is_empty());
    }

    #[test]
    fn deleting_a_module_clears_its_diagnostic() {
        let broken_util = "pub fn size() -> Int do\n  missing_name\nend\n";
        let (temp, uris) = test_project(&[("main.mpl", MAIN), ("util.mpl", broken_util)]);
        let root = temp.path().canonicalize().unwrap();
        let [main, util] = &uris[..] else {
            unreachable!()
        };
        let mut documents = Documents::default();

        let published = open(&mut documents, main, MAIN);
        let util_pub = published.iter().find(|p| &p.uri == util).unwrap();
        assert_eq!(codes(util_pub), ["E0004"], "{published:?}");

        std::fs::remove_file(root.join("util.mpl")).unwrap();
        let published = edit_and_finish(&mut documents, main, 2, MAIN);
        let cleared = published.iter().find(|p| &p.uri == util).unwrap();
        assert!(cleared.diagnostics.is_empty(), "{published:?}");
        assert!(published_codes(&documents, util).is_empty());
    }

    #[test]
    fn closing_a_test_file_clears_diagnostics_on_an_unopened_helper() {
        let helper = "pub fn help() -> Int do\n  undefined_name\nend\n";
        let clean_test = "test(\"a\") do\n  assert(true)\nend\n";
        let (_temp, uris) = test_project(&[
            ("main.mpl", MAIN),
            ("tests/a.test.mpl", clean_test),
            ("tests/helper.mpl", helper),
        ]);
        let [main, test, helper_uri] = &uris[..] else {
            unreachable!()
        };
        let mut documents = Documents::default();

        open(&mut documents, main, MAIN);
        let published = open(&mut documents, test, clean_test);
        let helper_pub = published.iter().find(|p| &p.uri == helper_uri).unwrap();
        assert_eq!(codes(helper_pub), ["E0004"], "{published:?}");

        let (pending, clears) = documents.close(test);
        assert!(pending.is_empty());
        assert_eq!(
            clears,
            [Publication {
                uri: helper_uri.clone(),
                version: None,
                diagnostics: vec![],
            }]
        );
        assert!(published_codes(&documents, helper_uri).is_empty());
    }

    #[test]
    fn editing_a_module_refreshes_an_open_test_file() {
        let util_before = "pub fn other() -> Int do\n  1\nend\n";
        let util_after = "pub fn help() -> Int do\n  1\nend\n";
        let test = "from Util import help\n\ntest(\"uses util\") do\n  assert_eq(help(), 1)\nend\n";
        let (_temp, uris) = test_project(&[
            ("main.mpl", MAIN),
            ("util.mpl", util_before),
            ("tests/a.test.mpl", test),
        ]);
        let [_main, util, test_uri] = &uris[..] else {
            unreachable!()
        };
        let mut documents = Documents::default();

        open(&mut documents, util, util_before);
        let published = open(&mut documents, test_uri, test);
        assert!(
            !published_codes(&documents, test_uri).is_empty(),
            "{published:?}"
        );

        let published = edit_and_finish(&mut documents, util, 2, util_after);
        let refreshed = published
            .iter()
            .rev()
            .find(|p| &p.uri == test_uri)
            .expect("open test file is refreshed when an imported module changes");
        assert!(refreshed.diagnostics.is_empty(), "{published:?}");
        assert!(published_codes(&documents, test_uri).is_empty());
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
