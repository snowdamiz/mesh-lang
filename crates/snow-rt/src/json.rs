//! JSON encoding and decoding runtime functions for the Snow standard library.
//!
//! Uses `serde_json` for parsing and serialization. Provides:
//! - `snow_json_parse`: parse a JSON string into a SnowJson tagged union
//! - `snow_json_encode`: convert a SnowJson back to a JSON string
//! - Convenience encode functions for primitives and collections
//! - ToJSON/FromJSON support functions for building Json values from Snow types
//!
//! ## SnowJson representation
//!
//! At the runtime level, JSON values are represented as GC-allocated tagged unions:
//! ```text
//! SnowJson { tag: u8, value: u64 }
//! ```
//! Tags: 0=Null, 1=Bool, 2=Number(i64), 3=Str(*SnowString), 4=Array(*SnowList), 5=Object(*SnowMap)

use crate::collections::list;
use crate::collections::map;
use crate::gc::snow_gc_alloc_actor;
use crate::io::SnowResult;
use crate::string::{snow_string_new, SnowString};

/// Tag constants for SnowJson variants.
const JSON_NULL: u8 = 0;
const JSON_BOOL: u8 = 1;
const JSON_NUMBER: u8 = 2;
const JSON_STR: u8 = 3;
const JSON_ARRAY: u8 = 4;
const JSON_OBJECT: u8 = 5;

/// GC-allocated JSON value.
///
/// Layout: `{ tag: u8, _pad: [u8; 7], value: u64 }` -- 16 bytes total.
/// The padding ensures 8-byte alignment for the value field.
#[repr(C)]
pub struct SnowJson {
    pub tag: u8,
    _pad: [u8; 7],
    pub value: u64,
}

// ── Allocation helpers ──────────────────────────────────────────────

fn alloc_json(tag: u8, value: u64) -> *mut SnowJson {
    unsafe {
        let ptr = snow_gc_alloc_actor(
            std::mem::size_of::<SnowJson>() as u64,
            std::mem::align_of::<SnowJson>() as u64,
        ) as *mut SnowJson;
        (*ptr).tag = tag;
        (*ptr)._pad = [0; 7];
        (*ptr).value = value;
        ptr
    }
}

fn alloc_result(tag: u8, value: *mut u8) -> *mut SnowResult {
    unsafe {
        let ptr = snow_gc_alloc_actor(
            std::mem::size_of::<SnowResult>() as u64,
            std::mem::align_of::<SnowResult>() as u64,
        ) as *mut SnowResult;
        (*ptr).tag = tag;
        (*ptr).value = value;
        ptr
    }
}

fn err_result(msg: &str) -> *mut SnowResult {
    let s = snow_string_new(msg.as_ptr(), msg.len() as u64);
    alloc_result(1, s as *mut u8)
}

// ── Conversion: serde_json::Value -> SnowJson ──────────────────────

/// Recursively convert a serde_json::Value to a GC-allocated SnowJson.
fn serde_value_to_snow_json(val: &serde_json::Value) -> *mut SnowJson {
    match val {
        serde_json::Value::Null => alloc_json(JSON_NULL, 0),
        serde_json::Value::Bool(b) => {
            alloc_json(JSON_BOOL, if *b { 1 } else { 0 })
        }
        serde_json::Value::Number(n) => {
            // Store as i64 if it fits, otherwise as f64 bits.
            if let Some(i) = n.as_i64() {
                alloc_json(JSON_NUMBER, i as u64)
            } else if let Some(f) = n.as_f64() {
                alloc_json(JSON_NUMBER, f.to_bits())
            } else {
                alloc_json(JSON_NUMBER, 0)
            }
        }
        serde_json::Value::String(s) => {
            let snow_str = snow_string_new(s.as_ptr(), s.len() as u64);
            alloc_json(JSON_STR, snow_str as u64)
        }
        serde_json::Value::Array(arr) => {
            // Build a SnowList from the array elements.
            let mut snow_list = list::snow_list_new();
            for item in arr {
                let json_ptr = serde_value_to_snow_json(item);
                snow_list = list::snow_list_append(snow_list, json_ptr as u64);
            }
            alloc_json(JSON_ARRAY, snow_list as u64)
        }
        serde_json::Value::Object(obj) => {
            // Build a SnowMap from the object entries.
            // Keys are stored as SnowString pointers (as u64), values as SnowJson pointers (as u64).
            let mut snow_map = map::snow_map_new();
            for (key, val) in obj {
                let key_str = snow_string_new(key.as_ptr(), key.len() as u64);
                let val_json = serde_value_to_snow_json(val);
                snow_map = map::snow_map_put(snow_map, key_str as u64, val_json as u64);
            }
            alloc_json(JSON_OBJECT, snow_map as u64)
        }
    }
}

