//! `Plaintext<T>` end to end: content computed on, sealed and opened at its
//! run-time value; refused exits fail the build; the build report lists
//! every `declassify` and `@display` export, and `--check` notices a change.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/test_artifacts.rs"]
mod artifacts;

fn meshc(args: &[&str]) -> Output {
    Command::new(artifacts::meshc_bin())
        .args(args)
        .output()
        .expect("meshc runs")
}

fn project(root: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let project = root.join(name);
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("mesh.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    for (file, source) in files {
        fs::write(project.join(file), source).unwrap();
    }
    project
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\nstdout:\n{}\nstderr:\n{}",
        text(&output.stdout),
        text(&output.stderr)
    );
}

const PROGRAM: &str = r#"
struct Note do
  author :: String
  text :: Plaintext<String>
end

service Vault do
  fn init() -> Int do
    0
  end
  call Measure(text :: Plaintext<String>) :: Int do |count|
    let size = declassify(Plaintext.map(text, fn(value) -> String.length(value) end), "e2e: the length a service measured")
    (count + 1, size)
  end
end

fn shout(text :: String) -> String do
  String.to_upper(text)
end

fn text_of(bytes :: Bytes) -> String do
  case Bytes.to_utf8(bytes) do
    Ok(text) -> text
    Err(_) -> ""
  end
end

fn round_trip(body :: Plaintext<Bytes>) -> Plaintext<Bytes> ! CryptoError do
  let key = Crypto.aead_key(Secret.random(32) ?) ?
  let nonce = Crypto.random_bytes(12) ?
  let aad = Bytes.from_utf8("e2e")
  let sealed = Crypto.aead_seal_plaintext(key, nonce, aad, body) ?
  Crypto.aead_open_plaintext(key, nonce, aad, sealed)
end

fn main() do
  let body = Plaintext.from("hello, world")
  let loud = Plaintext.map(body, shout)
  let joined = Plaintext.map2(loud, Plaintext.from("!"), fn(a, b) -> a <> b end)
  println(declassify(joined, "e2e: the test prints what it computed"))
  let note = Note { author: "ada", text: body }
  let first = Plaintext.map(note.text, fn(value) -> String.slice(value, 0, 5) end)
  println(note.author <> ": " <> declassify(first, "e2e: the test prints a field"))
  let bytes = Plaintext.map(body, fn(value) -> Bytes.from_utf8(value) end)
  case round_trip(bytes) do
    Ok(opened) -> println(declassify(Plaintext.map(opened, text_of), "e2e: the test prints what it opened"))
    Err(_) -> println("the seal failed")
  end
  let vault = Vault.start()
  let size = Vault.measure(vault, body)
  println("measured #{size}")
end
"#;

#[test]
fn plaintext_computes_seals_and_opens_at_its_value() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project(temp.path(), "plaintext-run", &[("main.mpl", PROGRAM)]);
    let binary = temp.path().join("plaintext-run-bin");
    assert_success(
        &meshc(&[
            "build",
            dir.to_str().unwrap(),
            "--output",
            binary.to_str().unwrap(),
        ]),
        "build",
    );
    let run = Command::new(&binary).output().unwrap();
    assert_success(&run, "run");
    assert_eq!(
        text(&run.stdout),
        "HELLO, WORLD!\nada: hello\nhello, world\nmeasured 12\n"
    );
}

/// A crash inside `Plaintext.map` says it crashed, not what the content
/// made of the runtime's message (`List.get`'s names the index).
#[test]
fn a_panic_while_computing_on_plaintext_withholds_its_message() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project(
        temp.path(),
        "plaintext-panic",
        &[(
            "main.mpl",
            r#"
fn main() do
  let secret = Plaintext.from("twenty-seven characters!!!!")
  let crashed = Plaintext.map(secret, fn(text) -> List.get([1], String.length(text)) end)
  println(declassify(Plaintext.map(crashed, fn(value) -> Int.to_string(value) end), "e2e: never reached"))
end
"#,
        )],
    );
    let binary = temp.path().join("plaintext-panic-bin");
    assert_success(
        &meshc(&[
            "build",
            dir.to_str().unwrap(),
            "--output",
            binary.to_str().unwrap(),
        ]),
        "build",
    );
    let run = Command::new(&binary).output().unwrap();
    assert!(!run.status.success(), "the program should crash");
    let stderr = text(&run.stderr);
    assert!(!stderr.contains("27"), "the index leaked: {stderr}");
    assert!(stderr.contains("plaintext"), "{stderr}");
}

#[test]
fn a_refused_exit_fails_the_build_and_says_why() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project(
        temp.path(),
        "plaintext-leak",
        &[(
            "main.mpl",
            "fn main() do\n  let body = Plaintext.from(\"hello\")\n  println(\"body: #{body}\")\nend\n",
        )],
    );
    let build = meshc(&["build", dir.to_str().unwrap()]);
    assert!(!build.status.success(), "the leak compiled");
    let stderr = text(&build.stderr);
    assert!(stderr.contains("E0093"), "{stderr}");
    assert!(stderr.contains("holds plaintext"), "{stderr}");
    assert!(stderr.contains("declassify"), "{stderr}");
}

