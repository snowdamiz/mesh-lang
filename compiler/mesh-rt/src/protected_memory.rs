//! Owned secret allocations backed by exclusively runtime-owned locked mappings.
//!
//! A mapping is protected before any secret is copied into it. Allocations may
//! share a mapping, but only the final owner can unlock it. Free subranges are
//! erased before reuse; no ordinary allocator pages are locked or unlocked.

use parking_lot::Mutex;
use std::ops::{Deref, DerefMut, Range};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use zeroize::Zeroize;

const ALIGNMENT: usize = 16;
const SLAB_BYTES: usize = 64 * 1024;
const MAX_ALLOCATION: usize = 64 * 1024;
const MAX_MAPPED_BYTES: usize = 64 * 1024 * 1024;

/// Called before the release runtime initializes actors or providers. Failing
/// to establish the process policy must stop initialization, never downgrade.
pub(crate) fn initialize_release_policy() -> Result<(), ProtectionError> {
    static POLICY: OnceLock<Result<(), ProtectionError>> = OnceLock::new();
    *POLICY.get_or_init(platform::initialize_release_policy)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtectionError {
    Limit,
    Unsupported,
    OperatingSystem,
}

struct Budget {
    used: AtomicUsize,
    limit: usize,
}

impl Budget {
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Reservation, ProtectionError> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|&next| next <= self.limit)
            })
            .map_err(|_| ProtectionError::Limit)?;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}

struct Reservation {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

struct Page {
    mapping: platform::Mapping,
    free: Mutex<Vec<Range<usize>>>,
    _reservation: Reservation,
}

// The mapping only exposes disjoint regions through unique ProtectedBytes
// owners. Its free-list mutex serializes allocation/release. Page destruction
// runs only after every owning region and temporary Arc reference is gone.
unsafe impl Send for Page {}
unsafe impl Sync for Page {}

impl Page {
    fn reserve(&self, size: usize) -> Option<usize> {
        let mut free = self.free.lock();
        let index = free.iter().position(|range| range.len() >= size)?;
        let offset = free[index].start;
        free[index].start += size;
        if free[index].is_empty() {
            free.remove(index);
        }
        Some(offset)
    }

    fn release(&self, offset: usize, size: usize) {
        // The allocation being dropped uniquely owns this entire aligned range.
        unsafe {
            std::slice::from_raw_parts_mut(self.mapping.base.as_ptr().add(offset), size).zeroize();
        }
        let mut free = self.free.lock();
        let index = free.partition_point(|range| range.start < offset);
        free.insert(index, offset..offset + size);
        if index + 1 < free.len() && free[index].end == free[index + 1].start {
            free[index].end = free.remove(index + 1).end;
        }
        if index > 0 && free[index - 1].end == free[index].start {
            free[index - 1].end = free.remove(index).end;
        }
    }
}

struct Pool {
    pages: Arc<Mutex<Vec<Arc<Page>>>>,
    budget: Arc<Budget>,
}

impl Pool {
    fn new(limit: usize) -> Self {
        Self {
            pages: Arc::new(Mutex::new(Vec::new())),
            budget: Arc::new(Budget {
                used: AtomicUsize::new(0),
                limit,
            }),
        }
    }

