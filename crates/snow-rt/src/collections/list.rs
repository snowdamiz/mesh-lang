//! GC-managed immutable List for the Snow runtime.
//!
//! A SnowList stores elements as uniform 8-byte (`u64`) values in a contiguous
//! GC-allocated buffer. Layout: `{ len: u64, cap: u64, data: [u64; cap] }`.
//!
//! All mutation operations (append, tail, concat, etc.) return a NEW list,
//! preserving immutability semantics.

use crate::gc::snow_gc_alloc_actor;
use std::ptr;

/// Header size: len (8 bytes) + cap (8 bytes) = 16 bytes.
const HEADER_SIZE: usize = 16;

/// Byte size of one element.
const ELEM_SIZE: usize = 8;

// ── Internal helpers ──────────────────────────────────────────────────

/// Read the length field from a list pointer.
unsafe fn list_len(list: *const u8) -> u64 {
    *(list as *const u64)
}

/// Read the capacity field from a list pointer.
unsafe fn list_cap(list: *const u8) -> u64 {
    *((list as *const u64).add(1))
}

/// Get a pointer to the data region (past the header).
unsafe fn list_data(list: *const u8) -> *const u64 {
    (list as *const u64).add(2)
}

/// Get a mutable pointer to the data region.
unsafe fn list_data_mut(list: *mut u8) -> *mut u64 {
    (list as *mut u64).add(2)
}

/// Allocate a new list with the given capacity, length set to 0.
unsafe fn alloc_list(cap: u64) -> *mut u8 {
    let total = HEADER_SIZE + (cap as usize) * ELEM_SIZE;
    let p = snow_gc_alloc_actor(total as u64, 8);
    // len = 0, cap = cap
    *(p as *mut u64) = 0;
    *((p as *mut u64).add(1)) = cap;
    p
}

/// Allocate a new list with the given length and capacity, copying `len` elements from `src`.
unsafe fn alloc_list_from(src: *const u64, len: u64, cap: u64) -> *mut u8 {
    let p = alloc_list(cap);
    *(p as *mut u64) = len;
    if len > 0 {
        ptr::copy_nonoverlapping(src, list_data_mut(p), len as usize);
    }
    p
}

// ── Public API ────────────────────────────────────────────────────────

/// Create an empty list.
#[no_mangle]
pub extern "C" fn snow_list_new() -> *mut u8 {
    unsafe { alloc_list(0) }
}

/// Return the number of elements in the list.
#[no_mangle]
pub extern "C" fn snow_list_length(list: *mut u8) -> i64 {
    unsafe { list_len(list) as i64 }
}

/// Return a NEW list with `element` appended at the end.
#[no_mangle]
pub extern "C" fn snow_list_append(list: *mut u8, element: u64) -> *mut u8 {
    unsafe {
        let len = list_len(list);
        let new_cap = len + 1;
        let new_list = alloc_list(new_cap);
        *(new_list as *mut u64) = new_cap; // len = old len + 1
        if len > 0 {
            ptr::copy_nonoverlapping(list_data(list), list_data_mut(new_list), len as usize);
        }
        *list_data_mut(new_list).add(len as usize) = element;
        new_list
    }
}

/// Return the first element. Panics if empty.
#[no_mangle]
pub extern "C" fn snow_list_head(list: *mut u8) -> u64 {
    unsafe {
        let len = list_len(list);
        if len == 0 {
            panic!("snow_list_head: empty list");
        }
        *list_data(list)
    }
}

/// Return a NEW list without the first element. Panics if empty.
#[no_mangle]
pub extern "C" fn snow_list_tail(list: *mut u8) -> *mut u8 {
    unsafe {
        let len = list_len(list);
        if len == 0 {
            panic!("snow_list_tail: empty list");
        }
        let new_len = len - 1;
        alloc_list_from(list_data(list).add(1), new_len, new_len)
    }
}

/// Get the element at `index`. Panics if out of bounds.
#[no_mangle]
pub extern "C" fn snow_list_get(list: *mut u8, index: i64) -> u64 {
    unsafe {
        let len = list_len(list);
        if index < 0 || index as u64 >= len {
            panic!(
                "snow_list_get: index {} out of bounds (len {})",
                index, len
            );
        }
        *list_data(list).add(index as usize)
    }
}

