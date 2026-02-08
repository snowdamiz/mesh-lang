//! File I/O runtime functions for the Snow standard library.
//!
//! Provides file read, write, append, exists, and delete operations.
//! All fallible operations return SnowResult (tag 0 = Ok, tag 1 = Err).

use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::gc::snow_gc_alloc_actor;
use crate::io::SnowResult;
use crate::string::{snow_string_new, SnowString};

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

/// Helper to create an Err result with a string message.
fn err_result(msg: &str) -> *mut SnowResult {
    let s = snow_string_new(msg.as_ptr(), msg.len() as u64);
    alloc_result(1, s as *mut u8)
}

/// Read the entire contents of a file as a UTF-8 string.
///
/// Returns SnowResult:
/// - tag 0 (Ok): value = pointer to SnowString containing file contents
/// - tag 1 (Err): value = pointer to SnowString containing error message
#[no_mangle]
pub extern "C" fn snow_file_read(path: *const SnowString) -> *mut SnowResult {
    unsafe {
        let path_str = (*path).as_str();
        match fs::read_to_string(path_str) {
            Ok(contents) => {
                let s = snow_string_new(contents.as_ptr(), contents.len() as u64);
                alloc_result(0, s as *mut u8)
            }
            Err(e) => err_result(&e.to_string()),
        }
    }
}

/// Write content to a file, creating or overwriting it.
///
/// Returns SnowResult:
/// - tag 0 (Ok): value = null (Unit payload)
/// - tag 1 (Err): value = pointer to SnowString containing error message
#[no_mangle]
pub extern "C" fn snow_file_write(
    path: *const SnowString,
    content: *const SnowString,
) -> *mut SnowResult {
    unsafe {
        let path_str = (*path).as_str();
        let content_str = (*content).as_str();
        match fs::write(path_str, content_str) {
            Ok(()) => alloc_result(0, std::ptr::null_mut()),
            Err(e) => err_result(&e.to_string()),
        }
    }
}

/// Append content to a file, creating it if it doesn't exist.
///
/// Returns SnowResult:
/// - tag 0 (Ok): value = null (Unit payload)
/// - tag 1 (Err): value = pointer to SnowString containing error message
#[no_mangle]
pub extern "C" fn snow_file_append(
    path: *const SnowString,
    content: *const SnowString,
) -> *mut SnowResult {
    unsafe {
        let path_str = (*path).as_str();
        let content_str = (*content).as_str();
        match OpenOptions::new()
            .append(true)
            .create(true)
            .open(path_str)
        {
            Ok(mut file) => match file.write_all(content_str.as_bytes()) {
                Ok(()) => alloc_result(0, std::ptr::null_mut()),
                Err(e) => err_result(&e.to_string()),
            },
            Err(e) => err_result(&e.to_string()),
        }
    }
}

/// Check if a file exists at the given path.
///
/// Returns 1 if the file exists, 0 otherwise.
#[no_mangle]
pub extern "C" fn snow_file_exists(path: *const SnowString) -> i8 {
    unsafe {
        let path_str = (*path).as_str();
        if std::path::Path::new(path_str).exists() {
            1
        } else {
            0
        }
    }
}