    fn allocate(&self, len: usize) -> Result<ProtectedBytes, ProtectionError> {
        if len > MAX_ALLOCATION {
            return Err(ProtectionError::Limit);
        }
        if len == 0 {
            return Ok(ProtectedBytes {
                allocation: None,
                len: 0,
            });
        }
        let size = len
            .checked_add(ALIGNMENT - 1)
            .ok_or(ProtectionError::Limit)?
            & !(ALIGNMENT - 1);
        let mut pages = self.pages.lock();
        for page in pages.iter() {
            if let Some(offset) = page.reserve(size) {
                return Ok(ProtectedBytes {
                    allocation: Some(Allocation {
                        page: Some(page.clone()),
                        pool: self.pages.clone(),
                        offset,
                        size,
                    }),
                    len,
                });
            }
        }
        let granule = platform::page_size()?;
        let length = SLAB_BYTES
            .max(size)
            .checked_add(granule - 1)
            .ok_or(ProtectionError::Limit)?
            / granule
            * granule;
        let reservation = self.budget.reserve(length)?;
        let mapping = platform::Mapping::new(length)?;
        let page = Arc::new(Page {
            mapping,
            free: Mutex::new(vec![size..length]),
            _reservation: reservation,
        });
        pages.push(page.clone());
        Ok(ProtectedBytes {
            allocation: Some(Allocation {
                page: Some(page),
                pool: self.pages.clone(),
                offset: 0,
                size,
            }),
            len,
        })
    }
}

struct Allocation {
    page: Option<Arc<Page>>,
    pool: Arc<Mutex<Vec<Arc<Page>>>>,
    offset: usize,
    size: usize,
}

impl Drop for Allocation {
    fn drop(&mut self) {
        let mut pool = self.pool.lock();
        let page = self.page.take().expect("live secret allocation");
        page.release(self.offset, self.size);
        // This allocation and the pool are the only owners when the count is
        // two. Serialize final unmapping and budget release with allocation;
        // otherwise an expired Weak can coexist with a still-charged mapping
        // and spuriously exhaust the budget on another thread.
        if Arc::strong_count(&page) == 2 {
            if let Some(index) = pool.iter().position(|entry| Arc::ptr_eq(entry, &page)) {
                pool.swap_remove(index);
            }
        }
        drop(page);
    }
}

/// Never formats secret data or implicitly clones it. A copy is fallible and
/// remains protected independently of the original allocation's lifetime.
pub(crate) struct ProtectedBytes {
    allocation: Option<Allocation>,
    len: usize,
}

impl ProtectedBytes {
    pub(crate) fn zeroed(len: usize) -> Result<Self, ProtectionError> {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(|| Pool::new(MAX_MAPPED_BYTES))
            .allocate(len)
    }

    pub(crate) fn copy_from(value: &[u8]) -> Result<Self, ProtectionError> {
        let mut allocation = Self::zeroed(value.len())?;
        allocation.copy_from_slice(value);
        Ok(allocation)
    }

    fn pointer(&self) -> *mut u8 {
        self.allocation
            .as_ref()
            .map_or(NonNull::<u8>::dangling().as_ptr(), |allocation| unsafe {
                allocation
                    .page
                    .as_ref()
                    .expect("live secret allocation")
                    .mapping
                    .base
                    .as_ptr()
                    .add(allocation.offset)
            })
    }
}

impl Deref for ProtectedBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.pointer(), self.len) }
    }
}

impl DerefMut for ProtectedBytes {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.pointer(), self.len) }
    }
}

impl Zeroize for ProtectedBytes {
    fn zeroize(&mut self) {
        self.deref_mut().zeroize();
    }
}

#[cfg(unix)]
mod platform {
    use super::*;

    #[cfg(test)]
    pub(super) static FAIL_NEXT_UNMAP: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    unsafe fn unmap(address: *mut libc::c_void, length: usize) -> i32 {
        #[cfg(test)]
        if FAIL_NEXT_UNMAP.swap(false, Ordering::SeqCst) {
            eprintln!("C7_UNMAP_FAILURE_INJECTED");
            return -1;
        }
        libc::munmap(address, length)
    }

    pub(super) fn page_size() -> Result<usize, ProtectionError> {
        let value = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        usize::try_from(value)
            .ok()
            .filter(|value| value.is_power_of_two())
            .ok_or(ProtectionError::OperatingSystem)
    }

    pub(super) struct Mapping {
        pub(super) base: NonNull<u8>,
        len: usize,
    }

