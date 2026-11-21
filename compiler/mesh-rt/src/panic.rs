//! Runtime panic handler for Mesh programs.
//!
//! Called when a Mesh program encounters an unrecoverable error at runtime,
//! such as a non-exhaustive match failure (guarded arms edge case).
//!
//! A panic unwinds rather than aborting, carrying its `R0001` diagnostic,
//! so that actor crash isolation via `catch_unwind` can intercept it
//! without bringing down the entire process.

use std::any::Any;
use std::io::Write as _;

use mesh_diagnostics::{Code, Diagnostic, Format, Reporter, SourceFile};

/// What a Mesh panic unwinds with. `reason` is what linked actors,
/// supervisors and jobs receive as the exit reason, worded as it always
/// was; `diagnostic` is what the panic hook, the test runner, or an
/// embedder reports.
pub(crate) struct MeshPanic {
    reason: String,
    diagnostic: Diagnostic,
}

impl MeshPanic {
    /// The Mesh panic `payload` is, if it is one.
    pub fn of(payload: &(dyn Any + Send)) -> Option<&MeshPanic> {
        payload.downcast_ref::<MeshPanic>()
    }

    /// `Mesh panic: <message>`.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }
}

/// Unwind with `diagnostic`. Its message was redacted already: inside a
/// `Plaintext.map` it is [`WITHHELD`].
fn unwind(reason: String, diagnostic: Diagnostic) -> ! {
    std::panic::panic_any(MeshPanic { reason, diagnostic })
}

/// `message`, or `None` when the panic comes from a computation on
/// plaintext: the message may carry what the content made of it, and where
/// the panic was raised may be a branch the content chose, so both are
/// withheld. Every Mesh panic passes its message through here before it
/// builds a reason or a diagnostic.
fn disclosed(message: impl FnOnce() -> String) -> Option<String> {
    (!computing_on_plaintext()).then(message)
}

/// Unwind with the panic of a computation on plaintext: [`WITHHELD`], at
/// no place.
fn withheld() -> ! {
    unwind(
        format!("Mesh panic: {WITHHELD}"),
        Diagnostic::new(Code::Panic, WITHHELD),
    )
}

/// Raise `message` at 1-based `line` and `col` of `file`; at no place for
/// an empty `file` or line 0.
fn raise_at(message: impl FnOnce() -> String, file: &str, line: u32, col: u32) -> ! {
    let Some(message) = disclosed(message) else {
        withheld()
    };
    unwind(
        format!("Mesh panic: {message}"),
        located(Code::Panic, file, line, col, message),
    )
}

/// `code` at 1-based `line` and `col` of `file`; a location compiled code
/// does not know (an empty file, line 0) is left out.
pub(crate) fn located(code: Code, file: &str, line: u32, col: u32, message: String) -> Diagnostic {
    if file.is_empty() || line == 0 {
        return Diagnostic::new(code, message);
    }
    let diagnostic = Diagnostic::at_line(code, SourceFile::unread(file), line, message);
    if col == 0 {
        diagnostic
    } else {
        diagnostic.at_column(col)
    }
}

/// # Safety
///
/// `ptr` must point to `len` valid UTF-8 bytes.
unsafe fn text<'a>(ptr: *const u8, len: u64) -> &'a str {
    std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len as usize))
}

/// Panic with `msg`, raised in the function `file` names.
///
/// The ABI of programs compiled before [`mesh_panic_at`]: they pass the
/// function's name where a file would be, and line 0, so neither is read
/// as a location. In actor contexts the panic is caught by `catch_unwind`
/// for crash isolation; outside actors it ends the process.
///
/// # Safety
///
/// `msg` must point to `msg_len` valid UTF-8 bytes.
/// `file` must point to `file_len` valid UTF-8 bytes.
#[no_mangle]
pub extern "C-unwind" fn mesh_panic(
    msg: *const u8,
    msg_len: u64,
    file: *const u8,
    file_len: u64,
    _line: u32,
) -> ! {
    let (msg, function) = unsafe { (text(msg, msg_len), text(file, file_len)) };
    let Some(message) = disclosed(|| msg.to_string()) else {
        withheld()
    };
    unwind(
        format!("Mesh panic in {function}: {message}"),
        Diagnostic::new(Code::Panic, message).with_note(format!("raised in `{function}`")),
    )
}

