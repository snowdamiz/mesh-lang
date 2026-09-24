//! GC-managed immutable Set for the Mesh runtime.
//!
//! A set's elements are uniform 8-byte words, compared as words, kept in
//! the order they were added. The storage (a table that small sets copy on a
//! change and large ones grow in place, with a hash index) is in
//! [`super::table`].
//!
//! All operations return a NEW set (immutable semantics): a value never sees
//! a change made after it.

use super::table::{self, Keys};

/// The live elements, in order, as a table of their own.
unsafe fn elements(set: *mut u8) -> (*const u64, usize) {
    let compact = table::compact::<1>(set);
    (table::entry::<1>(compact, 0), table::len(compact))
}

unsafe fn contains(set: *mut u8, element: u64) -> bool {
    let (table, n, _) = table::state::<1>(set);
    table::find::<1>(table, n, element, &Keys::WORDS).is_some()
}

/// The live elements of a set, in order, without allocating: for message
/// capture and the wire format.
pub(crate) unsafe fn live_elements(set: *const u8) -> Vec<u64> {
    table::live_entries::<1>(set)
        .into_iter()
        .map(|[element]| element)
        .collect()
}

/// A set of `elements` (unique).
pub(crate) unsafe fn set_from_elements(elements: &[u64]) -> *mut u8 {
    let entries: Vec<[u64; 1]> = elements.iter().map(|&element| [element]).collect();
    table::table_from::<1>(&entries, 0)
}

// ── Public API ────────────────────────────────────────────────────────

/// Create an empty set.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_new() -> *mut u8 {
    unsafe { table::alloc_table::<1>(0, 0) }
}

/// Return a NEW set with the element added (the set itself if it holds it).
#[no_mangle]
pub extern "C-unwind" fn mesh_set_add(set: *mut u8, element: u64) -> *mut u8 {
    unsafe {
        if contains(set, element) {
            set
        } else {
            table::put::<1>(set, [element], &Keys::WORDS)
        }
    }
}

/// Return a NEW set without the element.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_remove(set: *mut u8, element: u64) -> *mut u8 {
    unsafe { table::delete::<1>(set, element, &Keys::WORDS) }
}

/// Returns 1 if the element is in the set, 0 otherwise.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_contains(set: *mut u8, element: u64) -> i8 {
    unsafe { contains(set, element) as i8 }
}

/// Return the number of elements in the set.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_size(set: *mut u8) -> i64 {
    unsafe { table::size::<1>(set) as i64 }
}

/// Whether two sets hold the same elements, in any order.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_eq(a: *mut u8, b: *mut u8) -> i8 {
    unsafe {
        if table::size::<1>(a) != table::size::<1>(b) {
            return 0;
        }
        let (data, len) = elements(a);
        (0..len).all(|i| contains(b, *data.add(i))) as i8
    }
}

/// Hash a set by its elements, each hashed by `hash` (`fn(slot) -> Int`),
/// independently of their order.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_hash_by(set: *mut u8, hash: *mut u8) -> i64 {
    type ElemHash = unsafe extern "C-unwind" fn(u64) -> i64;
    unsafe {
        let f: ElemHash = std::mem::transmute(hash);
        let (data, len) = elements(set);
        let sum = (0..len).fold(0i64, |acc, i| acc.wrapping_add(f(*data.add(i))));
        crate::hash::mesh_hash_combine(crate::hash::mesh_hash_int(len as i64), sum)
    }
}

/// Return a NEW set that is the union of `a` and `b`: `a`'s elements, then
/// `b`'s that are not in `a`.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_union(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        let (data, len) = elements(b);
        let mut result = a;
        for i in 0..len {
            result = mesh_set_add(result, *data.add(i));
        }
        result
    }
}

/// The elements of `a` that are (`keep`) or are not in `b`, in `a`'s order.
unsafe fn filter_by(a: *mut u8, b: *mut u8, keep: bool) -> *mut u8 {
    let (data, len) = elements(a);
    let mut result = mesh_set_new();
    for i in 0..len {
        let element = *data.add(i);
        if contains(b, element) == keep {
            result = table::put::<1>(result, [element], &Keys::WORDS);
        }
    }
    result
}