/// Concatenate two lists into a NEW list.
#[no_mangle]
pub extern "C" fn snow_list_concat(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        let a_len = list_len(a);
        let b_len = list_len(b);
        let new_len = a_len + b_len;
        let new_list = alloc_list(new_len);
        *(new_list as *mut u64) = new_len;
        if a_len > 0 {
            ptr::copy_nonoverlapping(list_data(a), list_data_mut(new_list), a_len as usize);
        }
        if b_len > 0 {
            ptr::copy_nonoverlapping(
                list_data(b),
                list_data_mut(new_list).add(a_len as usize),
                b_len as usize,
            );
        }
        new_list
    }
}

/// Return a reversed copy of the list.
#[no_mangle]
pub extern "C" fn snow_list_reverse(list: *mut u8) -> *mut u8 {
    unsafe {
        let len = list_len(list);
        let new_list = alloc_list(len);
        *(new_list as *mut u64) = len;
        let src = list_data(list);
        let dst = list_data_mut(new_list);
        for i in 0..len as usize {
            *dst.add(i) = *src.add(len as usize - 1 - i);
        }
        new_list
    }
}

/// Apply a closure to each element, returning a new list.
///
/// If `env_ptr` is null, `fn_ptr` is called as `fn(element) -> result`.
/// If `env_ptr` is non-null, `fn_ptr` is called as `fn(env_ptr, element) -> result`.
#[no_mangle]
pub extern "C" fn snow_list_map(
    list: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    type BareFn = unsafe extern "C" fn(u64) -> u64;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64) -> u64;

    unsafe {
        let len = list_len(list);
        let new_list = alloc_list(len);
        *(new_list as *mut u64) = len;
        let src = list_data(list);
        let dst = list_data_mut(new_list);

        if env_ptr.is_null() {
            let f: BareFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                *dst.add(i) = f(*src.add(i));
            }
        } else {
            let f: ClosureFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                *dst.add(i) = f(env_ptr, *src.add(i));
            }
        }
        new_list
    }
}

/// Keep elements where the closure returns non-zero (true).
#[no_mangle]
pub extern "C" fn snow_list_filter(
    list: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    type BareFn = unsafe extern "C" fn(u64) -> u64;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64) -> u64;

    unsafe {
        let len = list_len(list);
        // Allocate worst case, then shrink.
        let temp = alloc_list(len);
        let src = list_data(list);
        let dst = list_data_mut(temp);
        let mut count = 0u64;

        if env_ptr.is_null() {
            let f: BareFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                let elem = *src.add(i);
                if f(elem) != 0 {
                    *dst.add(count as usize) = elem;
                    count += 1;
                }
            }
        } else {
            let f: ClosureFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                let elem = *src.add(i);
                if f(env_ptr, elem) != 0 {
                    *dst.add(count as usize) = elem;
                    count += 1;
                }
            }
        }

        // Set actual length.
        *(temp as *mut u64) = count;
        temp
    }
}

/// Fold left over the list with an accumulator.
///
/// If `env_ptr` is null: `fn_ptr(acc, element) -> acc`
/// If `env_ptr` is non-null: `fn_ptr(env_ptr, acc, element) -> acc`
#[no_mangle]
pub extern "C" fn snow_list_reduce(
    list: *mut u8,
    init: u64,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> u64 {
    type BareFn = unsafe extern "C" fn(u64, u64) -> u64;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64, u64) -> u64;

    unsafe {
        let len = list_len(list);
        let src = list_data(list);
        let mut acc = init;

        if env_ptr.is_null() {
            let f: BareFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                acc = f(acc, *src.add(i));
            }
        } else {
            let f: ClosureFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                acc = f(env_ptr, acc, *src.add(i));
            }
        }
        acc
    }
}

/// Create a list from an array of u64 elements.
#[no_mangle]
pub extern "C" fn snow_list_from_array(data: *const u64, count: i64) -> *mut u8 {
    unsafe {
        let count = count.max(0) as u64;
        alloc_list_from(data, count, count)
    }
}

