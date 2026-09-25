//! The language server over JSON-RPC, as an editor drives it: every request
//! and notification it answers, through an in-memory connection.

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tower_lsp::{LspService, Server};

use mesh_lsp::server::MeshBackend;

struct Client {
    to_server: DuplexStream,
    from_server: BufReader<DuplexStream>,
    next_id: i64,
    /// Notifications the server sent while a response was awaited.
    notifications: Vec<Value>,
}

impl Client {
    fn start() -> Client {
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

    async fn send(&mut self, message: Value) {
        let body = message.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        self.to_server.write_all(frame.as_bytes()).await.unwrap();
    }

    async fn receive(&mut self) -> Value {
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

    async fn request(&mut self, method: &str, params: Value) -> Value {
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

    async fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await;
    }

    /// The next notification of `method`, waiting for it if need be.
    async fn notification(&mut self, method: &str) -> Value {
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
}

const URI: &str = "file:///project/main.mpl";

fn at(line: u32, character: u32) -> Value {
    json!({"textDocument": {"uri": URI}, "position": {"line": line, "character": character}})
}

#[tokio::test]
async fn the_server_answers_what_an_editor_asks() {
    let mut client = Client::start();
    let init = client
        .request("initialize", json!({"capabilities": {}}))
        .await;
    let capabilities = &init["capabilities"];
    for capability in [
        "hoverProvider",
        "definitionProvider",
        "documentSymbolProvider",
        "documentFormattingProvider",
        "completionProvider",
        "signatureHelpProvider",
    ] {
        assert!(!capabilities[capability].is_null(), "{capability}: {init}");
    }
    client.notify("initialized", json!({})).await;
    let log = client.notification("window/logMessage").await;
    assert_eq!(log["message"], "Mesh LSP server initialized");

    // Asked about a document it has not seen, the server has nothing to say.
    for method in [
        "textDocument/hover",
        "textDocument/definition",
        "textDocument/completion",
        "textDocument/signatureHelp",
    ] {
        assert!(client.request(method, at(0, 0)).await.is_null(), "{method}");
    }
    let document = json!({"textDocument": {"uri": URI}});
    assert!(client
        .request("textDocument/documentSymbol", document.clone())
        .await
        .is_null());
    let formatting =
        json!({"textDocument": {"uri": URI}, "options": {"tabSize": 2, "insertSpaces": true}});
    assert!(client
        .request("textDocument/formatting", formatting.clone())
        .await
        .is_null());

    let source = "fn add(a :: Int, b :: Int) -> Int do\n  a + b\nend\n\nfn main() do\n  let total = add(1, 2)\n  println(\"#{total}\")\n  let bad :: String = 5\nend\n";
    client
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": URI, "languageId": "mesh", "version": 1, "text": source}}),
        )
        .await;
    let diagnostics = client.notification("textDocument/publishDiagnostics").await;
    assert_eq!(diagnostics["uri"], URI);
    let reported = diagnostics["diagnostics"].as_array().unwrap();
    assert_eq!(reported.len(), 1, "{diagnostics}");
    assert_eq!(reported[0]["range"]["start"]["line"], 7);

    // `total` on line 6 is an Int, defined on line 5.
    let hover = client.request("textDocument/hover", at(6, 13)).await;
    assert!(
        hover["contents"]["value"].as_str().unwrap().contains("Int"),
        "{hover}"
    );
    let definition = client.request("textDocument/definition", at(6, 13)).await;
    assert_eq!(definition["uri"], URI);
    assert_eq!(definition["range"]["start"]["line"], 5);
    assert!(client
        .request("textDocument/definition", at(3, 0))
        .await
        .is_null());

    let symbols = client
        .request("textDocument/documentSymbol", document.clone())
        .await;
    let names: Vec<&str> = symbols
        .as_array()
        .unwrap()
        .iter()
        .map(|symbol| symbol["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["add", "main"]);

    let completions = client.request("textDocument/completion", at(6, 2)).await;
    assert!(
        completions
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "total"),
        "{completions}"
    );
    // Inside `add(1, 2)`, at the second argument.
    let signature = client
        .request("textDocument/signatureHelp", at(5, 21))
        .await;
    assert_eq!(signature["activeParameter"], 1, "{signature}");

    // The document is already formatted: nothing to change.
    assert!(client
        .request("textDocument/formatting", formatting.clone())
        .await
        .is_null());

    let unformatted = "fn main() do\nprintln(\"hi\")\nend\n";
    client
        .notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": URI, "version": 2}, "contentChanges": [{"text": unformatted}]}),
        )
        .await;
    let diagnostics = client.notification("textDocument/publishDiagnostics").await;
    assert!(diagnostics["diagnostics"].as_array().unwrap().is_empty());
    let edits = client.request("textDocument/formatting", formatting).await;
    assert_eq!(
        edits[0]["newText"],
        "fn main() do\n  println(\"hi\")\nend\n"
    );
    // The edit replaces the whole text: it ends where the text ends.
    assert_eq!(edits[0]["range"]["end"], json!({"line": 3, "character": 0}));

    client.notify("textDocument/didClose", document).await;
    let diagnostics = client.notification("textDocument/publishDiagnostics").await;
    assert!(diagnostics["diagnostics"].as_array().unwrap().is_empty());
    assert!(client
        .request("textDocument/hover", at(1, 2))
        .await
        .is_null());

    assert!(client.request("shutdown", Value::Null).await.is_null());
    client.notify("exit", Value::Null).await;
}
