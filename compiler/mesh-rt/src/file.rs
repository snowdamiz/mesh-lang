//! File I/O runtime functions for the Mesh standard library.
//!
//! Provides file read, write, append, exists, and delete operations.
//! All fallible operations return MeshResult (tag 0 = Ok, tag 1 = Err).

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::io::{alloc_result, err_result, ok_int, MeshResult};
use crate::string::{mesh_str, MeshString};

const MAX_BINARY_CHUNK_BYTES: i64 = 64 * 1024;
// ponytail: 16 MiB is the messenger's reviewed file ceiling; add a versioned
// quota API before generalizing binary range I/O to larger files.
const MAX_BINARY_FILE_BYTES: i64 = 16 * 1024 * 1024;

/// `Ok(())` as a Mesh `Result<(), String>`, or the error's text.
fn unit_result(result: std::io::Result<()>) -> *mut MeshResult {
    match result {
        Ok(()) => alloc_result(0, std::ptr::null_mut()),
        Err(error) => err_result(&error.to_string()),
    }
}

fn valid_range(offset: i64, length: i64) -> Option<(u64, usize)> {
    let end = offset.checked_add(length)?;
    if offset < 0 || length <= 0 || length > MAX_BINARY_CHUNK_BYTES || end > MAX_BINARY_FILE_BYTES {
        None
    } else {
        Some((offset as u64, length as usize))
    }
}

/// Read the entire contents of a file as a UTF-8 string.
///
/// Returns MeshResult:
/// - tag 0 (Ok): value = pointer to MeshString containing file contents
/// - tag 1 (Err): value = pointer to MeshString containing error message
#[no_mangle]
pub extern "C" fn mesh_file_read(path: *const MeshString) -> *mut MeshResult {
    unsafe {
        let path_str = (*path).as_str();
        match fs::read_to_string(path_str) {
            Ok(contents) => {
                let s = mesh_str(&contents);
                alloc_result(0, s as *mut u8)
            }
            Err(e) => err_result(&e.to_string()),
        }
    }
}

/// Reads at most 64 KiB from a bounded byte range without decoding UTF-8.
///
/// Returns an error for negative offsets, zero or oversized lengths, ranges
/// past the 16 MiB ceiling, missing files, and operating-system I/O failures.
#[no_mangle]
pub extern "C" fn mesh_file_read_bytes(
    path: *const MeshString,
    offset: i64,
    length: i64,
) -> *mut MeshResult {
    let Some((offset, length)) = valid_range(offset, length) else {
        return err_result("invalid binary file range");
    };
    unsafe {
        let path = (*path).as_str();
        let result = (|| -> std::io::Result<Vec<u8>> {
            let mut file = fs::File::open(path)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut output = Vec::with_capacity(length);
            file.take(length as u64).read_to_end(&mut output)?;
            Ok(output)
        })();
        match result {
            Ok(output) => alloc_result(
                0,
                mesh_bytes_new(output.as_ptr(), output.len() as u64).cast(),
            ),
            Err(error) => err_result(&error.to_string()),
        }
    }
}

/// Writes one bounded binary chunk at an explicit offset.
///
/// `truncate` is accepted only at offset zero. Inputs are limited to 64 KiB
/// per call and to a 16 MiB resulting range.
#[no_mangle]
pub extern "C" fn mesh_file_write_bytes(
    path: *const MeshString,
    offset: i64,
    bytes: *const MeshBytes,
    truncate: i8,
) -> *mut MeshResult {
    if truncate != 0 && offset != 0 {
        return err_result("invalid binary file write");
    }
    unsafe {
        let input = (*bytes).as_slice();
        let Some((offset, _)) = valid_range(offset, input.len() as i64) else {
            return err_result("invalid binary file range");
        };
        let path = (*path).as_str();
        let result = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(truncate != 0)
                .open(path)?;
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(input)
        })();
        unit_result(result)
    }
}

/// Returns a regular file's byte length when it fits in a Mesh `Int`.
#[no_mangle]
pub extern "C" fn mesh_file_size(path: *const MeshString) -> *mut MeshResult {
    unsafe {
        let length = fs::metadata((*path).as_str()).and_then(|metadata| {
            if !metadata.is_file() {
                Err(std::io::Error::other("path is not a regular file"))
            } else {
                i64::try_from(metadata.len()).map_err(std::io::Error::other)
            }
        });
        match length {
            Ok(length) => ok_int(length),
            Err(error) => err_result(&error.to_string()),
        }
    }
}

