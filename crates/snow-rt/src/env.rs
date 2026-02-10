//! Environment variable and CLI argument access for the Snow standard library.
//!
//! Provides `Env.get(key)` and `Env.args()` for Snow programs.

use crate::gc::snow_gc_alloc_actor;
use crate::option::{SnowOption, alloc_option};
use crate::string::{snow_string_new, SnowString};

/// Get an environment variable by key. Returns SnowOption:
/// - tag 0, value = SnowString if the variable exists (Some)
/// - tag 1, value = null if the variable does not exist (None)
#[no_mangle]
pub extern "C" fn snow_env_get(key: *const SnowString) -> *mut SnowOption {
    unsafe {
        let key_str = (*key).as_str();
        match std::env::var(key_str) {
            Ok(val) => {
                let s = snow_string_new(val.as_ptr(), val.len() as u64);
                alloc_option(0, s as *mut u8)
            }
            Err(_) => alloc_option(1, std::ptr::null_mut()),
        }
    }
}

/// Return CLI arguments as a packed array of SnowString pointers.
///
/// Layout: `[u64 count, *mut SnowString arg0, *mut SnowString arg1, ...]`
///
/// This temporary representation will be replaced by proper List<String>
/// in Plan 02 when the List type is implemented.
#[no_mangle]
pub extern "C" fn snow_env_args() -> *mut u8 {
    let args: Vec<String> = std::env::args().collect();
    let count = args.len();
    // Layout: u64 count + count * pointer-sized entries
    let ptr_size = std::mem::size_of::<*mut SnowString>();
    let total_size = std::mem::size_of::<u64>() + count * ptr_size;

    unsafe {
        let buf = snow_gc_alloc_actor(total_size as u64, 8);
        // Write count
        *(buf as *mut u64) = count as u64;
        // Write string pointers
        let ptrs = buf.add(std::mem::size_of::<u64>()) as *mut *mut SnowString;
        for (i, arg) in args.iter().enumerate() {
            let s = snow_string_new(arg.as_ptr(), arg.len() as u64);
            *ptrs.add(i) = s;
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;
    use crate::string::SnowString;

    #[test]
    fn test_env_get_existing() {
        snow_rt_init();
        // PATH is almost always set
        let key = snow_string_new(b"PATH".as_ptr(), 4);
        let result = snow_env_get(key);
        unsafe {
            assert_eq!((*result).tag, 0, "PATH should exist");
            let value = (*result).value as *const SnowString;
            assert!(!value.is_null());
            assert!((*value).as_str().len() > 0, "PATH should be non-empty");
        }
    }

    #[test]
    fn test_env_get_missing() {
        snow_rt_init();
        let key = snow_string_new(
            b"SNOW_NONEXISTENT_VAR_12345".as_ptr(),
            25,
        );
        let result = snow_env_get(key);
        unsafe {
            assert_eq!((*result).tag, 1, "missing var should return None");
        }
    }

    #[test]
    fn test_env_args() {
        snow_rt_init();
        let buf = snow_env_args();
        unsafe {
            let count = *(buf as *const u64);
            // There should be at least 1 arg (the test binary itself)
            assert!(count >= 1, "expected at least 1 arg, got {}", count);
        }
    }
}
