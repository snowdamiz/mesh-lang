//! Environment variable and CLI argument access for the Mesh standard library.
//!
//! Provides `Env.get(key)` and `Env.args()` for Mesh programs.

use crate::collections::list::mesh_list_from_array;
use crate::option::{alloc_option, MeshOption};
use crate::string::{mesh_string_new, MeshString};

/// Get an environment variable by key. Returns MeshOption:
/// - tag 0, value = MeshString if the variable exists (Some)
/// - tag 1, value = null if the variable does not exist (None)
#[no_mangle]
pub extern "C" fn mesh_env_get(key: *const MeshString) -> *mut MeshOption {
    unsafe {
        let key_str = (*key).as_str();
        match std::env::var(key_str) {
            Ok(val) => {
                let s = mesh_string_new(val.as_ptr(), val.len() as u64);
                alloc_option(0, s as *mut u8)
            }
            Err(_) => alloc_option(1, std::ptr::null_mut()),
        }
    }
}

/// Get an environment variable by key, returning a default string if not set.
///
/// Returns the env var value as a MeshString pointer if set, or the provided
/// default MeshString pointer if the variable is not set.
#[no_mangle]
pub extern "C" fn mesh_env_get_with_default(
    key: *const MeshString,
    default: *const MeshString,
) -> *mut MeshString {
    unsafe {
        let key_str = (*key).as_str();
        match std::env::var(key_str) {
            Ok(val) => mesh_string_new(val.as_ptr(), val.len() as u64),
            Err(_) => default as *mut MeshString,
        }
    }
}

/// Get an environment variable by key and parse it as i64, returning a default if not set or invalid.
///
/// Returns the parsed integer value if the env var is set and parses successfully,
/// or the provided default if the variable is not set or cannot be parsed as i64.
#[no_mangle]
pub extern "C" fn mesh_env_get_int(key: *const MeshString, default: i64) -> i64 {
    let key_str = unsafe { (*key).as_str() };
    match std::env::var(key_str).ok() {
        Some(val) => val.parse::<i64>().unwrap_or(default),
        None => default,
    }
}

/// Return CLI arguments as a `List<String>`.
#[no_mangle]
pub extern "C" fn mesh_env_args() -> *mut u8 {
    let args = std::env::args()
        .map(|arg| mesh_string_new(arg.as_ptr(), arg.len() as u64) as u64)
        .collect::<Vec<_>>();
    mesh_list_from_array(args.as_ptr(), args.len() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;
    use crate::string::MeshString;

    #[test]
    fn test_env_get_existing() {
        mesh_rt_init();
        // PATH is almost always set
        let key = mesh_string_new(b"PATH".as_ptr(), 4);
        let result = mesh_env_get(key);
        unsafe {
            assert_eq!((*result).tag, 0, "PATH should exist");
            let value = (*result).value as *const MeshString;
            assert!(!value.is_null());
            assert!((*value).as_str().len() > 0, "PATH should be non-empty");
        }
    }

    #[test]
    fn test_env_get_missing() {
        mesh_rt_init();
        let key = mesh_string_new(b"MESH_NONEXISTENT_VAR_12345".as_ptr(), 25);
        let result = mesh_env_get(key);
        unsafe {
            assert_eq!((*result).tag, 1, "missing var should return None");
        }
    }

    #[test]
    fn test_env_get_with_default_missing() {
        mesh_rt_init();
        let key = mesh_string_new(b"MESH_NONEXISTENT_VAR_99999".as_ptr(), 25);
        let default = mesh_string_new(b"fallback".as_ptr(), 8);
        let result = mesh_env_get_with_default(key, default);
        unsafe {
            assert_eq!((*result).as_str(), "fallback");
        }
    }

    #[test]
    fn test_env_get_int_missing() {
        mesh_rt_init();
        let key = mesh_string_new(b"MESH_INT_NONEXISTENT_99999".as_ptr(), 25);
        let result = mesh_env_get_int(key, 8080);
        assert_eq!(result, 8080);
    }

    #[test]
    fn test_env_get_int_non_numeric() {
        mesh_rt_init();
        std::env::set_var("MESH_INT_NONNUMERIC_TEST", "not-a-number");
        let key = mesh_string_new(b"MESH_INT_NONNUMERIC_TEST".as_ptr(), 24);
        let result = mesh_env_get_int(key, 42);
        assert_eq!(result, 42);
        std::env::remove_var("MESH_INT_NONNUMERIC_TEST");
    }

    #[test]
    fn test_env_args() {
        mesh_rt_init();
        let args = mesh_env_args();
        let count = crate::collections::list::mesh_list_length(args);
        assert!(count >= 1, "expected at least 1 arg, got {count}");
        let first = crate::collections::list::mesh_list_get(args, 0) as *const MeshString;
        unsafe {
            assert!(!(*first).as_str().is_empty());
        }
    }
}
