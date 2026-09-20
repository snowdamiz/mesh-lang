//! Binary-safe cryptographic primitives and legacy encoding functions.
//!
//! Crypto V2 keeps private material in actor-owned zeroizing resources and
//! exposes raw hash bytes by default. The non-colliding legacy HMAC-SHA-512,
//! UUID, Base64, and Hex exports remain available for existing callers.

pub(crate) mod provider;

use std::ptr;

use base64::{engine::general_purpose, Engine as _};
use hmac::{Hmac, Mac};
use ml_kem::ml_kem_768::Ciphertext as MlKem768Ciphertext;
use ml_kem::{Decapsulate, DecapsulationKey768, EncapsulationKey768, Key, KeyExport, Seed, B32};
use rand::RngCore;
use sha2::Sha512;
use zeroize::Zeroizing;

#[cfg(feature = "fuzzing")]
use self::provider::FixedProvider;
use self::provider::{
    CryptoProvider, ProviderError, SystemProvider, MAX_ARGON2_ITERATIONS, MAX_ARGON2_MEMORY_KIB,
    MAX_ARGON2_OUTPUT_BYTES, MAX_ARGON2_PARALLELISM, MAX_ARGON2_SALT_BYTES,
    MIN_ARGON2_OUTPUT_BYTES, MIN_ARGON2_SALT_BYTES,
};
use crate::actor::Process;
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, MeshResult};
use crate::secret::{
    consume_and_retype_owned_resource, crypto_error, insert_owned_resource, mesh_resource_destroy,
    with_owned_resource, CryptoErrorTag, MeshSecretHandle, ResourceError, ResourceKind,
    RetypeError,
};
use crate::string::{mesh_string_new, MeshString};

type HmacSha512 = Hmac<Sha512>;

const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_RANDOM_BYTES: usize = 64 * 1024;
const MAX_HKDF_OUTPUT_BYTES: usize = 255 * 32;
const AEAD_NONCE_BYTES: usize = 12;
const AEAD_TAG_BYTES: usize = 16;
const MAX_AEAD_CIPHERTEXT_BYTES: usize = MAX_INPUT_BYTES + AEAD_TAG_BYTES;
const MLKEM_PRIVATE_SEED_BYTES: usize = 64;
const MLKEM_PUBLIC_KEY_BYTES: usize = 1184;
const MLKEM_CIPHERTEXT_BYTES: usize = 1088;
const MLKEM_SHARED_SECRET_BYTES: usize = 32;
const HPKE_ENCAPSULATED_KEY_BYTES: usize = 32;
const HPKE_MIN_SEALED_BYTES: usize = HPKE_ENCAPSULATED_KEY_BYTES + AEAD_TAG_BYTES;
const MAX_HPKE_INFO_BYTES: usize = MAX_INPUT_BYTES - 64;
const MAX_HPKE_SEALED_BYTES: usize = HPKE_ENCAPSULATED_KEY_BYTES + MAX_AEAD_CIPHERTEXT_BYTES;
const HPKE_KEM_SUITE_ID: &[u8] = b"KEM\x00\x20";
const HPKE_SUITE_ID: &[u8] = b"HPKE\x00\x20\x00\x01\x00\x03";
const HPKE_VERSION_LABEL: &[u8] = b"HPKE-v1";

#[repr(C)]
/// Runtime representation of an X25519 public-key wrapper.
pub struct MeshX25519PublicKey {
    pub bytes: *mut MeshBytes,
}

#[repr(C)]
/// Runtime representation of an X25519 key pair with an inline public wrapper.
pub struct MeshX25519KeyPair {
    pub private_key: *mut MeshSecretHandle,
    pub public_key: MeshX25519PublicKey,
}

#[repr(C)]
/// Runtime representation of an ML-KEM-768 encapsulation key.
pub struct MeshMlKemPublicKey {
    pub bytes: *mut MeshBytes,
}

#[repr(C)]
/// Runtime representation of an ML-KEM-768 ciphertext.
pub struct MeshMlKemCiphertext {
    pub bytes: *mut MeshBytes,
}

#[repr(C)]
/// Runtime representation of an ML-KEM-768 key pair.
pub struct MeshMlKemKeyPair {
    pub private_key: *mut MeshSecretHandle,
    pub public_key: MeshMlKemPublicKey,
}

#[repr(C)]
struct MeshTuple2Pointers {
    len: u64,
    first: *mut MeshBytes,
    second: *mut MeshSecretHandle,
}

#[cfg(test)]
struct GeneratedX25519KeyPair {
    private_key: *mut MeshSecretHandle,
    public_key: [u8; 32],
}

#[repr(C)]
/// Runtime representation of an Ed25519 public-key wrapper.
pub struct MeshSigningPublicKey {
    pub bytes: *mut MeshBytes,
}

#[repr(C)]
/// Runtime representation of an Ed25519 signature wrapper.
pub struct MeshSignature {
    pub bytes: *mut MeshBytes,
}

#[repr(C)]
/// Runtime representation of a signing key pair with an inline public wrapper.
pub struct MeshSigningKeyPair {
    pub private_key: *mut MeshSecretHandle,
    pub public_key: MeshSigningPublicKey,
}

#[cfg(test)]
struct GeneratedSigningKeyPair {
    private_key: *mut MeshSecretHandle,
    public_key: [u8; 32],
}

struct CryptoFailure {
    tag: CryptoErrorTag,
    expected: i64,
    actual: i64,
}

fn failure(tag: CryptoErrorTag, expected: i64, actual: i64) -> CryptoFailure {
    CryptoFailure {
        tag,
        expected,
        actual,
    }
}

fn error_result(error: CryptoFailure) -> *mut MeshResult {
    crypto_error(error.tag, error.expected, error.actual)
}

fn ok_result<T>(value: *mut T) -> *mut MeshResult {
    if value.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    alloc_result(0, value.cast())
}

fn complete_result<T>(result: *mut MeshResult, value: *mut T) -> *mut MeshResult {
    unsafe { (*result).value = value.cast() };
    result
}

fn allocate_value<T>(value: T) -> *mut T {
    let pointer = mesh_gc_alloc_actor(
        std::mem::size_of::<T>() as u64,
        std::mem::align_of::<T>() as u64,
    ) as *mut T;
    if !pointer.is_null() {
        unsafe { pointer.write(value) };
    }
    pointer
}

fn actual_length(length: u64) -> i64 {
    i64::try_from(length).unwrap_or(i64::MAX)
}

unsafe fn required_bytes<'a>(
    bytes: *const MeshBytes,
    maximum: usize,
) -> Result<&'a [u8], CryptoFailure> {
    if bytes.is_null() {
        return Err(failure(CryptoErrorTag::InvalidLength, maximum as i64, -1));
    }
    let length = (*bytes).len;
    if length > maximum as u64 {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            maximum as i64,
            actual_length(length),
        ));
    }
    Ok((*bytes).as_slice())
}

fn resource_failure(error: ResourceError) -> CryptoFailure {
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

fn with_current_process<R>(
    operation: impl FnOnce(&mut Process) -> Result<R, CryptoFailure>,
) -> Result<R, CryptoFailure> {
    let pid = crate::actor::stack::get_current_pid()
        .ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let scheduler = crate::actor::GLOBAL_SCHEDULER
        .get()
        .ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let process = scheduler
        .get_process(pid)
        .ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let mut process = process.lock();
    operation(&mut process)
}

unsafe fn valid_bytes<'a>(bytes: *const MeshBytes) -> Option<&'a [u8]> {
    if bytes.is_null() {
        return None;
    }
    Some((*bytes).as_slice())
}

fn digest_hex(digest: &[u8]) -> *mut MeshString {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = Vec::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize]);
        encoded.push(HEX[(byte & 0x0f) as usize]);
    }
    mesh_string_new(encoded.as_ptr(), encoded.len() as u64)
}

// ── Public ABI ─────────────────────────────────────────────────────────

/// Return the SHA-256 digest of arbitrary valid binary input.
#[no_mangle]
pub extern "C" fn mesh_crypto_sha256(input: *const MeshBytes) -> *mut MeshBytes {
    let Some(input) = (unsafe { valid_bytes(input) }) else {
        return ptr::null_mut();
    };
    let digest = SystemProvider.sha256(input);
    mesh_bytes_new(digest.as_ptr(), digest.len() as u64)
}

/// Return the SHA-512 digest of arbitrary valid binary input.
#[no_mangle]
pub extern "C" fn mesh_crypto_sha512(input: *const MeshBytes) -> *mut MeshBytes {
    let Some(input) = (unsafe { valid_bytes(input) }) else {
        return ptr::null_mut();
    };
    let digest = SystemProvider.sha512(input);
    mesh_bytes_new(digest.as_ptr(), digest.len() as u64)
}

/// Return the SHA-256 digest as lowercase hexadecimal text.
#[no_mangle]
pub extern "C" fn mesh_crypto_sha256_hex(input: *const MeshBytes) -> *mut MeshString {
    let Some(input) = (unsafe { valid_bytes(input) }) else {
        return ptr::null_mut();
    };
    digest_hex(&SystemProvider.sha256(input))
}

/// Return the SHA-512 digest as lowercase hexadecimal text.
#[no_mangle]
pub extern "C" fn mesh_crypto_sha512_hex(input: *const MeshBytes) -> *mut MeshString {
    let Some(input) = (unsafe { valid_bytes(input) }) else {
        return ptr::null_mut();
    };
    digest_hex(&SystemProvider.sha512(input))
}

fn random_bytes_with_provider(
    provider: &impl CryptoProvider,
    length: i64,
) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
    if length < 0 || length as u64 > MAX_RANDOM_BYTES as u64 {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            MAX_RANDOM_BYTES as i64,
            length,
        ));
    }
    let mut output = Zeroizing::new(vec![0; length as usize]);
    provider
        .fill_random(&mut output)
        .map_err(|error| match error {
            ProviderError::EntropyUnavailable => failure(CryptoErrorTag::EntropyUnavailable, 0, 0),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    Ok(output)
}

/// Return cryptographically secure random bytes from the operating system.
#[no_mangle]
pub extern "C" fn mesh_crypto_random_bytes(length: i64) -> *mut MeshResult {
    match random_bytes_with_provider(&SystemProvider, length) {
        Ok(bytes) => ok_result(mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64)),
        Err(error) => error_result(error),
    }
}

fn hmac_sha256_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    key: *const MeshSecretHandle,
    message: *const MeshBytes,
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    let message = unsafe { required_bytes(message, MAX_INPUT_BYTES) }?;
    let mut output = Zeroizing::new(vec![0; 32].into_boxed_slice());
    let operation = with_owned_resource(process, key, ResourceKind::SecretBytes, |key| {
        if key.len() > MAX_INPUT_BYTES {
            return Err(failure(
                CryptoErrorTag::InvalidLength,
                MAX_INPUT_BYTES as i64,
                key.len() as i64,
            ));
        }
        let output = <&mut [u8; 32]>::try_from(&mut output[..])
            .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
        provider
            .hmac_sha256(key, message, output)
            .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))
    })
    .map_err(resource_failure)?;
    operation?;
    insert_owned_resource(process, ResourceKind::SecretBytes, output).map_err(resource_failure)
}

/// Return HMAC-SHA-256 output as an actor-owned secret resource.
#[no_mangle]
pub extern "C" fn mesh_crypto_hmac_sha256(
    key: *const MeshSecretHandle,
    message: *const MeshBytes,
) -> *mut MeshResult {
    let result = alloc_result(0, ptr::null_mut());
    match with_current_process(|process| {
        hmac_sha256_for_process(process, &SystemProvider, key, message)
    }) {
        Ok(output) => complete_result(result, output),
        Err(error) => error_result(error),
    }
}

fn hkdf_sha256_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    input_key: *const MeshSecretHandle,
    salt: *const MeshBytes,
    info: *const MeshBytes,
    output_length: i64,
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    if output_length <= 0 || output_length as u64 > MAX_HKDF_OUTPUT_BYTES as u64 {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            MAX_HKDF_OUTPUT_BYTES as i64,
            output_length,
        ));
    }
    let salt = unsafe { required_bytes(salt, MAX_INPUT_BYTES) }?;
    let info = unsafe { required_bytes(info, MAX_INPUT_BYTES) }?;
    let mut output = Zeroizing::new(vec![0; output_length as usize].into_boxed_slice());
    let operation =
        with_owned_resource(process, input_key, ResourceKind::SecretBytes, |input_key| {
            if input_key.len() > MAX_INPUT_BYTES {
                return Err(failure(
                    CryptoErrorTag::InvalidLength,
                    MAX_INPUT_BYTES as i64,
                    input_key.len() as i64,
                ));
            }
            provider
                .hkdf_sha256(input_key, salt, info, &mut output)
                .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))
        })
        .map_err(resource_failure)?;
    operation?;
    insert_owned_resource(process, ResourceKind::SecretBytes, output).map_err(resource_failure)
}

/// Derive an actor-owned secret with HKDF-SHA-256.
#[no_mangle]
pub extern "C" fn mesh_crypto_hkdf_sha256(
    input_key: *const MeshSecretHandle,
    salt: *const MeshBytes,
    info: *const MeshBytes,
    output_length: i64,
) -> *mut MeshResult {
    let result = alloc_result(0, ptr::null_mut());
    match with_current_process(|process| {
        hkdf_sha256_for_process(
            process,
            &SystemProvider,
            input_key,
            salt,
            info,
            output_length,
        )
    }) {
        Ok(output) => complete_result(result, output),
        Err(error) => error_result(error),
    }
}

fn bounded_argon2_parameter(value: i64, minimum: u32, maximum: u32) -> Result<u32, CryptoFailure> {
    if value < i64::from(minimum) {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            i64::from(minimum),
            value,
        ));
    }
    if value > i64::from(maximum) {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            i64::from(maximum),
            value,
        ));
    }
    Ok(value as u32)
}

