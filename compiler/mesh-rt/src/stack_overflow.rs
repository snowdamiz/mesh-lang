//! Report a stack overflow instead of dying with a bare segmentation fault.
//!
//! Deep non-tail recursion runs off the end of the thread's or the actor's
//! stack into its guard page. The fault handler runs on a small alternate
//! stack and, when the faulting address is at the current stack's limit,
//! writes the `R0002` diagnostic and aborts. Any other fault goes to the
//! default action, as before.

use std::cell::Cell;

thread_local! {
    /// Lowest usable address of the stack the thread is running on: its own,
    /// or the running actor's coroutine stack. Zero when unknown.
    static CURRENT_LIMIT: Cell<usize> = const { Cell::new(0) };
    /// Lowest usable address of the thread's own stack.
    static THREAD_LIMIT: Cell<usize> = const { Cell::new(0) };
}

/// How far from a stack's limit a fault still counts as running off it: a
/// frame larger than the guard page can land below it.
const GUARD_WINDOW: usize = 64 * 1024;

/// The `R0002` diagnostic, rendered as the handler is installed: the
/// handler may only `write` bytes that already exist.
#[cfg(unix)]
static REPORT: std::sync::OnceLock<Box<[u8]>> = std::sync::OnceLock::new();

#[cfg(unix)]
fn overflow_report() -> Box<[u8]> {
    let diagnostic = mesh_diagnostics::Diagnostic::new(
        mesh_diagnostics::Code::StackOverflow,
        "stack overflow: a function recursed too deeply (only a call in tail position runs in constant stack)",
    );
    crate::panic::render_as(crate::panic::format(), &diagnostic)
        .into_bytes()
        .into_boxed_slice()
}

/// Set up reporting for the calling thread: an alternate signal stack, and
/// the handler (installed once per process).
pub fn install_for_current_thread() {
    #[cfg(unix)]
    unsafe {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            REPORT.get_or_init(overflow_report);
            for signal in [libc::SIGSEGV, libc::SIGBUS] {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = handler as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(signal, &action, std::ptr::null_mut());
            }
        });
        let size = libc::SIGSTKSZ.max(64 * 1024);
        let stack = libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        if stack != libc::MAP_FAILED {
            let alt = libc::stack_t {
                ss_sp: stack,
                ss_flags: 0,
                ss_size: size,
            };
            libc::sigaltstack(&alt, std::ptr::null_mut());
        }
        let limit = thread_stack_limit();
        THREAD_LIMIT.with(|c| c.set(limit));
        CURRENT_LIMIT.with(|c| c.set(limit));
    }
}

/// Run `f` on a coroutine stack whose lowest address is `limit`.
pub fn on_stack<T>(limit: usize, f: impl FnOnce() -> T) -> T {
    let previous = CURRENT_LIMIT.with(|c| c.replace(limit));
    let result = f();
    CURRENT_LIMIT.with(|c| c.set(previous));
    result
}

/// Restore the thread's own stack limit (an actor unwound out of `on_stack`).
pub fn back_on_thread_stack() {
    let limit = THREAD_LIMIT.with(|c| c.get());
    CURRENT_LIMIT.with(|c| c.set(limit));
}

#[cfg(unix)]
fn thread_stack_limit() -> usize {
    crate::actor::stack::current_thread_stack_bounds().0
}

#[cfg(unix)]
extern "C" fn handler(
    signal: libc::c_int,
    info: *mut libc::siginfo_t,
    _context: *mut libc::c_void,
) {
    on_fault(signal, unsafe { (*info).si_addr() } as usize);
}

/// What a fault at `address` does: a stack overflow is reported and aborts
/// the process; any other fault takes the default action, as without the
/// handler (the faulting instruction runs again and faults). Only
/// async-signal-safe calls: no allocation, no lock.
#[cfg(unix)]
fn on_fault(signal: libc::c_int, address: usize) {
    let limit = CURRENT_LIMIT.with(|c| c.get());
    let overflow = limit != 0
        && address >= limit.saturating_sub(GUARD_WINDOW)
        && address < limit + GUARD_WINDOW;
    unsafe {
        if overflow {
            if let Some(report) = REPORT.get() {
                libc::write(2, report.as_ptr().cast(), report.len());
            }
            // A coverage build counts the process, which abort would lose.
            crate::process_signal::write_coverage_profile();
            libc::abort();
        }
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A fault that is not at the stack's limit gives the signal back to its
    /// default action.
    #[test]
    fn a_fault_away_from_the_stack_limit_takes_the_default_action() {
        let disposition = |signal| unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(signal, std::ptr::null(), &mut action);
            action
        };
        // SIGBUS, not SIGSEGV: nothing in the test process expects it.
        install_for_current_thread();
        let installed = disposition(libc::SIGBUS);
        assert_eq!(installed.sa_sigaction, handler as *const () as usize);

        on_fault(libc::SIGBUS, 16);

        assert_eq!(disposition(libc::SIGBUS).sa_sigaction, libc::SIG_DFL);
        unsafe { libc::sigaction(libc::SIGBUS, &installed, std::ptr::null_mut()) };
    }

    /// The handler writes a report rendered before any fault: the `R0002`
    /// diagnostic, ready as soon as the handler is.
    #[test]
    fn the_overflow_report_is_rendered_with_the_handler() {
        install_for_current_thread();
        let report = std::str::from_utf8(REPORT.get().expect("rendered at install")).unwrap();
        assert!(report.contains("R0002"), "{report}");
        assert!(report.contains("recursed too deeply"), "{report}");
        assert!(report.ends_with('\n'), "{report}");
    }
}
