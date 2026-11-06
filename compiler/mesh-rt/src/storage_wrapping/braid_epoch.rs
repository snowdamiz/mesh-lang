//! Purpose23 only: opaque session-bound64-byte ML-KEM seed, protected staging.
//! Existing prekey15 and responder22 grammars/provider paths remain distinct.
use super::*;

pub(super) fn is_context(context: &[u8]) -> bool {
    context.get(CONTEXT_PURPOSE_OFFSET..CONTEXT_PURPOSE_OFFSET + 2)
        == Some(&23u16.to_be_bytes()[..])
}
fn typed_context(context: &[u8], expected: ResourceKind) -> Result<(), CryptoFailure> {
    if validate_context(context, expected)? != SecretPurpose::BraidEpochPrivateKey {
        return Err(failure(CryptoErrorTag::UnsupportedOperation, 23, 0));
    }
    Ok(())
}
pub(super) fn seal(
    provider: &impl CryptoProvider,
    plaintext: &[u8],
    material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    counter: u64,
    context: &[u8],
) -> Result<Vec<u8>, CryptoFailure> {
    typed_context(context, ResourceKind::MlKemPrivateKey)?;
    validate_material(ResourceKind::MlKemPrivateKey.into(), plaintext)?;
    let mut working = ProtectedBytes::copy_from(plaintext)
        .map_err(crate::secret::ResourceError::from)
        .map_err(resource_failure)?;
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&material[..32]);
    let mut nonce = [0u8; NONCE_BYTES];
    nonce[..4].copy_from_slice(&material[32..]);
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    let binding = context_binding(provider, context);
    let n = MLKEM_PRIVATE_SEED_BYTES as u32;
    let tag = ChaCha20Poly1305::new((&*key).into())
        .encrypt_in_place_detached((&nonce).into(), &associated_data(&binding, n), &mut working)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    let mut blob = Vec::with_capacity(FIXED_OVERHEAD_BYTES + MLKEM_PRIVATE_SEED_BYTES);
    blob.push(FORMAT_VERSION);
    blob.extend_from_slice(&ALGORITHM_CHACHA20_POLY1305.to_be_bytes());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&binding);
    blob.extend_from_slice(&n.to_be_bytes());
    blob.extend_from_slice(&working);
    blob.extend_from_slice(&tag);
    Ok(blob)
}
pub(super) fn open(
    provider: &impl CryptoProvider,
    blob: &[u8],
    material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    context: &[u8],
    expected: ResourceKind,
) -> Result<ProtectedBytes, CryptoFailure> {
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
    let algorithm = u16::from_be_bytes([blob[1], blob[2]]);
    if algorithm != ALGORITHM_CHACHA20_POLY1305 {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            ALGORITHM_CHACHA20_POLY1305 as i64,
            algorithm as i64,
        ));
    }
    let n = u32::from_be_bytes(
        blob[BLOB_LENGTH_OFFSET..BLOB_CIPHERTEXT_OFFSET]
            .try_into()
            .expect("fixed length"),
    ) as usize;
    if n > MLKEM_PRIVATE_SEED_BYTES {
        return Err(invalid_length(MLKEM_PRIVATE_SEED_BYTES, n));
    }
    let total = FIXED_OVERHEAD_BYTES + n;
    if blob.len() != total {
        return Err(invalid_length(total, blob.len()));
    }
    let binding = context_binding(provider, context);
    let supplied: &[u8; BINDING_BYTES] = blob[BLOB_BINDING_OFFSET..BLOB_LENGTH_OFFSET]
        .try_into()
        .expect("fixed binding");
    if supplied.ct_eq(&binding).unwrap_u8() != 1 {
        return Err(failure(CryptoErrorTag::AuthenticationFailed, 0, 0));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&material[..32]);
    let nonce: &[u8; NONCE_BYTES] = blob[BLOB_NONCE_OFFSET..BLOB_BINDING_OFFSET]
        .try_into()
        .expect("fixed nonce");
    let end = BLOB_CIPHERTEXT_OFFSET + n;
    let mut plaintext = ProtectedBytes::copy_from(&blob[BLOB_CIPHERTEXT_OFFSET..end])
        .map_err(crate::secret::ResourceError::from)
        .map_err(resource_failure)?;
    ChaCha20Poly1305::new((&*key).into())
        .decrypt_in_place_detached(
            nonce.into(),
            &associated_data(&binding, n as u32),
            &mut plaintext,
            Tag::from_slice(&blob[end..]),
        )
        .map_err(|_| failure(CryptoErrorTag::AuthenticationFailed, 0, 0))?;
    typed_context(context, expected)?;
    validate_material(ResourceKind::MlKemPrivateKey.into(), &plaintext)?;
    Ok(plaintext)
}
pub(super) fn seal_for_current_actor(
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: &[u8],
    expected: ResourceKind,
) -> Result<Vec<u8>, CryptoFailure> {
    typed_context(context, expected)?;
    let process = current_process()?;
    let mut preparation = {
        let mut p = process.lock();
        let secret =
            crate::secret::prepare_braid_epoch_key(&mut p, secret).map_err(resource_failure)?;
        let storage_key =
            prepare_storage_key_resource(&p, wrapping_key).map_err(storage_key_failure)?;
        SealPreparation {
            secret,
            storage_key,
        }
    };
    let counter = preparation
        .storage_key
        .reserve_counter()
        .map_err(storage_key_failure)?;
    commit_reserved_seal(&process.lock(), &preparation, counter, expected)?;
    crate::secret::validate_prepared_braid_epoch_key(&process.lock(), &preparation.secret)
        .map_err(resource_failure)?;
    let blob = seal(
        &SystemProvider,
        &preparation.secret.bytes,
        storage_key_material(&preparation.storage_key.material),
        counter,
        context,
    )?;
    revalidate_seal_inputs(&process.lock(), &preparation, expected)?;
    crate::secret::validate_prepared_braid_epoch_key(&process.lock(), &preparation.secret)
        .map_err(resource_failure)?;
    Ok(blob)
}
