//! Versioned storage wrapping for actor-owned private resources.

use std::ffi::c_void;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::actor::Process;
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::crypto::provider::{CryptoProvider, ProviderError, SystemProvider};
use crate::io::{alloc_result, MeshResult};
use crate::library::{secure_store_get_raw, secure_store_put_raw};
use crate::secret::{
    commit_storage_counter, crypto_error, insert_ephemeral_storage_key_resource,
    insert_owned_resource, insert_storage_key_resource, prepare_owned_resource,
    prepare_storage_key_resource, validate_prepared_owned_resource,
    validate_prepared_storage_key_resource, CryptoErrorTag, MeshSecretHandle,
    MeshStorageCounterReserve, PreparedOwnedResource, PreparedStorageKey, ResourceError,
    ResourceKind, StorageKeyError,
};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const DOMAIN_LABEL: &[u8] = b"mesh-msg/v1/storage-wrap";
const CONTEXT_BYTES: usize = 123;
const PLAINTEXT_BYTES: usize = 32;
const MLKEM_PRIVATE_SEED_BYTES: usize = 64;
const STORAGE_KEY_MATERIAL_BYTES: usize = 36;
const NONCE_BYTES: usize = 12;
const BINDING_BYTES: usize = 32;
const TAG_BYTES: usize = 16;
const FIXED_OVERHEAD_BYTES: usize = 67;
const MAX_PLAINTEXT_BYTES: usize = 65_536;
const MAX_BLOB_BYTES: usize = FIXED_OVERHEAD_BYTES + MAX_PLAINTEXT_BYTES;
const FORMAT_VERSION: u8 = 1;
const ALGORITHM_CHACHA20_POLY1305: u16 = 1;
const PLATFORM_KEY_ID: &[u8] = b"mesh/storage-key/v1";
const PLATFORM_COUNTER_ID: &[u8] = b"mesh/storage-counter/v1";
const HOST_NOT_FOUND: i32 = 2;

static PLATFORM_STORAGE_LOCK: Mutex<()> = Mutex::new(());

const CONTEXT_VERSION_OFFSET: usize = 0;
const CONTEXT_SESSION_OFFSET: usize = 49;
const CONTEXT_SESSION_END: usize = 81;
const CONTEXT_PURPOSE_OFFSET: usize = 113;
const CONTEXT_SNAPSHOT_OFFSET: usize = 115;

const BLOB_ALGORITHM_OFFSET: usize = 1;
const BLOB_NONCE_OFFSET: usize = 3;
const BLOB_BINDING_OFFSET: usize = 15;
const BLOB_LENGTH_OFFSET: usize = 47;
const BLOB_CIPHERTEXT_OFFSET: usize = 51;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StorageFailure {
    tag: CryptoErrorTag,
    expected: i64,
    actual: i64,
}

fn failure(tag: CryptoErrorTag, expected: i64, actual: i64) -> StorageFailure {
    StorageFailure {
        tag,
        expected,
        actual,
    }
}

fn invalid_length(expected: usize, actual: usize) -> StorageFailure {
    failure(
        CryptoErrorTag::InvalidLength,
        expected as i64,
        i64::try_from(actual).unwrap_or(i64::MAX),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecretPurpose {
    RootKey,
    SendingChainKey,
    ReceivingChainKey,
    HeaderKey,
    AttachmentKey,
    AccountAuthorizationKey,
    DeviceSigningKey,
    DeviceDhKey,
    SignedPrekey,
    OneTimePrekey,
    SkippedMessageKey,
    SkippedKeyMap,
    RatchetDhKey,
    LocalData,
    PostQuantumPrekey,
    GroupEpochSecret,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageValueKind {
    Resource(ResourceKind),
    Bytes,
}

impl StorageValueKind {
    fn code(self) -> i64 {
        match self {
            Self::Resource(kind) => kind as i64,
            Self::Bytes => 7,
        }
    }
}

impl From<ResourceKind> for StorageValueKind {
    fn from(value: ResourceKind) -> Self {
        Self::Resource(value)
    }
}

impl SecretPurpose {
    fn from_id(id: u16) -> Result<Self, StorageFailure> {
        match id {
            1 => Ok(Self::RootKey),
            2 => Ok(Self::SendingChainKey),
            3 => Ok(Self::ReceivingChainKey),
            4 => Ok(Self::HeaderKey),
            5 => Ok(Self::AttachmentKey),
            6 => Ok(Self::AccountAuthorizationKey),
            7 => Ok(Self::DeviceSigningKey),
            8 => Ok(Self::DeviceDhKey),
            9 => Ok(Self::SignedPrekey),
            10 => Ok(Self::OneTimePrekey),
            11 => Ok(Self::SkippedMessageKey),
            12 => Ok(Self::SkippedKeyMap),
            13 => Ok(Self::RatchetDhKey),
            14 => Ok(Self::LocalData),
            15 => Ok(Self::PostQuantumPrekey),
            16 => Ok(Self::GroupEpochSecret),
            _ => Err(failure(CryptoErrorTag::UnsupportedOperation, 0, id as i64)),
        }
    }

    fn value_kind(self) -> StorageValueKind {
        match self {
            Self::RootKey
            | Self::SendingChainKey
            | Self::ReceivingChainKey
            | Self::HeaderKey
            | Self::AttachmentKey
            | Self::SkippedMessageKey
            | Self::GroupEpochSecret => ResourceKind::SecretBytes.into(),
            Self::SkippedKeyMap => ResourceKind::SecretMap.into(),
            Self::AccountAuthorizationKey | Self::DeviceSigningKey => {
                ResourceKind::SigningPrivateKey.into()
            }
            Self::DeviceDhKey | Self::SignedPrekey | Self::OneTimePrekey | Self::RatchetDhKey => {
                ResourceKind::X25519PrivateKey.into()
            }
            Self::LocalData => StorageValueKind::Bytes,
            Self::PostQuantumPrekey => ResourceKind::MlKemPrivateKey.into(),
        }
    }

    fn requires_zero_session_id(self) -> bool {
        matches!(
            self,
            Self::AttachmentKey
                | Self::AccountAuthorizationKey
                | Self::DeviceSigningKey
                | Self::DeviceDhKey
                | Self::SignedPrekey
                | Self::OneTimePrekey
                | Self::PostQuantumPrekey
        )
    }
}

fn validate_plaintext_length(
    expected_kind: StorageValueKind,
    length: usize,
) -> Result<(), StorageFailure> {
    match expected_kind {
        StorageValueKind::Resource(ResourceKind::SecretMap) => {
            if length == 0 || length > MAX_PLAINTEXT_BYTES {
                return Err(invalid_length(MAX_PLAINTEXT_BYTES, length));
            }
        }
        StorageValueKind::Bytes => {
            if length > MAX_PLAINTEXT_BYTES {
                return Err(invalid_length(MAX_PLAINTEXT_BYTES, length));
            }
        }
        StorageValueKind::Resource(ResourceKind::MlKemPrivateKey) => {
            if length != MLKEM_PRIVATE_SEED_BYTES {
                return Err(invalid_length(MLKEM_PRIVATE_SEED_BYTES, length));
            }
        }
        StorageValueKind::Resource(_) if length != PLAINTEXT_BYTES => {
            return Err(invalid_length(PLAINTEXT_BYTES, length));
        }
        StorageValueKind::Resource(_) => {}
    }
    Ok(())
}

fn validate_context<K: Into<StorageValueKind>>(
    context: &[u8],
    expected_kind: K,
) -> Result<SecretPurpose, StorageFailure> {
    let expected_kind = expected_kind.into();
    if context.len() != CONTEXT_BYTES {
        return Err(invalid_length(CONTEXT_BYTES, context.len()));
    }
    if context[CONTEXT_VERSION_OFFSET] != FORMAT_VERSION {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            FORMAT_VERSION as i64,
            context[CONTEXT_VERSION_OFFSET] as i64,
        ));
    }
    let purpose_id = u16::from_be_bytes([
        context[CONTEXT_PURPOSE_OFFSET],
        context[CONTEXT_PURPOSE_OFFSET + 1],
    ]);
    let purpose = SecretPurpose::from_id(purpose_id)?;
    if purpose.value_kind() != expected_kind {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            expected_kind.code(),
            purpose.value_kind().code(),
        ));
    }
    let snapshot = u64::from_be_bytes(
        context[CONTEXT_SNAPSHOT_OFFSET..CONTEXT_BYTES]
            .try_into()
            .expect("fixed context snapshot range"),
    );
    if snapshot == 0 {
        return Err(failure(CryptoErrorTag::InvalidLength, 1, 0));
    }
    if purpose.requires_zero_session_id()
        && context[CONTEXT_SESSION_OFFSET..CONTEXT_SESSION_END]
            .iter()
            .any(|byte| *byte != 0)
    {
        return Err(failure(CryptoErrorTag::InvalidLength, 0, 32));
    }
    Ok(purpose)
}

