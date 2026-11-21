//! The language server over JSON-RPC, as an editor drives it: every request
//! and notification it answers, through an in-memory connection.

mod common;

use common::Client;
use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use tower_lsp::lsp_types::Url;

const URI: &str = "file:///project/main.mpl";

fn at(line: u32, character: u32) -> Value {
    json!({"textDocument": {"uri": URI}, "position": {"line": line, "character": character}})
}

struct Project {
    _dir: tempfile::TempDir,
    main: Url,
    util: Url,
}

impl Project {
    fn new(main: &str, util: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(
            root.join("mesh.toml"),
            "[package]\nname = \"project\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("main.mpl"), main).unwrap();
        fs::write(root.join("util.mpl"), util).unwrap();
        Self {
            _dir: dir,
            main: Url::from_file_path(root.join("main.mpl")).unwrap(),
            util: Url::from_file_path(root.join("util.mpl")).unwrap(),
        }
    }
}

async fn initialized_client() -> Client {
    let mut client = Client::start();
    client
        .request("initialize", json!({"capabilities": {}}))
        .await;
    client.notify("initialized", json!({})).await;
    client
}

async fn open(client: &mut Client, uri: &str, version: i32, text: &str) {
    client
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "mesh", "version": version, "text": text}}),
        )
        .await;
}

async fn change(client: &mut Client, uri: &str, version: i32, text: &str) {
    client
        .notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]}),
        )
        .await;
}

async fn published(client: &mut Client) -> Value {
    tokio::time::timeout(
        Duration::from_secs(10),
        client.notification("textDocument/publishDiagnostics"),
    )
    .await
    .expect("the server published diagnostics")
}

