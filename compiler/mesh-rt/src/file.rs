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

/// Atomically renames on the same filesystem, replacing an existing target.
/// No copy/delete fallback is used for cross-filesystem errors. Durability is
/// separate: sync the source before renaming and the parent directory after.
#[no_mangle]
pub extern "C" fn mesh_file_rename(
    source: *const MeshString,
    target: *const MeshString,
) -> *mut MeshResult {
    unsafe { unit_result(fs::rename((*source).as_str(), (*target).as_str())) }
}

/// Flushes an existing file's data and metadata, or a directory entry update,
/// to the operating system's durable-storage boundary. Unsupported filesystem
/// and platform operations are errors; they are never reported as success.
#[no_mangle]
pub extern "C" fn mesh_file_sync(path: *const MeshString) -> *mut MeshResult {
    let result = (|| -> std::io::Result<()> {
        let path = unsafe { (*path).as_str() };
        let directory = fs::metadata(path)?.is_dir();
        let mut options = OpenOptions::new();
        options.read(true).write(!directory);
        #[cfg(windows)]
        if directory {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0200_0000); // FILE_FLAG_BACKUP_SEMANTICS
        }
        options.open(path)?.sync_all()
    })();
    unit_result(result)
}

// Version two requires an explicit quota. It never widens the legacy API's
// ceiling or allocates a whole file: individual operations remain <=64 KiB.
const MAX_BINARY_V2_FILE_BYTES: i64 = 1024 * 1024 * 1024;

fn valid_range_v2(offset: i64, length: i64, maximum_size: i64) -> Option<(u64, usize)> {
    let end = offset.checked_add(length)?;
    if maximum_size <= 0
        || maximum_size > MAX_BINARY_V2_FILE_BYTES
        || offset < 0
        || length <= 0
        || length > MAX_BINARY_CHUNK_BYTES
        || end > maximum_size
    {
        None
    } else {
        Some((offset as u64, length as usize))
    }
}

// Nonblocking open reaches the same-descriptor type check even for a Unix
// FIFO with no peer. Regular-file I/O keeps its normal semantics; this is not
// a deadline guarantee for arbitrary devices or filesystems.
fn open_binary_v2(options: &mut OpenOptions, path: &str) -> std::io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options.open(path)
}

fn check_file_quota(file: &fs::File, maximum_size: i64) -> std::io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("path is not a regular file"));
    }
    if metadata.len() > maximum_size as u64 {
        return Err(std::io::Error::other("binary file exceeds explicit quota"));
    }
    Ok(())
}

