//! Snow Language Server Protocol (LSP) implementation.
//!
//! This crate provides an LSP server for the Snow programming language,
//! enabling real-time feedback in editors like VS Code and Neovim:
//!
//! - **Diagnostics**: Parse errors and type errors displayed inline
//! - **Hover**: Type information shown on hover
//! - **Go-to-definition**: Navigate to variable, function, and type definitions
//!
//! The server communicates via stdin/stdout using the LSP protocol over
//! JSON-RPC, powered by the `tower-lsp` framework.

pub mod analysis;
pub mod definition;
pub mod server;

use tower_lsp::{LspService, Server};

use server::SnowBackend;

/// Run the Snow LSP server on stdin/stdout.
///
/// This is the main entry point called by `snowc lsp`. It sets up the
/// tower-lsp service and runs the event loop until the client disconnects.
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| SnowBackend::new(client));
    Server::new(stdin, stdout, socket).serve(service).await;
}
