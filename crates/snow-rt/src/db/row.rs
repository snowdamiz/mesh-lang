//! Row parsing runtime functions for struct-to-row mapping.
//!
//! These functions are called by generated MIR code to extract column values
//! from query result maps (Map<String, String>) and parse them into Snow
//! primitive types. Each returns a SnowResult (tag 0 = Ok, tag 1 = Err).
//!
//! ## Functions
//!
//! - `snow_row_from_row_get`: Extract a column value from a row map by name
//! - `snow_row_parse_int`: Parse a string to Int (i64)
//! - `snow_row_parse_float`: Parse a string to Float (f64)
//! - `snow_row_parse_bool`: Parse a string to Bool (0 or 1)

use crate::collections::map::{snow_map_get, snow_map_has_key};
use crate::io::alloc_result;
use crate::string::{snow_string_new, SnowString};

/// Extract a column value from a row map (Map<String, String>).
///
/// # Signature
///
/// `snow_row_from_row_get(row: *mut u8, col_name: *mut u8) -> *mut u8 (SnowResult)`
///
/// - `row` is a pointer to a SnowMap (string-keyed).
/// - `col_name` is a `*const SnowString` cast to `*mut u8`.
///
/// Returns Ok(value_string_ptr) if the key exists, or Err("missing column: {name}").
#[no_mangle]
pub extern "C" fn snow_row_from_row_get(row: *mut u8, col_name: *mut u8) -> *mut u8 {
    unsafe {
        let key = col_name as u64;
        if snow_map_has_key(row, key) != 0 {
            let val = snow_map_get(row, key);
            alloc_result(0, val as *mut u8) as *mut u8
        } else {
            // Build descriptive error message
            let name_str = (*(col_name as *const SnowString)).as_str();
            let msg = format!("missing column: {}", name_str);
            let err_snow = snow_string_new(msg.as_ptr(), msg.len() as u64);
            alloc_result(1, err_snow as *mut u8) as *mut u8
        }
    }
}

/// Parse a string value to an Int (i64).
///
/// # Signature
///
/// `snow_row_parse_int(s: *mut u8) -> *mut u8 (SnowResult)`
///
/// Trims the input string and parses as i64.
/// Returns Ok(value_as_i64) or Err("cannot parse '{text}' as Int").
#[no_mangle]
pub extern "C" fn snow_row_parse_int(s: *mut u8) -> *mut u8 {
    unsafe {
        let text = (*(s as *const SnowString)).as_str().trim();
        match text.parse::<i64>() {
            Ok(val) => alloc_result(0, val as *mut u8) as *mut u8,
            Err(_) => {
                let msg = format!("cannot parse '{}' as Int", text);
                let err_snow = snow_string_new(msg.as_ptr(), msg.len() as u64);
                alloc_result(1, err_snow as *mut u8) as *mut u8
            }
        }
    }
}

/// Parse a string value to a Float (f64).
///
/// # Signature
///
/// `snow_row_parse_float(s: *mut u8) -> *mut u8 (SnowResult)`
///
/// Pre-normalizes PostgreSQL-specific representations:
/// - "Infinity" -> "inf"
/// - "-Infinity" -> "-inf"
///
/// Returns Ok(f64::to_bits(value)) or Err("cannot parse '{text}' as Float").
/// Uses `f64::to_bits()` for float-to-u64 encoding, matching Snow Float convention.
#[no_mangle]
pub extern "C" fn snow_row_parse_float(s: *mut u8) -> *mut u8 {
    unsafe {
        let raw = (*(s as *const SnowString)).as_str().trim();
        // Pre-normalize PostgreSQL-specific infinity representations
        let text = match raw {
            "Infinity" => "inf",
            "-Infinity" => "-inf",
            other => other,
        };
        match text.parse::<f64>() {
            Ok(val) => alloc_result(0, f64::to_bits(val) as *mut u8) as *mut u8,
            Err(_) => {
                let msg = format!("cannot parse '{}' as Float", raw);
                let err_snow = snow_string_new(msg.as_ptr(), msg.len() as u64);
                alloc_result(1, err_snow as *mut u8) as *mut u8
            }
        }
    }
}

