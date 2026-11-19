//! End-to-end integration tests for the Mesh supervisor compiler pipeline.
//!
//! Each test compiles a .mpl program that exercises supervisor features,
//! builds it into a native binary, and verifies expected behavior.

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Helper: compile a Mesh source file and assert it compiles successfully.
/// Returns the path to the compiled binary.
fn compile_mesh(source: &str) -> (tempfile::TempDir, PathBuf) {
    compile_at_level(source, "0")
}

fn compile_at_level(source: &str, level: &str) -> (tempfile::TempDir, PathBuf) {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_mesh = project_dir.join("main.mpl");
    std::fs::write(&main_mesh, source).expect("failed to write main.mpl");

    // Build with meshc
    let meshc = find_meshc();
    let output = Command::new(&meshc)
        .args(["build", "--opt-level", level, project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke meshc");

    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let binary = project_dir.join("project");
    assert!(
        binary.exists(),
        "compiled binary not found at {}",
        binary.display()
    );

    (temp_dir, binary)
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

fn find_meshc() -> PathBuf {
    artifacts::meshc_bin()
}

/// Helper: compile a Mesh source file without asserting success.
/// Returns the raw compilation output (for negative tests).
fn compile_only(source: &str) -> Output {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_mesh = project_dir.join("main.mpl");
    std::fs::write(&main_mesh, source).expect("failed to write main.mpl");

    let meshc = find_meshc();
    Command::new(&meshc)
        .args(["build", project_dir.to_str().unwrap()])
        .output()
        .expect("failed to invoke meshc")
}

/// Helper: run a compiled binary with a timeout and return stdout.
fn run_with_timeout(binary: &Path, timeout_secs: u64) -> String {
    let child = Command::new(binary)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn binary: {}", e));
    let output = artifacts::wait_with_timeout(child, std::time::Duration::from_secs(timeout_secs))
        .unwrap_or_else(|timed_out| panic!("{timed_out}"));

    assert!(
        output.status.success(),
        "binary failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

// ── Supervisor E2E Tests ─────────────────────────────────────────────────

/// Test: Supervisor block compiles to a native binary successfully.
///
/// This validates the full compiler pipeline: parsing supervisor blocks,
/// type checking, MIR lowering, and LLVM codegen with supervisor intrinsics.
#[test]
fn supervisor_basic() {
    let source = read_fixture("supervisor_basic.mpl");
    let (_temp_dir, binary) = compile_mesh(&source);

    // Verify the binary exists and is executable.
    assert!(binary.exists(), "compiled supervisor binary should exist");

    // Run the binary with a short timeout -- it should print and exit.
    let child = Command::new(&binary)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn binary: {}", e));
    let output = artifacts::wait_with_timeout(child, std::time::Duration::from_secs(10))
        .unwrap_or_else(|timed_out| panic!("supervisor {timed_out}"));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("supervisor started"),
        "Expected 'supervisor started' in output, got: {}",
        stdout
    );
}

/// Test: one_for_all supervisor compiles and runs.
///
/// Success Criterion 2: one_for_all strategy is recognized by compiler
/// and runtime. The supervisor starts and prints expected output.
#[test]
fn supervisor_one_for_all() {
    let source = read_fixture("supervisor_one_for_all.mpl");
    let (_temp_dir, binary) = compile_mesh(&source);

    let stdout = run_with_timeout(&binary, 10);
    assert!(
        stdout.contains("one_for_all supervisor started"),
        "Expected 'one_for_all supervisor started' in output, got: {}",
        stdout
    );
}

/// Test: supervisor with restart limits compiles and runs.
///
/// Success Criterion 3: restart limits are part of the compiled supervisor
/// configuration. The supervisor starts and manages restart counting.
#[test]
fn supervisor_restart_limit() {
    let source = read_fixture("supervisor_restart_limit.mpl");
    let (_temp_dir, binary) = compile_mesh(&source);

    let stdout = run_with_timeout(&binary, 10);
    assert!(
        stdout.contains("restart limit test started"),
        "Expected 'restart limit test started' in output, got: {}",
        stdout
    );
}

/// A permanent child that panics after yielding must be restarted by its
/// one_for_one supervisor without terminating the native Mesh process.
#[test]
fn supervisor_restarts_crashed_permanent_child() {
    let source = r#"
fn deliberate_crash(0) -> Int do
  0
end

actor worker() do
  println("worker-started")
  Timer.sleep(25)
  println("worker-awake")
  deliberate_crash(1)
end

supervisor WorkerSup do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child worker do
    start: fn -> spawn(worker) end
    restart: permanent
    shutdown: 100
  end
end

fn main() do
  let _ = spawn(WorkerSup)
  Timer.sleep(250)
end
"#;
    let (_temp_dir, binary) = compile_mesh(source);
    let stdout = run_with_timeout(&binary, 10);

    assert!(
        stdout.matches("worker-started").count() >= 2,
        "expected a crashed permanent child to restart, got: {stdout:?}"
    );
}

#[test]
fn optimized_explicit_panic_restarts_a_transient_child() {
    let source = r#"
actor worker(reason :: String) do
  println("started")
  Timer.sleep(5)
  panic(reason)
  println("continued")
end

supervisor Sup do
  strategy: one_for_one
  max_restarts: 1
  max_seconds: 5
  child worker do
    start: fn -> spawn(worker, "child failed") end
    restart: transient
    shutdown: 100
  end
end

actor watcher() do
  receive do
    "start" -> ()
    _ -> panic("unexpected start message")
  end
  let sup = spawn(Sup)
  Process.monitor(sup, "finished")
  receive do
    "finished" -> println("supervisor exited")
    _ -> panic("unexpected monitor message")
    after 5000 -> panic("supervisor did not exit")
  end
end

fn main() do
  let watcher_pid = spawn(watcher)
  Process.register("watcher", watcher_pid)
  send(watcher_pid, "start")
  while Process.whereis("watcher") == watcher_pid do
    Timer.sleep(1)
  end
  if Env.get_int("EXIT_AFTER_CRASH", 0) == 1 do
    Process.exit(17)
  end
end
"#;
    let (_temp_dir, binary) = compile_at_level(source, "2");
    let child = Command::new(&binary)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let output = artifacts::wait_with_timeout(child, std::time::Duration::from_secs(10)).unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["started", "started", "supervisor exited"]
    );
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["Mesh panic: child failed"; 2]
    );
    #[cfg(unix)]
    for explicit_exit in [false, true] {
        restart_with_blocked_stderr(&binary, explicit_exit);
    }
}

#[cfg(unix)]
fn restart_with_blocked_stderr(binary: &Path, explicit_exit: bool) {
    use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    use std::process::Stdio;
    use std::time::Duration;

    let timeout = Duration::from_secs(10);
    let (reader, stdout) = UnixStream::pair().unwrap();
    reader.set_read_timeout(Some(timeout)).unwrap();
    let (mut errors, mut stderr) = UnixStream::pair().unwrap();
    errors.set_read_timeout(Some(timeout)).unwrap();
    stderr.set_nonblocking(true).unwrap();
    let mut filled = 0;
    // Fill even the last few bytes so the first real diagnostic must wait.
    for chunk in [&[0; 4096][..], &[0][..]] {
        loop {
            match stderr.write(chunk) {
                Ok(n) if n > 0 => filled += n,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                result => panic!("could not fill stderr: {result:?}"),
            }
        }
    }
    stderr.set_nonblocking(false).unwrap();
    let mut child = Command::new(binary)
        .env("MESH_SCHEDULER_MIN_WORKERS", "1")
        .env("MESH_SCHEDULER_MAX_WORKERS", "1")
        .env("EXIT_AFTER_CRASH", if explicit_exit { "1" } else { "0" })
        .stdout(Stdio::from(OwnedFd::from(stdout)))
        .stderr(Stdio::from(OwnedFd::from(stderr)))
        .spawn()
        .unwrap();

    let progress: std::io::Result<Vec<_>> = BufReader::new(reader).lines().take(3).collect();
    // Release the writer even if recovery stalled, then reap the process.
    let mut bytes = Vec::new();
    let drained = errors.read_to_end(&mut bytes);
    if drained.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().unwrap();
    assert_eq!(
        progress.unwrap(),
        ["started", "started", "supervisor exited"]
    );
    drained.unwrap();
    assert_eq!(status.code(), Some(if explicit_exit { 17 } else { 0 }));
    assert_eq!(
        &bytes[filled..],
        b"Mesh panic: child failed\nMesh panic: child failed\n"
    );
}

/// Test: supervisor with invalid child spec is rejected at compile time.
///
/// Success Criterion 4: typed supervision validates at compile time.
/// A child spec whose start function returns Int (not Pid) produces
/// error E0018 (InvalidChildStart).
#[test]
fn supervisor_typed_error_rejected() {
    let source = read_fixture("supervisor_typed_error.mpl");
    let result = compile_only(&source);

    assert!(
        !result.status.success(),
        "Should fail to compile: start fn returns Int, not Pid"
    );

    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let all_output = format!("{}{}", stdout, stderr);

    assert!(
        all_output.contains("E0018")
            || all_output.contains("must return Pid")
            || all_output.contains("InvalidChildStart"),
        "Should report child start type error (E0018).\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );
}

#[test]
fn shutdown_stops_a_receive_inside_a_closure_without_returning_to_its_caller() {
    let source = r#"
actor worker() do
  let wait_for_work = fn ->
    receive do
      _ -> println("unexpected message")
    end
  end
  wait_for_work()
  println("worker alive")
end
fn main() do
  let worker_id = spawn(worker)
  Timer.sleep(50)
  println("nested worker started")
end
"#;
    let (_temp_dir, binary) = compile_mesh(source);
    let stdout = run_with_timeout(&binary, 10);
    assert_eq!(
        stdout.trim(),
        "nested worker started",
        "shutdown returned from receive: {stdout}"
    );
}
