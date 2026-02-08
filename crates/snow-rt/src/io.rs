//! Console I/O runtime functions for the Snow standard library.
//!
//! Provides stdin reading and stderr output. Functions match the Snow
//! module `IO` with `read_line` and `eprintln`.

use crate::gc::snow_gc_alloc_actor;
use crate::string::{snow_string_new, SnowString};

/// Tagged result value for Snow's Result<T, E> representation.
///
/// Layout matches the codegen layout for sum types:
/// - tag 0 = Ok (first variant)
/// - tag 1 = Err (second variant)
///
/// The value pointer points to the payload (a SnowString in both cases).
#[repr(C)]
pub struct SnowResult {
    pub tag: u8,
    pub value: *mut u8,
}

/// Allocate a SnowResult on the GC heap.
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

/// Read a line from stdin. Returns a SnowResult (tag 0 = Ok with string,
/// tag 1 = Err with error message string).
///
/// The trailing newline is stripped from the result.
#[no_mangle]
pub extern "C" fn snow_io_read_line() -> *mut SnowResult {
    let mut input = String::new();
    match std::io::stdin().read_line(&mut input) {
        Ok(_) => {
            // Strip trailing newline
            if input.ends_with('\n') {
                input.pop();
                if input.ends_with('\r') {
                    input.pop();
                }
            }
            let s = snow_string_new(input.as_ptr(), input.len() as u64);
            alloc_result(0, s as *mut u8)
        }
        Err(e) => {
            let msg = e.to_string();
            let s = snow_string_new(msg.as_ptr(), msg.len() as u64);
            alloc_result(1, s as *mut u8)
        }
    }
}

/// Print a Snow string to stderr with a trailing newline.
#[no_mangle]
pub extern "C" fn snow_io_eprintln(s: *const SnowString) {
    unsafe {
        let text = (*s).as_str();
        eprintln!("{}", text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_alloc_result_ok() {
        snow_rt_init();
        let s = snow_string_new(b"hello".as_ptr(), 5);
        let result = alloc_result(0, s as *mut u8);
        unsafe {
            assert_eq!((*result).tag, 0);
            let value = (*result).value as *const SnowString;
            assert_eq!((*value).as_str(), "hello");
        }
    }

    #[test]
    fn test_alloc_result_err() {
        snow_rt_init();
        let s = snow_string_new(b"error".as_ptr(), 5);
        let result = alloc_result(1, s as *mut u8);
        unsafe {
            assert_eq!((*result).tag, 1);
            let value = (*result).value as *const SnowString;
            assert_eq!((*value).as_str(), "error");
        }
    }

    #[test]
    fn test_eprintln_does_not_crash() {
        snow_rt_init();
        let s = snow_string_new(b"test error".as_ptr(), 10);
        // Just verify it doesn't panic/crash
        snow_io_eprintln(s);
    }
}