// ── Conversion: SnowJson -> serde_json::Value ──────────────────────

/// Recursively convert a SnowJson to a serde_json::Value for encoding.
unsafe fn snow_json_to_serde_value(json: *const SnowJson) -> serde_json::Value {
    match (*json).tag {
        JSON_NULL => serde_json::Value::Null,
        JSON_BOOL => {
            serde_json::Value::Bool((*json).value != 0)
        }
        JSON_NUMBER => {
            // Stored as i64; try to represent as integer first.
            let raw = (*json).value;
            let ival = raw as i64;
            // Heuristic: if the value round-trips as i64, use integer.
            // This works for typical JSON integers.
            serde_json::Value::Number(serde_json::Number::from(ival))
        }
        JSON_STR => {
            let s = (*json).value as *const SnowString;
            let text = (*s).as_str().to_string();
            serde_json::Value::String(text)
        }
        JSON_ARRAY => {
            let list_ptr = (*json).value as *mut u8;
            let len = list::snow_list_length(list_ptr);
            let mut arr = Vec::with_capacity(len as usize);
            for i in 0..len {
                let elem = list::snow_list_get(list_ptr, i);
                let elem_json = elem as *const SnowJson;
                arr.push(snow_json_to_serde_value(elem_json));
            }
            serde_json::Value::Array(arr)
        }
        JSON_OBJECT => {
            let map_ptr = (*json).value as *mut u8;
            let keys_list = map::snow_map_keys(map_ptr);
            let vals_list = map::snow_map_values(map_ptr);
            let len = list::snow_list_length(keys_list);
            let mut obj = serde_json::Map::new();
            for i in 0..len {
                let key_ptr = list::snow_list_get(keys_list, i) as *const SnowString;
                let val_ptr = list::snow_list_get(vals_list, i) as *const SnowJson;
                let key_str = (*key_ptr).as_str().to_string();
                let val_json = snow_json_to_serde_value(val_ptr);
                obj.insert(key_str, val_json);
            }
            serde_json::Value::Object(obj)
        }
        _ => serde_json::Value::Null,
    }
}

// ── Public API: Parse ───────────────────────────────────────────────

/// Parse a JSON string into a SnowJson value.
///
/// Returns SnowResult:
/// - tag 0 (Ok): value = pointer to SnowJson
/// - tag 1 (Err): value = pointer to SnowString error message
#[no_mangle]
pub extern "C" fn snow_json_parse(input: *const SnowString) -> *mut SnowResult {
    unsafe {
        let text = (*input).as_str();
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(val) => {
                let json = serde_value_to_snow_json(&val);
                alloc_result(0, json as *mut u8)
            }
            Err(e) => err_result(&e.to_string()),
        }
    }
}

// ── Public API: Encode ──────────────────────────────────────────────

/// Encode a SnowJson value to a JSON string.
#[no_mangle]
pub extern "C" fn snow_json_encode(json: *mut u8) -> *mut SnowString {
    unsafe {
        let json_ptr = json as *const SnowJson;
        let val = snow_json_to_serde_value(json_ptr);
        let text = serde_json::to_string(&val).unwrap_or_else(|_| "null".to_string());
        snow_string_new(text.as_ptr(), text.len() as u64)
    }
}

// ── Convenience encode functions ────────────────────────────────────

/// Encode a Snow string directly to a JSON string (with quotes).
#[no_mangle]
pub extern "C" fn snow_json_encode_string(s: *const SnowString) -> *mut SnowString {
    unsafe {
        let text = (*s).as_str();
        let val = serde_json::Value::String(text.to_string());
        let json_text = serde_json::to_string(&val).unwrap_or_else(|_| "null".to_string());
        snow_string_new(json_text.as_ptr(), json_text.len() as u64)
    }
}

/// Encode an integer to a JSON string.
#[no_mangle]
pub extern "C" fn snow_json_encode_int(val: i64) -> *mut SnowString {
    let text = val.to_string();
    snow_string_new(text.as_ptr(), text.len() as u64)
}

/// Encode a boolean to a JSON string.
#[no_mangle]
pub extern "C" fn snow_json_encode_bool(val: i8) -> *mut SnowString {
    let text = if val != 0 { "true" } else { "false" };
    snow_string_new(text.as_ptr(), text.len() as u64)
}

