//! GC-managed immutable Map for the Snow runtime.
//!
//! A SnowMap stores key-value pairs where both keys and values are uniform
//! 8-byte (`u64`) values. Backed by a simple vector of `(u64, u64)` pairs
//! with linear scan -- efficient for the small maps typical in Phase 8.
//!
//! All mutation operations return a NEW map (immutable semantics).
//!
//! The upper 8 bits of the capacity field store a key_type tag:
//! - 0 = integer keys (compared by value)
//! - 1 = string keys (compared by content via snow_string_eq)

use crate::gc::snow_gc_alloc;
use std::ptr;

/// Map header: len (u64), cap (u64).
const HEADER_SIZE: usize = 16;
/// Each entry is a (key, value) pair = 16 bytes.
const ENTRY_SIZE: usize = 16;

/// Key type tag: integer keys (compared by value equality).
const KEY_TYPE_INT: u64 = 0;
/// Key type tag: string keys (compared by content via snow_string_eq).
const KEY_TYPE_STR: u64 = 1;
/// Number of bits to shift for the key_type tag in the cap field.
const TAG_SHIFT: u64 = 56;
/// Mask for extracting the raw capacity (lower 56 bits).
const CAP_MASK: u64 = (1u64 << 56) - 1;

// ── Internal helpers ──────────────────────────────────────────────────

unsafe fn map_len(m: *const u8) -> u64 {
    *(m as *const u64)
}

/// Extract the key_type tag from the upper 8 bits of the cap field.
unsafe fn map_key_type(m: *const u8) -> u64 {
    (*((m as *const u64).add(1))) >> TAG_SHIFT
}

/// Extract the raw capacity (lower 56 bits of the cap field).
unsafe fn map_cap_raw(m: *const u8) -> u64 {
    (*((m as *const u64).add(1))) & CAP_MASK
}

#[allow(dead_code)]
unsafe fn map_cap(m: *const u8) -> u64 {
    map_cap_raw(m)
}

unsafe fn map_entries(m: *const u8) -> *const [u64; 2] {
    (m as *const u8).add(HEADER_SIZE) as *const [u64; 2]
}

unsafe fn map_entries_mut(m: *mut u8) -> *mut [u64; 2] {
    m.add(HEADER_SIZE) as *mut [u64; 2]
}

/// Check if two keys are equal, dispatching based on the map's key_type.
unsafe fn keys_equal(m: *const u8, a: u64, b: u64) -> bool {
    if map_key_type(m) == KEY_TYPE_STR {
        crate::string::snow_string_eq(
            a as *const crate::string::SnowString,
            b as *const crate::string::SnowString,
        ) != 0
    } else {
        a == b
    }
}

unsafe fn alloc_map(cap: u64, key_type: u64) -> *mut u8 {
    let total = HEADER_SIZE + (cap as usize) * ENTRY_SIZE;
    let p = snow_gc_alloc(total as u64, 8);
    *(p as *mut u64) = 0; // len
    *((p as *mut u64).add(1)) = (key_type << TAG_SHIFT) | cap; // key_type tag + cap
    p
}

/// Find the index of a key, or return None.
unsafe fn find_key(m: *const u8, key: u64) -> Option<usize> {
    let len = map_len(m) as usize;
    let entries = map_entries(m);
    for i in 0..len {
        if keys_equal(m, (*entries.add(i))[0], key) {
            return Some(i);
        }
    }
    None
}

// ── Public API ────────────────────────────────────────────────────────

/// Create an empty map (integer keys, backward compatible).
#[no_mangle]
pub extern "C" fn snow_map_new() -> *mut u8 {
    unsafe { alloc_map(0, KEY_TYPE_INT) }
}

/// Create an empty map with a specific key_type tag.
/// key_type: 0 = Int, 1 = String.
#[no_mangle]
pub extern "C" fn snow_map_new_typed(key_type: i64) -> *mut u8 {
    unsafe { alloc_map(0, key_type as u64) }
}