#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the six-parameter public Argon2id API with its process and provider context"
)]
fn argon2id_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    password: *const MeshSecretHandle,
    salt: *const MeshBytes,
    memory_kib: i64,
    iterations: i64,
    parallelism: i64,
    output_length: i64,
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    let parallelism = bounded_argon2_parameter(parallelism, 1, MAX_ARGON2_PARALLELISM)?;
    let memory_kib = bounded_argon2_parameter(memory_kib, parallelism * 8, MAX_ARGON2_MEMORY_KIB)?;
    let iterations = bounded_argon2_parameter(iterations, 1, MAX_ARGON2_ITERATIONS)?;
    let output_length = bounded_argon2_parameter(
        output_length,
        MIN_ARGON2_OUTPUT_BYTES as u32,
        MAX_ARGON2_OUTPUT_BYTES as u32,
    )? as usize;
    let salt = unsafe { required_bytes(salt, MAX_ARGON2_SALT_BYTES) }?;
    if salt.len() < MIN_ARGON2_SALT_BYTES {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            MIN_ARGON2_SALT_BYTES as i64,
            salt.len() as i64,
        ));
    }
    let password = with_owned_resource(process, password, ResourceKind::SecretBytes, |password| {
        if password.len() > MAX_INPUT_BYTES {
            return Err(failure(
                CryptoErrorTag::InvalidLength,
                MAX_INPUT_BYTES as i64,
                password.len() as i64,
            ));
        }
        Ok(Zeroizing::new(password.to_vec()))
    })
    .map_err(resource_failure)??;
    let mut output = Zeroizing::new(vec![0; output_length].into_boxed_slice());
    provider
        .argon2id(
            &password,
            salt,
            memory_kib,
            iterations,
            parallelism,
            &mut output,
        )
        .map_err(|error| match error {
            ProviderError::ResourceLimitExceeded => {
                failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
            }
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    insert_owned_resource(process, ResourceKind::SecretBytes, output).map_err(resource_failure)
}

/// Derive an actor-owned secret with Argon2id v1.3 and bounded costs.
#[no_mangle]
pub extern "C" fn mesh_crypto_argon2id(
    password: *const MeshSecretHandle,
    salt: *const MeshBytes,
    memory_kib: i64,
    iterations: i64,
    parallelism: i64,
    output_length: i64,
) -> *mut MeshResult {
    let result = alloc_result(0, ptr::null_mut());
    match with_current_process(|process| {
        argon2id_for_process(
            process,
            &SystemProvider,
            password,
            salt,
            memory_kib,
            iterations,
            parallelism,
            output_length,
        )
    }) {
        Ok(output) => complete_result(result, output),
        Err(error) => error_result(error),
    }
}

#[cfg(test)]
fn x25519_generate_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
) -> Result<GeneratedX25519KeyPair, CryptoFailure> {
    let (private_key, public_key) = x25519_key_material(provider)?;
    let private_key = insert_owned_resource(process, ResourceKind::X25519PrivateKey, private_key)
        .map_err(resource_failure)?;
    Ok(GeneratedX25519KeyPair {
        private_key,
        public_key,
    })
}

fn x25519_key_material(
    provider: &impl CryptoProvider,
) -> Result<(Zeroizing<Box<[u8]>>, [u8; 32]), CryptoFailure> {
    let mut private_key = Zeroizing::new(vec![0; 32].into_boxed_slice());
    provider
        .fill_random(&mut private_key)
        .map_err(|error| match error {
            ProviderError::EntropyUnavailable => failure(CryptoErrorTag::EntropyUnavailable, 0, 0),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    let private_array = <&[u8; 32]>::try_from(&private_key[..])
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let public_key = provider.x25519_public(private_array);
    Ok((private_key, public_key))
}

fn mlkem_key_material(
    provider: &impl CryptoProvider,
) -> Result<(Zeroizing<Box<[u8]>>, Vec<u8>), CryptoFailure> {
    let mut private_seed = Zeroizing::new(vec![0; MLKEM_PRIVATE_SEED_BYTES].into_boxed_slice());
    provider
        .fill_random(&mut private_seed)
        .map_err(|error| match error {
            ProviderError::EntropyUnavailable => failure(CryptoErrorTag::EntropyUnavailable, 0, 0),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    mlkem_from_seed_material(private_seed)
}

fn mlkem_from_seed_material(
    private_seed: Zeroizing<Box<[u8]>>,
) -> Result<(Zeroizing<Box<[u8]>>, Vec<u8>), CryptoFailure> {
    let seed = Seed::try_from(private_seed.as_ref()).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidKey,
            MLKEM_PRIVATE_SEED_BYTES as i64,
            private_seed.len() as i64,
        )
    })?;
    let private_key = DecapsulationKey768::from_seed(seed);
    let public_key = private_key.encapsulation_key().to_bytes().to_vec();
    Ok((private_seed, public_key))
}

fn mlkem_encapsulate_material(
    provider: &impl CryptoProvider,
    public_key: &[u8],
) -> Result<(Vec<u8>, Zeroizing<Box<[u8]>>), CryptoFailure> {
    let encoded_key = Key::<EncapsulationKey768>::try_from(public_key).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidLength,
            MLKEM_PUBLIC_KEY_BYTES as i64,
            public_key.len() as i64,
        )
    })?;
    let public_key = EncapsulationKey768::new(&encoded_key)
        .map_err(|_| failure(CryptoErrorTag::InvalidPublicKey, 0, 0))?;
    let mut randomness = Zeroizing::new(vec![0; MLKEM_SHARED_SECRET_BYTES].into_boxed_slice());
    provider
        .fill_random(&mut randomness)
        .map_err(|error| match error {
            ProviderError::EntropyUnavailable => failure(CryptoErrorTag::EntropyUnavailable, 0, 0),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    let randomness = B32::try_from(randomness.as_ref())
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let (ciphertext, shared_secret) = public_key.encapsulate_deterministic(&randomness);
    Ok((
        ciphertext.to_vec(),
        Zeroizing::new(shared_secret.to_vec().into_boxed_slice()),
    ))
}

fn mlkem_decapsulate_material(
    private_seed: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Box<[u8]>>, CryptoFailure> {
    let private_seed = Seed::try_from(private_seed).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidKey,
            MLKEM_PRIVATE_SEED_BYTES as i64,
            private_seed.len() as i64,
        )
    })?;
    let ciphertext = MlKem768Ciphertext::try_from(ciphertext).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidLength,
            MLKEM_CIPHERTEXT_BYTES as i64,
            ciphertext.len() as i64,
        )
    })?;
    let private_key = DecapsulationKey768::from_seed(private_seed);
    let shared_secret = private_key.decapsulate(&ciphertext);
    Ok(Zeroizing::new(shared_secret.to_vec().into_boxed_slice()))
}

/// Generate an actor-owned X25519 private key and its public key.
#[no_mangle]
pub extern "C" fn mesh_crypto_x25519_generate() -> *mut MeshResult {
    let (private_material, public_key) = match x25519_key_material(&SystemProvider) {
        Ok(material) => material,
        Err(error) => return error_result(error),
    };
    allocate_x25519_key_pair(private_material, public_key)
}

fn allocate_x25519_key_pair(
    private_material: Zeroizing<Box<[u8]>>,
    public_key: [u8; 32],
) -> *mut MeshResult {
    let public_bytes = mesh_bytes_new(public_key.as_ptr(), 32);
    if public_bytes.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let key_pair = allocate_value(MeshX25519KeyPair {
        private_key: ptr::null_mut(),
        public_key: MeshX25519PublicKey {
            bytes: public_bytes,
        },
    });
    if key_pair.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let result = alloc_result(0, key_pair.cast());
    let private_key = match with_current_process(move |process| {
        insert_owned_resource(process, ResourceKind::X25519PrivateKey, private_material)
            .map_err(resource_failure)
    }) {
        Ok(private_key) => private_key,
        Err(error) => return error_result(error),
    };
    unsafe { (*key_pair).private_key = private_key };
    result
}

/// Construct an actor-owned X25519 key pair from exact 32-byte private material.
#[no_mangle]
pub extern "C" fn mesh_crypto_x25519_from_seed(seed: *const MeshBytes) -> *mut MeshResult {
    let seed = match unsafe { required_bytes(seed, 32) } {
        Ok(seed) if seed.len() == 32 => seed,
        Ok(seed) => {
            return crypto_error(CryptoErrorTag::InvalidLength, 32, seed.len() as i64);
        }
        Err(error) => return error_result(error),
    };
    let private_material = Zeroizing::new(seed.to_vec().into_boxed_slice());
    let private_array = <&[u8; 32]>::try_from(&private_material[..]).expect("length checked above");
    let public_key = SystemProvider.x25519_public(private_array);
    allocate_x25519_key_pair(private_material, public_key)
}

fn x25519_from_secret_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    material: *const MeshSecretHandle,
) -> Result<(*mut MeshSecretHandle, [u8; 32]), CryptoFailure> {
    let mut public_key = [0; 32];
    let private_key = consume_and_retype_owned_resource(
        process,
        material,
        ResourceKind::SecretBytes,
        ResourceKind::X25519PrivateKey,
        |bytes| {
            let private_key = <&[u8; 32]>::try_from(bytes).map_err(|_| bytes.len() as i64)?;
            public_key = provider.x25519_public(private_key);
            Ok(())
        },
    )
    .map_err(|error| match error {
        RetypeError::Resource(error) => resource_failure(error),
        RetypeError::Rejected {
            error: actual,
            removed,
        } => {
            drop(removed);
            failure(CryptoErrorTag::InvalidKey, 32, actual)
        }
        RetypeError::GenerationExhausted { removed } => {
            drop(removed);
            failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
    })?;
    Ok((private_key, public_key))
}

/// Consume exactly 32 secret bytes and retype them as an X25519 private key.
#[no_mangle]
pub extern "C" fn mesh_crypto_x25519_from_secret(
    material: *const MeshSecretHandle,
) -> *mut MeshResult {
    let (private_key, public_key) = match with_current_process(|process| {
        x25519_from_secret_for_process(process, &SystemProvider, material)
    }) {
        Ok(value) => value,
        Err(error) => return error_result(error),
    };
    let public_bytes = mesh_bytes_new(public_key.as_ptr(), public_key.len() as u64);
    if public_bytes.is_null() {
        mesh_resource_destroy(private_key);
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let key_pair = allocate_value(MeshX25519KeyPair {
        private_key,
        public_key: MeshX25519PublicKey {
            bytes: public_bytes,
        },
    });
    if key_pair.is_null() {
        mesh_resource_destroy(private_key);
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    ok_result(key_pair)
}

unsafe fn x25519_public_key_bytes(
    public_key: *const MeshX25519PublicKey,
) -> Result<[u8; 32], CryptoFailure> {
    if public_key.is_null() || (*public_key).bytes.is_null() {
        return Err(failure(CryptoErrorTag::InvalidPublicKey, 32, -1));
    }
    let bytes = (*public_key).bytes;
    if (*bytes).len != 32 {
        return Err(failure(
            CryptoErrorTag::InvalidPublicKey,
            32,
            actual_length((*bytes).len),
        ));
    }
    <[u8; 32]>::try_from((*bytes).as_slice()).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidPublicKey,
            32,
            actual_length((*bytes).len),
        )
    })
}

fn x25519_public_for_process(
    process: &Process,
    provider: &impl CryptoProvider,
    private_key: *const MeshSecretHandle,
) -> Result<[u8; 32], CryptoFailure> {
    with_owned_resource(
        process,
        private_key,
        ResourceKind::X25519PrivateKey,
        |private_key| {
            let private_key = <&[u8; 32]>::try_from(private_key)
                .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, private_key.len() as i64))?;
            Ok(provider.x25519_public(private_key))
        },
    )
    .map_err(resource_failure)?
}

fn allocate_x25519_public_key(
    public_key: [u8; 32],
) -> Result<*mut MeshX25519PublicKey, CryptoFailure> {
    let bytes = mesh_bytes_new(public_key.as_ptr(), public_key.len() as u64);
    if bytes.is_null() {
        return Err(failure(CryptoErrorTag::InternalFailure, 0, 0));
    }
    let public_key = allocate_value(MeshX25519PublicKey { bytes });
    if public_key.is_null() {
        return Err(failure(CryptoErrorTag::InternalFailure, 0, 0));
    }
    Ok(public_key)
}

/// Derive the public half of an actor-owned X25519 private key.
#[no_mangle]
pub extern "C" fn mesh_crypto_x25519_public(
    private_key: *const MeshSecretHandle,
) -> *mut MeshResult {
    let public_key = match with_current_process(|process| {
        x25519_public_for_process(process, &SystemProvider, private_key)
    }) {
        Ok(public_key) => public_key,
        Err(error) => return error_result(error),
    };
    match allocate_x25519_public_key(public_key) {
        Ok(public_key) => ok_result(public_key),
        Err(error) => error_result(error),
    }
}

fn x25519_shared_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    private_key: *const MeshSecretHandle,
    peer_public_key: *const MeshX25519PublicKey,
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    let peer_public_key = unsafe { x25519_public_key_bytes(peer_public_key) }?;
    let mut shared_secret = Zeroizing::new(vec![0; 32].into_boxed_slice());
    let operation = with_owned_resource(
        process,
        private_key,
        ResourceKind::X25519PrivateKey,
        |private_key| {
            let private_key = <&[u8; 32]>::try_from(private_key)
                .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, private_key.len() as i64))?;
            let output = <&mut [u8; 32]>::try_from(&mut shared_secret[..])
                .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
            provider
                .x25519_shared(private_key, &peer_public_key, output)
                .map_err(|error| match error {
                    ProviderError::InvalidPublicKey => {
                        failure(CryptoErrorTag::InvalidPublicKey, 32, 32)
                    }
                    _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
                })
        },
    )
    .map_err(resource_failure)?;
    operation?;
    insert_owned_resource(process, ResourceKind::SecretBytes, shared_secret)
        .map_err(resource_failure)
}

