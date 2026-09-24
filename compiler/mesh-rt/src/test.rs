//! Test runtime support functions for the Mesh testing framework (Phase 138).
//!
//! Provides `extern "C"` functions called by compiled `*.test.mpl` programs.
//! The test harness (lowered by Plan 03) calls these functions to:
//!   - Begin each test (`mesh_test_begin`)
//!   - Record pass/fail outcomes (`mesh_test_pass`, `mesh_test_fail_msg`)
//!   - Assert conditions (`mesh_test_assert`, `mesh_test_assert_eq`,
//!     `mesh_test_assert_ne`, `mesh_test_assert_raises`)
//!   - Print summary at the end of the run (`mesh_test_summary`)
//!   - Clean up mock actors (`mesh_test_cleanup_actors`)
//!
//! ## State model
//!
//! All state is kept in `thread_local!` statics. Tests run single-threaded
//! from the generated `main`, so no locking is required.
//!
//! ## Failure output
//!
//! Failures print inline as each test fails, and are also accumulated in
//! `FAIL_MESSAGES`. `mesh_test_summary` reprints all failures in a
//! `Failures:` section before the final count line.
//!
//! ## Failures
//!
//! A failed assertion records its message and unwinds out of the test, or
//! out of its body when a teardown follows (`TestFailed`); the runner
//! catches it, as it catches a panic. A test counts once, however many of
//! its steps fail.
//! Inside `assert_raises`, a failed assertion only unwinds: it is the
//! "raise" the closure was expected to do.

use std::cell::{Cell, RefCell};
use std::io::Write as _;

use parking_lot::RwLock;

use crate::string::MeshString;

// ── ANSI color codes ─────────────────────────────────────────────────────────

/// ANSI color codes, empty when the output is not for a color terminal:
/// `meshc test` sets `MESH_TEST_COLOR` for the terminal it prints to (the
/// test binary's own stdout is a pipe); run directly, the binary looks at
/// its stdout. `NO_COLOR` turns colors off.
struct Palette {
    green: &'static str,
    red: &'static str,
    bold: &'static str,
    reset: &'static str,
}

fn palette() -> &'static Palette {
    static PALETTE: std::sync::OnceLock<Palette> = std::sync::OnceLock::new();
    PALETTE.get_or_init(|| {
        use std::io::IsTerminal;
        let color = match std::env::var("MESH_TEST_COLOR").as_deref() {
            Ok("1") => true,
            Ok(_) => false,
            Err(_) => std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
        };
        if color {
            Palette {
                green: "\x1b[32m",
                red: "\x1b[31m",
                bold: "\x1b[1m",
                reset: "\x1b[0m",
            }
        } else {
            Palette {
                green: "",
                red: "",
                bold: "",
                reset: "",
            }
        }
    })
}

/// `meshc test --quiet` (`MESH_TEST_QUIET=1`): a `.` or `F` per test
/// instead of its name; failures are shown in the summary.
fn quiet() -> bool {
    static QUIET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *QUIET.get_or_init(|| std::env::var("MESH_TEST_QUIET").as_deref() == Ok("1"))
}

// ── Per-process test state ────────────────────────────────────────────────────

thread_local! {
    static PASS_COUNT: Cell<i64> = Cell::new(0);
    static FAIL_COUNT: Cell<i64> = Cell::new(0);
    static CURRENT_TEST: RefCell<String> = RefCell::new(String::new());
    /// Accumulates failure messages for the end-of-run `Failures:` reprint.
    static FAIL_MESSAGES: RefCell<Vec<String>> = RefCell::new(Vec::new());
    /// Pids of mock actors spawned during the run; drained by cleanup_actors.
    static MOCK_ACTOR_PIDS: RefCell<Vec<i64>> = RefCell::new(Vec::new());

    /// Set while `assert_raises` runs its closure: a failed assertion there
    /// is the expected raise, not a test failure.
    static IN_ASSERT_RAISES: Cell<bool> = Cell::new(false);
    /// Whether the current test has failed (a step's assertion or panic).
    static CURRENT_FAILED: Cell<bool> = Cell::new(false);
}

/// The unwinding payload of a failed assertion, already recorded.
struct TestFailed;

static TEST_CASE_CLEANUP_HOOK: RwLock<Option<fn()>> = RwLock::new(None);

