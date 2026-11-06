//! Purposes24/25: canonical bounded public metadata and opaque protected maps.
use super::*;
use crate::secret::{
    prepare_spqr_storage_map, validate_prepared_spqr_storage_map, validate_spqr_storage_map,
    validate_spqr_storage_metadata, SpqrMapRole, SpqrStorageMetadata, SpqrStorageSlot,
};

pub(super) const MAX_CONTEXT_BYTES: usize = 1309;
const MIN_CONTEXT_BYTES: usize = 233;
const MAX_MAP_BYTES: usize = 3524;

pub(super) fn is_context(context: &[u8]) -> bool {
    matches!(
        context.get(CONTEXT_PURPOSE_OFFSET..CONTEXT_PURPOSE_OFFSET + 2),
        Some([0, 24]) | Some([0, 25])
    )
}

/// Preserve every result of the old copier; only new roles get the bounded fallback.
pub(crate) unsafe fn copy_resource_context(
    value: *const MeshBytes,
) -> Result<Vec<u8>, CryptoFailure> {
    match unsafe { braid_responder::copy_resource_context(value) } {
        Ok(context) => Ok(context),
        Err(original) => match unsafe { copy_mesh_bytes(value, MAX_CONTEXT_BYTES) } {
            Ok(context) if is_context(&context) => Ok(context),
            _ => Err(original),
        },
    }
}

fn metadata(
    context: &[u8],
    expected: StorageValueKind,
) -> Result<(SpqrMapRole, SpqrStorageMetadata), CryptoFailure> {
    if context.len() > MAX_CONTEXT_BYTES {
        return Err(invalid_length(MAX_CONTEXT_BYTES, context.len()));
    }
    if context.len() < MIN_CONTEXT_BYTES {
        return Err(invalid_length(MIN_CONTEXT_BYTES, context.len()));
    }
    if context[0] != FORMAT_VERSION {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            1,
            context[0] as i64,
        ));
    }
    let role = match u16::from_be_bytes(context[113..115].try_into().expect("fixed purpose")) {
        24 => SpqrMapRole::Chains,
        25 => SpqrMapRole::Skipped,
        id => return Err(failure(CryptoErrorTag::UnsupportedOperation, 24, id as i64)),
    };
    if expected != ResourceKind::SecretMap.into() {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            expected.code(),
            ResourceKind::SecretMap as i64,
        ));
    }
    if context[49..81].iter().all(|b| *b == 0) {
        return Err(failure(CryptoErrorTag::InvalidLength, 32, 0));
    }
    if u64::from_be_bytes(context[115..123].try_into().expect("fixed revision")) == 0 {
        return Err(failure(CryptoErrorTag::InvalidLength, 1, 0));
    }
    if context[123] != 1 {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            1,
            context[123] as i64,
        ));
    }
    let slots = usize::from(context[205]);
    if !(1..=3).contains(&slots) {
        return Err(resource_failure(crate::secret::ResourceError::WrongKind));
    }
    let slot_end = 206 + 26 * slots;
    let skipped = usize::from(
        *context
            .get(slot_end)
            .ok_or_else(|| invalid_length(slot_end + 1, context.len()))?,
    );
    if skipped > 64 {
        return Err(resource_failure(crate::secret::ResourceError::WrongKind));
    }
    let total = slot_end + 1 + 16 * skipped;
    if total != context.len() {
        return Err(invalid_length(total, context.len()));
    }
    let read = |offset| {
        u64::from_be_bytes(
            context[offset..offset + 8]
                .try_into()
                .expect("bounded metadata integer"),
        )
    };
    let mut result = SpqrStorageMetadata {
        direction: context[124],
        local_epoch: read(125),
        knowledge: read(133),
        slots: [SpqrStorageSlot::default(); 3],
        slot_count: slots as u8,
        skipped_index: [(0, 0); 64],
        skipped_count: skipped as u8,
    };
    for i in 0..slots {
        let start = 206 + 26 * i;
        if context[start + 8] > 1 || context[start + 17] > 1 {
            return Err(resource_failure(crate::secret::ResourceError::WrongKind));
        }
        result.slots[i] = SpqrStorageSlot {
            epoch: read(start),
            send_present: context[start + 8] == 1,
            send_count: read(start + 9),
            receive_present: context[start + 17] == 1,
            receive_count: read(start + 18),
        };
    }
    for i in 0..skipped {
        let start = slot_end + 1 + 16 * i;
        result.skipped_index[i] = (read(start), read(start + 8));
    }
    validate_spqr_storage_metadata(&result).map_err(resource_failure)?;
    Ok((role, result))
}

