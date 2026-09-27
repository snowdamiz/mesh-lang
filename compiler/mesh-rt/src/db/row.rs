//! Row parsing runtime functions for struct-to-row mapping.
//!
//! These functions are called by generated MIR code to extract column values
//! from query result maps (Map<String, String>) and parse them into Mesh
//! primitive types. Each returns a MeshResult (tag 0 = Ok, tag 1 = Err).
//!
//! ## Functions
//!
//! - `mesh_row_from_row_get`: Extract a column value from a row map by name
//! - `mesh_row_parse_int`: Parse a string to Int (i64)
//! - `mesh_row_parse_float`: Parse a string to Float (f64)
//! - `mesh_row_parse_bool`: Parse a string to Bool (0 or 1)

use crate::collections::map::{mesh_map_get, mesh_map_has_key};
use crate::io::{alloc_result, err_result};
use crate::string::text_of;

/// `Ok(payload)`, the payload unboxed as generated `from_row` code reads it.
fn ok(payload: *mut u8) -> *mut u8 {
    alloc_result(0, payload) as *mut u8
}

/// Extract a column value from a row map (Map<String, String>).
///
/// - `row` is a pointer to a MeshMap (string-keyed).
/// - `col_name` is a `*const MeshString` cast to `*mut u8`.
///
/// Returns Ok(value_string_ptr) if the key exists, or Err("missing column: {name}").
#[no_mangle]
pub extern "C" fn mesh_row_from_row_get(row: *mut u8, col_name: *mut u8) -> *mut u8 {
    unsafe {
        let key = col_name as u64;
        if mesh_map_has_key(row, key) != 0 {
            ok(mesh_map_get(row, key) as *mut u8)
        } else {
            err_result(&format!("missing column: {}", text_of(col_name)))
        }
    }
}

/// Parse a string value to an Int (i64).
///
/// Trims the input string and parses as i64.
/// Returns Ok(value_as_i64) or Err("cannot parse '{text}' as Int").
#[no_mangle]
pub extern "C" fn mesh_row_parse_int(s: *mut u8) -> *mut u8 {
    let text = unsafe { text_of(s) }.trim();
    match text.parse::<i64>() {
        Ok(val) => ok(val as *mut u8),
        Err(_) => err_result(&format!("cannot parse '{}' as Int", text)),
    }
}

/// Parse a string value to a Float (f64).
///
/// Pre-normalizes PostgreSQL-specific representations:
/// - "Infinity" -> "inf"
/// - "-Infinity" -> "-inf"
///
/// Returns Ok(f64::to_bits(value)) or Err("cannot parse '{text}' as Float").
/// Uses `f64::to_bits()` for float-to-u64 encoding, matching Mesh Float convention.
#[no_mangle]
pub extern "C" fn mesh_row_parse_float(s: *mut u8) -> *mut u8 {
    let raw = unsafe { text_of(s) }.trim();
    // Pre-normalize PostgreSQL-specific infinity representations
    let text = match raw {
        "Infinity" => "inf",
        "-Infinity" => "-inf",
        other => other,
    };
    match text.parse::<f64>() {
        Ok(val) => ok(f64::to_bits(val) as *mut u8),
        Err(_) => err_result(&format!("cannot parse '{}' as Float", raw)),
    }
}

