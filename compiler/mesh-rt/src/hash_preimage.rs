//! Random-only SHA256 preimages intended for explicit protocol publication.
//! This resource cannot import a key, caller bytes, or another secret resource.

use crate::actor::Process;
use crate::crypto::provider::{CryptoProvider, SystemProvider};
use crate::crypto::{
    bytes_value, crypto_result, failure, provider_failure, resource_failure, CryptoFailure,
};
use crate::io::MeshResult;
use crate::secret::{
    consume_owned_resource, insert_owned_resource, with_owned_resource, CryptoErrorTag,
    MeshSecretHandle, ResourceKind,
};
use zeroize::Zeroizing;

fn generate(process: &mut Process) -> Result<*mut MeshSecretHandle, CryptoFailure> {
    let mut bytes = Zeroizing::new(vec![0; 32].into_boxed_slice());
    SystemProvider
        .fill_random(&mut bytes)
        .map_err(provider_failure)?;
    insert_owned_resource(process, ResourceKind::HashPreimage, bytes).map_err(resource_failure)
}

fn commitment(
    process: &Process,
    value: *const MeshSecretHandle,
) -> Result<[u8; 32], CryptoFailure> {
    with_owned_resource(process, value, ResourceKind::HashPreimage, |bytes| {
        if bytes.len() != 32 {
            return Err(failure(
                CryptoErrorTag::InvalidLength,
                32,
                bytes.len() as i64,
            ));
        }
        Ok(SystemProvider.sha256(bytes))
    })
    .map_err(resource_failure)?
}

fn reveal(process: &Process, value: *const MeshSecretHandle) -> Result<[u8; 32], CryptoFailure> {
    let bytes = consume_owned_resource(process, value, ResourceKind::HashPreimage)
        .map_err(resource_failure)?;
    bytes
        .as_ref()
        .try_into()
        .map_err(|_| failure(CryptoErrorTag::InvalidLength, 32, bytes.len() as i64))
}

fn current<R>(
    operation: impl FnOnce(&mut Process) -> Result<R, CryptoFailure>,
) -> Result<R, CryptoFailure> {
    let process = crate::actor::current_process()
        .ok_or_else(|| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let result = operation(&mut process.lock());
    result
}

#[no_mangle]
pub extern "C" fn mesh_hash_preimage_generate() -> *mut MeshResult {
    crypto_result(current(generate))
}

#[no_mangle]
pub extern "C" fn mesh_hash_preimage_commitment(value: *const MeshSecretHandle) -> *mut MeshResult {
    crypto_result(current(|process| commitment(process, value)).map(|bytes| bytes_value(&bytes)))
}

/// Explicit protocol disclosure. The caller must durably authorize settlement
/// before revealing; restoring a sealed copy intentionally permits recovery.
#[no_mangle]
pub extern "C" fn mesh_hash_preimage_reveal(value: *const MeshSecretHandle) -> *mut MeshResult {
    crypto_result(current(|process| reveal(process, value)).map(|bytes| bytes_value(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{Priority, ProcessId};
    use crate::secret::{destroy_owned, insert_owned_resource, ResourceKind};
    use sha2::{Digest, Sha256};
    use zeroize::Zeroizing;

    #[test]
    fn random_commitments_reveal_once_and_preserve_other_secret_kinds() {
        let mut owner = Process::new(ProcessId(81_501), Priority::Normal);
        let other = Process::new(ProcessId(81_502), Priority::Normal);
        let first = generate(&mut owner).unwrap();
        let second = generate(&mut owner).unwrap();
        let digest = commitment(&owner, first).unwrap();
        assert_ne!(digest, commitment(&owner, second).unwrap());
        assert!(commitment(&other, first).is_err());
        assert!(reveal(&other, first).is_err());
        assert_eq!(commitment(&owner, first).unwrap(), digest);
        let public = reveal(&owner, first).unwrap();
        assert_eq!(Sha256::digest(public).as_slice(), digest);
        assert!(commitment(&owner, first).is_err());
        assert!(reveal(&owner, first).is_err());
        let secret = insert_owned_resource(
            &mut owner,
            ResourceKind::SecretBytes,
            Zeroizing::new(vec![0x59; 32].into_boxed_slice()),
        )
        .unwrap();
        assert!(commitment(&owner, secret).is_err());
        assert!(reveal(&owner, secret).is_err());
        destroy_owned(owner.pid);
        assert!(reveal(&owner, second).is_err());
    }
}