    fn disable_core_dumps() -> Result<(), ProtectionError> {
        // Per-process only. Do not change system settings or raise a resource
        // limit. This also covers Apple platforms without MADV_DONTDUMP.
        let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) } != 0 {
            return Err(ProtectionError::OperatingSystem);
        }
        if limit.rlim_cur != 0 || limit.rlim_max != 0 {
            limit.rlim_cur = 0;
            limit.rlim_max = 0;
            if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0 {
                return Err(ProtectionError::OperatingSystem);
            }
        }
        Ok(())
    }

    pub(super) fn initialize_release_policy() -> Result<(), ProtectionError> {
        disable_core_dumps()?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if unsafe {
            libc::prctl(
                libc::PR_SET_DUMPABLE,
                0 as libc::c_long,
                0 as libc::c_long,
                0 as libc::c_long,
                0 as libc::c_long,
            )
        } != 0
        {
            return Err(ProtectionError::OperatingSystem);
        }
        Ok(())
    }

    impl Mapping {
        pub(super) fn new(len: usize) -> Result<Self, ProtectionError> {
            disable_core_dumps()?;
            let pointer = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            if pointer == libc::MAP_FAILED {
                return Err(ProtectionError::OperatingSystem);
            }
            // mmap is allowed to return address zero. Do not keep such a mapping
            // because Rust references require non-null pointers.
            let Some(base) = NonNull::new(pointer.cast::<u8>()) else {
                if unsafe { unmap(pointer, len) } != 0 {
                    std::process::abort();
                }
                return Err(ProtectionError::OperatingSystem);
            };
            if unsafe { libc::mlock(pointer, len) } != 0 {
                if unsafe { unmap(pointer, len) } != 0 {
                    std::process::abort();
                }
                return Err(ProtectionError::OperatingSystem);
            }
            let mapping = Self { base, len };
            #[cfg(any(target_os = "linux", target_os = "android"))]
            {
                if unsafe { libc::madvise(pointer, len, libc::MADV_DONTDUMP) } != 0
                    || unsafe { libc::madvise(pointer, len, libc::MADV_DONTFORK) } != 0
                {
                    return Err(ProtectionError::OperatingSystem);
                }
            }
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                extern "C" {
                    fn minherit(address: *mut libc::c_void, length: usize, inherit: i32) -> i32;
                }
                // VM_INHERIT_NONE from the public Mach vm_inherit.h header;
                // older pinned libc releases do not re-export the constant.
                if unsafe { minherit(pointer, len, 2) } != 0
                    || unsafe { libc::madvise(pointer, len, libc::MADV_ZERO_WIRED_PAGES) } != 0
                {
                    return Err(ProtectionError::OperatingSystem);
                }
            }
            #[cfg(not(any(
                target_os = "linux",
                target_os = "android",
                target_os = "macos",
                target_os = "ios"
            )))]
            {
                return Err(ProtectionError::Unsupported);
            }
            Ok(mapping)
        }
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            unsafe {
                std::slice::from_raw_parts_mut(self.base.as_ptr(), self.len).zeroize();
                // munmap implicitly unlocks. Avoid an interval with plaintext
                // in unlocked memory (already erased even if unmapping fails).
                if unmap(self.base.as_ptr().cast(), self.len) != 0 {
                    // Losing track of a still-locked mapping would break the
                    // process-wide physical-memory bound. Never continue.
                    std::process::abort();
                }
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn VirtualAlloc(
            address: *mut c_void,
            size: usize,
            kind: u32,
            protection: u32,
        ) -> *mut c_void;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        fn VirtualLock(address: *const c_void, size: usize) -> i32;
        fn VirtualUnlock(address: *const c_void, size: usize) -> i32;
        fn WerRegisterExcludedMemoryBlock(address: *const c_void, size: u32) -> i32;
        fn WerUnregisterExcludedMemoryBlock(address: *const c_void) -> i32;
        fn WerGetFlags(process: *mut c_void, flags: *mut u32) -> i32;
        fn WerSetFlags(flags: u32) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn GetSystemInfo(info: *mut SystemInfo);
    }
    #[repr(C)]
    struct SystemInfo {
        architecture: u32,
        page_size: u32,
        minimum_address: *mut c_void,
        maximum_address: *mut c_void,
        active_processor_mask: usize,
        processor_count: u32,
        processor_type: u32,
        allocation_granularity: u32,
        processor_level: u16,
        processor_revision: u16,
    }
    pub(super) fn page_size() -> Result<usize, ProtectionError> {
        let mut info: SystemInfo = unsafe { std::mem::zeroed() };
        unsafe {
            GetSystemInfo(&mut info);
        }
        let size = info.page_size as usize;
        if size.is_power_of_two() {
            Ok(size)
        } else {
            Err(ProtectionError::OperatingSystem)
        }
    }
    pub(super) struct Mapping {
        pub(super) base: NonNull<u8>,
        len: usize,
        excluded: bool,
        locked: bool,
    }
    pub(super) fn initialize_release_policy() -> Result<(), ProtectionError> {
        let mut flags = 0;
        // WER_FAULT_REPORTING_FLAG_NOHEAP, preserving every existing process
        // flag. Region exclusions below cover the anonymous mappings as well.
        if unsafe { WerGetFlags(GetCurrentProcess(), &mut flags) } != 0
            || unsafe { WerSetFlags(flags | 1) } != 0
        {
            return Err(ProtectionError::OperatingSystem);
        }
        Ok(())
    }
    impl Mapping {
        pub(super) fn new(len: usize) -> Result<Self, ProtectionError> {
            initialize_release_policy()?;
            let size = u32::try_from(len).map_err(|_| ProtectionError::Limit)?;
            let pointer = unsafe { VirtualAlloc(std::ptr::null_mut(), len, 0x3000, 0x04) };
            let base = NonNull::new(pointer.cast()).ok_or(ProtectionError::OperatingSystem)?;
            let mut mapping = Self {
                base,
                len,
                excluded: false,
                locked: false,
            };
            if unsafe { VirtualLock(pointer, len) } == 0 {
                return Err(ProtectionError::OperatingSystem);
            }
            mapping.locked = true;
            if unsafe { WerRegisterExcludedMemoryBlock(pointer, size) } != 0 {
                return Err(ProtectionError::OperatingSystem);
            }
            mapping.excluded = true;
            Ok(mapping)
        }
    }
    impl Drop for Mapping {
        fn drop(&mut self) {
            unsafe {
                std::slice::from_raw_parts_mut(self.base.as_ptr(), self.len).zeroize();
                if self.excluded && WerUnregisterExcludedMemoryBlock(self.base.as_ptr().cast()) != 0
                {
                    std::process::abort();
                }
                if self.locked && VirtualUnlock(self.base.as_ptr().cast(), self.len) == 0 {
                    std::process::abort();
                }
                if VirtualFree(self.base.as_ptr().cast(), 0, 0x8000) == 0 {
                    std::process::abort();
                }
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;
    pub(super) struct Mapping {
        pub(super) base: NonNull<u8>,
    }
    pub(super) fn page_size() -> Result<usize, ProtectionError> {
        Err(ProtectionError::Unsupported)
    }
    pub(super) fn initialize_release_policy() -> Result<(), ProtectionError> {
        Err(ProtectionError::Unsupported)
    }
    impl Mapping {
        pub(super) fn new(_: usize) -> Result<Self, ProtectionError> {
            Err(ProtectionError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn unmap_failure_after_lock_denial_stops_process() {
        use std::os::unix::process::ExitStatusExt;
        const CHILD: &str = "MESH_C7_UNMAP_FAILURE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &limit) }, 0);
            platform::FAIL_NEXT_UNMAP.store(true, Ordering::SeqCst);
            let pool = Pool::new(SLAB_BYTES);
            assert!(matches!(
                pool.allocate(32),
                Err(ProtectionError::OperatingSystem)
            ));
            eprintln!("C7_UNMAP_CONTINUED_WITH_UNACCOUNTED_MAPPING");
            return;
        }
        let test_name = format!(
            "{}::unmap_failure_after_lock_denial_stops_process",
            module_path!().split_once("::").unwrap().1
        );
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("C7_UNMAP_FAILURE_INJECTED"),
            "cleanup fault injection did not run: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            result.status.signal(),
            Some(libc::SIGABRT),
            "must stop before releasing the budget of an unmapped-failure region: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    #[cfg(unix)]
    fn release_policy_precedes_secret_allocation() {
        initialize_release_policy().unwrap();
        let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) }, 0);
        assert_eq!(limit.rlim_cur, 0);
        assert_eq!(limit.rlim_max, 0);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert_eq!(
            unsafe {
                libc::prctl(
                    libc::PR_GET_DUMPABLE,
                    0 as libc::c_long,
                    0 as libc::c_long,
                    0 as libc::c_long,
                    0 as libc::c_long,
                )
            },
            0
        );
    }

    #[test]
    fn neighbors_remain_live_when_a_slot_is_erased_and_reused() {
        let pool = Pool::new(SLAB_BYTES);
        let mut first = pool.allocate(33).unwrap();
        let mut second = pool.allocate(32).unwrap();
        first.fill(0xd3);
        second.fill(0x7b);
        let old = first.as_ptr();
        let budget = pool.budget.clone();
        assert_eq!(budget.used.load(Ordering::Acquire), SLAB_BYTES);
        drop(first);
        assert_eq!(&*second, &[0x7b; 32]);
        let recycled = pool.allocate(48).unwrap();
        assert_eq!(recycled.as_ptr(), old);
        assert_eq!(&*recycled, &[0; 48]);
        drop(pool);
        assert_eq!(&*second, &[0x7b; 32]);
        drop(recycled);
        assert_eq!(budget.used.load(Ordering::Acquire), SLAB_BYTES);
        drop(second);
        assert_eq!(budget.used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn mapped_budget_includes_copies_and_releases_on_last_owner() {
        let pool = Pool::new(SLAB_BYTES);
        let full = pool.allocate(MAX_ALLOCATION).unwrap();
        assert!(matches!(pool.allocate(1), Err(ProtectionError::Limit)));
        drop(full);
        let small = pool.allocate(1).unwrap();
        assert_eq!(&*small, &[0]);
        assert!(matches!(
            pool.allocate(MAX_ALLOCATION + 1),
            Err(ProtectionError::Limit)
        ));
    }

    #[test]
    fn copies_are_independent_and_empty_allocations_are_safe() {
        let mut first = ProtectedBytes::copy_from(&[0x4a; 32]).unwrap();
        let copy = ProtectedBytes::copy_from(&first).unwrap();
        first.zeroize();
        assert_eq!(&*first, &[0; 32]);
        drop(first);
        assert_eq!(&*copy, &[0x4a; 32]);
        let empty = ProtectedBytes::zeroed(0).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn denied_memlock_fails_closed_and_releases_reserved_budget() {
        const CHILD: &str = "MESH_C7_MEMLOCK_DENIED_CHILD";
        if std::env::var_os(CHILD).is_some() {
            // Change only this spawned test process, never the parent runner.
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &limit) }, 0);
            let pool = Pool::new(SLAB_BYTES);
            assert!(matches!(
                pool.allocate(32),
                Err(ProtectionError::OperatingSystem)
            ));
            assert_eq!(pool.budget.used.load(Ordering::Acquire), 0);
            assert!(pool.pages.lock().is_empty());
            eprintln!("C7_LOCK_DENIAL_CHECKED");
            return;
        }
        let test_name = format!(
            "{}::denied_memlock_fails_closed_and_releases_reserved_budget",
            module_path!().split_once("::").unwrap().1
        );
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success()
                && String::from_utf8_lossy(&result.stderr).contains("C7_LOCK_DENIAL_CHECKED"),
            "child failed: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn concurrent_owners_preserve_disjoint_regions() {
        let pool = Arc::new(Pool::new(SLAB_BYTES));
        let mut workers = Vec::new();
        for id in 1..=8u8 {
            let pool = pool.clone();
            workers.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    let mut first = pool.allocate(257).unwrap();
                    let mut second = pool.allocate(97).unwrap();
                    first.fill(id);
                    second.fill(id + 100);
                    std::thread::yield_now();
                    assert!(first.iter().all(|&value| value == id));
                    drop(first);
                    assert!(second.iter().all(|&value| value == id + 100));
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(pool.budget.used.load(Ordering::Acquire), 0);
    }
}
