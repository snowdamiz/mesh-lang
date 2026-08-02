//! Actor-owned storage for opaque secret byte resources.

use crate::actor::heap::{GcHeader, GC_HEADER_SIZE};
use crate::actor::{Process, ProcessId, ProcessState};
use crate::bytes::MeshBytes;
use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, MeshResult};
use parking_lot::Mutex;
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem;
use std::sync::OnceLock;
use zeroize::{Zeroize, Zeroizing};

const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_SECRET_SLOTS: usize = 65_536;
const MAX_SECRETS_PER_ACTOR: usize = 4_096;
const MAX_SECRET_BYTES_PER_ACTOR: usize = 4 * 1024 * 1024;
const MAX_TOTAL_SECRET_BYTES: usize = 64 * 1024 * 1024;
const MAX_SECRET_MAP_CAPACITY: usize = 64;
const MAX_SECRET_MAP_KEY_BYTES: usize = 128;
const HANDLE_ALIGNMENT: usize = mem::align_of::<GcHeader>();

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum CryptoErrorTag {
    InvalidLength = 0,
    InvalidKey = 1,
    InvalidPublicKey = 2,
    InvalidSignature = 3,
    AuthenticationFailed = 4,
    EntropyUnavailable = 5,
    SecretDestroyed = 6,
    ResourceLimitExceeded = 7,
    UnsupportedOperation = 8,
    InternalFailure = 9,
}

/// Runtime representation of the largest `CryptoError` variant.
#[repr(C)]
struct MeshCryptoError {
    tag: u8,
    _padding: [u8; 7],
    expected: i64,
    actual: i64,
}

pub(crate) fn crypto_error(tag: CryptoErrorTag, expected: i64, actual: i64) -> *mut MeshResult {
    let error = mesh_gc_alloc_actor(
        mem::size_of::<MeshCryptoError>() as u64,
        mem::align_of::<MeshCryptoError>() as u64,
    ) as *mut MeshCryptoError;
    unsafe {
        error.write(MeshCryptoError {
            tag: tag as u8,
            _padding: [0; 7],
            expected,
            actual,
        });
    }
    alloc_result(1, error.cast())
}

