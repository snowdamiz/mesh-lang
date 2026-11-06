use super::super::*;
use super::*;
use crate::secret::{as_test_actor, owned_secret_count_for_test};
use crate::storage_wrapping::{
    mesh_mlkem_encapsulation_state_seal_for_storage,
    mesh_mlkem_encapsulation_state_unseal_from_storage, mesh_storage_key_ephemeral,
};
fn bytes(data: &[u8]) -> *mut MeshBytes {
    bytes_value(data)
}
unsafe fn created<T>(r: *mut MeshResult) -> *mut T {
    assert_eq!(unsafe { (*r).tag }, 0);
    unsafe { (*r).value.cast() }
}
unsafe fn tag(r: *mut MeshResult) -> u8 {
    assert_eq!(unsafe { (*r).tag }, 1);
    unsafe { *(*r).value.cast::<u8>() }
}
fn private(handle: *const MeshSecretHandle) -> Vec<u8> {
    with_current_process(|p| {
        with_owned_resource(p, handle, ResourceKind::SecretBytes, |b| b.to_vec())
            .map_err(resource_failure)
    })
    .unwrap()
}
fn context() -> Vec<u8> {
    let mut v = vec![1];
    v.extend([1; 32]);
    v.extend([2; 16]);
    v.extend([3; 32]);
    v.extend([4; 32]);
    v.extend(21u16.to_be_bytes());
    v.extend(1u64.to_be_bytes());
    v
}
#[test]
fn c1_ffi_exact_layout_and_six_native_signatures() {
    assert_eq!(std::mem::size_of::<PartsTuple>(), 24);
    assert_eq!(std::mem::offset_of!(PartsTuple, vector), 16);
    assert_eq!(std::mem::size_of::<FirstTuple>(), 32);
    assert_eq!(std::mem::offset_of!(FirstTuple, state), 8);
    assert_eq!(std::mem::offset_of!(FirstTuple, ciphertext), 16);
    assert_eq!(std::mem::offset_of!(FirstTuple, shared), 24);
    assert_eq!(std::mem::size_of::<MeshMlKemHeader64>(), 8);
    assert_eq!(std::mem::size_of::<MeshMlKemPublicVector1152>(), 8);
    let _: extern "C" fn(*const MeshSecretHandle) -> *mut MeshResult =
        mesh_crypto_mlkem_public_parts;
    let _: extern "C" fn(*const MeshMlKemHeader64) -> *mut MeshResult =
        mesh_crypto_mlkem_encapsulate1;
    let _: extern "C" fn(
        *const MeshSecretHandle,
        *const MeshMlKemPublicVector1152,
    ) -> *mut MeshResult = mesh_crypto_mlkem_encapsulate2;
    let _: extern "C" fn(
        *const MeshSecretHandle,
        *const MeshMlKemCiphertext1,
        *const MeshMlKemCiphertext2,
    ) -> *mut MeshResult = mesh_crypto_mlkem_decapsulate_parts;
    let _: extern "C" fn(
        *const MeshSecretHandle,
        *const MeshSecretHandle,
        *const MeshBytes,
    ) -> *mut MeshResult = mesh_mlkem_encapsulation_state_seal_for_storage;
    let _: extern "C" fn(
        *const MeshBytes,
        *const MeshSecretHandle,
        *const MeshBytes,
    ) -> *mut MeshResult = mesh_mlkem_encapsulation_state_unseal_from_storage;
}
#[test]
fn c1_ffi_null_width_and_off_actor_errors_precede_private_work() {
    crate::gc::mesh_rt_init();
    unsafe {
        assert_eq!(
            tag(mesh_crypto_mlkem_encapsulate1(ptr::null())),
            CryptoErrorTag::InvalidLength as u8
        );
        assert_eq!(
            tag(mesh_crypto_mlkem_encapsulate2(ptr::null(), ptr::null())),
            CryptoErrorTag::InvalidLength as u8
        );
        assert_eq!(
            tag(mesh_crypto_mlkem_decapsulate_parts(
                ptr::null(),
                ptr::null(),
                ptr::null()
            )),
            CryptoErrorTag::InvalidLength as u8
        );
        for n in [0, 63, 65] {
            let header = MeshMlKemHeader64 {
                bytes: bytes(&vec![0; n]),
            };
            assert_eq!(
                tag(mesh_crypto_mlkem_encapsulate1(&header)),
                CryptoErrorTag::InvalidLength as u8
            );
        }
        assert_eq!(
            tag(mesh_crypto_mlkem_public_parts(ptr::null())),
            CryptoErrorTag::InternalFailure as u8
        );
        let header = MeshMlKemHeader64 {
            bytes: bytes(&[0; 64]),
        };
        assert_eq!(
            tag(mesh_crypto_mlkem_encapsulate1(&header)),
            CryptoErrorTag::InternalFailure as u8
        );
    }
}
#[test]
fn c1_ffi_real_actor_roundtrip_implicit_rejection_retry_and_sealed_restore() {
    as_test_actor(|owner| unsafe {
        let pair: *mut MeshMlKemKeyPair = created(mesh_crypto_mlkem_from_seed(bytes(&[0x69; 64])));
        let parts: *mut PartsTuple = created(mesh_crypto_mlkem_public_parts((*pair).private_key));
        assert_eq!((*parts).len, 2);
        let header = MeshMlKemHeader64 {
            bytes: (*parts).header,
        };
        let vector = MeshMlKemPublicVector1152 {
            bytes: (*parts).vector,
        };
        let full_public = (*(*pair).public_key.bytes).as_slice();
        assert_eq!(&full_public[..1152], (*vector.bytes).as_slice());
        assert_eq!(&full_public[1152..], &(*header.bytes).as_slice()[..32]);
        let first: *mut FirstTuple = created(mesh_crypto_mlkem_encapsulate1(&header));
        assert_eq!((*first).len, 3);
        assert_eq!(owned_secret_count_for_test(owner), 3);
        let second: *mut MeshMlKemCiphertext2 =
            created(mesh_crypto_mlkem_encapsulate2((*first).state, &vector));
        let one = MeshMlKemCiphertext1 {
            bytes: (*first).ciphertext,
        };
        let expected = (*(*second).bytes).as_slice().to_vec();
        let recovered = created(mesh_crypto_mlkem_decapsulate_parts(
            (*pair).private_key,
            &one,
            second,
        ));
        assert_eq!(private(recovered), private((*first).shared));
        let mut full = (*one.bytes).as_slice().to_vec();
        full.extend(&expected);
        let full = MeshMlKemCiphertext {
            bytes: bytes(&full),
        };
        let old = created(mesh_crypto_mlkem_decapsulate((*pair).private_key, &full));
        assert_eq!(private(old), private(recovered));
        let wrong = MeshMlKemPublicVector1152 {
            bytes: bytes(&[0xff; 1152]),
        };
        let before = owned_secret_count_for_test(owner);
        assert_eq!(
            tag(mesh_crypto_mlkem_encapsulate2((*first).state, &wrong)),
            CryptoErrorTag::InvalidPublicKey as u8
        );
        assert_eq!(owned_secret_count_for_test(owner), before);
        let again: *mut MeshMlKemCiphertext2 =
            created(mesh_crypto_mlkem_encapsulate2((*first).state, &vector));
        assert_eq!((*(*again).bytes).as_slice(), expected);
        let mut changed = expected.clone();
        changed[0] ^= 1;
        let bad = MeshMlKemCiphertext2 {
            bytes: bytes(&changed),
        };
        let rejected = created(mesh_crypto_mlkem_decapsulate_parts(
            (*pair).private_key,
            &one,
            &bad,
        ));
        let repeated = created(mesh_crypto_mlkem_decapsulate_parts(
            (*pair).private_key,
            &one,
            &bad,
        ));
        assert_ne!(private(rejected), private(recovered));
        assert_eq!(private(rejected), private(repeated));
        let key = created(mesh_storage_key_ephemeral());
        let context = bytes(&context());
        let blob: *mut MeshBytes = created(mesh_mlkem_encapsulation_state_seal_for_storage(
            (*first).state,
            key,
            context,
        ));
        crate::secret::mesh_resource_destroy((*first).state);
        let count = owned_secret_count_for_test(owner);
        assert_eq!(
            tag(mesh_crypto_mlkem_encapsulate2((*first).state, &vector)),
            CryptoErrorTag::SecretDestroyed as u8
        );
        assert_eq!(owned_secret_count_for_test(owner), count);
        let state = created(mesh_mlkem_encapsulation_state_unseal_from_storage(
            blob, key, context,
        ));
        let resumed: *mut MeshMlKemCiphertext2 =
            created(mesh_crypto_mlkem_encapsulate2(state, &vector));
        assert_eq!((*(*resumed).bytes).as_slice(), expected);
        let mut tampered = (*blob).as_slice().to_vec();
        *tampered.last_mut().unwrap() ^= 1;
        let count = owned_secret_count_for_test(owner);
        assert_eq!(
            tag(mesh_mlkem_encapsulation_state_unseal_from_storage(
                bytes(&tampered),
                key,
                context
            )),
            CryptoErrorTag::AuthenticationFailed as u8
        );
        assert_eq!(owned_secret_count_for_test(owner), count);
        assert_eq!(
            tag(mesh_crypto_mlkem_public_parts(state)),
            CryptoErrorTag::InvalidKey as u8
        );
        assert_eq!(
            tag(mesh_crypto_mlkem_encapsulate2((*pair).private_key, &vector)),
            CryptoErrorTag::InvalidKey as u8
        );
    });
}