fn context_binding(provider: &impl CryptoProvider, context: &[u8]) -> [u8; BINDING_BYTES] {
    let mut input = Vec::with_capacity(DOMAIN_LABEL.len() + context.len());
    input.extend_from_slice(DOMAIN_LABEL);
    input.extend_from_slice(context);
    provider.sha256(&input)
}

fn associated_data(binding: &[u8; BINDING_BYTES], ciphertext_length: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(DOMAIN_LABEL.len() + 1 + 2 + BINDING_BYTES + 4);
    data.extend_from_slice(DOMAIN_LABEL);
    data.push(FORMAT_VERSION);
    data.extend_from_slice(&ALGORITHM_CHACHA20_POLY1305.to_be_bytes());
    data.extend_from_slice(binding);
    data.extend_from_slice(&ciphertext_length.to_be_bytes());
    data
}

fn provider_failure(error: ProviderError) -> StorageFailure {
    match error {
        ProviderError::AuthenticationFailed => failure(CryptoErrorTag::AuthenticationFailed, 0, 0),
        ProviderError::InvalidLength => failure(CryptoErrorTag::InvalidLength, 0, 0),
        ProviderError::EntropyUnavailable => failure(CryptoErrorTag::EntropyUnavailable, 0, 0),
        ProviderError::InvalidPublicKey => failure(CryptoErrorTag::InternalFailure, 0, 0),
    }
}

fn seal_value<K: Into<StorageValueKind> + Copy>(
    provider: &impl CryptoProvider,
    plaintext: &[u8],
    storage_key_material: &[u8],
    counter: u64,
    context: &[u8],
    expected_kind: K,
) -> Result<Vec<u8>, StorageFailure> {
    let expected_kind = expected_kind.into();
    validate_context(context, expected_kind)?;
    validate_plaintext_length(expected_kind, plaintext.len())?;
    if storage_key_material.len() != STORAGE_KEY_MATERIAL_BYTES {
        return Err(failure(
            CryptoErrorTag::InvalidKey,
            STORAGE_KEY_MATERIAL_BYTES as i64,
            storage_key_material.len() as i64,
        ));
    }

    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&storage_key_material[..32]);
    let mut nonce = [0u8; NONCE_BYTES];
    nonce[..4].copy_from_slice(&storage_key_material[32..]);
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    let binding = context_binding(provider, context);
    let ciphertext_length = u32::try_from(plaintext.len()).expect("bounded plaintext length");
    let associated_data = associated_data(&binding, ciphertext_length);
    let ciphertext_and_tag = provider
        .chacha20poly1305_seal(&key, &nonce, &associated_data, plaintext)
        .map_err(provider_failure)?;
    if ciphertext_and_tag.len() != plaintext.len() + TAG_BYTES {
        return Err(failure(CryptoErrorTag::InternalFailure, 0, 0));
    }
    let tag_offset = ciphertext_and_tag.len() - TAG_BYTES;

    let mut blob = Vec::with_capacity(FIXED_OVERHEAD_BYTES + plaintext.len());
    blob.push(FORMAT_VERSION);
    blob.extend_from_slice(&ALGORITHM_CHACHA20_POLY1305.to_be_bytes());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&binding);
    blob.extend_from_slice(&ciphertext_length.to_be_bytes());
    blob.extend_from_slice(&ciphertext_and_tag[..tag_offset]);
    blob.extend_from_slice(&ciphertext_and_tag[tag_offset..]);
    Ok(blob)
}

