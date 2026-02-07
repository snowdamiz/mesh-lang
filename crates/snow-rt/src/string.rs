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

// ── String operations (Phase 8) ──────────────────────────────────────────

/// Return the number of Unicode codepoints in the string (NOT byte length).
#[no_mangle]
pub extern "C" fn snow_string_length(s: *const SnowString) -> i64 {
    unsafe { (*s).as_str().chars().count() as i64 }
}

/// Codepoint-based slice (0-indexed, exclusive end). Clamps to bounds.
#[no_mangle]
pub extern "C" fn snow_string_slice(
    s: *const SnowString,
    start: i64,
    end: i64,
) -> *mut SnowString {
    unsafe {
        let text = (*s).as_str();
        let char_count = text.chars().count();
        let start = (start.max(0) as usize).min(char_count);
        let end = (end.max(0) as usize).min(char_count).max(start);
        let sliced: String = text.chars().skip(start).take(end - start).collect();
        snow_string_new(sliced.as_ptr(), sliced.len() as u64)
    }
}

/// Returns 1 if haystack contains needle, 0 otherwise.
#[no_mangle]
pub extern "C" fn snow_string_contains(
    haystack: *const SnowString,
    needle: *const SnowString,
) -> i8 {
    unsafe {
        if (*haystack).as_str().contains((*needle).as_str()) {
            1
        } else {
            0
        }
    }
}

/// Returns 1 if string starts with prefix, 0 otherwise.
#[no_mangle]
pub extern "C" fn snow_string_starts_with(
    s: *const SnowString,
    prefix: *const SnowString,
) -> i8 {
    unsafe {
        if (*s).as_str().starts_with((*prefix).as_str()) {
            1
        } else {
            0
        }
    }
}

/// Returns 1 if string ends with suffix, 0 otherwise.
#[no_mangle]
pub extern "C" fn snow_string_ends_with(
    s: *const SnowString,
    suffix: *const SnowString,
) -> i8 {
    unsafe {
        if (*s).as_str().ends_with((*suffix).as_str()) {
            1
        } else {
            0
        }
    }
}

/// Trim whitespace from both sides.
#[no_mangle]
pub extern "C" fn snow_string_trim(s: *const SnowString) -> *mut SnowString {
    unsafe {
        let trimmed = (*s).as_str().trim();
        snow_string_new(trimmed.as_ptr(), trimmed.len() as u64)
    }
}

/// Convert to uppercase.
#[no_mangle]
pub extern "C" fn snow_string_to_upper(s: *const SnowString) -> *mut SnowString {
    unsafe {
        let upper = (*s).as_str().to_uppercase();
        snow_string_new(upper.as_ptr(), upper.len() as u64)
    }
}

/// Convert to lowercase.
#[no_mangle]
pub extern "C" fn snow_string_to_lower(s: *const SnowString) -> *mut SnowString {
    unsafe {
        let lower = (*s).as_str().to_lowercase();
        snow_string_new(lower.as_ptr(), lower.len() as u64)
    }
}

/// Replace all occurrences of `from` with `to` in the string.
#[no_mangle]
pub extern "C" fn snow_string_replace(
    s: *const SnowString,
    from: *const SnowString,
    to: *const SnowString,
) -> *mut SnowString {
    unsafe {
        let result = (*s)
            .as_str()
            .replace((*from).as_str(), (*to).as_str());
        snow_string_new(result.as_ptr(), result.len() as u64)
    }
}