/// Parse a string value to a Bool (0 or 1).
///
/// Accepts PostgreSQL text-format booleans and common variants:
/// - true: "true", "t", "1", "yes"
/// - false: "false", "f", "0", "no"
///
/// Returns Ok(1) for true, Ok(0) for false, or Err("cannot parse '{text}' as Bool").
#[no_mangle]
pub extern "C" fn mesh_row_parse_bool(s: *mut u8) -> *mut u8 {
    let raw = unsafe { text_of(s) }.trim();
    match raw.to_lowercase().as_str() {
        // A `u8` dangling pointer is the address 1: true.
        "true" | "t" | "1" | "yes" => ok(std::ptr::dangling_mut::<u8>()),
        "false" | "f" | "0" | "no" => ok(std::ptr::null_mut()),
        _ => err_result(&format!("cannot parse '{}' as Bool", raw)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;
    use crate::io::MeshResult;
    use crate::string::{mesh_str, MeshString};

    /// Helper: read the MeshResult tag from a result pointer.
    unsafe fn result_tag(r: *mut u8) -> u8 {
        (*(r as *const MeshResult)).tag
    }

    /// Helper: read the MeshResult value as i64.
    unsafe fn result_value_i64(r: *mut u8) -> i64 {
        (*(r as *const MeshResult)).value as i64
    }

    /// Helper: read the MeshResult value as a MeshString.
    unsafe fn result_value_str(r: *mut u8) -> &'static str {
        let val = (*(r as *const MeshResult)).value;
        (*(val as *const MeshString)).as_str()
    }

    #[test]
    fn a_column_is_read_or_named_as_missing() {
        mesh_rt_init();
        let row = crate::collections::map::mesh_map_put(
            crate::collections::map::mesh_map_new_typed(1),
            mesh_str("name") as u64,
            mesh_str("Ada") as u64,
        );
        unsafe {
            let found = mesh_row_from_row_get(row, mesh_str("name") as *mut u8);
            assert_eq!((result_tag(found), result_value_str(found)), (0, "Ada"));
            let missing = mesh_row_from_row_get(row, mesh_str("age") as *mut u8);
            assert_eq!(
                (result_tag(missing), result_value_str(missing)),
                (1, "missing column: age")
            );
        }
    }

    // ── parse_int tests ──────────────────────────────────────────────────

    #[test]
    fn test_parse_int_positive() {
        mesh_rt_init();
        let s = mesh_str("42") as *mut u8;
        let r = mesh_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), 42);
        }
    }

    #[test]
    fn test_parse_int_negative() {
        mesh_rt_init();
        let s = mesh_str("-17") as *mut u8;
        let r = mesh_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), -17);
        }
    }

    #[test]
    fn test_parse_int_zero() {
        mesh_rt_init();
        let s = mesh_str("0") as *mut u8;
        let r = mesh_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), 0);
        }
    }

    #[test]
    fn test_parse_int_failure() {
        mesh_rt_init();
        let s = mesh_str("hello") as *mut u8;
        let r = mesh_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 1);
            let msg = result_value_str(r);
            assert!(msg.contains("cannot parse"), "got: {}", msg);
            assert!(msg.contains("hello"), "got: {}", msg);
        }
    }

    #[test]
    fn test_parse_int_with_whitespace() {
        mesh_rt_init();
        let s = mesh_str("  123  ") as *mut u8;
        let r = mesh_row_parse_int(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            assert_eq!(result_value_i64(r), 123);
        }
    }

    // ── parse_float tests ────────────────────────────────────────────────

    #[test]
    fn test_parse_float_normal() {
        mesh_rt_init();
        let s = mesh_str("2.75") as *mut u8;
        let r = mesh_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            let bits = result_value_i64(r) as u64;
            let val = f64::from_bits(bits);
            assert!((val - 2.75).abs() < 1e-10, "got: {}", val);
        }
    }

    #[test]
    fn test_parse_float_pg_infinity() {
        mesh_rt_init();
        let s = mesh_str("Infinity") as *mut u8;
        let r = mesh_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            let bits = result_value_i64(r) as u64;
            let val = f64::from_bits(bits);
            assert!(val.is_infinite() && val.is_sign_positive(), "got: {}", val);
        }
    }

    #[test]
    fn test_parse_float_pg_neg_infinity() {
        mesh_rt_init();
        let s = mesh_str("-Infinity") as *mut u8;
        let r = mesh_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 0);
            let bits = result_value_i64(r) as u64;
            let val = f64::from_bits(bits);
            assert!(val.is_infinite() && val.is_sign_negative(), "got: {}", val);
        }
    }

    #[test]
    fn test_parse_float_failure() {
        mesh_rt_init();
        let s = mesh_str("abc") as *mut u8;
        let r = mesh_row_parse_float(s);
        unsafe {
            assert_eq!(result_tag(r), 1);
            let msg = result_value_str(r);
            assert!(msg.contains("cannot parse"), "got: {}", msg);
        }
    }

    // ── parse_bool tests ─────────────────────────────────────────────────

    #[test]
    fn test_parse_bool_true_variants() {
        mesh_rt_init();
        for input in &["true", "t", "1", "yes"] {
            let s = mesh_str(input) as *mut u8;
            let r = mesh_row_parse_bool(s);
            unsafe {
                assert_eq!(result_tag(r), 0, "failed for input: {}", input);
                assert_eq!(result_value_i64(r), 1, "failed for input: {}", input);
            }
        }
    }

    #[test]
    fn test_parse_bool_false_variants() {
        mesh_rt_init();
        for input in &["false", "f", "0", "no"] {
            let s = mesh_str(input) as *mut u8;
            let r = mesh_row_parse_bool(s);
            unsafe {
                assert_eq!(result_tag(r), 0, "failed for input: {}", input);
                assert_eq!(result_value_i64(r), 0, "failed for input: {}", input);
            }
        }
    }

    #[test]
    fn test_parse_bool_case_insensitive() {
        mesh_rt_init();
        for input in &["TRUE", "True", "FALSE", "False", "T", "F", "YES", "NO"] {
            let s = mesh_str(input) as *mut u8;
            let r = mesh_row_parse_bool(s);
            unsafe {
                assert_eq!(result_tag(r), 0, "failed for input: {}", input);
            }
        }
    }

    #[test]
    fn test_parse_bool_failure() {
        mesh_rt_init();
        let s = mesh_str("maybe") as *mut u8;
        let r = mesh_row_parse_bool(s);
        unsafe {
            assert_eq!(result_tag(r), 1);
            let msg = result_value_str(r);
            assert!(msg.contains("cannot parse"), "got: {}", msg);
            assert!(msg.contains("maybe"), "got: {}", msg);
        }
    }
}