/// Derive an actor-owned X25519 shared secret.
#[no_mangle]
pub extern "C" fn mesh_crypto_x25519_shared(
    private_key: *const MeshSecretHandle,
    peer_public_key: *const MeshX25519PublicKey,
) -> *mut MeshResult {
    let result = alloc_result(0, ptr::null_mut());
    match with_current_process(|process| {
        x25519_shared_for_process(process, &SystemProvider, private_key, peer_public_key)
    }) {
        Ok(shared_secret) => complete_result(result, shared_secret),
        Err(error) => error_result(error),
    }
}

fn hpke_labeled_extract(
    provider: &impl CryptoProvider,
    suite_id: &[u8],
    salt: &[u8],
    label: &[u8],
    input_key_material: &[u8],
) -> Result<Zeroizing<[u8; 32]>, CryptoFailure> {
    let mut labeled_input = Zeroizing::new(Vec::with_capacity(
        HPKE_VERSION_LABEL.len() + suite_id.len() + label.len() + input_key_material.len(),
    ));
    labeled_input.extend_from_slice(HPKE_VERSION_LABEL);
    labeled_input.extend_from_slice(suite_id);
    labeled_input.extend_from_slice(label);
    labeled_input.extend_from_slice(input_key_material);
    let mut output = Zeroizing::new([0; 32]);
    provider
        .hmac_sha256(salt, &labeled_input, &mut output)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    Ok(output)
}

fn hpke_labeled_expand<const N: usize>(
    provider: &impl CryptoProvider,
    suite_id: &[u8],
    pseudo_random_key: &[u8; 32],
    label: &[u8],
    info: &[u8],
) -> Result<Zeroizing<[u8; N]>, CryptoFailure> {
    if N == 0 || N > 32 {
        return Err(failure(CryptoErrorTag::InternalFailure, 32, N as i64));
    }
    let length =
        u16::try_from(N).map_err(|_| failure(CryptoErrorTag::InternalFailure, 32, N as i64))?;
    let mut labeled_info = Vec::with_capacity(
        2 + HPKE_VERSION_LABEL.len() + suite_id.len() + label.len() + info.len() + 1,
    );
    labeled_info.extend_from_slice(&length.to_be_bytes());
    labeled_info.extend_from_slice(HPKE_VERSION_LABEL);
    labeled_info.extend_from_slice(suite_id);
    labeled_info.extend_from_slice(label);
    labeled_info.extend_from_slice(info);
    labeled_info.push(1);
    let mut block = Zeroizing::new([0; 32]);
    provider
        .hmac_sha256(pseudo_random_key, &labeled_info, &mut block)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let mut output = Zeroizing::new([0; N]);
    output.copy_from_slice(&block[..N]);
    Ok(output)
}

fn hpke_derive_private_key(
    provider: &impl CryptoProvider,
    input_key_material: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, CryptoFailure> {
    let pseudo_random_key = hpke_labeled_extract(
        provider,
        HPKE_KEM_SUITE_ID,
        &[],
        b"dkp_prk",
        input_key_material,
    )?;
    hpke_labeled_expand(provider, HPKE_KEM_SUITE_ID, &pseudo_random_key, b"sk", &[])
}

fn hpke_shared_secret(
    provider: &impl CryptoProvider,
    dh: &[u8; 32],
    encapsulated_key: &[u8; 32],
    recipient_public_key: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, CryptoFailure> {
    let pseudo_random_key = hpke_labeled_extract(provider, HPKE_KEM_SUITE_ID, &[], b"eae_prk", dh)?;
    let mut kem_context = [0; 64];
    kem_context[..32].copy_from_slice(encapsulated_key);
    kem_context[32..].copy_from_slice(recipient_public_key);
    hpke_labeled_expand(
        provider,
        HPKE_KEM_SUITE_ID,
        &pseudo_random_key,
        b"shared_secret",
        &kem_context,
    )
}

fn hpke_key_and_nonce(
    provider: &impl CryptoProvider,
    shared_secret: &[u8; 32],
    info: &[u8],
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 12]>), CryptoFailure> {
    let psk_id_hash = hpke_labeled_extract(provider, HPKE_SUITE_ID, &[], b"psk_id_hash", &[])?;
    let info_hash = hpke_labeled_extract(provider, HPKE_SUITE_ID, &[], b"info_hash", info)?;
    let mut key_schedule_context = Zeroizing::new([0; 65]);
    key_schedule_context[1..33].copy_from_slice(&psk_id_hash[..]);
    key_schedule_context[33..].copy_from_slice(&info_hash[..]);
    let secret = hpke_labeled_extract(provider, HPKE_SUITE_ID, shared_secret, b"secret", &[])?;
    let key = hpke_labeled_expand(
        provider,
        HPKE_SUITE_ID,
        &secret,
        b"key",
        &key_schedule_context[..],
    )?;
    let nonce = hpke_labeled_expand(
        provider,
        HPKE_SUITE_ID,
        &secret,
        b"base_nonce",
        &key_schedule_context[..],
    )?;
    Ok((key, nonce))
}

fn hpke_seal_material(
    provider: &impl CryptoProvider,
    recipient_public_key: &[u8; 32],
    info: &[u8],
    associated_data: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoFailure> {
    let mut input_key_material = Zeroizing::new([0; 32]);
    provider
        .fill_random(&mut input_key_material[..])
        .map_err(|error| match error {
            ProviderError::EntropyUnavailable => failure(CryptoErrorTag::EntropyUnavailable, 0, 0),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    let ephemeral_private_key = hpke_derive_private_key(provider, &input_key_material)?;
    let encapsulated_key = provider.x25519_public(&ephemeral_private_key);
    let mut dh = Zeroizing::new([0; 32]);
    provider
        .x25519_shared(&ephemeral_private_key, recipient_public_key, &mut dh)
        .map_err(|error| match error {
            ProviderError::InvalidPublicKey => failure(CryptoErrorTag::InvalidPublicKey, 32, 32),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    let shared_secret = hpke_shared_secret(provider, &dh, &encapsulated_key, recipient_public_key)?;
    let (key, nonce) = hpke_key_and_nonce(provider, &shared_secret, info)?;
    let ciphertext = provider
        .chacha20poly1305_seal(&key, &nonce, associated_data, plaintext)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let mut sealed = Vec::with_capacity(HPKE_ENCAPSULATED_KEY_BYTES + ciphertext.len());
    sealed.extend_from_slice(&encapsulated_key);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

fn hpke_open_material(
    provider: &impl CryptoProvider,
    recipient_private_key: &[u8; 32],
    info: &[u8],
    associated_data: &[u8],
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
    if sealed.len() < HPKE_MIN_SEALED_BYTES || sealed.len() > MAX_HPKE_SEALED_BYTES {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            HPKE_MIN_SEALED_BYTES as i64,
            sealed.len() as i64,
        ));
    }
    let encapsulated_key = <[u8; 32]>::try_from(&sealed[..32])
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let recipient_public_key = provider.x25519_public(recipient_private_key);
    let mut dh = Zeroizing::new([0; 32]);
    provider
        .x25519_shared(recipient_private_key, &encapsulated_key, &mut dh)
        .map_err(|error| match error {
            ProviderError::InvalidPublicKey => failure(CryptoErrorTag::InvalidPublicKey, 32, 32),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    let shared_secret =
        hpke_shared_secret(provider, &dh, &encapsulated_key, &recipient_public_key)?;
    let (key, nonce) = hpke_key_and_nonce(provider, &shared_secret, info)?;
    let mut plaintext = Zeroizing::new(sealed[32..].to_vec());
    provider
        .chacha20poly1305_open(&key, &nonce, associated_data, &mut plaintext)
        .map_err(|error| match error {
            ProviderError::AuthenticationFailed => {
                failure(CryptoErrorTag::AuthenticationFailed, 0, 0)
            }
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    Ok(plaintext)
}

/// Seal one RFC 9180 base-mode X25519/HKDF-SHA-256/ChaCha20-Poly1305 message.
/// The returned canonical wire value is `enc || ciphertext`.
#[no_mangle]
pub extern "C" fn mesh_crypto_hpke_seal(
    recipient_public_key: *const MeshX25519PublicKey,
    info: *const MeshBytes,
    associated_data: *const MeshBytes,
    plaintext: *const MeshBytes,
) -> *mut MeshResult {
    let recipient_public_key = match unsafe { x25519_public_key_bytes(recipient_public_key) } {
        Ok(key) => key,
        Err(error) => return error_result(error),
    };
    let info = match unsafe { required_bytes(info, MAX_HPKE_INFO_BYTES) } {
        Ok(info) => info,
        Err(error) => return error_result(error),
    };
    let associated_data = match unsafe { required_bytes(associated_data, MAX_INPUT_BYTES) } {
        Ok(data) => data,
        Err(error) => return error_result(error),
    };
    let plaintext = match unsafe { required_bytes(plaintext, MAX_INPUT_BYTES) } {
        Ok(plaintext) => plaintext,
        Err(error) => return error_result(error),
    };
    match hpke_seal_material(
        &SystemProvider,
        &recipient_public_key,
        info,
        associated_data,
        plaintext,
    ) {
        Ok(sealed) => ok_result(mesh_bytes_new(sealed.as_ptr(), sealed.len() as u64)),
        Err(error) => error_result(error),
    }
}

fn hpke_open_for_process(
    process: &Process,
    provider: &impl CryptoProvider,
    recipient_private_key: *const MeshSecretHandle,
    info: &[u8],
    associated_data: &[u8],
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
    with_owned_resource(
        process,
        recipient_private_key,
        ResourceKind::X25519PrivateKey,
        |private_key| {
            let private_key = <&[u8; 32]>::try_from(private_key)
                .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, private_key.len() as i64))?;
            hpke_open_material(provider, private_key, info, associated_data, sealed)
        },
    )
    .map_err(resource_failure)?
}

/// Open one canonical `enc || ciphertext` RFC 9180 base-mode message.
#[no_mangle]
pub extern "C" fn mesh_crypto_hpke_open(
    recipient_private_key: *const MeshSecretHandle,
    info: *const MeshBytes,
    associated_data: *const MeshBytes,
    sealed: *const MeshBytes,
) -> *mut MeshResult {
    let info = match unsafe { required_bytes(info, MAX_HPKE_INFO_BYTES) } {
        Ok(info) => info,
        Err(error) => return error_result(error),
    };
    let associated_data = match unsafe { required_bytes(associated_data, MAX_INPUT_BYTES) } {
        Ok(data) => data,
        Err(error) => return error_result(error),
    };
    let sealed = match unsafe { required_bytes(sealed, MAX_HPKE_SEALED_BYTES) } {
        Ok(sealed) => sealed,
        Err(error) => return error_result(error),
    };
    match with_current_process(|process| {
        hpke_open_for_process(
            process,
            &SystemProvider,
            recipient_private_key,
            info,
            associated_data,
            sealed,
        )
    }) {
        Ok(plaintext) => ok_result(mesh_bytes_new(plaintext.as_ptr(), plaintext.len() as u64)),
        Err(error) => error_result(error),
    }
}

fn hpke_seal_secret_for_process(
    process: &Process,
    provider: &impl CryptoProvider,
    recipient_public_key: &[u8; 32],
    info: &[u8],
    associated_data: &[u8],
    plaintext: *const MeshSecretHandle,
) -> Result<Vec<u8>, CryptoFailure> {
    with_owned_resource(process, plaintext, ResourceKind::SecretBytes, |plaintext| {
        hpke_seal_material(
            provider,
            recipient_public_key,
            info,
            associated_data,
            plaintext,
        )
    })
    .map_err(resource_failure)?
}

/// Seal actor-owned secret material without exposing it as ordinary bytes.
#[no_mangle]
pub extern "C" fn mesh_crypto_hpke_seal_secret(
    recipient_public_key: *const MeshX25519PublicKey,
    info: *const MeshBytes,
    associated_data: *const MeshBytes,
    plaintext: *const MeshSecretHandle,
) -> *mut MeshResult {
    let recipient_public_key = match unsafe { x25519_public_key_bytes(recipient_public_key) } {
        Ok(key) => key,
        Err(error) => return error_result(error),
    };
    let info = match unsafe { required_bytes(info, MAX_HPKE_INFO_BYTES) } {
        Ok(info) => info,
        Err(error) => return error_result(error),
    };
    let associated_data = match unsafe { required_bytes(associated_data, MAX_INPUT_BYTES) } {
        Ok(data) => data,
        Err(error) => return error_result(error),
    };
    match with_current_process(|process| {
        hpke_seal_secret_for_process(
            process,
            &SystemProvider,
            &recipient_public_key,
            info,
            associated_data,
            plaintext,
        )
    }) {
        Ok(sealed) => ok_result(mesh_bytes_new(sealed.as_ptr(), sealed.len() as u64)),
        Err(error) => error_result(error),
    }
}

fn hpke_open_secret_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    recipient_private_key: *const MeshSecretHandle,
    info: &[u8],
    associated_data: &[u8],
    sealed: &[u8],
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    let plaintext = hpke_open_for_process(
        process,
        provider,
        recipient_private_key,
        info,
        associated_data,
        sealed,
    )?;
    let plaintext = Zeroizing::new(plaintext.as_slice().to_vec().into_boxed_slice());
    insert_owned_resource(process, ResourceKind::SecretBytes, plaintext).map_err(resource_failure)
}

/// Open sealed material directly into an actor-owned secret resource.
#[no_mangle]
pub extern "C" fn mesh_crypto_hpke_open_secret(
    recipient_private_key: *const MeshSecretHandle,
    info: *const MeshBytes,
    associated_data: *const MeshBytes,
    sealed: *const MeshBytes,
) -> *mut MeshResult {
    let info = match unsafe { required_bytes(info, MAX_HPKE_INFO_BYTES) } {
        Ok(info) => info,
        Err(error) => return error_result(error),
    };
    let associated_data = match unsafe { required_bytes(associated_data, MAX_INPUT_BYTES) } {
        Ok(data) => data,
        Err(error) => return error_result(error),
    };
    let sealed = match unsafe { required_bytes(sealed, MAX_HPKE_SEALED_BYTES) } {
        Ok(sealed) => sealed,
        Err(error) => return error_result(error),
    };
    let result = alloc_result(0, ptr::null_mut());
    match with_current_process(|process| {
        hpke_open_secret_for_process(
            process,
            &SystemProvider,
            recipient_private_key,
            info,
            associated_data,
            sealed,
        )
    }) {
        Ok(plaintext) => complete_result(result, plaintext),
        Err(error) => error_result(error),
    }
}

fn allocate_mlkem_key_pair(
    private_material: Zeroizing<Box<[u8]>>,
    public_key: &[u8],
) -> *mut MeshResult {
    let public_bytes = mesh_bytes_new(public_key.as_ptr(), public_key.len() as u64);
    if public_bytes.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let key_pair = allocate_value(MeshMlKemKeyPair {
        private_key: ptr::null_mut(),
        public_key: MeshMlKemPublicKey {
            bytes: public_bytes,
        },
    });
    if key_pair.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let result = alloc_result(0, key_pair.cast());
    let private_key = match with_current_process(move |process| {
        insert_owned_resource(process, ResourceKind::MlKemPrivateKey, private_material)
            .map_err(resource_failure)
    }) {
        Ok(private_key) => private_key,
        Err(error) => return error_result(error),
    };
    unsafe { (*key_pair).private_key = private_key };
    result
}

/// Generate an actor-owned ML-KEM-768 decapsulation key and its public key.
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_generate() -> *mut MeshResult {
    match mlkem_key_material(&SystemProvider) {
        Ok((private_key, public_key)) => allocate_mlkem_key_pair(private_key, &public_key),
        Err(error) => error_result(error),
    }
}

/// Construct an actor-owned ML-KEM-768 key pair from an exact 64-byte seed.
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_from_seed(seed: *const MeshBytes) -> *mut MeshResult {
    let seed = match unsafe { required_bytes(seed, MLKEM_PRIVATE_SEED_BYTES) } {
        Ok(seed) if seed.len() == MLKEM_PRIVATE_SEED_BYTES => seed,
        Ok(seed) => {
            return crypto_error(
                CryptoErrorTag::InvalidLength,
                MLKEM_PRIVATE_SEED_BYTES as i64,
                seed.len() as i64,
            );
        }
        Err(error) => return error_result(error),
    };
    match mlkem_from_seed_material(Zeroizing::new(seed.to_vec().into_boxed_slice())) {
        Ok((private_key, public_key)) => allocate_mlkem_key_pair(private_key, &public_key),
        Err(error) => error_result(error),
    }
}

fn mlkem_from_secret_for_process(
    process: &mut Process,
    material: *const MeshSecretHandle,
) -> Result<(*mut MeshSecretHandle, Vec<u8>), CryptoFailure> {
    let mut public_key = Vec::new();
    let private_key = consume_and_retype_owned_resource(
        process,
        material,
        ResourceKind::SecretBytes,
        ResourceKind::MlKemPrivateKey,
        |bytes| {
            let seed = Seed::try_from(bytes).map_err(|_| {
                failure(
                    CryptoErrorTag::InvalidKey,
                    MLKEM_PRIVATE_SEED_BYTES as i64,
                    bytes.len() as i64,
                )
            })?;
            public_key = DecapsulationKey768::from_seed(seed)
                .encapsulation_key()
                .to_bytes()
                .to_vec();
            Ok(())
        },
    )
    .map_err(|error| match error {
        RetypeError::Resource(error) => resource_failure(error),
        RetypeError::Rejected { error, removed } => {
            drop(removed);
            error
        }
        RetypeError::GenerationExhausted { removed } => {
            drop(removed);
            failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
    })?;
    Ok((private_key, public_key))
}

/// Consume exactly 64 secret bytes and retype them as an ML-KEM-768 private key.
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_from_secret(
    material: *const MeshSecretHandle,
) -> *mut MeshResult {
    let (private_key, public_key) =
        match with_current_process(|process| mlkem_from_secret_for_process(process, material)) {
            Ok(value) => value,
            Err(error) => return error_result(error),
        };
    let public_bytes = mesh_bytes_new(public_key.as_ptr(), public_key.len() as u64);
    if public_bytes.is_null() {
        mesh_resource_destroy(private_key);
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let key_pair = allocate_value(MeshMlKemKeyPair {
        private_key,
        public_key: MeshMlKemPublicKey {
            bytes: public_bytes,
        },
    });
    if key_pair.is_null() {
        mesh_resource_destroy(private_key);
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    ok_result(key_pair)
}

unsafe fn mlkem_public_key_bytes<'a>(
    public_key: *const MeshMlKemPublicKey,
) -> Result<&'a [u8], CryptoFailure> {
    if public_key.is_null() || (*public_key).bytes.is_null() {
        return Err(failure(
            CryptoErrorTag::InvalidPublicKey,
            MLKEM_PUBLIC_KEY_BYTES as i64,
            -1,
        ));
    }
    let bytes = (*public_key).bytes;
    if (*bytes).len != MLKEM_PUBLIC_KEY_BYTES as u64 {
        return Err(failure(
            CryptoErrorTag::InvalidPublicKey,
            MLKEM_PUBLIC_KEY_BYTES as i64,
            actual_length((*bytes).len),
        ));
    }
    Ok((*bytes).as_slice())
}

unsafe fn mlkem_ciphertext_bytes<'a>(
    ciphertext: *const MeshMlKemCiphertext,
) -> Result<&'a [u8], CryptoFailure> {
    if ciphertext.is_null() || (*ciphertext).bytes.is_null() {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            MLKEM_CIPHERTEXT_BYTES as i64,
            -1,
        ));
    }
    let bytes = (*ciphertext).bytes;
    if (*bytes).len != MLKEM_CIPHERTEXT_BYTES as u64 {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            MLKEM_CIPHERTEXT_BYTES as i64,
            actual_length((*bytes).len),
        ));
    }
    Ok((*bytes).as_slice())
}

/// Encapsulate to an exact ML-KEM-768 public key.
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_encapsulate(
    public_key: *const MeshMlKemPublicKey,
) -> *mut MeshResult {
    let public_key = match unsafe { mlkem_public_key_bytes(public_key) } {
        Ok(public_key) => public_key,
        Err(error) => return error_result(error),
    };
    let (ciphertext, shared_secret) = match mlkem_encapsulate_material(&SystemProvider, public_key)
    {
        Ok(output) => output,
        Err(error) => return error_result(error),
    };
    let ciphertext = mesh_bytes_new(ciphertext.as_ptr(), ciphertext.len() as u64);
    if ciphertext.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let output = allocate_value(MeshTuple2Pointers {
        len: 2,
        first: ciphertext,
        second: ptr::null_mut(),
    });
    if output.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let result = alloc_result(0, output.cast());
    let shared_secret = match with_current_process(move |process| {
        insert_owned_resource(process, ResourceKind::SecretBytes, shared_secret)
            .map_err(resource_failure)
    }) {
        Ok(shared_secret) => shared_secret,
        Err(error) => return error_result(error),
    };
    unsafe { (*output).second = shared_secret };
    result
}

/// Decapsulate an ML-KEM-768 ciphertext with an actor-owned private key.
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_decapsulate(
    private_key: *const MeshSecretHandle,
    ciphertext: *const MeshMlKemCiphertext,
) -> *mut MeshResult {
    let ciphertext = match unsafe { mlkem_ciphertext_bytes(ciphertext) } {
        Ok(ciphertext) => ciphertext,
        Err(error) => return error_result(error),
    };
    let result = alloc_result(0, ptr::null_mut());
    match with_current_process(|process| {
        let shared_secret = with_owned_resource(
            process,
            private_key,
            ResourceKind::MlKemPrivateKey,
            |private_seed| mlkem_decapsulate_material(private_seed, ciphertext),
        )
        .map_err(resource_failure)??;
        insert_owned_resource(process, ResourceKind::SecretBytes, shared_secret)
            .map_err(resource_failure)
    }) {
        Ok(shared_secret) => complete_result(result, shared_secret),
        Err(error) => error_result(error),
    }
}

#[cfg(test)]
fn signing_generate_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
) -> Result<GeneratedSigningKeyPair, CryptoFailure> {
    let (private_key, public_key) = signing_key_material(provider)?;
    let private_key = insert_owned_resource(process, ResourceKind::SigningPrivateKey, private_key)
        .map_err(resource_failure)?;
    Ok(GeneratedSigningKeyPair {
        private_key,
        public_key,
    })
}