/// Reads a bounded range under an explicit quota of at most 1 GiB.
/// Descriptor metadata is checked before reading; concurrent writers are not
/// serialized, and this call does not promise an atomic file snapshot.
#[no_mangle]
pub extern "C" fn mesh_file_read_bytes_v2(
    path: *const MeshString,
    offset: i64,
    length: i64,
    maximum_size: i64,
) -> *mut MeshResult {
    let Some((offset, length)) = valid_range_v2(offset, length, maximum_size) else {
        return err_result("invalid binary file quota or range");
    };
    let result = (|| -> std::io::Result<Vec<u8>> {
        let mut file = open_binary_v2(OpenOptions::new().read(true), unsafe { (*path).as_str() })?;
        check_file_quota(&file, maximum_size)?;
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

/// Writes a bounded range under an explicit quota of at most 1 GiB.
/// Invalid arguments are refused before opening the path. An existing regular
/// file must fit the quota before truncation or writing can begin.
#[no_mangle]
pub extern "C" fn mesh_file_write_bytes_v2(
    path: *const MeshString,
    offset: i64,
    bytes: *const MeshBytes,
    truncate: i8,
    maximum_size: i64,
) -> *mut MeshResult {
    if !matches!(truncate, 0 | 1) || (truncate == 1 && offset != 0) {
        return err_result("invalid binary file write");
    }
    let input = unsafe { (*bytes).as_slice() };
    let Some((offset, _)) = valid_range_v2(offset, input.len() as i64, maximum_size) else {
        return err_result("invalid binary file quota or range");
    };
    let result = (|| -> std::io::Result<()> {
        // Never set OpenOptions::truncate here: inspect this same descriptor
        // first so an oversized destination survives a refused operation.
        let mut file = open_binary_v2(
            OpenOptions::new().create(true).write(true).truncate(false),
            unsafe { (*path).as_str() },
        )?;
        check_file_quota(&file, maximum_size)?;
        if truncate == 1 {
            file.set_len(0)?;
        }
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(input)
    })();
    unit_result(result)
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

    fn bytes_of(result: *mut MeshResult) -> Vec<u8> {
        assert_eq!(error_of(result), None, "expected successful byte read");
        unsafe { (*((*result).value as *const MeshBytes)).as_slice().to_vec() }
    }

    // Each potentially blocking FIFO operation runs in its own process. A
    // reached marker distinguishes an actual open hang from test setup failure.
    // The marker is renamed into place after its body is complete, because
    // `fs::write` creates the path before the bytes are visible. A refusal can
    // publish that marker and exit between the parent's read and `try_wait`,
    // so exit means "before the call" only when the finished marker is absent.
    #[cfg(unix)]
    fn publish_fifo_call_marker(marker: &std::path::Path, operation: &str) {
        let mut file_name = marker.file_name().unwrap().to_os_string();
        file_name.push(".pending");
        let pending = marker.with_file_name(file_name);
        fs::write(&pending, operation.as_bytes()).unwrap();
        fs::rename(&pending, marker).unwrap();
    }

    #[cfg(unix)]
    fn fifo_call_marker_published(marker: &std::path::Path, operation: &str) -> bool {
        match fs::read(marker) {
            Ok(bytes) => bytes == operation.as_bytes(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("FIFO call marker unreadable: {error}"),
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess probe invoked only by the bounded FIFO tests"]
    fn quota_v2_fifo_subprocess_probe() {
        mesh_rt_init();
        let path = std::env::var("FILE_QUOTA_FIFO_PATH").unwrap();
        let operation = std::env::var("FILE_QUOTA_FIFO_OPERATION").unwrap();
        let marker = std::env::var("FILE_QUOTA_FIFO_REACHED").unwrap();
        let native = mesh_str(&path);
        let bytes = crate::bytes::mesh_bytes_new(b"x".as_ptr(), 1);
        publish_fifo_call_marker(std::path::Path::new(&marker), &operation);
        let result = match operation.as_str() {
            "read" => mesh_file_read_bytes_v2(native, 0, 1, 16),
            "write" => mesh_file_write_bytes_v2(native, 0, bytes, 1, 16),
            _ => panic!("invalid test-only FIFO operation"),
        };
        assert!(
            error_of(result).is_some(),
            "FIFO must be refused as non-regular"
        );
        println!("FIFO_{operation}_REFUSED");
    }

    #[cfg(unix)]
    fn assert_fifo_refuses_without_peer(operation: &str) {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::FileTypeExt;
        use std::process::{Child, Command, Stdio};
        use std::time::{Duration, Instant};

        struct OwnedProbe(Child);
        impl Drop for OwnedProbe {
            fn drop(&mut self) {
                // kill is harmless after an already reaped exit. Never leave
                // the no-peer child behind on an assertion or setup failure.
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("no-peer");
        let native_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(native_path.as_ptr(), 0o600) }, 0);
        let reached = dir.path().join("reached");
        let stdout_path = dir.path().join("stdout");
        let stderr_path = dir.path().join("stderr");
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "file::tests::quota_v2_fifo_subprocess_probe",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("FILE_QUOTA_FIFO_PATH", &fifo)
            .env("FILE_QUOTA_FIFO_OPERATION", operation)
            .env("FILE_QUOTA_FIFO_REACHED", &reached)
            .stdout(Stdio::from(fs::File::create(&stdout_path).unwrap()))
            .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()))
            .spawn()
            .unwrap();
        let mut child = OwnedProbe(child);
        let setup_deadline = Instant::now() + Duration::from_secs(5);
        let mut early_status = None;
        loop {
            if fifo_call_marker_published(&reached, operation) {
                break;
            }
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(
                    fifo_call_marker_published(&reached, operation),
                    "FIFO probe exited before call marker"
                );
                early_status = Some(status);
                break;
            }
            assert!(
                Instant::now() < setup_deadline,
                "FIFO probe did not reach actual call"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fs::read(&reached).unwrap(), operation.as_bytes());
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = if let Some(status) = early_status {
            Some(status)
        } else {
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    break Some(status);
                }
                if Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        if status.is_none() {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
        }
        let stdout = fs::read_to_string(stdout_path).unwrap();
        let stderr = fs::read_to_string(stderr_path).unwrap();
        println!("FIFO_{operation}_REACHED; child stdout={stdout:?}; stderr={stderr:?}");
        assert!(
            status.is_some(),
            "FIFO {operation} hung after actual call marker; killed and reaped"
        );
        assert!(
            status.unwrap().success(),
            "FIFO {operation} refusal probe failed"
        );
        assert!(stdout.contains(&format!("FIFO_{operation}_REFUSED")));
        assert!(fs::symlink_metadata(fifo).unwrap().file_type().is_fifo());
    }

    #[cfg(unix)]
    #[test]
    fn quota_v2_fifo_read_refuses_without_peer() {
        assert_fifo_refuses_without_peer("read");
    }

    #[cfg(unix)]
    #[test]
    fn quota_v2_fifo_write_refuses_without_peer() {
        assert_fifo_refuses_without_peer("write");
    }

    #[cfg(unix)]
    #[test]
    fn quota_v2_refuses_socket_and_character_device() {
        use std::os::unix::net::UnixListener;
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("socket");
        let _listener = UnixListener::bind(&path).unwrap();
        let one = crate::bytes::mesh_bytes_new(b"x".as_ptr(), 1);
        for path in [path.as_path(), std::path::Path::new("/dev/null")] {
            let native = mesh_str(path.to_str().unwrap());
            assert!(error_of(mesh_file_read_bytes_v2(native, 0, 1, 16)).is_some());
            assert!(error_of(mesh_file_write_bytes_v2(native, 0, one, 1, 16)).is_some());
        }
    }

    #[test]
    fn quota_v2_allows_the_exact_framed_tail_beyond_legacy_limit() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("framed-backup");
        let native = mesh_str(path.to_str().unwrap());
        let quota = 16_796_864;
        let tail = vec![0xa7; 19_648];
        let bytes = crate::bytes::mesh_bytes_new(tail.as_ptr(), tail.len() as u64);
        assert_eq!(
            error_of(mesh_file_write_bytes_v2(
                native, 16_777_216, bytes, 0, quota
            )),
            None
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), quota as u64);
        assert_eq!(
            bytes_of(mesh_file_read_bytes_v2(native, 16_777_216, 19_648, quota)),
            tail
        );
        let before = fs::metadata(&path).unwrap().len();
        assert!(error_of(mesh_file_write_bytes_v2(
            native, 16_777_217, bytes, 0, quota
        ))
        .is_some());
        assert_eq!(fs::metadata(&path).unwrap().len(), before);
    }

    #[test]
    fn quota_v2_invalid_arguments_refuse_before_creating_a_file() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let one = crate::bytes::mesh_bytes_new(b"x".as_ptr(), 1);
        let large_data = vec![0u8; 65_537];
        let large = crate::bytes::mesh_bytes_new(large_data.as_ptr(), large_data.len() as u64);
        let empty = crate::bytes::mesh_bytes_new(b"".as_ptr(), 0);
        let cases = [
            (0, one, 0, 0),
            (0, one, 0, -1),
            (0, one, 0, 1_073_741_825),
            (-1, one, 0, 16),
            (16, one, 0, 16),
            (i64::MAX, one, 0, 16),
            (0, large, 0, 1_073_741_824),
            (0, empty, 0, 16),
            (1, one, 1, 16),
            (0, one, 2, 16),
        ];
        for (index, (offset, bytes, truncate, quota)) in cases.into_iter().enumerate() {
            let path = dir.path().join(format!("invalid-{index}"));
            let native = mesh_str(path.to_str().unwrap());
            assert!(
                error_of(mesh_file_write_bytes_v2(
                    native, offset, bytes, truncate, quota
                ))
                .is_some(),
                "case {index}"
            );
            assert!(!path.exists(), "invalid case {index} created a destination");
        }
        let path = dir.path().join("read-source");
        fs::write(&path, b"ok").unwrap();
        let native = mesh_str(path.to_str().unwrap());
        for (offset, length, quota) in [
            (0, 1, 0),
            (0, 1, -1),
            (0, 1, 1_073_741_825),
            (-1, 1, 16),
            (0, 0, 16),
            (0, 65_537, 1_073_741_824),
            (16, 1, 16),
            (i64::MAX, 1, 16),
        ] {
            assert!(error_of(mesh_file_read_bytes_v2(native, offset, length, quota)).is_some());
        }
        assert_eq!(fs::read(&path).unwrap(), b"ok");
    }

    #[test]
    fn quota_v2_checks_open_file_before_truncation_and_preserves_eof() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing");
        fs::write(&path, b"preserve-me").unwrap();
        let native = mesh_str(path.to_str().unwrap());
        let bytes = crate::bytes::mesh_bytes_new(b"x".as_ptr(), 1);
        for truncate in [0, 1] {
            assert!(error_of(mesh_file_write_bytes_v2(native, 0, bytes, truncate, 4)).is_some());
            assert_eq!(fs::read(&path).unwrap(), b"preserve-me");
        }
        assert!(error_of(mesh_file_read_bytes_v2(native, 0, 1, 4)).is_some());
        let directory = mesh_str(dir.path().to_str().unwrap());
        assert!(error_of(mesh_file_read_bytes_v2(directory, 0, 1, 16)).is_some());
        assert!(error_of(mesh_file_write_bytes_v2(directory, 0, bytes, 1, 16)).is_some());
        assert_eq!(bytes_of(mesh_file_read_bytes_v2(native, 9, 4, 16)), b"me");
        assert_eq!(bytes_of(mesh_file_read_bytes_v2(native, 11, 1, 16)), b"");
        assert_eq!(
            error_of(mesh_file_write_bytes_v2(native, 0, bytes, 1, 11)),
            None
        );
        assert_eq!(fs::read(&path).unwrap(), b"x");
    }

    #[test]
    fn quota_v2_preserves_legacy_limit_and_propagates_os_errors() {
        mesh_rt_init();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy");
        let native = mesh_str(path.to_str().unwrap());
        let bytes = crate::bytes::mesh_bytes_new(b"x".as_ptr(), 1);
        assert_eq!(
            error_of(mesh_file_write_bytes(native, 16_777_215, bytes, 0)),
            None
        );
        assert_eq!(bytes_of(mesh_file_read_bytes(native, 16_777_215, 1)), b"x");
        assert!(error_of(mesh_file_write_bytes(native, 16_777_216, bytes, 0)).is_some());
        assert!(error_of(mesh_file_read_bytes(native, 16_777_216, 1)).is_some());
        let missing = mesh_str(dir.path().join("missing/child").to_str().unwrap());
        assert!(error_of(mesh_file_write_bytes_v2(missing, 0, bytes, 0, 16)).is_some());
        assert!(error_of(mesh_file_read_bytes_v2(missing, 0, 1, 16)).is_some());
        assert_eq!(fs::metadata(&path).unwrap().len(), 16_777_216);
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
