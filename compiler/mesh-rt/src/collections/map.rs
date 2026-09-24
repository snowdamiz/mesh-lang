//! GC-managed immutable Map for the Mesh runtime.
//!
//! A MeshMap stores key-value pairs where both keys and values are uniform
//! 8-byte (`u64`) values. Backed by a simple vector of `(u64, u64)` pairs
//! with linear scan -- efficient for the small maps typical in Phase 8.
//!
//! All mutation operations return a NEW map (immutable semantics).
//!
//! The upper 8 bits of the capacity field store a key_type tag:
//! - 0 = integer keys (compared by value)
//! - 1 = string keys (compared by content via mesh_string_eq)

use super::list::alloc_pair;
use crate::gc::mesh_gc_alloc_actor;
use std::ptr;

/// Map header: len (u64), cap (u64).
const HEADER_SIZE: usize = 16;
/// Each entry is a (key, value) pair = 16 bytes.
const ENTRY_SIZE: usize = 16;

/// Key type tag: integer keys (compared by value equality).
const KEY_TYPE_INT: u64 = 0;
/// Key type tag: string keys (compared by content via mesh_string_eq).
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

/// A key type's Eq over two raw key slots (1 when equal), for keys that are
/// not words or strings: tuples, lists, structs, sum values. The `_by`
/// functions take one; a null pointer means "compare as the map's key_type".
type KeyEq = unsafe extern "C-unwind" fn(u64, u64) -> i8;

unsafe fn key_eq_fn(key_eq: *mut u8) -> Option<KeyEq> {
    (!key_eq.is_null()).then(|| std::mem::transmute::<*mut u8, KeyEq>(key_eq))
}

/// Check if two keys are equal: by `key_eq` when given, otherwise by the
/// map's key_type.
unsafe fn keys_equal(m: *const u8, a: u64, b: u64, key_eq: Option<KeyEq>) -> bool {
    if let Some(eq) = key_eq {
        return eq(a, b) != 0;
    }
    if map_key_type(m) == KEY_TYPE_STR {
        crate::string::mesh_string_eq(
            a as *const crate::string::MeshString,
            b as *const crate::string::MeshString,
        ) != 0
    } else {
        a == b
    }
}

unsafe fn alloc_map(cap: u64, key_type: u64) -> *mut u8 {
    let total = HEADER_SIZE + (cap as usize) * ENTRY_SIZE;
    let p = mesh_gc_alloc_actor(total as u64, 8);
    *(p as *mut u64) = 0; // len
    *((p as *mut u64).add(1)) = (key_type << TAG_SHIFT) | cap; // key_type tag + cap
    p
}

/// Find the index of a key, or return None.
unsafe fn find_key(m: *const u8, key: u64, key_eq: Option<KeyEq>) -> Option<usize> {
    let len = map_len(m) as usize;
    let entries = map_entries(m);
    for i in 0..len {
        if keys_equal(m, (*entries.add(i))[0], key, key_eq) {
            return Some(i);
        }
    }
    None
}

// ── Public API ────────────────────────────────────────────────────────

/// Create an empty map (integer keys, backward compatible).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_new() -> *mut u8 {
    unsafe { alloc_map(0, KEY_TYPE_INT) }
}

/// Create an empty map with a specific key_type tag.
/// key_type: 0 = Int, 1 = String.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_new_typed(key_type: i64) -> *mut u8 {
    unsafe { alloc_map(0, key_type as u64) }
}

pub(crate) fn mesh_map_from_string_entries(entries: &[[u64; 2]]) -> *mut u8 {
    unsafe {
        let map = alloc_map(entries.len() as u64, KEY_TYPE_STR);
        *(map as *mut u64) = entries.len() as u64;
        if !entries.is_empty() {
            ptr::copy_nonoverlapping(entries.as_ptr(), map_entries_mut(map), entries.len());
        }
        map
    }
}