pub(super) fn validate_context(
    context: &[u8],
    expected: StorageValueKind,
) -> Result<SecretPurpose, CryptoFailure> {
    let (role, _) = metadata(context, expected)?;
    Ok(match role {
        SpqrMapRole::Chains => SecretPurpose::SpqrChains,
        SpqrMapRole::Skipped => SecretPurpose::SpqrSkippedKeys,
    })
}

pub(crate) fn seal(
    provider: &impl CryptoProvider,
    plaintext: &[u8],
    material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    counter: u64,
    context: &[u8],
) -> Result<Vec<u8>, CryptoFailure> {
    let (role, metadata) = metadata(context, ResourceKind::SecretMap.into())?;
    validate_spqr_storage_map(plaintext, role, &metadata).map_err(resource_failure)?;
    let mut working = ProtectedBytes::copy_from(plaintext)
        .map_err(crate::secret::ResourceError::from)
        .map_err(resource_failure)?;
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&material[..32]);
    let mut nonce = [0u8; NONCE_BYTES];
    nonce[..4].copy_from_slice(&material[32..]);
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    let binding = context_binding(provider, context);
    let n = u32::try_from(working.len()).expect("bounded SPQR map");
    let tag = ChaCha20Poly1305::new((&*key).into())
        .encrypt_in_place_detached((&nonce).into(), &associated_data(&binding, n), &mut working)
        .map_err(|_| failure(CryptoErrorTag::InternalFailure, 0, 0))?;
    // Only authenticated ciphertext crosses into ordinary allocation.
    let mut blob = Vec::with_capacity(FIXED_OVERHEAD_BYTES + working.len());
    blob.push(FORMAT_VERSION);
    blob.extend_from_slice(&ALGORITHM_CHACHA20_POLY1305.to_be_bytes());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&binding);
    blob.extend_from_slice(&n.to_be_bytes());
    blob.extend_from_slice(&working);
    blob.extend_from_slice(&tag);
    Ok(blob)
}

pub(crate) fn open(
    provider: &impl CryptoProvider,
    blob: &[u8],
    material: &[u8; STORAGE_KEY_MATERIAL_BYTES],
    context: &[u8],
    expected: ResourceKind,
) -> Result<ProtectedBytes, CryptoFailure> {
    // Frame/binding/tag precede context and private schema semantics. A union
    // role bound caps unauthenticated staging without changing binding priority.
    if blob.len() < FIXED_OVERHEAD_BYTES {
        return Err(invalid_length(FIXED_OVERHEAD_BYTES, blob.len()));
    }
    if blob[0] != FORMAT_VERSION {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            1,
            blob[0] as i64,
        ));
    }
    let algorithm = u16::from_be_bytes([blob[1], blob[2]]);
    if algorithm != ALGORITHM_CHACHA20_POLY1305 {
        return Err(failure(
            CryptoErrorTag::UnsupportedOperation,
            1,
            algorithm as i64,
        ));
    }
    let n = u32::from_be_bytes(blob[47..51].try_into().expect("fixed blob length")) as usize;
    if n > MAX_MAP_BYTES {
        return Err(invalid_length(MAX_MAP_BYTES, n));
    }
    let total = FIXED_OVERHEAD_BYTES + n;
    if blob.len() != total {
        return Err(invalid_length(total, blob.len()));
    }
    let binding = context_binding(provider, context);
    let supplied: &[u8; 32] = blob[15..47].try_into().expect("fixed binding");
    if supplied.ct_eq(&binding).unwrap_u8() != 1 {
        return Err(failure(CryptoErrorTag::AuthenticationFailed, 0, 0));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&material[..32]);
    let nonce: &[u8; 12] = blob[3..15].try_into().expect("fixed nonce");
    let end = 51 + n;
    let mut plaintext = ProtectedBytes::copy_from(&blob[51..end])
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
    let (role, metadata) = metadata(context, expected.into())?;
    validate_spqr_storage_map(&plaintext, role, &metadata).map_err(resource_failure)?;
    Ok(plaintext)
}