fn signing_key_material(
    provider: &impl CryptoProvider,
) -> Result<(Zeroizing<Box<[u8]>>, [u8; 32]), CryptoFailure> {
    let mut private_key = Zeroizing::new(vec![0; 32].into_boxed_slice());
    provider
        .fill_random(&mut private_key)
        .map_err(|error| match error {
            ProviderError::EntropyUnavailable => failure(CryptoErrorTag::EntropyUnavailable, 0, 0),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })?;
    let private_array = <&[u8; 32]>::try_from(&private_key[..])
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let public_key = provider.ed25519_public(private_array);
    Ok((private_key, public_key))
}

/// Generate an actor-owned signing private key and its public key.
#[no_mangle]
pub extern "C" fn mesh_crypto_signing_generate() -> *mut MeshResult {
    let (private_material, public_key) = match signing_key_material(&SystemProvider) {
        Ok(material) => material,
        Err(error) => return error_result(error),
    };
    allocate_signing_key_pair(private_material, public_key)
}

fn allocate_signing_key_pair(
    private_material: Zeroizing<Box<[u8]>>,
    public_key: [u8; 32],
) -> *mut MeshResult {
    let public_bytes = mesh_bytes_new(public_key.as_ptr(), 32);
    if public_bytes.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let key_pair = allocate_value(MeshSigningKeyPair {
        private_key: ptr::null_mut(),
        public_key: MeshSigningPublicKey {
            bytes: public_bytes,
        },
    });
    if key_pair.is_null() {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let result = alloc_result(0, key_pair.cast());
    let private_key = match with_current_process(move |process| {
        insert_owned_resource(process, ResourceKind::SigningPrivateKey, private_material)
            .map_err(resource_failure)
    }) {
        Ok(private_key) => private_key,
        Err(error) => return error_result(error),
    };
    unsafe { (*key_pair).private_key = private_key };
    result
}

/// Construct an actor-owned signing key pair from an exact 32-byte Ed25519 seed.
#[no_mangle]
pub extern "C" fn mesh_crypto_signing_from_seed(seed: *const MeshBytes) -> *mut MeshResult {
    let seed = match unsafe { required_bytes(seed, 32) } {
        Ok(seed) if seed.len() == 32 => seed,
        Ok(seed) => {
            return crypto_error(CryptoErrorTag::InvalidLength, 32, seed.len() as i64);
        }
        Err(error) => return error_result(error),
    };
    let private_material = Zeroizing::new(seed.to_vec().into_boxed_slice());
    let private_array = <&[u8; 32]>::try_from(&private_material[..]).expect("length checked above");
    let public_key = SystemProvider.ed25519_public(private_array);
    allocate_signing_key_pair(private_material, public_key)
}

fn signing_from_secret_for_process(
    process: &mut Process,
    provider: &impl CryptoProvider,
    material: *const MeshSecretHandle,
) -> Result<(*mut MeshSecretHandle, [u8; 32]), CryptoFailure> {
    let mut public_key = [0; 32];
    let private_key = consume_and_retype_owned_resource(
        process,
        material,
        ResourceKind::SecretBytes,
        ResourceKind::SigningPrivateKey,
        |bytes| {
            let private_key = <&[u8; 32]>::try_from(bytes)
                .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, bytes.len() as i64))?;
            public_key = provider.ed25519_public(private_key);
            Ok(())
        },
    )
    .map_err(|error| match error {
        RetypeError::Resource(error) => resource_failure(error),
        RetypeError::Rejected { error, removed } => {
            drop(removed);
            error
        }
        RetypeError::GenerationExhausted { removed } => {
            drop(removed);
            failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
    })?;
    Ok((private_key, public_key))
}

/// Consume exactly 32 secret bytes and retype them as an Ed25519 signing private key.
#[no_mangle]
pub extern "C" fn mesh_crypto_signing_from_secret(
    material: *const MeshSecretHandle,
) -> *mut MeshResult {
    let (private_key, public_key) = match with_current_process(|process| {
        signing_from_secret_for_process(process, &SystemProvider, material)
    }) {
        Ok(value) => value,
        Err(error) => return error_result(error),
    };
    let public_bytes = mesh_bytes_new(public_key.as_ptr(), public_key.len() as u64);
    if public_bytes.is_null() {
        mesh_resource_destroy(private_key);
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    let key_pair = allocate_value(MeshSigningKeyPair {
        private_key,
        public_key: MeshSigningPublicKey {
            bytes: public_bytes,
        },
    });
    if key_pair.is_null() {
        mesh_resource_destroy(private_key);
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    }
    ok_result(key_pair)
}

fn sign_for_process(
    process: &Process,
    provider: &impl CryptoProvider,
    private_key: *const MeshSecretHandle,
    message: *const MeshBytes,
) -> Result<[u8; 64], CryptoFailure> {
    let message = unsafe { required_bytes(message, MAX_INPUT_BYTES) }?;
    with_owned_resource(
        process,
        private_key,
        ResourceKind::SigningPrivateKey,
        |private_key| {
            let private_key = <&[u8; 32]>::try_from(private_key)
                .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, private_key.len() as i64))?;
            provider
                .ed25519_sign(private_key, message)
                .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))
        },
    )
    .map_err(resource_failure)?
}

