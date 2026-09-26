//! Async-signal-safe shutdown notification for containerized applications.

use std::sync::atomic::{AtomicBool, Ordering};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(unix)]
extern "C" fn request_shutdown_from_signal(_signal: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

/// Coverage builds (`--cfg mesh_coverage`) write the coverage profile when a
/// program is told to stop: test harnesses and `docker stop` end servers with
/// SIGTERM, and a process a signal ends runs no exit hook to write it. A
/// program that handles the signals itself replaces this and exits the
/// ordinary way, which writes it.
#[cfg(all(mesh_coverage, unix))]
pub(crate) fn install_coverage_flush() {
    extern "C" {
        fn __llvm_profile_write_file() -> libc::c_int;
    }
    extern "C" fn flush_and_exit(signal: libc::c_int) {
        unsafe {
            __llvm_profile_write_file();
            libc::_exit(128 + signal);
        }
    }
    unsafe {
        let handler = flush_and_exit as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

#[no_mangle]
pub extern "C" fn mesh_process_install_shutdown_signals() {
    #[cfg(unix)]
    unsafe {
        let handler = request_shutdown_from_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

#[no_mangle]
pub extern "C" fn mesh_process_shutdown_requested() -> i8 {
    shutdown_requested() as i8
}

#[no_mangle]
pub extern "C" fn mesh_process_request_shutdown() {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

#[no_mangle]
pub extern "C" fn mesh_process_exit(code: i64) -> ! {
    let code = if (0..=255).contains(&code) {
        code as i32
    } else {
        1
    };
    std::process::exit(code)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn signal_handler_only_sets_the_shutdown_flag() {
        SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
        request_shutdown_from_signal(libc::SIGTERM);
        assert_eq!(mesh_process_shutdown_requested(), 1);
        SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    }
}
