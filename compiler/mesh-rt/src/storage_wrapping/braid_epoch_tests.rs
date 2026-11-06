//! Role23 fixtures use known synthetic seeds; no public seed export is added.
use super::*;

fn context() -> Vec<u8> {
    let mut c = vec![1];
    c.extend_from_slice(&[0x11; 32]);
    c.extend_from_slice(&[0x22; 16]);
    c.extend_from_slice(&[0x33; 32]);
    c.extend_from_slice(&[0x44; 32]);
    c.extend_from_slice(&23u16.to_be_bytes());
    c.extend_from_slice(&9u64.to_be_bytes());
    c
}
fn created<T>(result: *mut MeshResult) -> *mut T {
    let result = unsafe { &*result };
    assert_eq!(result.tag, 0, "expected typed storage Ok");
    result.value.cast()
}
fn bytes(value: &[u8]) -> *mut MeshBytes {
    crate::bytes::mesh_bytes_new(value.as_ptr(), value.len() as u64)
}

#[test]
fn c1_epoch_role23_real_typed_entrypoints_roundtrip_session_private_key() {
    crate::mesh_rt_init();
    crate::secret::as_test_actor(|_| {
        let p = crate::actor::current_process().unwrap();
        let seed = [0x51; 64];
        let private = insert_owned_resource(
            &mut p.lock(),
            ResourceKind::MlKemPrivateKey,
            ProtectedBytes::copy_from(&seed).unwrap(),
        )
        .unwrap();
        let key = created(mesh_storage_key_ephemeral());
        let c = bytes(&context());
        let sealed: *mut MeshBytes =
            created(mesh_mlkem_private_key_seal_for_storage(private, key, c));
        assert_eq!(unsafe { (*sealed).as_slice().len() }, 131);
        let opened = created(mesh_mlkem_private_key_unseal_from_storage(sealed, key, c));
        crate::secret::with_owned_resource(
            &p.lock(),
            opened,
            ResourceKind::MlKemPrivateKey,
            |value| assert_eq!(value, seed),
        )
        .unwrap();
    });
}

