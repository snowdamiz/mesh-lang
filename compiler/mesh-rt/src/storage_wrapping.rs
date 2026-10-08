//! Versioned storage wrapping for actor-owned private resources.

use std::ffi::c_void;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::actor::Process;
use crate::bytes::MeshBytes;
use crate::crypto::blind_rsa;
use crate::crypto::provider::{CryptoProvider, SystemProvider};
use crate::crypto::{
    bytes_value, crypto_result, failure, provider_failure, required_bytes, resource_failure,
    CryptoFailure,
};
use crate::io::MeshResult;
use crate::library::{secure_store_delete_raw, secure_store_get_raw, secure_store_put_raw};
use crate::secret::{
    commit_storage_counter, consume_owned_resource, insert_derived_storage_key_resource,
    insert_ephemeral_storage_key_resource, insert_owned_resource, insert_storage_key_resource,
    prepare_owned_resource, prepare_storage_key_resource, validate_prepared_owned_resource,
    validate_prepared_storage_key_resource, CryptoErrorTag, MeshSecretHandle,
    MeshStorageCounterReserve, PreparedOwnedResource, PreparedStorageKey, ResourceKind,
    StorageKeyError,
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
const PLATFORM_RECORD_ID: &[u8] = b"mesh/storage-key/v2";
const PLATFORM_RECORD_BYTES: usize = STORAGE_KEY_MATERIAL_BYTES + 8;
const PLATFORM_KEY_ID: &[u8] = b"mesh/storage-key/v1";
const PLATFORM_COUNTER_ID: &[u8] = b"mesh/storage-counter/v1";
const HOST_NOT_FOUND: i32 = 2;
/// A key derived from a secret: `HKDF-SHA-256(secret, salt, context)`.
const DERIVED_KEY_SALT: &[u8] = b"mesh/storage-key/derived/v1";
const DERIVED_KEY_CONTEXT_BYTES: usize = 256;
const MIN_DERIVED_SECRET_BYTES: usize = 16;

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

fn invalid_length(expected: usize, actual: usize) -> CryptoFailure {
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
    GroupTreeKemKey,
    BlindRsaIssuerKey,
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
    fn from_id(id: u16) -> Result<Self, CryptoFailure> {
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
            17 => Ok(Self::GroupTreeKemKey),
            18 => Ok(Self::BlindRsaIssuerKey),
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
            Self::DeviceDhKey
            | Self::SignedPrekey
            | Self::OneTimePrekey
            | Self::RatchetDhKey
            | Self::GroupTreeKemKey => ResourceKind::X25519PrivateKey.into(),
            Self::LocalData => StorageValueKind::Bytes,
            Self::PostQuantumPrekey => ResourceKind::MlKemPrivateKey.into(),
            Self::BlindRsaIssuerKey => ResourceKind::BlindRsaSecretKey.into(),
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
                | Self::BlindRsaIssuerKey
        )
    }
}

fn validate_plaintext_length(
    expected_kind: StorageValueKind,
    length: usize,
) -> Result<(), CryptoFailure> {
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
        // A 2,048-bit key's canonical PKCS#8 DER.
        StorageValueKind::Resource(ResourceKind::BlindRsaSecretKey) => {
            if length < blind_rsa::MIN_SECRET_KEY_BYTES {
                return Err(invalid_length(blind_rsa::MIN_SECRET_KEY_BYTES, length));
            }
            if length > blind_rsa::MAX_SECRET_KEY_BYTES {
                return Err(invalid_length(blind_rsa::MAX_SECRET_KEY_BYTES, length));
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
) -> Result<SecretPurpose, CryptoFailure> {
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

fn seal_value<K: Into<StorageValueKind> + Copy>(
    provider: &impl CryptoProvider,
    plaintext: &[u8],
    storage_key_material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    counter: u64,
    context: &[u8],
    expected_kind: K,
) -> Result<Vec<u8>, CryptoFailure> {
    let expected_kind = expected_kind.into();
    validate_context(context, expected_kind)?;
    validate_plaintext_length(expected_kind, plaintext.len())?;

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
    storage_key_material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    context: &[u8],
    expected_kind: K,
) -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
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
    let expected_total = FIXED_OVERHEAD_BYTES + ciphertext_length;
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

/// The material of a storage key resource, which is always its 32-byte key
/// and 4-byte nonce prefix: the table refuses a storage key of any other
/// length.
fn storage_key_material(bytes: &[u8]) -> &[u8; STORAGE_KEY_MATERIAL_BYTES] {
    bytes.try_into().expect("a 36-byte storage key")
}

struct SealPreparation {
    secret: PreparedOwnedResource,
    storage_key: PreparedStorageKey,
}

fn storage_key_failure(error: StorageKeyError) -> CryptoFailure {
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
) -> Result<SealPreparation, CryptoFailure> {
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
) -> Result<(), CryptoFailure> {
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
) -> Result<(), CryptoFailure> {
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
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    let storage_key = prepare_owned_resource(process, wrapping_key, ResourceKind::StorageKey)
        .map_err(resource_failure)?;
    let plaintext = open_value(
        provider,
        blob,
        storage_key_material(&storage_key.bytes),
        context,
        expected_kind,
    )?;
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
) -> Result<Vec<u8>, CryptoFailure> {
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
        storage_key_material(&preparation.storage_key.material),
        counter,
        context,
        expected_kind,
    )?;
    revalidate_seal_inputs(process, &preparation, expected_kind)?;
    Ok(blob)
}

fn current_process() -> Result<Arc<Mutex<Process>>, CryptoFailure> {
    crate::actor::current_process().ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))
}