/// Create an actor-owned secret filled by the operating system CSPRNG.
#[no_mangle]
pub extern "C" fn mesh_secret_random(length: i64) -> *mut MeshResult {
    if length <= 0 || length as u64 > MAX_SECRET_BYTES as u64 {
        return crypto_error(
            CryptoErrorTag::InvalidLength,
            MAX_SECRET_BYTES as i64,
            length,
        );
    }
    let Some(pid) = crate::actor::stack::get_current_pid() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let Some(scheduler) = crate::actor::GLOBAL_SCHEDULER.get() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let Some(process) = scheduler.get_process(pid) else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };

    let result = {
        let mut process = process.lock();
        let handle = {
            let mut table = secret_table().lock();
            create_random_secret_entry(&process, &mut table, length as usize)
        };
        handle.map(|handle| allocate_handle(&mut process, handle))
    };
    match result {
        Ok(handle) => alloc_result(0, handle.cast()),
        Err(CreateSecretError::EntropyUnavailable) => {
            crypto_error(CryptoErrorTag::EntropyUnavailable, 0, 0)
        }
        Err(CreateSecretError::ResourceLimitExceeded) => {
            crypto_error(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
        Err(CreateSecretError::OwnerExited) => crypto_error(CryptoErrorTag::SecretDestroyed, 0, 0),
    }
}

/// Concatenate two actor-owned secrets without exposing either as ordinary bytes.
/// Both inputs are consumed on success or failure.
#[no_mangle]
pub extern "C" fn mesh_secret_concat(
    first: *mut MeshSecretHandle,
    second: *mut MeshSecretHandle,
) -> *mut MeshResult {
    let Some(pid) = crate::actor::stack::get_current_pid() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let Some(scheduler) = crate::actor::GLOBAL_SCHEDULER.get() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let Some(process) = scheduler.get_process(pid) else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let mut process = process.lock();
    if matches!(process.state, ProcessState::Exited(_)) {
        drop(process);
        return crypto_error(CryptoErrorTag::SecretDestroyed, 0, 0);
    }
    let (Some(first_handle), Some(second_handle)) = (
        validate_handle_pointer(&process, first),
        validate_handle_pointer(&process, second),
    ) else {
        destroy_resource_for_process(&process, first, Some(ResourceKind::SecretBytes));
        destroy_resource_for_process(&process, second, Some(ResourceKind::SecretBytes));
        drop(process);
        return crypto_error(CryptoErrorTag::SecretDestroyed, 0, 0);
    };

    let result = match secret_table()
        .lock()
        .concat_secrets(pid, first_handle, second_handle)
    {
        Ok(handle) => Ok(allocate_handle(&mut process, handle)),
        Err(error) => Err(error),
    };
    drop(process);

    match result {
        Ok(handle) => alloc_result(0, handle.cast()),
        Err(ConcatSecretError::InvalidLength { maximum, actual }) => crypto_error(
            CryptoErrorTag::InvalidLength,
            i64::try_from(maximum).unwrap_or(i64::MAX),
            i64::try_from(actual).unwrap_or(i64::MAX),
        ),
        Err(ConcatSecretError::Resource(ResourceError::ResourceLimitExceeded)) => {
            crypto_error(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
        Err(ConcatSecretError::Resource(ResourceError::WrongKind)) => {
            crypto_error(CryptoErrorTag::InvalidKey, 0, 0)
        }
        Err(ConcatSecretError::Resource(_)) => crypto_error(CryptoErrorTag::SecretDestroyed, 0, 0),
    }
}

fn secret_map_error_result(error: SecretMapError) -> *mut MeshResult {
    let tag = match error {
        SecretMapError::InvalidCapacity | SecretMapError::CapacityExceeded => {
            CryptoErrorTag::ResourceLimitExceeded
        }
        SecretMapError::InvalidKey | SecretMapError::DuplicateKey => CryptoErrorTag::InvalidKey,
        SecretMapError::InvalidEncoding => CryptoErrorTag::InternalFailure,
        SecretMapError::Resource(ResourceError::ResourceLimitExceeded) => {
            CryptoErrorTag::ResourceLimitExceeded
        }
        SecretMapError::Resource(ResourceError::WrongKind) => CryptoErrorTag::InvalidKey,
        SecretMapError::Resource(
            ResourceError::StaleHandle | ResourceError::WrongOwner | ResourceError::OwnerExited,
        ) => CryptoErrorTag::SecretDestroyed,
    };
    crypto_error(tag, 0, 0)
}

fn with_current_secret_process<R>(
    operation: impl FnOnce(&mut Process) -> Result<R, SecretMapError>,
) -> Result<R, SecretMapError> {
    let owner = crate::actor::stack::get_current_pid()
        .ok_or(SecretMapError::Resource(ResourceError::OwnerExited))?;
    let scheduler = crate::actor::GLOBAL_SCHEDULER
        .get()
        .ok_or(SecretMapError::Resource(ResourceError::OwnerExited))?;
    let process = scheduler
        .get_process(owner)
        .ok_or(SecretMapError::Resource(ResourceError::OwnerExited))?;
    let mut process = process.lock();
    live_owner(&process).map_err(SecretMapError::Resource)?;
    operation(&mut process)
}

unsafe fn public_secret_map_key(key: *const MeshBytes) -> Result<Vec<u8>, SecretMapError> {
    if key.is_null() || (*key).len == 0 || (*key).len > MAX_SECRET_MAP_KEY_BYTES as u64 {
        return Err(SecretMapError::InvalidKey);
    }
    Ok((*key).as_slice().to_vec())
}

/// Create a zeroizing, actor-owned map with a fixed entry bound.
#[no_mangle]
pub extern "C" fn mesh_secret_map_new(capacity: i64) -> *mut MeshResult {
    let Ok(capacity) = usize::try_from(capacity) else {
        return secret_map_error_result(SecretMapError::InvalidCapacity);
    };
    match with_current_secret_process(|process| {
        let handle = secret_table()
            .lock()
            .insert_secret_map(process.pid, capacity)?;
        Ok(allocate_handle(process, handle))
    }) {
        Ok(handle) => alloc_result(0, handle.cast()),
        Err(error) => secret_map_error_result(error),
    }
}

/// Move one secret into a borrowed map. The secret is destroyed on every error.
#[no_mangle]
pub extern "C" fn mesh_secret_map_insert(
    map: *mut MeshSecretHandle,
    key: *const MeshBytes,
    value: *mut MeshSecretHandle,
) -> *mut MeshResult {
    let key = match unsafe { public_secret_map_key(key) } {
        Ok(key) => key,
        Err(error) => {
            destroy_resource_for_current_actor(value, Some(ResourceKind::SecretBytes));
            return secret_map_error_result(error);
        }
    };
    match with_current_secret_process(|process| {
        let map = validate_handle_pointer(process, map)
            .ok_or(SecretMapError::Resource(ResourceError::StaleHandle));
        let value_handle = validate_handle_pointer(process, value)
            .ok_or(SecretMapError::Resource(ResourceError::StaleHandle));
        let (map, value_handle) = match (map, value_handle) {
            (Ok(map), Ok(value_handle)) => (map, value_handle),
            (_, error) => {
                destroy_resource_for_process(process, value, Some(ResourceKind::SecretBytes));
                return Err(error
                    .err()
                    .unwrap_or(SecretMapError::Resource(ResourceError::StaleHandle)));
            }
        };
        secret_table()
            .lock()
            .secret_map_insert(process.pid, map, &key, value_handle)
    }) {
        Ok(()) => alloc_result(0, std::ptr::null_mut()),
        Err(error) => secret_map_error_result(error),
    }
}

#[no_mangle]
pub extern "C" fn mesh_secret_map_contains(
    map: *const MeshSecretHandle,
    key: *const MeshBytes,
) -> i8 {
    let Ok(key) = (unsafe { public_secret_map_key(key) }) else {
        return 0;
    };
    with_current_secret_process(|process| {
        let handle = validate_handle_pointer(process, map)
            .ok_or(SecretMapError::Resource(ResourceError::StaleHandle))?;
        secret_table()
            .lock()
            .secret_map_contains(process.pid, handle, &key)
    })
    .unwrap_or(false) as i8
}

/// Copy a stored key into a new affine secret while leaving the map unchanged.
#[no_mangle]
pub extern "C" fn mesh_secret_map_copy(
    map: *const MeshSecretHandle,
    key: *const MeshBytes,
) -> *mut MeshResult {
    let key = match unsafe { public_secret_map_key(key) } {
        Ok(key) => key,
        Err(error) => return secret_map_error_result(error),
    };
    match with_current_secret_process(|process| {
        let map = validate_handle_pointer(process, map)
            .ok_or(SecretMapError::Resource(ResourceError::StaleHandle))?;
        let copied = secret_table()
            .lock()
            .secret_map_copy(process.pid, map, &key)?;
        Ok(allocate_handle(process, copied))
    }) {
        Ok(handle) => alloc_result(0, handle.cast()),
        Err(error) => secret_map_error_result(error),
    }
}

#[no_mangle]
pub extern "C" fn mesh_secret_map_delete(
    map: *mut MeshSecretHandle,
    key: *const MeshBytes,
) -> *mut MeshResult {
    let key = match unsafe { public_secret_map_key(key) } {
        Ok(key) => key,
        Err(error) => return secret_map_error_result(error),
    };
    match with_current_secret_process(|process| {
        let map = validate_handle_pointer(process, map)
            .ok_or(SecretMapError::Resource(ResourceError::StaleHandle))?;
        secret_table()
            .lock()
            .secret_map_delete(process.pid, map, &key)
            .map(|_| ())
    }) {
        Ok(()) => alloc_result(0, std::ptr::null_mut()),
        Err(error) => secret_map_error_result(error),
    }
}

/// Atomically merge a speculative map into a committed map and consume it.
#[no_mangle]
pub extern "C" fn mesh_secret_map_merge(
    target: *mut MeshSecretHandle,
    source: *mut MeshSecretHandle,
) -> *mut MeshResult {
    match with_current_secret_process(|process| {
        let target_handle = validate_handle_pointer(process, target)
            .ok_or(SecretMapError::Resource(ResourceError::StaleHandle));
        let source_handle = validate_handle_pointer(process, source)
            .ok_or(SecretMapError::Resource(ResourceError::StaleHandle));
        let (target_handle, source_handle) = match (target_handle, source_handle) {
            (Ok(target_handle), Ok(source_handle)) => (target_handle, source_handle),
            (_, error) => {
                destroy_resource_for_process(process, source, Some(ResourceKind::SecretMap));
                return Err(error
                    .err()
                    .unwrap_or(SecretMapError::Resource(ResourceError::StaleHandle)));
            }
        };
        secret_table()
            .lock()
            .secret_map_merge(process.pid, target_handle, source_handle)
    }) {
        Ok(()) => alloc_result(0, std::ptr::null_mut()),
        Err(error) => secret_map_error_result(error),
    }
}

/// Explicitly destroy a secret owned by the current actor.
///
/// Null, stale, foreign, wrong-kind, and already-destroyed handles are no-ops.
#[no_mangle]
pub extern "C" fn mesh_secret_destroy(handle: *mut MeshSecretHandle) {
    destroy_resource_for_current_actor(handle, Some(ResourceKind::SecretBytes));
}

/// Destroy any registered actor-owned private resource.
///
/// This is the compiler drop target for affine resource values. The handle's
/// kind is decoded and then checked against the table entry; invalid, stale,
/// foreign, and already-destroyed handles are idempotent no-ops.
#[no_mangle]
pub extern "C" fn mesh_resource_destroy(handle: *mut MeshSecretHandle) {
    destroy_resource_for_current_actor(handle, None);
}

fn destroy_resource_for_current_actor(
    handle: *mut MeshSecretHandle,
    expected_kind: Option<ResourceKind>,
) {
    let Some(owner) = crate::actor::stack::get_current_pid() else {
        return;
    };
    let Some(scheduler) = crate::actor::GLOBAL_SCHEDULER.get() else {
        return;
    };
    let Some(process) = scheduler.get_process(owner) else {
        return;
    };
    let process = process.lock();
    destroy_resource_for_process(&process, handle, expected_kind);
}

fn destroy_resource_for_process(
    process: &Process,
    pointer: *mut MeshSecretHandle,
    expected_kind: Option<ResourceKind>,
) {
    let handle = validate_handle_pointer(process, pointer);
    let Some(handle) = handle else {
        return;
    };
    let result = match expected_kind {
        Some(kind) => secret_table()
            .lock()
            .destroy_kind(process.pid, handle, kind),
        None => secret_table().lock().destroy(process.pid, handle),
    };
    drop(result);
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    SecretBytes = 1,
    X25519PrivateKey = 2,
    SigningPrivateKey = 3,
    AeadKey = 4,
    StorageKey = 5,
    SecretMap = 6,
}

impl ResourceKind {
    fn from_raw(value: u32) -> Option<Self> {
        match value {
            value if value == Self::SecretBytes as u32 => Some(Self::SecretBytes),
            value if value == Self::X25519PrivateKey as u32 => Some(Self::X25519PrivateKey),
            value if value == Self::SigningPrivateKey as u32 => Some(Self::SigningPrivateKey),
            value if value == Self::AeadKey as u32 => Some(Self::AeadKey),
            value if value == Self::StorageKey as u32 => Some(Self::StorageKey),
            value if value == Self::SecretMap as u32 => Some(Self::SecretMap),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResourceHandle {
    slot: u32,
    generation: u32,
    kind: u32,
}

/// Opaque pointer payload used by the Mesh ABI. Secret material never lives here.
#[repr(C)]
pub struct MeshSecretHandle {
    slot: u32,
    generation: u32,
    kind: u32,
}

impl From<ResourceHandle> for MeshSecretHandle {
    fn from(handle: ResourceHandle) -> Self {
        Self {
            slot: handle.slot,
            generation: handle.generation,
            kind: handle.kind,
        }
    }
}

fn allocate_handle(process: &mut Process, handle: ResourceHandle) -> *mut MeshSecretHandle {
    let pointer = process
        .heap
        .alloc_exact(mem::size_of::<MeshSecretHandle>(), HANDLE_ALIGNMENT)
        as *mut MeshSecretHandle;
    unsafe { pointer.write(MeshSecretHandle::from(handle)) };
    pointer
}

/// Resolve only an exact live allocation in the supplied actor heap before
/// dereferencing the untrusted ABI pointer.
fn validate_handle_pointer(
    process: &Process,
    pointer: *const MeshSecretHandle,
) -> Option<ResourceHandle> {
    if pointer.is_null() {
        return None;
    }

    let mut current = process.heap.all_objects_head();
    while !current.is_null() {
        let header = unsafe { &*current };
        let next = header.next;
        let data_pointer = unsafe { (current as *const u8).add(GC_HEADER_SIZE) };
        let is_exact_handle = !header.is_free()
            && header.size as usize == mem::size_of::<MeshSecretHandle>()
            && data_pointer == pointer.cast::<u8>();
        if is_exact_handle {
            let handle = unsafe { &*pointer };
            return Some(ResourceHandle {
                slot: handle.slot,
                generation: handle.generation,
                kind: handle.kind,
            });
        }
        current = next;
    }
    None
}

#[derive(Clone, Copy)]
struct Limits {
    max_slots: usize,
    max_secrets_per_actor: usize,
    max_bytes_per_actor: usize,
    max_secret_bytes: usize,
    max_total_bytes: usize,
}

impl Limits {
    const fn production() -> Self {
        Self {
            max_slots: MAX_SECRET_SLOTS,
            max_secrets_per_actor: MAX_SECRETS_PER_ACTOR,
            max_bytes_per_actor: MAX_SECRET_BYTES_PER_ACTOR,
            max_secret_bytes: MAX_SECRET_BYTES,
            max_total_bytes: MAX_TOTAL_SECRET_BYTES,
        }
    }

    #[cfg(test)]
    const fn for_tests(
        max_slots: usize,
        max_secrets_per_actor: usize,
        max_bytes_per_actor: usize,
        max_secret_bytes: usize,
        max_total_bytes: usize,
    ) -> Self {
        Self {
            max_slots,
            max_secrets_per_actor,
            max_bytes_per_actor,
            max_secret_bytes,
            max_total_bytes,
        }
    }
}

pub(crate) type MeshStorageCounterReserve =
    unsafe extern "C" fn(context: *mut c_void, counter_out: *mut u64) -> i32;

#[derive(Clone, Copy)]
enum StorageCounterSource {
    Production {
        reserve: MeshStorageCounterReserve,
        context: usize,
        last_counter: Option<u64>,
    },
    #[cfg(test)]
    Deterministic { next_counter: u64 },
}

enum ResourceMetadata {
    Plain,
    StorageKey(StorageCounterSource),
}

struct Entry {
    owner: ProcessId,
    kind: ResourceKind,
    bytes: Zeroizing<Box<[u8]>>,
    metadata: ResourceMetadata,
}

struct Slot {
    generation: u32,
    entry: Option<Entry>,
}

#[derive(Default)]
struct Usage {
    secrets: usize,
    bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResourceError {
    ResourceLimitExceeded,
    StaleHandle,
    WrongOwner,
    WrongKind,
    OwnerExited,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StorageKeyError {
    Resource(ResourceError),
    ReservationFailed,
    CounterNotMonotonic,
    CounterExhausted,
}

pub(crate) struct PreparedStorageKey {
    pub(crate) handle: ResourceHandle,
    pub(crate) material: Zeroizing<Box<[u8]>>,
    counter: Option<PreparedStorageCounter>,
}

/// A zeroizing byte copy paired with the exact generational identity observed
/// when it was made. Revalidation uses this captured identity, never a handle
/// pointer that could have been recycled while a host/provider call ran.
pub(crate) struct PreparedOwnedResource {
    handle: ResourceHandle,
    pub(crate) bytes: Zeroizing<Box<[u8]>>,
}

impl PreparedStorageKey {
    pub(crate) fn reserve_counter(&mut self) -> Result<u64, StorageKeyError> {
        self.counter
            .take()
            .ok_or(StorageKeyError::ReservationFailed)?
            .reserve()
    }
}

#[derive(Clone, Copy)]
enum PreparedStorageCounter {
    Production {
        reserve: MeshStorageCounterReserve,
        context: usize,
    },
    #[cfg(test)]
    Reserved(u64),
}

impl PreparedStorageCounter {
    fn reserve(self) -> Result<u64, StorageKeyError> {
        match self {
            Self::Production { reserve, context } => {
                let mut counter = 0;
                let status = unsafe { reserve(context as *mut c_void, &mut counter) };
                if status == 0 {
                    Ok(counter)
                } else {
                    Err(StorageKeyError::ReservationFailed)
                }
            }
            #[cfg(test)]
            Self::Reserved(counter) => Ok(counter),
        }
    }
}

#[derive(Debug)]
pub(crate) enum RetypeError<E> {
    Resource(ResourceError),
    Rejected {
        error: E,
        removed: Zeroizing<Box<[u8]>>,
    },
    GenerationExhausted {
        removed: Zeroizing<Box<[u8]>>,
    },
}

#[derive(Debug)]
enum CreateSecretError {
    EntropyUnavailable,
    ResourceLimitExceeded,
    OwnerExited,
}

#[derive(Debug)]
enum ConcatSecretError {
    InvalidLength { maximum: usize, actual: usize },
    Resource(ResourceError),
}

#[derive(Debug, PartialEq, Eq)]
enum SecretMapError {
    Resource(ResourceError),
    InvalidCapacity,
    InvalidKey,
    InvalidEncoding,
    CapacityExceeded,
    DuplicateKey,
}

impl From<ResourceError> for SecretMapError {
    fn from(error: ResourceError) -> Self {
        Self::Resource(error)
    }
}

struct SecretMapData {
    capacity: u16,
    entries: Vec<(Vec<u8>, Zeroizing<Box<[u8]>>)>,
}

impl SecretMapData {
    fn empty(capacity: usize) -> Result<Self, SecretMapError> {
        if !(1..=MAX_SECRET_MAP_CAPACITY).contains(&capacity) {
            return Err(SecretMapError::InvalidCapacity);
        }
        Ok(Self {
            capacity: capacity as u16,
            entries: Vec::new(),
        })
    }

    fn decode(bytes: &[u8]) -> Result<Self, SecretMapError> {
        if bytes.len() < 4 {
            return Err(SecretMapError::InvalidEncoding);
        }
        let capacity = u16::from_be_bytes([bytes[0], bytes[1]]);
        let count = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        if capacity == 0 || capacity as usize > MAX_SECRET_MAP_CAPACITY || count > capacity as usize
        {
            return Err(SecretMapError::InvalidEncoding);
        }

        let mut offset = 4usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let header_end = offset
                .checked_add(6)
                .ok_or(SecretMapError::InvalidEncoding)?;
            let header = bytes
                .get(offset..header_end)
                .ok_or(SecretMapError::InvalidEncoding)?;
            let key_length = u16::from_be_bytes([header[0], header[1]]) as usize;
            let value_length =
                u32::from_be_bytes([header[2], header[3], header[4], header[5]]) as usize;
            if !(1..=MAX_SECRET_MAP_KEY_BYTES).contains(&key_length)
                || value_length == 0
                || value_length > MAX_SECRET_BYTES
            {
                return Err(SecretMapError::InvalidEncoding);
            }
            let key_start = header_end;
            let key_end = key_start
                .checked_add(key_length)
                .ok_or(SecretMapError::InvalidEncoding)?;
            let value_end = key_end
                .checked_add(value_length)
                .ok_or(SecretMapError::InvalidEncoding)?;
            let key = bytes
                .get(key_start..key_end)
                .ok_or(SecretMapError::InvalidEncoding)?
                .to_vec();
            let value = bytes
                .get(key_end..value_end)
                .ok_or(SecretMapError::InvalidEncoding)?;
            if entries.iter().any(|(existing, _)| existing == &key) {
                return Err(SecretMapError::InvalidEncoding);
            }
            entries.push((key, Zeroizing::new(value.to_vec().into_boxed_slice())));
            offset = value_end;
        }
        if offset != bytes.len() {
            return Err(SecretMapError::InvalidEncoding);
        }
        Ok(Self { capacity, entries })
    }

    fn encode(&self) -> Result<Zeroizing<Box<[u8]>>, SecretMapError> {
        let mut encoded = Vec::with_capacity(4);
        encoded.extend_from_slice(&self.capacity.to_be_bytes());
        encoded.extend_from_slice(&(self.entries.len() as u16).to_be_bytes());
        for (key, value) in &self.entries {
            let key_length = u16::try_from(key.len()).map_err(|_| SecretMapError::InvalidKey)?;
            let value_length =
                u32::try_from(value.len()).map_err(|_| SecretMapError::InvalidEncoding)?;
            encoded.extend_from_slice(&key_length.to_be_bytes());
            encoded.extend_from_slice(&value_length.to_be_bytes());
            encoded.extend_from_slice(key);
            encoded.extend_from_slice(value);
        }
        if encoded.len() > MAX_SECRET_BYTES {
            encoded.zeroize();
            return Err(SecretMapError::Resource(
                ResourceError::ResourceLimitExceeded,
            ));
        }
        Ok(Zeroizing::new(encoded.into_boxed_slice()))
    }
}

fn valid_secret_map_key(key: &[u8]) -> Result<(), SecretMapError> {
    if (1..=MAX_SECRET_MAP_KEY_BYTES).contains(&key.len()) {
        Ok(())
    } else {
        Err(SecretMapError::InvalidKey)
    }
}

struct ResourceTable {
    slots: Vec<Slot>,
    free_slots: Vec<u32>,
    usage: HashMap<ProcessId, Usage>,
    total_bytes: usize,
    limits: Limits,
}

static SECRET_TABLE: OnceLock<Mutex<ResourceTable>> = OnceLock::new();

fn secret_table() -> &'static Mutex<ResourceTable> {
    SECRET_TABLE.get_or_init(|| Mutex::new(ResourceTable::new(Limits::production())))
}

fn live_owner(process: &Process) -> Result<ProcessId, ResourceError> {
    if matches!(process.state, ProcessState::Exited(_)) {
        Err(ResourceError::OwnerExited)
    } else {
        Ok(process.pid)
    }
}

/// Store a private resource for a live actor and return an opaque ABI handle.
/// The table guard is released before the actor heap allocation.
pub(crate) fn insert_owned_resource(
    process: &mut Process,
    kind: ResourceKind,
    bytes: Zeroizing<Box<[u8]>>,
) -> Result<*mut MeshSecretHandle, ResourceError> {
    let owner = live_owner(process)?;
    let handle = { secret_table().lock().insert(owner, kind, bytes) }?;
    Ok(allocate_handle(process, handle))
}

/// Register a platform-provisioned storage-wrapping capability. The 36-byte
/// material is the wrapping key followed by its per-key nonce prefix.
pub(crate) fn insert_storage_key_resource(
    process: &mut Process,
    material: Zeroizing<Box<[u8]>>,
    reserve: MeshStorageCounterReserve,
    context: *mut c_void,
) -> Result<*mut MeshSecretHandle, ResourceError> {
    let owner = live_owner(process)?;
    let handle = secret_table().lock().insert_storage_key(
        owner,
        material,
        StorageCounterSource::Production {
            reserve,
            context: context as usize,
            last_counter: None,
        },
    )?;
    Ok(allocate_handle(process, handle))
}

#[cfg(test)]
pub(crate) fn insert_test_storage_key_resource(
    process: &mut Process,
    material: Zeroizing<Box<[u8]>>,
    next_counter: u64,
) -> Result<*mut MeshSecretHandle, ResourceError> {
    let owner = live_owner(process)?;
    let handle = secret_table().lock().insert_storage_key(
        owner,
        material,
        StorageCounterSource::Deterministic { next_counter },
    )?;
    Ok(allocate_handle(process, handle))
}

/// Copy a storage key and its reservation capability while holding the table
/// lock. Calling the platform callback is intentionally a separate step.
pub(crate) fn prepare_storage_key_resource(
    process: &Process,
    pointer: *const MeshSecretHandle,
) -> Result<PreparedStorageKey, StorageKeyError> {
    let owner = live_owner(process).map_err(StorageKeyError::Resource)?;
    let handle = validate_handle_pointer(process, pointer)
        .ok_or(StorageKeyError::Resource(ResourceError::StaleHandle))?;
    secret_table().lock().prepare_storage_key(owner, handle)
}

/// Revalidate ownership/generation after an unlocked platform reservation and
/// record the counter as consumed before any encryption or output allocation.
pub(crate) fn commit_storage_counter(
    process: &Process,
    prepared: &PreparedStorageKey,
    counter: u64,
) -> Result<(), StorageKeyError> {
    let owner = live_owner(process).map_err(StorageKeyError::Resource)?;
    secret_table()
        .lock()
        .commit_storage_counter(owner, prepared.handle, counter)
}

/// Borrow private bytes only for the duration of a callback. A temporary
/// zeroizing copy lets the table guard go before user/provider code runs, so
/// callbacks never execute while the global resource table is locked.
pub(crate) fn with_owned_resource<R>(
    process: &Process,
    pointer: *const MeshSecretHandle,
    expected_kind: ResourceKind,
    read: impl for<'a> FnOnce(&'a [u8]) -> R,
) -> Result<R, ResourceError> {
    let prepared = prepare_owned_resource(process, pointer, expected_kind)?;
    Ok(read(&prepared.bytes))
}

pub(crate) fn prepare_owned_resource(
    process: &Process,
    pointer: *const MeshSecretHandle,
    expected_kind: ResourceKind,
) -> Result<PreparedOwnedResource, ResourceError> {
    let owner = live_owner(process)?;
    let handle = validate_handle_pointer(process, pointer).ok_or(ResourceError::StaleHandle)?;
    let bytes = secret_table()
        .lock()
        .with_resource(owner, handle, expected_kind, |bytes| {
            Zeroizing::new(bytes.to_vec().into_boxed_slice())
        })?;
    Ok(PreparedOwnedResource { handle, bytes })
}

/// Revalidate a resource without copying its bytes. Used after unlocked host
/// callbacks and provider calls to close actor-exit/destroy races.
pub(crate) fn validate_prepared_owned_resource(
    process: &Process,
    prepared: &PreparedOwnedResource,
    expected_kind: ResourceKind,
) -> Result<(), ResourceError> {
    let owner = live_owner(process)?;
    secret_table()
        .lock()
        .validate(owner, prepared.handle, expected_kind)
        .map(|_| ())
}

pub(crate) fn validate_prepared_storage_key_resource(
    process: &Process,
    prepared: &PreparedStorageKey,
) -> Result<(), StorageKeyError> {
    let owner = live_owner(process).map_err(StorageKeyError::Resource)?;
    secret_table()
        .lock()
        .validate(owner, prepared.handle, ResourceKind::StorageKey)
        .map(|_| ())
        .map_err(StorageKeyError::Resource)
}

/// Remove a private resource from its owner table and invalidate its handle.
/// The returned allocation remains zeroizing so callers cannot accidentally
/// free live key material without wiping it.
#[cfg(test)]
pub(crate) fn consume_owned_resource(
    process: &Process,
    pointer: *const MeshSecretHandle,
    expected_kind: ResourceKind,
) -> Result<Zeroizing<Box<[u8]>>, ResourceError> {
    if expected_kind == ResourceKind::StorageKey {
        return Err(ResourceError::WrongKind);
    }
    let owner = live_owner(process)?;
    let handle = validate_handle_pointer(process, pointer).ok_or(ResourceError::StaleHandle)?;
    secret_table().lock().consume(owner, handle, expected_kind)
}

/// Atomically consume one resource kind and, after validation, re-register the
/// same zeroizing allocation under a fresh generation and target kind.
/// The table guard is released before allocating the new ABI handle.
pub(crate) fn consume_and_retype_owned_resource<E>(
    process: &mut Process,
    pointer: *const MeshSecretHandle,
    expected_kind: ResourceKind,
    target_kind: ResourceKind,
    validate_target: impl for<'a> FnOnce(&'a [u8]) -> Result<(), E>,
) -> Result<*mut MeshSecretHandle, RetypeError<E>> {
    if expected_kind == ResourceKind::StorageKey || target_kind == ResourceKind::StorageKey {
        return Err(RetypeError::Resource(ResourceError::WrongKind));
    }
    let owner = live_owner(process).map_err(RetypeError::Resource)?;
    let handle = validate_handle_pointer(process, pointer)
        .ok_or(RetypeError::Resource(ResourceError::StaleHandle))?;
    let retyped = {
        secret_table().lock().consume_and_retype(
            owner,
            handle,
            expected_kind,
            target_kind,
            validate_target,
        )
    }?;
    Ok(allocate_handle(process, retyped))
}

/// Zeroize every secret owned by an actor. Scheduler exit paths call this in M2 integration.
pub(crate) fn destroy_owned(owner: ProcessId) -> usize {
    secret_table().lock().destroy_owned(owner)
}

#[cfg(test)]
pub(crate) fn insert_test_secret(owner: ProcessId) {
    secret_table()
        .lock()
        .insert_secret(owner, vec![0xA5; 8].into_boxed_slice())
        .expect("insert lifecycle test secret");
}

#[cfg(test)]
pub(crate) fn owned_secret_count_for_test(owner: ProcessId) -> usize {
    secret_table()
        .lock()
        .usage
        .get(&owner)
        .map_or(0, |usage| usage.secrets)
}

impl ResourceTable {
    fn new(limits: Limits) -> Self {
        Self {
            slots: Vec::new(),
            free_slots: Vec::new(),
            usage: HashMap::new(),
            total_bytes: 0,
            limits,
        }
    }

    #[cfg(test)]
    fn insert_secret(
        &mut self,
        owner: ProcessId,
        bytes: Box<[u8]>,
    ) -> Result<ResourceHandle, ResourceError> {
        self.insert(owner, ResourceKind::SecretBytes, Zeroizing::new(bytes))
    }

    fn insert_secret_map(
        &mut self,
        owner: ProcessId,
        capacity: usize,
    ) -> Result<ResourceHandle, SecretMapError> {
        let encoded = SecretMapData::empty(capacity)?.encode()?;
        self.insert(owner, ResourceKind::SecretMap, encoded)
            .map_err(SecretMapError::Resource)
    }

    fn insert(
        &mut self,
        owner: ProcessId,
        kind: ResourceKind,
        bytes: Zeroizing<Box<[u8]>>,
    ) -> Result<ResourceHandle, ResourceError> {
        if kind == ResourceKind::StorageKey {
            return Err(ResourceError::WrongKind);
        }
        self.insert_with_metadata(owner, kind, bytes, ResourceMetadata::Plain)
    }

    fn insert_storage_key(
        &mut self,
        owner: ProcessId,
        bytes: Zeroizing<Box<[u8]>>,
        source: StorageCounterSource,
    ) -> Result<ResourceHandle, ResourceError> {
        if bytes.len() != 36 {
            return Err(ResourceError::WrongKind);
        }
        self.insert_with_metadata(
            owner,
            ResourceKind::StorageKey,
            bytes,
            ResourceMetadata::StorageKey(source),
        )
    }

    fn insert_with_metadata(
        &mut self,
        owner: ProcessId,
        kind: ResourceKind,
        bytes: Zeroizing<Box<[u8]>>,
        metadata: ResourceMetadata,
    ) -> Result<ResourceHandle, ResourceError> {
        let byte_count = bytes.len();
        if byte_count > self.limits.max_secret_bytes {
            return Err(ResourceError::ResourceLimitExceeded);
        }
        let actor_usage = self.usage.get(&owner);
        let actor_secret_count = actor_usage.map_or(0, |usage| usage.secrets);
        let actor_byte_count = actor_usage.map_or(0, |usage| usage.bytes);
        let next_actor_secret_count = actor_secret_count
            .checked_add(1)
            .ok_or(ResourceError::ResourceLimitExceeded)?;
        let next_actor_byte_count = actor_byte_count
            .checked_add(byte_count)
            .ok_or(ResourceError::ResourceLimitExceeded)?;
        let next_total_bytes = self
            .total_bytes
            .checked_add(byte_count)
            .ok_or(ResourceError::ResourceLimitExceeded)?;
        if next_actor_secret_count > self.limits.max_secrets_per_actor
            || next_actor_byte_count > self.limits.max_bytes_per_actor
            || next_total_bytes > self.limits.max_total_bytes
        {
            return Err(ResourceError::ResourceLimitExceeded);
        }

        let slot_index = if let Some(slot) = self.free_slots.pop() {
            slot
        } else {
            if self.slots.len() >= self.limits.max_slots || self.slots.len() >= u32::MAX as usize {
                return Err(ResourceError::ResourceLimitExceeded);
            }
            let slot = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 1,
                entry: None,
            });
            slot
        };
        let slot = &mut self.slots[slot_index as usize];
        slot.entry = Some(Entry {
            owner,
            kind,
            bytes,
            metadata,
        });
        self.usage.insert(
            owner,
            Usage {
                secrets: next_actor_secret_count,
                bytes: next_actor_byte_count,
            },
        );
        self.total_bytes = next_total_bytes;
        Ok(ResourceHandle {
            slot: slot_index,
            generation: slot.generation,
            kind: kind as u32,
        })
    }

    fn replace_resource_bytes(
        &mut self,
        owner: ProcessId,
        handle: ResourceHandle,
        expected_kind: ResourceKind,
        bytes: Zeroizing<Box<[u8]>>,
    ) -> Result<(), ResourceError> {
        let old_length = self
            .validate_entry(owner, handle, expected_kind)?
            .bytes
            .len();
        let new_length = bytes.len();
        if new_length > self.limits.max_secret_bytes {
            return Err(ResourceError::ResourceLimitExceeded);
        }
        let usage = self.usage.get(&owner).ok_or(ResourceError::StaleHandle)?;
        let next_actor_bytes = usage
            .bytes
            .checked_sub(old_length)
            .and_then(|value| value.checked_add(new_length))
            .ok_or(ResourceError::ResourceLimitExceeded)?;
        let next_total_bytes = self
            .total_bytes
            .checked_sub(old_length)
            .and_then(|value| value.checked_add(new_length))
            .ok_or(ResourceError::ResourceLimitExceeded)?;
        if next_actor_bytes > self.limits.max_bytes_per_actor
            || next_total_bytes > self.limits.max_total_bytes
        {
            return Err(ResourceError::ResourceLimitExceeded);
        }

        let entry = self
            .slots
            .get_mut(handle.slot as usize)
            .and_then(|slot| slot.entry.as_mut())
            .ok_or(ResourceError::StaleHandle)?;
        let old = mem::replace(&mut entry.bytes, bytes);
        self.usage
            .get_mut(&owner)
            .expect("validated owner usage")
            .bytes = next_actor_bytes;
        self.total_bytes = next_total_bytes;
        drop(old);
        Ok(())
    }

    fn secret_map_contains(
        &self,
        owner: ProcessId,
        map: ResourceHandle,
        key: &[u8],
    ) -> Result<bool, SecretMapError> {
        valid_secret_map_key(key)?;
        let data = SecretMapData::decode(self.validate(owner, map, ResourceKind::SecretMap)?)?;
        Ok(data.entries.iter().any(|(candidate, _)| candidate == key))
    }

    fn secret_map_copy(
        &mut self,
        owner: ProcessId,
        map: ResourceHandle,
        key: &[u8],
    ) -> Result<ResourceHandle, SecretMapError> {
        valid_secret_map_key(key)?;
        let data = SecretMapData::decode(self.validate(owner, map, ResourceKind::SecretMap)?)?;
        let value = data
            .entries
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| Zeroizing::new(value.to_vec().into_boxed_slice()))
            .ok_or(SecretMapError::InvalidKey)?;
        self.insert(owner, ResourceKind::SecretBytes, value)
            .map_err(SecretMapError::Resource)
    }

    fn secret_map_insert(
        &mut self,
        owner: ProcessId,
        map: ResourceHandle,
        key: &[u8],
        value: ResourceHandle,
    ) -> Result<(), SecretMapError> {
        let result = self.secret_map_insert_inner(owner, map, key, value);
        if result.is_err() {
            drop(self.destroy_kind(owner, value, ResourceKind::SecretBytes));
        }
        result
    }

    fn secret_map_insert_inner(
        &mut self,
        owner: ProcessId,
        map: ResourceHandle,
        key: &[u8],
        value: ResourceHandle,
    ) -> Result<(), SecretMapError> {
        valid_secret_map_key(key)?;
        self.validate(owner, value, ResourceKind::SecretBytes)?;
        let mut data =
            SecretMapData::decode(self.validate(owner, map, ResourceKind::SecretMap)?)?;
        if data.entries.iter().any(|(candidate, _)| candidate == key) {
            return Err(SecretMapError::DuplicateKey);
        }
        if data.entries.len() >= data.capacity as usize {
            return Err(SecretMapError::CapacityExceeded);
        }
        let value = self.consume(owner, value, ResourceKind::SecretBytes)?;
        data.entries.push((key.to_vec(), value));
        let encoded = data.encode()?;
        self.replace_resource_bytes(owner, map, ResourceKind::SecretMap, encoded)?;
        Ok(())
    }

    fn secret_map_delete(
        &mut self,
        owner: ProcessId,
        map: ResourceHandle,
        key: &[u8],
    ) -> Result<bool, SecretMapError> {
        valid_secret_map_key(key)?;
        let mut data =
            SecretMapData::decode(self.validate(owner, map, ResourceKind::SecretMap)?)?;
        let Some(index) = data
            .entries
            .iter()
            .position(|(candidate, _)| candidate == key)
        else {
            return Ok(false);
        };
        data.entries.remove(index);
        let encoded = data.encode()?;
        self.replace_resource_bytes(owner, map, ResourceKind::SecretMap, encoded)?;
        Ok(true)
    }

    fn secret_map_merge(
        &mut self,
        owner: ProcessId,
        target: ResourceHandle,
        source: ResourceHandle,
    ) -> Result<(), SecretMapError> {
        if target == source {
            return Err(SecretMapError::DuplicateKey);
        }
        let result = self.secret_map_merge_inner(owner, target, source);
        if result.is_err() {
            drop(self.destroy_kind(owner, source, ResourceKind::SecretMap));
        }
        result
    }

    fn secret_map_merge_inner(
        &mut self,
        owner: ProcessId,
        target: ResourceHandle,
        source: ResourceHandle,
    ) -> Result<(), SecretMapError> {
        let mut target_data =
            SecretMapData::decode(self.validate(owner, target, ResourceKind::SecretMap)?)?;
        let source_data =
            SecretMapData::decode(self.validate(owner, source, ResourceKind::SecretMap)?)?;
        if source_data.entries.iter().any(|(source_key, _)| {
            target_data
                .entries
                .iter()
                .any(|(target_key, _)| target_key == source_key)
        }) {
            return Err(SecretMapError::DuplicateKey);
        }
        target_data.entries.extend(source_data.entries);
        let excess = target_data
            .entries
            .len()
            .saturating_sub(target_data.capacity as usize);
        target_data.entries.drain(..excess);
        let encoded = target_data.encode()?;
        let consumed = self.consume(owner, source, ResourceKind::SecretMap)?;
        self.replace_resource_bytes(owner, target, ResourceKind::SecretMap, encoded)?;
        drop(consumed);
        Ok(())
    }

    fn with_resource<R>(
        &self,
        owner: ProcessId,
        handle: ResourceHandle,
        expected_kind: ResourceKind,
        read: impl for<'a> FnOnce(&'a [u8]) -> R,
    ) -> Result<R, ResourceError> {
        self.validate(owner, handle, expected_kind).map(read)
    }

    fn validate(
        &self,
        owner: ProcessId,
        handle: ResourceHandle,
        expected_kind: ResourceKind,
    ) -> Result<&[u8], ResourceError> {
        if handle.kind != expected_kind as u32 {
            return Err(ResourceError::WrongKind);
        }
        Ok(&self.validate_entry(owner, handle, expected_kind)?.bytes)
    }

    fn validate_entry(
        &self,
        owner: ProcessId,
        handle: ResourceHandle,
        expected_kind: ResourceKind,
    ) -> Result<&Entry, ResourceError> {
        if handle.kind != expected_kind as u32 {
            return Err(ResourceError::WrongKind);
        }
        let slot = self
            .slots
            .get(handle.slot as usize)
            .ok_or(ResourceError::StaleHandle)?;
        if slot.generation != handle.generation {
            return Err(ResourceError::StaleHandle);
        }
        let entry = slot.entry.as_ref().ok_or(ResourceError::StaleHandle)?;
        if entry.owner != owner {
            return Err(ResourceError::WrongOwner);
        }
        if entry.kind != expected_kind {
            return Err(ResourceError::WrongKind);
        }
        Ok(entry)
    }

    fn prepare_storage_key(
        &mut self,
        owner: ProcessId,
        handle: ResourceHandle,
    ) -> Result<PreparedStorageKey, StorageKeyError> {
        let entry = self
            .validate_entry(owner, handle, ResourceKind::StorageKey)
            .map_err(StorageKeyError::Resource)?;
        let material = Zeroizing::new(entry.bytes.to_vec().into_boxed_slice());
        let source = match entry.metadata {
            ResourceMetadata::StorageKey(source) => source,
            ResourceMetadata::Plain => {
                return Err(StorageKeyError::Resource(ResourceError::WrongKind));
            }
        };

        let counter = match source {
            StorageCounterSource::Production {
                reserve, context, ..
            } => PreparedStorageCounter::Production { reserve, context },
            #[cfg(test)]
            StorageCounterSource::Deterministic { next_counter } => {
                let incremented = next_counter
                    .checked_add(1)
                    .ok_or(StorageKeyError::CounterExhausted)?;
                let entry = self
                    .slots
                    .get_mut(handle.slot as usize)
                    .and_then(|slot| slot.entry.as_mut())
                    .ok_or(StorageKeyError::Resource(ResourceError::StaleHandle))?;
                entry.metadata =
                    ResourceMetadata::StorageKey(StorageCounterSource::Deterministic {
                        next_counter: incremented,
                    });
                PreparedStorageCounter::Reserved(next_counter)
            }
        };

        Ok(PreparedStorageKey {
            handle,
            material,
            counter: Some(counter),
        })
    }

    fn commit_storage_counter(
        &mut self,
        owner: ProcessId,
        handle: ResourceHandle,
        counter: u64,
    ) -> Result<(), StorageKeyError> {
        self.validate_entry(owner, handle, ResourceKind::StorageKey)
            .map_err(StorageKeyError::Resource)?;
        let entry = self
            .slots
            .get_mut(handle.slot as usize)
            .and_then(|slot| slot.entry.as_mut())
            .ok_or(StorageKeyError::Resource(ResourceError::StaleHandle))?;
        match &mut entry.metadata {
            ResourceMetadata::StorageKey(StorageCounterSource::Production {
                last_counter, ..
            }) => {
                if last_counter.is_some_and(|last| counter <= last) {
                    return Err(StorageKeyError::CounterNotMonotonic);
                }
                *last_counter = Some(counter);
                if counter == u64::MAX {
                    Err(StorageKeyError::CounterExhausted)
                } else {
                    Ok(())
                }
            }
            #[cfg(test)]
            ResourceMetadata::StorageKey(StorageCounterSource::Deterministic { next_counter }) => {
                if counter.checked_add(1) != Some(*next_counter) {
                    Err(StorageKeyError::CounterNotMonotonic)
                } else {
                    Ok(())
                }
            }
            ResourceMetadata::Plain => Err(StorageKeyError::Resource(ResourceError::WrongKind)),
        }
    }

    fn destroy(
        &mut self,
        owner: ProcessId,
        handle: ResourceHandle,
    ) -> Result<Option<Zeroizing<Box<[u8]>>>, ResourceError> {
        let kind = ResourceKind::from_raw(handle.kind).ok_or(ResourceError::WrongKind)?;
        self.destroy_kind(owner, handle, kind)
    }

    fn destroy_kind(
        &mut self,
        owner: ProcessId,
        handle: ResourceHandle,
        expected_kind: ResourceKind,
    ) -> Result<Option<Zeroizing<Box<[u8]>>>, ResourceError> {
        match self.consume(owner, handle, expected_kind) {
            Ok(mut bytes) => {
                bytes.zeroize();
                Ok(Some(bytes))
            }
            Err(ResourceError::StaleHandle) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn consume(
        &mut self,
        owner: ProcessId,
        handle: ResourceHandle,
        expected_kind: ResourceKind,
    ) -> Result<Zeroizing<Box<[u8]>>, ResourceError> {
        self.validate(owner, handle, expected_kind)?;
        let slot = &mut self.slots[handle.slot as usize];
        let entry = slot.entry.take().ok_or(ResourceError::StaleHandle)?;
        if slot.generation != u32::MAX {
            slot.generation += 1;
            self.free_slots.push(handle.slot);
        }
        self.release_usage(owner, entry.bytes.len());
        Ok(entry.bytes)
    }

    fn concat_secrets(
        &mut self,
        owner: ProcessId,
        first: ResourceHandle,
        second: ResourceHandle,
    ) -> Result<ResourceHandle, ConcatSecretError> {
        let first_length = self
            .validate(owner, first, ResourceKind::SecretBytes)
            .map(|bytes| bytes.len());
        let second_length = self
            .validate(owner, second, ResourceKind::SecretBytes)
            .map(|bytes| bytes.len());
        let (first_length, second_length) = match (first_length, second_length) {
            (Ok(first_length), Ok(second_length)) if first != second => {
                (first_length, second_length)
            }
            (first_result, second_result) => {
                let error = first_result
                    .err()
                    .or_else(|| second_result.err())
                    .unwrap_or(ResourceError::StaleHandle);
                drop(self.destroy_kind(owner, first, ResourceKind::SecretBytes));
                drop(self.destroy_kind(owner, second, ResourceKind::SecretBytes));
                return Err(ConcatSecretError::Resource(error));
            }
        };
        let total_length = first_length
            .checked_add(second_length)
            .unwrap_or(usize::MAX);
        let first_bytes = self
            .consume(owner, first, ResourceKind::SecretBytes)
            .map_err(ConcatSecretError::Resource)?;
        let second_bytes = self
            .consume(owner, second, ResourceKind::SecretBytes)
            .map_err(ConcatSecretError::Resource)?;
        if total_length > self.limits.max_secret_bytes {
            return Err(ConcatSecretError::InvalidLength {
                maximum: self.limits.max_secret_bytes,
                actual: total_length,
            });
        }

        let mut combined = Zeroizing::new(vec![0; total_length].into_boxed_slice());
        combined[..first_length].copy_from_slice(&first_bytes);
        combined[first_length..].copy_from_slice(&second_bytes);
        self.insert(owner, ResourceKind::SecretBytes, combined)
            .map_err(ConcatSecretError::Resource)
    }

    fn consume_and_retype<E>(
        &mut self,
        owner: ProcessId,
        handle: ResourceHandle,
        expected_kind: ResourceKind,
        target_kind: ResourceKind,
        validate_target: impl for<'a> FnOnce(&'a [u8]) -> Result<(), E>,
    ) -> Result<ResourceHandle, RetypeError<E>> {
        if expected_kind == ResourceKind::StorageKey || target_kind == ResourceKind::StorageKey {
            return Err(RetypeError::Resource(ResourceError::WrongKind));
        }
        self.validate(owner, handle, expected_kind)
            .map_err(RetypeError::Resource)?;
        let (mut entry, next_generation) = {
            let slot = &mut self.slots[handle.slot as usize];
            let entry = slot
                .entry
                .take()
                .ok_or(RetypeError::Resource(ResourceError::StaleHandle))?;
            let next_generation = slot.generation.checked_add(1);
            if let Some(generation) = next_generation {
                slot.generation = generation;
            }
            (entry, next_generation)
        };

        if let Err(error) = validate_target(&entry.bytes) {
            entry.bytes.zeroize();
            if next_generation.is_some() {
                self.free_slots.push(handle.slot);
            }
            self.release_usage(owner, entry.bytes.len());
            return Err(RetypeError::Rejected {
                error,
                removed: entry.bytes,
            });
        }

        let Some(next_generation) = next_generation else {
            entry.bytes.zeroize();
            self.release_usage(owner, entry.bytes.len());
            return Err(RetypeError::GenerationExhausted {
                removed: entry.bytes,
            });
        };
        entry.kind = target_kind;
        self.slots[handle.slot as usize].entry = Some(entry);

        Ok(ResourceHandle {
            slot: handle.slot,
            generation: next_generation,
            kind: target_kind as u32,
        })
    }

    fn release_usage(&mut self, owner: ProcessId, byte_count: usize) {
        let remove_usage = {
            let usage = self
                .usage
                .get_mut(&owner)
                .expect("live resource must have owner usage");
            usage.secrets = usage
                .secrets
                .checked_sub(1)
                .expect("resource count must not underflow");
            usage.bytes = usage
                .bytes
                .checked_sub(byte_count)
                .expect("resource bytes must not underflow");
            if usage.secrets == 0 {
                assert_eq!(usage.bytes, 0, "zero resources must account for zero bytes");
                true
            } else {
                false
            }
        };
        if remove_usage {
            self.usage.remove(&owner);
        }
        self.total_bytes = self
            .total_bytes
            .checked_sub(byte_count)
            .expect("total resource bytes must not underflow");
    }

    fn destroy_owned(&mut self, owner: ProcessId) -> usize {
        let handles: Vec<_> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot, value)| {
                let entry = value.entry.as_ref()?;
                (entry.owner == owner).then_some(ResourceHandle {
                    slot: slot as u32,
                    generation: value.generation,
                    kind: entry.kind as u32,
                })
            })
            .collect();

        let mut destroyed = 0;
        for handle in handles {
            if matches!(self.destroy(owner, handle), Ok(Some(_))) {
                destroyed += 1;
            }
        }
        destroyed
    }
}