fn open_value<K: Into<StorageValueKind> + Copy>(
    provider: &impl CryptoProvider,
    blob: &[u8],
    storage_key_material: &[u8],
    context: &[u8],
    expected_kind: K,
) -> Result<Zeroizing<Box<[u8]>>, StorageFailure> {
    let expected_kind = expected_kind.into();
    // The validation order here is part of the on-disk format contract.
    if blob.len() < FIXED_OVERHEAD_BYTES {
        return Err(invalid_length(FIXED_OVERHEAD_BYTES, blob.len()));
    }
    if blob[0] != FORMAT_VERSION {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            FORMAT_VERSION as i64,
            blob[0] as i64,
        ));
    }
    let algorithm =
        u16::from_be_bytes([blob[BLOB_ALGORITHM_OFFSET], blob[BLOB_ALGORITHM_OFFSET + 1]]);
    if algorithm != ALGORITHM_CHACHA20_POLY1305 {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            ALGORITHM_CHACHA20_POLY1305 as i64,
            algorithm as i64,
        ));
    }

    let ciphertext_length = u32::from_be_bytes(
        blob[BLOB_LENGTH_OFFSET..BLOB_CIPHERTEXT_OFFSET]
            .try_into()
            .expect("fixed blob length range"),
    ) as usize;
    if ciphertext_length > MAX_PLAINTEXT_BYTES {
        return Err(invalid_length(MAX_PLAINTEXT_BYTES, ciphertext_length));
    }
    let expected_total = FIXED_OVERHEAD_BYTES
        .checked_add(ciphertext_length)
        .ok_or_else(|| invalid_length(MAX_BLOB_BYTES, blob.len()))?;
    if blob.len() != expected_total {
        return Err(invalid_length(expected_total, blob.len()));
    }

    let supplied_binding: &[u8; BINDING_BYTES] = blob[BLOB_BINDING_OFFSET..BLOB_LENGTH_OFFSET]
        .try_into()
        .expect("fixed blob binding range");
    let expected_binding = context_binding(provider, context);
    if supplied_binding.ct_eq(&expected_binding).unwrap_u8() != 1 {
        return Err(failure(CryptoErrorTag::AuthenticationFailed, 0, 0));
    }
    if storage_key_material.len() != STORAGE_KEY_MATERIAL_BYTES {
        return Err(failure(
            CryptoErrorTag::InvalidKey,
            STORAGE_KEY_MATERIAL_BYTES as i64,
            storage_key_material.len() as i64,
        ));
    }

    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&storage_key_material[..32]);
    let nonce: &[u8; NONCE_BYTES] = blob[BLOB_NONCE_OFFSET..BLOB_BINDING_OFFSET]
        .try_into()
        .expect("fixed blob nonce range");
    let associated_data = associated_data(
        &expected_binding,
        u32::try_from(ciphertext_length).expect("bounded ciphertext length"),
    );
    let tag_offset = BLOB_CIPHERTEXT_OFFSET + ciphertext_length;
    let mut plaintext = Zeroizing::new(Vec::with_capacity(ciphertext_length + TAG_BYTES));
    plaintext.extend_from_slice(&blob[BLOB_CIPHERTEXT_OFFSET..tag_offset]);
    plaintext.extend_from_slice(&blob[tag_offset..]);
    provider
        .chacha20poly1305_open(&key, nonce, &associated_data, &mut plaintext)
        .map_err(provider_failure)?;

    // Purpose and plaintext semantics are checked only after authentication.
    validate_plaintext_length(expected_kind, plaintext.len())?;
    validate_context(context, expected_kind)?;
    let plaintext = std::mem::take(&mut *plaintext).into_boxed_slice();
    Ok(Zeroizing::new(plaintext))
}

struct SealPreparation {
    secret: PreparedOwnedResource,
    storage_key: PreparedStorageKey,
}

fn resource_failure(error: ResourceError) -> StorageFailure {
    match error {
        ResourceError::ResourceLimitExceeded => {
            failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
        ResourceError::WrongKind => failure(CryptoErrorTag::InvalidKey, 0, 0),
        ResourceError::StaleHandle | ResourceError::WrongOwner | ResourceError::OwnerExited => {
            failure(CryptoErrorTag::SecretDestroyed, 0, 0)
        }
    }
}

fn storage_key_failure(error: StorageKeyError) -> StorageFailure {
    match error {
        StorageKeyError::Resource(error) => resource_failure(error),
        StorageKeyError::ReservationFailed | StorageKeyError::CounterNotMonotonic => {
            failure(CryptoErrorTag::InternalFailure, 0, 0)
        }
        StorageKeyError::CounterExhausted => failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0),
    }
}

fn prepare_seal(
    process: &Process,
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: &[u8],
    expected_kind: ResourceKind,
) -> Result<SealPreparation, StorageFailure> {
    validate_context(context, expected_kind)?;
    let secret =
        prepare_owned_resource(process, secret, expected_kind).map_err(resource_failure)?;
    validate_plaintext_length(expected_kind.into(), secret.bytes.len())?;
    let storage_key =
        prepare_storage_key_resource(process, wrapping_key).map_err(storage_key_failure)?;
    Ok(SealPreparation {
        secret,
        storage_key,
    })
}

fn commit_reserved_seal(
    process: &Process,
    preparation: &SealPreparation,
    counter: u64,
    expected_kind: ResourceKind,
) -> Result<(), StorageFailure> {
    // Commit first: if the secret disappeared during the host call, the
    // already-reserved counter remains burned in both durable and runtime state.
    commit_storage_counter(process, &preparation.storage_key, counter)
        .map_err(storage_key_failure)?;
    validate_prepared_owned_resource(process, &preparation.secret, expected_kind)
        .map_err(resource_failure)
}

fn revalidate_seal_inputs(
    process: &Process,
    preparation: &SealPreparation,
    expected_kind: ResourceKind,
) -> Result<(), StorageFailure> {
    validate_prepared_storage_key_resource(process, &preparation.storage_key)
        .map_err(storage_key_failure)?;
    validate_prepared_owned_resource(process, &preparation.secret, expected_kind)
        .map_err(resource_failure)
}

#[cfg(test)]
fn open_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    blob: &[u8],
    wrapping_key: *const MeshSecretHandle,
    context: &[u8],
    expected_kind: ResourceKind,
) -> Result<*mut MeshSecretHandle, StorageFailure> {
    let storage_key = prepare_owned_resource(process, wrapping_key, ResourceKind::StorageKey)
        .map_err(resource_failure)?;
    let plaintext = open_value(provider, blob, &storage_key.bytes, context, expected_kind)?;
    validate_prepared_owned_resource(process, &storage_key, ResourceKind::StorageKey)
        .map_err(resource_failure)?;
    insert_owned_resource(process, expected_kind, plaintext).map_err(resource_failure)
}

#[cfg(test)]
fn seal_for_process_with_hook(
    process: &mut Process,
    provider: &impl CryptoProvider,
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: &[u8],
    expected_kind: ResourceKind,
    after_reservation: impl FnOnce(&mut Process),
) -> Result<Vec<u8>, StorageFailure> {
    let mut preparation = prepare_seal(process, secret, wrapping_key, context, expected_kind)?;
    let counter = preparation
        .storage_key
        .reserve_counter()
        .map_err(storage_key_failure)?;
    after_reservation(process);
    commit_reserved_seal(process, &preparation, counter, expected_kind)?;
    let blob = seal_value(
        provider,
        &preparation.secret.bytes,
        &preparation.storage_key.material,
        counter,
        context,
        expected_kind,
    )?;
    revalidate_seal_inputs(process, &preparation, expected_kind)?;
    Ok(blob)
}

