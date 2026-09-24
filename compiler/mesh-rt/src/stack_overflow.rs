//! Report a stack overflow instead of dying with a bare segmentation fault.
//!
//! Deep non-tail recursion runs off the end of the thread's or the actor's
//! stack into its guard page. The fault handler runs on a small alternate
//! stack and, when the faulting address is at the current stack's limit,
//! prints what happened and aborts. Any other fault goes to the default
//! action, as before.

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

/// Set up reporting for the calling thread: an alternate signal stack, and
/// the handler (installed once per process).
pub fn install_for_current_thread() {
    #[cfg(unix)]
    unsafe {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
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
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    unsafe {
        let thread = libc::pthread_self();
        let top = libc::pthread_get_stackaddr_np(thread) as usize;
        top.saturating_sub(libc::pthread_get_stacksize_np(thread))
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return 0;
        }
        let mut lowest: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let found = libc::pthread_attr_getstack(&attr, &mut lowest, &mut size) == 0;
        libc::pthread_attr_destroy(&mut attr);
        if found {
            lowest as usize
        } else {
            0
        }
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    )))]
    {
        0
    }
}

#[cfg(unix)]
extern "C" fn handler(
    signal: libc::c_int,
    info: *mut libc::siginfo_t,
    _context: *mut libc::c_void,
) {
    unsafe {
        let address = (*info).si_addr() as usize;
        let limit = CURRENT_LIMIT.with(|c| c.get());
        let overflow = limit != 0
            && address >= limit.saturating_sub(GUARD_WINDOW)
            && address < limit + GUARD_WINDOW;
        if overflow {
            let message: &[u8] = b"\nerror: stack overflow: a function recursed too deeply \
(only a call in tail position runs in constant stack)\n";
            libc::write(2, message.as_ptr().cast(), message.len());
            libc::abort();
        }
        // Not a stack overflow: take the default action, as without the
        // handler (the faulting instruction runs again and faults).
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}