/// Return a NEW map with the key-value pair added (or updated).
#[no_mangle]
pub extern "C" fn snow_map_put(map: *mut u8, key: u64, value: u64) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let kt = map_key_type(map);

        // Check if key already exists -- replace.
        if let Some(idx) = find_key(map, key) {
            let new_map = alloc_map(len as u64, kt);
            *(new_map as *mut u64) = len as u64;
            ptr::copy_nonoverlapping(
                map_entries(map) as *const u8,
                map_entries_mut(new_map) as *mut u8,
                len * ENTRY_SIZE,
            );
            (*map_entries_mut(new_map).add(idx))[1] = value;
            return new_map;
        }

        // Add new entry.
        let new_len = len + 1;
        let new_map = alloc_map(new_len as u64, kt);
        *(new_map as *mut u64) = new_len as u64;
        if len > 0 {
            ptr::copy_nonoverlapping(
                map_entries(map) as *const u8,
                map_entries_mut(new_map) as *mut u8,
                len * ENTRY_SIZE,
            );
        }
        (*map_entries_mut(new_map).add(len))[0] = key;
        (*map_entries_mut(new_map).add(len))[1] = value;
        new_map
    }
}

/// Get the value for a key. Returns 0 if not found.
#[no_mangle]
pub extern "C" fn snow_map_get(map: *mut u8, key: u64) -> u64 {
    unsafe {
        if let Some(idx) = find_key(map, key) {
            (*map_entries(map).add(idx))[1]
        } else {
            0
        }
    }
}

/// Returns 1 if the key exists, 0 otherwise.
#[no_mangle]
pub extern "C" fn snow_map_has_key(map: *mut u8, key: u64) -> i8 {
    unsafe {
        if find_key(map, key).is_some() {
            1
        } else {
            0
        }
    }
}

/// Return a NEW map without the given key.
#[no_mangle]
pub extern "C" fn snow_map_delete(map: *mut u8, key: u64) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let kt = map_key_type(map);
        match find_key(map, key) {
            Some(idx) => {
                let new_len = len - 1;
                let new_map = alloc_map(new_len as u64, kt);
                *(new_map as *mut u64) = new_len as u64;
                let src = map_entries(map);
                let dst = map_entries_mut(new_map);
                let mut j = 0;
                for i in 0..len {
                    if i != idx {
                        (*dst.add(j))[0] = (*src.add(i))[0];
                        (*dst.add(j))[1] = (*src.add(i))[1];
                        j += 1;
                    }
                }
                new_map
            }
            None => {
                // Key not found -- return a copy.
                let new_map = alloc_map(len as u64, kt);
                *(new_map as *mut u64) = len as u64;
                if len > 0 {
                    ptr::copy_nonoverlapping(
                        map_entries(map) as *const u8,
                        map_entries_mut(new_map) as *mut u8,
                        len * ENTRY_SIZE,
                    );
                }
                new_map
            }
        }
    }
}

/// Return the number of entries in the map.
#[no_mangle]
pub extern "C" fn snow_map_size(map: *mut u8) -> i64 {
    unsafe { map_len(map) as i64 }
}

/// Return a List of all keys in the map.
#[no_mangle]
pub extern "C" fn snow_map_keys(map: *mut u8) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let entries = map_entries(map);
        let mut list = super::list::snow_list_new();
        for i in 0..len {
            list = super::list::snow_list_append(list, (*entries.add(i))[0]);
        }
        list
    }
}