/// Ensure a map has string key_type. If the map is empty and has integer key_type,
/// returns a new empty map with string key_type. Otherwise returns the map unchanged.
/// Used by codegen to tag maps before the first string-key put.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_tag_string(map: *mut u8) -> *mut u8 {
    unsafe {
        if map_len(map) == 0 && map_key_type(map) != KEY_TYPE_STR {
            alloc_map(0, KEY_TYPE_STR)
        } else {
            map
        }
    }
}

/// Return a NEW map with the key-value pair added (or updated).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_put(map: *mut u8, key: u64, value: u64) -> *mut u8 {
    mesh_map_put_by(map, key, value, ptr::null_mut())
}

/// `mesh_map_put` with keys compared by `key_eq` (see `KeyEq`).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_put_by(
    map: *mut u8,
    key: u64,
    value: u64,
    key_eq: *mut u8,
) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let kt = map_key_type(map);

        // Check if key already exists -- replace.
        if let Some(idx) = find_key(map, key, key_eq_fn(key_eq)) {
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
pub extern "C-unwind" fn mesh_map_get(map: *mut u8, key: u64) -> u64 {
    mesh_map_get_by(map, key, ptr::null_mut())
}

/// `mesh_map_get` with keys compared by `key_eq` (see `KeyEq`).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_get_by(map: *mut u8, key: u64, key_eq: *mut u8) -> u64 {
    unsafe {
        if let Some(idx) = find_key(map, key, key_eq_fn(key_eq)) {
            (*map_entries(map).add(idx))[1]
        } else {
            0
        }
    }
}

/// `Map.get`: the value at `key`, which must be in the map. A missing key
/// is a Mesh panic, as `List.get` past the end is (it read as 0).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_fetch(map: *mut u8, key: u64) -> u64 {
    mesh_map_fetch_by(map, key, ptr::null_mut())
}

/// `mesh_map_fetch` with keys compared by `key_eq` (see `KeyEq`).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_fetch_by(map: *mut u8, key: u64, key_eq: *mut u8) -> u64 {
    unsafe {
        match find_key(map, key, key_eq_fn(key_eq)) {
            Some(idx) => (*map_entries(map).add(idx))[1],
            None => crate::panic::raise(format_args!(
                "Map.get: the key is not in the map (check with Map.has_key)"
            )),
        }
    }
}

/// Returns 1 if the key exists, 0 otherwise.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_has_key(map: *mut u8, key: u64) -> i8 {
    mesh_map_has_key_by(map, key, ptr::null_mut())
}

/// `mesh_map_has_key` with keys compared by `key_eq` (see `KeyEq`).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_has_key_by(map: *mut u8, key: u64, key_eq: *mut u8) -> i8 {
    unsafe { find_key(map, key, key_eq_fn(key_eq)).is_some() as i8 }
}

/// Return a NEW map without the given key.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_delete(map: *mut u8, key: u64) -> *mut u8 {
    mesh_map_delete_by(map, key, ptr::null_mut())
}

/// `mesh_map_delete` with keys compared by `key_eq` (see `KeyEq`).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_delete_by(map: *mut u8, key: u64, key_eq: *mut u8) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let kt = map_key_type(map);
        match find_key(map, key, key_eq_fn(key_eq)) {
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
pub extern "C-unwind" fn mesh_map_size(map: *mut u8) -> i64 {
    unsafe { map_len(map) as i64 }
}

/// Whether two maps hold the same keys with equal values, in any order.
/// Keys compare as the map compares them; `val_eq` is a bare
/// `fn(u64, u64) -> i8` over two raw value slots.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_eq(a: *mut u8, b: *mut u8, val_eq: *mut u8) -> i8 {
    mesh_map_eq_by(a, b, val_eq, ptr::null_mut())
}