/// Compare two lists for equality using an element-comparison callback.
///
/// `elem_eq` is a bare function pointer `fn(u64, u64) -> i8` that returns 1
/// if two elements are equal, 0 otherwise. Returns 1 if lists are equal, 0 if not.
#[no_mangle]
pub extern "C" fn snow_list_eq(
    list_a: *mut u8,
    list_b: *mut u8,
    elem_eq: *mut u8,
) -> i8 {
    type ElemEq = unsafe extern "C" fn(u64, u64) -> i8;

    unsafe {
        let len_a = list_len(list_a);
        let len_b = list_len(list_b);
        if len_a != len_b {
            return 0;
        }
        let data_a = list_data(list_a);
        let data_b = list_data(list_b);
        let f: ElemEq = std::mem::transmute(elem_eq);
        for i in 0..len_a as usize {
            if f(*data_a.add(i), *data_b.add(i)) == 0 {
                return 0;
            }
        }
        1
    }
}

/// Compare two lists lexicographically using an element-comparison callback.
///
/// `elem_cmp` is a bare function pointer `fn(u64, u64) -> i64` that returns
/// negative if a < b, 0 if equal, positive if a > b. Returns negative/0/positive
/// for the lexicographic ordering of the two lists.
#[no_mangle]
pub extern "C" fn snow_list_compare(
    list_a: *mut u8,
    list_b: *mut u8,
    elem_cmp: *mut u8,
) -> i64 {
    type ElemCmp = unsafe extern "C" fn(u64, u64) -> i64;

    unsafe {
        let len_a = list_len(list_a) as usize;
        let len_b = list_len(list_b) as usize;
        let data_a = list_data(list_a);
        let data_b = list_data(list_b);
        let f: ElemCmp = std::mem::transmute(elem_cmp);
        let min_len = len_a.min(len_b);
        for i in 0..min_len {
            let cmp = f(*data_a.add(i), *data_b.add(i));
            if cmp != 0 {
                return cmp;
            }
        }
        if len_a < len_b {
            -1
        } else if len_a > len_b {
            1
        } else {
            0
        }
    }
}

