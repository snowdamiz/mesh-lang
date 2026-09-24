//! GC-managed immutable Map for the Mesh runtime.
//!
//! A map's entries are `(key, value)` pairs of uniform 8-byte words, kept in
//! insertion order. The storage (a table that small maps copy on a change and
//! large ones grow in place, with a hash index) is in [`super::table`].
//!
//! All operations return a NEW map (immutable semantics): a value never sees
//! a change made after it.
//!
//! A table's key type tag (upper 8 bits of its capacity word) says how keys
//! compare when compiled code passes no Eq: 0 = by value (Int, Bool, Float),
//! 1 = as strings. The `_by` functions take the key type's Eq and Hash, for
//! keys that are neither (tuples, lists, structs, sum values).

use super::list::alloc_pair;
use super::table::{self, Keys};
use std::ptr;

/// Key type tag: integer keys (compared by value equality).
const KEY_TYPE_INT: u64 = 0;
/// Key type tag: string keys (compared by content via mesh_string_eq).
const KEY_TYPE_STR: u64 = 1;

type Entry = [u64; 2];

/// How the map's keys compare: by `key_eq` and `key_hash` when given (bare
/// function pointers, null for none), else as its tag says.
unsafe fn keys(map: *const u8, key_eq: *mut u8, key_hash: *mut u8) -> Keys {
    let table = table::state::<2>(map).0;
    Keys::new(table::tag(table) == KEY_TYPE_STR, key_eq, key_hash)
}

/// The value of `key`'s live entry.
unsafe fn lookup(map: *mut u8, key: u64, keys: &Keys) -> Option<u64> {
    let (table, n, _) = table::state::<2>(map);
    table::find::<2>(table, n, key, keys).map(|position| *table::entry::<2>(table, position).add(1))
}

/// The live entries, in order, as a table of their own.
unsafe fn entries(map: *mut u8) -> (*const Entry, usize) {
    let compact = table::compact::<2>(map);
    (
        table::entry::<2>(compact, 0) as *const Entry,
        table::len(compact),
    )
}

/// The live entries of a map, in order, without allocating: for message
/// capture and the wire format.
pub(crate) unsafe fn live_entries(map: *const u8) -> (u64, Vec<Entry>) {
    let table = table::state::<2>(map).0;
    (table::tag(table), table::live_entries::<2>(map))
}

/// A map of `entries` (keys unique) with key type tag `tag`.
pub(crate) unsafe fn map_from_entries(tag: u64, entries: &[Entry]) -> *mut u8 {
    table::table_from::<2>(entries, tag)
}

// ── Public API ────────────────────────────────────────────────────────

/// Create an empty map (integer keys, backward compatible).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_new() -> *mut u8 {
    unsafe { table::alloc_table::<2>(0, KEY_TYPE_INT) }
}

/// Create an empty map with a specific key_type tag.
/// key_type: 0 = Int, 1 = String.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_new_typed(key_type: i64) -> *mut u8 {
    unsafe { table::alloc_table::<2>(0, key_type as u64) }
}

pub(crate) fn mesh_map_from_string_entries(entries: &[[u64; 2]]) -> *mut u8 {
    let mut map = mesh_map_new_typed(KEY_TYPE_STR as i64);
    for &[key, value] in entries {
        map = mesh_map_put(map, key, value);
    }
    map
}

/// Ensure a map has string key_type. If the map is empty and has integer key_type,
/// returns a new empty map with string key_type. Otherwise returns the map unchanged.
/// Used by codegen to tag maps before the first string-key put.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_tag_string(map: *mut u8) -> *mut u8 {
    unsafe {
        let table = table::state::<2>(map).0;
        if table::size::<2>(map) == 0 && table::tag(table) != KEY_TYPE_STR {
            mesh_map_new_typed(KEY_TYPE_STR as i64)
        } else {
            map
        }
    }
}

/// Return a NEW map with the key-value pair added (or updated).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_put(map: *mut u8, key: u64, value: u64) -> *mut u8 {
    mesh_map_put_by(map, key, value, ptr::null_mut(), ptr::null_mut())
}

/// `mesh_map_put` with keys compared by `key_eq` and hashed by `key_hash`.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_put_by(
    map: *mut u8,
    key: u64,
    value: u64,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> *mut u8 {
    unsafe { table::put::<2>(map, [key, value], &keys(map, key_eq, key_hash)) }
}

