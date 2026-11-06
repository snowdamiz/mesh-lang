//! Role 22 only. Legacy roles keep their exact context and temporary-buffer paths.
use super::*;
const ROLE_CONTEXT_BYTES: usize = CONTEXT_BYTES + 1;
const PHASE_OFFSET: usize = CONTEXT_BYTES;

pub(super) fn is_context(context: &[u8]) -> bool {
    context.get(CONTEXT_PURPOSE_OFFSET..CONTEXT_PURPOSE_OFFSET + 2)
        == Some(&22u16.to_be_bytes()[..])
}

pub(super) fn validate_context(
    context: &[u8],
    expected: StorageValueKind,
) -> Result<SecretPurpose, CryptoFailure> {
    if context.len() != ROLE_CONTEXT_BYTES {
        return Err(invalid_length(ROLE_CONTEXT_BYTES, context.len()));
    }
    if context[0] != FORMAT_VERSION {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            FORMAT_VERSION as i64,
            context[0] as i64,
        ));
    }
    if !is_context(context) {
        return Err(failure(CryptoErrorTag::UnsupportedOperation, 22, 0));
    }
    if expected != ResourceKind::SecretMap.into() {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            expected.code(),
            ResourceKind::SecretMap as i64,
        ));
    }
    if u64::from_be_bytes(
        context[CONTEXT_SNAPSHOT_OFFSET..CONTEXT_BYTES]
            .try_into()
            .expect("fixed revision"),
    ) == 0
    {
        return Err(failure(CryptoErrorTag::InvalidLength, 1, 0));
    }
    if context[CONTEXT_SESSION_OFFSET..CONTEXT_SESSION_END]
        .iter()
        .all(|b| *b == 0)
    {
        return Err(failure(CryptoErrorTag::InvalidLength, 32, 0));
    }
    if context[PHASE_OFFSET] > 2 {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            2,
            context[PHASE_OFFSET] as i64,
        ));
    }
    Ok(SecretPurpose::BraidResponderSecrets)
}

/// Retain every legacy maximum-length error. Only exact role-22/124 is added.
pub(super) unsafe fn copy_resource_context(
    value: *const MeshBytes,
) -> Result<Vec<u8>, CryptoFailure> {
    match unsafe { copy_mesh_bytes(value, CONTEXT_BYTES) } {
        Ok(context) => Ok(context),
        Err(original) => match unsafe { copy_mesh_bytes(value, ROLE_CONTEXT_BYTES) } {
            Ok(context) if context.len() == ROLE_CONTEXT_BYTES && is_context(&context) => {
                Ok(context)
            }
            _ => Err(original),
        },
    }
}

fn validate_map(plaintext: &[u8], context: &[u8]) -> Result<(), CryptoFailure> {
    crate::secret::validate_braid_responder_map(plaintext, context[PHASE_OFFSET])
        .map_err(resource_failure)
}

pub(super) fn seal(
    provider: &impl CryptoProvider,
    plaintext: &[u8],
    material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    counter: u64,
    context: &[u8],
) -> Result<Vec<u8>, CryptoFailure> {
    validate_context(context, ResourceKind::SecretMap.into())?;
    validate_map(plaintext, context)?;
    let mut working = ProtectedBytes::copy_from(plaintext)
        .map_err(crate::secret::ResourceError::from)
        .map_err(resource_failure)?;
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&material[..32]);
    let mut nonce = [0u8; NONCE_BYTES];
    nonce[..4].copy_from_slice(&material[32..]);
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    let binding = context_binding(provider, context);
    let n = u32::try_from(working.len()).expect("fixed bounded map");
    let tag = ChaCha20Poly1305::new((&*key).into())
        .encrypt_in_place_detached((&nonce).into(), &associated_data(&binding, n), &mut working)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    // The ordinary output contains authenticated ciphertext only.
    let mut b = Vec::with_capacity(FIXED_OVERHEAD_BYTES + working.len());
    b.push(FORMAT_VERSION);
    b.extend_from_slice(&ALGORITHM_CHACHA20_POLY1305.to_be_bytes());
    b.extend_from_slice(&nonce);
    b.extend_from_slice(&binding);
    b.extend_from_slice(&n.to_be_bytes());
    b.extend_from_slice(&working);
    b.extend_from_slice(&tag);
    Ok(b)
}

pub(super) fn open(
    provider: &impl CryptoProvider,
    blob: &[u8],
    material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    context: &[u8],
    expected: ResourceKind,
) -> Result<ProtectedBytes, CryptoFailure> {
    // Public framing, binding and authentication precede private/phase semantics.
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
    if n > 131 {
        return Err(invalid_length(131, n));
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
    validate_context(context, expected.into())?;
    validate_map(&plaintext, context)?;
    Ok(plaintext)
}

pub(super) fn seal_for_current_actor(
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: &[u8],
    expected: ResourceKind,
) -> Result<Vec<u8>, CryptoFailure> {
    validate_context(context, expected.into())?;
    let process = current_process()?;
    let mut preparation = {
        let mut p = process.lock();
        let secret =
            crate::secret::prepare_braid_responder_map(&mut p, secret).map_err(resource_failure)?;
        validate_map(&secret.bytes, context)?;
        let storage_key =
            prepare_storage_key_resource(&p, wrapping_key).map_err(storage_key_failure)?;
        SealPreparation {
            secret,
            storage_key,
        }
    };
    // No process/table guard survives a host counter callback or AEAD operation.
    let counter = preparation
        .storage_key
        .reserve_counter()
        .map_err(storage_key_failure)?;
    commit_reserved_seal(&process.lock(), &preparation, counter, expected)?;
    crate::secret::validate_prepared_braid_map(&process.lock(), &preparation.secret)
        .map_err(resource_failure)?;
    let blob = seal(
        &SystemProvider,
        &preparation.secret.bytes,
        storage_key_material(&preparation.storage_key.material),
        counter,
        context,
    )?;
    revalidate_seal_inputs(&process.lock(), &preparation, expected)?;
    crate::secret::validate_prepared_braid_map(&process.lock(), &preparation.secret)
        .map_err(resource_failure)?;
    Ok(blob)
}