/// Panic with `msg` at 1-based `line` and `col` of the source `file`, as
/// compiled code raises a panic whose place it knows (a division by zero,
/// a match no arm accepts).
///
/// # Safety
///
/// `msg` must point to `msg_len` valid UTF-8 bytes.
/// `file` must point to `file_len` valid UTF-8 bytes.
#[no_mangle]
pub extern "C-unwind" fn mesh_panic_at(
    msg: *const u8,
    msg_len: u64,
    file: *const u8,
    file_len: u64,
    line: u32,
    col: u32,
) -> ! {
    let (msg, file) = unsafe { (text(msg, msg_len), text(file, file_len)) };
    raise_at(|| msg.to_string(), file, line, col)
}

/// Raise a Mesh panic from runtime code: a runtime function was given a
/// value it cannot handle (`List.get` past the end). It unwinds as
/// `mesh_panic` does, so an actor crashes alone and a test fails alone; the
/// runtime function raising it must be `extern "C-unwind"`.
pub(crate) fn raise(message: std::fmt::Arguments<'_>) -> ! {
    raise_at(|| message.to_string(), "", 0, 0)
}

/// What a panic inside `Plaintext.map` says instead of its message, which
/// may carry what the content made of it (`List.get` names its index).
const WITHHELD: &str = "a computation on plaintext failed; its message is withheld";

thread_local! {
    /// How many `Plaintext.map` computations this thread is inside.
    static PLAINTEXT_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Compiled code entered a `Plaintext.map`.
#[no_mangle]
pub extern "C" fn mesh_plaintext_enter() {
    PLAINTEXT_DEPTH.with(|depth| depth.set(depth.get() + 1));
}

/// Compiled code left a `Plaintext.map`.
#[no_mangle]
pub extern "C" fn mesh_plaintext_leave() {
    PLAINTEXT_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
}

/// Whether the panic being raised comes from a computation on plaintext.
/// The panic unwinds out of all of them, so it leaves them.
fn computing_on_plaintext() -> bool {
    PLAINTEXT_DEPTH.with(|depth| depth.replace(0) > 0)
}

/// `panic(message)` at no known place: end the actor, or the program with
/// status 101.
///
/// # Safety
///
/// `message` must point to a live `MeshString`.
#[no_mangle]
pub extern "C-unwind" fn mesh_panic_str(message: *const crate::string::MeshString) -> ! {
    raise(format_args!("{}", unsafe { (*message).as_str() }))
}

/// `panic(message)` at 1-based `line` and `col` of the source `file`.
///
/// # Safety
///
/// `message` must point to a live `MeshString`, and `file` to `file_len`
/// valid UTF-8 bytes.
#[no_mangle]
pub extern "C-unwind" fn mesh_panic_str_at(
    message: *const crate::string::MeshString,
    file: *const u8,
    file_len: u64,
    line: u32,
    col: u32,
) -> ! {
    let file = unsafe { text(file, file_len) };
    raise_at(
        || unsafe { (*message).as_str() }.to_string(),
        file,
        line,
        col,
    )
}

thread_local! {
    /// Set while the test runner runs a test body: it reports the panic.
    static QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DEFER_DIAGNOSTICS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Report grouped actor failures without writing to stderr on their worker.
/// The closure catches the actor's unwind before returning.
pub(crate) fn defer_diagnostics<T>(f: impl FnOnce() -> T) -> T {
    let previous = DEFER_DIAGNOSTICS.replace(true);
    let result = f();
    DEFER_DIAGNOSTICS.set(previous);
    result
}

enum Queued {
    Text(String),
    Flush(crossbeam_channel::Sender<()>),
}

static DIAGNOSTICS: std::sync::OnceLock<crossbeam_channel::Sender<Queued>> =
    std::sync::OnceLock::new();

fn enqueue_diagnostic(text: String) {
    let writer = DIAGNOSTICS.get_or_init(|| {
        let (sender, receiver) = crossbeam_channel::bounded(16);
        let _ = std::thread::Builder::new()
            .name("mesh-diagnostics".to_string())
            .spawn(move || {
                while let Ok(queued) = receiver.recv() {
                    match queued {
                        Queued::Text(text) => write_stderr(text.as_bytes()),
                        Queued::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            });
        sender
    });
    let _ = writer.try_send(Queued::Text(text));
}

pub(crate) fn flush_deferred_diagnostics() {
    if let Some(writer) = DIAGNOSTICS.get() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20);
        let (done, receiver) = crossbeam_channel::bounded(1);
        if writer.send_deadline(Queued::Flush(done), deadline).is_ok() {
            let _ = receiver.recv_deadline(deadline);
        }
    }
}

/// One write per diagnostic keeps its lines together. A closed stderr
/// leaves nowhere to say that writing failed.
fn write_stderr(text: &[u8]) {
    let _ = std::io::stderr().write_all(text);
}

/// How the program reports: JSON when `MESH_DIAGNOSTICS=json`, otherwise
/// for a person, in color on a terminal without `NO_COLOR`.
pub(crate) fn format() -> Format {
    static FORMAT: std::sync::OnceLock<Format> = std::sync::OnceLock::new();
    *FORMAT.get_or_init(Format::for_runtime)
}

/// `diagnostic` as `format` writes it: a person's lines with the footer
/// for its code, or one JSON object and a newline.
pub(crate) fn render_as(format: Format, diagnostic: &Diagnostic) -> String {
    rendered(format, diagnostic, true)
}

fn rendered(format: Format, diagnostic: &Diagnostic, footer: bool) -> String {
    let mut out = Vec::new();
    let reporter = Reporter::new(format, &mut out);
    let mut reporter = if footer {
        reporter
    } else {
        reporter.without_footers()
    };
    reporter.emit(diagnostic.clone());
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether this process reports `code` for the first time: a person is
/// shown its footer once, as one reporter shows it.
fn first_report_of(code: Code) -> bool {
    static REPORTED: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashSet<Code>>> =
        std::sync::OnceLock::new();
    REPORTED.get_or_init(Default::default).lock().insert(code)
}

/// Report `diagnostic` on standard error, in the program's [`format`]; on a
/// grouped actor's worker, through the writer thread.
pub(crate) fn report(diagnostic: &Diagnostic) {
    let format = format();
    let footer = matches!(format, Format::Human { .. }) && first_report_of(diagnostic.code());
    let text = rendered(format, diagnostic, footer);
    if DEFER_DIAGNOSTICS.get() {
        enqueue_diagnostic(text);
    } else {
        write_stderr(text.as_bytes());
    }
}

/// Report a Mesh panic as its diagnostic: the default report's thread name
/// and runtime source location say nothing about the Mesh program. Any
/// other panic is a runtime bug and keeps the default report.
pub(crate) fn install_hook() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| match MeshPanic::of(info.payload()) {
            Some(_) if QUIET.with(|quiet| quiet.get()) => {}
            Some(panic) => report(&panic.diagnostic),
            None => default(info),
        }));
    });
}

