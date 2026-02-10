//! End-to-end integration tests for Snow concurrency standard library (Phase 9).
//!
//! Tests Service and Job constructs through the full compiler pipeline:
//! Snow source -> parse -> typecheck -> MIR -> LLVM codegen -> native binary -> run.
//!
//! Service tests use generous timeouts since they involve actor concurrency.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Helper: compile a Snow source and run the binary with a timeout.
/// Returns stdout on success. Panics on compilation failure or timeout.
fn compile_and_run_with_timeout(source: &str, timeout_secs: u64) -> String {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_snow = project_dir.join("main.snow");
    std::fs::write(&main_snow, source).expect("failed to write main.snow");

    // Build with snowc
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

    // Run the compiled binary with a timeout
    let binary = project_dir.join("project");
    let child = Command::new(&binary)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn binary at {}: {}", binary.display(), e));

    let output = wait_with_timeout(child, Duration::from_secs(timeout_secs));

    match output {
        Ok(out) => {
            assert!(
                out.status.success(),
                "binary execution failed with exit code {:?}:\nstdout: {}\nstderr: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).to_string()
        }
        Err(msg) => panic!("{}", msg),
    }
}

/// Wait for a child process with a timeout. Kill it if it exceeds the timeout.
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(50);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    out.read_to_end(&mut stdout).ok();
                }
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    err.read_to_end(&mut stderr).ok();
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Binary timed out after {} seconds",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => return Err(format!("Error waiting for process: {}", e)),
        }
    }
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

// ── Service E2E Tests ──────────────────────────────────────────────────

/// Test: Counter service with start, call (GetCount, Increment), and cast (Reset).
/// Exercises: service definition, init, call with reply, cast fire-and-forget.
#[test]
fn e2e_service_counter() {
    let source = read_fixture("service_counter.snow");
    let output = compile_and_run_with_timeout(&source, 10);
    assert_eq!(output, "10\n15\n0\n");
}

/// Test: Service with multiple call/cast operations.
/// Exercises: multiple handler dispatch on type tags.
#[test]
fn e2e_service_call_cast() {
    let source = read_fixture("service_call_cast.snow");
    let output = compile_and_run_with_timeout(&source, 10);
    assert_eq!(output, "100\n200\n0\n");
}

/// Test: Accumulator service proving state persistence across calls.
/// Exercises: functional state management (handler receives state, returns new state).
#[test]
fn e2e_service_state_management() {
    let source = read_fixture("service_state_management.snow");
    let output = compile_and_run_with_timeout(&source, 10);
    assert_eq!(output, "6\n");
}

// ── Job E2E Tests ──────────────────────────────────────────────────────

/// Test: Job.async spawns work, Job.await collects Result.
/// Exercises: Job.async with closure, Job.await returning Ok(value).
#[test]
fn e2e_job_async_await() {
    let source = read_fixture("job_async_await.snow");
    let output = compile_and_run_with_timeout(&source, 10);
    assert_eq!(output, "42\n");
}

// ── Receive-with-timeout E2E Tests ──────────────────────────────────

/// Test: receive timeout fires when no message arrives within the deadline.
/// Exercises: receive do msg -> msg after 50 -> 99 end returns 99 on timeout.
#[test]
fn test_receive_after_timeout_fires() {
    let source = r#"
actor worker() do
  let result = receive do
    msg -> msg
  after 50 -> 99 end
  println("${result}")
end

fn main() do
  spawn(worker)
end
"#;
    let output = compile_and_run_with_timeout(source, 5);
    assert_eq!(output.trim(), "99");
}

/// Test: message arrives before timeout, so the arm body runs (not timeout body).
/// Exercises: receive do msg -> msg after 5000 -> 99 end with immediate send returns 42.
#[test]
fn test_receive_after_message_arrives_before_timeout() {
    let source = r#"
actor worker() do
  let result = receive do
    msg -> msg
  after 5000 -> 99 end
  println("${result}")
end

fn main() do
  let pid = spawn(worker)
  send(pid, 42)
end
"#;
    let output = compile_and_run_with_timeout(source, 5);
    assert_eq!(output.trim(), "42");
}

/// Test: timeout body returns String type (verifies type unification end-to-end).
/// Exercises: receive do msg -> msg after 50 -> "timeout" end returns the string.
#[test]
fn test_receive_after_timeout_returns_string() {
    let source = r#"
actor worker() do
  let result = receive do
    msg -> msg
  after 50 -> "timeout" end
  println(result)
end

fn main() do
  spawn(worker)
end
"#;
    let output = compile_and_run_with_timeout(source, 5);
    assert_eq!(output.trim(), "timeout");
}