/// Get the value for a key. Returns 0 if not found.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_get(map: *mut u8, key: u64) -> u64 {
    mesh_map_get_by(map, key, ptr::null_mut(), ptr::null_mut())
}

/// `mesh_map_get` with keys compared by `key_eq` and hashed by `key_hash`.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_get_by(
    map: *mut u8,
    key: u64,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> u64 {
    unsafe { lookup(map, key, &keys(map, key_eq, key_hash)).unwrap_or(0) }
}

/// `Map.get`: the value at `key`, which must be in the map. A missing key
/// is a Mesh panic, as `List.get` past the end is (it read as 0).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_fetch(map: *mut u8, key: u64) -> u64 {
    mesh_map_fetch_by(map, key, ptr::null_mut(), ptr::null_mut())
}

/// `mesh_map_fetch` with keys compared by `key_eq` and hashed by `key_hash`.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_fetch_by(
    map: *mut u8,
    key: u64,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> u64 {
    unsafe {
        match lookup(map, key, &keys(map, key_eq, key_hash)) {
            Some(value) => value,
            None => crate::panic::raise(format_args!(
                "Map.get: the key is not in the map (check with Map.has_key)"
            )),
        }
    }
}

/// Returns 1 if the key exists, 0 otherwise.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_has_key(map: *mut u8, key: u64) -> i8 {
    mesh_map_has_key_by(map, key, ptr::null_mut(), ptr::null_mut())
}

/// `mesh_map_has_key` with keys compared by `key_eq` and hashed by `key_hash`.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_has_key_by(
    map: *mut u8,
    key: u64,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> i8 {
    unsafe { lookup(map, key, &keys(map, key_eq, key_hash)).is_some() as i8 }
}

/// Return a NEW map without the given key.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_delete(map: *mut u8, key: u64) -> *mut u8 {
    mesh_map_delete_by(map, key, ptr::null_mut(), ptr::null_mut())
}

/// `mesh_map_delete` with keys compared by `key_eq` and hashed by `key_hash`.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_delete_by(
    map: *mut u8,
    key: u64,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> *mut u8 {
    unsafe { table::delete::<2>(map, key, &keys(map, key_eq, key_hash)) }
}

/// Return the number of entries in the map.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_size(map: *mut u8) -> i64 {
    unsafe { table::size::<2>(map) as i64 }
}

/// Whether two maps hold the same keys with equal values, in any order.
/// Keys compare as the map compares them; `val_eq` is a bare
/// `fn(u64, u64) -> i8` over two raw value slots.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_eq(a: *mut u8, b: *mut u8, val_eq: *mut u8) -> i8 {
    mesh_map_eq_by(a, b, val_eq, ptr::null_mut(), ptr::null_mut())
}

/// `mesh_map_eq` with keys compared by `key_eq` and hashed by `key_hash`.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_eq_by(
    a: *mut u8,
    b: *mut u8,
    val_eq: *mut u8,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> i8 {
    type ValEq = unsafe extern "C-unwind" fn(u64, u64) -> i8;

    unsafe {
        if table::size::<2>(a) != table::size::<2>(b) {
            return 0;
        }
        let f: ValEq = std::mem::transmute(val_eq);
        let keys = keys(b, key_eq, key_hash);
        let (entries, len) = entries(a);
        for i in 0..len {
            let [key, value] = *entries.add(i);
            match lookup(b, key, &keys) {
                Some(other) if f(value, other) != 0 => {}
                _ => return 0,
            }
        }
        1
    }
}

/// Hash a map by its entries, keys hashed by `key_hash` and values by
/// `val_hash` (`fn(slot) -> Int`), independently of their order.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_hash_by(
    map: *mut u8,
    key_hash: *mut u8,
    val_hash: *mut u8,
) -> i64 {
    type SlotHash = unsafe extern "C-unwind" fn(u64) -> i64;
    unsafe {
        let (k, v): (SlotHash, SlotHash) =
            (std::mem::transmute(key_hash), std::mem::transmute(val_hash));
        let (entries, len) = entries(map);
        let sum = (0..len).fold(0i64, |acc, i| {
            let [key, value] = *entries.add(i);
            acc.wrapping_add(crate::hash::mesh_hash_combine(k(key), v(value)))
        });
        crate::hash::mesh_hash_combine(crate::hash::mesh_hash_int(len as i64), sum)
    }
}

/// A list of one word of each entry: its key (0) or its value (1).
unsafe fn entry_list(map: *mut u8, word: usize) -> *mut u8 {
    let (entries, len) = entries(map);
    let mut list = super::list::mesh_list_builder_new(len as i64);
    for i in 0..len {
        list = super::list::mesh_list_builder_push(list, (*entries.add(i))[word]);
    }
    list
}

