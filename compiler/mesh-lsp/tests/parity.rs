//! For each catalog example, the editor reports the codes `meshc check` and
//! `meshc lint` report. The editor shows a repair and a docs link for each diagnostic,
//! then clears it when the source is fixed. Examples run in standalone
//! buffers and projects, or only in a project if they require a manifest.

mod common;

use std::path::Path;

use common::Client;
use mesh_diagnostics::{Code, Family, Lookup};
use mesh_pkg::frontend::{self, CheckOptions, Target};
use serde_json::{json, Value};
use tower_lsp::lsp_types::Url;

/// The codes of `check`'s diagnostics and of the lint findings in each file
/// that parsed, sorted: a multiset.
fn frontend_codes(target: Target<'_>) -> Vec<&'static str> {
    let checked = frontend::check(target, &CheckOptions::cli());
    let lints = checked
        .lintable()
        .flat_map(|(parse, file)| mesh_lint::lint(parse, file));
    let mut codes: Vec<_> = checked
        .diagnostics
        .iter()
        .map(|d| d.code().id())
        .chain(lints.map(|lint| lint.code().id()))
        .collect();
    codes.sort();
    codes
}

/// The codes of the published diagnostics, sorted, once each is checked to
/// carry its repair and its docs page.
fn published_codes(diagnostics: &[Value]) -> Vec<&'static str> {
    let mut codes: Vec<_> = diagnostics
        .iter()
        .map(|d| {
            let id = d["code"].as_str().unwrap_or_else(|| panic!("no code: {d}"));
            let Lookup::Found(code) = Code::lookup(id) else {
                panic!("`{id}` is not a catalog code: {d}");
            };
            assert_eq!(d["codeDescription"]["href"], code.docs_url(), "{d}");
            assert!(
                d["message"].as_str().unwrap().contains("\nhelp: "),
                "no repair: {d}"
            );
            code.id()
        })
        .collect();
    codes.sort();
    codes
}

/// Opens `bad` at `uri`, then changes it to `fixed`: what the server
/// published for each.
async fn publish(client: &mut Client, uri: &str, bad: &str, fixed: &str) -> [Vec<Value>; 2] {
    client
        .notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "mesh", "version": 1, "text": bad}}),
        )
        .await;
    let for_bad = client.diagnostics(uri, 1).await;
    client
        .notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": fixed}]}),
        )
        .await;
    let for_fixed = client.diagnostics(uri, 2).await;
    client
        .notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        )
        .await;
    [for_bad, for_fixed]
}

#[tokio::test]
async fn the_editor_publishes_what_the_compiler_reports() {
    let mut client = Client::start();
    client
        .request("initialize", json!({"capabilities": {}}))
        .await;
    client.notify("initialized", json!({})).await;

    let projects = tempfile::tempdir().unwrap();
    let projects = projects.path().canonicalize().unwrap();
    let mut failures = Vec::new();
    // No code that only codegen reports has an example yet.
    let examples = Code::all()
        .filter(|code| {
            matches!(
                code.family(),
                Family::Syntax | Family::Semantic | Family::Lint
            )
        })
        .filter_map(|code| Some((code, code.info().example?)));
    for (index, (code, example)) in examples.enumerate() {
        let native = code == Code::InvalidNativeDeclaration;
        if !native {
            // A buffer that is no file, analyzed on its own.
            let uri = format!("untitled:Untitled-{index}");
            let lone = Path::new("main.mpl");
            let [bad, fixed] = publish(&mut client, &uri, example.bad, example.fixed).await;
            let expected = frontend_codes(Target::Source {
                path: lone,
                text: example.bad,
            });
            compare(
                code,
                "a lone buffer",
                &bad,
                &fixed,
                &expected,
                &mut failures,
            );
            let fixed_expected = frontend_codes(Target::Source {
                path: lone,
                text: example.fixed,
            });
            assert!(
                fixed_expected.is_empty(),
                "{}: {fixed_expected:?}",
                code.id()
            );
        }

        let root = projects.join(code.id());
        std::fs::create_dir_all(&root).unwrap();
        let mut manifest = "[package]\nname = \"example\"\nversion = \"0.1.0\"\n".to_string();
        let main = if native {
            manifest.push_str("\n[native]\nabi = 1\nbindings = [\"binding.mpl\"]\n");
            std::fs::write(root.join("main.mpl"), "fn main() do\nend\n").unwrap();
            root.join("binding.mpl")
        } else {
            root.join("main.mpl")
        };
        std::fs::write(root.join("mesh.toml"), manifest).unwrap();
        std::fs::write(&main, example.bad).unwrap();
        let expected = frontend_codes(Target::Project { root: &root });
        let uri = Url::from_file_path(&main).unwrap().to_string();
        let [bad, fixed] = publish(&mut client, &uri, example.bad, example.fixed).await;
        compare(code, "a project", &bad, &fixed, &expected, &mut failures);
        std::fs::write(&main, example.fixed).unwrap();
        let fixed_expected = frontend_codes(Target::Project { root: &root });
        assert!(
            fixed_expected.is_empty(),
            "{}: {fixed_expected:?}",
            code.id()
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    assert!(client.request("shutdown", Value::Null).await.is_null());
    client.notify("exit", Value::Null).await;
}

fn compare(
    code: Code,
    placement: &str,
    bad: &[Value],
    fixed: &[Value],
    expected: &[&str],
    failures: &mut Vec<String>,
) {
    let published = published_codes(bad);
    if published != expected {
        failures.push(format!(
            "{} in {placement}: the editor got {published:?}, meshc {expected:?}",
            code.id()
        ));
    }
    if !published.contains(&code.id()) {
        failures.push(format!(
            "{} in {placement}: the bad example got {published:?}",
            code.id()
        ));
    }
    if !fixed.is_empty() {
        failures.push(format!(
            "{} in {placement}: the fixed example got {:?}",
            code.id(),
            published_codes(fixed)
        ));
    }
}
