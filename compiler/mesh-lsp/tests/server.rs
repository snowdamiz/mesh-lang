//! The language server over JSON-RPC, as an editor drives it: every request
//! and notification it answers, through an in-memory connection.

mod common;

use common::Client;
use serde_json::{json, Value};

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

#[tokio::test]
async fn pipe_placeholder_diagnostic_has_its_code_and_utf16_range() {
    let mut client = Client::start();
    client
        .request("initialize", json!({"capabilities": {}}))
        .await;
    client.notify("initialized", json!({})).await;

    let source = r#"fn triple(a :: String, b :: String, c :: String) do
  a <> b <> c
end
fn main() do
  println("é😀" |> triple("a", _, "c"))
  triple("a", "b", "c", "d")
end
"#;
    client
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": URI, "languageId": "mesh", "version": 1, "text": source}}),
        )
        .await;
    let published = client.notification("textDocument/publishDiagnostics").await;
    assert_eq!(published["version"], 1, "{published}");
    let diagnostics = published["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2, "{published}");
    let pipe = &diagnostics[0];
    assert_eq!(pipe["code"], "E0003");
    assert_eq!(
        pipe["codeDescription"]["href"],
        "https://meshlang.dev/docs/errors/#e0003"
    );
    assert_eq!(pipe["severity"], 1);
    assert_eq!(
        pipe["range"],
        json!({"start": {"line": 4, "character": 19}, "end": {"line": 4, "character": 38}})
    );
    let message = pipe["message"].as_str().unwrap();
    assert!(message.contains("Use `|2>` and remove `_`"), "{message}");
    let ordinary = &diagnostics[1];
    assert_eq!(ordinary["code"], "E0003");
    assert_eq!(
        ordinary["message"],
        "arity mismatch: expected 3 arguments, found 4\nhelp: 1 extra argument(s)"
    );

    let repaired = source
        .replace("|> triple(\"a\", _, \"c\")", "|2> triple(\"a\", \"c\")")
        .replace(", \"d\"", "");
    client
        .notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": URI, "version": 2}, "contentChanges": [{"text": repaired}]}),
        )
        .await;
    let published = client.notification("textDocument/publishDiagnostics").await;
    assert_eq!(published["version"], 2, "{published}");
    assert_eq!(published["diagnostics"], json!([]));
}

#[tokio::test]
async fn non_exhaustive_match_diagnostic_explains_the_wildcard_default() {
    let mut client = Client::start();
    client
        .request("initialize", json!({"capabilities": {}}))
        .await;
    client.notify("initialized", json!({})).await;

    let source = "fn main() do\n  case Some(1) do\n    Some(n) -> n\n  end\nend\n";
    client
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": URI, "languageId": "mesh", "version": 1, "text": source}}),
        )
        .await;
    let published = client.notification("textDocument/publishDiagnostics").await;
    let diagnostics = published["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{published}");
    assert_eq!(diagnostics[0]["code"], "E0012");
    let message = diagnostics[0]["message"].as_str().unwrap();
    assert!(message.contains("add an arm for `None`"), "{message}");
    assert!(message.contains("also covers future variants"), "{message}");
}
