//! Bounded private continuation for libcrux's incremental ML-KEM-768.
//!
//! This is a KEM adapter, not SPQR, an authenticated session, or a scheme proof.
//! Persist only encapsulation randomness plus its exact public header, not
//! libcrux's non-standard unpacked polynomial format. Every owned workspace is
//! protected. Upstream by-value internal stack/register copies retain C6/C7's
//! explicit provider-temporary limitations.
use super::provider::CryptoProvider;
use super::{
    failure, mlkem_backend, mlkem_decapsulate_material, provider_failure, resource_failure,
    CryptoErrorTag, CryptoFailure,
};
use crate::protected_memory::ProtectedBytes;
use crate::secret::ResourceError;
use libcrux_ml_kem::mlkem768::incremental;
use zeroize::Zeroizing;

pub(crate) const WORKSPACE_BYTES: usize = incremental::encaps_state_len();
pub(crate) const CONTINUATION_BYTES: usize = 4 + 64 + 32;
const FORMAT: &[u8; 4] = b"\x01MKE";
const _: () = assert!(WORKSPACE_BYTES == 2080);
const _: () = assert!(incremental::pk1_len() == 64);
const _: () = assert!(incremental::pk2_len() == 1152);

// No Clone, Debug, Deref, ordinary-byte exporter or public byte constructor.
pub(crate) struct EncapsulationState {
    material: ProtectedBytes,
}
impl EncapsulationState {
    pub(crate) fn len(&self) -> usize {
        self.material.len()
    }

    pub(crate) fn from_protected(material: ProtectedBytes) -> Result<Self, CryptoFailure> {
        validate_continuation(&material)?;
        Ok(Self { material })
    }

    pub(crate) fn into_protected(self) -> ProtectedBytes {
        self.material
    }
}

pub(crate) fn validate_continuation(material: &[u8]) -> Result<(), CryptoFailure> {
    if material.len() != CONTINUATION_BYTES {
        return Err(wrong_length(CONTINUATION_BYTES, material.len()));
    }
    if &material[..4] != FORMAT {
        return Err(failure(CryptoErrorTag::InvalidKey, 0, 0));
    }
    Ok(())
}
pub(crate) struct FirstEncapsulation {
    pub(crate) state: EncapsulationState,
    pub(crate) ciphertext: [u8; 960],
    pub(crate) shared: ProtectedBytes,
}

fn protected(length: usize) -> Result<ProtectedBytes, CryptoFailure> {
    ProtectedBytes::zeroed(length)
        .map_err(ResourceError::from)
        .map_err(resource_failure)
}
fn wrong_length(expected: usize, actual: usize) -> CryptoFailure {
    failure(
        CryptoErrorTag::InvalidLength,
        expected as i64,
        actual as i64,
    )
}

pub(crate) fn parts(seed: &[u8; 64]) -> ([u8; 64], [u8; 1152]) {
    mlkem_backend::public_parts(seed)
}

pub(crate) fn begin(
    provider: &impl CryptoProvider,
    header: &[u8],
) -> Result<FirstEncapsulation, CryptoFailure> {
    if header.len() != 64 {
        return Err(wrong_length(64, header.len()));
    }
    let mut material = protected(CONTINUATION_BYTES)?;
    material[..4].copy_from_slice(FORMAT);
    material[4..68].copy_from_slice(header);
    provider
        .fill_random(&mut material[68..100])
        .map_err(provider_failure)?;
    let randomness =
        Zeroizing::new(<[u8; 32]>::try_from(&material[68..100]).expect("checked continuation"));
    let mut workspace = protected(WORKSPACE_BYTES)?;
    let mut shared = protected(32)?;
    let ciphertext = incremental::encapsulate1(header, *randomness, &mut workspace, &mut shared)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    // workspace and the adapter's by-value seed copy wipe at scope exit.
    Ok(FirstEncapsulation {
        state: EncapsulationState { material },
        ciphertext: ciphertext.value,
        shared,
    })
}

/// Public-only canonical/binding validation. No entropy or private material.
pub(crate) fn validate_public_parts(header: &[u8], vector: &[u8]) -> Result<(), CryptoFailure> {
    if header.len() != 64 {
        return Err(wrong_length(64, header.len()));
    }
    if vector.len() != 1152 {
        return Err(wrong_length(1152, vector.len()));
    }
    incremental::validate_pk_bytes(header, vector)
        .map_err(|_| failure(CryptoErrorTag::InvalidPublicKey, 0, 0))
}

pub(crate) fn finish(
    state: &EncapsulationState,
    vector: &[u8],
) -> Result<[u8; 128], CryptoFailure> {
    finish_material(&state.material, vector)
}

// Trusted runtime-only borrow of a protected prepared resource; never a Mesh
// byte constructor/exporter. Its table generation token stays with the caller.
pub(super) fn finish_material(material: &[u8], vector: &[u8]) -> Result<[u8; 128], CryptoFailure> {
    if vector.len() != 1152 {
        return Err(wrong_length(1152, vector.len()));
    }
    if material.len() != CONTINUATION_BYTES || &material[..4] != FORMAT {
        return Err(failure(CryptoErrorTag::InvalidKey, 0, 0));
    }
    let header = &material[4..68];
    // This checks both H(vector || seed) and canonical FIPS203 coefficients.
    // No private state is mutated and no entropy is requested on any finish.
    validate_public_parts(header, vector)?;
    let randomness =
        Zeroizing::new(<[u8; 32]>::try_from(&material[68..100]).expect("checked continuation"));
    let mut workspace = protected(WORKSPACE_BYTES)?;
    let mut shared = protected(32)?;
    let _ct1 = incremental::encapsulate1(header, *randomness, &mut workspace, &mut shared)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let workspace =
        <&[u8; WORKSPACE_BYTES]>::try_from(&*workspace).expect("exact protected workspace");
    let vector = <&[u8; 1152]>::try_from(vector).expect("checked public vector");
    Ok(incremental::encapsulate2(workspace, vector).value)
}

pub(crate) fn decapsulate_parts(
    seed: &[u8; 64],
    first: &[u8],
    second: &[u8],
) -> Result<ProtectedBytes, CryptoFailure> {
    if first.len() != 960 {
        return Err(wrong_length(960, first.len()));
    }
    if second.len() != 128 {
        return Err(wrong_length(128, second.len()));
    }
    let mut ciphertext = [0u8; 1088];
    ciphertext[..960].copy_from_slice(first);
    ciphertext[960..].copy_from_slice(second);
    // Preserve full-KEM implicit rejection; never return ciphertext validity.
    let shared = mlkem_decapsulate_material(seed, &ciphertext);
    ProtectedBytes::copy_from(shared.as_ref())
        .map_err(ResourceError::from)
        .map_err(resource_failure)
}

#[cfg(test)]
#[path = "mlkem_incremental_tests.rs"]
mod tests;