/// Convert a list to a human-readable SnowString: `[elem1, elem2, ...]`.
///
/// `elem_to_str` is a bare function pointer `fn(u64) -> *mut u8` that converts
/// each element (stored as a uniform u64) to a SnowString pointer. The MIR
/// lowerer passes the appropriate runtime to_string function (e.g.,
/// `snow_int_to_string` for `List<Int>`).
#[no_mangle]
pub extern "C" fn snow_list_to_string(
    list: *mut u8,
    elem_to_str: *mut u8,
) -> *mut u8 {
    type ElemToStr = unsafe extern "C" fn(u64) -> *mut u8;

    unsafe {
        let len = list_len(list) as usize;
        let data = list_data(list);
        let f: ElemToStr = std::mem::transmute(elem_to_str);

        // Build the result string piece by piece using snow_string_concat.
        let mut result = crate::string::snow_string_new(b"[".as_ptr(), 1) as *mut u8;
        for i in 0..len {
            if i > 0 {
                let sep = crate::string::snow_string_new(b", ".as_ptr(), 2) as *mut u8;
                result = crate::string::snow_string_concat(
                    result as *const crate::string::SnowString,
                    sep as *const crate::string::SnowString,
                ) as *mut u8;
            }
            let elem_str = f(*data.add(i));
            result = crate::string::snow_string_concat(
                result as *const crate::string::SnowString,
                elem_str as *const crate::string::SnowString,
            ) as *mut u8;
        }
        let close = crate::string::snow_string_new(b"]".as_ptr(), 1) as *mut u8;
        result = crate::string::snow_string_concat(
            result as *const crate::string::SnowString,
            close as *const crate::string::SnowString,
        ) as *mut u8;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_list_new_is_empty() {
        snow_rt_init();
        let list = snow_list_new();
        assert_eq!(snow_list_length(list), 0);
    }

    #[test]
    fn test_list_append_and_length() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 10);
        let list = snow_list_append(list, 20);
        let list = snow_list_append(list, 30);
        assert_eq!(snow_list_length(list), 3);
    }

    #[test]
    fn test_list_head_tail() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 1);
        let list = snow_list_append(list, 2);
        let list = snow_list_append(list, 3);
        assert_eq!(snow_list_head(list), 1);
        let tail = snow_list_tail(list);
        assert_eq!(snow_list_length(tail), 2);
        assert_eq!(snow_list_head(tail), 2);
    }

    #[test]
    fn test_list_get() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 100);
        let list = snow_list_append(list, 200);
        let list = snow_list_append(list, 300);
        assert_eq!(snow_list_get(list, 0), 100);
        assert_eq!(snow_list_get(list, 1), 200);
        assert_eq!(snow_list_get(list, 2), 300);
    }

    #[test]
    fn test_list_concat() {
        snow_rt_init();
        let a = snow_list_new();
        let a = snow_list_append(a, 1);
        let a = snow_list_append(a, 2);
        let b = snow_list_new();
        let b = snow_list_append(b, 3);
        let b = snow_list_append(b, 4);
        let c = snow_list_concat(a, b);
        assert_eq!(snow_list_length(c), 4);
        assert_eq!(snow_list_get(c, 0), 1);
        assert_eq!(snow_list_get(c, 3), 4);
    }

    #[test]
    fn test_list_reverse() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 1);
        let list = snow_list_append(list, 2);
        let list = snow_list_append(list, 3);
        let rev = snow_list_reverse(list);
        assert_eq!(snow_list_get(rev, 0), 3);
        assert_eq!(snow_list_get(rev, 1), 2);
        assert_eq!(snow_list_get(rev, 2), 1);
    }

    #[test]
    fn test_list_map() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 1);
        let list = snow_list_append(list, 2);
        let list = snow_list_append(list, 3);

        unsafe extern "C" fn double(x: u64) -> u64 {
            x * 2
        }

        let mapped = snow_list_map(list, double as *mut u8, std::ptr::null_mut());
        assert_eq!(snow_list_length(mapped), 3);
        assert_eq!(snow_list_get(mapped, 0), 2);
        assert_eq!(snow_list_get(mapped, 1), 4);
        assert_eq!(snow_list_get(mapped, 2), 6);
    }

    #[test]
    fn test_list_filter() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 1);
        let list = snow_list_append(list, 2);
        let list = snow_list_append(list, 3);
        let list = snow_list_append(list, 4);

        // Keep only even numbers (value % 2 == 0).
        unsafe extern "C" fn is_even(x: u64) -> u64 {
            if x % 2 == 0 { 1 } else { 0 }
        }

        let filtered = snow_list_filter(list, is_even as *mut u8, std::ptr::null_mut());
        assert_eq!(snow_list_length(filtered), 2);
        assert_eq!(snow_list_get(filtered, 0), 2);
        assert_eq!(snow_list_get(filtered, 1), 4);
    }

    #[test]
    fn test_list_reduce() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 1);
        let list = snow_list_append(list, 2);
        let list = snow_list_append(list, 3);

        unsafe extern "C" fn add(acc: u64, x: u64) -> u64 {
            acc + x
        }

        let sum = snow_list_reduce(list, 0, add as *mut u8, std::ptr::null_mut());
        assert_eq!(sum, 6);
    }

    #[test]
    fn test_list_map_with_closure() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 10);
        let list = snow_list_append(list, 20);

        // Simulate a closure with an environment: add the value stored at env_ptr.
        unsafe extern "C" fn add_env(env: *mut u8, x: u64) -> u64 {
            let offset = *(env as *const u64);
            x + offset
        }

        // Create a fake "environment" that holds the value 5.
        let mut env_val: u64 = 5;
        let env_ptr = &mut env_val as *mut u64 as *mut u8;

        let mapped = snow_list_map(list, add_env as *mut u8, env_ptr);
        assert_eq!(snow_list_get(mapped, 0), 15);
        assert_eq!(snow_list_get(mapped, 1), 25);
    }

    #[test]
    fn test_list_from_array() {
        snow_rt_init();
        let data: [u64; 3] = [10, 20, 30];
        let list = snow_list_from_array(data.as_ptr(), 3);
        assert_eq!(snow_list_length(list), 3);
        assert_eq!(snow_list_get(list, 0), 10);
        assert_eq!(snow_list_get(list, 2), 30);
    }

    #[test]
    fn test_list_empty_reverse() {
        snow_rt_init();
        let list = snow_list_new();
        let rev = snow_list_reverse(list);
        assert_eq!(snow_list_length(rev), 0);
    }

    #[test]
    fn test_list_reduce_empty() {
        snow_rt_init();
        let list = snow_list_new();

        unsafe extern "C" fn add(acc: u64, x: u64) -> u64 {
            acc + x
        }

        let result = snow_list_reduce(list, 42, add as *mut u8, std::ptr::null_mut());
        assert_eq!(result, 42); // Initial value returned unchanged.
    }

    #[test]
    fn test_list_to_string() {
        snow_rt_init();
        let list = snow_list_new();
        let list = snow_list_append(list, 1);
        let list = snow_list_append(list, 2);
        let list = snow_list_append(list, 3);

        let result = snow_list_to_string(
            list,
            crate::string::snow_int_to_string as *mut u8,
        );
        let s = unsafe { &*(result as *const crate::string::SnowString) };
        let text = unsafe { s.as_str() };
        assert_eq!(text, "[1, 2, 3]");
    }

    #[test]
    fn test_list_eq_same() {
        snow_rt_init();
        let a = snow_list_new();
        let a = snow_list_append(a, 1);
        let a = snow_list_append(a, 2);
        let a = snow_list_append(a, 3);
        let b = snow_list_new();
        let b = snow_list_append(b, 1);
        let b = snow_list_append(b, 2);
        let b = snow_list_append(b, 3);

        unsafe extern "C" fn int_eq(a: u64, b: u64) -> i8 {
            if a == b { 1 } else { 0 }
        }

        assert_eq!(snow_list_eq(a, b, int_eq as *mut u8), 1);
    }

    #[test]
    fn test_list_eq_different() {
        snow_rt_init();
        let a = snow_list_new();
        let a = snow_list_append(a, 1);
        let a = snow_list_append(a, 2);
        let b = snow_list_new();
        let b = snow_list_append(b, 1);
        let b = snow_list_append(b, 3);

        unsafe extern "C" fn int_eq(a: u64, b: u64) -> i8 {
            if a == b { 1 } else { 0 }
        }

        assert_eq!(snow_list_eq(a, b, int_eq as *mut u8), 0);
    }

    #[test]
    fn test_list_eq_different_length() {
        snow_rt_init();
        let a = snow_list_new();
        let a = snow_list_append(a, 1);
        let a = snow_list_append(a, 2);
        let b = snow_list_new();
        let b = snow_list_append(b, 1);

        unsafe extern "C" fn int_eq(a: u64, b: u64) -> i8 {
            if a == b { 1 } else { 0 }
        }

        assert_eq!(snow_list_eq(a, b, int_eq as *mut u8), 0);
    }

    #[test]
    fn test_list_compare_less() {
        snow_rt_init();
        let a = snow_list_new();
        let a = snow_list_append(a, 1);
        let a = snow_list_append(a, 2);
        let b = snow_list_new();
        let b = snow_list_append(b, 1);
        let b = snow_list_append(b, 3);

        unsafe extern "C" fn int_cmp(a: u64, b: u64) -> i64 {
            (a as i64) - (b as i64)
        }

        assert!(snow_list_compare(a, b, int_cmp as *mut u8) < 0);
    }

    #[test]
    fn test_list_compare_equal() {
        snow_rt_init();
        let a = snow_list_new();
        let a = snow_list_append(a, 1);
        let a = snow_list_append(a, 2);
        let b = snow_list_new();
        let b = snow_list_append(b, 1);
        let b = snow_list_append(b, 2);

        unsafe extern "C" fn int_cmp(a: u64, b: u64) -> i64 {
            (a as i64) - (b as i64)
        }

        assert_eq!(snow_list_compare(a, b, int_cmp as *mut u8), 0);
    }

    #[test]
    fn test_list_compare_length() {
        snow_rt_init();
        let a = snow_list_new();
        let a = snow_list_append(a, 1);
        let a = snow_list_append(a, 2);
        let b = snow_list_new();
        let b = snow_list_append(b, 1);
        let b = snow_list_append(b, 2);
        let b = snow_list_append(b, 3);

        unsafe extern "C" fn int_cmp(a: u64, b: u64) -> i64 {
            (a as i64) - (b as i64)
        }

        assert!(snow_list_compare(a, b, int_cmp as *mut u8) < 0);
    }

    #[test]
    fn test_list_to_string_empty() {
        snow_rt_init();
        let list = snow_list_new();

        let result = snow_list_to_string(
            list,
            crate::string::snow_int_to_string as *mut u8,
        );
        let s = unsafe { &*(result as *const crate::string::SnowString) };
        let text = unsafe { s.as_str() };
        assert_eq!(text, "[]");
    }
}
