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

#[no_mangle]
pub extern "C" fn mesh_process_install_shutdown_signals() {
    #[cfg(unix)]
    unsafe {
        libc::signal(
            libc::SIGINT,
            request_shutdown_from_signal as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            request_shutdown_from_signal as libc::sighandler_t,
        );
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