/// Encode a SnowMap to a JSON string.
///
/// Assumes map keys are SnowString pointers and values are SnowString pointers.
/// Produces a JSON object like `{"key1":"val1","key2":"val2"}`.
#[no_mangle]
pub extern "C" fn snow_json_encode_map(map_ptr: *mut u8) -> *mut SnowString {
    unsafe {
        let keys = map::snow_map_keys(map_ptr);
        let vals = map::snow_map_values(map_ptr);
        let len = list::snow_list_length(keys);
        let mut obj = serde_json::Map::new();
        for i in 0..len {
            let key = list::snow_list_get(keys, i) as *const SnowString;
            let val = list::snow_list_get(vals, i) as *const SnowString;
            let key_str = (*key).as_str().to_string();
            let val_str = (*val).as_str().to_string();
            obj.insert(key_str, serde_json::Value::String(val_str));
        }
        let text = serde_json::to_string(&serde_json::Value::Object(obj))
            .unwrap_or_else(|_| "{}".to_string());
        snow_string_new(text.as_ptr(), text.len() as u64)
    }
}

/// Encode a SnowList of strings to a JSON array string.
///
/// Assumes list elements are SnowString pointers.
/// Produces a JSON array like `["a","b","c"]`.
#[no_mangle]
pub extern "C" fn snow_json_encode_list(list_ptr: *mut u8) -> *mut SnowString {
    unsafe {
        let len = list::snow_list_length(list_ptr);
        let mut arr = Vec::with_capacity(len as usize);
        for i in 0..len {
            let elem = list::snow_list_get(list_ptr, i) as *const SnowString;
            let text = (*elem).as_str().to_string();
            arr.push(serde_json::Value::String(text));
        }
        let text = serde_json::to_string(&serde_json::Value::Array(arr))
            .unwrap_or_else(|_| "[]".to_string());
        snow_string_new(text.as_ptr(), text.len() as u64)
    }
}

// ── ToJSON/FromJSON support ─────────────────────────────────────────

/// Create a SnowJson Number from an i64.
#[no_mangle]
pub extern "C" fn snow_json_from_int(val: i64) -> *mut u8 {
    alloc_json(JSON_NUMBER, val as u64) as *mut u8
}

/// Create a SnowJson Number from an f64.
#[no_mangle]
pub extern "C" fn snow_json_from_float(val: f64) -> *mut u8 {
    alloc_json(JSON_NUMBER, val.to_bits()) as *mut u8
}

/// Create a SnowJson Bool from an i8 (0 = false, non-zero = true).
#[no_mangle]
pub extern "C" fn snow_json_from_bool(val: i8) -> *mut u8 {
    alloc_json(JSON_BOOL, if val != 0 { 1 } else { 0 }) as *mut u8
}