fn current_process() -> Result<Arc<Mutex<Process>>, StorageFailure> {
    let pid = crate::actor::stack::get_current_pid()
        .ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let scheduler = crate::actor::GLOBAL_SCHEDULER
        .get()
        .ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    scheduler
        .get_process(pid)
        .ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))
}

unsafe fn copy_mesh_bytes(
    value: *const MeshBytes,
    maximum: usize,
) -> Result<Vec<u8>, StorageFailure> {
    if value.is_null() {
        return Err(failure(CryptoErrorTag::InvalidLength, maximum as i64, -1));
    }
    let length = usize::try_from(unsafe { (*value).len })
        .map_err(|_| invalid_length(maximum, usize::MAX))?;
    if length > maximum {
        return Err(invalid_length(maximum, length));
    }
    Ok(unsafe { (*value).as_slice() }.to_vec())
}

unsafe fn copy_native_exact(
    value: *const u8,
    length: u64,
    expected: usize,
) -> Result<Zeroizing<Vec<u8>>, StorageFailure> {
    if value.is_null() || length != expected as u64 {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            expected as i64,
            if value.is_null() {
                -1
            } else {
                i64::try_from(length).unwrap_or(i64::MAX)
            },
        ));
    }
    Ok(Zeroizing::new(
        unsafe { std::slice::from_raw_parts(value, expected) }.to_vec(),
    ))
}

fn error_result(error: StorageFailure) -> *mut MeshResult {
    crypto_error(error.tag, error.expected, error.actual)
}

fn ok_result<T>(value: *mut T) -> *mut MeshResult {
    if value.is_null() {
        error_result(failure(CryptoErrorTag::InternalFailure, 0, 0))
    } else {
        alloc_result(0, value.cast())
    }
}

fn seal_for_current_actor(
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
    expected_kind: ResourceKind,
) -> Result<Vec<u8>, StorageFailure> {
    let context = unsafe { copy_mesh_bytes(context, CONTEXT_BYTES) }?;
    let process = current_process()?;
    let mut preparation = {
        let process = process.lock();
        prepare_seal(&process, secret, wrapping_key, &context, expected_kind)?
    };

    // No Process or resource-table guard is held across this host callback.
    let counter = preparation
        .storage_key
        .reserve_counter()
        .map_err(storage_key_failure)?;
    {
        let process = process.lock();
        commit_reserved_seal(&process, &preparation, counter, expected_kind)?;
    }

    let blob = seal_value(
        &SystemProvider,
        &preparation.secret.bytes,
        &preparation.storage_key.material,
        counter,
        &context,
        expected_kind,
    )?;
    {
        let process = process.lock();
        revalidate_seal_inputs(&process, &preparation, expected_kind)?;
    }
    Ok(blob)
}

fn unseal_for_current_actor(
    blob: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
    expected_kind: ResourceKind,
) -> Result<*mut MeshSecretHandle, StorageFailure> {
    let blob = unsafe { copy_mesh_bytes(blob, MAX_BLOB_BYTES) }?;
    let context = unsafe { copy_mesh_bytes(context, CONTEXT_BYTES) }?;
    let process = current_process()?;
    let storage_key = {
        let process = process.lock();
        prepare_owned_resource(&process, wrapping_key, ResourceKind::StorageKey)
            .map_err(resource_failure)?
    };
    let plaintext = open_value(
        &SystemProvider,
        &blob,
        &storage_key.bytes,
        &context,
        expected_kind,
    )?;
    let handle = {
        let mut process = process.lock();
        validate_prepared_owned_resource(&process, &storage_key, ResourceKind::StorageKey)
            .map_err(resource_failure)?;
        insert_owned_resource(&mut process, expected_kind, plaintext).map_err(resource_failure)?
    };
    Ok(handle)
}

/// Create a process-local storage key for snapshots that do not need to
/// survive a process restart. Persistent applications must provision a
/// platform-backed key through `mesh_storage_key_provision` instead.
#[no_mangle]
pub extern "C" fn mesh_storage_key_ephemeral() -> *mut MeshResult {
    let result = (|| {
        let mut material = Zeroizing::new(vec![0u8; STORAGE_KEY_MATERIAL_BYTES]);
        SystemProvider
            .fill_random(&mut material)
            .map_err(provider_failure)?;
        let material = Zeroizing::new(std::mem::take(&mut *material).into_boxed_slice());
        let process = current_process()?;
        let handle = {
            let mut process = process.lock();
            insert_ephemeral_storage_key_resource(&mut process, material)
                .map_err(resource_failure)?
        };
        Ok(handle)
    })();
    match result {
        Ok(handle) => ok_result(handle),
        Err(error) => error_result(error),
    }
}

fn platform_failure(status: i32) -> StorageFailure {
    failure(CryptoErrorTag::InternalFailure, 0, status as i64)
}