/// Run `f` with Mesh panics left for the caller to report.
pub(crate) fn quietly<T>(f: impl FnOnce() -> T) -> T {
    let previous = QUIET.with(|quiet| quiet.replace(true));
    let result = f();
    QUIET.with(|quiet| quiet.set(previous));
    result
}

/// Run `f` for an embedder that goes on after a panic (the REPL): a panic
/// out of it is returned as its diagnostic for the caller to report, an
/// `R0001` for one that is not a Mesh panic (the runtime reported it).
pub fn catch_panic<T>(f: impl FnOnce() -> T) -> Result<T, Diagnostic> {
    quietly(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))).map_err(|payload| {
        match MeshPanic::of(&*payload) {
            Some(panic) => panic.diagnostic.clone(),
            None => Diagnostic::new(Code::Panic, "the runtime failed (reported above)"),
        }
    })
}

/// The exit reason of an actor that `payload` unwound out of, for its
/// links, monitors and supervisor. A Mesh panic was reported as it was
/// raised, and so was a failed assertion; any other panic is the runtime
/// failing inside the actor, reported here as `R0101`.
pub(crate) fn actor_exit_reason(payload: &(dyn Any + Send)) -> String {
    if let Some(panic) = MeshPanic::of(payload) {
        return panic.reason.clone();
    }
    if payload.is::<crate::test::TestFailed>() {
        return "assertion failed".to_string();
    }
    report(&Diagnostic::new(
        Code::ActorCrashed,
        "an actor crashed: the runtime failed (reported above)",
    ));
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "actor panicked".into())
}

/// Run the program's `main` function on the main thread.
///
/// A Mesh panic there ends the process with status 101 once the panic hook
/// has printed it. Without a handler to catch it the unwinder cannot start,
/// and the process aborted with "failed to initiate panic" instead.
///
/// Only executables come through here, so this is also where SIGPIPE is
/// ignored: a library must not change its host's signal handling.
#[no_mangle]
pub extern "C" fn mesh_run_main(entry: extern "C-unwind" fn()) {
    ignore_sigpipe();
    if std::panic::catch_unwind(|| entry()).is_err() {
        std::process::exit(101);
    }
}

