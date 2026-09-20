//! End-to-end integration tests for the Mesh actor runtime.
//!
//! Each test compiles a .mpl program that exercises actor features,
//! runs the resulting binary, and asserts the expected stdout output.
//!
//! Actor tests use generous timeouts since they involve concurrency.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

// ponytail: serialize compile-heavy actor proofs; remove when isolated builds stay below deadlines.
static ACTOR_E2E_LOCK: Mutex<()> = Mutex::new(());

/// Helper: compile a Mesh source and run the binary with a timeout.
/// Returns stdout on success. Panics on compilation failure or timeout.
fn compile_and_run_with_timeout(source: &str, timeout_secs: u64) -> String {
    compile_and_run(source, &[], timeout_secs)
}

/// As above, with extra `meshc build` arguments such as `--opt-level`.
fn compile_and_run(source: &str, build_args: &[&str], timeout_secs: u64) -> String {
    let _guard = ACTOR_E2E_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let project_dir = temp_dir.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

    let main_mesh = project_dir.join("main.mpl");
    std::fs::write(&main_mesh, source).expect("failed to write main.mpl");

    // Build with meshc
    let meshc = find_meshc();
    let output = Command::new(&meshc)
        .args(["build", project_dir.to_str().unwrap()])
        .args(build_args)
        .output()
        .expect("failed to invoke meshc");

    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
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
                // Process exited. Collect stdout/stderr.
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
                // Still running
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

/// Find the meshc binary in the target directory.
fn find_meshc() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot find current exe")
        .parent()
        .expect("cannot find parent dir")
        .to_path_buf();

    if path.file_name().map_or(false, |n| n == "deps") {
        path = path.parent().unwrap().to_path_buf();
    }

    let meshc = path.join("meshc");
    assert!(
        meshc.exists(),
        "meshc binary not found at {}. Run `cargo build -p meshc` first.",
        meshc.display()
    );
    meshc
}

// ── Actor E2E Tests ─────────────────────────────────────────────────────

/// Test 1: Basic actor spawning and messaging.
/// An actor receives a message and prints a response.
#[test]
fn actors_basic() {
    let source = read_fixture("actors_basic.mpl");
    let output = compile_and_run_with_timeout(&source, 10);
    assert!(
        output.contains("actor received"),
        "Expected 'actor received' in output, got: {}",
        output
    );
    assert!(
        output.contains("main done"),
        "Expected 'main done' in output, got: {}",
        output
    );
}

/// Test 2: Receive with message processing.
/// Multiple actors receive messages and process them.
#[test]
fn actors_messaging() {
    let source = read_fixture("actors_messaging.mpl");
    let output = compile_and_run_with_timeout(&source, 10);
    // All three workers should print their done message
    let count = output.matches("worker done").count();
    assert!(
        count >= 3,
        "Expected at least 3 'worker done' messages, got {} in: {}",
        count,
        output
    );
}

/// Test 3: Preemptive scheduling -- a tight-loop actor does not starve others.
/// One actor does a lot of work while another waits for a message.
/// Both should complete.
#[test]
fn actors_preemption() {
    let source = read_fixture("actors_preemption.mpl");
    let output = compile_and_run_with_timeout(&source, 10);
    assert!(
        output.contains("fast done"),
        "Expected 'fast done' in output (fast actor was not starved), got: {}",
        output
    );
    assert!(
        output.contains("slow done"),
        "Expected 'slow done' in output (slow actor completed), got: {}",
        output
    );
}

/// Test 4: Process linking -- when one actor exits, linked actor is notified.
/// This tests the exit signal propagation through link().
#[test]
fn actors_linking() {
    let source = read_fixture("actors_linking.mpl");
    let output = compile_and_run_with_timeout(&source, 10);
    assert!(
        output.contains("link test done"),
        "Expected 'link test done' in output, got: {}",
        output
    );
}

/// Test 5: Typed Pid prevents wrong-type sends at compile time.
/// A program that tries to send the wrong type to a typed Pid should fail.
#[test]
fn actors_typed_pid() {
    let source = read_fixture("actors_typed_pid.mpl");
    let output = compile_and_run_with_timeout(&source, 10);
    assert!(
        output.contains("typed pid ok"),
        "Expected 'typed pid ok' in output, got: {}",
        output
    );
}

/// Test 6: 100K actor benchmark -- spawn 100K actors and verify they all respond.
#[test]
fn actors_100k() {
    let source = read_fixture("actors_100k.mpl");
    let output = compile_and_run_with_timeout(&source, 30);
    assert!(
        output.contains("100000 actors done"),
        "Expected '100000 actors done' in output, got: {}",
        output
    );
}

/// Test 7: Terminate callback -- cleanup logic runs before actor exit.
#[test]
fn actors_terminate() {
    let source = read_fixture("actors_terminate.mpl");
    let output = compile_and_run_with_timeout(&source, 10);
    assert!(
        output.contains("terminate test done"),
        "Expected 'terminate test done' in output, got: {}",
        output
    );
}

/// Test 8: GC bounded memory -- a long-running actor allocates and discards
/// strings in a tight loop. With mark-sweep GC, unreachable allocations are
/// reclaimed at yield points so memory stays bounded.
#[test]
fn gc_bounded_memory() {
    let source = read_fixture("gc_bounded_memory.mpl");
    let output = compile_and_run_with_timeout(&source, 30);
    assert!(
        output.contains("gc bounded memory test done"),
        "Expected 'gc bounded memory test done' in output, got: {}",
        output
    );
}

