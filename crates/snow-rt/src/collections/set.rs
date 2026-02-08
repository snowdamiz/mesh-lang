//! GC-managed immutable Set for the Snow runtime.
//!
//! A SnowSet stores unique elements as uniform 8-byte (`u64`) values.
//! Backed by a simple sorted vector with linear scan -- efficient for
//! the small sets typical in Phase 8.
//!
//! All mutation operations return a NEW set (immutable semantics).

use crate::gc::snow_gc_alloc_actor;
use std::ptr;

/// Header: len (u64), cap (u64).
const HEADER_SIZE: usize = 16;
const ELEM_SIZE: usize = 8;

// ── Internal helpers ──────────────────────────────────────────────────

unsafe fn set_len(s: *const u8) -> u64 {
    *(s as *const u64)
}

unsafe fn set_data(s: *const u8) -> *const u64 {
    (s as *const u64).add(2)
}

unsafe fn set_data_mut(s: *mut u8) -> *mut u64 {
    (s as *mut u64).add(2)
}

unsafe fn alloc_set(cap: u64) -> *mut u8 {
    let total = HEADER_SIZE + (cap as usize) * ELEM_SIZE;
    let p = snow_gc_alloc_actor(total as u64, 8);
    *(p as *mut u64) = 0;
    *((p as *mut u64).add(1)) = cap;
    p
}

unsafe fn contains_elem(s: *const u8, elem: u64) -> bool {
    let len = set_len(s) as usize;
    let data = set_data(s);
    for i in 0..len {
        if *data.add(i) == elem {
            return true;
        }
    }
    false
}

// ── Public API ────────────────────────────────────────────────────────

/// Create an empty set.
#[no_mangle]
pub extern "C" fn snow_set_new() -> *mut u8 {
    unsafe { alloc_set(0) }
}

/// Return a NEW set with the element added (no-op if already present).
#[no_mangle]
pub extern "C" fn snow_set_add(set: *mut u8, element: u64) -> *mut u8 {
    unsafe {
        if contains_elem(set, element) {
            // Already present -- return a copy.
            let len = set_len(set);
            let new_set = alloc_set(len);
            *(new_set as *mut u64) = len;
            if len > 0 {
                ptr::copy_nonoverlapping(set_data(set), set_data_mut(new_set), len as usize);
            }
            return new_set;
        }

        let len = set_len(set) as usize;
        let new_len = len + 1;
        let new_set = alloc_set(new_len as u64);
        *(new_set as *mut u64) = new_len as u64;
        if len > 0 {
            ptr::copy_nonoverlapping(set_data(set), set_data_mut(new_set), len);
        }
        *set_data_mut(new_set).add(len) = element;
        new_set
    }
}

/// Return a NEW set without the element.
#[no_mangle]
pub extern "C" fn snow_set_remove(set: *mut u8, element: u64) -> *mut u8 {
    unsafe {
        let len = set_len(set) as usize;
        let data = set_data(set);
        let new_set = alloc_set(len as u64);
        let dst = set_data_mut(new_set);
        let mut j = 0;
        for i in 0..len {
            if *data.add(i) != element {
                *dst.add(j) = *data.add(i);
                j += 1;
            }
        }
        *(new_set as *mut u64) = j as u64;
        new_set
    }
}

/// Returns 1 if the element is in the set, 0 otherwise.
#[no_mangle]
pub extern "C" fn snow_set_contains(set: *mut u8, element: u64) -> i8 {
    unsafe {
        if contains_elem(set, element) {
            1
        } else {
            0
        }
    }
}

/// Return the number of elements in the set.
#[no_mangle]
pub extern "C" fn snow_set_size(set: *mut u8) -> i64 {
    unsafe { set_len(set) as i64 }
}

