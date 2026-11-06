use super::*;
use crate::bytes::MeshBytes;
use crate::crypto::{bytes_value, mesh_crypto_hmac_sha256_tag};
use crate::string::mesh_string_new;

unsafe fn ok<T>(result: *mut MeshResult) -> *mut T {
    assert_eq!((*result).tag, 0, "expected successful recovery operation");
    (*result).value.cast()
}
unsafe fn refused(result: *mut MeshResult) -> u8 {
    assert_eq!((*result).tag, 1, "expected refused recovery operation");
    *(*result).value.cast::<u8>()
}
fn text(value: &str) -> *mut MeshString {
    mesh_string_new(value.as_ptr(), value.len() as u64)
}
unsafe fn tag(secret: *const MeshSecretHandle) -> Vec<u8> {
    let result: *mut MeshBytes = ok(mesh_crypto_hmac_sha256_tag(
        secret,
        bytes_value(b"public recovery-code agreement"),
    ));
    (*result).as_slice().to_vec()
}

#[test]
fn recovery_code_fixed_vectors_import_exact_secret_and_destroy() {
    let vectors = [
        (
            "MRC1-00010203-04050607-08090A0B-0C0D0E0F-10111213-14151617-18191A1B-1C1D1E1F-3A37D678",
            "ef7d4fb10fe673230e3d6b7e3f4d109ce63f9f67f0edd745b797273b0a8e2a5c",
        ),
        (
            "MRC1-FFFFFFFF-FFFFFFFF-FFFFFFFF-FFFFFFFF-FFFFFFFF-FFFFFFFF-FFFFFFFF-FFFFFFFF-AEC8895F",
            "75b4e69dec7f732091e58d278810a8b6a55b8bf9c7421f22068f512a3cc28f39",
        ),
        (
            "MRC1-00000000-00000000-00000000-00000000-00000000-00000000-00000000-00000000-46527319",
            "6fef023b663bd6165f2974414b1cc23780594a3a51f2e9e6c59a5ccefd3c60b3",
        ),
    ];
    as_test_actor(|owner| unsafe {
        for (code, expected) in vectors {
            let secret = ok(mesh_recovery_code_parse(text(code)));
            assert_eq!(owned_secret_count_for_test(owner), 1);
            let encoded: String = tag(secret).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(encoded, expected);
            mesh_resource_destroy(secret);
            assert_eq!(owned_secret_count_for_test(owner), 0);
            assert_eq!(
                refused(mesh_crypto_hmac_sha256_tag(secret, bytes_value(b"retry"))),
                CryptoErrorTag::SecretDestroyed as u8
            );
        }
    });
}

#[test]
fn recovery_code_generate_import_agree_without_export_and_use_distinct_entropy() {
    as_test_actor(|owner| unsafe {
        let pair: *mut RecoveryCodeTuple = ok(mesh_recovery_code_generate());
        assert_eq!((*pair).len, 2);
        assert_eq!((*(*pair).code).len, 85);
        assert_eq!(owned_secret_count_for_test(owner), 1);
        let imported = ok(mesh_recovery_code_parse((*pair).code));
        assert_eq!(tag((*pair).secret), tag(imported));
        let another: *mut RecoveryCodeTuple = ok(mesh_recovery_code_generate());
        assert!(
            tag((*pair).secret) != tag((*another).secret),
            "independent generations reused recovery material"
        );
        assert_eq!(owned_secret_count_for_test(owner), 3);
        for secret in [(*pair).secret, imported, (*another).secret] {
            mesh_resource_destroy(secret);
        }
        assert_eq!(owned_secret_count_for_test(owner), 0);
    });
}

#[test]
fn recovery_code_malformed_inputs_publish_no_secret_and_valid_retry_succeeds() {
    const CODE: &str =
        "MRC1-00010203-04050607-08090A0B-0C0D0E0F-10111213-14151617-18191A1B-1C1D1E1F-3A37D678";
    as_test_actor(|owner| unsafe {
        let mut altered = CODE.as_bytes().to_vec();
        altered[5] = if altered[5] == b'0' { b'1' } else { b'0' };
        let mut checksum = CODE.as_bytes().to_vec();
        checksum[84] = if checksum[84] == b'0' { b'1' } else { b'0' };
        let variants = vec![
            String::new(),
            CODE.to_lowercase(),
            CODE.replacen("MRC1", "MRC2", 1),
            CODE.replacen('-', "_", 1),
            CODE.replace('-', ""),
            format!(" {CODE}"),
            format!("{CODE} "),
            format!("{CODE}x"),
            CODE[..84].to_string(),
            CODE.replacen('0', "é", 1),
            String::from_utf8(altered).unwrap(),
            String::from_utf8(checksum).unwrap(),
        ];
        for value in variants {
            assert_eq!(
                refused(mesh_recovery_code_parse(text(&value))),
                CryptoErrorTag::InvalidKey as u8
            );
            assert_eq!(owned_secret_count_for_test(owner), 0);
        }
        assert_eq!(
            refused(mesh_recovery_code_parse(std::ptr::null())),
            CryptoErrorTag::InvalidKey as u8
        );
        let secret = ok(mesh_recovery_code_parse(text(CODE)));
        assert_eq!(owned_secret_count_for_test(owner), 1);
        mesh_resource_destroy(secret);
        assert_eq!(owned_secret_count_for_test(owner), 0);
    });
}

#[test]
fn recovery_code_tuple_abi_and_actor_exit_cleanup() {
    let _: extern "C" fn() -> *mut MeshResult = mesh_recovery_code_generate;
    let _: extern "C" fn(*const MeshString) -> *mut MeshResult = mesh_recovery_code_parse;
    assert_eq!(std::mem::size_of::<RecoveryCodeTuple>(), 24);
    assert_eq!(std::mem::offset_of!(RecoveryCodeTuple, len), 0);
    assert_eq!(std::mem::offset_of!(RecoveryCodeTuple, secret), 8);
    assert_eq!(std::mem::offset_of!(RecoveryCodeTuple, code), 16);
    let owner = as_test_actor(|owner| unsafe {
        let _: *mut RecoveryCodeTuple = ok(mesh_recovery_code_generate());
        assert_eq!(owned_secret_count_for_test(owner), 1);
        owner
    });
    assert_eq!(owned_secret_count_for_test(owner), 0);
}

#[test]
fn recovery_code_entropy_failure_returns_no_pair_or_owned_secret() {
    as_test_actor(|owner| {
        let result = fresh_material(|out| {
            out.fill(0xA5);
            Err(failure(CryptoErrorTag::EntropyUnavailable, 0, 0))
        });
        assert!(
            result.is_err(),
            "entropy failure published recovery material"
        );
        assert_eq!(owned_secret_count_for_test(owner), 0);
    });
}

#[test]
fn recovery_code_off_actor_refuses_before_reading_or_generating_material() {
    crate::gc::mesh_rt_init();
    unsafe {
        assert_eq!(
            refused(mesh_recovery_code_generate()),
            CryptoErrorTag::InternalFailure as u8
        );
        assert_eq!(
            refused(mesh_recovery_code_parse(std::ptr::null())),
            CryptoErrorTag::InternalFailure as u8
        );
    }
}