/// Return a List of all values in the map.
#[no_mangle]
pub extern "C" fn snow_map_values(map: *mut u8) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let entries = map_entries(map);
        let mut list = super::list::snow_list_new();
        for i in 0..len {
            list = super::list::snow_list_append(list, (*entries.add(i))[1]);
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_map_new_is_empty() {
        snow_rt_init();
        let map = snow_map_new();
        assert_eq!(snow_map_size(map), 0);
    }

    #[test]
    fn test_map_put_get() {
        snow_rt_init();
        let map = snow_map_new();
        let map = snow_map_put(map, 42, 100);
        assert_eq!(snow_map_size(map), 1);
        assert_eq!(snow_map_get(map, 42), 100);
        assert_eq!(snow_map_has_key(map, 42), 1);
        assert_eq!(snow_map_has_key(map, 99), 0);
    }

    #[test]
    fn test_map_put_overwrite() {
        snow_rt_init();
        let map = snow_map_new();
        let map = snow_map_put(map, 1, 10);
        let map = snow_map_put(map, 1, 20);
        assert_eq!(snow_map_size(map), 1);
        assert_eq!(snow_map_get(map, 1), 20);
    }

    #[test]
    fn test_map_delete() {
        snow_rt_init();
        let map = snow_map_new();
        let map = snow_map_put(map, 1, 10);
        let map = snow_map_put(map, 2, 20);
        let map = snow_map_delete(map, 1);
        assert_eq!(snow_map_size(map), 1);
        assert_eq!(snow_map_has_key(map, 1), 0);
        assert_eq!(snow_map_get(map, 2), 20);
    }

    #[test]
    fn test_map_keys_values() {
        snow_rt_init();
        let map = snow_map_new();
        let map = snow_map_put(map, 1, 10);
        let map = snow_map_put(map, 2, 20);
        let keys = snow_map_keys(map);
        let vals = snow_map_values(map);
        assert_eq!(super::super::list::snow_list_length(keys), 2);
        assert_eq!(super::super::list::snow_list_length(vals), 2);
    }

    #[test]
    fn test_map_immutability() {
        snow_rt_init();
        let map1 = snow_map_new();
        let map2 = snow_map_put(map1, 1, 10);
        // Original map unchanged.
        assert_eq!(snow_map_size(map1), 0);
        assert_eq!(snow_map_size(map2), 1);
    }

    #[test]
    fn test_map_string_keys() {
        snow_rt_init();
        // Create a string-key map (key_type = 1).
        let map = snow_map_new_typed(1);

        // Create string keys and values.
        let key1 = crate::string::snow_string_new(b"name".as_ptr(), 4) as u64;
        let val1 = crate::string::snow_string_new(b"Alice".as_ptr(), 5) as u64;
        let key2 = crate::string::snow_string_new(b"city".as_ptr(), 4) as u64;
        let val2 = crate::string::snow_string_new(b"Portland".as_ptr(), 8) as u64;

        let map = snow_map_put(map, key1, val1);
        let map = snow_map_put(map, key2, val2);

        assert_eq!(snow_map_size(map), 2);

        // Look up with a DIFFERENT string pointer but same content.
        let lookup_key = crate::string::snow_string_new(b"name".as_ptr(), 4) as u64;
        let got = snow_map_get(map, lookup_key);
        assert_eq!(got, val1);

        let lookup_key2 = crate::string::snow_string_new(b"city".as_ptr(), 4) as u64;
        assert_eq!(snow_map_has_key(map, lookup_key2), 1);

        // Non-existent key.
        let missing = crate::string::snow_string_new(b"missing".as_ptr(), 7) as u64;
        assert_eq!(snow_map_has_key(map, missing), 0);
    }

    #[test]
    fn test_map_string_key_overwrite() {
        snow_rt_init();
        let map = snow_map_new_typed(1);

        let key = crate::string::snow_string_new(b"name".as_ptr(), 4) as u64;
        let val1 = crate::string::snow_string_new(b"Alice".as_ptr(), 5) as u64;
        let val2 = crate::string::snow_string_new(b"Bob".as_ptr(), 3) as u64;

        let map = snow_map_put(map, key, val1);
        // Use a different pointer for the same key content.
        let key2 = crate::string::snow_string_new(b"name".as_ptr(), 4) as u64;
        let map = snow_map_put(map, key2, val2);

        assert_eq!(snow_map_size(map), 1);

        let lookup = crate::string::snow_string_new(b"name".as_ptr(), 4) as u64;
        let got = snow_map_get(map, lookup);
        assert_eq!(got, val2);
    }
}