/// Write content to a file, creating or overwriting it.
///
/// Returns MeshResult:
/// - tag 0 (Ok): value = null (Unit payload)
/// - tag 1 (Err): value = pointer to MeshString containing error message
#[no_mangle]
pub extern "C" fn mesh_file_write(
    path: *const MeshString,
    content: *const MeshString,
) -> *mut MeshResult {
    unsafe {
        let path_str = (*path).as_str();
        let content_str = (*content).as_str();
        unit_result(fs::write(path_str, content_str))
    }
}

/// Append content to a file, creating it if it doesn't exist.
///
/// Returns MeshResult:
/// - tag 0 (Ok): value = null (Unit payload)
/// - tag 1 (Err): value = pointer to MeshString containing error message
#[no_mangle]
pub extern "C" fn mesh_file_append(
    path: *const MeshString,
    content: *const MeshString,
) -> *mut MeshResult {
    unsafe {
        let path_str = (*path).as_str();
        let content_str = (*content).as_str();
        let file = OpenOptions::new().append(true).create(true).open(path_str);
        unit_result(file.and_then(|mut file| file.write_all(content_str.as_bytes())))
    }
}

/// Check if a file exists at the given path.
///
/// Returns 1 if the file exists, 0 otherwise.
#[no_mangle]
pub extern "C" fn mesh_file_exists(path: *const MeshString) -> i8 {
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
/// Returns MeshResult:
/// - tag 0 (Ok): value = null (Unit payload)
/// - tag 1 (Err): value = pointer to MeshString containing error message
#[no_mangle]
pub extern "C" fn mesh_file_delete(path: *const MeshString) -> *mut MeshResult {
    unsafe { unit_result(fs::remove_file((*path).as_str())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;

    /// The text of a failed `MeshResult`, or `None` for an `Ok`.
    fn error_of(result: *mut MeshResult) -> Option<String> {
        let result = unsafe { &*result };
        (result.tag == 1)
            .then(|| unsafe { (*(result.value as *const MeshString)).as_str() }.to_string())
    }

    /// What the file functions say for a directory, a missing file or
    /// directory, and a byte range or write they cannot take.
    #[test]
    fn file_functions_report_what_they_cannot_do() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let text = |path: std::path::PathBuf| mesh_str(path.to_str().unwrap());
        let (directory, missing) = (
            text(dir.path().to_path_buf()),
            text(dir.path().join("none")),
        );
        let lost = text(dir.path().join("no-dir").join("file"));
        let content = mesh_str("x");
        let bytes = crate::bytes::mesh_bytes_new(b"xy".as_ptr(), 2);

        assert_eq!(
            error_of(mesh_file_size(directory)).unwrap(),
            "path is not a regular file"
        );
        for failed in [
            mesh_file_size(missing),
            mesh_file_read_bytes(missing, 0, 1),
            mesh_file_write_bytes(lost, 0, bytes, 0),
            mesh_file_write(lost, content),
            mesh_file_append(directory, content),
        ] {
            assert!(error_of(failed).is_some());
        }
        let range = error_of(mesh_file_write_bytes(missing, -1, bytes, 0));
        assert_eq!(range.unwrap(), "invalid binary file range");
        let truncate = error_of(mesh_file_write_bytes(missing, 4, bytes, 1));
        assert_eq!(truncate.unwrap(), "invalid binary file write");
    }

    #[test]
    fn test_file_write_and_read() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();

        let path_mesh = mesh_str(path_str);
        let content = mesh_str("Hello, Mesh!");

        // Write
        let write_result = mesh_file_write(path_mesh, content);
        unsafe {
            assert_eq!((*write_result).tag, 0, "write should succeed");
        }

        // Read back
        let read_result = mesh_file_read(path_mesh);
        unsafe {
            assert_eq!((*read_result).tag, 0, "read should succeed");
            let value = (*read_result).value as *const MeshString;
            assert_eq!((*value).as_str(), "Hello, Mesh!");
        }
    }

    #[test]
    fn test_file_read_nonexistent() {
        mesh_rt_init();
        let path_mesh = mesh_str("/tmp/mesh_nonexistent_file_12345.txt");

        let result = mesh_file_read(path_mesh);
        unsafe {
            assert_eq!(
                (*result).tag,
                1,
                "reading nonexistent file should return Err"
            );
            let value = (*result).value as *const MeshString;
            assert!(!value.is_null());
            let msg = (*value).as_str();
            assert!(msg.contains("No such file"), "error msg: {}", msg);
        }
    }

    #[test]
    fn test_file_append() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("append_test.txt");
        let path_str = path.to_str().unwrap();

        let path_mesh = mesh_str(path_str);
        let content1 = mesh_str("Hello");
        let content2 = mesh_str(", Mesh!");

        // Append twice
        let r1 = mesh_file_append(path_mesh, content1);
        unsafe {
            assert_eq!((*r1).tag, 0);
        }

        let r2 = mesh_file_append(path_mesh, content2);
        unsafe {
            assert_eq!((*r2).tag, 0);
        }

        // Read back
        let read_result = mesh_file_read(path_mesh);
        unsafe {
            assert_eq!((*read_result).tag, 0);
            let value = (*read_result).value as *const MeshString;
            assert_eq!((*value).as_str(), "Hello, Mesh!");
        }
    }

    #[test]
    fn test_file_exists() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exists_test.txt");
        let path_str = path.to_str().unwrap();

        let path_mesh = mesh_str(path_str);

        // Should not exist yet
        assert_eq!(mesh_file_exists(path_mesh), 0);

        // Create the file
        let content = mesh_str("test");
        mesh_file_write(path_mesh, content);

        // Should now exist
        assert_eq!(mesh_file_exists(path_mesh), 1);
    }

    #[test]
    fn test_file_delete() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delete_test.txt");
        let path_str = path.to_str().unwrap();

        let path_mesh = mesh_str(path_str);
        let content = mesh_str("to be deleted");

        // Write
        mesh_file_write(path_mesh, content);
        assert_eq!(mesh_file_exists(path_mesh), 1);

        // Delete
        let del_result = mesh_file_delete(path_mesh);
        unsafe {
            assert_eq!((*del_result).tag, 0, "delete should succeed");
        }

        // Should not exist now
        assert_eq!(mesh_file_exists(path_mesh), 0);
    }

    #[test]
    fn test_file_delete_nonexistent() {
        mesh_rt_init();
        let path_mesh = mesh_str("/tmp/mesh_nonexistent_delete_12345.txt");

        let result = mesh_file_delete(path_mesh);
        unsafe {
            assert_eq!(
                (*result).tag,
                1,
                "deleting nonexistent file should return Err"
            );
        }
    }

    #[test]
    fn test_file_full_cycle() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cycle_test.txt");
        let path_str = path.to_str().unwrap();
        let path_mesh = mesh_str(path_str);

        // 1. File does not exist
        assert_eq!(mesh_file_exists(path_mesh), 0);

        // 2. Write
        let content = mesh_str("initial content");
        let r = mesh_file_write(path_mesh, content);
        unsafe {
            assert_eq!((*r).tag, 0);
        }

        // 3. Exists
        assert_eq!(mesh_file_exists(path_mesh), 1);

        // 4. Read
        let r = mesh_file_read(path_mesh);
        unsafe {
            assert_eq!((*r).tag, 0);
            let v = (*r).value as *const MeshString;
            assert_eq!((*v).as_str(), "initial content");
        }

        // 5. Append
        let more = mesh_str(" + appended");
        let r = mesh_file_append(path_mesh, more);
        unsafe {
            assert_eq!((*r).tag, 0);
        }

        // 6. Read again
        let r = mesh_file_read(path_mesh);
        unsafe {
            assert_eq!((*r).tag, 0);
            let v = (*r).value as *const MeshString;
            assert_eq!((*v).as_str(), "initial content + appended");
        }

        // 7. Delete
        let r = mesh_file_delete(path_mesh);
        unsafe {
            assert_eq!((*r).tag, 0);
        }

        // 8. No longer exists
        assert_eq!(mesh_file_exists(path_mesh), 0);
    }
}