fn allocate_signature(signature: [u8; 64]) -> Result<*mut MeshSignature, CryptoFailure> {
    let bytes = mesh_bytes_new(signature.as_ptr(), signature.len() as u64);
    if bytes.is_null() {
        return Err(failure(CryptoErrorTag::InternalFailure, 0, 0));
    }
    let signature = allocate_value(MeshSignature { bytes });
    if signature.is_null() {
        return Err(failure(CryptoErrorTag::InternalFailure, 0, 0));
    }
    Ok(signature)
}

/// Sign bounded binary input with an actor-owned Ed25519 private key.
#[no_mangle]
pub extern "C" fn mesh_crypto_sign(
    private_key: *const MeshSecretHandle,
    message: *const MeshBytes,
) -> *mut MeshResult {
    let signature = match with_current_process(|process| {
        sign_for_process(process, &SystemProvider, private_key, message)
    }) {
        Ok(signature) => signature,
        Err(error) => return error_result(error),
    };
    match allocate_signature(signature) {
        Ok(signature) => ok_result(signature),
        Err(error) => error_result(error),
    }
}

unsafe fn signing_public_key_bytes(
    public_key: *const MeshSigningPublicKey,
) -> Result<[u8; 32], CryptoFailure> {
    if public_key.is_null() || (*public_key).bytes.is_null() {
        return Err(failure(CryptoErrorTag::InvalidPublicKey, 32, -1));
    }
    let bytes = (*public_key).bytes;
    if (*bytes).len != 32 {
        return Err(failure(
            CryptoErrorTag::InvalidPublicKey,
            32,
            actual_length((*bytes).len),
        ));
    }
    <[u8; 32]>::try_from((*bytes).as_slice()).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidPublicKey,
            32,
            actual_length((*bytes).len),
        )
    })
}

unsafe fn signature_bytes(signature: *const MeshSignature) -> Result<[u8; 64], CryptoFailure> {
    if signature.is_null() || (*signature).bytes.is_null() {
        return Err(failure(CryptoErrorTag::InvalidSignature, 64, -1));
    }
    let bytes = (*signature).bytes;
    if (*bytes).len != 64 {
        return Err(failure(
            CryptoErrorTag::InvalidSignature,
            64,
            actual_length((*bytes).len),
        ));
    }
    <[u8; 64]>::try_from((*bytes).as_slice()).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidSignature,
            64,
            actual_length((*bytes).len),
        )
    })
}

fn verify_with_provider(
    provider: &impl CryptoProvider,
    public_key: *const MeshSigningPublicKey,
    message: *const MeshBytes,
    signature: *const MeshSignature,
) -> Result<bool, CryptoFailure> {
    let public_key = unsafe { signing_public_key_bytes(public_key) }?;
    let message = unsafe { required_bytes(message, MAX_INPUT_BYTES) }?;
    let signature = unsafe { signature_bytes(signature) }?;
    provider
        .ed25519_verify(&public_key, message, &signature)
        .map_err(|error| match error {
            ProviderError::InvalidPublicKey => failure(CryptoErrorTag::InvalidPublicKey, 32, 32),
            _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
        })
}

/// Verify an Ed25519 signature, returning `Ok(false)` for valid-sized mismatches.
#[no_mangle]
pub extern "C" fn mesh_crypto_verify(
    public_key: *const MeshSigningPublicKey,
    message: *const MeshBytes,
    signature: *const MeshSignature,
) -> *mut MeshResult {
    match verify_with_provider(&SystemProvider, public_key, message, signature) {
        Ok(verified) => ok_result(allocate_value(verified)),
        Err(error) => error_result(error),
    }
}

fn aead_key_for_process(
    process: &mut Process,
    material: *const MeshSecretHandle,
) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    consume_and_retype_owned_resource(
        process,
        material,
        ResourceKind::SecretBytes,
        ResourceKind::AeadKey,
        |bytes| {
            if bytes.len() == 32 {
                Ok(())
            } else {
                Err(bytes.len() as i64)
            }
        },
    )
    .map_err(|error| match error {
        RetypeError::Resource(error) => resource_failure(error),
        RetypeError::Rejected {
            error: actual,
            removed,
        } => {
            drop(removed);
            failure(CryptoErrorTag::InvalidKey, 32, actual)
        }
        RetypeError::GenerationExhausted { removed } => {
            drop(removed);
            failure(CryptoErrorTag::ResourceLimitExceeded, 0, 0)
        }
    })
}

/// Consume `SecretBytes` and retype exactly 32 bytes as an AEAD key.
#[no_mangle]
pub extern "C" fn mesh_crypto_aead_key(material: *const MeshSecretHandle) -> *mut MeshResult {
    let result = alloc_result(0, ptr::null_mut());
    match with_current_process(|process| aead_key_for_process(process, material)) {
        Ok(key) => complete_result(result, key),
        Err(error) => error_result(error),
    }
}

unsafe fn aead_nonce_bytes(nonce: *const MeshBytes) -> Result<[u8; 12], CryptoFailure> {
    if nonce.is_null() {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            AEAD_NONCE_BYTES as i64,
            -1,
        ));
    }
    if (*nonce).len != AEAD_NONCE_BYTES as u64 {
        return Err(failure(
            CryptoErrorTag::InvalidLength,
            AEAD_NONCE_BYTES as i64,
            actual_length((*nonce).len),
        ));
    }
    <[u8; 12]>::try_from((*nonce).as_slice()).map_err(|_| {
        failure(
            CryptoErrorTag::InvalidLength,
            AEAD_NONCE_BYTES as i64,
            actual_length((*nonce).len),
        )
    })
}

fn aead_seal_for_process(
    process: &Process,
    provider: &impl CryptoProvider,
    key: *const MeshSecretHandle,
    nonce: *const MeshBytes,
    associated_data: *const MeshBytes,
    plaintext: *const MeshBytes,
) -> Result<Vec<u8>, CryptoFailure> {
    let nonce = unsafe { aead_nonce_bytes(nonce) }?;
    let associated_data = unsafe { required_bytes(associated_data, MAX_INPUT_BYTES) }?;
    let plaintext = unsafe { required_bytes(plaintext, MAX_INPUT_BYTES) }?;
    with_owned_resource(process, key, ResourceKind::AeadKey, |key| {
        let key = <&[u8; 32]>::try_from(key)
            .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, key.len() as i64))?;
        provider
            .chacha20poly1305_seal(key, &nonce, associated_data, plaintext)
            .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))
    })
    .map_err(resource_failure)?
}

/// Encrypt and authenticate bounded binary input with ChaCha20-Poly1305.
#[no_mangle]
pub extern "C" fn mesh_crypto_aead_seal(
    key: *const MeshSecretHandle,
    nonce: *const MeshBytes,
    associated_data: *const MeshBytes,
    plaintext: *const MeshBytes,
) -> *mut MeshResult {
    match with_current_process(|process| {
        aead_seal_for_process(
            process,
            &SystemProvider,
            key,
            nonce,
            associated_data,
            plaintext,
        )
    }) {
        Ok(ciphertext) => ok_result(mesh_bytes_new(ciphertext.as_ptr(), ciphertext.len() as u64)),
        Err(error) => error_result(error),
    }
}

fn aead_open_for_process(
    process: &Process,
    provider: &impl CryptoProvider,
    key: *const MeshSecretHandle,
    nonce: *const MeshBytes,
    associated_data: *const MeshBytes,
    ciphertext: *const MeshBytes,
) -> Result<Zeroizing<Vec<u8>>, CryptoFailure> {
    let ciphertext = unsafe { required_bytes(ciphertext, MAX_AEAD_CIPHERTEXT_BYTES) }?;
    let mut plaintext = Zeroizing::new(Vec::from(ciphertext));
    let nonce = unsafe { aead_nonce_bytes(nonce) }?;
    let associated_data = unsafe { required_bytes(associated_data, MAX_INPUT_BYTES) }?;
    let operation = with_owned_resource(process, key, ResourceKind::AeadKey, |key| {
        let key = <&[u8; 32]>::try_from(key)
            .map_err(|_| failure(CryptoErrorTag::InvalidKey, 32, key.len() as i64))?;
        provider
            .chacha20poly1305_open(key, &nonce, associated_data, &mut plaintext)
            .map_err(|error| match error {
                ProviderError::AuthenticationFailed => {
                    failure(CryptoErrorTag::AuthenticationFailed, 0, 0)
                }
                _ => failure(CryptoErrorTag::InternalFailure, 0, 0),
            })
    })
    .map_err(resource_failure)?;
    operation?;
    Ok(plaintext)
}

/// Authenticate and decrypt ChaCha20-Poly1305 ciphertext.
#[no_mangle]
pub extern "C" fn mesh_crypto_aead_open(
    key: *const MeshSecretHandle,
    nonce: *const MeshBytes,
    associated_data: *const MeshBytes,
    ciphertext: *const MeshBytes,
) -> *mut MeshResult {
    match with_current_process(|process| {
        aead_open_for_process(
            process,
            &SystemProvider,
            key,
            nonce,
            associated_data,
            ciphertext,
        )
    }) {
        Ok(plaintext) => ok_result(mesh_bytes_new(plaintext.as_ptr(), plaintext.len() as u64)),
        Err(error) => error_result(error),
    }
}

/// Crypto.hmac_sha512(key, msg) -> String
///
/// Returns the HMAC-SHA512 of `msg` keyed with `key`, as a lowercase hex string.
/// RFC 2202 test vector: hmac_sha512("Jefe", "what do ya want for nothing?")
///   = "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
#[no_mangle]
pub extern "C" fn mesh_crypto_hmac_sha512(
    key: *const MeshString,
    msg: *const MeshString,
) -> *mut MeshString {
    if key.is_null() || msg.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let k = (*key).as_str().as_bytes();
        let m = (*msg).as_str().as_bytes();
        let Ok(mut mac) = HmacSha512::new_from_slice(k) else {
            return ptr::null_mut();
        };
        mac.update(m);
        digest_hex(&mac.finalize().into_bytes())
    }
}

/// Crypto.uuid4() -> String
///
/// Generates a random RFC 4122 v4 UUID using cryptographically random bytes.
/// Format: 8-4-4-4-12 hex characters separated by hyphens (36 characters total).
/// Version nibble set to 4 (0100), variant bits set to 10xx.
///
/// Uses rand 0.9 API: rand::rng().fill_bytes() (NOT rand::thread_rng() — removed in 0.9).
#[no_mangle]
pub extern "C" fn mesh_crypto_uuid4() -> *mut MeshString {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    // RFC 4122 version 4 (0b0100) + variant 10xx (0b10xx)
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let uuid = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    );
    mesh_string_new(uuid.as_ptr(), uuid.len() as u64)
}

// ── Standard library: Base64 functions (Phase 135) ─────────────────────────

/// Base64.encode(s) -> String
///
/// Returns the standard Base64-encoded form of `s` using the RFC 4648 standard
/// alphabet with `=` padding characters. Example: encode("hello") = "aGVsbG8="
#[no_mangle]
pub extern "C" fn mesh_base64_encode(s: *const MeshString) -> *mut MeshString {
    unsafe {
        let input = (*s).as_str().as_bytes();
        let encoded = general_purpose::STANDARD.encode(input);
        mesh_string_new(encoded.as_ptr(), encoded.len() as u64)
    }
}

/// Base64.decode(s) -> Result<String, String>
///
/// Decodes a standard Base64 string. Tries padded first, then unpadded (lenient).
/// Validates that decoded bytes are valid UTF-8.
/// Returns Err("invalid base64") if decoding fails, Err("invalid utf-8") if not UTF-8.
#[no_mangle]
pub extern "C" fn mesh_base64_decode(s: *const MeshString) -> *mut MeshResult {
    unsafe {
        let text = (*s).as_str();
        let bytes = general_purpose::STANDARD
            .decode(text)
            .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(text));
        match bytes {
            Err(_) => {
                let e = "invalid base64";
                alloc_result(1, mesh_string_new(e.as_ptr(), e.len() as u64) as *mut u8)
            }
            Ok(decoded) => match std::str::from_utf8(&decoded) {
                Err(_) => {
                    let e = "invalid utf-8";
                    alloc_result(1, mesh_string_new(e.as_ptr(), e.len() as u64) as *mut u8)
                }
                Ok(valid) => alloc_result(
                    0,
                    mesh_string_new(valid.as_ptr(), valid.len() as u64) as *mut u8,
                ),
            },
        }
    }
}

/// Base64.encode_url(s) -> String
///
/// Returns the URL-safe Base64-encoded form of `s` using the RFC 4648 URL-safe
/// alphabet without padding characters. Example: encode_url("hello") = "aGVsbG8"
#[no_mangle]
pub extern "C" fn mesh_base64_encode_url(s: *const MeshString) -> *mut MeshString {
    unsafe {
        let input = (*s).as_str().as_bytes();
        let encoded = general_purpose::URL_SAFE_NO_PAD.encode(input);
        mesh_string_new(encoded.as_ptr(), encoded.len() as u64)
    }
}

/// Base64.decode_url(s) -> Result<String, String>
///
/// Decodes a URL-safe Base64 string (no padding). Validates UTF-8 after decoding.
/// Returns Err("invalid base64") if decoding fails, Err("invalid utf-8") if not UTF-8.
#[no_mangle]
pub extern "C" fn mesh_base64_decode_url(s: *const MeshString) -> *mut MeshResult {
    unsafe {
        let text = (*s).as_str();
        let bytes = general_purpose::URL_SAFE_NO_PAD.decode(text);
        match bytes {
            Err(_) => {
                let e = "invalid base64";
                alloc_result(1, mesh_string_new(e.as_ptr(), e.len() as u64) as *mut u8)
            }
            Ok(decoded) => match std::str::from_utf8(&decoded) {
                Err(_) => {
                    let e = "invalid utf-8";
                    alloc_result(1, mesh_string_new(e.as_ptr(), e.len() as u64) as *mut u8)
                }
                Ok(valid) => alloc_result(
                    0,
                    mesh_string_new(valid.as_ptr(), valid.len() as u64) as *mut u8,
                ),
            },
        }
    }
}