/// Ignore SIGPIPE, as Rust programs do: a write to a peer that hung up then
/// fails with EPIPE instead of killing the process. A Mesh executable's `main`
/// is not Rust's, so the runtime has to do it; an HTTP server died with the
/// first client that closed its connection early.
pub(crate) fn ignore_sigpipe() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

/// End the process as an unhandled SIGPIPE would, once stdout is gone.
pub(crate) fn die_of_sigpipe() -> ! {
    crate::process_signal::write_coverage_profile();
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        libc::raise(libc::SIGPIPE);
    }
    std::process::exit(141)
}

/// The exit reason of the Mesh panic `f` raises, for tests of runtime
/// functions that raise one.
#[cfg(test)]
pub(crate) fn raised(f: impl FnOnce()) -> String {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .expect_err("a Mesh panic was raised");
    MeshPanic::of(&*payload)
        .expect("the panic is a Mesh panic")
        .reason()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_failures_report_catalog_codes() {
        const CHILD: &str = "MESH_RUNTIME_FAILURE_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert_eq!(actor_exit_reason(&"unexpected unwind"), "unexpected unwind");
            let target = "remote@127.0.0.1:1";
            assert_eq!(
                crate::dist::node::mesh_node_connect(target.as_ptr(), target.len() as u64),
                -1
            );
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "panic::tests::runtime_failures_report_catalog_codes",
            ])
            .env(CHILD, "1")
            .env("MESH_DIAGNOSTICS", "json")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let codes: Vec<_> = stderr
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["code"].clone())
            .collect();
        assert_eq!(codes, ["R0101", "R0301"], "{stderr}");
    }

    const SECRET: &str = "twenty-seven characters!!!!";

    /// The payload of the panic `raise` unwinds with, reported nowhere.
    fn caught(raise: impl FnOnce()) -> Box<dyn Any + Send> {
        install_hook();
        quietly(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(raise)))
            .expect_err("it panics")
    }

    /// Every format the runtime renders `panic` in.
    fn rendered(panic: &MeshPanic) -> Vec<String> {
        [Format::Json, Format::Human { color: false }]
            .into_iter()
            .map(|format| render_as(format, panic.diagnostic()))
            .chain([panic.reason().to_string()])
            .collect()
    }

    /// A panic inside `Plaintext.map` is redacted before its diagnostic is
    /// built, so neither the exit reason nor any rendering of its `R0001`
    /// shows what the content made of the message, or where it was raised:
    /// a branch on the content picks the place.
    #[test]
    fn a_panic_on_plaintext_withholds_its_message_and_place_from_every_format() {
        crate::gc::mesh_rt_init();
        let message = format!("List.get: index {} is out of bounds", SECRET.len());
        let shown = crate::string::mesh_string_new(message.as_ptr(), message.len() as u64)
            as *const crate::string::MeshString;
        let file = "secret_branch.mpl";
        let function = "Main__secret_branch";
        let raises: [Box<dyn Fn()>; 4] = [
            Box::new(|| raise(format_args!("{message}"))),
            Box::new(|| {
                mesh_panic_at(
                    message.as_ptr(),
                    message.len() as u64,
                    file.as_ptr(),
                    file.len() as u64,
                    41,
                    39,
                )
            }),
            Box::new(|| mesh_panic_str_at(shown, file.as_ptr(), file.len() as u64, 41, 39)),
            Box::new(|| {
                mesh_panic(
                    message.as_ptr(),
                    message.len() as u64,
                    function.as_ptr(),
                    function.len() as u64,
                    0,
                )
            }),
        ];
        for raise in raises {
            mesh_plaintext_enter();
            let payload = caught(raise);
            let panic = MeshPanic::of(&*payload).expect("a Mesh panic");
            let diagnostic = panic.diagnostic();
            assert_eq!(diagnostic.code(), Code::Panic);
            assert_eq!(
                (diagnostic.file(), diagnostic.line(), diagnostic.col()),
                (None, None, None)
            );
            assert!(diagnostic.notes().is_empty(), "{:?}", diagnostic.notes());
            for text in rendered(panic) {
                assert!(!text.contains("27"), "the index leaked: {text}");
                assert!(!text.contains("secret_branch"), "the place leaked: {text}");
                assert!(!text.contains("41"), "the line leaked: {text}");
                assert!(text.contains(WITHHELD), "{text}");
            }
            assert!(!computing_on_plaintext(), "the panic left the computation");
        }
    }

    /// `panic(message)` compiled where its call is known is an `R0001` at
    /// that call.
    #[test]
    fn a_panic_call_is_an_r0001_at_the_call() {
        crate::gc::mesh_rt_init();
        let message = "boom";
        let boom = crate::string::mesh_string_new(message.as_ptr(), message.len() as u64)
            as *const crate::string::MeshString;
        let payload = caught(|| mesh_panic_str_at(boom, "src/main.mpl".as_ptr(), 12, 3, 5));
        let panic = MeshPanic::of(&*payload).unwrap();
        assert_eq!(panic.reason(), "Mesh panic: boom");
        let json: serde_json::Value =
            serde_json::from_str(&render_as(Format::Json, panic.diagnostic())).unwrap();
        assert_eq!(json["code"], "R0001");
        assert_eq!(json["message"], "boom");
        assert_eq!(json["file"], "src/main.mpl");
        assert_eq!(
            (json["line"].as_u64(), json["col"].as_u64()),
            (Some(3), Some(5))
        );
    }

    /// A process reports many diagnostics, each through a fresh reporter:
    /// a person still sees the footer of a code once.
    #[test]
    fn a_codes_footer_is_shown_once_per_process() {
        const CHILD: &str = "MESH_RUNTIME_FOOTER_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            for crash in ["one", "two", "three"] {
                report(&Diagnostic::new(Code::Panic, crash));
            }
            report(&Diagnostic::new(Code::ActorCrashed, "four"));
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "panic::tests::a_codes_footer_is_shown_once_per_process",
            ])
            .env(CHILD, "1")
            .env_remove("MESH_DIAGNOSTICS")
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        for crash in ["one", "two", "three", "four"] {
            assert!(stderr.contains(crash), "{stderr}");
        }
        assert_eq!(stderr.matches("meshc explain R0001").count(), 1, "{stderr}");
        assert_eq!(stderr.matches("meshc explain R0101").count(), 1, "{stderr}");
    }

    /// Outside `Plaintext.map` the message is the diagnostic's, and a
    /// panic compiled code places names its file, line and column.
    #[test]
    fn a_placed_panic_is_an_r0001_at_its_line_and_column() {
        let message = "division by zero";
        let payload = caught(|| {
            mesh_panic_at(
                message.as_ptr(),
                message.len() as u64,
                "src/math.mpl".as_ptr(),
                12,
                7,
                14,
            )
        });
        let panic = MeshPanic::of(&*payload).unwrap();
        assert_eq!(panic.reason(), "Mesh panic: division by zero");
        let json: serde_json::Value =
            serde_json::from_str(&render_as(Format::Json, panic.diagnostic())).unwrap();
        assert_eq!(json["code"], "R0001");
        assert_eq!(json["message"], message);
        assert_eq!(json["file"], "src/math.mpl");
        assert_eq!(
            (json["line"].as_u64(), json["col"].as_u64()),
            (Some(7), Some(14))
        );
        let human = render_as(Format::Human { color: false }, panic.diagnostic());
        assert!(
            human.starts_with("[R0001] Error: division by zero\n   --> src/math.mpl:7:14\n"),
            "{human}"
        );
    }

    /// The ABI before `mesh_panic_at` passes a function's name where the
    /// file would be: it is a note, never a location.
    #[test]
    fn a_legacy_panic_names_its_function_not_a_file() {
        let payload = caught(|| mesh_panic("no match".as_ptr(), 8, "Main__f".as_ptr(), 7, 3));
        let panic = MeshPanic::of(&*payload).unwrap();
        assert_eq!(panic.reason(), "Mesh panic in Main__f: no match");
        assert_eq!(panic.diagnostic().file(), None);
        assert_eq!(panic.diagnostic().line(), None);
        assert_eq!(panic.diagnostic().notes(), ["raised in `Main__f`"]);
    }

    /// An embedder gets the diagnostic of a Mesh panic, and an `R0001` for
    /// any other.
    #[test]
    fn catch_panic_returns_the_diagnostic() {
        install_hook();
        let mesh = catch_panic(|| raise(format_args!("boom"))).unwrap_err();
        assert_eq!((mesh.code(), mesh.message()), (Code::Panic, "boom"));
        assert_eq!(catch_panic(|| 7).unwrap(), 7);
    }
}