/// Compare two Snow strings for equality. Returns 1 if equal, 0 otherwise.
#[no_mangle]
pub extern "C" fn snow_string_eq(a: *const SnowString, b: *const SnowString) -> i8 {
    unsafe {
        if (*a).as_str() == (*b).as_str() {
            1
        } else {
            0
        }
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

    // ── Phase 8 string operation tests ─────────────────────────────────

    #[test]
    fn test_string_length() {
        snow_rt_init();
        let s = snow_string_new(b"hello".as_ptr(), 5);
        assert_eq!(snow_string_length(s), 5);
    }

    #[test]
    fn test_string_length_unicode() {
        snow_rt_init();
        // "cafe\u{0301}" = "cafe\u{0301}" -- 5 codepoints, but more bytes
        let text = "caf\u{00e9}"; // 4 codepoints, e-with-accent is 2 bytes
        let s = snow_string_new(text.as_ptr(), text.len() as u64);
        assert_eq!(snow_string_length(s), 4);
    }

    #[test]
    fn test_string_length_empty() {
        snow_rt_init();
        let s = snow_string_new(std::ptr::null(), 0);
        assert_eq!(snow_string_length(s), 0);
    }

    #[test]
    fn test_string_slice() {
        snow_rt_init();
        let s = snow_string_new(b"hello world".as_ptr(), 11);
        let sliced = snow_string_slice(s, 0, 5);
        unsafe {
            assert_eq!((*sliced).as_str(), "hello");
        }
    }

    #[test]
    fn test_string_slice_clamp() {
        snow_rt_init();
        let s = snow_string_new(b"abc".as_ptr(), 3);
        // Out of bounds clamps
        let sliced = snow_string_slice(s, -5, 100);
        unsafe {
            assert_eq!((*sliced).as_str(), "abc");
        }
    }

    #[test]
    fn test_string_contains() {
        snow_rt_init();
        let hay = snow_string_new(b"hello world".as_ptr(), 11);
        let needle = snow_string_new(b"world".as_ptr(), 5);
        let missing = snow_string_new(b"xyz".as_ptr(), 3);
        assert_eq!(snow_string_contains(hay, needle), 1);
        assert_eq!(snow_string_contains(hay, missing), 0);
    }

    #[test]
    fn test_string_starts_with() {
        snow_rt_init();
        let s = snow_string_new(b"hello world".as_ptr(), 11);
        let prefix = snow_string_new(b"hello".as_ptr(), 5);
        let wrong = snow_string_new(b"world".as_ptr(), 5);
        assert_eq!(snow_string_starts_with(s, prefix), 1);
        assert_eq!(snow_string_starts_with(s, wrong), 0);
    }

    #[test]
    fn test_string_ends_with() {
        snow_rt_init();
        let s = snow_string_new(b"hello world".as_ptr(), 11);
        let suffix = snow_string_new(b"world".as_ptr(), 5);
        let wrong = snow_string_new(b"hello".as_ptr(), 5);
        assert_eq!(snow_string_ends_with(s, suffix), 1);
        assert_eq!(snow_string_ends_with(s, wrong), 0);
    }

    #[test]
    fn test_string_trim() {
        snow_rt_init();
        let s = snow_string_new(b"  hello  ".as_ptr(), 9);
        let trimmed = snow_string_trim(s);
        unsafe {
            assert_eq!((*trimmed).as_str(), "hello");
        }
    }

    #[test]
    fn test_string_to_upper() {
        snow_rt_init();
        let s = snow_string_new(b"hello".as_ptr(), 5);
        let upper = snow_string_to_upper(s);
        unsafe {
            assert_eq!((*upper).as_str(), "HELLO");
        }
    }

    #[test]
    fn test_string_to_lower() {
        snow_rt_init();
        let s = snow_string_new(b"HELLO".as_ptr(), 5);
        let lower = snow_string_to_lower(s);
        unsafe {
            assert_eq!((*lower).as_str(), "hello");
        }
    }

    #[test]
    fn test_string_replace() {
        snow_rt_init();
        let s = snow_string_new(b"hello world".as_ptr(), 11);
        let from = snow_string_new(b"world".as_ptr(), 5);
        let to = snow_string_new(b"snow".as_ptr(), 4);
        let result = snow_string_replace(s, from, to);
        unsafe {
            assert_eq!((*result).as_str(), "hello snow");
        }
    }

    #[test]
    fn test_string_eq() {
        snow_rt_init();
        let a = snow_string_new(b"hello".as_ptr(), 5);
        let b = snow_string_new(b"hello".as_ptr(), 5);
        let c = snow_string_new(b"world".as_ptr(), 5);
        assert_eq!(snow_string_eq(a, b), 1);
        assert_eq!(snow_string_eq(a, c), 0);
    }
}
