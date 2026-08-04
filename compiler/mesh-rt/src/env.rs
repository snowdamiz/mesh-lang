//! Environment variable and CLI argument access for the Mesh standard library.
//!
//! Provides public environment access, including direct secret-hex ingestion.

use zeroize::Zeroizing;

use crate::collections::list::mesh_list_from_array;
use crate::io::{alloc_result, MeshResult};
use crate::option::{alloc_option, MeshOption};
use crate::secret::{
    crypto_error, insert_owned_resource, CryptoErrorTag, ResourceError, ResourceKind,
    MAX_SECRET_BYTES,
};
use crate::string::{mesh_string_new, MeshString};

enum SecretHexError {
    Invalid,
    TooLong(usize),
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_secret_hex(encoded: &str) -> Result<Zeroizing<Box<[u8]>>, SecretHexError> {
    let decoded_length = encoded.len().saturating_add(1) / 2;
    if decoded_length > MAX_SECRET_BYTES {
        return Err(SecretHexError::TooLong(decoded_length));
    }
    if encoded.is_empty() || encoded.len() % 2 != 0 {
        return Err(SecretHexError::Invalid);
    }
    let mut decoded = Zeroizing::new(vec![0; decoded_length].into_boxed_slice());
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or(SecretHexError::Invalid)?;
        let low = hex_nibble(pair[1]).ok_or(SecretHexError::Invalid)?;
        decoded[index] = (high << 4) | low;
    }
    Ok(decoded)
}

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

/// Read a required hexadecimal environment value directly into actor-owned secret storage.
#[no_mangle]
pub extern "C" fn mesh_env_get_secret_hex(key: *const MeshString) -> *mut MeshResult {
    if key.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let key = unsafe { (*key).as_str() };
    if key.is_empty() || key.as_bytes().iter().any(|byte| matches!(byte, 0 | b'=')) {
        return crypto_error(CryptoErrorTag::InvalidKey, 0, 0);
    }
    let encoded = match std::env::var(key) {
        Ok(value) => Zeroizing::new(value),
        Err(_) => return crypto_error(CryptoErrorTag::InvalidKey, 0, 0),
    };
    let secret = match decode_secret_hex(&encoded) {
        Ok(secret) => secret,
        Err(SecretHexError::Invalid) => {
            return crypto_error(CryptoErrorTag::InvalidKey, 0, 0);
        }
        Err(SecretHexError::TooLong(actual)) => {
            return crypto_error(
                CryptoErrorTag::InvalidLength,
                MAX_SECRET_BYTES as i64,
                i64::try_from(actual).unwrap_or(i64::MAX),
            );
        }
    };
    let Some(pid) = crate::actor::stack::get_current_pid() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let Some(scheduler) = crate::actor::GLOBAL_SCHEDULER.get() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let Some(process) = scheduler.get_process(pid) else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let result = insert_owned_resource(&mut process.lock(), ResourceKind::SecretBytes, secret);
    match result {
        Ok(secret) => alloc_result(0, secret.cast()),
        Err(ResourceError::ResourceLimitExceeded) => {
            crypto_error(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
        Err(ResourceError::OwnerExited) => crypto_error(CryptoErrorTag::SecretDestroyed, 0, 0),
        Err(_) => crypto_error(CryptoErrorTag::InternalFailure, 0, 0),
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

    #[repr(C)]
    struct CryptoErrorLayout {
        tag: u8,
        _padding: [u8; 7],
        expected: i64,
        actual: i64,
    }

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
    fn secret_hex_rejects_oversized_values_with_a_typed_length_error() {
        mesh_rt_init();
        let name = "MESH_SECRET_HEX_OVERSIZED_TEST";
        std::env::set_var(name, "00".repeat(MAX_SECRET_BYTES + 1));
        let key = mesh_string_new(name.as_ptr(), name.len() as u64);

        let result = mesh_env_get_secret_hex(key);

        std::env::remove_var(name);
        let (result_tag, error) = unsafe {
            (
                (*result).tag,
                &*((*result).value as *const CryptoErrorLayout),
            )
        };
        assert_eq!(
            (result_tag, error.tag, error.expected, error.actual),
            (
                1,
                CryptoErrorTag::InvalidLength as u8,
                MAX_SECRET_BYTES as i64,
                MAX_SECRET_BYTES as i64 + 1,
            )
        );
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
