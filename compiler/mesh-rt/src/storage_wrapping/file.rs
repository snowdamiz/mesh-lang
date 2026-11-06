//! Private filesystem storage using the existing material/counter encoding.
//! A directory flock serializes first initialization. The stable lock file
//! serializes all record reads and replacements. The process mutex also
//! serializes threads sharing an open file description. No resource-table
//! lock is held during any filesystem operation.

use super::*;
use crate::secret::StorageCounterReservation;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};

pub(super) const MAX_PATH_BYTES: usize = 4096;
const RECORD: &CStr = c"key";
const LOCK: &CStr = c"lock";
static PROCESS_LOCK: Mutex<()> = Mutex::new(());

fn rejected() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "unsafe or incomplete storage key directory",
    )
}
fn checked_call(result: libc::c_int) -> io::Result<()> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn open_at(dir: RawFd, name: &CStr, flags: libc::c_int, mode: libc::mode_t) -> io::Result<File> {
    let fd = unsafe {
        libc::openat(
            dir,
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            libc::c_uint::from(mode),
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn stat(file: &File) -> io::Result<libc::stat> {
    let mut out = std::mem::MaybeUninit::uninit();
    checked_call(unsafe { libc::fstat(file.as_raw_fd(), out.as_mut_ptr()) })?;
    Ok(unsafe { out.assume_init() })
}
fn private(file: &File, directory: bool, size: Option<usize>) -> io::Result<libc::stat> {
    let value = stat(file)?;
    let kind = if directory {
        libc::S_IFDIR
    } else {
        libc::S_IFREG
    };
    let mode = if directory { 0o700 } else { 0o600 };
    if value.st_mode & libc::S_IFMT != kind
        || value.st_mode & 0o7777 != mode
        || value.st_uid != unsafe { libc::geteuid() }
        || (!directory && value.st_nlink != 1)
        || size.is_some_and(|size| value.st_size != size as libc::off_t)
    {
        return Err(rejected());
    }
    Ok(value)
}
fn same(left: &libc::stat, right: &libc::stat) -> bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
}
fn named_file(dir: &File, name: &CStr, file: &File, size: usize) -> io::Result<()> {
    let actual = private(file, false, Some(size))?;
    let mut named = std::mem::MaybeUninit::<libc::stat>::uninit();
    checked_call(unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            named.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    if !same(&actual, &unsafe { named.assume_init() }) {
        return Err(rejected());
    }
    Ok(())
}
struct Locked<'a>(&'a File);
impl<'a> Locked<'a> {
    fn acquire(file: &'a File) -> io::Result<Self> {
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Self(file));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}
impl Drop for Locked<'_> {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn directory(path: &str, create: bool) -> io::Result<File> {
    // Reject aliases rather than following them and then canonicalizing.
    if path.len() > MAX_PATH_BYTES
        || !path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\0')
    {
        return Err(rejected());
    }
    let components: Vec<_> = path[1..].split('/').collect();
    if components.len() > 64
        || components
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == ".." || part.len() > 255)
    {
        return Err(rejected());
    }
    let mut parent = open_at(libc::AT_FDCWD, c"/", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    for (index, component) in components.iter().enumerate() {
        let component = CString::new(*component).map_err(|_| rejected())?;
        let final_part = index + 1 == components.len();
        let child = match open_at(
            parent.as_raw_fd(),
            &component,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        ) {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound && final_part && create => {
                let made = unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700) };
                if made != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                    return Err(io::Error::last_os_error());
                }
                let child = open_at(
                    parent.as_raw_fd(),
                    &component,
                    libc::O_RDONLY | libc::O_DIRECTORY,
                    0,
                )?;
                parent.sync_all()?;
                child
            }
            Err(error) => return Err(error),
        };
        parent = child;
    }
    private(&parent, true, None)?;
    Ok(parent)
}
fn durable(file: &File) -> io::Result<()> {
    file.sync_all()?;
    #[cfg(target_os = "macos")]
    checked_call(unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) })?;
    Ok(())
}
fn read_record(dir: &File) -> io::Result<Zeroizing<[u8; PLATFORM_RECORD_BYTES]>> {
    let mut file = open_at(dir.as_raw_fd(), RECORD, libc::O_RDONLY, 0)?;
    named_file(dir, RECORD, &file, PLATFORM_RECORD_BYTES)?;
    let mut record = Zeroizing::new([0; PLATFORM_RECORD_BYTES]);
    file.read_exact(&mut *record)?;
    let mut extra = [0];
    if file.read(&mut extra)? != 0 {
        return Err(rejected());
    }
    named_file(dir, RECORD, &file, PLATFORM_RECORD_BYTES)?;
    Ok(record)
}
struct Temporary<'a> {
    dir: &'a File,
    name: CString,
    renamed: bool,
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        if !self.renamed {
            unsafe {
                libc::unlinkat(self.dir.as_raw_fd(), self.name.as_ptr(), 0);
            }
        }
    }
}
fn write_record(dir: &File, record: &[u8; PLATFORM_RECORD_BYTES]) -> io::Result<()> {
    let mut random = [0; 16];
    SystemProvider
        .fill_random(&mut random)
        .map_err(|_| io::Error::other("storage entropy unavailable"))?;
    let name = CString::new(format!(
        ".key-{}",
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
    .expect("hex name");
    let mut temporary = Temporary {
        dir,
        name,
        renamed: false,
    };
    let mut file = open_at(
        dir.as_raw_fd(),
        &temporary.name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o600,
    )?;
    named_file(dir, &temporary.name, &file, 0)?;
    file.write_all(record)?;
    durable(&file)?;
    #[cfg(test)]
    crash_checkpoint("after-file-sync");
    checked_call(unsafe {
        libc::renameat(
            dir.as_raw_fd(),
            temporary.name.as_ptr(),
            dir.as_raw_fd(),
            RECORD.as_ptr(),
        )
    })?;
    temporary.renamed = true;
    #[cfg(test)]
    crash_checkpoint("after-rename");
    dir.sync_all()?;
    // Flush rename metadata through the drive cache on macOS too.
    durable(&file)?;
    #[cfg(test)]
    crash_checkpoint("after-directory-sync");
    named_file(dir, RECORD, &file, PLATFORM_RECORD_BYTES)?;
    Ok(())
}

#[cfg(test)]
fn crash_checkpoint(phase: &str) {
    if std::env::var("MESH_STORAGE_TEST_CRASH").as_deref() == Ok(phase) {
        // Test-only subprocess fault injection: no unwinding or cleanup.
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
        unreachable!("SIGKILL");
    }
}
pub(super) struct FileCounter {
    path: String,
    directory: File,
    lock: File,
    material: Zeroizing<Box<[u8]>>,
}
impl FileCounter {
    fn reserve_inner(&self) -> io::Result<u64> {
        let _process = PROCESS_LOCK.lock();
        let _lock = Locked::acquire(&self.lock)?;
        private(&self.directory, true, None)?;
        let reopened = directory(&self.path, false)?;
        if !same(&stat(&self.directory)?, &stat(&reopened)?) {
            return Err(rejected());
        }
        named_file(&self.directory, LOCK, &self.lock, 0)?;
        let mut record = read_record(&self.directory)?;
        if !bool::from(record[..STORAGE_KEY_MATERIAL_BYTES].ct_eq(&self.material)) {
            return Err(rejected());
        }
        let current = u64::from_be_bytes(
            record[STORAGE_KEY_MATERIAL_BYTES..]
                .try_into()
                .expect("record size"),
        );
        let next = current
            .checked_add(1)
            .ok_or_else(|| io::Error::other("storage counter exhausted"))?;
        record[STORAGE_KEY_MATERIAL_BYTES..].copy_from_slice(&next.to_be_bytes());
        write_record(&self.directory, &record)?;
        #[cfg(test)]
        crash_checkpoint("before-return");
        Ok(current)
    }
}
impl StorageCounterReservation for FileCounter {
    fn reserve(&self) -> Result<u64, StorageKeyError> {
        self.reserve_inner()
            .map_err(|_| StorageKeyError::ReservationFailed)
    }
}
pub(super) fn load(path: &str) -> io::Result<(Zeroizing<Box<[u8]>>, Arc<FileCounter>)> {
    let _process = PROCESS_LOCK.lock();
    let dir = directory(path, true)?;
    let _initialize = Locked::acquire(&dir)?;
    let (lock, new) = match open_at(dir.as_raw_fd(), LOCK, libc::O_RDWR, 0) {
        Ok(lock) => (lock, false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Do this BEFORE creating a lock: a second open must not accept a
            // replacement lock after the first open correctly refused it.
            match open_at(dir.as_raw_fd(), RECORD, libc::O_RDONLY, 0) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => return Err(rejected()),
            }
            (
                open_at(
                    dir.as_raw_fd(),
                    LOCK,
                    libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )?,
                true,
            )
        }
        Err(error) => return Err(error),
    };
    let held = Locked::acquire(&lock)?;
    named_file(&dir, LOCK, &lock, 0)?;
    let record = if new {
        durable(&lock)?;
        dir.sync_all()?;
        let mut record = Zeroizing::new([0; PLATFORM_RECORD_BYTES]);
        SystemProvider
            .fill_random(&mut record[..STORAGE_KEY_MATERIAL_BYTES])
            .map_err(|_| io::Error::other("storage entropy unavailable"))?;
        write_record(&dir, &record)?;
        record
    } else {
        read_record(&dir)?
    };
    let material = Zeroizing::new(
        record[..STORAGE_KEY_MATERIAL_BYTES]
            .to_vec()
            .into_boxed_slice(),
    );
    let source_material = Zeroizing::new(material.to_vec().into_boxed_slice());
    drop(held);
    drop(_initialize);
    let source = Arc::new(FileCounter {
        path: path.to_owned(),
        directory: dir,
        lock,
        material: source_material,
    });
    Ok((material, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    fn path(parent: &tempfile::TempDir) -> String {
        parent
            .path()
            .canonicalize()
            .unwrap()
            .join("private")
            .to_str()
            .unwrap()
            .to_owned()
    }
    #[test]
    fn file_storage_key_survives_reopen() {
        let parent = tempfile::tempdir().unwrap();
        let path = path(&parent);
        let (first, source) = load(&path).unwrap();
        assert_eq!(source.reserve_inner().unwrap(), 0);
        assert_eq!(source.reserve_inner().unwrap(), 1);
        drop(source);
        let (second, reopened) = load(&path).unwrap();
        assert!(
            bool::from(first.ct_eq(&second)),
            "reopen must preserve wrapping material"
        );
        assert_eq!(reopened.reserve_inner().unwrap(), 2);
        assert_eq!(std::fs::metadata(format!("{path}/key")).unwrap().len(), 44);
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o700);
        for name in ["key", "lock"] {
            assert_eq!(
                std::fs::metadata(format!("{path}/{name}")).unwrap().mode() & 0o7777,
                0o600
            );
        }
    }
    #[test]
    fn file_storage_rejects_aliases_unsafe_modes_links_and_incomplete_records() {
        let parent = tempfile::tempdir().unwrap();
        let path = path(&parent);
        let (_, source) = load(&path).unwrap();
        for alias in [
            format!("{path}/"),
            format!("{path}/../private"),
            format!("{path}//."),
            "relative".into(),
            "/".into(),
            format!("{path}\0"),
        ] {
            assert!(load(&alias).is_err());
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(load(&path).is_err());
        assert!(source.reserve_inner().is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = parent.path().join("alias");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(load(alias.to_str().unwrap()).is_err());
        let key = format!("{path}/key");
        std::fs::hard_link(&key, parent.path().join("hard")).unwrap();
        assert!(load(&path).is_err());
        assert!(source.reserve_inner().is_err());
        std::fs::remove_file(parent.path().join("hard")).unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load(&path).is_err());
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let record = Zeroizing::new(std::fs::read(&key).unwrap());
        for length in [0, 1, 35, 36, 43, 45] {
            std::fs::write(&key, vec![0; length]).unwrap();
            assert!(load(&path).is_err());
            assert!(source.reserve_inner().is_err());
        }
        std::fs::write(&key, &*record).unwrap();
        std::fs::remove_file(&key).unwrap();
        assert!(load(&path).is_err());
        assert!(
            !std::path::Path::new(&key).exists(),
            "never replace missing used material"
        );
    }
    #[test]
    fn file_storage_detects_record_or_lock_replacement_and_counter_exhaustion() {
        let parent = tempfile::tempdir().unwrap();
        let path = path(&parent);
        let (_, source) = load(&path).unwrap();
        let key = format!("{path}/key");
        let mut record = Zeroizing::new(std::fs::read(&key).unwrap());
        record[0] ^= 1;
        std::fs::write(&key, &*record).unwrap();
        assert!(source.reserve_inner().is_err());
        record[0] ^= 1;
        record[36..].copy_from_slice(&u64::MAX.to_be_bytes());
        std::fs::write(&key, &*record).unwrap();
        assert!(source.reserve_inner().is_err());
        std::fs::rename(format!("{path}/lock"), format!("{path}/old-lock")).unwrap();
        File::create(format!("{path}/lock")).unwrap();
        assert!(source.reserve_inner().is_err());
    }
    #[test]
    fn file_storage_never_recreates_missing_lock_for_an_existing_key() {
        let parent = tempfile::tempdir().unwrap();
        let path = path(&parent);
        let (_, source) = load(&path).unwrap();
        assert_eq!(source.reserve_inner().unwrap(), 0);
        std::fs::remove_file(format!("{path}/lock")).unwrap();
        for _ in 0..2 {
            assert!(load(&path).is_err());
            assert!(!std::path::Path::new(&format!("{path}/lock")).exists());
        }
        assert!(source.reserve_inner().is_err());
    }
    #[test]
    fn file_storage_rejects_nonregular_and_linked_lock_or_key() {
        for target in ["lock", "key"] {
            for attack in ["symlink", "fifo", "hardlink", "wide-mode", "truncated"] {
                let parent = tempfile::tempdir().unwrap();
                let path = path(&parent);
                let (_, source) = load(&path).unwrap();
                let name = format!("{path}/{target}");
                match attack {
                    "symlink" => {
                        let saved = format!("{path}/saved");
                        std::fs::rename(&name, &saved).unwrap();
                        std::os::unix::fs::symlink(saved, &name).unwrap();
                    }
                    "fifo" => {
                        std::fs::remove_file(&name).unwrap();
                        let name = CString::new(name.clone()).unwrap();
                        checked_call(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }).unwrap();
                    }
                    "hardlink" => std::fs::hard_link(&name, parent.path().join("hard")).unwrap(),
                    "wide-mode" => {
                        std::fs::set_permissions(&name, std::fs::Permissions::from_mode(0o660))
                            .unwrap()
                    }
                    "truncated" => std::fs::write(&name, [0; 1]).unwrap(),
                    _ => unreachable!(),
                }
                assert!(load(&path).is_err(), "{target} {attack}");
                assert!(source.reserve_inner().is_err(), "{target} {attack}");
            }
        }
    }
    #[test]
    fn file_storage_concurrent_initialization_and_reservations_are_unique() {
        let parent = tempfile::tempdir().unwrap();
        let path = path(&parent);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let (_, source) = load(&path).unwrap();
                    (0..16)
                        .map(|_| source.reserve_inner().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut counters: Vec<_> = threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect();
        counters.sort_unstable();
        assert_eq!(counters, (0..128).collect::<Vec<_>>());
    }

    fn worker(path: &str, output: &std::path::Path) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "storage_wrapping::file::tests::file_storage_subprocess_worker",
                "--nocapture",
            ])
            .env("MESH_STORAGE_TEST_PATH", path)
            .env("MESH_STORAGE_TEST_OUTPUT", output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    #[test]
    fn file_storage_subprocess_worker() {
        let Ok(path) = std::env::var("MESH_STORAGE_TEST_PATH") else {
            return;
        };
        let (_, source) = load(&path).unwrap();
        let count = std::env::var("MESH_STORAGE_TEST_COUNT")
            .unwrap_or_else(|_| "1".into())
            .parse::<usize>()
            .unwrap();
        let counters: Vec<_> = (0..count)
            .map(|_| source.reserve_inner().unwrap())
            .collect();
        std::fs::write(
            std::env::var("MESH_STORAGE_TEST_OUTPUT").unwrap(),
            serde_json::to_vec(&counters).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn file_storage_cross_process_initialization_and_counters_are_unique() {
        let parent = tempfile::tempdir().unwrap();
        let path = path(&parent);
        let outputs: Vec<_> = (0..6)
            .map(|index| parent.path().join(format!("counter-{index}")))
            .collect();
        let children: Vec<_> = outputs
            .iter()
            .map(|output| {
                worker(&path, output)
                    .env("MESH_STORAGE_TEST_COUNT", "12")
                    .spawn()
                    .unwrap()
            })
            .collect();
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
        let mut counters: Vec<u64> = outputs
            .iter()
            .flat_map(|output| {
                serde_json::from_slice::<Vec<u64>>(&std::fs::read(output).unwrap()).unwrap()
            })
            .collect();
        counters.sort_unstable();
        assert_eq!(counters, (0..72).collect::<Vec<_>>());
    }

    #[test]
    fn file_storage_crash_before_output_burns_committed_counter() {
        use std::os::unix::process::ExitStatusExt;
        for phase in [
            "after-file-sync",
            "after-rename",
            "after-directory-sync",
            "before-return",
        ] {
            let parent = tempfile::tempdir().unwrap();
            let path = path(&parent);
            let (_, source) = load(&path).unwrap();
            assert_eq!(source.reserve_inner().unwrap(), 0);
            drop(source);
            let output = parent.path().join("output");
            let status = worker(&path, &output)
                .env("MESH_STORAGE_TEST_CRASH", phase)
                .status()
                .unwrap();
            assert_eq!(status.signal(), Some(libc::SIGKILL));
            assert!(!output.exists());
            let (_, reopened) = load(&path).unwrap();
            assert_eq!(
                reopened.reserve_inner().unwrap(),
                if phase == "after-file-sync" { 1 } else { 2 }
            );
        }
    }
}
