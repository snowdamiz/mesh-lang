//! An editor's side of the language server, over an in-memory connection.
// Each test crate uses part of the client.
#![allow(dead_code)]

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tower_lsp::{LspService, Server};

use mesh_lsp::server::MeshBackend;

pub struct Client {
    to_server: DuplexStream,
    from_server: BufReader<DuplexStream>,
    next_id: i64,
    /// Notifications the server sent while a response was awaited.
    notifications: Vec<Value>,
}

impl Client {
    pub fn start() -> Client {
        let (client_out, server_in) = tokio::io::duplex(1 << 16);
        let (server_out, client_in) = tokio::io::duplex(1 << 16);
        let (service, socket) = LspService::new(MeshBackend::new);
        tokio::spawn(Server::new(server_in, server_out, socket).serve(service));
        Client {
            to_server: client_out,
            from_server: BufReader::new(client_in),
            next_id: 0,
            notifications: Vec::new(),
        }
    }

    pub async fn send(&mut self, message: Value) {
        let body = message.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        self.to_server.write_all(frame.as_bytes()).await.unwrap();
    }

    pub async fn receive(&mut self) -> Value {
        let mut length = 0;
        loop {
            let mut line = String::new();
            self.from_server.read_line(&mut line).await.unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = value.parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        self.from_server.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let message = self.receive().await;
            if message["id"] == id {
                return message["result"].clone();
            }
            self.notifications.push(message);
        }
    }

    pub async fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await;
    }

    /// The next notification of `method`, waiting for it if need be.
    pub async fn notification(&mut self, method: &str) -> Value {
        if let Some(index) = self
            .notifications
            .iter()
            .position(|n| n["method"] == method)
        {
            return self.notifications.remove(index)["params"].clone();
        }
        loop {
            let message = self.receive().await;
            if message["method"] == method {
                return message["params"].clone();
            }
            self.notifications.push(message);
        }
    }

    /// The diagnostics published on `uri` at `version`, waiting for them if
    /// need be. Other notifications are kept for later calls.
    pub async fn diagnostics(&mut self, uri: &str, version: i64) -> Vec<Value> {
        let matches = |n: &Value| {
            n["method"] == "textDocument/publishDiagnostics"
                && n["params"]["uri"] == uri
                && n["params"]["version"] == version
        };
        if let Some(index) = self.notifications.iter().position(matches) {
            let mut message = self.notifications.remove(index);
            return message["params"]["diagnostics"]
                .take()
                .as_array()
                .unwrap()
                .clone();
        }
        loop {
            let mut message = self.receive().await;
            if matches(&message) {
                return message["params"]["diagnostics"]
                    .take()
                    .as_array()
                    .unwrap()
                    .clone();
            }
            self.notifications.push(message);
        }
    }
}
