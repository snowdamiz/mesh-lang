use super::*;
use crate::actor::{Priority, ProcessId};
use crate::secret::{destroy_owned, insert_test_storage_key_resource};

#[test]
fn index_tag_matches_independent_hkdf_hmac_vector_and_domains() {
    let mut material = [0x31; STORAGE_KEY_MATERIAL_BYTES];
    let namespace = b"catalog\0v1";
    let value = b"\0\xffprofile/v1";
    let first = index_tag_value(&SystemProvider, &material, namespace, value).unwrap();
    let expected = "2132ed67a6facbd7dc9d08cb94d2efdf0c9169eea6e05d39d2b28f1e3766400b";
    let encoded: String = first.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(encoded, expected);
    // The nonce prefix and durable reservation count do not define the index key.
    material[32..].fill(0xff);
    assert_eq!(
        index_tag_value(&SystemProvider, &material, namespace, value).unwrap(),
        first
    );
    assert_ne!(
        index_tag_value(&SystemProvider, &material, b"catalog\0v2", value).unwrap(),
        first
    );
    assert_ne!(
        index_tag_value(&SystemProvider, &material, namespace, b"profile/v1").unwrap(),
        first
    );
    material[0] ^= 1;
    assert_ne!(
        index_tag_value(&SystemProvider, &material, namespace, value).unwrap(),
        first
    );
}

#[test]
fn index_tag_bounds_are_exact_and_the_key_is_borrowed() {
    let material = [0x31; STORAGE_KEY_MATERIAL_BYTES];
    assert!(index_tag_value(&SystemProvider, &material, &[b'n'; 256], &[0; 65536]).is_ok());
    assert!(index_tag_value(&SystemProvider, &material, b"n", &[]).is_ok());
    for (namespace, value) in [
        (vec![], vec![]),
        (vec![0; 257], vec![]),
        (vec![1], vec![0; 65537]),
    ] {
        assert_eq!(
            index_tag_value(&SystemProvider, &material, &namespace, &value)
                .unwrap_err()
                .tag,
            CryptoErrorTag::InvalidLength
        );
    }
    crate::gc::mesh_rt_init();
    let owner = ProcessId(81_501);
    let mut process = Process::new(owner, Priority::Normal);
    let key = insert_test_storage_key_resource(
        &mut process,
        Zeroizing::new(material.to_vec().into_boxed_slice()),
        u64::MAX,
    )
    .unwrap();
    let first = index_tag_for_process(&process, key, b"n", b"record").unwrap();
    assert_eq!(
        index_tag_for_process(&process, key, b"n", b"record").unwrap(),
        first
    );
    let wrong_kind = insert_owned_resource(
        &mut process,
        ResourceKind::SecretBytes,
        Zeroizing::new(vec![0x31; 32].into_boxed_slice()),
    )
    .unwrap();
    assert_eq!(
        index_tag_for_process(&process, wrong_kind, b"n", b"record")
            .unwrap_err()
            .tag,
        CryptoErrorTag::InvalidKey
    );
    // Indexing does not reserve a nonce, including after the counter is exhausted.
    let other = Process::new(ProcessId(81_502), Priority::Normal);
    assert_eq!(
        index_tag_for_process(&other, key, b"n", b"record")
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
    destroy_owned(owner);
    assert_eq!(
        index_tag_for_process(&process, key, b"n", b"record")
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
}