const REPORTED_MAIN: &str = r##"import Words

fn bucket(body :: Plaintext<String>) -> Int do
  declassify(Plaintext.map(body, fn(text) -> String.length(text) / 16 end), "padding bucket")
end

fn main() do
  let body = Plaintext.from("hello")
  println("#{bucket(body)} #{Words.preview(body)}")
end
"##;

const REPORTED_WORDS: &str = r#"pub fn preview(body :: Plaintext<String>) -> String do
  declassify(Plaintext.map(body, fn(text) -> String.slice(text, 0, 2) end), "notification preview the user allowed")
end
"#;

fn report_entries(report: &serde_json::Value) -> Vec<(String, u64, String, String)> {
    report["declassify"]
        .as_array()
        .expect("declassify sites")
        .iter()
        .map(|site| {
            (
                site["file"].as_str().unwrap().to_string(),
                site["line"].as_u64().unwrap(),
                site["function"].as_str().unwrap().to_string(),
                site["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn the_build_report_lists_every_site_and_check_catches_a_change() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project(
        temp.path(),
        "plaintext-report",
        &[("main.mpl", REPORTED_MAIN), ("words.mpl", REPORTED_WORDS)],
    );
    let dir_arg = dir.to_str().unwrap();
    let report_path = temp.path().join("plaintext-report.json");
    let binary = temp.path().join("plaintext-report-bin");
    let build = || {
        meshc(&[
            "build",
            dir_arg,
            "--output",
            binary.to_str().unwrap(),
            "--plaintext-report",
            report_path.to_str().unwrap(),
        ])
    };
    assert_success(&build(), "build");
    let first = fs::read_to_string(&report_path).unwrap();
    let report: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(report["format"], "mesh-plaintext-report/1");
    assert_eq!(report["package"], "plaintext-report");
    assert_eq!(
        report_entries(&report),
        [
            (
                "main.mpl".to_string(),
                4,
                "bucket".to_string(),
                "padding bucket".to_string()
            ),
            (
                "words.mpl".to_string(),
                2,
                "preview".to_string(),
                "notification preview the user allowed".to_string()
            ),
        ]
    );
    assert_eq!(report["display"], serde_json::json!([]));

    // The same source gives the same report, byte for byte.
    assert_success(&build(), "second build");
    assert_eq!(fs::read_to_string(&report_path).unwrap(), first);
    let printed = meshc(&["plaintext-report", dir_arg]);
    assert_success(&printed, "plaintext-report");
    assert_eq!(text(&printed.stdout), first);

    let check = || {
        meshc(&[
            "plaintext-report",
            dir_arg,
            "--check",
            report_path.to_str().unwrap(),
        ])
    };
    assert_success(&check(), "check against the report just written");

    // Moving a site is no change to review.
    fs::write(dir.join("words.mpl"), format!("\n\n{REPORTED_WORDS}")).unwrap();
    assert_success(&check(), "check after lines moved");

    // A new site is.
    fs::write(
        dir.join("words.mpl"),
        format!(
            "{REPORTED_WORDS}\npub fn whole(body :: Plaintext<String>) -> String do\n  declassify(body, \"debugging\")\nend\n"
        ),
    )
    .unwrap();
    let changed = check();
    assert!(!changed.status.success(), "an unreviewed declassify passed");
    let stderr = text(&changed.stderr);
    assert!(
        stderr.contains("+ declassify in `whole` (words.mpl): debugging"),
        "{stderr}"
    );
}

#[test]
fn display_exports_are_reported_and_carry_plaintext_to_the_host() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plaintext_library");
    let report = meshc(&["plaintext-report", fixture.to_str().unwrap()]);
    assert_success(&report, "plaintext-report");
    let report: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(
        report["display"],
        serde_json::json!([{
            "file": "main.mpl",
            "line": 16,
            "function": "shout",
            "symbol": "mesh_plaintext_fixture_shout"
        }])
    );

    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("libmesh_plaintext.dylib");
    let library = if cfg!(target_os = "macos") {
        library
    } else {
        library.with_extension("so")
    };
    assert_success(
        &meshc(&[
            "build",
            fixture.to_str().unwrap(),
            "--artifact",
            "cdylib",
            "--output",
            library.to_str().unwrap(),
        ]),
        "library build",
    );
    let host = temp.path().join("plaintext-host");
    let mut cc = Command::new("cc");
    cc.arg(fixture.join("host.c"))
        .arg("-I")
        .arg(temp.path())
        .arg("-L")
        .arg(temp.path())
        .arg("-lmesh_plaintext")
        .arg(format!("-Wl,-rpath,{}", temp.path().display()))
        .arg("-o")
        .arg(&host);
    if cfg!(target_os = "macos") {
        cc.args(["-framework", "Security", "-framework", "CoreFoundation"]);
    }
    assert_success(&cc.output().unwrap(), "C host link");
    let run = Command::new(&host).output().unwrap();
    assert_success(&run, "C host run");
    assert!(text(&run.stderr).contains("display export passed"));
}