// ── Standard library: Hex functions (Phase 135) ────────────────────────────

/// Hex.encode(s) -> String
///
/// Returns the lowercase hexadecimal encoding of the bytes of `s`.
/// Example: encode("hi") = "6869"
#[no_mangle]
pub extern "C" fn mesh_hex_encode(s: *const MeshString) -> *mut MeshString {
    unsafe {
        let input = (*s).as_str().as_bytes();
        let hex: String = input.iter().map(|b| format!("{:02x}", b)).collect();
        mesh_string_new(hex.as_ptr(), hex.len() as u64)
    }
}

/// Hex.decode(s) -> Result<String, String>
///
/// Decodes a hex string (case-insensitive) to the string it represents.
/// Rejects odd-length strings and strings with non-hex characters.
/// Validates that decoded bytes are valid UTF-8.
/// Returns Err("invalid hex") if parsing fails, Err("invalid utf-8") if not UTF-8.
#[no_mangle]
pub extern "C" fn mesh_hex_decode(s: *const MeshString) -> *mut MeshResult {
    unsafe {
        let text = (*s).as_str().to_lowercase();
        if text.len() % 2 != 0 {
            let e = "invalid hex";
            return alloc_result(1, mesh_string_new(e.as_ptr(), e.len() as u64) as *mut u8);
        }
        let mut decoded = Vec::with_capacity(text.len() / 2);
        for chunk in text.as_bytes().chunks(2) {
            let hex_str = std::str::from_utf8(chunk).unwrap();
            match u8::from_str_radix(hex_str, 16) {
                Ok(b) => decoded.push(b),
                Err(_) => {
                    let e = "invalid hex";
                    return alloc_result(1, mesh_string_new(e.as_ptr(), e.len() as u64) as *mut u8);
                }
            }
        }
        match std::str::from_utf8(&decoded) {
            Err(_) => {
                let e = "invalid utf-8";
                alloc_result(1, mesh_string_new(e.as_ptr(), e.len() as u64) as *mut u8)
            }
            Ok(valid) => alloc_result(
                0,
                mesh_string_new(valid.as_ptr(), valid.len() as u64) as *mut u8,
            ),
        }
    }
}

