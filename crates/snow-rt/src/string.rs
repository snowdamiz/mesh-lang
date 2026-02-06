//! GC-managed string operations for the Snow runtime.
//!
//! Strings in Snow are length-prefixed, UTF-8, GC-managed values. The layout
//! is `{ len: u64, data: [u8; len] }` -- the data bytes immediately follow
//! the length field in memory.
//!
//! All string functions allocate via `snow_gc_alloc` so they are managed by
//! the GC arena.

use std::ptr;

use crate::gc::snow_gc_alloc;

/// A GC-managed Snow string.
///
/// Layout: `[u64 len][u8 data...]` -- the `data` bytes immediately follow
/// the `len` field in memory. This struct only declares the `len` field;
/// the data is accessed by pointer arithmetic past the struct.
#[repr(C)]
pub struct SnowString {
    pub len: u64,
    // data bytes follow immediately after this struct in memory
}

impl SnowString {
    /// Size of the header (the `len` field).
    const HEADER_SIZE: usize = std::mem::size_of::<u64>();

    /// Get a pointer to the data bytes following the header.
    ///
    /// # Safety
    ///
    /// Caller must ensure `self` points to a valid SnowString allocation
    /// with at least `self.len` bytes following the header.
    pub unsafe fn data_ptr(&self) -> *const u8 {
        (self as *const Self as *const u8).add(Self::HEADER_SIZE)
    }

    /// Get a mutable pointer to the data bytes following the header.
    ///
    /// # Safety
    ///
    /// Caller must ensure `self` points to a valid, mutable SnowString
    /// allocation with at least `self.len` bytes following the header.
    pub unsafe fn data_ptr_mut(&mut self) -> *mut u8 {
        (self as *mut Self as *mut u8).add(Self::HEADER_SIZE)
    }

    /// View the string data as a byte slice.
    ///
    /// # Safety
    ///
    /// Caller must ensure the SnowString was properly initialized with
    /// valid UTF-8 data of length `self.len`.
    pub unsafe fn as_bytes(&self) -> &[u8] {
        std::slice::from_raw_parts(self.data_ptr(), self.len as usize)
    }

    /// View the string data as a `&str`.
    ///
    /// # Safety
    ///
    /// Caller must ensure the SnowString contains valid UTF-8.
    pub unsafe fn as_str(&self) -> &str {
        std::str::from_utf8_unchecked(self.as_bytes())
    }
}

/// Create a new GC-managed Snow string from raw bytes.
///
/// Allocates `sizeof(u64) + len` bytes from the GC arena, copies `data`
/// into the allocation, and returns a pointer to the new `SnowString`.
///
/// # Safety
///
/// `data` must point to at least `len` valid bytes. If `data` is null,
/// the string data is zeroed.
#[no_mangle]
pub extern "C" fn snow_string_new(data: *const u8, len: u64) -> *mut SnowString {
    unsafe {
        let total = SnowString::HEADER_SIZE + len as usize;
        let ptr = snow_gc_alloc(total as u64, 8) as *mut SnowString;
        (*ptr).len = len;
        if !data.is_null() && len > 0 {
            let dst = (*ptr).data_ptr_mut();
            ptr::copy_nonoverlapping(data, dst, len as usize);
        }
        ptr
    }
}

/// Concatenate two Snow strings, returning a new GC-managed string.
#[no_mangle]
pub extern "C" fn snow_string_concat(
    a: *const SnowString,
    b: *const SnowString,
) -> *mut SnowString {
    unsafe {
        let a_len = (*a).len;
        let b_len = (*b).len;
        let new_len = a_len + b_len;
        let result = snow_string_new(ptr::null(), new_len);
        let dst = (*result).data_ptr_mut();
        ptr::copy_nonoverlapping((*a).data_ptr(), dst, a_len as usize);
        ptr::copy_nonoverlapping((*b).data_ptr(), dst.add(a_len as usize), b_len as usize);
        result
    }
}

/// Convert an i64 integer to a GC-managed Snow string.
#[no_mangle]
pub extern "C" fn snow_int_to_string(val: i64) -> *mut SnowString {
    let s = val.to_string();
    snow_string_new(s.as_ptr(), s.len() as u64)
}

/// Convert an f64 float to a GC-managed Snow string.
#[no_mangle]
pub extern "C" fn snow_float_to_string(val: f64) -> *mut SnowString {
    let s = val.to_string();
    snow_string_new(s.as_ptr(), s.len() as u64)
}

/// Convert a boolean (i8: 0 = false, non-zero = true) to a GC-managed Snow string.
#[no_mangle]
pub extern "C" fn snow_bool_to_string(val: i8) -> *mut SnowString {
    let s = if val != 0 { "true" } else { "false" };
    snow_string_new(s.as_ptr(), s.len() as u64)
}

/// Print a Snow string to stdout (no trailing newline).
#[no_mangle]
pub extern "C" fn snow_print(s: *const SnowString) {
    unsafe {
        let text = (*s).as_str();
        print!("{}", text);
    }
}

/// Print a Snow string to stdout with a trailing newline.
#[no_mangle]
pub extern "C" fn snow_println(s: *const SnowString) {
    unsafe {
        let text = (*s).as_str();
        println!("{}", text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_string_new_and_read() {
        snow_rt_init();
        let data = b"hello";
        let s = snow_string_new(data.as_ptr(), data.len() as u64);
        unsafe {
            assert_eq!((*s).len, 5);
            assert_eq!((*s).as_str(), "hello");
        }
    }

    #[test]
    fn test_string_concat() {
        snow_rt_init();
        let a = snow_string_new(b"hello ".as_ptr(), 6);
        let b = snow_string_new(b"world".as_ptr(), 5);
        let result = snow_string_concat(a, b);
        unsafe {
            assert_eq!((*result).len, 11);
            assert_eq!((*result).as_str(), "hello world");
        }
    }

    #[test]
    fn test_int_to_string() {
        snow_rt_init();
        let s = snow_int_to_string(42);
        unsafe {
            assert_eq!((*s).as_str(), "42");
        }
    }

    #[test]
    fn test_int_to_string_negative() {
        snow_rt_init();
        let s = snow_int_to_string(-123);
        unsafe {
            assert_eq!((*s).as_str(), "-123");
        }
    }

    #[test]
    fn test_float_to_string() {
        snow_rt_init();
        let s = snow_float_to_string(3.14);
        unsafe {
            let text = (*s).as_str();
            assert!(text.starts_with("3.14"), "got: {}", text);
        }
    }

    #[test]
    fn test_bool_to_string() {
        snow_rt_init();
        let t = snow_bool_to_string(1);
        let f = snow_bool_to_string(0);
        unsafe {
            assert_eq!((*t).as_str(), "true");
            assert_eq!((*f).as_str(), "false");
        }
    }

    #[test]
    fn test_empty_string() {
        snow_rt_init();
        let s = snow_string_new(std::ptr::null(), 0);
        unsafe {
            assert_eq!((*s).len, 0);
            assert_eq!((*s).as_str(), "");
        }
    }
}