/// A copy of `value`, at most `maximum` bytes of it: no host callback can
/// see the actor heap it came from.
unsafe fn copy_mesh_bytes(
    value: *const MeshBytes,
    maximum: usize,
) -> Result<Vec<u8>, CryptoFailure> {
    required_bytes(value, maximum).map(<[u8]>::to_vec)
}

unsafe fn copy_native_exact(
    value: *const u8,
    length: u64,
    expected: usize,
) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
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

fn seal_for_current_actor(
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
    expected_kind: ResourceKind,
) -> Result<Vec<u8>, CryptoFailure> {
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

    // prepare_seal checked the context and the plaintext: sealing them
    // cannot fail.
    let blob = seal_value(
        &SystemProvider,
        &preparation.secret.bytes,
        storage_key_material(&preparation.storage_key.material),
        counter,
        &context,
        expected_kind,
    )
    .expect("a context and plaintext checked before the reservation");
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
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
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
        storage_key_material(&storage_key.bytes),
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
    crypto_result(result)
}

/// Derive a storage key from a secret someone holds, such as the key a
/// recovery code yields, so that what it seals can be opened on another device
/// or after a reinstall from that secret alone. The key is
/// `HKDF-SHA-256(secret, "mesh/storage-key/derived/v1", context)`; `context`
/// (1 to 256 bytes) separates the uses of one secret. Every derivation of the
/// same secret and context has the same key, and none has a durable counter,
/// so each draws a random 4-byte nonce prefix and a random starting counter
/// below 2^63: two derivations share a nonce with probability about 2^-95 per
/// seal. The secret (at least 16 bytes) is consumed on every path.
#[no_mangle]
pub extern "C" fn mesh_storage_key_from_secret(
    material: *mut MeshSecretHandle,
    context: *const MeshBytes,
) -> *mut MeshResult {
    let result = (|| {
        let process = current_process()?;
        let secret = {
            let process = process.lock();
            consume_owned_resource(&process, material, ResourceKind::SecretBytes)
                .map_err(resource_failure)?
        };
        let context = unsafe { copy_mesh_bytes(context, DERIVED_KEY_CONTEXT_BYTES) }?;
        if context.is_empty() {
            return Err(invalid_length(DERIVED_KEY_CONTEXT_BYTES, 0));
        }
        if secret.len() < MIN_DERIVED_SECRET_BYTES {
            return Err(invalid_length(MIN_DERIVED_SECRET_BYTES, secret.len()));
        }
        let mut material = Zeroizing::new(vec![0u8; STORAGE_KEY_MATERIAL_BYTES]);
        SystemProvider
            .hkdf_sha256(&secret, DERIVED_KEY_SALT, &context, &mut material[..32])
            .map_err(provider_failure)?;
        SystemProvider
            .fill_random(&mut material[32..])
            .map_err(provider_failure)?;
        let mut start = [0u8; 8];
        SystemProvider
            .fill_random(&mut start)
            .map_err(provider_failure)?;
        let next_counter = u64::from_be_bytes(start) >> 1;
        let material = Zeroizing::new(std::mem::take(&mut *material).into_boxed_slice());
        let handle = {
            let mut process = process.lock();
            insert_derived_storage_key_resource(&mut process, material, next_counter)
                .map_err(resource_failure)?
        };
        Ok(handle)
    })();
    crypto_result(result)
}

fn platform_failure(status: i32) -> CryptoFailure {
    failure(CryptoErrorTag::InternalFailure, 0, status as i64)
}

