//! End-to-end integration tests for all Phase 10 developer tools.
//!
//! Verifies that the snowc binary's developer-facing subcommands work together:
//! - `snowc build --json` produces valid JSON diagnostics for type errors
//! - `snowc fmt` formats files, `snowc fmt --check` verifies formatting
//! - `snowc init` creates a compilable project
//! - `snowc repl --help` confirms REPL subcommand availability
//! - `snowc lsp --help` confirms LSP subcommand availability

use std::path::PathBuf;
use std::process::Command;

/// Locate the snowc binary built by cargo.
fn snowc_bin() -> PathBuf {
    // CARGO_BIN_EXE_snowc is set by cargo when running integration tests
    // for the snowc package.
    PathBuf::from(env!("CARGO_BIN_EXE_snowc"))
}

// ── Error messages (--json) ──────────────────────────────────────────

#[test]
fn test_build_json_output() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();

    // Write a Snow file with a type error (assigning string to Int annotation).
    std::fs::write(
        project.join("main.snow"),
        "let x :: Int = \"hello\"\n",
    )
    .unwrap();

    let output = Command::new(snowc_bin())
        .args(["build", "--json", project.to_str().unwrap()])
        .output()
        .expect("failed to run snowc build --json");

    assert!(
        !output.status.success(),
        "Expected build to fail on type error"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    // stderr contains concatenated JSON objects. Use a streaming deserializer
    // to extract the first one (the type error diagnostic).
    let mut stream =
        serde_json::Deserializer::from_str(&stderr).into_iter::<serde_json::Value>();
    let json = stream
        .next()
        .expect("no JSON object in stderr")
        .expect("first JSON object is not valid");

    // Verify required JSON fields.
    assert!(json.get("code").is_some(), "JSON missing 'code' field");
    assert!(
        json.get("severity").is_some(),
        "JSON missing 'severity' field"
    );
    assert!(
        json.get("message").is_some(),
        "JSON missing 'message' field"
    );
    assert!(json.get("spans").is_some(), "JSON missing 'spans' field");

    // Verify the error code starts with E (type error).
    let code = json["code"].as_str().unwrap();
    assert!(
        code.starts_with('E'),
        "Expected error code starting with E, got: {}",
        code
    );

    // Verify spans array is non-empty.
    let spans = json["spans"].as_array().unwrap();
    assert!(!spans.is_empty(), "Expected at least one span");

    // Verify no ANSI escape codes in output.
    assert!(
        !stderr.contains("\x1b["),
        "JSON mode should not contain ANSI escape codes"
    );
}

// ── Formatter ────────────────────────────────────────────────────────

#[test]
fn test_fmt_formats_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.snow");

    // Write an unformatted Snow file (no spaces around operator, no indentation).
    std::fs::write(&file, "fn add(a,b) do\na+b\nend").unwrap();

    let output = Command::new(snowc_bin())
        .args(["fmt", file.to_str().unwrap()])
        .output()
        .expect("failed to run snowc fmt");

    assert!(
        output.status.success(),
        "snowc fmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let contents = std::fs::read_to_string(&file).unwrap();

    // Verify the file was reformatted (canonical 2-space indent, spaces around ops).
    assert_eq!(contents, "fn add(a, b) do\n  a + b\nend\n");
}

#[test]
fn test_fmt_check_formatted() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("good.snow");

    // Write an already-formatted file.
    std::fs::write(&file, "fn add(a, b) do\n  a + b\nend\n").unwrap();

    let output = Command::new(snowc_bin())
        .args(["fmt", "--check", file.to_str().unwrap()])
        .output()
        .expect("failed to run snowc fmt --check");

    assert!(
        output.status.success(),
        "Expected exit 0 for already-formatted file, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_fmt_check_unformatted() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.snow");

    // Write an unformatted file.
    std::fs::write(&file, "fn bad(a,b) do\na+b\nend").unwrap();

    let output = Command::new(snowc_bin())
        .args(["fmt", "--check", file.to_str().unwrap()])
        .output()
        .expect("failed to run snowc fmt --check");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected exit 1 for unformatted file"
    );

    // File should NOT be modified in check mode.
    let contents = std::fs::read_to_string(&file).unwrap();
    assert_eq!(contents, "fn bad(a,b) do\na+b\nend");
}

#[test]
fn test_fmt_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("idem.snow");

    // Write an unformatted file.
    std::fs::write(&file, "fn foo(x) do\nlet y = x\ny\nend").unwrap();

    // Format once.
    let output1 = Command::new(snowc_bin())
        .args(["fmt", file.to_str().unwrap()])
        .output()
        .expect("failed to run snowc fmt (first pass)");
    assert!(output1.status.success(), "First format pass failed");

    let after_first = std::fs::read_to_string(&file).unwrap();

    // Format again.
    let output2 = Command::new(snowc_bin())
        .args(["fmt", file.to_str().unwrap()])
        .output()
        .expect("failed to run snowc fmt (second pass)");
    assert!(output2.status.success(), "Second format pass failed");

    let after_second = std::fs::read_to_string(&file).unwrap();

    // Both passes should produce identical output.
    assert_eq!(
        after_first, after_second,
        "Formatting is not idempotent!\nFirst pass:\n{}\nSecond pass:\n{}",
        after_first, after_second
    );

    // Additionally verify --check agrees the file is formatted.
    let check = Command::new(snowc_bin())
        .args(["fmt", "--check", file.to_str().unwrap()])
        .output()
        .expect("failed to run snowc fmt --check after formatting");
    assert!(
        check.status.success(),
        "fmt --check disagrees after formatting: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

// ── Package manager ──────────────────────────────────────────────────

#[test]
fn test_init_creates_project() {
    let dir = tempfile::tempdir().unwrap();

    let output = Command::new(snowc_bin())
        .args(["init", "test-project"])
        .current_dir(dir.path())
        .output()
        .expect("failed to run snowc init");

    assert!(
        output.status.success(),
        "snowc init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify snow.toml exists.
    let toml_path = dir.path().join("test-project").join("snow.toml");
    assert!(
        toml_path.exists(),
        "snow.toml not created at {}",
        toml_path.display()
    );

    // Verify main.snow exists.
    let main_path = dir.path().join("test-project").join("main.snow");
    assert!(
        main_path.exists(),
        "main.snow not created at {}",
        main_path.display()
    );

    // Verify snow.toml contains the project name.
    let toml_contents = std::fs::read_to_string(&toml_path).unwrap();
    assert!(
        toml_contents.contains("test-project"),
        "snow.toml does not contain project name"
    );
}

// ── REPL ─────────────────────────────────────────────────────────────

#[test]
fn test_repl_help() {
    let output = Command::new(snowc_bin())
        .args(["repl", "--help"])
        .output()
        .expect("failed to run snowc repl --help");

    assert!(
        output.status.success(),
        "snowc repl --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Help text should mention REPL or interactive.
    let mentions_repl = stdout.to_lowercase().contains("repl")
        || stdout.to_lowercase().contains("interactive");
    assert!(
        mentions_repl,
        "repl --help should mention REPL or interactive, got:\n{}",
        stdout
    );
}

// ── LSP ──────────────────────────────────────────────────────────────

#[test]
fn test_lsp_subcommand_exists() {
    let output = Command::new(snowc_bin())
        .args(["lsp", "--help"])
        .output()
        .expect("failed to run snowc lsp --help");

    assert!(
        output.status.success(),
        "snowc lsp --help should exit 0, got: {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}