pub(super) fn seal_for_current_actor(
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: &[u8],
    expected: ResourceKind,
) -> Result<Vec<u8>, CryptoFailure> {
    seal_for_owner(current_process()?, secret, wrapping_key, context, expected)
}

/// The production actor and private callback controls exercise this same path.
pub(crate) fn seal_for_owner(
    process: Arc<Mutex<Process>>,
    secret: *const MeshSecretHandle,
    wrapping_key: *const MeshSecretHandle,
    context: &[u8],
    expected: ResourceKind,
) -> Result<Vec<u8>, CryptoFailure> {
    let (role, metadata) = metadata(context, expected.into())?;
    let mut preparation = {
        let mut p = process.lock();
        let secret =
            prepare_spqr_storage_map(&mut p, secret, role, &metadata).map_err(resource_failure)?;
        let storage_key =
            prepare_storage_key_resource(&p, wrapping_key).map_err(storage_key_failure)?;
        SealPreparation {
            secret,
            storage_key,
        }
    };
    // No process/table lock survives the actual host nonce reservation.
    let counter = preparation
        .storage_key
        .reserve_counter()
        .map_err(storage_key_failure)?;
    {
        let p = process.lock();
        commit_reserved_seal(&p, &preparation, counter, expected)?;
        // SPQR_POST_RESERVATION_CONTENT_GUARD
        validate_prepared_spqr_storage_map(&p, &preparation.secret).map_err(resource_failure)?;
    }
    #[cfg(test)]
    test_hooks::before();
    let blob = seal(
        &SystemProvider,
        &preparation.secret.bytes,
        storage_key_material(&preparation.storage_key.material),
        counter,
        context,
    )?;
    #[cfg(test)]
    test_hooks::after();
    {
        let p = process.lock();
        revalidate_seal_inputs(&p, &preparation, expected)?;
        // SPQR_FINAL_CONTENT_GUARD
        validate_prepared_spqr_storage_map(&p, &preparation.secret).map_err(resource_failure)?;
    }
    Ok(blob)
}

#[cfg(test)]
mod test_hooks {
    use std::cell::RefCell;
    struct Hooks {
        before: Box<dyn FnMut()>,
        after: Box<dyn FnMut()>,
    }
    thread_local! {static HOOKS:RefCell<Option<Hooks>>=const{RefCell::new(None)};}
    struct Restore(Option<Hooks>);
    impl Drop for Restore {
        fn drop(&mut self) {
            HOOKS.with(|h| *h.borrow_mut() = self.0.take());
        }
    }
    pub(super) fn with<R>(
        before: impl FnMut() + 'static,
        after: impl FnMut() + 'static,
        run: impl FnOnce() -> R,
    ) -> R {
        let prior = HOOKS.with(|h| {
            h.borrow_mut().replace(Hooks {
                before: Box::new(before),
                after: Box::new(after),
            })
        });
        let _restore = Restore(prior);
        run()
    }
    pub(super) fn before() {
        HOOKS.with(|h| {
            if let Some(h) = h.borrow_mut().as_mut() {
                (h.before)();
            }
        });
    }
    pub(super) fn after() {
        HOOKS.with(|h| {
            if let Some(h) = h.borrow_mut().as_mut() {
                (h.after)();
            }
        });
    }
}
#[cfg(test)]
pub(crate) fn with_test_hooks<R>(
    before: impl FnMut() + 'static,
    after: impl FnMut() + 'static,
    run: impl FnOnce() -> R,
) -> R {
    test_hooks::with(before, after, run)
}