async fn published_pair(client: &mut Client, project: &Project) -> (Value, Value) {
    let first = published(client).await;
    let second = published(client).await;
    let (main, util) = if first["uri"] == project.main.as_str() {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(main["uri"], project.main.as_str());
    assert_eq!(util["uri"], project.util.as_str());
    (main, util)
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
    assert_eq!(
        capabilities["codeActionProvider"]["codeActionKinds"],
        json!(["quickfix"])
    );
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

#[tokio::test]
async fn editing_an_open_imported_file_refreshes_and_clears_the_callers_diagnostics() {
    let main = "from Util import size\nfn main() do\n  let n :: Int = size()\nend\n";
    let good_util = "pub fn size() -> Int do\n  1\nend\n";
    let bad_util = "pub fn size() -> String do\n  \"one\"\nend\n";
    let project = Project::new(main, good_util);
    let mut client = initialized_client().await;

    open(&mut client, project.main.as_str(), 9, main).await;
    assert_eq!(published(&mut client).await["diagnostics"], json!([]));
    open(&mut client, project.util.as_str(), 1, good_util).await;
    assert_eq!(published(&mut client).await["diagnostics"], json!([]));

    change(&mut client, project.util.as_str(), 2, bad_util).await;
    let (main, util) = published_pair(&mut client, &project).await;
    assert_eq!(main["version"], 9);
    assert_eq!(main["diagnostics"][0]["code"], "E0001");
    assert_eq!(main["diagnostics"][0]["range"]["start"]["line"], 2);
    assert_eq!(util["version"], 2);
    assert_eq!(util["diagnostics"], json!([]));

    change(&mut client, project.util.as_str(), 3, good_util).await;
    let (main, util) = published_pair(&mut client, &project).await;
    assert_eq!(main["version"], 9);
    assert_eq!(main["diagnostics"], json!([]));
    assert_eq!(util["version"], 3);
    assert_eq!(util["diagnostics"], json!([]));
}

#[tokio::test]
async fn duplicate_exports_are_reported_on_the_unopened_sibling_and_cleared() {
    let main = "from Util import size\npub fn size() -> Int do\n  2\nend\nfn main() do\nend\n";
    let fixed = main.replace("pub fn size()", "pub fn length()");
    let util = "pub fn size() -> Int do\n  1\nend\n";
    let project = Project::new(main, util);
    let mut client = initialized_client().await;

    open(&mut client, project.main.as_str(), 1, main).await;
    let (main, util) = published_pair(&mut client, &project).await;
    assert_eq!(main["diagnostics"], json!([]));
    assert!(util.get("version").is_none(), "{util}");
    let d = &util["diagnostics"][0];
    assert_eq!(d["code"], "E0103", "{util}");
    assert_eq!(
        d["range"],
        json!({"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 11}})
    );
    assert_eq!(
        d["relatedInformation"][0]["location"]["uri"],
        project.main.as_str()
    );

    change(&mut client, project.main.as_str(), 2, &fixed).await;
    let (main, util) = published_pair(&mut client, &project).await;
    assert_eq!(main["version"], 2);
    assert_eq!(main["diagnostics"], json!([]));
    assert!(util.get("version").is_none(), "{util}");
    assert_eq!(util["diagnostics"], json!([]));
}

#[tokio::test]
async fn closing_a_sibling_uses_the_disk_source_until_the_last_buffer_closes() {
    let main = "from Util import size\nfn main() do\n  size()\nend\n";
    let disk = "pub fn size() -> Int do\n  \"one\"\nend\n";
    let overlay = "pub fn size() -> Int do\n  1\nend\n";
    let project = Project::new(main, disk);
    let mut client = initialized_client().await;

    open(&mut client, project.main.as_str(), 1, main).await;
    let (_, util) = published_pair(&mut client, &project).await;
    assert!(util.get("version").is_none(), "{util}");
    assert_eq!(util["diagnostics"][0]["code"], "E0001");

    open(&mut client, project.util.as_str(), 2, overlay).await;
    let util = published(&mut client).await;
    assert_eq!(util["uri"], project.util.as_str());
    assert_eq!(util["version"], 2);
    assert_eq!(util["diagnostics"], json!([]));

    client
        .notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": project.util}}),
        )
        .await;
    let (_, util) = published_pair(&mut client, &project).await;
    assert!(util.get("version").is_none(), "{util}");
    assert_eq!(util["diagnostics"][0]["code"], "E0001");

    client
        .notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": project.main}}),
        )
        .await;
    let (main, util) = published_pair(&mut client, &project).await;
    for clear in [main, util] {
        assert!(clear.get("version").is_none(), "{clear}");
        assert_eq!(clear["diagnostics"], json!([]));
    }
}

#[tokio::test]
async fn a_spelling_suggestion_is_a_nonpreferred_utf16_quick_fix() {
    let mut client = initialized_client().await;
    let source = "fn main() do\n  let count = 1\n  (\"é😀\", cuont)\nend\n";
    open(&mut client, URI, 1, source).await;
    let published_diagnostics = published(&mut client).await;
    let diagnostic = &published_diagnostics["diagnostics"][0];
    assert_eq!(diagnostic["code"], "E0004", "{published_diagnostics}");
    let range = json!({"start": {"line": 2, "character": 10}, "end": {"line": 2, "character": 15}});
    assert_eq!(diagnostic["range"], range);

    let request = |range: &Value, diagnostics: Value| {
        json!({
            "textDocument": {"uri": URI},
            "range": range,
            "context": {"diagnostics": diagnostics, "only": ["quickfix"]}
        })
    };
    let actions = client
        .request("textDocument/codeAction", request(&range, json!([])))
        .await;
    assert_eq!(actions.as_array().unwrap().len(), 1, "{actions}");
    let action = &actions[0];
    assert_eq!(action["kind"], "quickfix");
    assert_eq!(action["title"], "did you mean `count`?");
    assert_eq!(action["isPreferred"], false);
    assert_eq!(action["diagnostics"], json!([diagnostic]));
    assert_eq!(
        action["edit"]["changes"][URI],
        json!([{"range": range, "newText": "count"}])
    );

    let elsewhere =
        json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}});
    assert!(client
        .request("textDocument/codeAction", request(&elsewhere, json!([])))
        .await
        .is_null());
    assert_eq!(
        client
            .request(
                "textDocument/codeAction",
                request(&elsewhere, json!([diagnostic]))
            )
            .await,
        actions
    );

    change(&mut client, URI, 2, &source.replace("cuont", "unknownzzzz")).await;
    let no_edit = published(&mut client).await;
    let diagnostic = &no_edit["diagnostics"][0];
    assert_eq!(diagnostic["code"], "E0004", "{no_edit}");
    assert!(client
        .request(
            "textDocument/codeAction",
            request(&diagnostic["range"], json!([diagnostic]))
        )
        .await
        .is_null());
}

#[tokio::test]
async fn a_bool_comparison_is_a_preferred_quick_fix_to_the_value() {
    let mut client = initialized_client().await;
    let example = mesh_diagnostics::Code::BoolComparison
        .info()
        .example
        .unwrap();
    open(&mut client, URI, 1, example.bad).await;
    let published_diagnostics = published(&mut client).await;
    let diagnostic = &published_diagnostics["diagnostics"][0];
    assert_eq!(diagnostic["code"], "L0004", "{published_diagnostics}");
    assert_eq!(diagnostic["severity"], 2, "a warning");
    let range = json!({"start": {"line": 2, "character": 5}, "end": {"line": 2, "character": 18}});
    assert_eq!(diagnostic["range"], range);

    let actions = client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": {"uri": URI},
                "range": range,
                "context": {"diagnostics": [], "only": ["quickfix"]}
            }),
        )
        .await;
    assert_eq!(actions.as_array().unwrap().len(), 1, "{actions}");
    let action = &actions[0];
    assert_eq!(action["title"], "Replace the comparison with `ready`.");
    assert_eq!(action["isPreferred"], true);
    assert_eq!(
        action["edit"]["changes"][URI],
        json!([{"range": range, "newText": "ready"}])
    );
    assert_eq!(
        example.bad.replacen("ready == true", "ready", 1),
        example.fixed
    );
}
