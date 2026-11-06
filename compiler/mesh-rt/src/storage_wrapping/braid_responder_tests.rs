//! Synthetic role-22 storage inputs. No secret export is added by these tests.
use super::braid_responder::{open, seal};
use super::*;
use crate::crypto::provider::ProviderError;
use std::sync::atomic::{AtomicUsize, Ordering};
fn material() -> [u8; 36] {
    [0x31; 36]
}
fn context(phase: u8) -> Vec<u8> {
    let mut c = vec![1];
    c.extend_from_slice(&[0x11; 32]);
    c.extend_from_slice(&[0x22; 16]);
    c.extend_from_slice(&[0x33; 32]);
    c.extend_from_slice(&[0x44; 32]);
    c.extend_from_slice(&22u16.to_be_bytes());
    c.extend_from_slice(&9u64.to_be_bytes());
    c.push(phase);
    c
}
fn map(phase: u8) -> Vec<u8> {
    let names: &[&[u8]] = if phase == 2 {
        &[b"root", b"mac", b"output"]
    } else {
        &[b"root", b"mac"]
    };
    let mut v = Vec::new();
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&(names.len() as u16).to_be_bytes());
    for (i, n) in names.iter().enumerate() {
        v.extend_from_slice(&(n.len() as u16).to_be_bytes());
        v.extend_from_slice(&32u32.to_be_bytes());
        v.extend_from_slice(n);
        v.extend_from_slice(&[i as u8 + 1; 32]);
    }
    v
}
// The baseline encoder deliberately permits authenticated malformed fixtures.
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
fn tag(blob: &[u8], c: &[u8]) -> CryptoErrorTag {
    open(
        &SystemProvider,
        blob,
        &material(),
        c,
        ResourceKind::SecretMap,
    )
    .err()
    .unwrap()
    .tag
}
#[derive(Default)]
struct ForbidLegacy(AtomicUsize);
impl CryptoProvider for ForbidLegacy {
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
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::InternalFailure)
    }
    fn chacha20poly1305_open(
        &self,
        _: &[u8; 32],
        _: &[u8; 12],
        _: &[u8],
        _: &mut Zeroizing<Vec<u8>>,
    ) -> Result<(), ProviderError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::InternalFailure)
    }
}
#[test]
fn c1_braid_role22_all_three_phases_use_protected_detached_storage() {
    let p = ForbidLegacy::default();
    for phase in 0..=2 {
        let private = map(phase);
        assert_eq!(private.len(), if phase == 2 { 131 } else { 87 });
        let blob = seal(&p, &private, &material(), 19, &context(phase)).unwrap();
        assert_eq!(blob, fixture_blob(&private, &context(phase)));
        assert_eq!(
            &*open(
                &p,
                &blob,
                &material(),
                &context(phase),
                ResourceKind::SecretMap
            )
            .unwrap(),
            &private
        );
    }
    assert_eq!(p.0.load(Ordering::SeqCst), 0);
}
#[test]
fn c1_braid_role22_fixed_schema_refuses_authenticated_malformed_maps() {
    for phase in 0..=2 {
        let c = context(phase);
        let good = map(phase);
        let mut malformed = vec![vec![], good[..good.len() - 1].to_vec(), {
            let mut v = good.clone();
            v.push(0);
            v
        }];
        for (offset, byte) in [(1, 4), (3, 1), (5, 0), (9, 31), (10, b'x')] {
            let mut v = good.clone();
            v[offset] = byte;
            malformed.push(v);
        }
        // A duplicate root, unknown key, wrong width, wrong capacity/count and trailing bytes all refuse.
        let mut duplicate = good[..4].to_vec();
        duplicate[2..4].copy_from_slice(&2u16.to_be_bytes());
        duplicate.extend_from_slice(&good[4..46]);
        duplicate.extend_from_slice(&good[4..46]);
        malformed.push(duplicate);
        malformed.push(map(if phase == 2 { 0 } else { 2 }));
        for value in malformed {
            assert!(open(
                &SystemProvider,
                &fixture_blob(&value, &c),
                &material(),
                &c,
                ResourceKind::SecretMap
            )
            .is_err());
            assert!(seal(&SystemProvider, &value, &material(), 19, &c).is_err());
        }
    }
    // Slot order is not an extra private semantic: an exact alternate order is accepted.
    let original = map(0);
    let mut reordered = original[..4].to_vec();
    reordered.extend_from_slice(&original[46..]);
    reordered.extend_from_slice(&original[4..46]);
    assert!(seal(&SystemProvider, &reordered, &material(), 19, &context(0)).is_ok());
}
#[test]
fn c1_braid_role22_context_and_error_order_are_explicit() {
    let c = context(2);
    let b = fixture_blob(&map(2), &c);
    for offset in [0, 1, 33, 49, 81, 113, 122, 123] {
        let mut changed = c.clone();
        changed[offset] ^= 1;
        assert_eq!(tag(&b, &changed), CryptoErrorTag::AuthenticationFailed);
    }
    for phase in [3, 255] {
        let c = context(phase);
        assert_eq!(
            tag(&fixture_blob(&map(0), &c), &c),
            CryptoErrorTag::UnsupportedOperation
        );
    }
    for range in [49..81, 115..123] {
        let mut c = context(0);
        c[range].fill(0);
        assert_eq!(
            tag(&fixture_blob(&map(0), &c), &c),
            CryptoErrorTag::InvalidLength
        );
    }
    for n in [0, 122, 123, 125] {
        let mut c = context(0);
        c.resize(n, 0);
        assert!(open(
            &SystemProvider,
            &fixture_blob(&map(0), &c),
            &material(),
            &c,
            ResourceKind::SecretMap
        )
        .is_err());
    }
    let mut malformed = fixture_blob(b"invalid", &c);
    *malformed.last_mut().unwrap() ^= 1;
    assert_eq!(tag(&malformed, &c), CryptoErrorTag::AuthenticationFailed);
    assert_eq!(
        tag(&fixture_blob(b"invalid", &c), &c),
        CryptoErrorTag::InvalidKey
    );
    let mut short = b[..66].to_vec();
    assert_eq!(tag(&short, &[]), CryptoErrorTag::InvalidLength);
    short = b.clone();
    short[0] = 9;
    assert_eq!(tag(&short, &[]), CryptoErrorTag::UnsupportedOperation);
    short = b.clone();
    short.push(0);
    assert_eq!(tag(&short, &[]), CryptoErrorTag::InvalidLength);
    assert!(open(
        &SystemProvider,
        &b,
        &material(),
        &c,
        ResourceKind::SecretBytes
    )
    .is_err());
}
#[test]
fn c1_braid_role22_legacy_context_and_continuation_remain_distinct() {
    for purpose in 1u16..=21 {
        let mut old = context(0);
        old.truncate(123);
        old[113..115].copy_from_slice(&purpose.to_be_bytes());
        let kind = SecretPurpose::from_id(purpose).unwrap().value_kind();
        let original = validate_context(&old, kind);
        let mut extra = old.clone();
        extra.push(0);
        let e = validate_context(&extra, kind).unwrap_err();
        assert_eq!(e.tag, CryptoErrorTag::InvalidLength);
        assert_eq!((e.expected, e.actual), (123, 124));
        // The old semantic outcome is not changed by routing the new role.
        if purpose == 12 || purpose == 21 {
            assert!(original.is_ok());
        }
    }
    let c = context(0);
    let b = fixture_blob(&map(0), &c);
    assert_eq!(
        open_value(
            &SystemProvider,
            &b,
            &material(),
            &c,
            ResourceKind::SecretMap
        )
        .err()
        .unwrap()
        .tag,
        CryptoErrorTag::UnsupportedOperation
    );
}
#[test]
fn c1_braid_role22_open_refusals_publish_no_handle_and_valid_restore_does() {
    use crate::actor::{Priority, Process, ProcessId};
    use crate::secret::{
        destroy_owned, insert_test_storage_key_resource, owned_secret_count_for_test,
        with_owned_resource,
    };
    crate::mesh_rt_init();
    let owner = ProcessId(191_221);
    let mut p = Process::new(owner, Priority::Normal);
    let key = insert_test_storage_key_resource(
        &mut p,
        Zeroizing::new(material().to_vec().into_boxed_slice()),
        0,
    )
    .unwrap();
    let c = context(2);
    let b = fixture_blob(&map(2), &c);
    for candidate in [fixture_blob(b"invalid", &c), b[..b.len() - 1].to_vec(), {
        let mut x = b.clone();
        *x.last_mut().unwrap() ^= 1;
        x
    }] {
        assert!(open_for_process(
            &mut p,
            &SystemProvider,
            &candidate,
            key,
            &c,
            ResourceKind::SecretMap
        )
        .is_err());
        assert_eq!(owned_secret_count_for_test(owner), 1);
    }
    let h = open_for_process(
        &mut p,
        &SystemProvider,
        &b,
        key,
        &c,
        ResourceKind::SecretMap,
    )
    .unwrap();
    assert_eq!(owned_secret_count_for_test(owner), 2);
    with_owned_resource(&p, h, ResourceKind::SecretMap, |v| assert_eq!(v, map(2))).unwrap();
    destroy_owned(owner);
    assert_eq!(owned_secret_count_for_test(owner), 0);
}