#[test]
fn c1_ffi_foreign_owner_and_malformed_private_resource_refuse_without_publication() {
    use crate::actor::{Priority, Process, ProcessId};
    use crate::protected_memory::ProtectedBytes;
    let foreign_owner = ProcessId(191_201);
    let mut foreign = Process::new(foreign_owner, Priority::Normal);
    let foreign_key = insert_owned_resource(
        &mut foreign,
        ResourceKind::MlKemPrivateKey,
        ProtectedBytes::copy_from(&[0x69; 64]).unwrap(),
    )
    .unwrap();
    as_test_actor(|owner| unsafe {
        let count = owned_secret_count_for_test(owner);
        assert_eq!(
            tag(mesh_crypto_mlkem_public_parts(foreign_key)),
            CryptoErrorTag::SecretDestroyed as u8
        );
        assert_eq!(owned_secret_count_for_test(owner), count);
        let short = with_current_process(|p| {
            insert_owned_resource(
                p,
                ResourceKind::MlKemPrivateKey,
                ProtectedBytes::copy_from(&[0; 63]).unwrap(),
            )
            .map_err(resource_failure)
        })
        .unwrap();
        let count = owned_secret_count_for_test(owner);
        assert_eq!(
            tag(mesh_crypto_mlkem_public_parts(short)),
            CryptoErrorTag::InvalidKey as u8
        );
        let one = MeshMlKemCiphertext1 {
            bytes: bytes(&[0; 960]),
        };
        let two = MeshMlKemCiphertext2 {
            bytes: bytes(&[0; 128]),
        };
        assert_eq!(
            tag(mesh_crypto_mlkem_decapsulate_parts(short, &one, &two)),
            CryptoErrorTag::InvalidKey as u8
        );
        assert_eq!(owned_secret_count_for_test(owner), count);
    });
    crate::secret::destroy_owned(foreign_owner);
}