/// `mesh_map_eq` with keys compared by `key_eq` (see `KeyEq`).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_eq_by(
    a: *mut u8,
    b: *mut u8,
    val_eq: *mut u8,
    key_eq: *mut u8,
) -> i8 {
    type ValEq = unsafe extern "C-unwind" fn(u64, u64) -> i8;

    unsafe {
        if map_len(a) != map_len(b) {
            return 0;
        }
        let f: ValEq = std::mem::transmute(val_eq);
        let entries = map_entries(a);
        for i in 0..map_len(a) as usize {
            let [key, value] = *entries.add(i);
            match find_key(b, key, key_eq_fn(key_eq)) {
                Some(j) if f(value, (*map_entries(b).add(j))[1]) != 0 => {}
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
        let (entries, len) = (map_entries(map), map_len(map));
        let sum = (0..len as usize).fold(0i64, |acc, i| {
            let [key, value] = *entries.add(i);
            acc.wrapping_add(crate::hash::mesh_hash_combine(k(key), v(value)))
        });
        crate::hash::mesh_hash_combine(crate::hash::mesh_hash_int(len as i64), sum)
    }
}

/// Return a List of all keys in the map.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_keys(map: *mut u8) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let entries = map_entries(map);
        let mut list = super::list::mesh_list_builder_new(len as i64);
        for i in 0..len {
            list = super::list::mesh_list_builder_push(list, (*entries.add(i))[0]);
        }
        list
    }
}

/// Return a List of all values in the map.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_values(map: *mut u8) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let entries = map_entries(map);
        let mut list = super::list::mesh_list_builder_new(len as i64);
        for i in 0..len {
            list = super::list::mesh_list_builder_push(list, (*entries.add(i))[1]);
        }
        list
    }
}

/// Get the key at index i (insertion order). Panics if out of bounds.
/// Used by for-in codegen for indexed map iteration.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_entry_key(map: *mut u8, index: i64) -> u64 {
    unsafe {
        let len = map_len(map);
        if index < 0 || index as u64 >= len {
            panic!(
                "mesh_map_entry_key: index {} out of bounds (len {})",
                index, len
            );
        }
        let entries = map_entries(map);
        (*entries.add(index as usize))[0]
    }
}

/// Get the value at index i (insertion order). Panics if out of bounds.
/// Used by for-in codegen for indexed map iteration.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_entry_value(map: *mut u8, index: i64) -> u64 {
    unsafe {
        let len = map_len(map);
        if index < 0 || index as u64 >= len {
            panic!(
                "mesh_map_entry_value: index {} out of bounds (len {})",
                index, len
            );
        }
        let entries = map_entries(map);
        (*entries.add(index as usize))[1]
    }
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
        let len = map_len(map) as usize;
        let entries = map_entries(map);
        let kf: ElemToStr = std::mem::transmute(key_to_str);
        let vf: ElemToStr = std::mem::transmute(val_to_str);

        let mut result = String::from("%{");
        for i in 0..len {
            if i > 0 {
                result.push_str(", ");
            }
            let key = (*entries.add(i))[0];
            let val = (*entries.add(i))[1];
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
    mesh_map_merge_by(a, b, ptr::null_mut())
}

/// `mesh_map_merge` with keys compared by `key_eq` (see `KeyEq`).
#[no_mangle]
pub extern "C-unwind" fn mesh_map_merge_by(a: *mut u8, b: *mut u8, key_eq: *mut u8) -> *mut u8 {
    unsafe {
        let a_len = map_len(a) as usize;
        let b_len = map_len(b) as usize;
        let kt = map_key_type(a);

        // Start with a copy of `a`.
        let mut result = alloc_map((a_len + b_len) as u64, kt);
        *(result as *mut u64) = a_len as u64;
        if a_len > 0 {
            ptr::copy_nonoverlapping(
                map_entries(a) as *const u8,
                map_entries_mut(result) as *mut u8,
                a_len * ENTRY_SIZE,
            );
        }

        // Add/overwrite entries from `b`.
        let b_entries = map_entries(b);
        for i in 0..b_len {
            let key = (*b_entries.add(i))[0];
            let val = (*b_entries.add(i))[1];
            result = mesh_map_put_by(result, key, val, key_eq);
        }

        result
    }
}

/// Convert a map to a list of (key, value) 2-tuples.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_to_list(map: *mut u8) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let entries = map_entries(map);
        let mut list = super::list::mesh_list_builder_new(len as i64);
        for i in 0..len {
            let key = (*entries.add(i))[0];
            let val = (*entries.add(i))[1];
            let pair = alloc_pair(key, val);
            list = super::list::mesh_list_builder_push(list, pair as u64);
        }
        list
    }
}

