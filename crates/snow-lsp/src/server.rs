//! Tower-lsp Backend implementation for the Snow language server.
//!
//! Implements the LSP `LanguageServer` trait with support for:
//! - textDocument/didOpen, didChange, didClose (diagnostics)
//! - textDocument/hover (type information)
//! - Server capabilities advertisement

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::analysis::{self, AnalysisResult};

/// Per-document state stored in the server.
struct DocumentState {
    /// The latest source text.
    source: String,
    /// The latest analysis result.
    analysis: AnalysisResult,
}

/// The Snow LSP server backend.
///
/// Holds a reference to the LSP client (for sending notifications like
/// diagnostics) and an in-memory document store keyed by URI.
pub struct SnowBackend {
    /// The LSP client used to send notifications (e.g., publishDiagnostics).
    client: Client,
    /// Document store: URI -> (source, analysis result).
    documents: Mutex<HashMap<String, DocumentState>>,
}

impl SnowBackend {
    /// Create a new Snow LSP backend.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
        }
    }

    /// Analyze a document and publish diagnostics.
    async fn analyze_and_publish(&self, uri: Url, source: String) {
        let uri_str = uri.to_string();
        let result = analysis::analyze_document(&uri_str, &source);
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
impl LanguageServer for SnowBackend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Snow LSP server initialized")
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

        let type_info =
            analysis::type_at_position(&doc.source, &doc.analysis.typeck, &position);

        match type_info {
            Some(ty_str) => Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```snow\n{}\n```", ty_str),
                }),
                range: None,
            })),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the server advertises the expected capabilities.
    #[tokio::test]
    async fn server_capabilities() {
        let (service, _) = tower_lsp::LspService::new(|client| SnowBackend::new(client));
        let server = service.inner();
        let result = server
            .initialize(InitializeParams::default())
            .await
            .unwrap();

        let caps = result.capabilities;
        assert!(caps.hover_provider.is_some());
        assert!(caps.text_document_sync.is_some());
    }
}