/// Return a List of all keys in the map.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_keys(map: *mut u8) -> *mut u8 {
    unsafe { entry_list(map, 0) }
}

/// Return a List of all values in the map.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_values(map: *mut u8) -> *mut u8 {
    unsafe { entry_list(map, 1) }
}

/// Word `word` of the entry at `index` (insertion order), for indexed map
/// iteration in compiled `for` loops. Panics if out of bounds.
unsafe fn entry_word(map: *mut u8, index: i64, word: usize, what: &str) -> u64 {
    let (entries, len) = entries(map);
    if index < 0 || index as usize >= len {
        panic!("{what}: index {index} out of bounds (len {len})");
    }
    (*entries.add(index as usize))[word]
}

/// Get the key at index i (insertion order). Panics if out of bounds.
/// Used by for-in codegen for indexed map iteration.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_entry_key(map: *mut u8, index: i64) -> u64 {
    unsafe { entry_word(map, index, 0, "mesh_map_entry_key") }
}

/// Get the value at index i (insertion order). Panics if out of bounds.
/// Used by for-in codegen for indexed map iteration.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_entry_value(map: *mut u8, index: i64) -> u64 {
    unsafe { entry_word(map, index, 1, "mesh_map_entry_value") }
}

/// Convert a map to a human-readable MeshString: `%{k1 => v1, k2 => v2, ...}`.
///
/// `key_to_str` and `val_to_str` are bare function pointers `fn(u64) -> *mut u8`
/// that convert keys and values to MeshString pointers respectively.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_to_string(
    map: *mut u8,
    key_to_str: *mut u8,
    val_to_str: *mut u8,
) -> *mut u8 {
    type ElemToStr = unsafe extern "C-unwind" fn(u64) -> *mut u8;

    unsafe {
        let (entries, len) = entries(map);
        let kf: ElemToStr = std::mem::transmute(key_to_str);
        let vf: ElemToStr = std::mem::transmute(val_to_str);

        let mut result = String::from("%{");
        for i in 0..len {
            if i > 0 {
                result.push_str(", ");
            }
            let [key, val] = *entries.add(i);
            let key_str = kf(key) as *const crate::string::MeshString;
            result.push_str((*key_str).as_str());
            result.push_str(" => ");
            let val_str = vf(val) as *const crate::string::MeshString;
            result.push_str((*val_str).as_str());
        }
        result.push('}');
        crate::string::mesh_string_new(result.as_ptr(), result.len() as u64) as *mut u8
    }
}

/// Merge two maps. All entries from `a` are included; entries from `b`
/// overwrite duplicates from `a`. Returns a NEW merged map.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_merge(a: *mut u8, b: *mut u8) -> *mut u8 {
    mesh_map_merge_by(a, b, ptr::null_mut(), ptr::null_mut())
}

/// `mesh_map_merge` with keys compared by `key_eq` and hashed by `key_hash`.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_merge_by(
    a: *mut u8,
    b: *mut u8,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> *mut u8 {
    unsafe {
        let keys = keys(a, key_eq, key_hash);
        let (entries, len) = entries(b);
        let mut result = a;
        for i in 0..len {
            result = table::put::<2>(result, *entries.add(i), &keys);
        }
        result
    }
}

/// Convert a map to a list of (key, value) 2-tuples.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_to_list(map: *mut u8) -> *mut u8 {
    unsafe {
        let (entries, len) = entries(map);
        let mut list = super::list::mesh_list_builder_new(len as i64);
        for i in 0..len {
            let [key, val] = *entries.add(i);
            let pair = alloc_pair(key, val);
            list = super::list::mesh_list_builder_push(list, pair as u64);
        }
        list
    }
}

/// Build a map with Int keys from a list of (key, value) 2-tuples.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_from_list(list: *mut u8) -> *mut u8 {
    mesh_map_from_list_by(list, KEY_TYPE_INT as i64, ptr::null_mut(), ptr::null_mut())
}

