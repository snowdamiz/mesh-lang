//! Checked runtime ABI only; no app protocol, key import or private Bytes export.
use super::provider::SystemProvider;
use super::{
    allocate_value, bytes_value, crypto_result, exact_bytes, failure, mlkem_incremental,
    resource_failure, with_current_process, CryptoErrorTag, CryptoFailure,
};
use crate::{
    bytes::MeshBytes,
    io::MeshResult,
    secret::{
        insert_owned_mlkem_encapsulation, insert_owned_resource, prepare_owned_resource,
        validate_prepared_owned_resource, with_owned_resource, MeshSecretHandle, ResourceKind,
    },
};
use std::ptr;
macro_rules! public_part {
    ($name:ident) => {
        #[repr(C)]
        pub struct $name {
            pub bytes: *mut MeshBytes,
        }
    };
}
public_part!(MeshMlKemHeader64);
public_part!(MeshMlKemPublicVector1152);
public_part!(MeshMlKemCiphertext1);
public_part!(MeshMlKemCiphertext2);
#[repr(C)]
pub(crate) struct PartsTuple {
    pub len: u64,
    pub header: *mut MeshBytes,
    pub vector: *mut MeshBytes,
}
#[repr(C)]
pub(crate) struct FirstTuple {
    pub len: u64,
    pub state: *mut MeshSecretHandle,
    pub ciphertext: *mut MeshBytes,
    pub shared: *mut MeshSecretHandle,
}
// Every public wrapper has exactly one Bytes field. The caller supplies a valid
// wrapper address as in existing Crypto V2; null is refused before dereference.
unsafe fn part<'a, const N: usize>(
    wrapper: *const *mut MeshBytes,
) -> Result<&'a [u8; N], CryptoFailure> {
    if wrapper.is_null() {
        return Err(failure(CryptoErrorTag::InvalidLength, N as i64, -1));
    }
    unsafe { exact_bytes(*wrapper, CryptoErrorTag::InvalidLength) }
}
fn seed(bytes: &[u8]) -> Result<&[u8; 64], CryptoFailure> {
    bytes
        .try_into()
        .map_err(|_| failure(CryptoErrorTag::InvalidKey, 64, bytes.len() as i64))
}
/// Validate only public wrappers; success is the established null unit payload.
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_validate_public_parts(
    header: *const MeshMlKemHeader64,
    vector: *const MeshMlKemPublicVector1152,
) -> *mut MeshResult {
    crypto_result((|| {
        let header = unsafe { part::<64>(header.cast()) }?;
        let vector = unsafe { part::<1152>(vector.cast()) }?;
        mlkem_incremental::validate_public_parts(header, vector)?;
        Ok(ptr::null_mut::<()>())
    })())
}
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_public_parts(key: *const MeshSecretHandle) -> *mut MeshResult {
    crypto_result((|| {
        let (header, vector) = with_current_process(|process| {
            with_owned_resource(process, key, ResourceKind::MlKemPrivateKey, |bytes| {
                seed(bytes).map(mlkem_incremental::parts)
            })
            .map_err(resource_failure)?
        })?;
        Ok(allocate_value(PartsTuple {
            len: 2,
            header: bytes_value(&header),
            vector: bytes_value(&vector),
        }))
    })())
}
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_encapsulate1(
    header: *const MeshMlKemHeader64,
) -> *mut MeshResult {
    crypto_result((|| {
        let header = unsafe { part::<64>(header.cast()) }?;
        // Resolve a live actor before entropy; private intermediates have no handles.
        with_current_process(|_| Ok(()))?;
        let first = mlkem_incremental::begin(&SystemProvider, header)?;
        let out = allocate_value(FirstTuple {
            len: 3,
            state: ptr::null_mut(),
            ciphertext: bytes_value(&first.ciphertext),
            shared: ptr::null_mut(),
        });
        let (state, shared) = with_current_process(move |process| {
            insert_owned_mlkem_encapsulation(process, first.state, first.shared)
                .map_err(resource_failure)
        })?;
        unsafe {
            (*out).state = state;
            (*out).shared = shared;
        }
        Ok(out)
    })())
}
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_encapsulate2(
    state: *const MeshSecretHandle,
    vector: *const MeshMlKemPublicVector1152,
) -> *mut MeshResult {
    crypto_result((|| {
        let vector = unsafe { part::<1152>(vector.cast()) }?;
        let prepared = with_current_process(|process| {
            prepare_owned_resource(process, state, ResourceKind::MlKemEncapsulationState)
                .map_err(resource_failure)
        })?;
        // Prepared ownership stays protected; finish never consumes the table entry.
        // Retain its generation token for validation before returning public output.
        let value = mlkem_incremental::finish_material(&prepared.bytes, vector)?;
        with_current_process(|process| {
            validate_prepared_owned_resource(
                process,
                &prepared,
                ResourceKind::MlKemEncapsulationState,
            )
            .map_err(resource_failure)
        })?;
        Ok(allocate_value(MeshMlKemCiphertext2 {
            bytes: bytes_value(&value),
        }))
    })())
}
#[no_mangle]
pub extern "C" fn mesh_crypto_mlkem_decapsulate_parts(
    key: *const MeshSecretHandle,
    first: *const MeshMlKemCiphertext1,
    second: *const MeshMlKemCiphertext2,
) -> *mut MeshResult {
    crypto_result((|| {
        let first = unsafe { part::<960>(first.cast()) }?;
        let second = unsafe { part::<128>(second.cast()) }?;
        with_current_process(|process| {
            let shared =
                with_owned_resource(process, key, ResourceKind::MlKemPrivateKey, |bytes| {
                    mlkem_incremental::decapsulate_parts(seed(bytes)?, first, second)
                })
                .map_err(resource_failure)??;
            insert_owned_resource(process, ResourceKind::SecretBytes, shared)
                .map_err(resource_failure)
        })
    })())
}
#[cfg(test)]
#[path = "mlkem_incremental_ffi_tests.rs"]
mod tests;