fn read_platform_record(
    key: &[u8],
    maximum: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>, StorageFailure> {
    let mut output = Zeroizing::new(vec![0u8; maximum]);
    match secure_store_get_raw(key, &mut output) {
        Ok(length) => {
            output.truncate(length);
            Ok(Some(output))
        }
        Err(HOST_NOT_FOUND) => Ok(None),
        Err(status) => Err(platform_failure(status)),
    }
}

fn write_platform_record(key: &[u8], value: &[u8]) -> Result<(), StorageFailure> {
    let key_length =
        u32::try_from(key.len()).map_err(|_| invalid_length(u32::MAX as usize, key.len()))?;
    let mut request = Zeroizing::new(Vec::with_capacity(4 + key.len() + value.len()));
    request.extend_from_slice(&key_length.to_be_bytes());
    request.extend_from_slice(key);
    request.extend_from_slice(value);
    secure_store_put_raw(&request).map_err(platform_failure)
}

fn load_platform_material() -> Result<Zeroizing<Box<[u8]>>, StorageFailure> {
    let material = match read_platform_record(PLATFORM_KEY_ID, STORAGE_KEY_MATERIAL_BYTES)? {
        Some(material) if material.len() == STORAGE_KEY_MATERIAL_BYTES => material,
        Some(material) => return Err(invalid_length(STORAGE_KEY_MATERIAL_BYTES, material.len())),
        None => {
            let mut material = Zeroizing::new(vec![0u8; STORAGE_KEY_MATERIAL_BYTES]);
            SystemProvider
                .fill_random(&mut material)
                .map_err(provider_failure)?;
            write_platform_record(PLATFORM_KEY_ID, &material)?;
            material
        }
    };
    match read_platform_record(PLATFORM_COUNTER_ID, 8)? {
        Some(counter) if counter.len() == 8 => {}
        Some(counter) => return Err(invalid_length(8, counter.len())),
        None => write_platform_record(PLATFORM_COUNTER_ID, &0u64.to_be_bytes())?,
    }
    Ok(Zeroizing::new(material.to_vec().into_boxed_slice()))
}

// ponytail: one process-wide lock; use a host atomic-increment callback if app
// extensions ever share this storage key concurrently.
unsafe extern "C" fn reserve_platform_counter(_context: *mut c_void, counter_out: *mut u64) -> i32 {
    if counter_out.is_null() {
        return 1;
    }
    let _guard = PLATFORM_STORAGE_LOCK.lock();
    let result = (|| {
        let counter = read_platform_record(PLATFORM_COUNTER_ID, 8)?
            .ok_or_else(|| platform_failure(HOST_NOT_FOUND))?;
        if counter.len() != 8 {
            return Err(invalid_length(8, counter.len()));
        }
        let current = u64::from_be_bytes(counter.as_slice().try_into().expect("checked counter"));
        let next = current
            .checked_add(1)
            .ok_or_else(|| failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0))?;
        write_platform_record(PLATFORM_COUNTER_ID, &next.to_be_bytes())?;
        unsafe { counter_out.write(current) };
        Ok(())
    })();
    result.map_or(1, |_| 0)
}

/// Load or create the app-wide storage key through the registered secure-store
/// callbacks without exposing its material as Mesh `Bytes`.
#[no_mangle]
pub extern "C" fn mesh_storage_key_platform() -> *mut MeshResult {
    let result = (|| {
        let _guard = PLATFORM_STORAGE_LOCK.lock();
        let material = load_platform_material()?;
        let process = current_process()?;
        let context = std::ptr::NonNull::<u8>::dangling()
            .as_ptr()
            .cast::<c_void>();
        let handle = {
            let mut process = process.lock();
            insert_storage_key_resource(&mut process, material, reserve_platform_counter, context)
                .map_err(resource_failure)?
        };
        Ok(handle)
    })();
    match result {
        Ok(handle) => ok_result(handle),
        Err(error) => error_result(error),
    }
}