/// Build a map from a list of (key, value) 2-tuples, with keys compared as
/// `key_type` (0 Int, 1 String) or by `key_eq` and `key_hash`. The runtime
/// cannot tell the key type from the values, so the compiler says it. A
/// repeated key keeps its first place and takes the last value, as with
/// `put`, which grows the map in place.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_from_list_by(
    list: *mut u8,
    key_type: i64,
    key_eq: *mut u8,
    key_hash: *mut u8,
) -> *mut u8 {
    unsafe {
        let len = super::list::mesh_list_length(list);
        let mut map = mesh_map_new_typed(key_type);
        for i in 0..len {
            let tuple_ptr = super::list::mesh_list_get(list, i) as *const u64;
            // A pair is `{ len, key, value }`.
            map = mesh_map_put_by(map, *tuple_ptr.add(1), *tuple_ptr.add(2), key_eq, key_hash);
        }
        map
    }
}

// ── Iterator handle ───────────────────────────────────────────────────

/// Internal iterator state for Map iteration.
#[repr(C)]
struct MapIterator {
    tag: u8,
    map: *mut u8,
    index: i64,
    size: i64,
}

/// Create a new iterator handle for a map.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_iter_new(map: *mut u8) -> *mut u8 {
    unsafe {
        // The entries in order, laid out once.
        let map = table::compact::<2>(map);
        let size = mesh_map_size(map);
        let iter = crate::gc::mesh_gc_alloc_actor(
            std::mem::size_of::<MapIterator>() as u64,
            std::mem::align_of::<MapIterator>() as u64,
        ) as *mut MapIterator;
        (*iter).tag = 1; // ITER_TAG_MAP
        (*iter).map = map;
        (*iter).index = 0;
        (*iter).size = size;
        iter as *mut u8
    }
}