/// Delete a file at the given path.
///
/// Returns SnowResult:
/// - tag 0 (Ok): value = null (Unit payload)
/// - tag 1 (Err): value = pointer to SnowString containing error message
#[no_mangle]
pub extern "C" fn snow_file_delete(path: *const SnowString) -> *mut SnowResult {
    unsafe {
        let path_str = (*path).as_str();
        match fs::remove_file(path_str) {
            Ok(()) => alloc_result(0, std::ptr::null_mut()),
            Err(e) => err_result(&e.to_string()),
        }
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
    fn test_file_write_and_read() {
        snow_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();

        let path_snow = make_string(path_str);
        let content = make_string("Hello, Snow!");

        // Write
        let write_result = snow_file_write(path_snow, content);
        unsafe {
            assert_eq!((*write_result).tag, 0, "write should succeed");
        }

        // Read back
        let read_result = snow_file_read(path_snow);
        unsafe {
            assert_eq!((*read_result).tag, 0, "read should succeed");
            let value = (*read_result).value as *const SnowString;
            assert_eq!((*value).as_str(), "Hello, Snow!");
        }
    }

    #[test]
    fn test_file_read_nonexistent() {
        snow_rt_init();
        let path_snow = make_string("/tmp/snow_nonexistent_file_12345.txt");

        let result = snow_file_read(path_snow);
        unsafe {
            assert_eq!((*result).tag, 1, "reading nonexistent file should return Err");
            let value = (*result).value as *const SnowString;
            assert!(!value.is_null());
            let msg = (*value).as_str();
            assert!(msg.contains("No such file"), "error msg: {}", msg);
        }
    }

    #[test]
    fn test_file_append() {
        snow_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("append_test.txt");
        let path_str = path.to_str().unwrap();

        let path_snow = make_string(path_str);
        let content1 = make_string("Hello");
        let content2 = make_string(", Snow!");

        // Append twice
        let r1 = snow_file_append(path_snow, content1);
        unsafe { assert_eq!((*r1).tag, 0); }

        let r2 = snow_file_append(path_snow, content2);
        unsafe { assert_eq!((*r2).tag, 0); }

        // Read back
        let read_result = snow_file_read(path_snow);
        unsafe {
            assert_eq!((*read_result).tag, 0);
            let value = (*read_result).value as *const SnowString;
            assert_eq!((*value).as_str(), "Hello, Snow!");
        }
    }

    #[test]
    fn test_file_exists() {
        snow_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exists_test.txt");
        let path_str = path.to_str().unwrap();

        let path_snow = make_string(path_str);

        // Should not exist yet
        assert_eq!(snow_file_exists(path_snow), 0);

        // Create the file
        let content = make_string("test");
        snow_file_write(path_snow, content);

        // Should now exist
        assert_eq!(snow_file_exists(path_snow), 1);
    }

    #[test]
    fn test_file_delete() {
        snow_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delete_test.txt");
        let path_str = path.to_str().unwrap();

        let path_snow = make_string(path_str);
        let content = make_string("to be deleted");

        // Write
        snow_file_write(path_snow, content);
        assert_eq!(snow_file_exists(path_snow), 1);

        // Delete
        let del_result = snow_file_delete(path_snow);
        unsafe {
            assert_eq!((*del_result).tag, 0, "delete should succeed");
        }

        // Should not exist now
        assert_eq!(snow_file_exists(path_snow), 0);
    }

    #[test]
    fn test_file_delete_nonexistent() {
        snow_rt_init();
        let path_snow = make_string("/tmp/snow_nonexistent_delete_12345.txt");

        let result = snow_file_delete(path_snow);
        unsafe {
            assert_eq!((*result).tag, 1, "deleting nonexistent file should return Err");
        }
    }

    #[test]
    fn test_file_full_cycle() {
        snow_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cycle_test.txt");
        let path_str = path.to_str().unwrap();
        let path_snow = make_string(path_str);

        // 1. File does not exist
        assert_eq!(snow_file_exists(path_snow), 0);

        // 2. Write
        let content = make_string("initial content");
        let r = snow_file_write(path_snow, content);
        unsafe { assert_eq!((*r).tag, 0); }

        // 3. Exists
        assert_eq!(snow_file_exists(path_snow), 1);

        // 4. Read
        let r = snow_file_read(path_snow);
        unsafe {
            assert_eq!((*r).tag, 0);
            let v = (*r).value as *const SnowString;
            assert_eq!((*v).as_str(), "initial content");
        }

        // 5. Append
        let more = make_string(" + appended");
        let r = snow_file_append(path_snow, more);
        unsafe { assert_eq!((*r).tag, 0); }

        // 6. Read again
        let r = snow_file_read(path_snow);
        unsafe {
            assert_eq!((*r).tag, 0);
            let v = (*r).value as *const SnowString;
            assert_eq!((*v).as_str(), "initial content + appended");
        }

        // 7. Delete
        let r = snow_file_delete(path_snow);
        unsafe { assert_eq!((*r).tag, 0); }

        // 8. No longer exists
        assert_eq!(snow_file_exists(path_snow), 0);
    }
}