/// Register cleanup owned by a separately linked test-runtime extension.
///
/// This is a Rust-only hook: production host callbacks still require the
/// lifecycle-checked `mesh_library_register_host_callbacks` C API.
pub fn register_test_case_cleanup_hook(hook: fn()) {
    *TEST_CASE_CLEANUP_HOOK.write() = Some(hook);
}

fn run_test_case_cleanup_hook() {
    let hook = *TEST_CASE_CLEANUP_HOOK.read();
    if let Some(hook) = hook {
        hook();
    }
}

// ── Helper: read a MeshString as a &str ──────────────────────────────────────

/// Extract a `&str` from a pointer to a `MeshString`.
///
/// # Safety
///
/// `s` must be a valid, non-null pointer to an initialised `MeshString`
/// whose data bytes are valid UTF-8.
unsafe fn mesh_str<'a>(s: *const MeshString) -> &'a str {
    (*s).as_str()
}

/// Record a failure of the current test: the first one counts the test as
/// failed and marks it `✗`; each one prints its message.
fn record_failure(msg: &str) {
    let name = CURRENT_TEST.with(|ct| ct.borrow().clone());
    let first = !CURRENT_FAILED.with(|failed| failed.replace(true));
    if first {
        FAIL_COUNT.with(|c| c.set(c.get() + 1));
    }
    let Palette {
        red, bold, reset, ..
    } = palette();
    // Every line of the message under the test's name.
    let msg = msg.replace('\n', "\n    ");
    if quiet() {
        if first {
            print!("{red}F{reset}");
            let _ = std::io::stdout().flush();
        }
    } else {
        if first {
            println!("  {red}✗{reset} {name}");
        }
        println!("    {red}{msg}{reset}");
    }
    FAIL_MESSAGES.with(|fm| {
        let mut messages = fm.borrow_mut();
        let line = format!("    {red}{msg}{reset}");
        match messages.last_mut() {
            Some(entry) if !first => {
                entry.push('\n');
                entry.push_str(&line);
            }
            _ => messages.push(format!("  {red}{bold}✗{reset} {name}\n{line}")),
        }
    });
}

/// A failed assertion: the test fails, or, inside `assert_raises`, the
/// closure has raised. Either way it ends the step that is running.
fn assertion_failed(msg: impl FnOnce() -> String) -> ! {
    if !IN_ASSERT_RAISES.with(|f| f.get()) {
        record_failure(&msg());
    }
    std::panic::resume_unwind(Box::new(TestFailed))
}

// ── Public extern "C" functions ───────────────────────────────────────────────

/// Called by the test harness before each test: stores the test name for
/// the pass/fail lines, printed when the test ends (output the test prints
/// comes before its line).
#[no_mangle]
pub extern "C" fn mesh_test_begin(name: *const MeshString) {
    run_test_case_cleanup_hook();
    let name_str = unsafe { mesh_str(name) }.to_owned();
    CURRENT_TEST.with(|ct| *ct.borrow_mut() = name_str.clone());
    CURRENT_FAILED.with(|failed| failed.set(false));
}

/// Record the current test as passed: `✓ name`, or `.` in quiet mode.
#[no_mangle]
pub extern "C" fn mesh_test_pass() {
    PASS_COUNT.with(|c| c.set(c.get() + 1));

    let Palette { green, reset, .. } = palette();
    if quiet() {
        print!("{green}.{reset}");
        let _ = std::io::stdout().flush();
    } else {
        let name = CURRENT_TEST.with(|ct| ct.borrow().clone());
        println!("  {green}✓{reset} {name}");
    }
}

/// `test_fail_msg(msg)` (what `assert_receive` expands to on a miss): fail
/// the current test with `msg`.
#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_fail_msg(msg: *const MeshString) {
    assertion_failed(|| mesh_str(msg).to_owned())
}

/// Assert that `cond` is non-zero; a failure ends the test (see
/// `assertion_failed`).
#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_assert(
    cond: i8,
    expr_src: *const MeshString,
    _file: *const u8,
    _file_len: i64,
    _line: i64,
) {
    if cond == 0 {
        assertion_failed(|| format!("assert failed: {}", mesh_str(expr_src)));
    }
}