/// Exercise provider-backed cryptographic boundaries without runtime handles.
#[cfg(feature = "fuzzing")]
pub fn fuzz_crypto_boundaries(data: &[u8]) {
    let input = &data[..data.len().min(MAX_INPUT_BYTES)];
    let provider = SystemProvider;
    let key = provider.sha256(input);
    let salt = provider.sha256(&key);

    let _ = provider.sha512(input);
    let mut hmac = [0; 32];
    let _ = provider.hmac_sha256(&key, input, &mut hmac);
    let mut derived = vec![0; input.first().copied().unwrap_or(31) as usize % 64 + 1];
    let _ = provider.hkdf_sha256(&key, &salt, input, &mut derived);

    if input.first().copied().unwrap_or_default() & 0x1f == 1 {
        let memory_kib = input.get(1).copied().unwrap_or(32) as u32 % 80;
        let iterations = input.get(2).copied().unwrap_or(1) as u32 % 12;
        let parallelism = input.get(3).copied().unwrap_or(1) as u32 % 10;
        let output_length = input.get(4).copied().unwrap_or(32) as usize % 80;
        let argon_salt = &input[..input.len().min(65)];
        let mut output = Zeroizing::new(vec![0xaa; output_length]);
        let _ = provider.argon2id(
            &key,
            argon_salt,
            memory_kib,
            iterations,
            parallelism,
            &mut output,
        );
    }

    let x25519_public = provider.x25519_public(&key);
    let mut shared = [0; 32];
    let _ = provider.x25519_shared(&key, &x25519_public, &mut shared);
    let _ = provider.x25519_shared(&key, &salt, &mut shared);

    let signing_public = provider.ed25519_public(&key);
    if let Ok(signature) = provider.ed25519_sign(&key, input) {
        let _ = provider.ed25519_verify(&signing_public, input, &signature);
        let arbitrary_signature = provider.sha512(input);
        let _ = provider.ed25519_verify(&salt, input, &arbitrary_signature);
    }

    let nonce: [u8; 12] = salt[..12].try_into().unwrap();
    let associated_data = &input[..input.len() / 2];
    if let Ok(ciphertext) = provider.chacha20poly1305_seal(&key, &nonce, associated_data, input) {
        let mut valid = Zeroizing::new(ciphertext);
        let _ = provider.chacha20poly1305_open(&key, &nonce, associated_data, &mut valid);
    }
    let mut arbitrary = Zeroizing::new(input.to_vec());
    let _ = provider.chacha20poly1305_open(&key, &nonce, associated_data, &mut arbitrary);

    // Keep expensive KEM/HPKE paths sparse while still giving libFuzzer a
    // stable selector it can learn to preserve.
    if input.first().copied().unwrap_or_default() & 0x1f == 0 {
        let mut entropy = [0; MLKEM_PRIVATE_SEED_BYTES];
        entropy[..32].copy_from_slice(&key);
        entropy[32..].copy_from_slice(&salt);
        let fixed = FixedProvider::with_random(&entropy);
        let seed = Zeroizing::new(entropy.to_vec().into_boxed_slice());
        if let Ok((private_seed, public_key)) = mlkem_from_seed_material(seed) {
            if let Ok((ciphertext, _)) = mlkem_encapsulate_material(&fixed, &public_key) {
                let _ = mlkem_decapsulate_material(&private_seed, &ciphertext);
            }
        }

        let info = &input[..input.len().min(MAX_HPKE_INFO_BYTES)];
        if let Ok(sealed) = hpke_seal_material(&fixed, &x25519_public, info, associated_data, input)
        {
            let _ = hpke_open_material(&fixed, &key, info, associated_data, &sealed);
        }
        let _ = hpke_open_material(&fixed, &key, info, associated_data, input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Priority, Process, ProcessId};
    use crate::bytes::{mesh_bytes_new, MeshBytes};
    use crate::crypto::provider::FixedProvider;
    use crate::gc::mesh_rt_init;
    use crate::secret::{
        destroy_owned, insert_owned_resource, with_owned_resource, ResourceError, ResourceKind,
    };
    use zeroize::Zeroizing;

    #[repr(C)]
    struct CryptoErrorLayout {
        tag: u8,
        padding: [u8; 7],
        expected: i64,
        actual: i64,
    }

    #[test]
    fn crypto_v2_public_abi_matches_64_bit_layout() {
        let _: extern "C" fn(*const MeshBytes) -> *mut MeshBytes = mesh_crypto_sha256;
        let _: extern "C" fn(*const MeshBytes) -> *mut MeshBytes = mesh_crypto_sha512;
        let _: extern "C" fn(*const MeshBytes) -> *mut MeshString = mesh_crypto_sha256_hex;
        let _: extern "C" fn(*const MeshBytes) -> *mut MeshString = mesh_crypto_sha512_hex;
        let _: extern "C" fn(i64) -> *mut MeshResult = mesh_crypto_random_bytes;
        let _: extern "C" fn(*const MeshSecretHandle, *const MeshBytes) -> *mut MeshResult =
            mesh_crypto_hmac_sha256;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshBytes,
            *const MeshBytes,
            i64,
        ) -> *mut MeshResult = mesh_crypto_hkdf_sha256;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshBytes,
            i64,
            i64,
            i64,
            i64,
        ) -> *mut MeshResult = mesh_crypto_argon2id;
        let _: extern "C" fn() -> *mut MeshResult = mesh_crypto_x25519_generate;
        let _: extern "C" fn(*const MeshBytes) -> *mut MeshResult = mesh_crypto_x25519_from_seed;
        let _: extern "C" fn(*const MeshSecretHandle) -> *mut MeshResult =
            mesh_crypto_x25519_from_secret;
        let _: extern "C" fn(*const MeshSecretHandle) -> *mut MeshResult =
            mesh_crypto_x25519_public;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshX25519PublicKey,
        ) -> *mut MeshResult = mesh_crypto_x25519_shared;
        let _: extern "C" fn(
            *const MeshX25519PublicKey,
            *const MeshBytes,
            *const MeshBytes,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_crypto_hpke_seal;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshBytes,
            *const MeshBytes,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_crypto_hpke_open;
        let _: extern "C" fn(
            *const MeshX25519PublicKey,
            *const MeshBytes,
            *const MeshBytes,
            *const MeshSecretHandle,
        ) -> *mut MeshResult = mesh_crypto_hpke_seal_secret;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshBytes,
            *const MeshBytes,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_crypto_hpke_open_secret;
        let _: extern "C" fn() -> *mut MeshResult = mesh_crypto_mlkem_generate;
        let _: extern "C" fn(*const MeshBytes) -> *mut MeshResult = mesh_crypto_mlkem_from_seed;
        let _: extern "C" fn(*const MeshSecretHandle) -> *mut MeshResult =
            mesh_crypto_mlkem_from_secret;
        let _: extern "C" fn(*const MeshMlKemPublicKey) -> *mut MeshResult =
            mesh_crypto_mlkem_encapsulate;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshMlKemCiphertext,
        ) -> *mut MeshResult = mesh_crypto_mlkem_decapsulate;
        let _: extern "C" fn() -> *mut MeshResult = mesh_crypto_signing_generate;
        let _: extern "C" fn(*const MeshBytes) -> *mut MeshResult = mesh_crypto_signing_from_seed;
        let _: extern "C" fn(*const MeshSecretHandle) -> *mut MeshResult =
            mesh_crypto_signing_from_secret;
        let _: extern "C" fn(*const MeshSecretHandle, *const MeshBytes) -> *mut MeshResult =
            mesh_crypto_sign;
        let _: extern "C" fn(
            *const MeshSigningPublicKey,
            *const MeshBytes,
            *const MeshSignature,
        ) -> *mut MeshResult = mesh_crypto_verify;
        let _: extern "C" fn(*const MeshSecretHandle) -> *mut MeshResult = mesh_crypto_aead_key;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshBytes,
            *const MeshBytes,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_crypto_aead_seal;
        let _: extern "C" fn(
            *const MeshSecretHandle,
            *const MeshBytes,
            *const MeshBytes,
            *const MeshBytes,
        ) -> *mut MeshResult = mesh_crypto_aead_open;

        assert_eq!(
            [
                std::mem::size_of::<usize>(),
                std::mem::size_of::<MeshResult>(),
                std::mem::align_of::<MeshResult>(),
                std::mem::size_of::<MeshSecretHandle>(),
                std::mem::size_of::<MeshX25519PublicKey>(),
                std::mem::size_of::<MeshX25519KeyPair>(),
                std::mem::offset_of!(MeshX25519KeyPair, public_key),
                std::mem::size_of::<MeshMlKemPublicKey>(),
                std::mem::size_of::<MeshMlKemCiphertext>(),
                std::mem::size_of::<MeshMlKemKeyPair>(),
                std::mem::offset_of!(MeshMlKemKeyPair, public_key),
                std::mem::size_of::<MeshTuple2Pointers>(),
                std::mem::offset_of!(MeshTuple2Pointers, first),
                std::mem::offset_of!(MeshTuple2Pointers, second),
                std::mem::size_of::<MeshSigningPublicKey>(),
                std::mem::size_of::<MeshSignature>(),
                std::mem::size_of::<MeshSigningKeyPair>(),
                std::mem::offset_of!(MeshSigningKeyPair, public_key),
                std::mem::size_of::<CryptoErrorLayout>(),
                std::mem::align_of::<CryptoErrorLayout>(),
                std::mem::size_of::<bool>(),
            ],
            [8, 16, 8, 12, 8, 16, 8, 8, 8, 16, 8, 24, 8, 16, 8, 8, 16, 8, 24, 8, 1,]
        );
    }

    #[test]
    fn random_bytes_rejects_negative_length_with_typed_error() {
        mesh_rt_init();

        let result = mesh_crypto_random_bytes(-1);

        unsafe {
            let error = &*((*result).value as *const CryptoErrorLayout);
            assert_eq!(
                ((*result).tag, error.tag, error.expected, error.actual),
                (
                    1,
                    CryptoErrorTag::InvalidLength as u8,
                    MAX_RANDOM_BYTES as i64,
                    -1
                )
            );
        }
    }

    #[test]
    fn random_bytes_rejects_length_above_ceiling() {
        mesh_rt_init();

        let result = mesh_crypto_random_bytes(MAX_RANDOM_BYTES as i64 + 1);

        unsafe {
            let error = &*((*result).value as *const CryptoErrorLayout);
            assert_eq!(
                (error.tag, error.expected, error.actual),
                (
                    CryptoErrorTag::InvalidLength as u8,
                    MAX_RANDOM_BYTES as i64,
                    MAX_RANDOM_BYTES as i64 + 1
                )
            );
        }
    }

    #[test]
    fn random_bytes_maps_provider_failure_to_entropy_unavailable() {
        let Err(error) = random_bytes_with_provider(&FixedProvider::entropy_failure(), 32) else {
            panic!("provider entropy failure was ignored");
        };

        assert_eq!(error.tag as u8, CryptoErrorTag::EntropyUnavailable as u8);
    }

    #[test]
    fn sha256_returns_raw_digest_bytes() {
        mesh_rt_init();
        let input = mesh_bytes_new(b"abc".as_ptr(), 3);

        let digest: *mut MeshBytes = mesh_crypto_sha256(input);

        let expected = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(unsafe { (*digest).as_slice() }, expected);
    }

    #[test]
    fn hash_hex_exports_return_lowercase_text() {
        mesh_rt_init();
        let input = mesh_bytes_new(b"abc".as_ptr(), 3);

        let sha256 = mesh_crypto_sha256_hex(input);
        let sha512 = mesh_crypto_sha512_hex(input);

        unsafe {
            assert_eq!(
                ((*sha256).as_str(), (*sha512).as_str()),
                (
                    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                    "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2\
                     192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                )
            );
        }
    }

    #[test]
    fn direct_hashes_reject_null_but_accept_input_above_the_fallible_api_bound() {
        mesh_rt_init();
        let oversized_data = vec![0u8; MAX_INPUT_BYTES + 1];
        let oversized = mesh_bytes_new(oversized_data.as_ptr(), oversized_data.len() as u64);

        assert!(mesh_crypto_sha256(ptr::null()).is_null());
        assert!(mesh_crypto_sha512(ptr::null()).is_null());
        assert_eq!(unsafe { (*mesh_crypto_sha256(oversized)).len }, 32);
        assert_eq!(unsafe { (*mesh_crypto_sha512(oversized)).len }, 64);
        assert!(!mesh_crypto_sha256_hex(oversized).is_null());
        assert!(!mesh_crypto_sha512_hex(oversized).is_null());
    }

    #[test]
    fn random_bytes_returns_requested_length() {
        mesh_rt_init();

        let result = mesh_crypto_random_bytes(32);

        unsafe {
            assert_eq!((*result).tag, 0);
            assert_eq!((*((*result).value as *const MeshBytes)).len, 32);
        }
    }

    #[test]
    fn hmac_sha256_returns_an_actor_owned_secret() {
        mesh_rt_init();
        let owner = ProcessId(70_001);
        let mut process = Process::new(owner, Priority::Normal);
        let key = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x2a; 32].into_boxed_slice()),
        )
        .expect("insert HMAC key");
        let message = mesh_bytes_new(b"message".as_ptr(), 7);

        let Ok(output) = hmac_sha256_for_process(&mut process, &SystemProvider, key, message)
        else {
            panic!("HMAC output failed");
        };
        let length = with_owned_resource(&process, output, ResourceKind::SecretBytes, |bytes| {
            bytes.len()
        })
        .expect("read HMAC output");

        assert_eq!(length, 32);
        destroy_owned(owner);
    }

    #[test]
    fn hkdf_sha256_returns_the_requested_secret_length() {
        mesh_rt_init();
        let owner = ProcessId(70_002);
        let mut process = Process::new(owner, Priority::Normal);
        let input_key = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x0b; 22].into_boxed_slice()),
        )
        .expect("insert HKDF input key");
        let salt = mesh_bytes_new(b"salt".as_ptr(), 4);
        let info = mesh_bytes_new(b"mesh-test".as_ptr(), 9);

        let Ok(output) =
            hkdf_sha256_for_process(&mut process, &SystemProvider, input_key, salt, info, 42)
        else {
            panic!("HKDF output failed");
        };
        let length = with_owned_resource(&process, output, ResourceKind::SecretBytes, |bytes| {
            bytes.len()
        })
        .expect("read HKDF output");

        assert_eq!(length, 42);
        destroy_owned(owner);
    }

    #[test]
    fn argon2id_returns_an_actor_owned_secret() {
        mesh_rt_init();
        let owner = ProcessId(70_020);
        let mut process = Process::new(owner, Priority::Normal);
        let password = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(b"password".to_vec().into_boxed_slice()),
        )
        .expect("insert Argon2id password");
        let salt = mesh_bytes_new(b"somesalt".as_ptr(), 8);

        let Ok(output) =
            argon2id_for_process(&mut process, &SystemProvider, password, salt, 32, 3, 1, 32)
        else {
            panic!("Argon2id derivation failed");
        };
        let length = with_owned_resource(&process, output, ResourceKind::SecretBytes, |bytes| {
            bytes.len()
        })
        .expect("read Argon2id output");

        assert_eq!(length, 32);
        destroy_owned(owner);
    }

    #[test]
    fn argon2id_rejects_parameters_outside_public_bounds() {
        let mut process = Process::new(ProcessId(70_021), Priority::Normal);
        let short_salt = mesh_bytes_new([0u8; 7].as_ptr(), 7);
        let long_salt = mesh_bytes_new([0u8; 65].as_ptr(), 65);
        let cases = [
            (ptr::null(), 32, 3, 0, 32, 1, 0),
            (ptr::null(), 32, 3, 9, 32, 8, 9),
            (ptr::null(), 31, 3, 4, 32, 32, 31),
            (ptr::null(), 65_537, 3, 1, 32, 65_536, 65_537),
            (ptr::null(), 32, 0, 1, 32, 1, 0),
            (ptr::null(), 32, 11, 1, 32, 10, 11),
            (ptr::null(), 32, 3, 1, 15, 16, 15),
            (ptr::null(), 32, 3, 1, 65, 64, 65),
            (short_salt, 32, 3, 1, 32, 8, 7),
            (long_salt, 32, 3, 1, 32, 64, 65),
        ];

        for (salt, memory, iterations, parallelism, output, expected, actual) in cases {
            let Err(error) = argon2id_for_process(
                &mut process,
                &SystemProvider,
                ptr::null(),
                salt,
                memory,
                iterations,
                parallelism,
                output,
            ) else {
                panic!("invalid Argon2id parameters were accepted");
            };
            assert_eq!(
                (error.tag, error.expected, error.actual),
                (CryptoErrorTag::InvalidLength, expected, actual),
            );
        }
    }

    #[test]
    fn hkdf_sha256_rejects_output_outside_rfc5869_bounds() {
        let mut process = Process::new(ProcessId(70_012), Priority::Normal);

        let Err(empty) = hkdf_sha256_for_process(
            &mut process,
            &SystemProvider,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
        ) else {
            panic!("zero-length HKDF output was accepted");
        };
        let Err(oversized) = hkdf_sha256_for_process(
            &mut process,
            &SystemProvider,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            MAX_HKDF_OUTPUT_BYTES as i64 + 1,
        ) else {
            panic!("oversized HKDF output was accepted");
        };

        assert_eq!(
            (
                empty.tag as u8,
                empty.actual,
                oversized.tag as u8,
                oversized.actual,
            ),
            (
                CryptoErrorTag::InvalidLength as u8,
                0,
                CryptoErrorTag::InvalidLength as u8,
                MAX_HKDF_OUTPUT_BYTES as i64 + 1,
            )
        );
    }

    #[test]
    fn x25519_generation_keeps_private_material_actor_owned() {
        mesh_rt_init();
        let owner = ProcessId(70_003);
        let mut process = Process::new(owner, Priority::Normal);
        let entropy = [0x42; 32];
        let provider = FixedProvider::with_random(&entropy);

        let Ok(generated) = x25519_generate_for_process(&mut process, &provider) else {
            panic!("X25519 generation failed");
        };
        let private_length = with_owned_resource(
            &process,
            generated.private_key,
            ResourceKind::X25519PrivateKey,
            |bytes| bytes.len(),
        )
        .expect("read generated private key");
        let Ok(derived_public) =
            x25519_public_for_process(&process, &SystemProvider, generated.private_key)
        else {
            panic!("derive generated X25519 public key failed");
        };

        assert_eq!((private_length, derived_public), (32, generated.public_key));
        destroy_owned(owner);
    }

    #[test]
    fn x25519_shared_agrees_for_both_keypairs() {
        mesh_rt_init();
        let owner = ProcessId(70_004);
        let mut process = Process::new(owner, Priority::Normal);
        let alice_entropy = [0x31; 32];
        let bob_entropy = [0x72; 32];
        let Ok(alice) =
            x25519_generate_for_process(&mut process, &FixedProvider::with_random(&alice_entropy))
        else {
            panic!("Alice X25519 generation failed");
        };
        let Ok(bob) =
            x25519_generate_for_process(&mut process, &FixedProvider::with_random(&bob_entropy))
        else {
            panic!("Bob X25519 generation failed");
        };
        let alice_public_bytes = mesh_bytes_new(alice.public_key.as_ptr(), 32);
        let bob_public_bytes = mesh_bytes_new(bob.public_key.as_ptr(), 32);
        let alice_public = MeshX25519PublicKey {
            bytes: alice_public_bytes,
        };
        let bob_public = MeshX25519PublicKey {
            bytes: bob_public_bytes,
        };

        let Ok(alice_shared) = x25519_shared_for_process(
            &mut process,
            &SystemProvider,
            alice.private_key,
            &bob_public,
        ) else {
            panic!("Alice shared secret failed");
        };
        let Ok(bob_shared) = x25519_shared_for_process(
            &mut process,
            &SystemProvider,
            bob.private_key,
            &alice_public,
        ) else {
            panic!("Bob shared secret failed");
        };
        let secrets_match = with_owned_resource(
            &process,
            alice_shared,
            ResourceKind::SecretBytes,
            |alice_bytes| {
                with_owned_resource(
                    &process,
                    bob_shared,
                    ResourceKind::SecretBytes,
                    |bob_bytes| alice_bytes == bob_bytes,
                )
            },
        )
        .expect("read Alice shared secret")
        .expect("read Bob shared secret");

        assert!(secrets_match);
        destroy_owned(owner);
    }

    #[test]
    fn hpke_base_mode_matches_rfc9180_a2_1_sequence_zero() {
        fn hex(value: &str) -> Vec<u8> {
            value
                .as_bytes()
                .chunks_exact(2)
                .map(|chunk| u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap())
                .collect()
        }

        let input_key_material =
            hex("909a9b35d3dc4713a5e72a4da274b55d3d3821a37e5d099e74a647db583a904b");
        let recipient_public_key: [u8; 32] =
            hex("4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a")
                .try_into()
                .unwrap();
        let recipient_private_key: [u8; 32] =
            hex("8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb")
                .try_into()
                .unwrap();
        let info = hex("4f6465206f6e2061204772656369616e2055726e");
        let plaintext = hex("4265617574792069732074727574682c20747275746820626561757479");
        let associated_data = hex("436f756e742d30");
        let expected = hex(
            "1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a\
             1c5250d8034ec2b784ba2cfd69dbdb8af406cfe3ff938e131f0def8c8b60b4db\
             21993c62ce81883d2dd1b51a28",
        );
        let provider = FixedProvider::with_random(&input_key_material);

        let Ok(sealed) = hpke_seal_material(
            &provider,
            &recipient_public_key,
            &info,
            &associated_data,
            &plaintext,
        ) else {
            panic!("RFC 9180 seal failed");
        };
        assert_eq!(sealed, expected);
        let Ok(opened) = hpke_open_material(
            &provider,
            &recipient_private_key,
            &info,
            &associated_data,
            &sealed,
        ) else {
            panic!("RFC 9180 open failed");
        };
        assert_eq!(&opened[..], plaintext);

        let mut tampered = sealed;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(matches!(
            hpke_open_material(
                &provider,
                &recipient_private_key,
                &info,
                &associated_data,
                &tampered,
            ),
            Err(CryptoFailure {
                tag: CryptoErrorTag::AuthenticationFailed,
                ..
            })
        ));
    }

    #[test]
    fn mlkem768_round_trip_keeps_seed_and_shared_secret_zeroizing() {
        let seed_entropy = [0x41; 64];
        let encapsulation_entropy = [0x73; 32];
        let Ok((private_seed, public_key)) =
            mlkem_key_material(&FixedProvider::with_random(&seed_entropy))
        else {
            panic!("ML-KEM key generation failed");
        };
        let Ok((ciphertext, sender_secret)) = mlkem_encapsulate_material(
            &FixedProvider::with_random(&encapsulation_entropy),
            &public_key,
        ) else {
            panic!("ML-KEM encapsulation failed");
        };
        let Ok(receiver_secret) = mlkem_decapsulate_material(&private_seed, &ciphertext) else {
            panic!("ML-KEM decapsulation failed");
        };

        assert_eq!(
            (
                private_seed.len(),
                public_key.len(),
                ciphertext.len(),
                sender_secret.as_ref(),
            ),
            (64, 1184, 1088, receiver_secret.as_ref())
        );
    }

    #[test]
    fn x25519_rejects_malformed_public_key_wrapper() {
        mesh_rt_init();
        let short_key = [0u8; 31];
        let public_key = MeshX25519PublicKey {
            bytes: mesh_bytes_new(short_key.as_ptr(), short_key.len() as u64),
        };

        let Err(error) = (unsafe { x25519_public_key_bytes(&public_key) }) else {
            panic!("malformed X25519 public key was accepted");
        };

        assert_eq!(
            (error.tag as u8, error.expected, error.actual),
            (CryptoErrorTag::InvalidPublicKey as u8, 32, 31)
        );
    }

    #[test]
    fn x25519_secret_constructor_matches_rfc7748_and_consumes_source() {
        mesh_rt_init();
        let owner = ProcessId(70_019);
        let mut process = Process::new(owner, Priority::Normal);
        let private_key = [
            0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2,
            0x66, 0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5,
            0x1d, 0xb9, 0x2c, 0x2a,
        ];
        let expected_public_key = [
            0x85, 0x20, 0xf0, 0x09, 0x89, 0x30, 0xa7, 0x54, 0x74, 0x8b, 0x7d, 0xdc, 0xb4, 0x3e,
            0xf7, 0x5a, 0x0d, 0xbf, 0x3a, 0x0d, 0x26, 0x38, 0x1a, 0xf4, 0xeb, 0xa4, 0xa9, 0x8e,
            0xaa, 0x9b, 0x4e, 0x6a,
        ];
        let source = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(private_key.to_vec().into_boxed_slice()),
        )
        .expect("insert X25519 material");

        let Ok((derived_private_key, public_key)) =
            x25519_from_secret_for_process(&mut process, &SystemProvider, source)
        else {
            panic!("derive X25519 keypair failed");
        };

        assert_eq!(public_key, expected_public_key);
        assert_eq!(
            with_owned_resource(&process, source, ResourceKind::SecretBytes, |_| ()),
            Err(ResourceError::StaleHandle)
        );
        assert!(with_owned_resource(
            &process,
            derived_private_key,
            ResourceKind::X25519PrivateKey,
            |_| ()
        )
        .is_ok());
        destroy_owned(owner);
    }

    #[test]
    fn secret_key_constructors_consume_sources_rejected_for_invalid_length() {
        mesh_rt_init();
        let owner = ProcessId(70_020);
        let mut process = Process::new(owner, Priority::Normal);
        let signing_source = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x51; 31].into_boxed_slice()),
        )
        .expect("insert invalid signing material");
        let signing_error =
            signing_from_secret_for_process(&mut process, &SystemProvider, signing_source)
                .expect_err("invalid signing material was accepted");
        assert_eq!(
            (
                signing_error.tag as u8,
                signing_error.expected,
                signing_error.actual,
                with_owned_resource(&process, signing_source, ResourceKind::SecretBytes, |_| (),),
            ),
            (
                CryptoErrorTag::InvalidKey as u8,
                32,
                31,
                Err(ResourceError::StaleHandle),
            )
        );

        let mlkem_source = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x52; MLKEM_PRIVATE_SEED_BYTES - 1].into_boxed_slice()),
        )
        .expect("insert invalid ML-KEM material");
        let mlkem_error = mlkem_from_secret_for_process(&mut process, mlkem_source)
            .expect_err("invalid ML-KEM material was accepted");
        assert_eq!(
            (
                mlkem_error.tag as u8,
                mlkem_error.expected,
                mlkem_error.actual,
                with_owned_resource(&process, mlkem_source, ResourceKind::SecretBytes, |_| (),),
            ),
            (
                CryptoErrorTag::InvalidKey as u8,
                MLKEM_PRIVATE_SEED_BYTES as i64,
                MLKEM_PRIVATE_SEED_BYTES as i64 - 1,
                Err(ResourceError::StaleHandle),
            )
        );
        destroy_owned(owner);
    }

    #[test]
    fn signing_round_trip_verifies() {
        mesh_rt_init();
        let owner = ProcessId(70_005);
        let mut process = Process::new(owner, Priority::Normal);
        let entropy = [0x55; 32];
        let Ok(generated) =
            signing_generate_for_process(&mut process, &FixedProvider::with_random(&entropy))
        else {
            panic!("signing key generation failed");
        };
        let public_bytes = mesh_bytes_new(generated.public_key.as_ptr(), 32);
        let public_key = MeshSigningPublicKey {
            bytes: public_bytes,
        };
        let message = mesh_bytes_new(b"signed message".as_ptr(), 14);
        let Ok(signature_bytes) =
            sign_for_process(&process, &SystemProvider, generated.private_key, message)
        else {
            panic!("signing failed");
        };
        let signature = MeshSignature {
            bytes: mesh_bytes_new(signature_bytes.as_ptr(), 64),
        };

        let Ok(verified) = verify_with_provider(&SystemProvider, &public_key, message, &signature)
        else {
            panic!("verification input should be valid");
        };

        assert!(verified);
        destroy_owned(owner);
    }

    #[test]
    fn signing_seed_requires_exactly_32_bytes() {
        mesh_rt_init();

        for seed in [&b"short"[..], &[0u8; 33][..]] {
            let input = mesh_bytes_new(seed.as_ptr(), seed.len() as u64);
            let result = mesh_crypto_signing_from_seed(input);
            let (result_tag, error) = unsafe {
                (
                    (*result).tag,
                    &*((*result).value as *const CryptoErrorLayout),
                )
            };

            assert_eq!(
                (result_tag, error.tag, error.expected, error.actual),
                (
                    1,
                    CryptoErrorTag::InvalidLength as u8,
                    32,
                    seed.len() as i64
                )
            );
        }
    }

    #[test]
    fn x25519_seed_requires_exactly_32_bytes() {
        mesh_rt_init();

        for seed in [&b"short"[..], &[0u8; 33][..]] {
            let input = mesh_bytes_new(seed.as_ptr(), seed.len() as u64);
            let result = mesh_crypto_x25519_from_seed(input);
            let (result_tag, error) = unsafe {
                (
                    (*result).tag,
                    &*((*result).value as *const CryptoErrorLayout),
                )
            };

            assert_eq!(
                (result_tag, error.tag, error.expected, error.actual),
                (
                    1,
                    CryptoErrorTag::InvalidLength as u8,
                    32,
                    seed.len() as i64
                )
            );
        }
    }

    #[test]
    fn mlkem768_rejects_noncanonical_seed_public_key_and_ciphertext_lengths() {
        mesh_rt_init();

        for seed in [&[0u8; 63][..], &[0u8; 65][..]] {
            let input = mesh_bytes_new(seed.as_ptr(), seed.len() as u64);
            let result = mesh_crypto_mlkem_from_seed(input);
            let (result_tag, error) = unsafe {
                (
                    (*result).tag,
                    &*((*result).value as *const CryptoErrorLayout),
                )
            };
            assert_eq!(
                (result_tag, error.tag, error.expected, error.actual),
                (
                    1,
                    CryptoErrorTag::InvalidLength as u8,
                    MLKEM_PRIVATE_SEED_BYTES as i64,
                    seed.len() as i64,
                )
            );
        }

        let short_public = [0u8; MLKEM_PUBLIC_KEY_BYTES - 1];
        let public_key = MeshMlKemPublicKey {
            bytes: mesh_bytes_new(short_public.as_ptr(), short_public.len() as u64),
        };
        let result = mesh_crypto_mlkem_encapsulate(&public_key);
        let error = unsafe { &*((*result).value as *const CryptoErrorLayout) };
        assert_eq!(
            (error.tag, error.expected, error.actual),
            (
                CryptoErrorTag::InvalidPublicKey as u8,
                MLKEM_PUBLIC_KEY_BYTES as i64,
                short_public.len() as i64,
            )
        );

        let short_ciphertext = [0u8; MLKEM_CIPHERTEXT_BYTES - 1];
        let ciphertext = MeshMlKemCiphertext {
            bytes: mesh_bytes_new(short_ciphertext.as_ptr(), short_ciphertext.len() as u64),
        };
        let result = mesh_crypto_mlkem_decapsulate(ptr::null(), &ciphertext);
        let error = unsafe { &*((*result).value as *const CryptoErrorLayout) };
        assert_eq!(
            (error.tag, error.expected, error.actual),
            (
                CryptoErrorTag::InvalidLength as u8,
                MLKEM_CIPHERTEXT_BYTES as i64,
                short_ciphertext.len() as i64,
            )
        );
    }

    #[test]
    fn private_key_use_rejects_a_different_actor_owner() {
        mesh_rt_init();
        let owner = ProcessId(70_010);
        let other = ProcessId(70_011);
        let mut owner_process = Process::new(owner, Priority::Normal);
        let other_process = Process::new(other, Priority::Normal);
        let entropy = [0x29; 32];
        let Ok(generated) =
            signing_generate_for_process(&mut owner_process, &FixedProvider::with_random(&entropy))
        else {
            panic!("signing key generation failed");
        };
        let message = mesh_bytes_new(b"message".as_ptr(), 7);

        let Err(error) = sign_for_process(
            &other_process,
            &SystemProvider,
            generated.private_key,
            message,
        ) else {
            panic!("foreign actor used private key");
        };
        let owner_still_has_key = with_owned_resource(
            &owner_process,
            generated.private_key,
            ResourceKind::SigningPrivateKey,
            |bytes| bytes.len() == 32,
        )
        .expect("owner key disappeared after rejected access");

        assert_eq!(
            (error.tag as u8, owner_still_has_key),
            (CryptoErrorTag::SecretDestroyed as u8, true)
        );
        destroy_owned(owner);
    }

    #[test]
    fn verify_returns_false_for_a_valid_sized_bad_signature() {
        mesh_rt_init();
        let owner = ProcessId(70_006);
        let mut process = Process::new(owner, Priority::Normal);
        let entropy = [0x37; 32];
        let Ok(generated) =
            signing_generate_for_process(&mut process, &FixedProvider::with_random(&entropy))
        else {
            panic!("signing key generation failed");
        };
        let public_key = MeshSigningPublicKey {
            bytes: mesh_bytes_new(generated.public_key.as_ptr(), 32),
        };
        let message = mesh_bytes_new(b"message".as_ptr(), 7);
        let Ok(mut signature_bytes) =
            sign_for_process(&process, &SystemProvider, generated.private_key, message)
        else {
            panic!("signing failed");
        };
        signature_bytes[0] ^= 1;
        let signature = MeshSignature {
            bytes: mesh_bytes_new(signature_bytes.as_ptr(), 64),
        };

        let Ok(verified) = verify_with_provider(&SystemProvider, &public_key, message, &signature)
        else {
            panic!("valid-sized signature should not be a typed error");
        };

        assert!(!verified);
        destroy_owned(owner);
    }

    #[test]
    fn verify_rejects_a_malformed_signature_with_typed_error() {
        mesh_rt_init();
        let public_data = [0u8; 32];
        let public_key = MeshSigningPublicKey {
            bytes: mesh_bytes_new(public_data.as_ptr(), 32),
        };
        let message = mesh_bytes_new(b"message".as_ptr(), 7);
        let short_signature = [0u8; 63];
        let signature = MeshSignature {
            bytes: mesh_bytes_new(short_signature.as_ptr(), short_signature.len() as u64),
        };

        let Err(error) = verify_with_provider(&SystemProvider, &public_key, message, &signature)
        else {
            panic!("malformed signature was not rejected");
        };

        assert_eq!(
            (error.tag as u8, error.expected, error.actual),
            (CryptoErrorTag::InvalidSignature as u8, 64, 63)
        );
    }

    #[test]
    fn aead_key_rejection_still_consumes_source_secret() {
        mesh_rt_init();
        let owner = ProcessId(70_007);
        let mut process = Process::new(owner, Priority::Normal);
        let source = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x91; 31].into_boxed_slice()),
        )
        .expect("insert invalid AEAD material");

        let Err(error) = aead_key_for_process(&mut process, source) else {
            panic!("invalid AEAD material was accepted");
        };
        let old_handle = with_owned_resource(&process, source, ResourceKind::SecretBytes, |_| ());

        assert_eq!(
            (error.tag as u8, old_handle),
            (
                CryptoErrorTag::InvalidKey as u8,
                Err(ResourceError::StaleHandle)
            )
        );
        destroy_owned(owner);
    }

    #[test]
    fn aead_seal_and_open_round_trip() {
        mesh_rt_init();
        let owner = ProcessId(70_008);
        let mut process = Process::new(owner, Priority::Normal);
        let source = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x44; 32].into_boxed_slice()),
        )
        .expect("insert AEAD material");
        let Ok(key) = aead_key_for_process(&mut process, source) else {
            panic!("valid AEAD material rejected");
        };
        let nonce_data = [0x22u8; 12];
        let nonce = mesh_bytes_new(nonce_data.as_ptr(), 12);
        let associated_data = mesh_bytes_new(b"context".as_ptr(), 7);
        let plaintext = mesh_bytes_new(b"secret plaintext".as_ptr(), 16);

        let Ok(ciphertext) = aead_seal_for_process(
            &process,
            &SystemProvider,
            key,
            nonce,
            associated_data,
            plaintext,
        ) else {
            panic!("AEAD seal failed");
        };
        let ciphertext = mesh_bytes_new(ciphertext.as_ptr(), ciphertext.len() as u64);
        let Ok(opened) = aead_open_for_process(
            &process,
            &SystemProvider,
            key,
            nonce,
            associated_data,
            ciphertext,
        ) else {
            panic!("AEAD open failed");
        };

        assert_eq!(&opened[..], b"secret plaintext");
        destroy_owned(owner);
    }

    #[test]
    fn aead_rejects_nonstandard_nonce_length_before_key_use() {
        mesh_rt_init();
        let process = Process::new(ProcessId(70_013), Priority::Normal);
        let nonce_data = [0u8; 11];
        let nonce = mesh_bytes_new(nonce_data.as_ptr(), nonce_data.len() as u64);
        let empty = mesh_bytes_new(ptr::null(), 0);

        let Err(error) =
            aead_seal_for_process(&process, &SystemProvider, ptr::null(), nonce, empty, empty)
        else {
            panic!("nonstandard AEAD nonce was accepted");
        };

        assert_eq!(
            (error.tag as u8, error.expected, error.actual),
            (CryptoErrorTag::InvalidLength as u8, 12, 11)
        );
    }

    #[test]
    fn aead_open_reports_authentication_failure_without_plaintext() {
        mesh_rt_init();
        let owner = ProcessId(70_009);
        let mut process = Process::new(owner, Priority::Normal);
        let source = insert_owned_resource(
            &mut process,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x64; 32].into_boxed_slice()),
        )
        .expect("insert AEAD material");
        let Ok(key) = aead_key_for_process(&mut process, source) else {
            panic!("valid AEAD material rejected");
        };
        let nonce_data = [0x32u8; 12];
        let nonce = mesh_bytes_new(nonce_data.as_ptr(), 12);
        let associated_data = mesh_bytes_new(b"context".as_ptr(), 7);
        let plaintext = mesh_bytes_new(b"private".as_ptr(), 7);
        let Ok(mut ciphertext) = aead_seal_for_process(
            &process,
            &SystemProvider,
            key,
            nonce,
            associated_data,
            plaintext,
        ) else {
            panic!("AEAD seal failed");
        };
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;
        let ciphertext = mesh_bytes_new(ciphertext.as_ptr(), ciphertext.len() as u64);

        let Err(error) = aead_open_for_process(
            &process,
            &SystemProvider,
            key,
            nonce,
            associated_data,
            ciphertext,
        ) else {
            panic!("tampered ciphertext released plaintext");
        };

        assert_eq!(error.tag as u8, CryptoErrorTag::AuthenticationFailed as u8);
        destroy_owned(owner);
    }
}