/// Return a NEW set that is the intersection of `a` and `b`.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_intersection(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe { filter_by(a, b, true) }
}

/// Get the element at index i. Panics if out of bounds.
/// Used by for-in codegen for indexed set iteration.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_element_at(set: *mut u8, index: i64) -> u64 {
    unsafe {
        let (data, len) = elements(set);
        if index < 0 || index as usize >= len {
            panic!("mesh_set_element_at: index {index} out of bounds (len {len})");
        }
        *data.add(index as usize)
    }
}

/// Convert a set to a human-readable MeshString: `#{elem1, elem2, ...}`.
///
/// `elem_to_str` is a bare function pointer `fn(u64) -> *mut u8` that converts
/// each element to a MeshString pointer.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_to_string(set: *mut u8, elem_to_str: *mut u8) -> *mut u8 {
    type ElemToStr = unsafe extern "C-unwind" fn(u64) -> *mut u8;

    unsafe {
        let (data, len) = elements(set);
        let f: ElemToStr = std::mem::transmute(elem_to_str);

        let mut result = String::from("#{");
        for i in 0..len {
            if i > 0 {
                result.push_str(", ");
            }
            let elem_str = f(*data.add(i)) as *const crate::string::MeshString;
            result.push_str((*elem_str).as_str());
        }
        result.push('}');
        crate::string::mesh_string_new(result.as_ptr(), result.len() as u64) as *mut u8
    }
}

/// Return a NEW set containing elements in `a` that are NOT in `b`.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_difference(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe { filter_by(a, b, false) }
}

/// Convert a set to a list of its elements.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_to_list(set: *mut u8) -> *mut u8 {
    unsafe {
        let (data, len) = elements(set);
        let mut list = super::list::mesh_list_builder_new(len as i64);
        for i in 0..len {
            list = super::list::mesh_list_builder_push(list, *data.add(i));
        }
        list
    }
}

/// Build a set from a list, without its repeats; `add` grows it in place.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_from_list(list: *mut u8) -> *mut u8 {
    unsafe {
        let (len, data) = super::list::list_slots(list);
        let mut set = mesh_set_new();
        for i in 0..len {
            set = mesh_set_add(set, *data.add(i));
        }
        set
    }
}

// ── Iterator handle ───────────────────────────────────────────────────

/// Internal iterator state for Set iteration.
#[repr(C)]
struct SetIterator {
    tag: u8,
    set: *mut u8,
    index: i64,
    size: i64,
}

/// Create a new iterator handle for a set.
#[no_mangle]
pub extern "C-unwind" fn mesh_set_iter_new(set: *mut u8) -> *mut u8 {
    unsafe {
        // The elements in order, laid out once.
        let set = table::compact::<1>(set);
        let size = mesh_set_size(set);
        let iter = crate::gc::mesh_gc_alloc_actor(
            std::mem::size_of::<SetIterator>() as u64,
            std::mem::align_of::<SetIterator>() as u64,
        ) as *mut SetIterator;
        (*iter).tag = 2; // ITER_TAG_SET
        (*iter).set = set;
        (*iter).index = 0;
        (*iter).size = size;
        iter as *mut u8
    }
}