/// Parse a string value to a Bool (0 or 1).
///
/// # Signature
///
/// `snow_row_parse_bool(s: *mut u8) -> *mut u8 (SnowResult)`
///
/// Accepts PostgreSQL text-format booleans and common variants:
/// - true: "true", "t", "1", "yes"
/// - false: "false", "f", "0", "no"
///
/// Returns Ok(1) for true, Ok(0) for false, or Err("cannot parse '{text}' as Bool").
#[no_mangle]
pub extern "C" fn snow_row_parse_bool(s: *mut u8) -> *mut u8 {
    unsafe {
        let raw = (*(s as *const SnowString)).as_str().trim();
        let lower = raw.to_lowercase();
        match lower.as_str() {
            "true" | "t" | "1" | "yes" => alloc_result(0, 1i64 as *mut u8) as *mut u8,
            "false" | "f" | "0" | "no" => alloc_result(0, 0i64 as *mut u8) as *mut u8,
            _ => {
                let msg = format!("cannot parse '{}' as Bool", raw);
                let err_snow = snow_string_new(msg.as_ptr(), msg.len() as u64);
                alloc_result(1, err_snow as *mut u8) as *mut u8
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;
    use crate::io::SnowResult;

    /// Helper: create a SnowString from a &str and return as *mut u8.
    fn make_snow_string(s: &str) -> *mut u8 {
        snow_string_new(s.as_ptr(), s.len() as u64) as *mut u8
    }

    /// Helper: read the SnowResult tag from a result pointer.
    unsafe fn result_tag(r: *mut u8) -> u8 {
        (*(r as *const SnowResult)).tag
    }

    /// Helper: read the SnowResult value as i64.
    unsafe fn result_value_i64(r: *mut u8) -> i64 {
        (*(r as *const SnowResult)).value as i64
    }

    /// Helper: read the SnowResult value as a SnowString.
    unsafe fn result_value_str(r: *mut u8) -> &'static str {
        let val = (*(r as *const SnowResult)).value;
        (*(val as *const SnowString)).as_str()
    }

    // ── parse_int tests ──────────────────────────────────────────────────

    #[test]
    fn test_parse_int_positive() {
        snow_rt_init();
        let s = make_snow_string("42");
        let r = snow_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), 42);
        }
    }

    #[test]
    fn test_parse_int_negative() {
        snow_rt_init();
        let s = make_snow_string("-17");
        let r = snow_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), -17);
        }
    }

    #[test]
    fn test_parse_int_zero() {
        snow_rt_init();
        let s = make_snow_string("0");
        let r = snow_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), 0);
        }
    }

    #[test]
    fn test_parse_int_failure() {
        snow_rt_init();
        let s = make_snow_string("hello");
        let r = snow_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 1);
            let msg = result_value_str(r);
            assert!(msg.contains("cannot parse"), "got: {}", msg);
            assert!(msg.contains("hello"), "got: {}", msg);
        }
    }

    #[test]
    fn test_parse_int_with_whitespace() {
        snow_rt_init();
        let s = make_snow_string("  123  ");
        let r = snow_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), 123);
        }
    }

    // ── parse_float tests ────────────────────────────────────────────────

    #[test]
    fn test_parse_float_normal() {
        snow_rt_init();
        let s = make_snow_string("3.14");
        let r = snow_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            let bits = result_value_i64(r) as u64;
            let val = f64::from_bits(bits);
            assert!((val - 3.14).abs() < 1e-10, "got: {}", val);
        }
    }

    #[test]
    fn test_parse_float_pg_infinity() {
        snow_rt_init();
        let s = make_snow_string("Infinity");
        let r = snow_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            let bits = result_value_i64(r) as u64;
            let val = f64::from_bits(bits);
            assert!(val.is_infinite() && val.is_sign_positive(), "got: {}", val);
        }
    }

    #[test]
    fn test_parse_float_pg_neg_infinity() {
        snow_rt_init();
        let s = make_snow_string("-Infinity");
        let r = snow_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            let bits = result_value_i64(r) as u64;
            let val = f64::from_bits(bits);
            assert!(val.is_infinite() && val.is_sign_negative(), "got: {}", val);
        }
    }

    #[test]
    fn test_parse_float_failure() {
        snow_rt_init();
        let s = make_snow_string("abc");
        let r = snow_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 1);
            let msg = result_value_str(r);
            assert!(msg.contains("cannot parse"), "got: {}", msg);
        }
    }

    // ── parse_bool tests ─────────────────────────────────────────────────

    #[test]
    fn test_parse_bool_true_variants() {
        snow_rt_init();
        for input in &["true", "t", "1", "yes"] {
            let s = make_snow_string(input);
            let r = snow_row_parse_bool(s);
            unsafe {
                assert_eq!(result_tag(r), 0, "failed for input: {}", input);
                assert_eq!(result_value_i64(r), 1, "failed for input: {}", input);
            }
        }
    }

    #[test]
    fn test_parse_bool_false_variants() {
        snow_rt_init();
        for input in &["false", "f", "0", "no"] {
            let s = make_snow_string(input);
            let r = snow_row_parse_bool(s);
            unsafe {
                assert_eq!(result_tag(r), 0, "failed for input: {}", input);
                assert_eq!(result_value_i64(r), 0, "failed for input: {}", input);
            }
        }
    }

    #[test]
    fn test_parse_bool_case_insensitive() {
        snow_rt_init();
        for input in &["TRUE", "True", "FALSE", "False", "T", "F", "YES", "NO"] {
            let s = make_snow_string(input);
            let r = snow_row_parse_bool(s);
            unsafe {
                assert_eq!(result_tag(r), 0, "failed for input: {}", input);
            }
        }
    }

    #[test]
    fn test_parse_bool_failure() {
        snow_rt_init();
        let s = make_snow_string("maybe");
        let r = snow_row_parse_bool(s);
        unsafe {
            assert_eq!(result_tag(r), 1);
            let msg = result_value_str(r);
            assert!(msg.contains("cannot parse"), "got: {}", msg);
            assert!(msg.contains("maybe"), "got: {}", msg);
        }
    }
}
