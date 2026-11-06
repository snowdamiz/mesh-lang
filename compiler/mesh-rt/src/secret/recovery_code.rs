//! Deliberate user-held recovery disclosure, restricted to newly generated material.
//! No operation takes an existing key and returns its representation.
use super::*;
use crate::string::{mesh_string_new, MeshString};

const DOMAIN: &[u8] = b"mesh/recovery-code/v1";
const CODE_BYTES: usize = 85;
const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[repr(C)]
pub(super) struct RecoveryCodeTuple {
    pub(super) len: u64,
    pub(super) secret: *mut MeshSecretHandle,
    pub(super) code: *mut MeshString,
}

fn invalid_code() -> CryptoFailure {
    // Neither the text nor a secret-dependent diagnostic is published.
    failure(CryptoErrorTag::InvalidKey, 0, 0)
}

fn checksum(secret: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut input = Zeroizing::new([0u8; DOMAIN.len() + 32]);
    input[..DOMAIN.len()].copy_from_slice(DOMAIN);
    input[DOMAIN.len()..].copy_from_slice(secret);
    Zeroizing::new(SystemProvider.sha256(&input[..]))
}

fn fresh_material(
    fill: impl FnOnce(&mut [u8]) -> Result<(), CryptoFailure>,
) -> Result<(ProtectedBytes, Zeroizing<[u8; CODE_BYTES]>), CryptoFailure> {
    let mut secret = ProtectedBytes::zeroed(32)
        .map_err(ResourceError::from)
        .map_err(resource_failure)?;
    fill(&mut secret)?;
    let check = checksum(&secret);
    let mut display = Zeroizing::new([b'-'; CODE_BYTES]);
    display[..4].copy_from_slice(b"MRC1");
    for (i, byte) in secret.iter().chain(check[..4].iter()).enumerate() {
        let offset = 5 + (i / 4) * 9 + (i % 4) * 2;
        display[offset] = HEX[(byte >> 4) as usize];
        display[offset + 1] = HEX[(byte & 15) as usize];
    }
    Ok((secret, display))
}

fn nibble(value: u8) -> Result<u8, CryptoFailure> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(invalid_code()),
    }
}

fn parse_material(code: &[u8]) -> Result<ProtectedBytes, CryptoFailure> {
    if code.len() != CODE_BYTES || &code[..5] != b"MRC1-" {
        return Err(invalid_code());
    }
    for offset in (13..CODE_BYTES).step_by(9) {
        if code[offset] != b'-' {
            return Err(invalid_code());
        }
    }
    let mut secret = ProtectedBytes::zeroed(32)
        .map_err(ResourceError::from)
        .map_err(resource_failure)?;
    let mut supplied = Zeroizing::new([0u8; 4]);
    for i in 0..36 {
        let offset = 5 + (i / 4) * 9 + (i % 4) * 2;
        let byte = (nibble(code[offset])? << 4) | nibble(code[offset + 1])?;
        if i < 32 {
            secret[i] = byte;
        } else {
            supplied[i - 32] = byte;
        }
    }
    let expected = checksum(&secret);
    let mismatch = supplied
        .iter()
        .zip(expected.iter())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b));
    if mismatch != 0 {
        return Err(invalid_code());
    }
    Ok(secret)
}

/// Create a new recovery secret and its one-time application display value.
/// The caller must never log, persist or send the deliberately disclosed String.
#[no_mangle]
pub extern "C" fn mesh_recovery_code_generate() -> *mut MeshResult {
    let Some(process) = crate::actor::current_process() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let result = (|| {
        live_owner(&process.lock()).map_err(resource_failure)?;
        // Protected allocation and entropy acquisition hold neither owner nor table lock.
        let (secret, display) =
            fresh_material(|out| SystemProvider.fill_random(out).map_err(provider_failure))?;
        let handle = {
            let mut process = process.lock();
            insert_owned_resource(&mut process, ResourceKind::SecretBytes, secret)
                .map_err(resource_failure)?
        };
        // Publishing GC results must not recursively acquire the owner lock.
        let code = mesh_string_new(display.as_ptr(), CODE_BYTES as u64);
        let tuple = mesh_gc_alloc_actor(
            mem::size_of::<RecoveryCodeTuple>() as u64,
            mem::align_of::<RecoveryCodeTuple>() as u64,
        ) as *mut RecoveryCodeTuple;
        unsafe {
            tuple.write(RecoveryCodeTuple {
                len: 2,
                secret: handle,
                code,
            });
        }
        Ok(tuple)
    })();
    crypto_result(result)
}

/// Import only the canonical, bounded v1 user-held recovery representation.
#[no_mangle]
pub extern "C" fn mesh_recovery_code_parse(code: *const MeshString) -> *mut MeshResult {
    let Some(process) = crate::actor::current_process() else {
        return crypto_error(CryptoErrorTag::InternalFailure, 0, 0);
    };
    let result = (|| {
        live_owner(&process.lock()).map_err(resource_failure)?;
        if code.is_null() || unsafe { (*code).len } != CODE_BYTES as u64 {
            return Err(invalid_code());
        }
        let secret = parse_material(unsafe { (*code).as_bytes() })?;
        let mut process = process.lock();
        insert_owned_resource(&mut process, ResourceKind::SecretBytes, secret)
            .map_err(resource_failure)
    })();
    crypto_result(result)
}

#[cfg(test)]
#[path = "recovery_code_tests.rs"]
mod tests;