/// Create a SnowJson Str from a SnowString.
#[no_mangle]
pub extern "C" fn snow_json_from_string(s: *const SnowString) -> *mut u8 {
    alloc_json(JSON_STR, s as u64) as *mut u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    fn make_string(s: &str) -> *const SnowString {
        snow_string_new(s.as_ptr(), s.len() as u64)
    }

    #[test]
    fn test_json_parse_object() {
        snow_rt_init();
        let input = make_string(r#"{"name":"Snow","version":1}"#);
        let result = snow_json_parse(input);
        unsafe {
            assert_eq!((*result).tag, 0, "parse should succeed");
            let json = (*result).value as *const SnowJson;
            assert_eq!((*json).tag, JSON_OBJECT, "should be an object");
        }
    }

    #[test]
    fn test_json_parse_array() {
        snow_rt_init();
        let input = make_string(r#"[1, 2, 3]"#);
        let result = snow_json_parse(input);
        unsafe {
            assert_eq!((*result).tag, 0, "parse should succeed");
            let json = (*result).value as *const SnowJson;
            assert_eq!((*json).tag, JSON_ARRAY, "should be an array");
        }
    }

    #[test]
    fn test_json_parse_primitives() {
        snow_rt_init();

        // null
        let result = snow_json_parse(make_string("null"));
        unsafe {
            assert_eq!((*result).tag, 0);
            let json = (*result).value as *const SnowJson;
            assert_eq!((*json).tag, JSON_NULL);
        }

        // boolean
        let result = snow_json_parse(make_string("true"));
        unsafe {
            assert_eq!((*result).tag, 0);
            let json = (*result).value as *const SnowJson;
            assert_eq!((*json).tag, JSON_BOOL);
            assert_eq!((*json).value, 1);
        }

        // number
        let result = snow_json_parse(make_string("42"));
        unsafe {
            assert_eq!((*result).tag, 0);
            let json = (*result).value as *const SnowJson;
            assert_eq!((*json).tag, JSON_NUMBER);
            assert_eq!((*json).value as i64, 42);
        }

        // string
        let result = snow_json_parse(make_string(r#""hello""#));
        unsafe {
            assert_eq!((*result).tag, 0);
            let json = (*result).value as *const SnowJson;
            assert_eq!((*json).tag, JSON_STR);
            let s = (*json).value as *const SnowString;
            assert_eq!((*s).as_str(), "hello");
        }
    }

    #[test]
    fn test_json_parse_invalid() {
        snow_rt_init();
        let input = make_string("{invalid json}");
        let result = snow_json_parse(input);
        unsafe {
            assert_eq!((*result).tag, 1, "parse should fail");
            let err_msg = (*result).value as *const SnowString;
            assert!(!err_msg.is_null());
            // Should contain some error message
            let msg = (*err_msg).as_str();
            assert!(!msg.is_empty(), "error message should not be empty");
        }
    }

    #[test]
    fn test_json_encode_roundtrip() {
        snow_rt_init();
        let input = make_string(r#"{"a":1,"b":"hello","c":true}"#);
        let result = snow_json_parse(input);
        unsafe {
            assert_eq!((*result).tag, 0);
            let json = (*result).value as *mut u8;
            let encoded = snow_json_encode(json);
            let text = (*encoded).as_str();
            // Parse again to verify valid JSON
            let reparsed: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(reparsed["a"], 1);
            assert_eq!(reparsed["b"], "hello");
            assert_eq!(reparsed["c"], true);
        }
    }

    #[test]
    fn test_json_encode_string() {
        snow_rt_init();
        let s = make_string("hello world");
        let encoded = snow_json_encode_string(s);
        unsafe {
            assert_eq!((*encoded).as_str(), r#""hello world""#);
        }
    }

    #[test]
    fn test_json_encode_int() {
        snow_rt_init();
        let encoded = snow_json_encode_int(42);
        unsafe {
            assert_eq!((*encoded).as_str(), "42");
        }
    }

    #[test]
    fn test_json_encode_bool() {
        snow_rt_init();
        let t = snow_json_encode_bool(1);
        let f = snow_json_encode_bool(0);
        unsafe {
            assert_eq!((*t).as_str(), "true");
            assert_eq!((*f).as_str(), "false");
        }
    }

    #[test]
    fn test_json_encode_map() {
        snow_rt_init();
        let key1 = make_string("name");
        let val1 = make_string("Snow");
        let key2 = make_string("lang");
        let val2 = make_string("rust");

        let mut m = map::snow_map_new();
        m = map::snow_map_put(m, key1 as u64, val1 as u64);
        m = map::snow_map_put(m, key2 as u64, val2 as u64);

        let encoded = snow_json_encode_map(m);
        unsafe {
            let text = (*encoded).as_str();
            let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(parsed["name"], "Snow");
            assert_eq!(parsed["lang"], "rust");
        }
    }

    #[test]
    fn test_json_from_int() {
        snow_rt_init();
        let json = snow_json_from_int(99) as *const SnowJson;
        unsafe {
            assert_eq!((*json).tag, JSON_NUMBER);
            assert_eq!((*json).value as i64, 99);
        }
    }

    #[test]
    fn test_json_from_bool() {
        snow_rt_init();
        let json_true = snow_json_from_bool(1) as *const SnowJson;
        let json_false = snow_json_from_bool(0) as *const SnowJson;
        unsafe {
            assert_eq!((*json_true).tag, JSON_BOOL);
            assert_eq!((*json_true).value, 1);
            assert_eq!((*json_false).tag, JSON_BOOL);
            assert_eq!((*json_false).value, 0);
        }
    }

    #[test]
    fn test_json_from_string() {
        snow_rt_init();
        let s = make_string("hello");
        let json = snow_json_from_string(s) as *const SnowJson;
        unsafe {
            assert_eq!((*json).tag, JSON_STR);
            let str_ptr = (*json).value as *const SnowString;
            assert_eq!((*str_ptr).as_str(), "hello");
        }
    }

    #[test]
    fn test_json_encode_list() {
        snow_rt_init();
        let s1 = make_string("a");
        let s2 = make_string("b");
        let s3 = make_string("c");

        let mut l = list::snow_list_new();
        l = list::snow_list_append(l, s1 as u64);
        l = list::snow_list_append(l, s2 as u64);
        l = list::snow_list_append(l, s3 as u64);

        let encoded = snow_json_encode_list(l);
        unsafe {
            let text = (*encoded).as_str();
            assert_eq!(text, r#"["a","b","c"]"#);
        }
    }
}
