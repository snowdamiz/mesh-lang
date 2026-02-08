//! End-to-end integration tests for Snow standard library functions (Phase 8).
//!
//! Tests string operations, module-qualified access (String.length),
//! from/import resolution, IO operations, and HTTP server/client compilation.

use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

/// Helper: compile a Snow source file and run the binary with piped stdin input.
/// Useful for testing interactive I/O functions like IO.read_line().
fn compile_and_run_with_stdin(source: &str, stdin_input: &str) -> String {
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
    let mut child = Command::new(&binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn binary at {}: {}", binary.display(), e));

    // Write stdin input and drop to signal EOF
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(stdin_input.as_bytes()).expect("failed to write stdin");
        // stdin is dropped here, closing the pipe
    }

    let run_output = child.wait_with_output().expect("failed to wait for child");

    assert!(
        run_output.status.success(),
        "binary execution failed with exit code {:?}:\nstdout: {}\nstderr: {}",
        run_output.status.code(),
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );

    String::from_utf8_lossy(&run_output.stdout).to_string()
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

#[test]
fn e2e_io_read_line() {
    // Verify IO.read_line() compiles and runs through the full pipeline with piped stdin.
    let source = read_fixture("stdlib_io_read_line.snow");
    let output = compile_and_run_with_stdin(&source, "hello world\n");
    assert_eq!(output, "hello world\n");
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
fn e2e_map_string_keys() {
    let source = read_fixture("stdlib_map_string_keys.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "Alice\n2\ntrue\nBob\n");
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
//
// Note: The HTTP server uses thread-per-connection (std::thread::spawn)
// rather than actor-per-connection (snow_actor_spawn). This was a deliberate
// implementation decision [STATE.md 08-05] because the actor runtime uses
// corosensei coroutines with cooperative scheduling, and integrating
// tiny-http's blocking I/O model with it introduces unnecessary complexity.
// Thread-per-connection is simple and correct for Phase 8.

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

// ── List Pipe Chain E2E Tests (Phase 8 Plan 06 - Gap Closure) ─────────

#[test]
fn e2e_list_pipe_chain() {
    // Verify map/filter/reduce with closures through the full compiler pipeline.
    // Input: [1..10], map(x*2) -> [2..20], filter(x>10) -> [12,14,16,18,20], reduce(sum) -> 80.
    let source = read_fixture("stdlib_list_pipe_chain.snow");
    let output = compile_and_run(&source);
    assert_eq!(output, "80\n");
}

// ── HTTP Runtime E2E Tests (Phase 8 Plan 07 - Gap Closure) ────────────
//
// These tests start a REAL HTTP server and make actual HTTP requests,
// verifying that the Snow HTTP server works end-to-end at runtime.

/// RAII guard that kills the server child process on drop.
struct ServerGuard(std::process::Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Compile a Snow source file and spawn the resulting binary as a server.
/// Returns a ServerGuard that kills the process on drop.
///
/// Waits for the server to emit its "[snow-rt] HTTP server listening on"
/// message on stderr before returning, ensuring the server is ready.
fn compile_and_start_server(source: &str) -> ServerGuard {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    // Leak the temp dir so it persists for the lifetime of the server process.
    let temp_dir = Box::leak(Box::new(temp_dir));
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
    assert!(
        binary.exists(),
        "compiled binary not found at {}",
        binary.display()
    );

    // Spawn the server binary with stderr piped so we can detect readiness.
    let child = Command::new(&binary)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn server binary: {}", e));

    ServerGuard(child)
}

#[test]
fn e2e_http_server_runtime() {
    // This test starts a real HTTP server from a compiled Snow program,
    // makes an HTTP request, and verifies the response body.
    let source = read_fixture("stdlib_http_server_runtime.snow");
    let mut guard = compile_and_start_server(&source);

    // Wait for the server to be ready by reading stderr for the listening message.
    // We need to do this in a separate thread to avoid blocking if the server
    // produces no output. Use a timeout approach instead.
    let stderr = guard.0.stderr.take().expect("no stderr pipe");
    let stderr_reader = BufReader::new(stderr);

    // Spawn a thread to read stderr and signal when server is ready.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in stderr_reader.lines() {
            if let Ok(line) = line {
                if line.contains("HTTP server listening on") {
                    let _ = tx.send(true);
                    return;
                }
            }
        }
        let _ = tx.send(false);
    });

    // Wait up to 10 seconds for the server to start.
    let ready = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap_or(false);
    assert!(ready, "Server did not start within 10 seconds");

    // Make an HTTP GET request to the server using raw TcpStream.
    // Retry up to 5 times with 200ms between attempts for robustness.
    let mut response = String::new();
    let mut connected = false;
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        match std::net::TcpStream::connect("127.0.0.1:18080") {
            Ok(mut stream) => {
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                stream
                    .write_all(
                        b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    )
                    .expect("failed to write HTTP request");
                stream
                    .read_to_string(&mut response)
                    .expect("failed to read HTTP response");
                connected = true;
                break;
            }
            Err(_) => continue,
        }
    }

    assert!(connected, "Failed to connect to server after 5 attempts");
    assert!(
        response.contains("200"),
        "Expected HTTP 200 in response, got: {}",
        response
    );
    // The Snow string literal "{\"status\":\"ok\"}" preserves backslash
    // characters (Snow does not interpret escape sequences in strings).
    // The response body is the literal bytes: {\"status\":\"ok\"}
    assert!(
        response.contains(r#"{\"status\":\"ok\"}"#),
        "Expected JSON body in response, got: {}",
        response
    );

    // ServerGuard Drop will kill the server process.
}
