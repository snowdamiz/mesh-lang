//! Runtime panic handler for Mesh programs.
//!
//! Called when a Mesh program encounters an unrecoverable error at runtime,
//! such as a non-exhaustive match failure (guarded arms edge case).
//!
//! Uses `panic!()` rather than `abort()` so that actor crash isolation
//! via `catch_unwind` can intercept handler failures without bringing
//! down the entire process.

/// Panic with a source-located error message.
///
/// Triggers a Rust `panic!()` with a formatted source-located error
/// message. In actor contexts, this is caught by `catch_unwind` for
/// crash isolation. Outside actors, the panic unwinds and terminates
/// the process as usual.
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
    line: u32,
) -> ! {
    unsafe {
        let msg = std::str::from_utf8_unchecked(std::slice::from_raw_parts(msg, msg_len as usize));
        let file =
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(file, file_len as usize));
        if line == 0 {
            // No source line: `file` names the function that panicked.
            panic!("Mesh panic in {}: {}", file, msg);
        }
        panic!("Mesh panic at {}:{}: {}", file, line, msg);
    }
}

/// Raise a Mesh panic from runtime code: a runtime function was given a
/// value it cannot handle (`List.get` past the end). It unwinds as
/// `mesh_panic` does, so an actor crashes alone; the runtime function
/// raising it must be `extern "C-unwind"`.
pub(crate) fn raise(message: std::fmt::Arguments<'_>) -> ! {
    panic!("Mesh panic: {message}")
}

/// Print a Mesh panic as its message alone: the default report's thread
/// name and runtime source location say nothing about the Mesh program.
/// Any other panic is a runtime bug and keeps the default report.
pub(crate) fn install_hook() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            match mesh_panic_message(info.payload()) {
                Some(message) => {
                    use std::io::Write;
                    let _ = writeln!(std::io::stderr(), "{message}");
                }
                None => default(info),
            }
        }));
    });
}

/// The message of a Mesh panic; `None` for any other panic payload.
pub(crate) fn mesh_panic_message(payload: &(dyn std::any::Any + Send)) -> Option<&str> {
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())?;
    message.starts_with("Mesh panic").then_some(message)
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
fn ignore_sigpipe() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

/// End the process as an unhandled SIGPIPE would, once stdout is gone.
pub(crate) fn die_of_sigpipe() -> ! {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        libc::raise(libc::SIGPIPE);
    }
    std::process::exit(141)
}