/// Advance the set iterator, returning Option (tag 0 = Some, tag 1 = None).
#[no_mangle]
pub extern "C-unwind" fn mesh_set_iter_next(iter_ptr: *mut u8) -> *mut u8 {
    unsafe {
        let iter = iter_ptr as *mut SetIterator;
        if (*iter).index >= (*iter).size {
            crate::option::alloc_option(1, std::ptr::null_mut()) as *mut u8
        } else {
            let elem = mesh_set_element_at((*iter).set, (*iter).index);
            (*iter).index += 1;
            crate::option::alloc_option(0, elem as usize as *mut u8) as *mut u8
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;

    #[test]
    fn test_set_new_is_empty() {
        mesh_rt_init();
        let set = mesh_set_new();
        assert_eq!(mesh_set_size(set), 0);
    }

    #[test]
    fn test_set_add_contains() {
        mesh_rt_init();
        let set = mesh_set_new();
        let set = mesh_set_add(set, 10);
        let set = mesh_set_add(set, 20);
        assert_eq!(mesh_set_size(set), 2);
        assert_eq!(mesh_set_contains(set, 10), 1);
        assert_eq!(mesh_set_contains(set, 20), 1);
        assert_eq!(mesh_set_contains(set, 30), 0);
    }

    #[test]
    fn test_set_add_duplicate() {
        mesh_rt_init();
        let set = mesh_set_new();
        let set = mesh_set_add(set, 10);
        let set = mesh_set_add(set, 10);
        assert_eq!(mesh_set_size(set), 1);
    }

    #[test]
    fn test_set_remove() {
        mesh_rt_init();
        let set = mesh_set_new();
        let set = mesh_set_add(set, 1);
        let set = mesh_set_add(set, 2);
        let set = mesh_set_add(set, 3);
        let set = mesh_set_remove(set, 2);
        assert_eq!(mesh_set_size(set), 2);
        assert_eq!(mesh_set_contains(set, 2), 0);
        assert_eq!(mesh_set_contains(set, 1), 1);
        assert_eq!(mesh_set_contains(set, 3), 1);
    }

    #[test]
    fn test_set_union() {
        mesh_rt_init();
        let a = mesh_set_new();
        let a = mesh_set_add(a, 1);
        let a = mesh_set_add(a, 2);
        let b = mesh_set_new();
        let b = mesh_set_add(b, 2);
        let b = mesh_set_add(b, 3);
        let c = mesh_set_union(a, b);
        assert_eq!(mesh_set_size(c), 3);
        assert_eq!(mesh_set_contains(c, 1), 1);
        assert_eq!(mesh_set_contains(c, 2), 1);
        assert_eq!(mesh_set_contains(c, 3), 1);
    }

    #[test]
    fn test_set_intersection() {
        mesh_rt_init();
        let a = mesh_set_new();
        let a = mesh_set_add(a, 1);
        let a = mesh_set_add(a, 2);
        let a = mesh_set_add(a, 3);
        let b = mesh_set_new();
        let b = mesh_set_add(b, 2);
        let b = mesh_set_add(b, 3);
        let b = mesh_set_add(b, 4);
        let c = mesh_set_intersection(a, b);
        assert_eq!(mesh_set_size(c), 2);
        assert_eq!(mesh_set_contains(c, 2), 1);
        assert_eq!(mesh_set_contains(c, 3), 1);
        assert_eq!(mesh_set_contains(c, 1), 0);
    }

    #[test]
    fn test_set_immutability() {
        mesh_rt_init();
        let s1 = mesh_set_new();
        let s2 = mesh_set_add(s1, 1);
        assert_eq!(mesh_set_size(s1), 0);
        assert_eq!(mesh_set_size(s2), 1);
    }

    #[test]
    fn test_set_to_string() {
        mesh_rt_init();
        let set = mesh_set_new();
        let set = mesh_set_add(set, 10);
        let set = mesh_set_add(set, 20);
        let set = mesh_set_add(set, 30);

        let result = mesh_set_to_string(set, crate::string::mesh_int_to_string as *mut u8);
        let s = unsafe { &*(result as *const crate::string::MeshString) };
        let text = unsafe { s.as_str() };
        assert_eq!(text, "#{10, 20, 30}");
    }

    #[test]
    fn test_set_to_string_empty() {
        mesh_rt_init();
        let set = mesh_set_new();

        let result = mesh_set_to_string(set, crate::string::mesh_int_to_string as *mut u8);
        let s = unsafe { &*(result as *const crate::string::MeshString) };
        let text = unsafe { s.as_str() };
        assert_eq!(text, "#{}");
    }

    #[test]
    fn test_set_element_at() {
        mesh_rt_init();
        let set = mesh_set_new();
        let set = mesh_set_add(set, 10);
        let set = mesh_set_add(set, 20);
        let set = mesh_set_add(set, 30);

        assert_eq!(mesh_set_element_at(set, 0), 10);
        assert_eq!(mesh_set_element_at(set, 1), 20);
        assert_eq!(mesh_set_element_at(set, 2), 30);
    }
}