/// Advance the map iterator, returning Option<(K, V)> (tag 0 = Some, tag 1 = None).
/// The Some payload is a GC-allocated 2-tuple (key, value).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_iter_next(iter_ptr: *mut u8) -> *mut u8 {
    unsafe {
        let iter = iter_ptr as *mut MapIterator;
        if (*iter).index >= (*iter).size {
            crate::option::alloc_option(1, std::ptr::null_mut()) as *mut u8
        } else {
            let key = mesh_map_entry_key((*iter).map, (*iter).index);
            let val = mesh_map_entry_value((*iter).map, (*iter).index);
            (*iter).index += 1;
            let pair = alloc_pair(key, val);
            crate::option::alloc_option(0, pair as *mut u8) as *mut u8
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;

    #[test]
    fn test_map_new_is_empty() {
        mesh_rt_init();
        let map = mesh_map_new();
        assert_eq!(mesh_map_size(map), 0);
    }

    #[test]
    fn test_map_put_get() {
        mesh_rt_init();
        let map = mesh_map_new();
        let map = mesh_map_put(map, 42, 100);
        assert_eq!(mesh_map_size(map), 1);
        assert_eq!(mesh_map_get(map, 42), 100);
        assert_eq!(mesh_map_has_key(map, 42), 1);
        assert_eq!(mesh_map_has_key(map, 99), 0);
    }

    #[test]
    fn test_map_put_overwrite() {
        mesh_rt_init();
        let map = mesh_map_new();
        let map = mesh_map_put(map, 1, 10);
        let map = mesh_map_put(map, 1, 20);
        assert_eq!(mesh_map_size(map), 1);
        assert_eq!(mesh_map_get(map, 1), 20);
    }

    #[test]
    fn test_map_delete() {
        mesh_rt_init();
        let map = mesh_map_new();
        let map = mesh_map_put(map, 1, 10);
        let map = mesh_map_put(map, 2, 20);
        let map = mesh_map_delete(map, 1);
        assert_eq!(mesh_map_size(map), 1);
        assert_eq!(mesh_map_has_key(map, 1), 0);
        assert_eq!(mesh_map_get(map, 2), 20);
    }

    #[test]
    fn test_map_keys_values() {
        mesh_rt_init();
        let map = mesh_map_new();
        let map = mesh_map_put(map, 1, 10);
        let map = mesh_map_put(map, 2, 20);
        let keys = mesh_map_keys(map);
        let vals = mesh_map_values(map);
        assert_eq!(super::super::list::mesh_list_length(keys), 2);
        assert_eq!(super::super::list::mesh_list_length(vals), 2);
        for (index, key, value) in [(0, 1, 10), (1, 2, 20)] {
            assert_eq!(super::super::list::mesh_list_get(keys, index), key);
            assert_eq!(super::super::list::mesh_list_get(vals, index), value);
            assert_eq!(mesh_map_get(map, key), value);
        }
        let empty = mesh_map_new();
        assert_eq!(
            super::super::list::mesh_list_length(mesh_map_keys(empty)),
            0
        );
        assert_eq!(
            super::super::list::mesh_list_length(mesh_map_values(empty)),
            0
        );
    }

    #[test]
    fn test_map_immutability() {
        mesh_rt_init();
        let map1 = mesh_map_new();
        let map2 = mesh_map_put(map1, 1, 10);
        // Original map unchanged.
        assert_eq!(mesh_map_size(map1), 0);
        assert_eq!(mesh_map_size(map2), 1);
    }

    #[test]
    fn test_map_string_keys() {
        mesh_rt_init();
        // Create a string-key map (key_type = 1).
        let map = mesh_map_new_typed(1);

        // Create string keys and values.
        let key1 = crate::string::mesh_string_new(b"name".as_ptr(), 4) as u64;
        let val1 = crate::string::mesh_string_new(b"Alice".as_ptr(), 5) as u64;
        let key2 = crate::string::mesh_string_new(b"city".as_ptr(), 4) as u64;
        let val2 = crate::string::mesh_string_new(b"Portland".as_ptr(), 8) as u64;

        let map = mesh_map_put(map, key1, val1);
        let map = mesh_map_put(map, key2, val2);

        assert_eq!(mesh_map_size(map), 2);

        // Look up with a DIFFERENT string pointer but same content.
        let lookup_key = crate::string::mesh_string_new(b"name".as_ptr(), 4) as u64;
        let got = mesh_map_get(map, lookup_key);
        assert_eq!(got, val1);

        let lookup_key2 = crate::string::mesh_string_new(b"city".as_ptr(), 4) as u64;
        assert_eq!(mesh_map_has_key(map, lookup_key2), 1);

        // Non-existent key.
        let missing = crate::string::mesh_string_new(b"missing".as_ptr(), 7) as u64;
        assert_eq!(mesh_map_has_key(map, missing), 0);
    }

    #[test]
    fn test_map_string_key_overwrite() {
        mesh_rt_init();
        let map = mesh_map_new_typed(1);

        let key = crate::string::mesh_string_new(b"name".as_ptr(), 4) as u64;
        let val1 = crate::string::mesh_string_new(b"Alice".as_ptr(), 5) as u64;
        let val2 = crate::string::mesh_string_new(b"Bob".as_ptr(), 3) as u64;

        let map = mesh_map_put(map, key, val1);
        // Use a different pointer for the same key content.
        let key2 = crate::string::mesh_string_new(b"name".as_ptr(), 4) as u64;
        let map = mesh_map_put(map, key2, val2);

        assert_eq!(mesh_map_size(map), 1);

        let lookup = crate::string::mesh_string_new(b"name".as_ptr(), 4) as u64;
        let got = mesh_map_get(map, lookup);
        assert_eq!(got, val2);
    }

    #[test]
    fn test_map_to_string() {
        mesh_rt_init();
        let map = mesh_map_new();
        let map = mesh_map_put(map, 1, 10);
        let map = mesh_map_put(map, 2, 20);

        let result = mesh_map_to_string(
            map,
            crate::string::mesh_int_to_string as *mut u8,
            crate::string::mesh_int_to_string as *mut u8,
        );
        let s = unsafe { &*(result as *const crate::string::MeshString) };
        let text = unsafe { s.as_str() };
        assert_eq!(text, "%{1 => 10, 2 => 20}");
    }

    #[test]
    fn test_map_to_string_empty() {
        mesh_rt_init();
        let map = mesh_map_new();

        let result = mesh_map_to_string(
            map,
            crate::string::mesh_int_to_string as *mut u8,
            crate::string::mesh_int_to_string as *mut u8,
        );
        let s = unsafe { &*(result as *const crate::string::MeshString) };
        let text = unsafe { s.as_str() };
        assert_eq!(text, "%{}");
    }

    #[test]
    fn test_map_entry_key_value() {
        mesh_rt_init();
        let map = mesh_map_new();
        let map = mesh_map_put(map, 10, 100);
        let map = mesh_map_put(map, 20, 200);
        let map = mesh_map_put(map, 30, 300);

        assert_eq!(mesh_map_entry_key(map, 0), 10);
        assert_eq!(mesh_map_entry_value(map, 0), 100);
        assert_eq!(mesh_map_entry_key(map, 1), 20);
        assert_eq!(mesh_map_entry_value(map, 1), 200);
        assert_eq!(mesh_map_entry_key(map, 2), 30);
        assert_eq!(mesh_map_entry_value(map, 2), 300);
    }
}
