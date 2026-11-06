//! Hook under crypto.rs only when the RED lane is granted.
//! Fixed material is public RFC vector input imported by a Rust-only test helper.
use super::*;
use crate::actor::{Priority, ProcessId};
use crate::secret::{destroy_owned, owned_secret_count_for_test};
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/vectors/secret-primitives/rfc5869-4231.json"
    )))
    .unwrap()
}
fn unhex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
fn secret(p: &mut Process, data: &[u8]) -> *mut MeshSecretHandle {
    insert_owned_resource(
        p,
        ResourceKind::SecretBytes,
        Zeroizing::new(data.to_vec().into_boxed_slice()),
    )
    .unwrap()
}
fn private(p: &Process, key: *const MeshSecretHandle) -> Vec<u8> {
    with_owned_resource(p, key, ResourceKind::SecretBytes, |v| v.to_vec()).unwrap()
}
fn bytes(v: &[u8]) -> *mut MeshBytes {
    bytes_value(v)
}
#[test]
fn secret_salt_hkdf_rfc5869_known_answers_borrow_both_inputs() {
    crate::mesh_rt_init();
    let id = ProcessId(914_001);
    let mut p = Process::new(id, Priority::Normal);
    for row in fixture()["hkdf_sha256"].as_array().unwrap() {
        let ikm = unhex(row["ikm"].as_str().unwrap());
        let salt = unhex(row["salt"].as_str().unwrap());
        let info = unhex(row["info"].as_str().unwrap());
        let a = secret(&mut p, &ikm);
        let b = secret(&mut p, &salt);
        let n = row["length"].as_i64().unwrap();
        let out =
            hkdf_sha256_secret_salt_for_process(&mut p, &SystemProvider, a, b, bytes(&info), n)
                .unwrap();
        assert_eq!(
            private(&p, out),
            unhex(row["okm"].as_str().unwrap()),
            "{}",
            row["case"]
        );
        assert_eq!(private(&p, a), ikm);
        assert_eq!(private(&p, b), salt);
        let old_salt = if row["case"].as_str().unwrap().contains("A.3") {
            bytes(&[])
        } else {
            bytes(&salt)
        };
        let old =
            hkdf_sha256_for_process(&mut p, &SystemProvider, a, old_salt, bytes(&info), n).unwrap();
        assert_eq!(private(&p, old), private(&p, out));
        destroy_owned(id);
    }
}
#[test]
fn hmac_sha256_tag_rfc4231_full32_octets_and_borrow_retry() {
    crate::mesh_rt_init();
    let id = ProcessId(914_002);
    let mut p = Process::new(id, Priority::Normal);
    for row in fixture()["hmac_sha256"].as_array().unwrap() {
        let k = unhex(row["key"].as_str().unwrap());
        let m = unhex(row["message"].as_str().unwrap());
        let key = secret(&mut p, &k);
        let tag = hmac_sha256_tag_for_process(&mut p, &SystemProvider, key, bytes(&m)).unwrap();
        assert_eq!(tag.len(), 32);
        assert_eq!(&tag[..], unhex(row["tag"].as_str().unwrap()));
        let again = hmac_sha256_tag_for_process(&mut p, &SystemProvider, key, bytes(&m)).unwrap();
        assert_eq!(&tag[..], &again[..]);
        let old = hmac_sha256_for_process(&mut p, &SystemProvider, key, bytes(&m)).unwrap();
        assert_eq!(&tag[..], private(&p, old));
        assert_eq!(private(&p, key), k);
        destroy_owned(id);
    }
}
#[test]
fn secret_primitives_public_shape_precedes_handle_read_inside_live_actor() {
    crate::mesh_rt_init();
    let id = ProcessId(914_003);
    let mut p = Process::new(id, Priority::Normal);
    for n in [i64::MIN, -1, 0, 8161, i64::MAX] {
        let e = hkdf_sha256_secret_salt_for_process(
            &mut p,
            &SystemProvider,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            n,
        )
        .unwrap_err();
        assert_eq!(e.tag, CryptoErrorTag::InvalidLength);
        assert_eq!((e.expected, e.actual), (8160, n));
    }
    let too_long = vec![0; 65537];
    for msg in [ptr::null(), bytes(&too_long)] {
        assert_eq!(
            hmac_sha256_tag_for_process(&mut p, &SystemProvider, ptr::null(), msg)
                .unwrap_err()
                .tag,
            CryptoErrorTag::InvalidLength
        );
        assert_eq!(
            hkdf_sha256_secret_salt_for_process(
                &mut p,
                &SystemProvider,
                ptr::null(),
                ptr::null(),
                msg,
                32
            )
            .unwrap_err()
            .tag,
            CryptoErrorTag::InvalidLength
        );
    }
    let a = secret(&mut p, &[1; 32]);
    let b = secret(&mut p, &[2; 32]);
    let full_tag =
        hmac_sha256_tag_for_process(&mut p, &SystemProvider, a, bytes(&[3; 65536])).unwrap();
    assert_eq!(full_tag.len(), 32);
    let out = hkdf_sha256_secret_salt_for_process(
        &mut p,
        &SystemProvider,
        a,
        b,
        bytes(&[3; 65536]),
        8160,
    )
    .unwrap();
    assert_eq!(private(&p, out).len(), 8160);
    let same =
        hkdf_sha256_secret_salt_for_process(&mut p, &SystemProvider, a, a, bytes(&[]), 32).unwrap();
    assert_eq!(private(&p, same).len(), 32);
    destroy_owned(id);
}
#[test]
fn secret_primitives_wrong_owner_kind_and_stale_refuse_without_destroying_inputs() {
    crate::mesh_rt_init();
    let id = ProcessId(914_004);
    let mut p = Process::new(id, Priority::Normal);
    let mut foreign = Process::new(ProcessId(914_005), Priority::Normal);
    let a = secret(&mut p, &[1; 32]);
    let b = secret(&mut p, &[2; 32]);
    let wrong = insert_owned_resource(
        &mut p,
        ResourceKind::AeadKey,
        Zeroizing::new(vec![0; 32].into_boxed_slice()),
    )
    .unwrap();
    assert_eq!(
        hkdf_sha256_secret_salt_for_process(&mut foreign, &SystemProvider, a, b, bytes(&[]), 32)
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
    for (left, right) in [(wrong, b), (a, wrong)] {
        assert_eq!(
            hkdf_sha256_secret_salt_for_process(
                &mut p,
                &SystemProvider,
                left,
                right,
                bytes(&[]),
                32
            )
            .unwrap_err()
            .tag,
            CryptoErrorTag::InvalidKey
        );
    }
    assert_eq!(
        hmac_sha256_tag_for_process(&mut p, &SystemProvider, wrong, bytes(&[]))
            .unwrap_err()
            .tag,
        CryptoErrorTag::InvalidKey
    );
    assert_eq!(private(&p, a), vec![1; 32]);
    assert_eq!(private(&p, b), vec![2; 32]);
    destroy_owned(id);
    assert_eq!(
        hmac_sha256_tag_for_process(&mut p, &SystemProvider, a, bytes(&[]))
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(
        hkdf_sha256_secret_salt_for_process(&mut p, &SystemProvider, a, b, bytes(&[]), 32)
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
}
struct FailingProvider;
impl CryptoProvider for FailingProvider {
    fn fill_random(&self, _: &mut [u8]) -> Result<(), provider::ProviderError> {
        panic!("these deterministic APIs must not request randomness")
    }
    fn hkdf_sha256(
        &self,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        out: &mut [u8],
    ) -> Result<(), provider::ProviderError> {
        out.fill(0xa5);
        Err(provider::ProviderError::InternalFailure)
    }
    fn hmac_sha256(
        &self,
        _: &[u8],
        _: &[u8],
        out: &mut [u8; 32],
    ) -> Result<(), provider::ProviderError> {
        out.fill(0xa5);
        Err(provider::ProviderError::InternalFailure)
    }
}
struct DestroyDuringProvider(ProcessId);
impl CryptoProvider for DestroyDuringProvider {
    fn fill_random(&self, _: &mut [u8]) -> Result<(), provider::ProviderError> {
        panic!("unexpected RNG")
    }
    fn hkdf_sha256(
        &self,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        out: &mut [u8],
    ) -> Result<(), provider::ProviderError> {
        out.fill(0xa5);
        destroy_owned(self.0);
        Ok(())
    }
    fn hmac_sha256(
        &self,
        _: &[u8],
        _: &[u8],
        out: &mut [u8; 32],
    ) -> Result<(), provider::ProviderError> {
        out.fill(0xa5);
        destroy_owned(self.0);
        Ok(())
    }
}
#[test]
fn secret_primitives_provider_failure_preserves_borrows_and_generation_change_blocks_publication() {
    crate::mesh_rt_init();
    let id = ProcessId(914_006);
    let mut p = Process::new(id, Priority::Normal);
    let a = secret(&mut p, &[1; 32]);
    let b = secret(&mut p, &[2; 32]);
    let count = owned_secret_count_for_test(id);
    assert_eq!(
        hkdf_sha256_secret_salt_for_process(&mut p, &FailingProvider, a, b, bytes(&[]), 32)
            .unwrap_err()
            .tag,
        CryptoErrorTag::InternalFailure
    );
    assert_eq!(
        hmac_sha256_tag_for_process(&mut p, &FailingProvider, a, bytes(&[]))
            .unwrap_err()
            .tag,
        CryptoErrorTag::InternalFailure
    );
    assert_eq!(owned_secret_count_for_test(id), count);
    assert_eq!(private(&p, a), vec![1; 32]);
    assert_eq!(private(&p, b), vec![2; 32]);
    assert_eq!(
        hkdf_sha256_secret_salt_for_process(
            &mut p,
            &DestroyDuringProvider(id),
            a,
            b,
            bytes(&[]),
            32
        )
        .unwrap_err()
        .tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(owned_secret_count_for_test(id), 0);
    let key = secret(&mut p, &[3; 32]);
    assert_eq!(
        hmac_sha256_tag_for_process(&mut p, &DestroyDuringProvider(id), key, bytes(&[]))
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(owned_secret_count_for_test(id), 0);
}
#[test]
fn secret_salt_output_quota_failure_does_not_consume_borrowed_inputs() {
    crate::mesh_rt_init();
    let id = ProcessId(914_007);
    let mut p = Process::new(id, Priority::Normal);
    let a = secret(&mut p, &[1; 32]);
    let b = secret(&mut p, &[2; 32]);
    for _ in 2..4096 {
        let _ = secret(&mut p, &[0]);
    }
    assert_eq!(
        hkdf_sha256_secret_salt_for_process(&mut p, &SystemProvider, a, b, bytes(&[]), 32)
            .unwrap_err()
            .tag,
        CryptoErrorTag::ResourceLimitExceeded
    );
    assert_eq!(owned_secret_count_for_test(id), 4096);
    assert_eq!(private(&p, a), vec![1; 32]);
    assert_eq!(private(&p, b), vec![2; 32]);
    destroy_owned(id);
}