fn seal_bytes_for_current_actor(
    value: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> Result<Vec<u8>, StorageFailure> {
    let value = Zeroizing::new(unsafe { copy_mesh_bytes(value, MAX_PLAINTEXT_BYTES) }?);
    let context = unsafe { copy_mesh_bytes(context, CONTEXT_BYTES) }?;
    validate_context(&context, StorageValueKind::Bytes)?;
    let process = current_process()?;
    let mut storage_key = {
        let process = process.lock();
        prepare_storage_key_resource(&process, wrapping_key).map_err(storage_key_failure)?
    };
    let counter = storage_key.reserve_counter().map_err(storage_key_failure)?;
    {
        let process = process.lock();
        commit_storage_counter(&process, &storage_key, counter).map_err(storage_key_failure)?;
    }
    let blob = seal_value(
        &SystemProvider,
        &value,
        &storage_key.material,
        counter,
        &context,
        StorageValueKind::Bytes,
    )?;
    {
        let process = process.lock();
        validate_prepared_storage_key_resource(&process, &storage_key)
            .map_err(storage_key_failure)?;
    }
    Ok(blob)
}

fn unseal_bytes_for_current_actor(
    blob: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> Result<Zeroizing<Box<[u8]>>, StorageFailure> {
    let blob = unsafe { copy_mesh_bytes(blob, MAX_BLOB_BYTES) }?;
    let context = unsafe { copy_mesh_bytes(context, CONTEXT_BYTES) }?;
    let process = current_process()?;
    let storage_key = {
        let process = process.lock();
        prepare_owned_resource(&process, wrapping_key, ResourceKind::StorageKey)
            .map_err(resource_failure)?
    };
    let plaintext = open_value(
        &SystemProvider,
        &blob,
        &storage_key.bytes,
        &context,
        StorageValueKind::Bytes,
    )?;
    {
        let process = process.lock();
        validate_prepared_owned_resource(&process, &storage_key, ResourceKind::StorageKey)
            .map_err(resource_failure)?;
    }
    Ok(plaintext)
}

#[no_mangle]
pub extern "C" fn mesh_storage_key_seal_bytes(
    value: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> *mut MeshResult {
    match seal_bytes_for_current_actor(value, wrapping_key, context) {
        Ok(blob) => ok_result(mesh_bytes_new(blob.as_ptr(), blob.len() as u64)),
        Err(error) => error_result(error),
    }
}

#[no_mangle]
pub extern "C" fn mesh_storage_key_unseal_bytes(
    blob: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> *mut MeshResult {
    match unseal_bytes_for_current_actor(blob, wrapping_key, context) {
        Ok(value) => ok_result(mesh_bytes_new(value.as_ptr(), value.len() as u64)),
        Err(error) => error_result(error),
    }
}

/// Provision a platform-backed storage key and durable nonce reservation
/// callback. Native key material is borrowed only for this call.
///
/// Host contract:
///
/// - The callback code and `callback_context` must remain valid and thread-safe
///   until runtime shutdown. Forced actor exit can destroy the `StorageKey`
///   while a reservation is still in flight, and destruction does not notify
///   the host.
/// - The callback must atomically return the current durable counter through
///   `counter_out` and increment the persisted record before returning zero.
/// - Zero means success and a fully initialized `counter_out`; any nonzero
///   status means failure and the output is ignored.
/// - A counter exposed by a successful reservation is permanently consumed,
///   even if validation, encryption, or output allocation later fails.
/// - The callback must never re-enter Mesh or attempt to acquire actor/resource
///   locks. The runtime releases those locks before invoking it, but the call
///   executes on the actor thread and should not block indefinitely.
#[no_mangle]
pub extern "C" fn mesh_storage_key_provision(
    key: *const u8,
    key_length: u64,
    nonce_prefix: *const u8,
    nonce_prefix_length: u64,
    reserve_counter: Option<MeshStorageCounterReserve>,
    callback_context: *mut c_void,
) -> *mut MeshResult {
    let result = (|| {
        let key = unsafe { copy_native_exact(key, key_length, 32) }?;
        let prefix = unsafe { copy_native_exact(nonce_prefix, nonce_prefix_length, 4) }?;
        let reserve_counter =
            reserve_counter.ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
        if callback_context.is_null() {
            return Err(failure(CryptoErrorTag::InternalFailure, 0, 0));
        }
        let mut material = Zeroizing::new(Vec::with_capacity(STORAGE_KEY_MATERIAL_BYTES));
        material.extend_from_slice(&key);
        material.extend_from_slice(&prefix);
        let material = Zeroizing::new(std::mem::take(&mut *material).into_boxed_slice());
        let process = current_process()?;
        let handle = {
            let mut process = process.lock();
            insert_storage_key_resource(&mut process, material, reserve_counter, callback_context)
                .map_err(resource_failure)?
        };
        Ok(handle)
    })();
    match result {
        Ok(handle) => ok_result(handle),
        Err(error) => error_result(error),
    }
}

macro_rules! storage_seal_abi {
    ($name:ident, $kind:expr) => {
        #[no_mangle]
        pub extern "C" fn $name(
            secret: *const MeshSecretHandle,
            wrapping_key: *const MeshSecretHandle,
            context: *const MeshBytes,
        ) -> *mut MeshResult {
            match seal_for_current_actor(secret, wrapping_key, context, $kind) {
                Ok(blob) => ok_result(mesh_bytes_new(blob.as_ptr(), blob.len() as u64)),
                Err(error) => error_result(error),
            }
        }
    };
}

macro_rules! storage_unseal_abi {
    ($name:ident, $kind:expr) => {
        #[no_mangle]
        pub extern "C" fn $name(
            blob: *const MeshBytes,
            wrapping_key: *const MeshSecretHandle,
            context: *const MeshBytes,
        ) -> *mut MeshResult {
            match unseal_for_current_actor(blob, wrapping_key, context, $kind) {
                Ok(handle) => ok_result(handle),
                Err(error) => error_result(error),
            }
        }
    };
}

storage_seal_abi!(mesh_secret_seal_for_storage, ResourceKind::SecretBytes);
storage_seal_abi!(mesh_secret_map_seal_for_storage, ResourceKind::SecretMap);
storage_seal_abi!(
    mesh_signing_private_key_seal_for_storage,
    ResourceKind::SigningPrivateKey
);
storage_seal_abi!(
    mesh_x25519_private_key_seal_for_storage,
    ResourceKind::X25519PrivateKey
);
storage_seal_abi!(
    mesh_mlkem_private_key_seal_for_storage,
    ResourceKind::MlKemPrivateKey
);

storage_unseal_abi!(mesh_secret_unseal_from_storage, ResourceKind::SecretBytes);
storage_unseal_abi!(mesh_secret_map_unseal_from_storage, ResourceKind::SecretMap);
storage_unseal_abi!(
    mesh_signing_private_key_unseal_from_storage,
    ResourceKind::SigningPrivateKey
);
storage_unseal_abi!(
    mesh_x25519_private_key_unseal_from_storage,
    ResourceKind::X25519PrivateKey
);
storage_unseal_abi!(
    mesh_mlkem_private_key_unseal_from_storage,
    ResourceKind::MlKemPrivateKey
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{ExitReason, Priority, Process, ProcessId};
    use crate::crypto::provider::SystemProvider;
    use crate::gc::mesh_rt_init;
    use crate::secret::{
        destroy_owned, insert_owned_resource, insert_test_storage_key_resource,
        owned_secret_count_for_test, ResourceKind,
    };
    use std::ptr;

    fn hex_encode(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            output.push(DIGITS[(byte >> 4) as usize] as char);
            output.push(DIGITS[(byte & 0x0f) as usize] as char);
        }
        output
    }

    fn material() -> Vec<u8> {
        let mut material = vec![0x31; 32];
        material.extend_from_slice(&[0x91, 0x92, 0x93, 0x94]);
        material
    }

    fn assert_tag(error: StorageFailure, tag: CryptoErrorTag) {
        assert_eq!(error.tag, tag, "unexpected storage failure: {error:?}");
    }

    fn context(purpose: u16) -> Vec<u8> {
        let mut context = Vec::with_capacity(123);
        context.push(1);
        context.extend(0u8..32);
        context.extend(0x20u8..0x30);
        if matches!(purpose, 1..=4 | 11..=13 | 16) {
            context.extend(0x30u8..0x50);
        } else {
            context.extend_from_slice(&[0; 32]);
        }
        context.extend(0x50u8..0x70);
        context.extend_from_slice(&purpose.to_be_bytes());
        context.extend_from_slice(&9u64.to_be_bytes());
        context
    }

    #[test]
    fn version_one_encoding_matches_the_independent_golden_vector() {
        let key: Vec<_> = (0u8..32).collect();
        let mut material = key;
        material.extend_from_slice(&[0xa1, 0xb2, 0xc3, 0xd4]);
        let plaintext: Vec<_> = (0x80u8..0xa0).collect();

        let blob = seal_value(
            &SystemProvider,
            &plaintext,
            &material,
            0x0102_0304_0506_0708,
            &context(1),
            ResourceKind::SecretBytes,
        )
        .expect("seal golden value");

        assert_eq!(
            hex_encode(&blob),
            "010001a1b2c3d401020304050607085da041521825930774a1b63a83fa73c93177531b71174ff2d5290828424101a1000000203492e188cb9a3d59e1eea6d26a6875cbfb112897190935867bdf26d13a95a83624c5d7af69209fc0168b6b0f2e36af30"
        );
    }

    #[test]
    fn every_registered_purpose_round_trips_to_its_exact_resource_kind() {
        let material = material();

        for purpose in (1..=13).chain([15, 16]) {
            let kind = match purpose {
                1..=5 | 11 | 16 => ResourceKind::SecretBytes,
                12 => ResourceKind::SecretMap,
                6..=7 => ResourceKind::SigningPrivateKey,
                8..=10 | 13 => ResourceKind::X25519PrivateKey,
                15 => ResourceKind::MlKemPrivateKey,
                _ => unreachable!(),
            };
            let plaintext_length = if kind == ResourceKind::MlKemPrivateKey {
                MLKEM_PRIVATE_SEED_BYTES
            } else {
                PLAINTEXT_BYTES
            };
            let plaintext = vec![purpose as u8; plaintext_length];
            let context = context(purpose);
            let blob = seal_value(
                &SystemProvider,
                &plaintext,
                &material,
                purpose as u64,
                &context,
                kind,
            )
            .expect("seal registered purpose");

            let opened = open_value(&SystemProvider, &blob, &material, &context, kind)
                .expect("open registered purpose");

            assert_eq!(&opened[..], &plaintext);
        }
    }

    #[test]
    fn every_bound_context_field_and_authenticated_blob_field_rejects_tampering() {
        let material = material();
        let plaintext = [0x73; PLAINTEXT_BYTES];
        let context = context(1);
        let blob = seal_value(
            &SystemProvider,
            &plaintext,
            &material,
            42,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect("seal tamper fixture");

        for offset in [0, 1, 33, 49, 81, 113, 122] {
            let mut changed_context = context.clone();
            changed_context[offset] ^= 1;
            let error = open_value(
                &SystemProvider,
                &blob,
                &material,
                &changed_context,
                ResourceKind::SecretBytes,
            )
            .expect_err("changed context field authenticated");
            assert_tag(error, CryptoErrorTag::AuthenticationFailed);
        }

        for offset in [
            BLOB_NONCE_OFFSET,
            BLOB_BINDING_OFFSET,
            BLOB_CIPHERTEXT_OFFSET,
            blob.len() - 1,
        ] {
            let mut changed_blob = blob.clone();
            changed_blob[offset] ^= 1;
            let error = open_value(
                &SystemProvider,
                &changed_blob,
                &material,
                &context,
                ResourceKind::SecretBytes,
            )
            .expect_err("changed authenticated blob field");
            assert_tag(error, CryptoErrorTag::AuthenticationFailed);
        }

        let mut changed_length = blob.clone();
        changed_length[BLOB_LENGTH_OFFSET..BLOB_CIPHERTEXT_OFFSET]
            .copy_from_slice(&31u32.to_be_bytes());
        changed_length.remove(BLOB_CIPHERTEXT_OFFSET + 31);
        let error = open_value(
            &SystemProvider,
            &changed_length,
            &material,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect_err("changed authenticated ciphertext length");
        assert_tag(error, CryptoErrorTag::AuthenticationFailed);

        let mut wrong_key = material.clone();
        wrong_key[0] ^= 1;
        let error = open_value(
            &SystemProvider,
            &blob,
            &wrong_key,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect_err("wrong storage key");
        assert_tag(error, CryptoErrorTag::AuthenticationFailed);
    }

    #[test]
    fn parser_enforces_fixed_validation_order_and_canonical_length() {
        let material = material();
        let context = context(1);
        let blob = seal_value(
            &SystemProvider,
            &[0x42; PLAINTEXT_BYTES],
            &material,
            3,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect("seal parser fixture");

        let mut below_minimum = vec![0; FIXED_OVERHEAD_BYTES - 1];
        below_minimum[0] = 9;
        let error = open_value(
            &SystemProvider,
            &below_minimum,
            &material,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect_err("minimum length precedes version");
        assert_tag(error, CryptoErrorTag::InvalidLength);

        let mut unsupported_version = blob.clone();
        unsupported_version[0] = 2;
        assert_tag(
            open_value(
                &SystemProvider,
                &unsupported_version,
                &material,
                &context,
                ResourceKind::SecretBytes,
            )
            .expect_err("unsupported version"),
            CryptoErrorTag::UnsupportedOperation,
        );
        let mut unsupported_algorithm = blob.clone();
        unsupported_algorithm[BLOB_ALGORITHM_OFFSET..BLOB_NONCE_OFFSET]
            .copy_from_slice(&2u16.to_be_bytes());
        assert_tag(
            open_value(
                &SystemProvider,
                &unsupported_algorithm,
                &material,
                &context,
                ResourceKind::SecretBytes,
            )
            .expect_err("unsupported algorithm"),
            CryptoErrorTag::UnsupportedOperation,
        );

        for malformed in [blob[..blob.len() - 1].to_vec(), {
            let mut trailing = blob.clone();
            trailing.push(0);
            trailing
        }] {
            assert_tag(
                open_value(
                    &SystemProvider,
                    &malformed,
                    &material,
                    &context,
                    ResourceKind::SecretBytes,
                )
                .expect_err("noncanonical total length"),
                CryptoErrorTag::InvalidLength,
            );
        }

        let mut oversized = vec![0; MAX_BLOB_BYTES + 1];
        oversized[0] = FORMAT_VERSION;
        oversized[BLOB_ALGORITHM_OFFSET..BLOB_NONCE_OFFSET]
            .copy_from_slice(&ALGORITHM_CHACHA20_POLY1305.to_be_bytes());
        oversized[BLOB_LENGTH_OFFSET..BLOB_CIPHERTEXT_OFFSET]
            .copy_from_slice(&(MAX_PLAINTEXT_BYTES as u32 + 1).to_be_bytes());
        let error = open_value(
            &SystemProvider,
            &oversized,
            &material,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect_err("oversized ciphertext");
        assert_eq!(
            (error.tag, error.expected, error.actual),
            (
                CryptoErrorTag::InvalidLength,
                MAX_PLAINTEXT_BYTES as i64,
                MAX_PLAINTEXT_BYTES as i64 + 1,
            )
        );
    }

    #[test]
    fn authentic_blob_cannot_cross_a_typed_purpose_entrypoint() {
        let material = material();
        let signing_context = context(6);
        let blob = seal_value(
            &SystemProvider,
            &[0x61; PLAINTEXT_BYTES],
            &material,
            7,
            &signing_context,
            ResourceKind::SigningPrivateKey,
        )
        .expect("seal signing fixture");

        let error = open_value(
            &SystemProvider,
            &blob,
            &material,
            &signing_context,
            ResourceKind::SecretBytes,
        )
        .expect_err("signing purpose through SecretBytes entrypoint");

        assert_tag(error, CryptoErrorTag::UnsupportedOperation);
    }

    #[test]
    fn forced_exit_after_reservation_burns_the_attempt_and_returns_no_blob() {
        mesh_rt_init();
        let owner = ProcessId(80_001);
        let mut process = Process::new(owner, Priority::Normal);
        let secret = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x51; PLAINTEXT_BYTES].into_boxed_slice()),
        )
        .expect("insert secret");
        let storage_key = insert_test_storage_key_resource(
            &mut process,
            Zeroizing::new(material().into_boxed_slice()),
            19,
        )
        .expect("insert storage key");

        let error = seal_for_process_with_hook(
            &mut process,
            &SystemProvider,
            secret,
            storage_key,
            &context(1),
            ResourceKind::SecretBytes,
            |process| {
                assert!(process.mark_exited(ExitReason::Killed));
            },
        )
        .expect_err("exited actor completed storage seal");

        assert_eq!(
            (error.tag, owned_secret_count_for_test(owner)),
            (CryptoErrorTag::SecretDestroyed, 0)
        );
    }

    struct FailingSealProvider;

    impl CryptoProvider for FailingSealProvider {
        fn fill_random(&self, _output: &mut [u8]) -> Result<(), ProviderError> {
            Err(ProviderError::EntropyUnavailable)
        }

        fn chacha20poly1305_seal(
            &self,
            _key: &[u8; 32],
            _nonce: &[u8; 12],
            _associated_data: &[u8],
            _plaintext: &[u8],
        ) -> Result<Vec<u8>, ProviderError> {
            Err(ProviderError::InvalidLength)
        }
    }

    #[test]
    fn provider_failure_still_burns_the_reserved_nonce_counter() {
        mesh_rt_init();
        let owner = ProcessId(80_002);
        let mut process = Process::new(owner, Priority::Normal);
        let secret = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x52; PLAINTEXT_BYTES].into_boxed_slice()),
        )
        .expect("insert secret");
        let storage_key = insert_test_storage_key_resource(
            &mut process,
            Zeroizing::new(material().into_boxed_slice()),
            5,
        )
        .expect("insert storage key");

        let failure = seal_for_process_with_hook(
            &mut process,
            &FailingSealProvider,
            secret,
            storage_key,
            &context(1),
            ResourceKind::SecretBytes,
            |_| {},
        )
        .expect_err("injected provider failure");
        let next_blob = seal_for_process_with_hook(
            &mut process,
            &SystemProvider,
            secret,
            storage_key,
            &context(1),
            ResourceKind::SecretBytes,
            |_| {},
        )
        .expect("next seal");

        assert_eq!(failure.tag, CryptoErrorTag::InvalidLength);
        assert_eq!(
            &next_blob[BLOB_NONCE_OFFSET + 4..BLOB_BINDING_OFFSET],
            &6u64.to_be_bytes()
        );
        destroy_owned(owner);
    }

    #[test]
    fn failed_authentication_registers_no_plaintext_resource() {
        mesh_rt_init();
        let owner = ProcessId(80_003);
        let mut process = Process::new(owner, Priority::Normal);
        let storage_key = insert_test_storage_key_resource(
            &mut process,
            Zeroizing::new(material().into_boxed_slice()),
            0,
        )
        .expect("insert storage key");
        let context = context(1);
        let mut blob = seal_value(
            &SystemProvider,
            &[0x53; PLAINTEXT_BYTES],
            &material(),
            0,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect("seal auth fixture");
        *blob.last_mut().expect("tag byte") ^= 1;

        let error = open_for_process(
            &mut process,
            &SystemProvider,
            &blob,
            storage_key,
            &context,
            ResourceKind::SecretBytes,
        )
        .expect_err("tampered blob opened");

        assert_eq!(
            (error.tag, owned_secret_count_for_test(owner)),
            (CryptoErrorTag::AuthenticationFailed, 1)
        );
        destroy_owned(owner);
    }

    #[test]
    fn process_layer_rejects_foreign_and_wrong_kind_handles() {
        mesh_rt_init();
        let owner = ProcessId(80_004);
        let other = ProcessId(80_005);
        let mut owner_process = Process::new(owner, Priority::Normal);
        let other_process = Process::new(other, Priority::Normal);
        let signing_key = insert_owned_resource(
            &mut owner_process,
            ResourceKind::SigningPrivateKey,
            Zeroizing::new(vec![0x54; PLAINTEXT_BYTES].into_boxed_slice()),
        )
        .expect("insert signing key");
        let storage_key = insert_test_storage_key_resource(
            &mut owner_process,
            Zeroizing::new(material().into_boxed_slice()),
            0,
        )
        .expect("insert storage key");

        let wrong_kind = prepare_seal(
            &owner_process,
            signing_key,
            storage_key,
            &context(1),
            ResourceKind::SecretBytes,
        )
        .err()
        .expect("wrong private-resource kind sealed");
        let foreign = prepare_seal(
            &other_process,
            signing_key,
            storage_key,
            &context(6),
            ResourceKind::SigningPrivateKey,
        )
        .err()
        .expect("foreign private resources sealed");

        assert_eq!(
            (wrong_kind.tag, foreign.tag),
            (CryptoErrorTag::InvalidKey, CryptoErrorTag::SecretDestroyed)
        );
        destroy_owned(owner);
    }

    #[test]
    fn public_abi_signatures_keep_typed_seal_and_unseal_separate() {
        unsafe extern "C" fn reserve_zero(_context: *mut c_void, counter_out: *mut u64) -> i32 {
            if counter_out.is_null() {
                return 1;
            }
            unsafe { counter_out.write(0) };
            0
        }

        let _: extern "C" fn() -> *mut MeshResult = mesh_storage_key_ephemeral;
        let _: extern "C" fn(
            *const u8,
            u64,
            *const u8,
            u64,
            Option<MeshStorageCounterReserve>,
            *mut c_void,
        ) -> *mut MeshResult = mesh_storage_key_provision;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshSecretHandle,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_secret_seal_for_storage;
        let _: extern "C" fn(
            *const MeshBytes,
            *const MeshSecretHandle,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_secret_unseal_from_storage;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshSecretHandle,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_secret_map_seal_for_storage;
        let _: extern "C" fn(
            *const MeshBytes,
            *const MeshSecretHandle,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_secret_map_unseal_from_storage;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshSecretHandle,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_mlkem_private_key_seal_for_storage;
        let _: extern "C" fn(
            *const MeshBytes,
            *const MeshSecretHandle,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_mlkem_private_key_unseal_from_storage;

        let key = [0x11; 32];
        let prefix = [0x22; 4];
        let result = mesh_storage_key_provision(
            key.as_ptr(),
            key.len() as u64,
            prefix.as_ptr(),
            prefix.len() as u64,
            None,
            ptr::dangling_mut(),
        );
        unsafe {
            assert_eq!((*result).tag, 1);
            assert_eq!(*(*result).value, CryptoErrorTag::InternalFailure as u8);
        }

        let null_context = mesh_storage_key_provision(
            key.as_ptr(),
            key.len() as u64,
            prefix.as_ptr(),
            prefix.len() as u64,
            Some(reserve_zero),
            ptr::null_mut(),
        );
        unsafe {
            assert_eq!((*null_context).tag, 1);
            assert_eq!(
                *(*null_context).value,
                CryptoErrorTag::InternalFailure as u8
            );
        }
    }
}