/// Assert that `lhs` and `rhs` (already converted to strings by the lowerer)
/// are equal. Fails with an `expected`/`actual` diagnostic.
#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_assert_eq(
    lhs: *const MeshString,
    rhs: *const MeshString,
    expr_src: *const MeshString,
    _file: *const u8,
    _file_len: i64,
    _line: i64,
) {
    let l = mesh_str(lhs);
    let r = mesh_str(rhs);
    if l != r {
        assertion_failed(|| {
            let src = mesh_str(expr_src);
            format!("assert_eq failed: {src}\n  left:  {l}\n  right: {r}")
        });
    }
}

/// Assert that `lhs` and `rhs` are NOT equal. Fails when they are equal.
#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_assert_ne(
    lhs: *const MeshString,
    rhs: *const MeshString,
    expr_src: *const MeshString,
    _file: *const u8,
    _file_len: i64,
    _line: i64,
) {
    let l = mesh_str(lhs);
    let r = mesh_str(rhs);
    if l == r {
        assertion_failed(|| {
            let src = mesh_str(expr_src);
            format!("assert_ne failed: {src}\n  both sides equal: {l}")
        });
    }
}

/// Assert that calling the closure `fn_ptr(env_ptr)` raises: panics (a
/// failed match, `List.get` past the end) or fails an assertion, which
/// inside it only unwinds (`IN_ASSERT_RAISES`).
///
/// The closure ABI matches the Mesh runtime closure convention:
/// `extern "C" fn(*const u8) -> i64`.
#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_assert_raises(
    fn_ptr: *const u8,
    env_ptr: *const u8,
    _file: *const u8,
    _file_len: i64,
    _line: i64,
) {
    // Nested assert_raises calls restore the outer state.
    let prev_in_raises = IN_ASSERT_RAISES.with(|f| f.replace(true));
    let raised = call_catching_panic(fn_ptr, env_ptr).is_err();
    IN_ASSERT_RAISES.with(|f| f.set(prev_in_raises));

    if !raised {
        assertion_failed(|| "assert_raises failed: expression did not raise".to_string());
    }
}

/// Print the run summary and exit the process with the appropriate code.
///
/// First reprints all accumulated failure messages in a `Failures:` section,
/// then prints the final `N passed` / `N failed, M passed` count line.
///
/// Exits with code `0` when all tests passed, `1` when any tests failed.
/// This lets the outer `meshc test` runner detect test failures via exit code.
///
/// The harness (`Plan 03`) passes the elapsed time as milliseconds.
#[no_mangle]
pub extern "C" fn mesh_test_summary(passed: i64, failed: i64, elapsed_ms: i64) {
    let Palette {
        green,
        red,
        bold,
        reset,
    } = palette();
    if quiet() {
        println!();
    }
    // Reprint accumulated failures at the bottom of the run.
    FAIL_MESSAGES.with(|fm| {
        let messages = fm.borrow();
        if !messages.is_empty() {
            println!("\n{bold}Failures:{reset}");
            for msg in messages.iter() {
                println!("{msg}");
            }
        }
    });

    let elapsed = elapsed_ms as f64 / 1000.0;
    if failed > 0 {
        println!("\n{red}{bold}{failed} failed{reset}, {passed} passed in {elapsed:.2}s");
        std::process::exit(1);
    } else {
        println!("\n{green}{bold}{passed} passed{reset} in {elapsed:.2}s");
        std::process::exit(0);
    }
}

/// Clean up any mock actors registered during the test run.
///
/// Drains `MOCK_ACTOR_PIDS` and calls `mesh_actor_exit` for each Pid.
/// Plan 03 populates `MOCK_ACTOR_PIDS` when `Test.mock_actor` is called.
#[no_mangle]
pub extern "C" fn mesh_test_cleanup_actors() {
    let pids: Vec<i64> = MOCK_ACTOR_PIDS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for pid in pids {
        // Reason tag 0 = normal exit (same convention as actor/mod.rs).
        crate::actor::mesh_actor_exit(pid as u64, 0);
    }
}

/// Register a mock actor Pid for cleanup at the end of the run.
///
/// Called by the Plan 03 `Test.mock_actor` implementation.
#[allow(dead_code)]
pub fn register_mock_actor_pid(pid: i64) {
    MOCK_ACTOR_PIDS.with(|p| p.borrow_mut().push(pid));
}

/// Return the current pass count (for use in test harness summary).
#[no_mangle]
pub extern "C" fn mesh_test_pass_count() -> i64 {
    PASS_COUNT.with(|c| c.get())
}