fn material() -> [u8; 36] {
    [0x31; 36]
}
fn fixture_blob(value: &[u8], c: &[u8]) -> Vec<u8> {
    let key = material();
    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(&key[32..]);
    nonce[4..].copy_from_slice(&19u64.to_be_bytes());
    let binding = context_binding(&SystemProvider, c);
    let n = value.len() as u32;
    let encrypted = SystemProvider
        .chacha20poly1305_seal(
            key[..32].try_into().unwrap(),
            &nonce,
            &associated_data(&binding, n),
            value,
        )
        .unwrap();
    let mut b = vec![1];
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&nonce);
    b.extend_from_slice(&binding);
    b.extend_from_slice(&n.to_be_bytes());
    b.extend_from_slice(&encrypted);
    b
}
fn error(blob: &[u8], c: &[u8]) -> CryptoErrorTag {
    braid_epoch::open(
        &SystemProvider,
        blob,
        &material(),
        c,
        ResourceKind::MlKemPrivateKey,
    )
    .err()
    .unwrap()
    .tag
}
#[derive(Default)]
struct ForbidLegacy(std::sync::atomic::AtomicUsize);
impl CryptoProvider for ForbidLegacy {
    fn fill_random(&self, _: &mut [u8]) -> Result<(), crate::crypto::provider::ProviderError> {
        Err(crate::crypto::provider::ProviderError::EntropyUnavailable)
    }
    fn chacha20poly1305_seal(
        &self,
        _: &[u8; 32],
        _: &[u8; 12],
        _: &[u8],
        _: &[u8],
    ) -> Result<Vec<u8>, crate::crypto::provider::ProviderError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(crate::crypto::provider::ProviderError::InternalFailure)
    }
    fn chacha20poly1305_open(
        &self,
        _: &[u8; 32],
        _: &[u8; 12],
        _: &[u8],
        _: &mut Zeroizing<Vec<u8>>,
    ) -> Result<(), crate::crypto::provider::ProviderError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(crate::crypto::provider::ProviderError::InternalFailure)
    }
}
#[test]
fn c1_epoch_role23_protected_wire_matches_known_vector_and_never_legacy_plaintext() {
    let provider = ForbidLegacy::default();
    let c = context();
    let seed = [0x51; 64];
    let sealed = braid_epoch::seal(&provider, &seed, &material(), 19, &c).unwrap();
    assert_eq!(sealed, fixture_blob(&seed, &c));
    assert_eq!(sealed.len(), 131);
    assert_eq!(
        &*braid_epoch::open(
            &provider,
            &sealed,
            &material(),
            &c,
            ResourceKind::MlKemPrivateKey
        )
        .unwrap(),
        &seed
    );
    assert_eq!(provider.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        open_value(
            &provider,
            &sealed,
            &material(),
            &c,
            ResourceKind::MlKemPrivateKey
        )
        .err()
        .unwrap()
        .tag,
        CryptoErrorTag::UnsupportedOperation
    );
}
#[test]
fn c1_epoch_role23_context_authentication_and_width_order_are_explicit() {
    let c = context();
    let b = fixture_blob(&[0x51; 64], &c);
    for offset in [0, 1, 33, 49, 81, 113, 114, 122] {
        let mut changed = c.clone();
        changed[offset] ^= 1;
        assert_eq!(error(&b, &changed), CryptoErrorTag::AuthenticationFailed);
    }
    for range in [49..81, 115..123] {
        let mut changed = c.clone();
        changed[range].fill(0);
        assert_eq!(
            error(&fixture_blob(&[0x51; 64], &changed), &changed),
            CryptoErrorTag::InvalidLength
        );
    }
    for n in [0, 122, 124] {
        let mut changed = c.clone();
        changed.resize(n, 0);
        assert!(braid_epoch::open(
            &SystemProvider,
            &fixture_blob(&[0x51; 64], &changed),
            &material(),
            &changed,
            ResourceKind::MlKemPrivateKey
        )
        .is_err());
    }
    for n in [0, 63] {
        let mut wrong = fixture_blob(&vec![0x51; n], &c);
        *wrong.last_mut().unwrap() ^= 1;
        assert_eq!(error(&wrong, &c), CryptoErrorTag::AuthenticationFailed);
        assert_eq!(
            error(&fixture_blob(&vec![0x51; n], &c), &c),
            CryptoErrorTag::InvalidLength
        );
    }
    assert_eq!(
        error(&fixture_blob(&[0; 65], &c), &c),
        CryptoErrorTag::InvalidLength
    );
    let mut trailing = b.clone();
    trailing.push(0);
    assert_eq!(error(&trailing, &c), CryptoErrorTag::InvalidLength);
    let mut wrong = b.clone();
    wrong[0] = 2;
    assert_eq!(error(&wrong, &[]), CryptoErrorTag::UnsupportedOperation);
    wrong = b.clone();
    wrong[2] = 2;
    assert_eq!(error(&wrong, &[]), CryptoErrorTag::UnsupportedOperation);
    assert_eq!(error(&b[..66], &[]), CryptoErrorTag::InvalidLength);
    assert_eq!(
        braid_epoch::open(
            &SystemProvider,
            &b,
            &material(),
            &c,
            ResourceKind::SecretBytes
        )
        .err()
        .unwrap()
        .tag,
        CryptoErrorTag::UnsupportedOperation
    );
}
#[test]
fn c1_epoch_role23_old_prekey_and_responder_contexts_do_not_change() {
    let mut old = context();
    old[113..115].copy_from_slice(&15u16.to_be_bytes());
    assert_eq!(
        validate_context(&old, ResourceKind::MlKemPrivateKey)
            .unwrap_err()
            .tag,
        CryptoErrorTag::InvalidLength
    );
    old[49..81].fill(0);
    assert_eq!(
        validate_context(&old, ResourceKind::MlKemPrivateKey).unwrap(),
        SecretPurpose::PostQuantumPrekey
    );
    let blob = seal_value(
        &SystemProvider,
        &[0x51; 64],
        &material(),
        19,
        &old,
        ResourceKind::MlKemPrivateKey,
    )
    .unwrap();
    assert_eq!(blob, fixture_blob(&[0x51; 64], &old));
    let mut added = old.clone();
    added.push(0);
    let e = validate_context(&added, ResourceKind::MlKemPrivateKey).unwrap_err();
    assert_eq!(
        (e.tag, e.expected, e.actual),
        (CryptoErrorTag::InvalidLength, 123, 124)
    );
    for phase in 0..=2 {
        let mut newer = context();
        newer[113..115].copy_from_slice(&22u16.to_be_bytes());
        newer.push(phase);
        assert_eq!(
            validate_context(&newer, ResourceKind::SecretMap).unwrap(),
            SecretPurpose::BraidResponderSecrets
        );
        assert!(validate_context(&newer, ResourceKind::MlKemPrivateKey).is_err());
    }
}
#[test]
fn c1_epoch_role23_open_failures_and_final_quota_publish_no_handle() {
    use crate::actor::{Priority, Process, ProcessId};
    use crate::secret::{
        destroy_owned, insert_test_storage_key_resource, owned_secret_count_for_test,
    };
    crate::mesh_rt_init();
    let owner = ProcessId(191_231);
    let mut p = Process::new(owner, Priority::Normal);
    let key = insert_test_storage_key_resource(
        &mut p,
        Zeroizing::new(material().to_vec().into_boxed_slice()),
        0,
    )
    .unwrap();
    let c = context();
    let good = fixture_blob(&[0x51; 64], &c);
    for bad in [good[..130].to_vec(), fixture_blob(&[0; 63], &c), {
        let mut b = good.clone();
        *b.last_mut().unwrap() ^= 1;
        b
    }] {
        assert!(open_for_process(
            &mut p,
            &SystemProvider,
            &bad,
            key,
            &c,
            ResourceKind::MlKemPrivateKey
        )
        .is_err());
        assert_eq!(owned_secret_count_for_test(owner), 1);
    }
    for _ in 1..4096 {
        insert_owned_resource(
            &mut p,
            ResourceKind::SecretBytes,
            ProtectedBytes::zeroed(0).unwrap(),
        )
        .unwrap();
    }
    let error = open_for_process(
        &mut p,
        &SystemProvider,
        &good,
        key,
        &c,
        ResourceKind::MlKemPrivateKey,
    )
    .unwrap_err();
    assert_eq!(error.tag, CryptoErrorTag::ResourceLimitExceeded);
    assert_eq!(owned_secret_count_for_test(owner), 4096);
    destroy_owned(owner);
    assert_eq!(owned_secret_count_for_test(owner), 0);
}
#[test]
fn c1_epoch_role23_actual_callback_reentrancy_rechecks_source_key_and_owner() {
    use crate::secret::{insert_managed_storage_key_resource, StorageCounterReservation};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    struct Reservation {
        next: AtomicU64,
        action: u8,
        source: usize,
        key: AtomicUsize,
    }
    impl StorageCounterReservation for Reservation {
        fn reserve(&self) -> Result<u64, StorageKeyError> {
            // Real wrapper must release process AND table locks for callback.
            let p = crate::actor::current_process().unwrap();
            assert!(p.try_lock_for(std::time::Duration::from_secs(5)).is_some());
            let next = self.next.fetch_add(1, Ordering::SeqCst);
            if self.action == 0 {
                crate::secret::mesh_resource_destroy(self.source as *mut MeshSecretHandle);
            }
            if self.action == 1 {
                crate::secret::mesh_resource_destroy(
                    self.key.load(Ordering::SeqCst) as *mut MeshSecretHandle
                );
            }
            if self.action == 2 {
                assert!(p.lock().mark_exited(crate::actor::ExitReason::Killed));
            }
            Ok(next)
        }
    }
    crate::mesh_rt_init();
    for action in 0..3 {
        crate::secret::as_test_actor(|_| {
            let p = crate::actor::current_process().unwrap();
            let h = insert_owned_resource(
                &mut p.lock(),
                ResourceKind::MlKemPrivateKey,
                ProtectedBytes::copy_from(&[0x51; 64]).unwrap(),
            )
            .unwrap();
            let source = Arc::new(Reservation {
                next: AtomicU64::new(7),
                action,
                source: h as usize,
                key: AtomicUsize::new(0),
            });
            let key = insert_managed_storage_key_resource(
                &mut p.lock(),
                Zeroizing::new(material().to_vec().into_boxed_slice()),
                source.clone(),
            )
            .unwrap();
            source.key.store(key as usize, Ordering::SeqCst);
            let result =
                seal_for_current_actor(h, key, bytes(&context()), ResourceKind::MlKemPrivateKey);
            assert_eq!(result.unwrap_err().tag, CryptoErrorTag::SecretDestroyed);
            assert_eq!(
                source.next.load(Ordering::SeqCst),
                8,
                "reserved nonce was not retained as spent"
            );
        });
    }
}