/// Builds a map in place, for `from_list` and `collect`: `put` copies the
/// map it is given, so adding one entry at a time took quadratic time. A
/// repeated key keeps its first place and takes the last value, as with
/// `put`. Int and String keys are found through a hash index, keys compared
/// by `key_eq` by a scan.
///
/// The entries stay in the GC map held here (on the caller's stack), so a
/// collection while `key_eq` or an iterator runs Mesh code still sees them.
pub(crate) struct MapBuilder {
    map: *mut u8,
    key_eq: Option<KeyEq>,
    ints: std::collections::HashMap<u64, usize>,
    strings: std::collections::HashMap<Vec<u8>, usize>,
}

impl MapBuilder {
    pub(crate) unsafe fn new(key_type: u64, key_eq: *mut u8) -> Self {
        MapBuilder {
            map: alloc_map(4, key_type),
            key_eq: key_eq_fn(key_eq),
            ints: Default::default(),
            strings: Default::default(),
        }
    }

    pub(crate) unsafe fn put(&mut self, key: u64, value: u64) {
        use std::collections::hash_map::Entry;
        let len = map_len(self.map) as usize;
        let found = if self.key_eq.is_some() {
            // ponytail: a scan per key, quadratic for keys with their own Eq;
            // hashing them by the key type's Hash would make it linear.
            find_key(self.map, key, self.key_eq)
        } else if map_key_type(self.map) == KEY_TYPE_STR {
            let bytes = (*(key as *const crate::string::MeshString)).as_bytes();
            match self.strings.entry(bytes.to_vec()) {
                Entry::Occupied(at) => Some(*at.get()),
                Entry::Vacant(slot) => {
                    slot.insert(len);
                    None
                }
            }
        } else {
            match self.ints.entry(key) {
                Entry::Occupied(at) => Some(*at.get()),
                Entry::Vacant(slot) => {
                    slot.insert(len);
                    None
                }
            }
        };
        if let Some(index) = found {
            (*map_entries_mut(self.map).add(index))[1] = value;
            return;
        }
        if len as u64 == map_cap_raw(self.map) {
            let grown = alloc_map(len as u64 * 2, map_key_type(self.map));
            ptr::copy_nonoverlapping(map_entries(self.map), map_entries_mut(grown), len);
            *(grown as *mut u64) = len as u64;
            self.map = grown;
        }
        *map_entries_mut(self.map).add(len) = [key, value];
        *(self.map as *mut u64) = len as u64 + 1;
    }

    /// The map, with no room to spare: every other map's capacity is its
    /// length.
    pub(crate) unsafe fn finish(self) -> *mut u8 {
        let len = map_len(self.map);
        let map = alloc_map(len, map_key_type(self.map));
        ptr::copy_nonoverlapping(map_entries(self.map), map_entries_mut(map), len as usize);
        *(map as *mut u64) = len;
        map
    }
}

/// Build a map with Int keys from a list of (key, value) 2-tuples.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_from_list(list: *mut u8) -> *mut u8 {
    mesh_map_from_list_by(list, KEY_TYPE_INT as i64, ptr::null_mut())
}

/// Build a map from a list of (key, value) 2-tuples, with keys compared as
/// `key_type` (0 Int, 1 String) or by `key_eq` (see `KeyEq`). The runtime
/// cannot tell the key type from the values, so the compiler says it.
#[no_mangle]
pub extern "C-unwind" fn mesh_map_from_list_by(
    list: *mut u8,
    key_type: i64,
    key_eq: *mut u8,
) -> *mut u8 {
    unsafe {
        let len = super::list::mesh_list_length(list);
        let mut map = MapBuilder::new(key_type as u64, key_eq);
        for i in 0..len {
            let tuple_ptr = super::list::mesh_list_get(list, i) as *const u64;
            // A pair is `{ len, key, value }`.
            map.put(*tuple_ptr.add(1), *tuple_ptr.add(2));
        }
        map.finish()
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
