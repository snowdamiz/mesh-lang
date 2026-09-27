//! Calling a Mesh function value the runtime was handed as a `(fn, env)`
//! pair: the environment goes first, unless there is none. Compiled code
//! passes every callback through the uniform-slot adapter, a closure, so
//! its arguments and result are uniform slots (`u64`).

/// `fn(a)`, or `fn(env, a)` for a closure.
///
/// # Safety
///
/// `fn_ptr` must be a function taking `env_ptr` (when not null) and one
/// slot, and returning one.
pub(crate) unsafe fn call1(fn_ptr: *mut u8, env_ptr: *mut u8, a: u64) -> u64 {
    if env_ptr.is_null() {
        std::mem::transmute::<*mut u8, unsafe extern "C-unwind" fn(u64) -> u64>(fn_ptr)(a)
    } else {
        std::mem::transmute::<*mut u8, unsafe extern "C-unwind" fn(*mut u8, u64) -> u64>(fn_ptr)(
            env_ptr, a,
        )
    }
}

/// `fn(a, b)`, or `fn(env, a, b)` for a closure.
///
/// # Safety
///
/// As for [`call1`], with two slots.
pub(crate) unsafe fn call2(fn_ptr: *mut u8, env_ptr: *mut u8, a: u64, b: u64) -> u64 {
    if env_ptr.is_null() {
        std::mem::transmute::<*mut u8, unsafe extern "C-unwind" fn(u64, u64) -> u64>(fn_ptr)(a, b)
    } else {
        std::mem::transmute::<*mut u8, unsafe extern "C-unwind" fn(*mut u8, u64, u64) -> u64>(
            fn_ptr,
        )(env_ptr, a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C-unwind" fn double(a: u64) -> u64 {
        a * 2
    }

    extern "C-unwind" fn add_env(env: *mut u8, a: u64) -> u64 {
        a + unsafe { *(env as *const u64) }
    }

    extern "C-unwind" fn sub(a: u64, b: u64) -> u64 {
        a - b
    }

    extern "C-unwind" fn sub_env(env: *mut u8, a: u64, b: u64) -> u64 {
        a - b - unsafe { *(env as *const u64) }
    }

    #[test]
    fn a_callback_takes_its_environment_first_unless_it_has_none() {
        let mut ten = 10u64;
        let env = &mut ten as *mut u64 as *mut u8;
        unsafe {
            assert_eq!(call1(double as *mut u8, std::ptr::null_mut(), 4), 8);
            assert_eq!(call1(add_env as *mut u8, env, 4), 14);
            assert_eq!(call2(sub as *mut u8, std::ptr::null_mut(), 9, 4), 5);
            assert_eq!(call2(sub_env as *mut u8, env, 29, 4), 15);
        }
    }
}