/// Return a NEW set that is the union of `a` and `b`.
#[no_mangle]
pub extern "C" fn snow_set_union(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        // Start with a copy of `a`, then add elements from `b`.
        let a_len = set_len(a) as usize;
        let b_len = set_len(b) as usize;
        let max_len = a_len + b_len;
        let result = alloc_set(max_len as u64);
        let dst = set_data_mut(result);

        // Copy all of `a`.
        let a_data = set_data(a);
        if a_len > 0 {
            ptr::copy_nonoverlapping(a_data, dst, a_len);
        }
        let mut count = a_len;

        // Add elements from `b` that are not in `a`.
        let b_data = set_data(b);
        for i in 0..b_len {
            let elem = *b_data.add(i);
            // Linear search in result so far.
            let mut found = false;
            for j in 0..count {
                if *dst.add(j) == elem {
                    found = true;
                    break;
                }
            }
            if !found {
                *dst.add(count) = elem;
                count += 1;
            }
        }

        *(result as *mut u64) = count as u64;
        result
    }
}

/// Return a NEW set that is the intersection of `a` and `b`.
#[no_mangle]
pub extern "C" fn snow_set_intersection(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        let a_len = set_len(a) as usize;
        let result = alloc_set(a_len as u64);
        let a_data = set_data(a);
        let dst = set_data_mut(result);
        let mut count = 0;

        for i in 0..a_len {
            let elem = *a_data.add(i);
            if contains_elem(b, elem) {
                *dst.add(count) = elem;
                count += 1;
            }
        }

        *(result as *mut u64) = count as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_set_new_is_empty() {
        snow_rt_init();
        let set = snow_set_new();
        assert_eq!(snow_set_size(set), 0);
    }

    #[test]
    fn test_set_add_contains() {
        snow_rt_init();
        let set = snow_set_new();
        let set = snow_set_add(set, 10);
        let set = snow_set_add(set, 20);
        assert_eq!(snow_set_size(set), 2);
        assert_eq!(snow_set_contains(set, 10), 1);
        assert_eq!(snow_set_contains(set, 20), 1);
        assert_eq!(snow_set_contains(set, 30), 0);
    }

    #[test]
    fn test_set_add_duplicate() {
        snow_rt_init();
        let set = snow_set_new();
        let set = snow_set_add(set, 10);
        let set = snow_set_add(set, 10);
        assert_eq!(snow_set_size(set), 1);
    }

    #[test]
    fn test_set_remove() {
        snow_rt_init();
        let set = snow_set_new();
        let set = snow_set_add(set, 1);
        let set = snow_set_add(set, 2);
        let set = snow_set_add(set, 3);
        let set = snow_set_remove(set, 2);
        assert_eq!(snow_set_size(set), 2);
        assert_eq!(snow_set_contains(set, 2), 0);
        assert_eq!(snow_set_contains(set, 1), 1);
        assert_eq!(snow_set_contains(set, 3), 1);
    }

    #[test]
    fn test_set_union() {
        snow_rt_init();
        let a = snow_set_new();
        let a = snow_set_add(a, 1);
        let a = snow_set_add(a, 2);
        let b = snow_set_new();
        let b = snow_set_add(b, 2);
        let b = snow_set_add(b, 3);
        let c = snow_set_union(a, b);
        assert_eq!(snow_set_size(c), 3);
        assert_eq!(snow_set_contains(c, 1), 1);
        assert_eq!(snow_set_contains(c, 2), 1);
        assert_eq!(snow_set_contains(c, 3), 1);
    }

    #[test]
    fn test_set_intersection() {
        snow_rt_init();
        let a = snow_set_new();
        let a = snow_set_add(a, 1);
        let a = snow_set_add(a, 2);
        let a = snow_set_add(a, 3);
        let b = snow_set_new();
        let b = snow_set_add(b, 2);
        let b = snow_set_add(b, 3);
        let b = snow_set_add(b, 4);
        let c = snow_set_intersection(a, b);
        assert_eq!(snow_set_size(c), 2);
        assert_eq!(snow_set_contains(c, 2), 1);
        assert_eq!(snow_set_contains(c, 3), 1);
        assert_eq!(snow_set_contains(c, 1), 0);
    }

    #[test]
    fn test_set_immutability() {
        snow_rt_init();
        let s1 = snow_set_new();
        let s2 = snow_set_add(s1, 1);
        assert_eq!(snow_set_size(s1), 0);
        assert_eq!(snow_set_size(s2), 1);
    }
}
