//! Draft, UNRUN; typed ABI and actual actor lifecycle, no secret exporter.
use super::*;
use crate::secret::{
    as_test_actor, mesh_secret_random, mesh_secret_split_at, owned_secret_count_for_test,
    MeshSecretSplitTuple,
};
unsafe fn created<T>(r: *mut MeshResult) -> *mut T {
    assert_eq!(unsafe { (*r).tag }, 0);
    unsafe { (*r).value.cast() }
}
unsafe fn error(r: *mut MeshResult) -> u8 {
    assert_eq!(unsafe { (*r).tag }, 1);
    unsafe { *(*r).value.cast::<u8>() }
}
#[test]
fn secret_primitives_ffi_signatures_and_tuple_offsets() {
    let _: extern "C" fn(
        *const MeshSecretHandle,
        *const MeshSecretHandle,
        *const MeshBytes,
        i64,
    ) -> *mut MeshResult = mesh_crypto_hkdf_sha256_secret_salt;
    let _: extern "C" fn(*const MeshSecretHandle, *const MeshBytes) -> *mut MeshResult =
        mesh_crypto_hmac_sha256_tag;
    let _: extern "C" fn(*mut MeshSecretHandle, i64) -> *mut MeshResult = mesh_secret_split_at;
    assert_eq!(std::mem::size_of::<MeshSecretSplitTuple>(), 24);
    assert_eq!(std::mem::offset_of!(MeshSecretSplitTuple, len), 0);
    assert_eq!(std::mem::offset_of!(MeshSecretSplitTuple, left), 8);
    assert_eq!(std::mem::offset_of!(MeshSecretSplitTuple, right), 16);
}
#[test]
fn secret_primitives_ffi_actor_tuple_count_stale_source_and_borrowed_retry() {
    as_test_actor(|id| unsafe {
        let a = created(mesh_secret_random(32));
        let b = created(mesh_secret_random(32));
        let info = bytes_value(b"public info");
        assert_eq!(owned_secret_count_for_test(id), 2);
        assert_eq!(
            error(mesh_crypto_hkdf_sha256_secret_salt(a, b, info, 0)),
            CryptoErrorTag::InvalidLength as u8
        );
        let out = created(mesh_crypto_hkdf_sha256_secret_salt(a, b, info, 64));
        assert_eq!(owned_secret_count_for_test(id), 3);
        let before: *mut MeshBytes = created(mesh_crypto_hmac_sha256_tag(out, info));
        assert_eq!((*before).len, 32);
        let pair: *mut MeshSecretSplitTuple = created(mesh_secret_split_at(out, 32));
        assert_eq!((*pair).len, 2);
        assert_eq!(owned_secret_count_for_test(id), 4);
        assert_eq!(
            error(mesh_crypto_hmac_sha256_tag(out, info)),
            CryptoErrorTag::SecretDestroyed as u8
        );
        let joined = created(crate::secret::mesh_secret_concat(
            (*pair).left,
            (*pair).right,
        ));
        let after: *mut MeshBytes = created(mesh_crypto_hmac_sha256_tag(joined, info));
        assert_eq!((*before).as_slice(), (*after).as_slice());
        let retry = created(mesh_crypto_hkdf_sha256_secret_salt(a, b, info, 64));
        let again: *mut MeshBytes = created(mesh_crypto_hmac_sha256_tag(retry, info));
        assert_eq!((*before).as_slice(), (*again).as_slice());
        for value in [a, b, joined, retry] {
            crate::secret::mesh_resource_destroy(value);
        }
        assert_eq!(owned_secret_count_for_test(id), 0);
    });
}
#[test]
fn secret_primitives_ffi_split_error_consumes_while_borrow_error_preserves() {
    as_test_actor(|id| unsafe {
        let key = created(mesh_secret_random(32));
        let info = bytes_value(b"public");
        assert_eq!(
            error(mesh_crypto_hkdf_sha256_secret_salt(key, key, info, 8161)),
            CryptoErrorTag::InvalidLength as u8
        );
        let _: *mut MeshBytes = created(mesh_crypto_hmac_sha256_tag(key, info));
        assert_eq!(owned_secret_count_for_test(id), 1);
        assert_eq!(
            error(mesh_secret_split_at(key, 32)),
            CryptoErrorTag::InvalidKey as u8
        );
        assert_eq!(owned_secret_count_for_test(id), 0);
        assert_eq!(
            error(mesh_crypto_hmac_sha256_tag(key, info)),
            CryptoErrorTag::SecretDestroyed as u8
        );
    });
}

#[test]
fn secret_primitives_ffi_off_actor_is_internal_failure_before_private_work() {
    crate::gc::mesh_rt_init();
    unsafe {
        assert_eq!(
            error(mesh_crypto_hkdf_sha256_secret_salt(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                32
            )),
            CryptoErrorTag::InternalFailure as u8
        );
        assert_eq!(
            error(mesh_crypto_hmac_sha256_tag(ptr::null(), ptr::null())),
            CryptoErrorTag::InternalFailure as u8
        );
        assert_eq!(
            error(mesh_secret_split_at(ptr::null_mut(), 16)),
            CryptoErrorTag::InternalFailure as u8
        );
    }
}