fn read_platform_record(
    key: &[u8],
    maximum: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>, CryptoFailure> {
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

fn write_platform_record(key: &[u8], value: &[u8]) -> Result<(), CryptoFailure> {
    let key_length =
        u32::try_from(key.len()).map_err(|_| invalid_length(u32::MAX as usize, key.len()))?;
    let mut request = Zeroizing::new(Vec::with_capacity(4 + key.len() + value.len()));
    request.extend_from_slice(&key_length.to_be_bytes());
    request.extend_from_slice(key);
    request.extend_from_slice(value);
    secure_store_put_raw(&request).map_err(platform_failure)
}

fn retire_legacy_records(record: &[u8]) -> Result<(), CryptoFailure> {
    let material = read_platform_record(PLATFORM_KEY_ID, STORAGE_KEY_MATERIAL_BYTES)?;
    let counter = read_platform_record(PLATFORM_COUNTER_ID, 8)?;
    if material
        .as_ref()
        .is_some_and(|value| value.as_slice() != &record[..STORAGE_KEY_MATERIAL_BYTES])
    {
        return Err(platform_failure(HOST_NOT_FOUND));
    }
    if let Some(counter) = &counter {
        if counter.len() != 8 || counter.as_slice() > &record[STORAGE_KEY_MATERIAL_BYTES..] {
            return Err(platform_failure(HOST_NOT_FOUND));
        }
    }
    // The v2 record is durable first. Remove the old key before its counter so
    // an older binary cannot restart that key's nonce counter after rollback.
    for (key, present) in [
        (PLATFORM_KEY_ID, material.is_some()),
        (PLATFORM_COUNTER_ID, counter.is_some()),
    ] {
        if present {
            match secure_store_delete_raw(key) {
                Ok(()) | Err(HOST_NOT_FOUND) => {}
                Err(status) => return Err(platform_failure(status)),
            }
        }
    }
    Ok(())
}

fn load_platform_material() -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
    let record = match read_platform_record(PLATFORM_RECORD_ID, PLATFORM_RECORD_BYTES)? {
        Some(record) => {
            if record.len() != PLATFORM_RECORD_BYTES {
                return Err(invalid_length(PLATFORM_RECORD_BYTES, record.len()));
            }
            record
        }
        None => {
            let material = read_platform_record(PLATFORM_KEY_ID, STORAGE_KEY_MATERIAL_BYTES)?;
            let counter = read_platform_record(PLATFORM_COUNTER_ID, 8)?;
            let mut record = Zeroizing::new(Vec::with_capacity(PLATFORM_RECORD_BYTES));
            match (material, counter) {
                (None, None) => {
                    record.resize(STORAGE_KEY_MATERIAL_BYTES, 0);
                    SystemProvider
                        .fill_random(&mut record)
                        .map_err(provider_failure)?;
                    record.extend_from_slice(&0u64.to_be_bytes());
                }
                (Some(material), Some(counter))
                    if material.len() == STORAGE_KEY_MATERIAL_BYTES && counter.len() == 8 =>
                {
                    record.extend_from_slice(&material);
                    record.extend_from_slice(&counter);
                }
                // Preserve incomplete legacy records; never reset a used counter.
                _ => return Err(platform_failure(HOST_NOT_FOUND)),
            }
            write_platform_record(PLATFORM_RECORD_ID, &record)?;
            record
        }
    };
    retire_legacy_records(&record)?;
    Ok(Zeroizing::new(
        record[..STORAGE_KEY_MATERIAL_BYTES]
            .to_vec()
            .into_boxed_slice(),
    ))
}

// ponytail: one process-wide lock; use a host atomic-increment callback if app
// extensions ever share this storage key concurrently.
unsafe extern "C" fn reserve_platform_counter(_context: *mut c_void, counter_out: *mut u64) -> i32 {
    let _guard = PLATFORM_STORAGE_LOCK.lock();
    let result = (|| {
        let mut record = read_platform_record(PLATFORM_RECORD_ID, PLATFORM_RECORD_BYTES)?
            .ok_or_else(|| platform_failure(HOST_NOT_FOUND))?;
        if record.len() != PLATFORM_RECORD_BYTES {
            return Err(invalid_length(PLATFORM_RECORD_BYTES, record.len()));
        }
        let current = u64::from_be_bytes(
            record[STORAGE_KEY_MATERIAL_BYTES..]
                .try_into()
                .expect("checked record"),
        );
        let next = current
            .checked_add(1)
            .ok_or_else(|| failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0))?;
        record[STORAGE_KEY_MATERIAL_BYTES..].copy_from_slice(&next.to_be_bytes());
        write_platform_record(PLATFORM_RECORD_ID, &record)?;
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
    crypto_result(result)
}

fn seal_bytes_for_current_actor(
    value: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> Result<Vec<u8>, CryptoFailure> {
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
    // The context and the value's length were checked above: sealing them
    // cannot fail.
    let blob = seal_value(
        &SystemProvider,
        &value,
        storage_key_material(&storage_key.material),
        counter,
        &context,
        StorageValueKind::Bytes,
    )
    .expect("a context and plaintext checked before the reservation");
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
) -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
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
        storage_key_material(&storage_key.bytes),
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
    crypto_result(
        seal_bytes_for_current_actor(value, wrapping_key, context).map(|blob| bytes_value(&blob)),
    )
}

#[no_mangle]
pub extern "C" fn mesh_storage_key_unseal_bytes(
    blob: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> *mut MeshResult {
    crypto_result(
        unseal_bytes_for_current_actor(blob, wrapping_key, context)
            .map(|value| bytes_value(&value)),
    )
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
    crypto_result(result)
}

macro_rules! storage_seal_abi {
    ($name:ident, $kind:expr) => {
        #[no_mangle]
        pub extern "C" fn $name(
            secret: *const MeshSecretHandle,
            wrapping_key: *const MeshSecretHandle,
            context: *const MeshBytes,
        ) -> *mut MeshResult {
            crypto_result(
                seal_for_current_actor(secret, wrapping_key, context, $kind)
                    .map(|blob| bytes_value(&blob)),
            )
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
            crypto_result(unseal_for_current_actor(blob, wrapping_key, context, $kind))
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

/// Seal a blind RSA issuer key (purpose 18). Servers only, as the key is.
#[no_mangle]
pub extern "C" fn mesh_blind_rsa_secret_key_seal_for_storage(
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> *mut MeshResult {
    crypto_result(
        blind_rsa::server_target()
            .and_then(|()| {
                seal_for_current_actor(
                    secret,
                    wrapping_key,
                    context,
                    ResourceKind::BlindRsaSecretKey,
                )
            })
            .map(|blob| bytes_value(&blob)),
    )
}

/// Restore a blind RSA issuer key sealed under purpose 18. Servers only.
#[no_mangle]
pub extern "C" fn mesh_blind_rsa_secret_key_unseal_from_storage(
    blob: *const MeshBytes,
    wrapping_key: *const MeshSecretHandle,
    context: *const MeshBytes,
) -> *mut MeshResult {
    crypto_result(blind_rsa::server_target().and_then(|()| {
        unseal_for_current_actor(blob, wrapping_key, context, ResourceKind::BlindRsaSecretKey)
    }))
}

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
    use crate::crypto::provider::{ProviderError, SystemProvider};
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

    fn material() -> [u8; STORAGE_KEY_MATERIAL_BYTES] {
        let mut material = [0x31; STORAGE_KEY_MATERIAL_BYTES];
        material[32..].copy_from_slice(&[0x91, 0x92, 0x93, 0x94]);
        material
    }

    fn assert_tag(error: CryptoFailure, tag: CryptoErrorTag) {
        assert_eq!(error.tag, tag, "unexpected storage failure: {error:?}");
    }

    fn context(purpose: u16) -> Vec<u8> {
        let mut context = Vec::with_capacity(123);
        context.push(1);
        context.extend(0u8..32);
        context.extend(0x20u8..0x30);
        if matches!(purpose, 1..=4 | 11..=13 | 16..=17) {
            context.extend(0x30u8..0x50);
        } else {
            context.extend_from_slice(&[0; 32]);
        }
        context.extend(0x50u8..0x70);
        context.extend_from_slice(&purpose.to_be_bytes());
        context.extend_from_slice(&9u64.to_be_bytes());
        context
    }

    /// A context is refused for each way it can be wrong, before any key is
    /// touched: its length, version, purpose, kind, snapshot and session.
    #[test]
    fn a_context_is_refused_for_what_is_wrong_with_it() {
        let bytes = ResourceKind::SecretBytes;
        assert!(validate_context(&context(1), bytes).is_ok());
        assert_tag(
            validate_context(&context(1)[..10], bytes).unwrap_err(),
            CryptoErrorTag::InvalidLength,
        );
        let mut version = context(1);
        version[CONTEXT_VERSION_OFFSET] = 2;
        assert_tag(
            validate_context(&version, bytes).unwrap_err(),
            CryptoErrorTag::UnsupportedOperation,
        );
        assert_tag(
            validate_context(&context(14), bytes).unwrap_err(),
            CryptoErrorTag::UnsupportedOperation,
        );
        assert_tag(
            validate_context(&context(12), bytes).unwrap_err(),
            CryptoErrorTag::UnsupportedOperation,
        );
        let mut snapshot = context(1);
        snapshot[CONTEXT_SNAPSHOT_OFFSET..CONTEXT_BYTES].fill(0);
        assert_tag(
            validate_context(&snapshot, bytes).unwrap_err(),
            CryptoErrorTag::InvalidLength,
        );
        let mut session = context(5);
        assert!(validate_context(&session, bytes).is_ok());
        session[CONTEXT_SESSION_OFFSET] = 1;
        assert_tag(
            validate_context(&session, bytes).unwrap_err(),
            CryptoErrorTag::InvalidLength,
        );
    }

    unsafe extern "C" fn reserve_nothing(_: *mut c_void, _: *mut u64) -> i32 {
        1
    }

    /// Provisioning a storage key takes exactly a 32-byte key, a 4-byte
    /// nonce prefix, a counter callback and its context.
    #[test]
    fn a_storage_key_is_provisioned_only_from_exact_material() {
        mesh_rt_init();
        let key = [7u8; 32];
        let prefix = [1u8; 4];
        let mut callback_context = 0u8;
        let context = &mut callback_context as *mut u8 as *mut c_void;
        let reserve = Some(reserve_nothing as MeshStorageCounterReserve);
        let tag_of = |result: *mut MeshResult| unsafe {
            let result = &*result;
            assert_eq!(result.tag, 1, "expected an error");
            // A crypto error starts with its tag byte.
            *result.value
        };
        let invalid = CryptoErrorTag::InvalidLength as u8;
        let internal = CryptoErrorTag::InternalFailure as u8;
        assert_eq!(
            tag_of(mesh_storage_key_provision(
                key.as_ptr(),
                31,
                prefix.as_ptr(),
                4,
                reserve,
                context
            )),
            invalid
        );
        assert_eq!(
            tag_of(mesh_storage_key_provision(
                ptr::null(),
                32,
                prefix.as_ptr(),
                4,
                reserve,
                context
            )),
            invalid
        );
        assert_eq!(
            tag_of(mesh_storage_key_provision(
                key.as_ptr(),
                32,
                prefix.as_ptr(),
                3,
                reserve,
                context
            )),
            invalid
        );
        assert_eq!(
            tag_of(mesh_storage_key_provision(
                key.as_ptr(),
                32,
                prefix.as_ptr(),
                4,
                None,
                context
            )),
            internal
        );
        assert_eq!(
            tag_of(mesh_storage_key_provision(
                key.as_ptr(),
                32,
                prefix.as_ptr(),
                4,
                reserve,
                ptr::null_mut()
            )),
            internal
        );
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
            material.as_slice().try_into().unwrap(),
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

        for purpose in (1..=13).chain([15, 16, 17, 18]) {
            let kind = match purpose {
                1..=5 | 11 | 16 => ResourceKind::SecretBytes,
                12 => ResourceKind::SecretMap,
                6..=7 => ResourceKind::SigningPrivateKey,
                8..=10 | 13 | 17 => ResourceKind::X25519PrivateKey,
                15 => ResourceKind::MlKemPrivateKey,
                18 => ResourceKind::BlindRsaSecretKey,
                _ => unreachable!(),
            };
            let plaintext_length = match kind {
                ResourceKind::MlKemPrivateKey => MLKEM_PRIVATE_SEED_BYTES,
                ResourceKind::BlindRsaSecretKey => 1_217,
                _ => PLAINTEXT_BYTES,
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

        let mut wrong_key = material;
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
            Zeroizing::new(material().to_vec().into_boxed_slice()),
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
            Zeroizing::new(material().to_vec().into_boxed_slice()),
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

        assert_eq!(failure.tag, CryptoErrorTag::InternalFailure);
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
            Zeroizing::new(material().to_vec().into_boxed_slice()),
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
            Zeroizing::new(material().to_vec().into_boxed_slice()),
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

    /// A purpose the format does not define, and a value of the wrong size
    /// for its kind, are refused; each storage-key failure is reported as the
    /// CryptoError the docs give it.
    #[test]
    fn purposes_lengths_and_counter_failures_are_refused_as_documented() {
        let mut unknown = context(1);
        unknown[CONTEXT_PURPOSE_OFFSET..CONTEXT_SNAPSHOT_OFFSET]
            .copy_from_slice(&99u16.to_be_bytes());
        assert_tag(
            validate_context(&unknown, ResourceKind::SecretBytes).unwrap_err(),
            CryptoErrorTag::UnsupportedOperation,
        );
        for (kind, length) in [
            (StorageValueKind::Resource(ResourceKind::SecretMap), 0),
            (
                StorageValueKind::Resource(ResourceKind::SecretMap),
                MAX_PLAINTEXT_BYTES + 1,
            ),
            (StorageValueKind::Bytes, MAX_PLAINTEXT_BYTES + 1),
            (
                StorageValueKind::Resource(ResourceKind::MlKemPrivateKey),
                32,
            ),
            (StorageValueKind::Resource(ResourceKind::SecretBytes), 64),
            (
                StorageValueKind::Resource(ResourceKind::BlindRsaSecretKey),
                blind_rsa::MIN_SECRET_KEY_BYTES - 1,
            ),
            (
                StorageValueKind::Resource(ResourceKind::BlindRsaSecretKey),
                blind_rsa::MAX_SECRET_KEY_BYTES + 1,
            ),
            (
                StorageValueKind::Resource(ResourceKind::BlindRsaSecretKey),
                PLAINTEXT_BYTES,
            ),
        ] {
            assert_tag(
                validate_plaintext_length(kind, length).unwrap_err(),
                CryptoErrorTag::InvalidLength,
            );
        }
        assert_eq!(
            [
                StorageKeyError::Resource(crate::secret::ResourceError::StaleHandle),
                StorageKeyError::ReservationFailed,
                StorageKeyError::CounterNotMonotonic,
                StorageKeyError::CounterExhausted,
            ]
            .map(|error| storage_key_failure(error).tag),
            [
                CryptoErrorTag::SecretDestroyed,
                CryptoErrorTag::InternalFailure,
                CryptoErrorTag::InternalFailure,
                CryptoErrorTag::ResourceLimitExceeded,
            ]
        );
    }

    fn created<T>(result: *mut MeshResult) -> *mut T {
        let result = unsafe { &*result };
        assert_eq!(result.tag, 0, "expected Ok");
        result.value.cast()
    }

    fn refused(result: *mut MeshResult) -> CryptoErrorTag {
        let result = unsafe { &*result };
        assert_eq!(result.tag, 1, "expected Err");
        let tag = unsafe { *result.value };
        [
            CryptoErrorTag::InvalidLength,
            CryptoErrorTag::InvalidKey,
            CryptoErrorTag::InvalidPublicKey,
            CryptoErrorTag::InvalidSignature,
            CryptoErrorTag::AuthenticationFailed,
            CryptoErrorTag::EntropyUnavailable,
            CryptoErrorTag::SecretDestroyed,
            CryptoErrorTag::ResourceLimitExceeded,
            CryptoErrorTag::UnsupportedOperation,
            CryptoErrorTag::InternalFailure,
            CryptoErrorTag::UnsupportedTarget,
        ][tag as usize]
    }

    fn mesh_bytes(data: &[u8]) -> *mut MeshBytes {
        crate::bytes::mesh_bytes_new(data.as_ptr(), data.len() as u64)
    }

    /// A resource of the calling actor holding `data`.
    fn resource(kind: ResourceKind, data: &[u8]) -> *mut MeshSecretHandle {
        let process = crate::actor::current_process().expect("an actor");
        let mut process = process.lock();
        let material = Zeroizing::new(data.to_vec().into_boxed_slice());
        insert_owned_resource(&mut process, kind, material).expect("resource")
    }

    fn revealed(handle: *const MeshSecretHandle, kind: ResourceKind) -> Vec<u8> {
        let process = crate::actor::current_process().expect("an actor");
        let process = process.lock();
        let prepared = prepare_owned_resource(&process, handle, kind).expect("resource");
        prepared.bytes.to_vec()
    }

    static PROVISIONED_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    unsafe extern "C" fn reserve_next(_context: *mut c_void, counter_out: *mut u64) -> i32 {
        let counter = PROVISIONED_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        counter_out.write(counter);
        0
    }

    /// A blind RSA issuer key seals under purpose 18 only and comes back as
    /// the same key: its public key and signatures are unchanged.
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn blind_rsa_issuer_keys_seal_and_unseal_as_the_same_key() {
        use crate::crypto::{
            mesh_crypto_blind_rsa_generate, mesh_crypto_blind_rsa_public, MeshBlindRsaPublicKey,
        };
        mesh_rt_init();
        crate::secret::as_test_actor(|_| unsafe {
            let wrapping_key = created(mesh_storage_key_ephemeral());
            let issuer = created(mesh_crypto_blind_rsa_generate());
            let issuer_context = mesh_bytes(&context(18));
            let sealed: *mut MeshBytes = created(mesh_blind_rsa_secret_key_seal_for_storage(
                issuer,
                wrapping_key,
                issuer_context,
            ));
            let length = (*sealed).len as usize;
            assert!((FIXED_OVERHEAD_BYTES + blind_rsa::MIN_SECRET_KEY_BYTES
                ..=FIXED_OVERHEAD_BYTES + blind_rsa::MAX_SECRET_KEY_BYTES)
                .contains(&length));
            let restored = created(mesh_blind_rsa_secret_key_unseal_from_storage(
                sealed,
                wrapping_key,
                issuer_context,
            ));
            assert_eq!(
                revealed(restored, ResourceKind::BlindRsaSecretKey),
                revealed(issuer, ResourceKind::BlindRsaSecretKey)
            );
            let original: *mut MeshBlindRsaPublicKey =
                created(mesh_crypto_blind_rsa_public(issuer));
            let reopened: *mut MeshBlindRsaPublicKey =
                created(mesh_crypto_blind_rsa_public(restored));
            assert_eq!(
                (*(*original).bytes).as_slice(),
                (*(*reopened).bytes).as_slice()
            );
            for purpose in [7, 15] {
                assert_eq!(
                    refused(mesh_blind_rsa_secret_key_seal_for_storage(
                        issuer,
                        wrapping_key,
                        mesh_bytes(&context(purpose)),
                    )),
                    CryptoErrorTag::UnsupportedOperation
                );
            }
            let mlkem_blob = mesh_mlkem_private_key_seal_for_storage(
                resource(ResourceKind::MlKemPrivateKey, &[4; 64]),
                wrapping_key,
                mesh_bytes(&context(15)),
            );
            assert_eq!(
                refused(mesh_blind_rsa_secret_key_unseal_from_storage(
                    created(mlkem_blob),
                    wrapping_key,
                    issuer_context,
                )),
                CryptoErrorTag::AuthenticationFailed
            );
        });
    }

    /// A storage key derived from a secret someone holds opens what any other
    /// derivation of the same secret and context sealed, and nothing sealed
    /// under another secret or context. Its key is HKDF-SHA-256 of the secret
    /// with the fixed salt and the context; each derivation draws its own
    /// nonce prefix and starting counter, so two derivations never share a
    /// nonce. The secret is consumed on every path, refusals included.
    #[test]
    fn keys_derived_from_one_secret_open_each_others_blobs() {
        mesh_rt_init();
        assert_eq!(
            refused(mesh_storage_key_from_secret(
                ptr::null_mut(),
                mesh_bytes(b"x")
            )),
            CryptoErrorTag::InternalFailure
        );
        crate::secret::as_test_actor(|owner| {
            let derive = |secret: &[u8], label: &[u8]| -> *mut MeshSecretHandle {
                created(mesh_storage_key_from_secret(
                    resource(ResourceKind::SecretBytes, secret),
                    mesh_bytes(label),
                ))
            };
            let first = derive(&[7; 32], b"backup-keys/v1");
            let again = derive(&[7; 32], b"backup-keys/v1");
            let other_secret = derive(&[8; 32], b"backup-keys/v1");
            let other_context = derive(&[7; 32], b"backup-keys/v2");

            let (first_material, again_material) = {
                let process = crate::actor::current_process().expect("an actor");
                let process = process.lock();
                let material = |key| {
                    prepare_storage_key_resource(&process, key)
                        .ok()
                        .expect("derived key")
                        .material
                        .to_vec()
                };
                (material(first), material(again))
            };
            let mut expected = [0u8; 32];
            SystemProvider
                .hkdf_sha256(&[7; 32], DERIVED_KEY_SALT, b"backup-keys/v1", &mut expected)
                .expect("hkdf");
            assert_eq!(first_material[..32], expected);
            assert_eq!(again_material[..32], expected);

            let signing = resource(ResourceKind::SigningPrivateKey, &[5; 32]);
            let signing_context = mesh_bytes(&context(6));
            let sealed: *mut MeshBytes = created(mesh_signing_private_key_seal_for_storage(
                signing,
                first,
                signing_context,
            ));
            let opened = created(mesh_signing_private_key_unseal_from_storage(
                sealed,
                again,
                signing_context,
            ));
            assert_eq!(revealed(opened, ResourceKind::SigningPrivateKey), [5; 32]);
            for key in [other_secret, other_context] {
                assert_eq!(
                    refused(mesh_signing_private_key_unseal_from_storage(
                        sealed,
                        key,
                        signing_context,
                    )),
                    CryptoErrorTag::AuthenticationFailed
                );
            }
            let resealed: *mut MeshBytes = created(mesh_signing_private_key_seal_for_storage(
                signing,
                again,
                signing_context,
            ));
            let nonce = |blob: *mut MeshBytes| unsafe {
                (*blob).as_slice()[BLOB_NONCE_OFFSET..BLOB_BINDING_OFFSET].to_vec()
            };
            assert_ne!(nonce(sealed), nonce(resealed));

            let before = crate::secret::owned_secret_count_for_test(owner);
            for (secret, label) in [
                (&[7u8; 15][..], &b"backup-keys/v1"[..]),
                (&[7u8; 32][..], &b""[..]),
                (&[7u8; 32][..], &[b'x'; DERIVED_KEY_CONTEXT_BYTES + 1][..]),
            ] {
                let material = resource(ResourceKind::SecretBytes, secret);
                assert_eq!(
                    refused(mesh_storage_key_from_secret(material, mesh_bytes(label))),
                    CryptoErrorTag::InvalidLength
                );
                assert_eq!(
                    refused(mesh_storage_key_from_secret(material, mesh_bytes(b"x"))),
                    CryptoErrorTag::SecretDestroyed
                );
            }
            let wrong_kind = resource(ResourceKind::SigningPrivateKey, &[7; 32]);
            assert_eq!(
                refused(mesh_storage_key_from_secret(wrong_kind, mesh_bytes(b"x"))),
                CryptoErrorTag::InvalidKey
            );
            assert_eq!(
                crate::secret::owned_secret_count_for_test(owner),
                before + 1
            );
        });
    }

    /// The entry points seal and unseal for the calling actor with an
    /// ephemeral or a provisioned key, refuse a blob opened under another
    /// context, a destroyed key, and a context of the wrong length, and do
    /// nothing off an actor.
    #[test]
    fn entry_points_seal_and_unseal_for_the_calling_actor() {
        mesh_rt_init();
        let context_one = mesh_bytes(&context(1));
        let local = mesh_bytes(&context(14));
        let internal = CryptoErrorTag::InternalFailure;
        assert_eq!(refused(mesh_storage_key_ephemeral()), internal);
        assert_eq!(refused(mesh_storage_key_platform()), internal);
        let (key, prefix) = ([7u8; 32], [1u8; 4]);
        let provision = || {
            mesh_storage_key_provision(
                key.as_ptr(),
                32,
                prefix.as_ptr(),
                4,
                Some(reserve_next),
                ptr::dangling_mut(),
            )
        };
        assert_eq!(refused(provision()), internal);
        let null = ptr::null();
        assert_eq!(
            refused(mesh_secret_seal_for_storage(null, null, context_one)),
            internal
        );
        let blob = mesh_bytes(&[0; FIXED_OVERHEAD_BYTES]);
        assert_eq!(
            refused(mesh_secret_unseal_from_storage(blob, null, context_one)),
            internal
        );
        assert_eq!(
            refused(mesh_storage_key_seal_bytes(blob, null, local)),
            internal
        );
        assert_eq!(
            refused(mesh_storage_key_unseal_bytes(blob, null, local)),
            internal
        );

        crate::secret::as_test_actor(|_| {
            for key in [created(mesh_storage_key_ephemeral()), created(provision())] {
                let secret = resource(ResourceKind::SecretBytes, &[0x42; PLAINTEXT_BYTES]);
                let sealed: *mut MeshBytes =
                    created(mesh_secret_seal_for_storage(secret, key, context_one));
                let opened = created(mesh_secret_unseal_from_storage(sealed, key, context_one));
                assert_eq!(
                    revealed(opened, ResourceKind::SecretBytes),
                    [0x42; PLAINTEXT_BYTES]
                );
                let other = mesh_bytes(&context(2));
                assert_eq!(
                    refused(mesh_secret_unseal_from_storage(sealed, key, other)),
                    CryptoErrorTag::AuthenticationFailed
                );

                let value = mesh_bytes(b"local data");
                let sealed: *mut MeshBytes =
                    created(mesh_storage_key_seal_bytes(value, key, local));
                let opened: *mut MeshBytes =
                    created(mesh_storage_key_unseal_bytes(sealed, key, local));
                assert_eq!(unsafe { (*opened).as_slice() }, b"local data");
                assert_eq!(
                    refused(mesh_storage_key_unseal_bytes(sealed, key, context_one)),
                    CryptoErrorTag::AuthenticationFailed
                );
            }
            let key = created(mesh_storage_key_ephemeral());
            let map = resource(ResourceKind::SecretMap, &[0, 1, 0, 0]);
            let map_context = mesh_bytes(&context(12));
            let sealed = created(mesh_secret_map_seal_for_storage(map, key, map_context));
            let opened = created(mesh_secret_map_unseal_from_storage(
                sealed,
                key,
                map_context,
            ));
            assert_eq!(revealed(opened, ResourceKind::SecretMap), [0, 1, 0, 0]);

            let short = mesh_bytes(&context(1)[..10]);
            let secret = resource(ResourceKind::SecretBytes, &[1; PLAINTEXT_BYTES]);
            assert_eq!(
                refused(mesh_secret_seal_for_storage(secret, key, short)),
                CryptoErrorTag::InvalidLength
            );
            assert_eq!(
                refused(mesh_storage_key_seal_bytes(mesh_bytes(b"x"), key, short)),
                CryptoErrorTag::InvalidLength
            );
            crate::secret::mesh_resource_destroy(key);
            let destroyed = CryptoErrorTag::SecretDestroyed;
            assert_eq!(
                refused(mesh_secret_seal_for_storage(secret, key, context_one)),
                destroyed
            );
            assert_eq!(
                refused(mesh_storage_key_seal_bytes(blob, key, local)),
                destroyed
            );
            assert_eq!(
                refused(mesh_secret_unseal_from_storage(blob, key, context_one)),
                destroyed
            );
            assert_eq!(
                refused(mesh_storage_key_unseal_bytes(blob, key, local)),
                destroyed
            );
        });
    }
}
