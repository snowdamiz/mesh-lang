//! Locked, dump-excluded memory for the secret resource table, and the
//! release-build switch that turns core dumps off.
//!
//! Every secret the resource table holds is copied into a pool of 64 KiB
//! chunks mapped for secrets alone. Each chunk is locked into RAM (`mlock`;
//! `VirtualLock` on Windows) so the kernel never writes it to swap, and on
//! Linux and Android it is excluded from core dumps (`MADV_DONTDUMP`). A
//! buffer is zeroized before its units go back to the pool. A chunk that
//! empties is unlocked and unmapped, except the first, which is kept for
//! reuse. Dedicated chunks keep secrets dense (a small `RLIMIT_MEMLOCK`
//! covers many of them) and keep lock and dump flags off the ordinary heap.
//!
//! Locking can fail: `RLIMIT_MEMLOCK` is small on many Linux and Android
//! systems. A chunk that cannot be locked is still used (on Linux it is still
//! left out of core dumps), and when no chunk can be mapped a secret stays on
//! the ordinary heap. The secret works either way; each one placed outside
//! locked memory adds one to `mesh_secret_unlocked_count`.
//! docs/security/secret-memory-model.md states the guarantees per OS.

use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use zeroize::{Zeroize, Zeroizing};

const UNIT: usize = 32;
const CHUNK_BYTES: usize = 64 * 1024;
const UNITS: usize = CHUNK_BYTES / UNIT;
const _: () = assert!(crate::secret::MAX_SECRET_BYTES <= CHUNK_BYTES);

static POOL: Pool = Pool::new();
static UNLOCKED: AtomicU64 = AtomicU64::new(0);

/// How many secrets have been placed outside locked memory since the process
/// started: a chunk that could not be locked, or the ordinary heap when no
/// chunk could be mapped. Zero means every secret was locked.
#[no_mangle]
pub extern "C" fn mesh_secret_unlocked_count() -> u64 {
    UNLOCKED.load(Ordering::Relaxed)
}

/// Turn off core dumps for the rest of this process: the core size limit
/// becomes 0 (soft and hard), and on Linux and Android the process is no
/// longer dumpable, which also covers a `core_pattern` pipe and refuses
/// ptrace attachment by other processes of the same user. `meshc build`
/// calls this first in `main` at `--opt-level 2` and above; debug builds and
/// `meshc test` never do. A host embedding a Mesh library may call it itself.
#[no_mangle]
pub extern "C" fn mesh_rt_disable_core_dumps() {
    #[cfg(unix)]
    unsafe {
        let none = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &none);
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        let zero: libc::c_ulong = 0;
        libc::prctl(libc::PR_SET_DUMPABLE, zero, zero, zero, zero);
    }
}

/// Secret bytes in the pool (or, when no chunk could be mapped, on the heap).
/// Dereferences to the bytes; dropping it zeroizes them before release.
pub(crate) struct SecretBuf {
    ptr: NonNull<u8>,
    len: usize,
    /// `None` when the bytes are a heap `Box<[u8]>`.
    pool: Option<&'static Pool>,
}

// The buffer exclusively owns its bytes, like the `Box<[u8]>` it replaces.
unsafe impl Send for SecretBuf {}
unsafe impl Sync for SecretBuf {}

impl SecretBuf {
    /// Move `bytes` into locked memory. The caller's copy is zeroized when
    /// `bytes` drops here.
    pub(crate) fn new(bytes: Zeroizing<Box<[u8]>>) -> Self {
        Self::new_in(&POOL, bytes)
    }

    fn new_in(pool: &'static Pool, mut bytes: Zeroizing<Box<[u8]>>) -> Self {
        let len = bytes.len();
        if len > 0 {
            match pool.allocate(len) {
                Some((ptr, locked)) => {
                    if !locked {
                        UNLOCKED.fetch_add(1, Ordering::Relaxed);
                    }
                    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), len) };
                    return Self {
                        ptr,
                        len,
                        pool: Some(pool),
                    };
                }
                None => {
                    UNLOCKED.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        let heap = Box::into_raw(std::mem::take(&mut *bytes));
        Self {
            ptr: NonNull::new(heap.cast::<u8>()).expect("a box is never null"),
            len,
            pool: None,
        }
    }

    /// Hand the bytes to a caller that needs an owned heap copy (a consumed
    /// resource); the pooled copy is zeroized and released.
    pub(crate) fn into_inner(self) -> Zeroizing<Box<[u8]>> {
        Zeroizing::new(Box::from(&self[..]))
    }
}

