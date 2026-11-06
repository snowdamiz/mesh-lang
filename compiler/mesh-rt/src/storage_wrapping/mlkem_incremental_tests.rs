//! C1 synthetic storage fixtures; no ordinary-byte continuation API exists.
use super::*;
use crate::crypto::provider::{FixedProvider, ProviderError};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct ForbidLegacyPlaintextBuffer {
    calls: AtomicUsize,
}
impl CryptoProvider for ForbidLegacyPlaintextBuffer {
    fn fill_random(&self, _: &mut [u8]) -> Result<(), ProviderError> {
        Err(ProviderError::EntropyUnavailable)
    }
    fn chacha20poly1305_seal(
        &self,
        _: &[u8; 32],
        _: &[u8; 12],
        _: &[u8],
        _: &[u8],
    ) -> Result<Vec<u8>, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::InternalFailure)
    }
    fn chacha20poly1305_open(
        &self,
        _: &[u8; 32],
        _: &[u8; 12],
        _: &[u8],
        _: &mut Zeroizing<Vec<u8>>,
    ) -> Result<(), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::InternalFailure)
    }
}
fn material() -> [u8; 36] {
    [0x31; 36]
}
fn context() -> Vec<u8> {
    let mut out = vec![1];
    out.extend_from_slice(&[0x11; 32]);
    out.extend_from_slice(&[0x22; 16]);
    out.extend_from_slice(&[0x33; 32]);
    out.extend_from_slice(&[0x44; 32]);
    out.extend_from_slice(&21u16.to_be_bytes());
    out.extend_from_slice(&9u64.to_be_bytes());
    out
}
fn continuation() -> ProtectedBytes {
    let (header, _) = crate::crypto::mlkem_incremental::parts(&[0x69; 64]);
    crate::crypto::mlkem_incremental::begin(&FixedProvider::with_random(&[0x73; 32]), &header)
        .unwrap()
        .state
        .into_protected()
}
// Independent baseline encoder using the existing provider; used only for
// public synthetic fixtures, including authenticated malformed private records.
fn legacy_fixture_blob(private: &[u8], context: &[u8]) -> Vec<u8> {
    let key = material();
    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(&key[32..]);
    nonce[4..].copy_from_slice(&19u64.to_be_bytes());
    let binding = context_binding(&SystemProvider, context);
    let length = private.len() as u32;
    let cipher = SystemProvider
        .chacha20poly1305_seal(
            key[..32].try_into().unwrap(),
            &nonce,
            &associated_data(&binding, length),
            private,
        )
        .unwrap();
    let mut out = vec![FORMAT_VERSION];
    out.extend_from_slice(&ALGORITHM_CHACHA20_POLY1305.to_be_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&binding);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&cipher);
    out
}
fn tag(blob: &[u8], context: &[u8]) -> CryptoErrorTag {
    open_continuation_value(&SystemProvider, blob, &material(), context)
        .err()
        .unwrap()
        .tag
}
#[test]
fn c1_seal_never_uses_legacy_plaintext_vec_aead() {
    let provider = ForbidLegacyPlaintextBuffer::default();
    let private = continuation();
    let actual = seal_continuation_value(&provider, &private, &material(), 19, &context());
    assert!(
        actual.is_ok(),
        "protected detached seal must not call the legacy plaintext-buffer AEAD: {:?}",
        actual.err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    // Same existing frame/AAD/nonce/key and exact bytes, not a new storage format.
    let expected = legacy_fixture_blob(&private, &context());
    let actual = seal_continuation_value(&provider, &private, &material(), 19, &context()).unwrap();
    assert_eq!(actual, expected);
}
#[test]
fn c1_open_never_uses_legacy_plaintext_vec_aead() {
    let provider = ForbidLegacyPlaintextBuffer::default();
    let private = continuation();
    let blob = legacy_fixture_blob(&private, &context());
    let actual = open_continuation_value(&provider, &blob, &material(), &context());
    assert!(
        actual.is_ok(),
        "protected detached open must not call the legacy plaintext-buffer AEAD: {:?}",
        actual.err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let restored = open_continuation_value(&provider, &blob, &material(), &context()).unwrap();
    assert_eq!(&*restored, &*private);
}

#[test]
fn c1_authenticated_storage_restores_exact_finish_and_binds_every_context_component() {
    use crate::crypto::mlkem_incremental::{begin, finish, parts, EncapsulationState};
    let (header, vector) = parts(&[0x69; 64]);
    let first = begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap();
    let expected = finish(&first.state, &vector).unwrap();
    let private = first.state.into_protected();
    let blob =
        seal_continuation_value(&SystemProvider, &private, &material(), 19, &context()).unwrap();
    drop(private);
    let restored = EncapsulationState::from_protected(
        open_continuation_value(&SystemProvider, &blob, &material(), &context()).unwrap(),
    )
    .unwrap();
    assert_eq!(finish(&restored, &vector).unwrap(), expected);
    for offset in [0, 1, 33, 49, 81, 113, 122] {
        let mut changed = context();
        changed[offset] ^= 1;
        assert_eq!(tag(&blob, &changed), CryptoErrorTag::AuthenticationFailed);
    }
    let mut key = material();
    key[0] ^= 1;
    assert_eq!(
        open_continuation_value(&SystemProvider, &blob, &key, &context())
            .err()
            .unwrap()
            .tag,
        CryptoErrorTag::AuthenticationFailed
    );
    for offset in [BLOB_NONCE_OFFSET, BLOB_CIPHERTEXT_OFFSET, blob.len() - 1] {
        let mut changed = blob.clone();
        changed[offset] ^= 1;
        assert_eq!(
            tag(&changed, &context()),
            CryptoErrorTag::AuthenticationFailed
        );
    }
    // The generic plaintext-returning path must never accept this new kind.
    assert_eq!(
        open_value(
            &SystemProvider,
            &blob,
            &material(),
            &context(),
            ResourceKind::MlKemEncapsulationState
        )
        .err()
        .unwrap()
        .tag,
        CryptoErrorTag::UnsupportedOperation
    );
}

#[test]
fn c1_storage_retains_public_authentication_material_context_error_priority() {
    let private = continuation();
    let blob = legacy_fixture_blob(&private, &context());
    let mut short = vec![9; FIXED_OVERHEAD_BYTES - 1];
    assert_eq!(tag(&short, &[]), CryptoErrorTag::InvalidLength);
    short = blob.clone();
    short[0] = 9;
    assert_eq!(tag(&short, &[]), CryptoErrorTag::UnsupportedOperation);
    short = blob.clone();
    short[BLOB_ALGORITHM_OFFSET + 1] = 9;
    assert_eq!(tag(&short, &[]), CryptoErrorTag::UnsupportedOperation);
    for mut changed in [blob[..blob.len() - 1].to_vec(), blob.clone()] {
        if changed.len() == blob.len() {
            changed.push(0);
        }
        assert_eq!(tag(&changed, &[]), CryptoErrorTag::InvalidLength);
    }
    let mut over = blob.clone();
    over[BLOB_LENGTH_OFFSET..BLOB_CIPHERTEXT_OFFSET].copy_from_slice(&65537u32.to_be_bytes());
    assert_eq!(tag(&over, &[]), CryptoErrorTag::InvalidLength);
    // Binding precedes auth; auth precedes interpreting private format/context.
    let mut bad = private.to_vec();
    bad[0] ^= 1;
    let valid_bad_format = legacy_fixture_blob(&bad, &context());
    assert_eq!(
        tag(&valid_bad_format, &context()),
        CryptoErrorTag::InvalidKey
    );
    let mut bad_tag = valid_bad_format.clone();
    *bad_tag.last_mut().unwrap() ^= 1;
    assert_eq!(
        tag(&bad_tag, &context()),
        CryptoErrorTag::AuthenticationFailed
    );
    for length in [0, 99, 101] {
        assert_eq!(
            tag(
                &legacy_fixture_blob(&vec![0; length], &context()),
                &context()
            ),
            CryptoErrorTag::InvalidLength
        );
    }
    let mut wrong_purpose = context();
    wrong_purpose[113..115].copy_from_slice(&1u16.to_be_bytes());
    assert_eq!(
        tag(
            &legacy_fixture_blob(&private, &wrong_purpose),
            &wrong_purpose
        ),
        CryptoErrorTag::UnsupportedOperation
    );
    assert_eq!(
        tag(&legacy_fixture_blob(&bad, &wrong_purpose), &wrong_purpose),
        CryptoErrorTag::InvalidKey
    );
    let mut zero_session = context();
    zero_session[49..81].fill(0);
    assert_eq!(
        tag(&legacy_fixture_blob(&private, &zero_session), &zero_session),
        CryptoErrorTag::InvalidLength
    );
    let mut zero_snapshot = context();
    zero_snapshot[115..123].fill(0);
    assert_eq!(
        tag(
            &legacy_fixture_blob(&private, &zero_snapshot),
            &zero_snapshot
        ),
        CryptoErrorTag::InvalidLength
    );
}

#[test]
fn c1_failed_unseal_publishes_no_actor_resource_and_valid_kind_restores() {
    use crate::actor::{Priority, Process, ProcessId};
    use crate::secret::{
        destroy_owned, insert_test_storage_key_resource, owned_secret_count_for_test,
        with_owned_resource,
    };
    let owner = ProcessId(191_121);
    let mut process = Process::new(owner, Priority::Normal);
    let key = insert_test_storage_key_resource(
        &mut process,
        Zeroizing::new(material().to_vec().into_boxed_slice()),
        0,
    )
    .unwrap();
    assert_eq!(owned_secret_count_for_test(owner), 1);
    let private = continuation();
    let mut invalid = private.to_vec();
    invalid[0] ^= 1;
    let malformed = legacy_fixture_blob(&invalid, &context());
    let blob = legacy_fixture_blob(&private, &context());
    for candidate in [&malformed[..], &blob[..blob.len() - 1]] {
        assert!(open_for_process(
            &mut process,
            &SystemProvider,
            candidate,
            key,
            &context(),
            ResourceKind::MlKemEncapsulationState
        )
        .is_err());
        assert_eq!(owned_secret_count_for_test(owner), 1);
    }
    let handle = open_for_process(
        &mut process,
        &SystemProvider,
        &blob,
        key,
        &context(),
        ResourceKind::MlKemEncapsulationState,
    )
    .unwrap();
    assert_eq!(owned_secret_count_for_test(owner), 2);
    with_owned_resource(
        &process,
        handle,
        ResourceKind::MlKemEncapsulationState,
        |bytes| assert_eq!(bytes, &*private),
    )
    .unwrap();
    assert!(with_owned_resource(&process, handle, ResourceKind::SecretBytes, |_| ()).is_err());
    let foreign = Process::new(ProcessId(191_122), Priority::Normal);
    assert!(with_owned_resource(
        &foreign,
        handle,
        ResourceKind::MlKemEncapsulationState,
        |_| ()
    )
    .is_err());
    destroy_owned(owner);
    assert_eq!(owned_secret_count_for_test(owner), 0);
}

#[test]
#[cfg(unix)]
fn c1_allocation_refusal_preserves_borrowed_state_and_retry_without_unlocked_fallback() {
    const CHILD: &str = "MESH_C1_PROTECTED_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use crate::crypto::mlkem_incremental::{begin, finish, parts};
        let (header, vector) = parts(&[0x69; 64]);
        let first = begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap();
        let expected = finish(&first.state, &vector).unwrap();
        let private = first.state.into_protected();
        drop(first.shared);
        let blob = legacy_fixture_blob(&private, &context());
        // One retained 112-aligned-byte state and the exact remainder fill the
        // already locked 64KiB slab; a new mapping must now genuinely fail.
        let blocker = ProtectedBytes::zeroed(65536 - 112).unwrap();
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &limit) }, 0);
        let state =
            crate::crypto::mlkem_incremental::EncapsulationState::from_protected(private).unwrap();
        assert_eq!(
            finish(&state, &vector).err().unwrap().tag,
            CryptoErrorTag::InternalFailure
        );
        assert_eq!(
            open_continuation_value(&SystemProvider, &blob, &material(), &context())
                .err()
                .unwrap()
                .tag,
            CryptoErrorTag::InternalFailure
        );
        let private = state.into_protected();
        assert_eq!(
            seal_continuation_value(&SystemProvider, &private, &material(), 19, &context())
                .err()
                .unwrap()
                .tag,
            CryptoErrorTag::InternalFailure
        );
        assert_eq!(
            begin(&FixedProvider::with_random(&[0x73; 32]), &header)
                .err()
                .unwrap()
                .tag,
            CryptoErrorTag::InternalFailure
        );
        drop(blocker);
        let state =
            crate::crypto::mlkem_incremental::EncapsulationState::from_protected(private).unwrap();
        assert_eq!(finish(&state, &vector).unwrap(), expected);
        eprintln!("C1_PROTECTED_REFUSAL_AND_RETRY_CHECKED");
        return;
    }
    let name = format!(
        "{}::c1_allocation_refusal_preserves_borrowed_state_and_retry_without_unlocked_fallback",
        module_path!().split_once("::").unwrap().1
    );
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        child.status.success()
            && String::from_utf8_lossy(&child.stderr)
                .contains("C1_PROTECTED_REFUSAL_AND_RETRY_CHECKED"),
        "child failed: {} {}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}