#[test]
fn gc_preserves_live_values_held_across_collection_points() {
    let source = read_fixture("gc_live_register_roots.mpl");
    let output = compile_and_run_with_timeout(&source, 30);
    assert!(
        output.contains("gc live register roots preserved"),
        "live value was corrupted across collection: {output}"
    );
}

/// A heap value passed to `spawn` is borrowed from the spawner's heap. It must
/// survive the spawner dropping its own reference and collecting repeatedly.
///
/// Built at `--opt-level 2`: unoptimized code leaves the value in a dead stack
/// slot, which the conservative collector still treats as a root, so the
/// default build would pass even without the runtime keeping the loan alive.
#[test]
fn actors_spawn_arg_survives_spawner_gc() {
    let source = read_fixture("actors_spawn_arg_survives_gc.mpl");
    let output = compile_and_run(&source, &["--opt-level", "2"], 30);
    assert_eq!(
        output.trim(),
        "payload-42",
        "spawn argument was freed and reused by the spawner"
    );
}

/// The same value handed on to a grandchild outlives the actor in the middle.
#[test]
fn actors_relayed_spawn_arg_outlives_the_relay() {
    let source = read_fixture("actors_spawn_arg_relayed.mpl");
    let output = compile_and_run(&source, &["--opt-level", "2"], 30);
    assert_eq!(
        output.trim(),
        "payload-42",
        "relayed spawn argument was freed and reused by its owner"
    );
}

/// A `String` message is copied for the receiver rather than shared with the
/// sender's heap, so it survives the sender collecting afterwards.
#[test]
fn actors_send_string_survives_sender_gc() {
    let source = read_fixture("actors_send_string_survives_gc.mpl");
    let output = compile_and_run(&source, &["--opt-level", "2"], 30);
    assert_eq!(
        output.trim(),
        "payload-42",
        "sent string was freed and reused by the sender"
    );
}

/// Run a fixture built at `--opt-level 2` and compare its output lines, in any
/// order: the actors in these fixtures finish in no particular sequence.
fn assert_lines(fixture: &str, expected: &[&str]) {
    let source = read_fixture(fixture);
    let output = compile_and_run(&source, &["--opt-level", "2"], 60);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(lines, expected, "{fixture} printed:\n{output}");
}

/// Every common message shape arrives as the receiver's own copy, while the
/// sender drops each value and keeps collecting. Also covers messages wider
/// than one word, which `receive` used to truncate to their first 8 bytes, and
/// a one-word struct, which a tuple field holds without a box.
#[test]
fn actors_send_shapes_survive_sender_gc() {
    assert_lines(
        "actors_send_shapes_survive_gc.mpl",
        &[
            "named=named-1001",
            "pair=paired-102/7",
            "wrapped=boxjob-103/3",
            "tuple=tupled-104/4",
            "struct=struct-105/5",
            "batch=batch-a-115+batch-b-116/15",
            "list=listed-106+listed-107",
            "jobs=jobs-b-109/9",
            "map=mapped-110",
            "option=option-111",
            "result=result-112",
            "pairs=pairs-b-114/14",
        ],
    );
}

/// A heap string handed to `Timer.send_after` is copied when the timer is set,
/// not read from the sender's heap when it fires.
#[test]
fn actors_timer_send_string_survives_sender_gc() {
    assert_lines("actors_timer_send_string_survives_gc.mpl", &["payload-42"]);
}

/// Closure- and struct-typed actor parameters, which are wider than one
/// argument slot, and a string relayed through an actor that exits at once.
#[test]
fn actors_spawn_arg_shapes_reach_their_actor() {
    assert_lines(
        "actors_spawn_arg_shapes.mpl",
        &[
            "closure=item-11/7",
            "struct=job-33x3",
            "batch=batch-44+batch-55",
            "relayed=relay-22",
        ],
    );
}

/// Service arguments and replies are copied between the caller and the
/// service: list and struct arguments, list and string replies, and a cast.
#[test]
fn service_heap_values_survive_gc() {
    assert_lines(
        "service_heap_values_survive_gc.mpl",
        &[
            "reply=stocked-2",
            "tagged=item-a-101,item-b-102,entry-103#7",
            "loaded=2:batch-105",
            "first=cast-x-104",
        ],
    );
}

/// A job's result outlives the job actor's heap: strings, lists, structs,
/// options, tuples and a closure from `Job.async`, and `Job.map` called
/// directly and through a pipe. Struct results also used to come back through the wrong
/// calling convention, and string results as a box around the pointer.
#[test]
fn job_heap_results_survive_gc() {
    assert_lines(
        "job_heap_results_survive_gc.mpl",
        &[
            "payload-42",
            "line-41+line-42",
            "report-41: a-41+b-41 = 82",
            "some inner-41",
            "pair-41 -> 43",
            "123",
            "2.5",
            "true",
            "mapped-100",
            "mapped-300",
            "piped-500",
            "mapped-report-8: m-8 = 8",
            "made-41#9",
        ],
    );
}

/// Test 9: Actors with arguments -- spawn passes initial state correctly.
/// Actors receive typed arguments, not raw buffer pointers. Tests the
/// wrapper function that deserializes args from the spawn buffer.
#[test]
fn actors_with_args() {
    let source = read_fixture("actors_with_args.mpl");
    let output = compile_and_run_with_timeout(&source, 10);
    assert!(
        output.contains("15"),
        "Expected '15' (adder initial=10 + msg=5) in output, got: {}",
        output
    );
    assert!(
        output.contains("42"),
        "Expected '42' (calculator 30 + 12) in output, got: {}",
        output
    );
    assert!(
        output.contains("args test done"),
        "Expected 'args test done' in output, got: {}",
        output
    );
}