impl Deref for SecretBuf {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl DerefMut for SecretBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for SecretBuf {
    fn drop(&mut self) {
        self.zeroize();
        match self.pool {
            Some(pool) => pool.release(self.ptr, self.len),
            None => drop(unsafe {
                Box::from_raw(ptr::slice_from_raw_parts_mut(self.ptr.as_ptr(), self.len))
            }),
        }
    }
}

pub(crate) struct Pool {
    chunks: Mutex<Vec<Chunk>>,
}

struct Chunk {
    base: NonNull<u8>,
    locked: bool,
    used: [u64; UNITS / 64],
}

// A chunk is a private mapping reached only under the pool lock.
unsafe impl Send for Chunk {}

impl Pool {
    const fn new() -> Self {
        Self {
            chunks: parking_lot::const_mutex(Vec::new()),
        }
    }

    fn allocate(&self, len: usize) -> Option<(NonNull<u8>, bool)> {
        let units = len.div_ceil(UNIT);
        if units > UNITS {
            return None;
        }
        let mut chunks = self.chunks.lock();
        let found = chunks
            .iter()
            .enumerate()
            .find_map(|(index, chunk)| Some((index, chunk.free_run(units)?)));
        let (index, unit) = match found {
            Some(place) => place,
            None => {
                chunks.push(Chunk::map()?);
                (chunks.len() - 1, 0)
            }
        };
        let chunk = &mut chunks[index];
        chunk.mark(unit, units, true);
        let ptr = unsafe { NonNull::new_unchecked(chunk.base.as_ptr().add(unit * UNIT)) };
        Some((ptr, chunk.locked))
    }

    /// Return zeroized units; an emptied chunk other than the first is
    /// unlocked and unmapped.
    fn release(&self, ptr: NonNull<u8>, len: usize) {
        let mut chunks = self.chunks.lock();
        let address = ptr.as_ptr() as usize;
        let index = chunks
            .iter()
            .position(|chunk| {
                let base = chunk.base.as_ptr() as usize;
                (base..base + CHUNK_BYTES).contains(&address)
            })
            .expect("a pooled secret lies in one of its pool's chunks");
        let chunk = &mut chunks[index];
        let unit = (address - chunk.base.as_ptr() as usize) / UNIT;
        chunk.mark(unit, len.div_ceil(UNIT), false);
        if index > 0 && chunk.is_empty() {
            chunks.swap_remove(index).unmap();
        }
    }

    #[cfg(test)]
    fn chunk_count(&self) -> usize {
        self.chunks.lock().len()
    }
}

impl Chunk {
    fn map() -> Option<Self> {
        let (base, locked) = os::map_chunk(CHUNK_BYTES)?;
        Some(Self {
            base,
            locked,
            used: [0; UNITS / 64],
        })
    }

    fn unmap(self) {
        os::unmap_chunk(self.base, CHUNK_BYTES, self.locked);
    }

    fn is_used(&self, unit: usize) -> bool {
        self.used[unit / 64] & (1 << (unit % 64)) != 0
    }

    fn mark(&mut self, start: usize, units: usize, used: bool) {
        for unit in start..start + units {
            if used {
                self.used[unit / 64] |= 1 << (unit % 64);
            } else {
                self.used[unit / 64] &= !(1 << (unit % 64));
            }
        }
    }

    /// The first run of `units` free units (first fit).
    fn free_run(&self, units: usize) -> Option<usize> {
        let mut run = 0;
        for unit in 0..UNITS {
            run = if self.is_used(unit) { 0 } else { run + 1 };
            if run == units {
                return Some(unit + 1 - units);
            }
        }
        None
    }

    fn is_empty(&self) -> bool {
        self.used.iter().all(|word| *word == 0)
    }
}

#[cfg(unix)]
mod os {
    use std::ptr::{self, NonNull};

    pub(super) fn map_chunk(len: usize) -> Option<(NonNull<u8>, bool)> {
        unsafe {
            let base = libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            );
            if base == libc::MAP_FAILED {
                return None;
            }
            // Linux 3.4 and later; the chunk is used whether or not it takes.
            #[cfg(any(target_os = "linux", target_os = "android"))]
            libc::madvise(base, len, libc::MADV_DONTDUMP);
            let locked = libc::mlock(base, len) == 0;
            Some((NonNull::new(base.cast())?, locked))
        }
    }

    pub(super) fn unmap_chunk(base: NonNull<u8>, len: usize, locked: bool) {
        unsafe {
            if locked {
                libc::munlock(base.as_ptr().cast(), len);
            }
            libc::munmap(base.as_ptr().cast(), len);
        }
    }
}

#[cfg(windows)]
mod os {
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_READWRITE: u32 = 0x04;