/// Return the current fail count (for use in test harness summary).
#[no_mangle]
pub extern "C" fn mesh_test_fail_count() -> i64 {
    FAIL_COUNT.with(|c| c.get())
}

/// Call a Mesh closure, catching a panic that unwinds out of it (a failed
/// assertion, a failed match, `List.get` past the end). `Err(None)` is a
/// failed assertion, already recorded; `Err(Some(message))` a panic. The
/// panic hook leaves reporting a panic to the caller.
unsafe fn call_catching_panic(fn_ptr: *const u8, env_ptr: *const u8) -> Result<(), Option<String>> {
    let f: extern "C-unwind" fn(*const u8) -> i64 = std::mem::transmute(fn_ptr);
    crate::panic::quietly(|| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f(env_ptr);
        }))
    })
    .map_err(|payload| {
        if payload.is::<TestFailed>() {
            return None;
        }
        Some(
            crate::panic::mesh_panic_message(&*payload)
                .unwrap_or("Mesh panic: the runtime failed (reported above)")
                .to_string(),
        )
    })
}

/// Run a test (the harness calls this with a closure), or its body when a
/// teardown follows: a failed assertion or a panic fails the test.
#[no_mangle]
pub unsafe extern "C" fn mesh_test_run_body(fn_ptr: *const u8, env_ptr: *const u8) {
    if let Err(Some(message)) = call_catching_panic(fn_ptr, env_ptr) {
        // "panicked in f: ...", "panicked: List.get: ..."
        let detail = message.strip_prefix("Mesh panic").unwrap_or(&message);
        record_failure(&format!("panicked{detail}"));
    }
}

/// End the current test: it passed unless a step failed.
#[no_mangle]
pub extern "C" fn mesh_test_end() {
    run_test_case_cleanup_hook();
    if !CURRENT_FAILED.with(|failed| failed.get()) {
        mesh_test_pass();
    }
}

/// Spawn a mock actor whose body is the given closure.
///
/// The spawned actor runs the closure for every message it receives.
/// The Pid is tracked in `MOCK_ACTOR_PIDS` for cleanup between tests.
///
/// Closure ABI: `extern "C" fn(env_ptr: *const u8) -> i64`.
///
/// Requires `mesh_rt_init_actor` to have been called first.
#[no_mangle]
pub unsafe extern "C" fn mesh_test_mock_actor(fn_ptr: *const u8, env_ptr: *const u8) -> i64 {
    // Build a small args block: {fn_ptr, env_ptr} so the spawned actor
    // has access to the closure. The actor entry function is the standard
    // closure dispatch shim at mesh_actor_closure_runner (from actor/mod.rs).
    // Since we don't have a dedicated closure-runner entry point exposed here,
    // we use mesh_actor_spawn with the fn_ptr directly.
    //
    // Pack fn_ptr and env_ptr into a heap-allocated args block.
    #[repr(C)]
    struct MockArgs {
        fn_ptr: *const u8,
        env_ptr: *const u8,
    }

    // Leak args so the actor thread can read them.
    let args = Box::new(MockArgs { fn_ptr, env_ptr });
    let args_ptr = Box::into_raw(args) as *const u8;
    let args_size = std::mem::size_of::<MockArgs>() as u64;

    // The actor entry function: reads MockArgs and calls fn_ptr(env_ptr) in a loop.
    // We use a generic wrapper defined below.
    extern "C" fn mock_actor_entry(args: *const u8) {
        unsafe {
            let mock_args = &*(args as *const MockArgs);
            let f: extern "C" fn(*const u8) -> i64 = std::mem::transmute(mock_args.fn_ptr);
            // Run the closure once per message received.
            // In a real implementation this would loop on receive; for test mocks
            // we run the closure once and exit.
            loop {
                let msg_ptr = crate::actor::mesh_actor_receive(100); // 100ms timeout
                if msg_ptr.is_null() {
                    break; // no message in time window — exit actor
                }
                f(mock_args.env_ptr);
            }
        }
    }

    let pid = crate::actor::mesh_actor_spawn(
        mock_actor_entry as *const u8,
        args_ptr,
        args_size,
        1, // Normal priority
    ) as i64;

    MOCK_ACTOR_PIDS.with(|p| p.borrow_mut().push(pid));
    pid
}