#[test]
fn c1_public_validation_ffi_needs_no_actor_and_publishes_no_secret() {
    crate::gc::mesh_rt_init();
    let _: extern "C" fn(
        *const MeshMlKemHeader64,
        *const MeshMlKemPublicVector1152,
    ) -> *mut MeshResult = mesh_crypto_mlkem_validate_public_parts;
    let (h, v) = mlkem_incremental::parts(&[0x69; 64]);
    let header = MeshMlKemHeader64 { bytes: bytes(&h) };
    let vector = MeshMlKemPublicVector1152 { bytes: bytes(&v) };
    unsafe {
        // MeshCryptoError's existing repr(C) stores tag/padding then expected/actual.
        let both_null = mesh_crypto_mlkem_validate_public_parts(ptr::null(), ptr::null());
        assert_eq!(tag(both_null), CryptoErrorTag::InvalidLength as u8);
        let sizes = (*both_null).value.cast::<i64>();
        assert_eq!((sizes.add(1).read(), sizes.add(2).read()), (64, -1));
        let bad_header = MeshMlKemHeader64 {
            bytes: bytes(&[0; 13]),
        };
        let header_before_vector =
            mesh_crypto_mlkem_validate_public_parts(&bad_header, ptr::null());
        assert_eq!(
            tag(header_before_vector),
            CryptoErrorTag::InvalidLength as u8
        );
        let sizes = (*header_before_vector).value.cast::<i64>();
        assert_eq!((sizes.add(1).read(), sizes.add(2).read()), (64, 13));
        let vector_null = mesh_crypto_mlkem_validate_public_parts(&header, ptr::null());
        let sizes = (*vector_null).value.cast::<i64>();
        assert_eq!((sizes.add(1).read(), sizes.add(2).read()), (1152, -1));
        let success = mesh_crypto_mlkem_validate_public_parts(&header, &vector);
        assert_eq!((*success).tag, 0);
        assert!(
            (*success).value.is_null(),
            "unit result must use the existing null payload"
        );
        assert_eq!(
            tag(mesh_crypto_mlkem_validate_public_parts(
                ptr::null(),
                &vector
            )),
            CryptoErrorTag::InvalidLength as u8
        );
        assert_eq!(
            tag(mesh_crypto_mlkem_validate_public_parts(
                &header,
                ptr::null()
            )),
            CryptoErrorTag::InvalidLength as u8
        );
        for n in [0, 63, 65] {
            let wrong = MeshMlKemHeader64 {
                bytes: bytes(&vec![0; n]),
            };
            assert_eq!(
                tag(mesh_crypto_mlkem_validate_public_parts(&wrong, &vector)),
                CryptoErrorTag::InvalidLength as u8
            );
        }
        for n in [0, 1151, 1153] {
            let wrong = MeshMlKemPublicVector1152 {
                bytes: bytes(&vec![0; n]),
            };
            assert_eq!(
                tag(mesh_crypto_mlkem_validate_public_parts(&header, &wrong)),
                CryptoErrorTag::InvalidLength as u8
            );
        }
    }
    as_test_actor(|owner| unsafe {
        let before = owned_secret_count_for_test(owner);
        let wrong = MeshMlKemPublicVector1152 {
            bytes: bytes(&[0; 1152]),
        };
        for _ in 0..3 {
            assert_eq!(
                (*mesh_crypto_mlkem_validate_public_parts(&header, &vector)).tag,
                0
            );
            assert_eq!(
                tag(mesh_crypto_mlkem_validate_public_parts(&header, &wrong)),
                CryptoErrorTag::InvalidPublicKey as u8
            );
        }
        assert_eq!(owned_secret_count_for_test(owner), before);
    });
}