    #[link(name = "kernel32")]
    extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        fn VirtualLock(address: *mut c_void, size: usize) -> i32;
        fn VirtualUnlock(address: *mut c_void, size: usize) -> i32;
    }

    pub(super) fn map_chunk(len: usize) -> Option<(NonNull<u8>, bool)> {
        unsafe {
            let base = VirtualAlloc(
                ptr::null_mut(),
                len,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            );
            let base = NonNull::new(base.cast::<u8>())?;
            let locked = VirtualLock(base.as_ptr().cast(), len) != 0;
            Some((base, locked))
        }
    }

    pub(super) fn unmap_chunk(base: NonNull<u8>, len: usize, locked: bool) {
        unsafe {
            if locked {
                VirtualUnlock(base.as_ptr().cast(), len);
            }
            VirtualFree(base.as_ptr().cast(), 0, MEM_RELEASE);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    use std::ptr::NonNull;

    pub(super) fn map_chunk(_len: usize) -> Option<(NonNull<u8>, bool)> {
        None
    }

    pub(super) fn unmap_chunk(_base: NonNull<u8>, _len: usize, _locked: bool) {}
}

/// What the kernel says about the page holding `address`: whether it is
/// locked in RAM and whether it is left out of core dumps (`None` where the
/// OS has no such flag). Tests use it to check the pool against the kernel.
#[cfg(test)]
pub(crate) fn kernel_page_flags(address: usize) -> (bool, Option<bool>) {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let smaps = std::fs::read_to_string("/proc/self/smaps").expect("/proc/self/smaps");
        let mut inside = false;
        for line in smaps.lines() {
            if let Some((range, _)) = line.split_once(' ') {
                if let Some((start, end)) = range.split_once('-') {
                    if let (Ok(start), Ok(end)) = (
                        usize::from_str_radix(start, 16),
                        usize::from_str_radix(end, 16),
                    ) {
                        inside = (start..end).contains(&address);
                        continue;
                    }
                }
            }
            if let Some(flags) = line.strip_prefix("VmFlags:").filter(|_| inside) {
                let flags: Vec<_> = flags.split_whitespace().collect();
                return (flags.contains(&"lo"), Some(flags.contains(&"dd")));
            }
        }
        panic!("no mapping holds {address:#x}");
    }
    #[cfg(target_vendor = "apple")]
    {
        #[repr(C, packed(4))]
        #[derive(Default)]
        struct RegionBasicInfo64 {
            protection: i32,
            max_protection: i32,
            inheritance: u32,
            shared: u32,
            reserved: u32,
            offset: u64,
            behavior: i32,
            user_wired_count: u16,
        }
        const VM_REGION_BASIC_INFO_64: i32 = 9;
        extern "C" {
            static mach_task_self_: u32;
            fn mach_vm_region(
                task: u32,
                address: *mut u64,
                size: *mut u64,
                flavor: i32,
                info: *mut RegionBasicInfo64,
                count: *mut u32,
                object_name: *mut u32,
            ) -> i32;
        }
        let mut region = address as u64;
        let mut size = 0;
        let mut info = RegionBasicInfo64::default();
        let mut count = (std::mem::size_of::<RegionBasicInfo64>() / 4) as u32;
        let mut object_name = 0;
        let status = unsafe {
            mach_vm_region(
                mach_task_self_,
                &mut region,
                &mut size,
                VM_REGION_BASIC_INFO_64,
                &mut info,
                &mut count,
                &mut object_name,
            )
        };
        assert_eq!(status, 0, "mach_vm_region");
        assert!((region..region + size).contains(&(address as u64)));
        // XNU has no per-region core-dump exclusion.
        (info.user_wired_count > 0, None)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    {
        let _ = address;
        unimplemented!("no kernel page query on this OS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_pool() -> &'static Pool {
        Box::leak(Box::new(Pool::new()))
    }

    fn secret(byte: u8, len: usize) -> Zeroizing<Box<[u8]>> {
        Zeroizing::new(vec![byte; len].into_boxed_slice())
    }

    #[test]
    fn pooled_secrets_are_locked_and_zeroized_before_their_units_return() {
        let pool = private_pool();
        let buffer = SecretBuf::new_in(pool, secret(0xA5, 40));
        assert_eq!(&buffer[..], &[0xA5; 40]);
        let address = buffer.as_ptr() as usize;
        let (locked, dump_excluded) = kernel_page_flags(address);
        assert!(
            locked,
            "the kernel does not report the secret's page locked"
        );
        assert_ne!(
            dump_excluded,
            Some(false),
            "the secret's page is in core dumps"
        );

        drop(buffer);
        let freed = unsafe { std::slice::from_raw_parts(address as *const u8, 40) };
        assert!(freed.iter().all(|byte| *byte == 0));
        let reused = SecretBuf::new_in(pool, secret(0x5A, 64));
        assert_eq!(reused.as_ptr() as usize, address);
    }

    #[test]
    fn an_emptied_chunk_is_unmapped_unless_it_is_the_first() {
        let pool = private_pool();
        let first = SecretBuf::new_in(pool, secret(1, CHUNK_BYTES));
        let second = SecretBuf::new_in(pool, secret(2, UNIT + 1));
        assert_eq!(pool.chunk_count(), 2);
        drop(second);
        assert_eq!(pool.chunk_count(), 1);
        drop(first);
        assert_eq!(pool.chunk_count(), 1);
        let empty = SecretBuf::new_in(pool, secret(3, 0));
        assert!(empty.is_empty());
    }
}
