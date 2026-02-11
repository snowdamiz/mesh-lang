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
//! Tags: 0=Null, 1=Bool, 2=Int(i64), 3=Str(*SnowString), 4=Array(*SnowList), 5=Object(*SnowMap), 6=Float(f64)

use crate::collections::list;
use crate::collections::map;
use crate::gc::snow_gc_alloc_actor;
use crate::io::SnowResult;
use crate::string::{snow_string_new, SnowString};

/// Tag constants for SnowJson variants.
const JSON_NULL: u8 = 0;
const JSON_BOOL: u8 = 1;
const JSON_INT: u8 = 2;
const JSON_STR: u8 = 3;
const JSON_ARRAY: u8 = 4;
const JSON_OBJECT: u8 = 5;
const JSON_FLOAT: u8 = 6;

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
            // Int and Float are separate tags for round-trip fidelity.
            if let Some(i) = n.as_i64() {
                alloc_json(JSON_INT, i as u64)
            } else if let Some(f) = n.as_f64() {
                alloc_json(JSON_FLOAT, f.to_bits())
            } else {
                alloc_json(JSON_INT, 0)
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
            // Use typed map with KEY_TYPE_STR (1) so lookups use string content comparison.
            let mut snow_map = map::snow_map_new_typed(1);
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
        JSON_INT => {
            let ival = (*json).value as i64;
            serde_json::Value::Number(serde_json::Number::from(ival))
        }
        JSON_FLOAT => {
            let bits = (*json).value;
            let f = f64::from_bits(bits);
            serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
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

/// Create a SnowJson Int from an i64.
#[no_mangle]
pub extern "C" fn snow_json_from_int(val: i64) -> *mut u8 {
    alloc_json(JSON_INT, val as u64) as *mut u8
}

/// Create a SnowJson Float from an f64.
#[no_mangle]
pub extern "C" fn snow_json_from_float(val: f64) -> *mut u8 {
    alloc_json(JSON_FLOAT, val.to_bits()) as *mut u8
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

// ── Structured JSON object/array functions (Phase 49) ───────────────

/// Create an empty JSON object.
/// Uses string-typed map (KEY_TYPE_STR) so key lookups use content comparison.
#[no_mangle]
pub extern "C" fn snow_json_object_new() -> *mut u8 {
    let m = map::snow_map_new_typed(1);
    alloc_json(JSON_OBJECT, m as u64) as *mut u8
}

/// Add a key-value pair to a JSON object. Returns a new JSON object.
#[no_mangle]
pub extern "C" fn snow_json_object_put(obj: *mut u8, key: *mut u8, val: *mut u8) -> *mut u8 {
    unsafe {
        let j = obj as *mut SnowJson;
        let m = (*j).value as *mut u8;
        let new_map = map::snow_map_put(m, key as u64, val as u64);
        alloc_json(JSON_OBJECT, new_map as u64) as *mut u8
    }
}

/// Get a value from a JSON object by key. Returns SnowResult (Ok/Err).
#[no_mangle]
pub extern "C" fn snow_json_object_get(obj: *mut u8, key: *mut u8) -> *mut u8 {
    unsafe {
        let j = obj as *mut SnowJson;
        if (*j).tag != JSON_OBJECT {
            return err_result("expected Object") as *mut u8;
        }
        let m = (*j).value as *mut u8;
        if map::snow_map_has_key(m, key as u64) != 0 {
            let val = map::snow_map_get(m, key as u64);
            alloc_result(0, val as *mut u8) as *mut u8
        } else {
            let key_str = key as *const SnowString;
            err_result(&format!("missing field: {}", (*key_str).as_str())) as *mut u8
        }
    }
}

/// Create an empty JSON array.
#[no_mangle]
pub extern "C" fn snow_json_array_new() -> *mut u8 {
    let l = list::snow_list_new();
    alloc_json(JSON_ARRAY, l as u64) as *mut u8
}

/// Append a value to a JSON array. Returns a new JSON array.
#[no_mangle]
pub extern "C" fn snow_json_array_push(arr: *mut u8, val: *mut u8) -> *mut u8 {
    unsafe {
        let j = arr as *mut SnowJson;
        let l = (*j).value as *mut u8;
        let new_list = list::snow_list_append(l, val as u64);
        alloc_json(JSON_ARRAY, new_list as u64) as *mut u8
    }
}

/// Extract an Int from a SnowJson value. Returns SnowResult.
/// Coerces Float to Int if needed.
#[no_mangle]
pub extern "C" fn snow_json_as_int(json: *mut u8) -> *mut u8 {
    unsafe {
        let j = json as *mut SnowJson;
        match (*j).tag {
            JSON_INT => {
                alloc_result(0, (*j).value as i64 as *mut u8) as *mut u8
            }
            JSON_FLOAT => {
                let f = f64::from_bits((*j).value);
                alloc_result(0, f as i64 as *mut u8) as *mut u8
            }
            _ => err_result("expected Int") as *mut u8,
        }
    }
}

/// Extract a Float from a SnowJson value. Returns SnowResult.
/// Promotes Int to Float if needed.
#[no_mangle]
pub extern "C" fn snow_json_as_float(json: *mut u8) -> *mut u8 {
    unsafe {
        let j = json as *mut SnowJson;
        match (*j).tag {
            JSON_FLOAT => {
                alloc_result(0, (*j).value as *mut u8) as *mut u8
            }
            JSON_INT => {
                let i = (*j).value as i64;
                let f = (i as f64).to_bits();
                alloc_result(0, f as *mut u8) as *mut u8
            }
            _ => err_result("expected Float") as *mut u8,
        }
    }
}

/// Extract a String from a SnowJson value. Returns SnowResult.
#[no_mangle]
pub extern "C" fn snow_json_as_string(json: *mut u8) -> *mut u8 {
    unsafe {
        let j = json as *mut SnowJson;
        if (*j).tag == JSON_STR {
            alloc_result(0, (*j).value as *mut u8) as *mut u8
        } else {
            err_result("expected String") as *mut u8
        }
    }
}

/// Extract a Bool from a SnowJson value. Returns SnowResult.
#[no_mangle]
pub extern "C" fn snow_json_as_bool(json: *mut u8) -> *mut u8 {
    unsafe {
        let j = json as *mut SnowJson;
        if (*j).tag == JSON_BOOL {
            alloc_result(0, (*j).value as *mut u8) as *mut u8
        } else {
            err_result("expected Bool") as *mut u8
        }
    }
}

/// Return a SnowJson null value. Used for Option::None encoding.
#[no_mangle]
pub extern "C" fn snow_json_null() -> *mut u8 {
    alloc_json(JSON_NULL, 0) as *mut u8
}

/// Extract an element at the given index from a JSON array. Returns SnowResult.
/// Ok(element) on success, Err(message) if not an array or index out of bounds.
#[no_mangle]
pub extern "C" fn snow_json_array_get(json_arr: *mut u8, index: i64) -> *mut u8 {
    unsafe {
        let j = json_arr as *mut SnowJson;
        if (*j).tag != JSON_ARRAY {
            return err_result("expected Array") as *mut u8;
        }
        let inner_list = (*j).value as *mut u8;
        let len = list::snow_list_length(inner_list);
        if index < 0 || index >= len as i64 {
            return err_result(&format!(
                "array index {} out of bounds (length {})",
                index, len
            )) as *mut u8;
        }
        let elem = list::snow_list_get(inner_list, index);
        alloc_result(0, elem as *mut u8) as *mut u8
    }
}

// ── Collection helpers (Phase 49) ───────────────────────────────────

/// Convert a SnowList to a JSON array using a per-element callback.
/// `elem_fn` converts each list element (u64) to a *mut SnowJson.
#[no_mangle]
pub extern "C" fn snow_json_from_list(
    list_ptr: *mut u8,
    elem_fn: extern "C" fn(u64) -> *mut u8,
) -> *mut u8 {
    let len = list::snow_list_length(list_ptr);
    let mut arr = snow_json_array_new();
    for i in 0..len {
        let elem = list::snow_list_get(list_ptr, i);
        let json_elem = elem_fn(elem);
        arr = snow_json_array_push(arr, json_elem);
    }
    arr
}

/// Convert a SnowMap to a JSON object using a per-value callback.
/// Keys must be SnowString pointers. `val_fn` converts each value (u64) to a *mut SnowJson.
#[no_mangle]
pub extern "C" fn snow_json_from_map(
    map_ptr: *mut u8,
    val_fn: extern "C" fn(u64) -> *mut u8,
) -> *mut u8 {
    let keys_list = map::snow_map_keys(map_ptr);
    let vals_list = map::snow_map_values(map_ptr);
    let len = list::snow_list_length(keys_list);
    let mut obj = snow_json_object_new();
    for i in 0..len {
        let key = list::snow_list_get(keys_list, i);
        let val = list::snow_list_get(vals_list, i);
        let json_key = key as *mut u8; // SnowString pointer used as map key
        let json_val = val_fn(val);
        obj = snow_json_object_put(obj, json_key, json_val);
    }
    obj
}

/// Decode a JSON array into a SnowList using a per-element callback.
/// `elem_fn` converts each *mut SnowJson to a *mut SnowResult.
/// Returns SnowResult: Ok(SnowList) or Err on first element failure.
#[no_mangle]
pub extern "C" fn snow_json_to_list(
    json_arr: *mut u8,
    elem_fn: extern "C" fn(*mut u8) -> *mut u8,
) -> *mut u8 {
    unsafe {
        let j = json_arr as *mut SnowJson;
        if (*j).tag != JSON_ARRAY {
            return err_result("expected Array") as *mut u8;
        }
        let inner_list = (*j).value as *mut u8;
        let len = list::snow_list_length(inner_list);
        let mut result_list = list::snow_list_new();
        for i in 0..len {
            let elem = list::snow_list_get(inner_list, i);
            let decoded = elem_fn(elem as *mut u8);
            let res = decoded as *mut SnowResult;
            if (*res).tag != 0 {
                // Propagate error
                return decoded;
            }
            result_list = list::snow_list_append(result_list, (*res).value as u64);
        }
        alloc_result(0, result_list as *mut u8) as *mut u8
    }
}

/// Decode a JSON object into a SnowMap using a per-value callback.
/// Keys remain as SnowStrings. `val_fn` converts each *mut SnowJson to a *mut SnowResult.
/// Returns SnowResult: Ok(SnowMap) or Err on first value failure.
#[no_mangle]
pub extern "C" fn snow_json_to_map(
    json_obj: *mut u8,
    val_fn: extern "C" fn(*mut u8) -> *mut u8,
) -> *mut u8 {
    unsafe {
        let j = json_obj as *mut SnowJson;
        if (*j).tag != JSON_OBJECT {
            return err_result("expected Object") as *mut u8;
        }
        let inner_map = (*j).value as *mut u8;
        let keys_list = map::snow_map_keys(inner_map);
        let vals_list = map::snow_map_values(inner_map);
        let len = list::snow_list_length(keys_list);
        let mut result_map = map::snow_map_new();
        for i in 0..len {
            let key = list::snow_list_get(keys_list, i);
            let val = list::snow_list_get(vals_list, i);
            let decoded = val_fn(val as *mut u8);
            let res = decoded as *mut SnowResult;
            if (*res).tag != 0 {
                // Propagate error
                return decoded;
            }
            result_map = map::snow_map_put(result_map, key, (*res).value as u64);
        }
        alloc_result(0, result_map as *mut u8) as *mut u8
    }
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

        // number (integer)
        let result = snow_json_parse(make_string("42"));
        unsafe {
            assert_eq!((*result).tag, 0);
            let json = (*result).value as *const SnowJson;
            assert_eq!((*json).tag, JSON_INT);
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
            assert_eq!((*json).tag, JSON_INT);
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

    // ── Phase 49 structured JSON tests ──────────────────────────────

    #[test]
    fn test_json_object_new_put_get_roundtrip() {
        snow_rt_init();
        let mut obj = snow_json_object_new();
        let key = make_string("name") as *mut u8;
        let val = snow_json_from_string(make_string("Snow"));
        obj = snow_json_object_put(obj, key, val);

        // Get the value back
        let result = snow_json_object_get(obj, key);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0, "should be Ok");
            let got_json = (*res).value as *const SnowJson;
            assert_eq!((*got_json).tag, JSON_STR);
            let s = (*got_json).value as *const SnowString;
            assert_eq!((*s).as_str(), "Snow");
        }
    }

    #[test]
    fn test_json_object_get_missing_key() {
        snow_rt_init();
        let obj = snow_json_object_new();
        let key = make_string("nonexistent") as *mut u8;
        let result = snow_json_object_get(obj, key);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should be Err for missing key");
        }
    }

    #[test]
    fn test_json_array_new_push() {
        snow_rt_init();
        let mut arr = snow_json_array_new();
        arr = snow_json_array_push(arr, snow_json_from_int(1));
        arr = snow_json_array_push(arr, snow_json_from_int(2));

        // Encode and verify
        let encoded = snow_json_encode(arr);
        unsafe {
            let text = (*encoded).as_str();
            assert_eq!(text, "[1,2]");
        }
    }

    #[test]
    fn test_json_as_int_from_int() {
        snow_rt_init();
        let json = snow_json_from_int(42);
        let result = snow_json_as_int(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0);
            assert_eq!((*res).value as i64, 42);
        }
    }

    #[test]
    fn test_json_as_int_from_float() {
        snow_rt_init();
        let json = snow_json_from_float(3.7);
        let result = snow_json_as_int(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0);
            assert_eq!((*res).value as i64, 3); // truncated
        }
    }

    #[test]
    fn test_json_as_int_from_string_error() {
        snow_rt_init();
        let json = snow_json_from_string(make_string("hello"));
        let result = snow_json_as_int(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should be Err");
        }
    }

    #[test]
    fn test_json_as_float_from_float() {
        snow_rt_init();
        let json = snow_json_from_float(2.5);
        let result = snow_json_as_float(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0);
            let bits = (*res).value as u64;
            let f = f64::from_bits(bits);
            assert!((f - 2.5).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn test_json_as_float_from_int() {
        snow_rt_init();
        let json = snow_json_from_int(5);
        let result = snow_json_as_float(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0);
            let bits = (*res).value as u64;
            let f = f64::from_bits(bits);
            assert!((f - 5.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn test_json_as_string_happy() {
        snow_rt_init();
        let json = snow_json_from_string(make_string("world"));
        let result = snow_json_as_string(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0);
            let s = (*res).value as *const SnowString;
            assert_eq!((*s).as_str(), "world");
        }
    }

    #[test]
    fn test_json_as_string_error() {
        snow_rt_init();
        let json = snow_json_from_int(42);
        let result = snow_json_as_string(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should be Err for non-string");
        }
    }

    #[test]
    fn test_json_as_bool() {
        snow_rt_init();
        let json = snow_json_from_bool(1);
        let result = snow_json_as_bool(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0);
            assert_eq!((*res).value as u64, 1);
        }
    }

    #[test]
    fn test_json_as_bool_error() {
        snow_rt_init();
        let json = snow_json_from_int(1);
        let result = snow_json_as_bool(json);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should be Err for non-bool");
        }
    }

    #[test]
    fn test_json_null() {
        snow_rt_init();
        let json = snow_json_null() as *const SnowJson;
        unsafe {
            assert_eq!((*json).tag, JSON_NULL);
            assert_eq!((*json).value, 0);
        }
    }

    extern "C" fn int_to_json(val: u64) -> *mut u8 {
        snow_json_from_int(val as i64)
    }

    #[test]
    fn test_json_from_list() {
        snow_rt_init();
        let mut l = list::snow_list_new();
        l = list::snow_list_append(l, 10u64);
        l = list::snow_list_append(l, 20u64);
        l = list::snow_list_append(l, 30u64);

        let json_arr = snow_json_from_list(l, int_to_json);
        let encoded = snow_json_encode(json_arr);
        unsafe {
            assert_eq!((*encoded).as_str(), "[10,20,30]");
        }
    }

    extern "C" fn json_to_int_result(json: *mut u8) -> *mut u8 {
        snow_json_as_int(json)
    }

    #[test]
    fn test_json_to_list_roundtrip() {
        snow_rt_init();
        // Build a JSON array [1, 2, 3]
        let mut arr = snow_json_array_new();
        arr = snow_json_array_push(arr, snow_json_from_int(1));
        arr = snow_json_array_push(arr, snow_json_from_int(2));
        arr = snow_json_array_push(arr, snow_json_from_int(3));

        let result = snow_json_to_list(arr, json_to_int_result);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0, "should be Ok");
            let decoded_list = (*res).value as *mut u8;
            assert_eq!(list::snow_list_length(decoded_list), 3);
            assert_eq!(list::snow_list_get(decoded_list, 0) as i64, 1);
            assert_eq!(list::snow_list_get(decoded_list, 1) as i64, 2);
            assert_eq!(list::snow_list_get(decoded_list, 2) as i64, 3);
        }
    }

    #[test]
    fn test_json_to_list_error_propagation() {
        snow_rt_init();
        // Build a JSON array with a string element -- int decode should fail
        let mut arr = snow_json_array_new();
        arr = snow_json_array_push(arr, snow_json_from_int(1));
        arr = snow_json_array_push(arr, snow_json_from_string(make_string("oops")));

        let result = snow_json_to_list(arr, json_to_int_result);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should propagate Err from element decode");
        }
    }

    #[test]
    fn test_json_float_roundtrip() {
        snow_rt_init();
        // Create a float, encode it, re-parse, verify it comes back as float
        let json = snow_json_from_float(3.14);
        let encoded = snow_json_encode(json as *mut u8);
        unsafe {
            let text = (*encoded).as_str();
            let parsed: f64 = text.parse().unwrap();
            assert!((parsed - 3.14).abs() < 0.001);
        }
    }

    // ── Phase 50: snow_json_array_get tests ──────────────────────────

    #[test]
    fn test_json_array_get_valid() {
        snow_rt_init();
        let mut arr = snow_json_array_new();
        arr = snow_json_array_push(arr, snow_json_from_int(10));
        arr = snow_json_array_push(arr, snow_json_from_int(20));
        arr = snow_json_array_push(arr, snow_json_from_int(30));

        // Get element at index 0
        let result = snow_json_array_get(arr, 0);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 0, "should be Ok for valid index 0");
            let elem = (*res).value as *const SnowJson;
            assert_eq!((*elem).tag, JSON_INT);
            assert_eq!((*elem).value as i64, 10);
        }

        // Get element at index 2
        let result2 = snow_json_array_get(arr, 2);
        unsafe {
            let res = result2 as *mut SnowResult;
            assert_eq!((*res).tag, 0, "should be Ok for valid index 2");
            let elem = (*res).value as *const SnowJson;
            assert_eq!((*elem).value as i64, 30);
        }
    }

    #[test]
    fn test_json_array_get_out_of_bounds() {
        snow_rt_init();
        let mut arr = snow_json_array_new();
        arr = snow_json_array_push(arr, snow_json_from_int(1));

        // Index too large
        let result = snow_json_array_get(arr, 5);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should be Err for out of bounds");
        }

        // Negative index
        let result2 = snow_json_array_get(arr, -1);
        unsafe {
            let res = result2 as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should be Err for negative index");
        }
    }

    #[test]
    fn test_json_array_get_wrong_type() {
        snow_rt_init();
        let obj = snow_json_object_new();

        // Passing an object instead of array
        let result = snow_json_array_get(obj, 0);
        unsafe {
            let res = result as *mut SnowResult;
            assert_eq!((*res).tag, 1, "should be Err for non-array");
        }
    }
}
