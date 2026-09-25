//! Mesh Language Server Protocol (LSP) implementation.
//!
//! This crate provides an LSP server for the Mesh programming language,
//! enabling real-time feedback in editors like VS Code and Neovim:
//!
//! - **Diagnostics**: Parse errors and type errors displayed inline
//! - **Hover**: Type information shown on hover
//! - **Go-to-definition**: Navigate to variable, function, and type definitions
//! - **Completion**: Keywords, built-in types, snippets, and scope-aware names
//! - **Signature help**: Parameter info and active parameter highlighting in function calls
//!
//! The server communicates via stdin/stdout using the LSP protocol over
//! JSON-RPC, powered by the `tower-lsp` framework.

pub mod analysis;
pub mod completion;
pub mod definition;
pub mod server;
pub mod signature_help;
mod syntax;

use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::sync::Notify;
use tower_lsp::jsonrpc::{Request, Response};
use tower_lsp::{ExitedError, LspService, Server};
use tower_service::Service;

use server::MeshBackend;

/// Run the Mesh LSP server on stdin/stdout.
///
/// This is the main entry point called by `meshc lsp`. It runs until the
/// client says `exit` or closes stdin. The caller should not wait for the
/// runtime's stdin read, which cannot be cancelled.
pub async fn run_server() {
    let (service, socket) = LspService::new(MeshBackend::new);
    let exited = Arc::new(Notify::new());
    let service = ExitWatch {
        service,
        exited: exited.clone(),
    };
    // tower-lsp reads on after `exit` until stdin ends, so a client that
    // waits for the server to end, as the protocol has it, would wait for
    // good.
    tokio::select! {
        () = Server::new(tokio::io::stdin(), tokio::io::stdout(), socket).serve(service) => {}
        () = exited.notified() => {}
    }
}

/// The language server, noting when the client says `exit`.
struct ExitWatch {
    service: LspService<MeshBackend>,
    exited: Arc<Notify>,
}

impl Service<Request> for ExitWatch {
    type Response = Option<Response>;
    type Error = ExitedError;
    type Future = <LspService<MeshBackend> as Service<Request>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), ExitedError>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        if request.method() == "exit" {
            self.exited.notify_one();
        }
        self.service.call(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_server_stops_when_the_client_says_exit() {
        let (service, _socket) = LspService::new(MeshBackend::new);
        let exited = Arc::new(Notify::new());
        let mut watch = ExitWatch {
            service,
            exited: exited.clone(),
        };
        std::future::poll_fn(|cx| watch.poll_ready(cx))
            .await
            .unwrap();
        watch.call(Request::build("exit").finish()).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), exited.notified())
            .await
            .expect("exit is noticed");
    }
}