fn create_random_secret_entry(
    process: &Process,
    table: &mut ResourceTable,
    length: usize,
) -> Result<ResourceHandle, CreateSecretError> {
    if matches!(process.state, ProcessState::Exited(_)) {
        return Err(CreateSecretError::OwnerExited);
    }
    let mut bytes = Zeroizing::new(vec![0u8; length].into_boxed_slice());
    if SystemRandom::new().fill(&mut bytes).is_err() {
        bytes.zeroize();
        return Err(CreateSecretError::EntropyUnavailable);
    }
    table
        .insert(process.pid, ResourceKind::SecretBytes, bytes)
        .map_err(|_| CreateSecretError::ResourceLimitExceeded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Priority, Process};
    use std::sync::atomic::{AtomicU64, Ordering};

    static GLOBAL_TABLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn destroyed_slots_are_reused_with_a_new_generation() {
        let owner = ProcessId(41);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));

        let first = table
            .insert_secret(owner, vec![1; 8].into_boxed_slice())
            .expect("first insert");
        table.destroy(owner, first).expect("first destroy");
        let second = table
            .insert_secret(owner, vec![2; 8].into_boxed_slice())
            .expect("second insert");

        assert_eq!(second.slot, first.slot);
        assert_eq!(second.generation, first.generation + 1);
        assert!(matches!(
            table.validate(owner, first, ResourceKind::SecretBytes),
            Err(ResourceError::StaleHandle)
        ));
    }

    #[test]
    fn validates_owner_and_resource_kind_on_every_access() {
        let owner = ProcessId(51);
        let other_actor = ProcessId(52);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));
        let handle = table
            .insert_secret(owner, vec![7; 8].into_boxed_slice())
            .expect("insert secret");

        assert!(matches!(
            table.validate(other_actor, handle, ResourceKind::SecretBytes),
            Err(ResourceError::WrongOwner)
        ));

        let wrong_kind = ResourceHandle { kind: 99, ..handle };
        assert!(matches!(
            table.validate(owner, wrong_kind, ResourceKind::SecretBytes),
            Err(ResourceError::WrongKind)
        ));
    }

    #[test]
    fn stores_every_private_resource_kind_and_reads_only_through_a_closure() {
        let owner = ProcessId(53);
        let mut table = ResourceTable::new(Limits::for_tests(8, 8, 128, 64, 128));
        let cases = [
            (ResourceKind::SecretBytes, 0x11),
            (ResourceKind::X25519PrivateKey, 0x22),
            (ResourceKind::SigningPrivateKey, 0x33),
            (ResourceKind::AeadKey, 0x44),
        ];

        for (kind, marker) in cases {
            let handle = table
                .insert(
                    owner,
                    kind,
                    Zeroizing::new(vec![marker; 8].into_boxed_slice()),
                )
                .expect("insert private resource");
            let observed = table
                .with_resource(owner, handle, kind, |bytes| (bytes.len(), bytes[0]))
                .expect("read matching private resource");

            assert_eq!(observed, (8, marker));
            assert!(matches!(
                table.with_resource(ProcessId(54), handle, kind, |_| ()),
                Err(ResourceError::WrongOwner)
            ));
            assert!(
                matches!(
                    table.with_resource(owner, handle, ResourceKind::SecretBytes, |_| ()),
                    Err(ResourceError::WrongKind)
                ) || kind == ResourceKind::SecretBytes
            );
        }
    }

    #[test]
    fn secret_map_bounds_copies_and_deletes_zeroizing_values() {
        let owner = ProcessId(55);
        let mut table = ResourceTable::new(Limits::for_tests(16, 16, 4096, 1024, 4096));
        let map = table.insert_secret_map(owner, 2).expect("create map");
        let first = table
            .insert_secret(owner, vec![0x11; 32].into_boxed_slice())
            .expect("first key");
        table
            .secret_map_insert(owner, map, b"first", first)
            .expect("store first key");

        assert!(table
            .secret_map_contains(owner, map, b"first")
            .expect("contains first"));
        let copied = table
            .secret_map_copy(owner, map, b"first")
            .expect("copy first");
        assert_eq!(
            table
                .validate(owner, copied, ResourceKind::SecretBytes)
                .expect("copied key"),
            &[0x11; 32]
        );

        let second = table
            .insert_secret(owner, vec![0x22; 32].into_boxed_slice())
            .expect("second key");
        table
            .secret_map_insert(owner, map, b"second", second)
            .expect("store second key");
        let excess = table
            .insert_secret(owner, vec![0x33; 32].into_boxed_slice())
            .expect("excess key");
        assert_eq!(
            table.secret_map_insert(owner, map, b"third", excess),
            Err(SecretMapError::CapacityExceeded)
        );
        assert!(matches!(
            table.validate(owner, excess, ResourceKind::SecretBytes),
            Err(ResourceError::StaleHandle)
        ));

        assert!(table
            .secret_map_delete(owner, map, b"first")
            .expect("delete first"));
        assert!(!table
            .secret_map_contains(owner, map, b"first")
            .expect("first removed"));
        assert!(!table
            .secret_map_delete(owner, map, b"first")
            .expect("repeat delete"));

        let source = table
            .insert_secret_map(owner, 2)
            .expect("create source map");
        let moved = table
            .insert_secret(owner, vec![0x44; 32].into_boxed_slice())
            .expect("source key");
        table
            .secret_map_insert(owner, source, b"moved", moved)
            .expect("store source key");
        table
            .secret_map_merge(owner, map, source)
            .expect("merge source map");
        assert!(table
            .secret_map_contains(owner, map, b"moved")
            .expect("merged key"));
        assert!(matches!(
            table.validate(owner, source, ResourceKind::SecretMap),
            Err(ResourceError::StaleHandle)
        ));

        let newest = table
            .insert_secret_map(owner, 1)
            .expect("create newest map");
        let newest_key = table
            .insert_secret(owner, vec![0x55; 32].into_boxed_slice())
            .expect("newest key");
        table
            .secret_map_insert(owner, newest, b"newest", newest_key)
            .expect("store newest key");
        table
            .secret_map_merge(owner, map, newest)
            .expect("bounded merge evicts oldest key");
        assert!(!table
            .secret_map_contains(owner, map, b"second")
            .expect("oldest evicted"));
        assert!(table
            .secret_map_contains(owner, map, b"moved")
            .expect("newer retained"));
        assert!(table
            .secret_map_contains(owner, map, b"newest")
            .expect("newest retained"));
    }

    #[test]
    fn storage_keys_require_nonce_metadata_and_reserve_test_counters_monotonically() {
        let owner = ProcessId(55);
        let mut table = ResourceTable::new(Limits::for_tests(8, 8, 128, 64, 128));
        let material = Zeroizing::new(vec![0x41; 36].into_boxed_slice());

        assert!(matches!(
            table.insert(owner, ResourceKind::StorageKey, material.clone()),
            Err(ResourceError::WrongKind)
        ));

        let handle = table
            .insert_storage_key(
                owner,
                material,
                StorageCounterSource::Deterministic { next_counter: 7 },
            )
            .expect("insert test storage key");
        assert!(matches!(
            table.consume_and_retype::<()>(
                owner,
                handle,
                ResourceKind::StorageKey,
                ResourceKind::AeadKey,
                |_| Ok(()),
            ),
            Err(RetypeError::Resource(ResourceError::WrongKind))
        ));
        let mut first = table
            .prepare_storage_key(owner, handle)
            .expect("first counter reservation");
        assert_eq!(first.reserve_counter().expect("reserved counter"), 7);
        table
            .commit_storage_counter(owner, handle, 7)
            .expect("revalidate first reservation");

        let mut second = table
            .prepare_storage_key(owner, handle)
            .expect("second counter reservation");
        assert_eq!(second.reserve_counter().expect("reserved counter"), 8);
        assert_eq!(&second.material[..], &vec![0x41; 36]);
    }

    unsafe extern "C" fn atomic_counter_reservation(
        context: *mut c_void,
        counter_out: *mut u64,
    ) -> i32 {
        if context.is_null() || counter_out.is_null() {
            return 1;
        }
        let counter = unsafe { &*context.cast::<AtomicU64>() }.fetch_add(1, Ordering::SeqCst);
        unsafe { counter_out.write(counter) };
        0
    }

    unsafe extern "C" fn fixed_counter_reservation(
        context: *mut c_void,
        counter_out: *mut u64,
    ) -> i32 {
        if context.is_null() || counter_out.is_null() {
            return 1;
        }
        let counter = unsafe { &*context.cast::<AtomicU64>() }.load(Ordering::SeqCst);
        unsafe { counter_out.write(counter) };
        0
    }

    unsafe extern "C" fn failed_counter_reservation(
        _context: *mut c_void,
        _counter_out: *mut u64,
    ) -> i32 {
        7
    }

    #[test]
    fn production_callback_rejects_failure_repeat_regression_and_exhaustion() {
        let owner = ProcessId(56);
        let mut table = ResourceTable::new(Limits::for_tests(8, 8, 512, 64, 512));
        let next_counter = AtomicU64::new(7);
        let monotonic = table
            .insert_storage_key(
                owner,
                Zeroizing::new(vec![0x42; 36].into_boxed_slice()),
                StorageCounterSource::Production {
                    reserve: atomic_counter_reservation,
                    context: (&next_counter as *const AtomicU64)
                        .cast_mut()
                        .cast::<c_void>() as usize,
                    last_counter: None,
                },
            )
            .expect("insert production storage key");

        let mut first = table
            .prepare_storage_key(owner, monotonic)
            .expect("prepare first production reservation");
        assert_eq!(first.reserve_counter(), Ok(7));
        assert_eq!(
            first.reserve_counter(),
            Err(StorageKeyError::ReservationFailed),
            "one preparation must invoke its host callback at most once"
        );
        table
            .commit_storage_counter(owner, monotonic, 7)
            .expect("commit first production counter");
        let mut second = table
            .prepare_storage_key(owner, monotonic)
            .expect("prepare second production reservation");
        assert_eq!(second.reserve_counter(), Ok(8));
        table
            .commit_storage_counter(owner, monotonic, 8)
            .expect("commit second production counter");

        let fixed_counter = AtomicU64::new(9);
        let guarded = table
            .insert_storage_key(
                owner,
                Zeroizing::new(vec![0x43; 36].into_boxed_slice()),
                StorageCounterSource::Production {
                    reserve: fixed_counter_reservation,
                    context: (&fixed_counter as *const AtomicU64)
                        .cast_mut()
                        .cast::<c_void>() as usize,
                    last_counter: None,
                },
            )
            .expect("insert guarded production storage key");
        let mut initial = table
            .prepare_storage_key(owner, guarded)
            .expect("prepare fixed initial counter");
        assert_eq!(initial.reserve_counter(), Ok(9));
        table
            .commit_storage_counter(owner, guarded, 9)
            .expect("commit fixed initial counter");

        let mut repeated = table
            .prepare_storage_key(owner, guarded)
            .expect("prepare repeated counter");
        assert_eq!(repeated.reserve_counter(), Ok(9));
        assert_eq!(
            table.commit_storage_counter(owner, guarded, 9),
            Err(StorageKeyError::CounterNotMonotonic)
        );
        fixed_counter.store(8, Ordering::SeqCst);
        let mut regressed = table
            .prepare_storage_key(owner, guarded)
            .expect("prepare regressed counter");
        assert_eq!(regressed.reserve_counter(), Ok(8));
        assert_eq!(
            table.commit_storage_counter(owner, guarded, 8),
            Err(StorageKeyError::CounterNotMonotonic)
        );
        fixed_counter.store(u64::MAX, Ordering::SeqCst);
        let mut exhausted = table
            .prepare_storage_key(owner, guarded)
            .expect("prepare exhausted counter");
        assert_eq!(exhausted.reserve_counter(), Ok(u64::MAX));
        assert_eq!(
            table.commit_storage_counter(owner, guarded, u64::MAX),
            Err(StorageKeyError::CounterExhausted)
        );

        let failed = table
            .insert_storage_key(
                owner,
                Zeroizing::new(vec![0x44; 36].into_boxed_slice()),
                StorageCounterSource::Production {
                    reserve: failed_counter_reservation,
                    context: (&fixed_counter as *const AtomicU64)
                        .cast_mut()
                        .cast::<c_void>() as usize,
                    last_counter: None,
                },
            )
            .expect("insert failing production storage key");
        let mut failed = table
            .prepare_storage_key(owner, failed)
            .expect("prepare failed host reservation");
        assert_eq!(
            failed.reserve_counter(),
            Err(StorageKeyError::ReservationFailed)
        );
    }

    #[test]
    fn enforces_secret_count_and_byte_quotas() {
        let owner = ProcessId(61);

        let mut per_secret = ResourceTable::new(Limits::for_tests(8, 8, 64, 4, 64));
        assert!(matches!(
            per_secret.insert_secret(owner, vec![0; 5].into_boxed_slice()),
            Err(ResourceError::ResourceLimitExceeded)
        ));

        let mut per_actor_count = ResourceTable::new(Limits::for_tests(8, 1, 64, 64, 64));
        per_actor_count
            .insert_secret(owner, vec![0; 1].into_boxed_slice())
            .expect("first actor secret");
        assert!(matches!(
            per_actor_count.insert_secret(owner, vec![0; 1].into_boxed_slice()),
            Err(ResourceError::ResourceLimitExceeded)
        ));

        let mut per_actor_bytes = ResourceTable::new(Limits::for_tests(8, 8, 6, 64, 64));
        per_actor_bytes
            .insert_secret(owner, vec![0; 4].into_boxed_slice())
            .expect("first actor allocation");
        assert!(matches!(
            per_actor_bytes.insert_secret(owner, vec![0; 3].into_boxed_slice()),
            Err(ResourceError::ResourceLimitExceeded)
        ));

        let mut global_bytes = ResourceTable::new(Limits::for_tests(8, 8, 64, 64, 6));
        global_bytes
            .insert_secret(owner, vec![0; 4].into_boxed_slice())
            .expect("first global allocation");
        assert!(matches!(
            global_bytes.insert_secret(ProcessId(62), vec![0; 3].into_boxed_slice()),
            Err(ResourceError::ResourceLimitExceeded)
        ));

        let mut slots = ResourceTable::new(Limits::for_tests(1, 8, 64, 64, 64));
        slots
            .insert_secret(owner, vec![0; 1].into_boxed_slice())
            .expect("first slot");
        assert!(matches!(
            slots.insert_secret(ProcessId(63), vec![0; 1].into_boxed_slice()),
            Err(ResourceError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn destroy_zeroizes_secret_allocation_before_removal() {
        let owner = ProcessId(71);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));
        let handle = table
            .insert_secret(owner, vec![0xA5; 16].into_boxed_slice())
            .expect("insert sentinel secret");

        let removed = table
            .destroy(owner, handle)
            .expect("destroy secret")
            .expect("live allocation");

        assert!(removed.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn consume_invalidates_the_handle_and_releases_exact_quota() {
        let owner = ProcessId(72);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));
        let handle = table
            .insert(
                owner,
                ResourceKind::AeadKey,
                Zeroizing::new(vec![0xA5; 32].into_boxed_slice()),
            )
            .expect("insert AEAD key");

        let consumed = table
            .consume(owner, handle, ResourceKind::AeadKey)
            .expect("consume matching key");

        assert_eq!(&consumed[..], &[0xA5; 32]);
        assert!(matches!(
            table.with_resource(owner, handle, ResourceKind::AeadKey, |_| ()),
            Err(ResourceError::StaleHandle)
        ));
        assert_eq!(table.total_bytes, 0);
        assert!(!table.usage.contains_key(&owner));
    }

    #[test]
    fn valid_retype_keeps_the_zeroizing_allocation_under_a_fresh_handle() {
        let owner = ProcessId(73);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));
        let original = table
            .insert(
                owner,
                ResourceKind::SecretBytes,
                Zeroizing::new(vec![0x5A; 32].into_boxed_slice()),
            )
            .expect("insert source secret");
        let allocation = table.slots[original.slot as usize]
            .entry
            .as_ref()
            .expect("source entry")
            .bytes
            .as_ptr();

        let retyped = table
            .consume_and_retype(
                owner,
                original,
                ResourceKind::SecretBytes,
                ResourceKind::AeadKey,
                |bytes| (bytes.len() == 32).then_some(()).ok_or("invalid length"),
            )
            .expect("valid AEAD key length");

        assert_eq!(retyped.slot, original.slot);
        assert_eq!(retyped.generation, original.generation + 1);
        assert_eq!(retyped.kind, ResourceKind::AeadKey as u32);
        assert!(matches!(
            table.with_resource(owner, original, ResourceKind::SecretBytes, |_| ()),
            Err(ResourceError::StaleHandle)
        ));
        let retained = table
            .with_resource(owner, retyped, ResourceKind::AeadKey, |bytes| {
                (bytes.as_ptr(), bytes.len(), bytes[0])
            })
            .expect("read retyped key");
        assert_eq!(retained, (allocation, 32, 0x5A));
        assert_eq!(table.total_bytes, 32);
        assert_eq!(
            table
                .usage
                .get(&owner)
                .map(|usage| (usage.secrets, usage.bytes)),
            Some((1, 32))
        );
    }

    #[test]
    fn rejected_retype_consumes_the_source_and_zeroizes_removed_material() {
        let owner = ProcessId(74);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));
        let original = table
            .insert(
                owner,
                ResourceKind::SecretBytes,
                Zeroizing::new(vec![0xC7; 31].into_boxed_slice()),
            )
            .expect("insert invalid-length source secret");

        let error = table
            .consume_and_retype(
                owner,
                original,
                ResourceKind::SecretBytes,
                ResourceKind::AeadKey,
                |bytes| (bytes.len() == 32).then_some(()).ok_or("invalid length"),
            )
            .expect_err("invalid AEAD length must be rejected");

        let RetypeError::Rejected { error, removed } = error else {
            panic!("expected validation rejection");
        };
        assert_eq!(error, "invalid length");
        assert!(removed.iter().all(|byte| *byte == 0));
        assert!(matches!(
            table.with_resource(owner, original, ResourceKind::SecretBytes, |_| ()),
            Err(ResourceError::StaleHandle)
        ));
        assert_eq!(table.total_bytes, 0);
        assert!(!table.usage.contains_key(&owner));

        let replacement = table
            .insert(
                owner,
                ResourceKind::AeadKey,
                Zeroizing::new(vec![0x33; 32].into_boxed_slice()),
            )
            .expect("rejected source released slot and quota");
        assert_eq!(replacement.slot, original.slot);
        assert_eq!(replacement.generation, original.generation + 1);
    }

    #[test]
    fn generation_exhaustion_consumes_and_zeroizes_the_source() {
        let owner = ProcessId(75);
        let mut table = ResourceTable::new(Limits::for_tests(1, 4, 64, 64, 64));
        let inserted = table
            .insert(
                owner,
                ResourceKind::SecretBytes,
                Zeroizing::new(vec![0xD4; 32].into_boxed_slice()),
            )
            .expect("insert source secret");
        table.slots[inserted.slot as usize].generation = u32::MAX;
        let exhausted = ResourceHandle {
            generation: u32::MAX,
            ..inserted
        };

        let error = table
            .consume_and_retype(
                owner,
                exhausted,
                ResourceKind::SecretBytes,
                ResourceKind::AeadKey,
                |_| Ok::<_, ()>(()),
            )
            .expect_err("exhausted generation cannot issue a fresh handle");

        let RetypeError::GenerationExhausted { removed } = error else {
            panic!("expected generation exhaustion");
        };
        assert!(removed.iter().all(|byte| *byte == 0));
        assert!(matches!(
            table.with_resource(owner, exhausted, ResourceKind::SecretBytes, |_| ()),
            Err(ResourceError::StaleHandle)
        ));
        assert_eq!(table.total_bytes, 0);
        assert!(!table.usage.contains_key(&owner));
        assert!(matches!(
            table.insert(
                owner,
                ResourceKind::AeadKey,
                Zeroizing::new(vec![0x33; 32].into_boxed_slice()),
            ),
            Err(ResourceError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn destroy_rejects_wrong_owner_and_kind_without_removing_secret() {
        let owner = ProcessId(81);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));
        let handle = table
            .insert_secret(owner, vec![9; 8].into_boxed_slice())
            .expect("insert secret");

        assert!(matches!(
            table.destroy(ProcessId(82), handle),
            Err(ResourceError::WrongOwner)
        ));
        assert!(table
            .validate(owner, handle, ResourceKind::SecretBytes)
            .is_ok());

        let wrong_kind = ResourceHandle { kind: 99, ..handle };
        assert!(matches!(
            table.destroy(owner, wrong_kind),
            Err(ResourceError::WrongKind)
        ));
        assert!(table
            .validate(owner, handle, ResourceKind::SecretBytes)
            .is_ok());
    }

    #[test]
    fn destroy_releases_quota_and_repeated_destroy_is_idempotent() {
        let owner = ProcessId(91);
        let mut table = ResourceTable::new(Limits::for_tests(2, 1, 8, 8, 8));
        let first = table
            .insert_secret(owner, vec![3; 8].into_boxed_slice())
            .expect("first secret");

        assert!(table
            .destroy(owner, first)
            .expect("first destroy")
            .is_some());
        assert!(table
            .destroy(owner, first)
            .expect("repeated destroy")
            .is_none());
        table
            .insert_secret(owner, vec![4; 8].into_boxed_slice())
            .expect("destroyed allocation released quotas");
    }

    #[test]
    fn owner_cleanup_removes_only_owned_secrets_and_is_idempotent() {
        let owner = ProcessId(101);
        let other_actor = ProcessId(102);
        let mut table = ResourceTable::new(Limits::for_tests(8, 8, 128, 64, 160));
        let kinds = [
            ResourceKind::SecretBytes,
            ResourceKind::X25519PrivateKey,
            ResourceKind::SigningPrivateKey,
            ResourceKind::AeadKey,
        ];
        let owned: Vec<_> = kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                table
                    .insert(
                        owner,
                        kind,
                        Zeroizing::new(vec![index as u8 + 5; 8].into_boxed_slice()),
                    )
                    .expect("owned private resource")
            })
            .collect();
        let storage_key = table
            .insert_storage_key(
                owner,
                Zeroizing::new(vec![0x45; 36].into_boxed_slice()),
                StorageCounterSource::Deterministic { next_counter: 0 },
            )
            .expect("owned storage key");
        let other = table
            .insert(
                other_actor,
                ResourceKind::AeadKey,
                Zeroizing::new(vec![9; 8].into_boxed_slice()),
            )
            .expect("other actor private resource");

        assert_eq!(table.destroy_owned(owner), 5);
        for (handle, kind) in owned.into_iter().zip(kinds) {
            assert!(matches!(
                table.with_resource(owner, handle, kind, |_| ()),
                Err(ResourceError::StaleHandle)
            ));
        }
        assert!(matches!(
            table.with_resource(owner, storage_key, ResourceKind::StorageKey, |_| ()),
            Err(ResourceError::StaleHandle)
        ));
        assert!(table
            .validate(other_actor, other, ResourceKind::AeadKey)
            .is_ok());
        assert_eq!(table.total_bytes, 8);
        assert_eq!(
            table
                .usage
                .get(&other_actor)
                .map(|usage| (usage.secrets, usage.bytes)),
            Some((1, 8))
        );
        assert_eq!(table.destroy_owned(owner), 0);
    }

    #[test]
    fn random_rejects_invalid_length_with_typed_error() {
        for length in [-1, 0, MAX_SECRET_BYTES as i64 + 1] {
            let result = mesh_secret_random(length);
            let result = unsafe { &*result };
            let error = unsafe { &*(result.value as *const MeshCryptoError) };

            assert_eq!(result.tag, 1);
            assert_eq!(error.tag, CryptoErrorTag::InvalidLength as u8);
            assert_eq!(error.actual, length);
        }
    }

    #[test]
    fn handle_pointer_must_name_an_exact_live_actor_heap_allocation() {
        let owner = ProcessId(111);
        let mut process = Process::new(owner, Priority::Normal);
        let handle = ResourceHandle {
            slot: 3,
            generation: 7,
            kind: ResourceKind::SecretBytes as u32,
        };
        let pointer = allocate_handle(&mut process, handle);

        let validated = validate_handle_pointer(&process, pointer).expect("exact handle pointer");
        assert_eq!(validated.slot, handle.slot);
        assert_eq!(validated.generation, handle.generation);
        assert_eq!(validated.kind, handle.kind);

        let interior = unsafe { pointer.cast::<u8>().add(1).cast::<MeshSecretHandle>() };
        assert!(validate_handle_pointer(&process, interior).is_none());

        let wrong_size = process
            .heap
            .alloc(mem::size_of::<MeshSecretHandle>() + 1, HANDLE_ALIGNMENT)
            as *mut MeshSecretHandle;
        unsafe { wrong_size.write(MeshSecretHandle::from(handle)) };
        assert!(validate_handle_pointer(&process, wrong_size).is_none());
    }

    #[test]
    fn handle_from_a_larger_reused_heap_block_keeps_exact_abi_size() {
        let owner = ProcessId(112);
        let mut process = Process::new(owner, Priority::Normal);
        let larger = process.heap.alloc(64, HANDLE_ALIGNMENT);
        let larger_header = unsafe { GcHeader::from_data_ptr(larger) };
        let next = unsafe { (*larger_header).next };
        process.heap.set_all_objects_head(next);
        unsafe { (*larger_header).set_free() };
        process.heap.add_to_free_list(larger_header);

        let handle = ResourceHandle {
            slot: 4,
            generation: 8,
            kind: ResourceKind::AeadKey as u32,
        };
        let pointer = allocate_handle(&mut process, handle);

        assert_ne!(pointer.cast::<u8>(), larger);
        let header = unsafe { &*GcHeader::from_data_ptr(pointer.cast()) };
        assert_eq!(header.size as usize, mem::size_of::<MeshSecretHandle>());
        assert_eq!(validate_handle_pointer(&process, pointer), Some(handle));
        assert_eq!(process.heap.free_list_head(), larger_header);
    }

    #[test]
    fn owned_resource_helpers_validate_and_consume_actor_heap_handles() {
        let _global_table_test = GLOBAL_TABLE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owner = ProcessId(50_113);
        let mut process = Process::new(owner, Priority::Normal);
        let pointer = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x6B; 32].into_boxed_slice()),
        )
        .expect("insert actor-owned resource");

        let observed = with_owned_resource(&process, pointer, ResourceKind::SecretBytes, |bytes| {
            (bytes.len(), bytes[0], secret_table().try_lock().is_some())
        })
        .expect("read matching actor-owned resource");
        assert_eq!(observed, (32, 0x6B, true));
        assert!(matches!(
            with_owned_resource(&process, pointer, ResourceKind::AeadKey, |_| ()),
            Err(ResourceError::WrongKind)
        ));

        let retyped = consume_and_retype_owned_resource(
            &mut process,
            pointer,
            ResourceKind::SecretBytes,
            ResourceKind::AeadKey,
            |bytes| (bytes.len() == 32).then_some(()).ok_or("invalid length"),
        )
        .expect("retype valid AEAD key");
        assert!(matches!(
            with_owned_resource(&process, pointer, ResourceKind::SecretBytes, |_| ()),
            Err(ResourceError::StaleHandle)
        ));

        let consumed = consume_owned_resource(&process, retyped, ResourceKind::AeadKey)
            .expect("consume retyped actor-owned resource");
        assert_eq!(&consumed[..], &[0x6B; 32]);
        assert!(matches!(
            with_owned_resource(&process, retyped, ResourceKind::AeadKey, |_| ()),
            Err(ResourceError::StaleHandle)
        ));
        assert_eq!(destroy_owned(owner), 0);
    }

    #[test]
    fn prepared_identity_rejects_a_handle_pointer_rebound_to_a_new_generation() {
        let _global_table_test = GLOBAL_TABLE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owner = ProcessId(50_111);
        let mut process = Process::new(owner, Priority::Normal);
        let original = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x31; 32].into_boxed_slice()),
        )
        .expect("insert original resource");
        let prepared = prepare_owned_resource(&process, original, ResourceKind::SecretBytes)
            .expect("prepare original identity");
        drop(
            consume_owned_resource(&process, original, ResourceKind::SecretBytes)
                .expect("consume original resource"),
        );
        let replacement = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x32; 32].into_boxed_slice()),
        )
        .expect("insert replacement resource");
        let replacement_handle =
            validate_handle_pointer(&process, replacement).expect("replacement handle");

        // Simulate a collected handle allocation being reused at the same ABI
        // address. Pointer-based revalidation would now observe the replacement.
        unsafe { original.write(MeshSecretHandle::from(replacement_handle)) };
        assert!(prepare_owned_resource(&process, original, ResourceKind::SecretBytes).is_ok());
        assert_eq!(
            validate_prepared_owned_resource(&process, &prepared, ResourceKind::SecretBytes,),
            Err(ResourceError::StaleHandle)
        );
        destroy_owned(owner);
    }

    #[test]
    fn generic_resource_destructor_drops_every_registered_kind_idempotently() {
        let _global_table_test = GLOBAL_TABLE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _: extern "C" fn(*mut MeshSecretHandle) = mesh_resource_destroy;
        let owner = ProcessId(50_112);
        let mut process = Process::new(owner, Priority::Normal);
        let kinds = [
            ResourceKind::SecretBytes,
            ResourceKind::X25519PrivateKey,
            ResourceKind::SigningPrivateKey,
            ResourceKind::AeadKey,
        ];
        let resources: Vec<_> = kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                let pointer = insert_owned_resource(
                    &mut process,
                    kind,
                    Zeroizing::new(vec![index as u8 + 0x70; 32].into_boxed_slice()),
                )
                .expect("insert registered private resource");
                (pointer, kind)
            })
            .collect();

        for (pointer, kind) in resources {
            destroy_resource_for_process(&process, pointer, None);
            destroy_resource_for_process(&process, pointer, None);
            assert!(matches!(
                with_owned_resource(&process, pointer, kind, |_| ()),
                Err(ResourceError::StaleHandle)
            ));
        }

        let storage_key = insert_test_storage_key_resource(
            &mut process,
            Zeroizing::new(vec![0x73; 36].into_boxed_slice()),
            0,
        )
        .expect("insert storage key");
        destroy_resource_for_process(&process, storage_key, None);
        destroy_resource_for_process(&process, storage_key, None);
        assert!(matches!(
            with_owned_resource(&process, storage_key, ResourceKind::StorageKey, |_| ()),
            Err(ResourceError::StaleHandle)
        ));

        let aead_key = insert_owned_resource(
            &mut process,
            ResourceKind::AeadKey,
            Zeroizing::new(vec![0x72; 32].into_boxed_slice()),
        )
        .expect("insert AEAD key");

        destroy_resource_for_process(&process, aead_key, Some(ResourceKind::SecretBytes));
        assert!(with_owned_resource(&process, aead_key, ResourceKind::AeadKey, |_| ()).is_ok());
        destroy_resource_for_process(&process, aead_key, None);
        assert_eq!(owned_secret_count_for_test(owner), 0);
    }

    #[test]
    fn random_secret_is_stored_in_the_owner_table_not_the_actor_heap() {
        let owner = ProcessId(121);
        let mut process = Process::new(owner, Priority::Normal);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));

        let entry =
            create_random_secret_entry(&process, &mut table, 32).expect("operating-system entropy");
        let pointer = allocate_handle(&mut process, entry);
        let handle = validate_handle_pointer(&process, pointer).expect("opaque handle");
        let secret = table
            .validate(owner, handle, ResourceKind::SecretBytes)
            .expect("owner table entry");

        assert_eq!(secret.len(), 32);
        assert_eq!(mem::size_of::<MeshSecretHandle>(), 12);
    }

    #[test]
    fn concat_consumes_inputs_and_keeps_only_the_combined_secret() {
        let owner = ProcessId(130);
        let mut table = ResourceTable::new(Limits::for_tests(8, 8, 64, 64, 64));
        let first = table
            .insert_secret(owner, vec![1, 2].into_boxed_slice())
            .expect("first secret");
        let second = table
            .insert_secret(owner, vec![3, 4, 5].into_boxed_slice())
            .expect("second secret");

        let combined = table
            .concat_secrets(owner, first, second)
            .expect("combined secret");

        assert_eq!(
            table.with_resource(owner, combined, ResourceKind::SecretBytes, |bytes| bytes
                .to_vec()),
            Ok(vec![1, 2, 3, 4, 5])
        );
        assert_eq!(
            table.validate(owner, first, ResourceKind::SecretBytes),
            Err(ResourceError::StaleHandle)
        );
        assert_eq!(
            table.validate(owner, second, ResourceKind::SecretBytes),
            Err(ResourceError::StaleHandle)
        );
        assert_eq!(table.usage.get(&owner).map_or(0, |usage| usage.secrets), 1);

        let mut bounded = ResourceTable::new(Limits::for_tests(8, 8, 16, 8, 16));
        let oversized_first = bounded
            .insert_secret(owner, vec![1; 5].into_boxed_slice())
            .expect("bounded first secret");
        let oversized_second = bounded
            .insert_secret(owner, vec![2; 4].into_boxed_slice())
            .expect("bounded second secret");
        assert!(matches!(
            bounded.concat_secrets(owner, oversized_first, oversized_second),
            Err(ConcatSecretError::InvalidLength {
                maximum: 8,
                actual: 9
            })
        ));
        assert!(bounded.usage.get(&owner).is_none());
    }

    #[test]
    fn exited_actor_cannot_create_a_random_secret() {
        let owner = ProcessId(131);
        let mut process = Process::new(owner, Priority::Normal);
        process.mark_exited(crate::actor::ExitReason::Killed);
        let mut table = ResourceTable::new(Limits::for_tests(4, 4, 64, 64, 64));

        let result = create_random_secret_entry(&process, &mut table, 32);

        assert!(result.is_err());
        assert_eq!(table.usage.get(&owner).map_or(0, |usage| usage.secrets), 0);
    }
}
