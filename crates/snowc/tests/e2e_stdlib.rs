//! End-to-end integration tests for Snow standard library functions (Phase 8).
//!
//! Tests string operations, module-qualified access (String.length),
//! from/import resolution, IO operations, and HTTP server/client compilation.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Helper: compile a Snow source file and run the resulting binary, returning stdout.
fn compile_and_run(source: &str) -> String {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, source).expect("failed to write main.snow");

    let snowc = find_snowc();
    let output = Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke snowc");

    assert!(
        output.status.success(),
        "snowc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let binary = project_dir.join("project");
    let run_output = Command::new(&binary)
        .output()
        .unwrap_or_else(|e| panic!("failed to run binary at {}: {}", binary.display(), e));

    assert!(
        run_output.status.success(),
        "binary execution failed with exit code {:?}:\nstdout: {}\nstderr: {}",
        run_output.status.code(),
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );

    String::from_utf8_lossy(&run_output.stdout).to_string()
}

/// Find the snowc binary in the target directory.
fn find_snowc() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot find current exe")
        .parent()
        .expect("cannot find parent dir")
        .to_path_buf();

    if path.file_name().map_or(false, |n| n == "deps") {
        path = path.parent().unwrap().to_path_buf();
    }

    let snowc = path.join("snowc");
    assert!(
        snowc.exists(),
        "snowc binary not found at {}. Run `cargo build -p snowc` first.",
        snowc.display()
    );
    snowc
}

/// Helper: compile a Snow source file without running it. Returns compilation output.
fn compile_only(source: &str) -> Output {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, source).expect("failed to write main.snow");

    let snowc = find_snowc();
    Command::new(&snowc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke snowc")
}

/// Read a test fixture from the tests/e2e/ directory.
fn read_fixture(name: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("e2e")
        .join(name);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", fixture_path.display(), e))
}

// ── String Operation E2E Tests ──────────────────────────────────────────

#[test]
fn e2e_string_length() {
    let source = read_fixture("stdlib_string_length.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "5\n");
}

#[test]
fn e2e_string_contains() {
    let source = read_fixture("stdlib_string_contains.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "true\nfalse\n");
}

#[test]
fn e2e_string_trim() {
    let source = read_fixture("stdlib_string_trim.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "hello\n");
}

#[test]
fn e2e_string_case_conversion() {
    let source = read_fixture("stdlib_string_case.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "HELLO\nworld\n");
}

#[test]
fn e2e_string_replace() {
    let source = read_fixture("stdlib_string_replace.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "hello snow\n");
}

// ── Module Resolution E2E Tests ─────────────────────────────────────────

#[test]
fn e2e_module_qualified_access() {
    let source = read_fixture("stdlib_module_qualified.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "4\n");
}

#[test]
fn e2e_from_import_resolution() {
    let source = read_fixture("stdlib_from_import.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "4\n");
}

// ── File I/O E2E Tests ──────────────────────────────────────────────────

#[test]
fn e2e_file_write_and_read() {
    let source = read_fixture("stdlib_file_write_read.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "Hello, Snow!\n");
}

#[test]
fn e2e_file_exists() {
    let source = read_fixture("stdlib_file_exists.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "false\ntrue\n\n");
}

#[test]
fn e2e_file_read_process_write() {
    let source = read_fixture("stdlib_file_process.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "HELLO WORLD\n");
}

#[test]
fn e2e_file_error_handling() {
    let source = read_fixture("stdlib_file_error.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "error\n");
}

// ── IO E2E Tests ────────────────────────────────────────────────────────

#[test]
fn e2e_io_eprintln_does_not_crash() {
    let source = read_fixture("stdlib_io_eprintln.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "done\n");
}

// ── Collection E2E Tests (Phase 8 Plan 02) ────────────────────────────

#[test]
fn e2e_list_basic() {
    let source = read_fixture("stdlib_list_basic.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "3\n1\n");
}

#[test]
fn e2e_map_basic() {
    let source = read_fixture("stdlib_map_basic.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "10\n2\n");
}

#[test]
fn e2e_set_basic() {
    let source = read_fixture("stdlib_set_basic.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "2\n");
}

#[test]
fn e2e_range_basic() {
    let source = read_fixture("stdlib_range_basic.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "3\n3\n1\n");
}

#[test]
fn e2e_queue_basic() {
    let source = read_fixture("stdlib_queue_basic.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "2\n10\n");
}

// ── JSON E2E Tests (Phase 8 Plan 04) ──────────────────────────────────

#[test]
fn e2e_json_encode_int() {
    let source = read_fixture("stdlib_json_encode_int.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "42\n");
}

#[test]
fn e2e_json_encode_string() {
    let source = read_fixture("stdlib_json_encode_string.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "\"hello\"\n");
}

#[test]
fn e2e_json_encode_bool() {
    let source = read_fixture("stdlib_json_encode_bool.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "true\nfalse\n");
}

#[test]
fn e2e_json_encode_map() {
    // Tests multiple JSON encode functions together (encode_int, encode_string, encode_bool)
    let source = read_fixture("stdlib_json_encode_map.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "100\n\"test\"\ntrue\n");
}

#[test]
fn e2e_json_parse_roundtrip() {
    let source = read_fixture("stdlib_json_parse_roundtrip.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "99\n");
}

// ── HTTP E2E Tests (Phase 8 Plan 05) ──────────────────────────────────

#[test]
fn e2e_http_server_compiles() {
    // Verify a server program compiles (cannot run because it blocks on serve).
    let source = read_fixture("stdlib_http_response.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "compiled\n");
}

#[test]
fn e2e_http_client_compiles_and_runs() {
    // Verify an HTTP client program compiles and runs.
    // This makes a real HTTP request to example.com.
    let source = read_fixture("stdlib_http_client.snow");
    let output = compile_and_run(&source);
    // Should print "ok" (successful GET to example.com) or "error" (no network).
    assert!(
        output == "ok\n" || output == "error\n",
        "unexpected output: {}",
        output
    );
}

#[test]
fn e2e_http_full_server_compile_only() {
    // Verify a full server program with handler and serve compiles.
    let source = r#"
fn handler(request) do
  let m = Request.method(request)
  HTTP.response(200, m)
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/", handler)
  HTTP.serve(r, 0)
end
"#;
    let result = compile_only(source);
    assert!(
        result.status.success(),
        "HTTP server with Request accessors should compile:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