#[test]
fn c1_removed_state_burns_reserved_counter_and_next_seal_uses_fresh_nonce() {
    use crate::actor::{Priority, Process, ProcessId};
    use crate::secret::{consume_owned_resource, destroy_owned, insert_test_storage_key_resource};
    let owner = ProcessId(191_145);
    let mut process = Process::new(owner, Priority::Normal);
    let kind = ResourceKind::MlKemEncapsulationState;
    let secret = insert_owned_resource(&mut process, kind, continuation()).unwrap();
    let key = insert_test_storage_key_resource(
        &mut process,
        Zeroizing::new(material().to_vec().into_boxed_slice()),
        5,
    )
    .unwrap();
    let error = seal_for_process_with_hook(
        &mut process,
        &SystemProvider,
        secret,
        key,
        &context(),
        kind,
        |process| drop(consume_owned_resource(process, secret, kind).unwrap()),
    )
    .err()
    .unwrap();
    assert_eq!(error.tag, CryptoErrorTag::SecretDestroyed);
    let replacement = insert_owned_resource(&mut process, kind, continuation()).unwrap();
    let blob = seal_for_process_with_hook(
        &mut process,
        &SystemProvider,
        replacement,
        key,
        &context(),
        kind,
        |_| {},
    )
    .unwrap();
    assert_eq!(
        &blob[BLOB_NONCE_OFFSET + 4..BLOB_BINDING_OFFSET],
        &6u64.to_be_bytes()
    );
    assert!(open_for_process(&mut process, &SystemProvider, &blob, key, &context(), kind).is_ok());
    destroy_owned(owner);
}
