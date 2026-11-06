//! Public OHTTP FFI admission and one-shot consumption checks.
use super::ohttp::{decapsulate_request, encapsulate_request};
use super::provider::{CryptoProvider, FixedProvider, SystemProvider};
use super::{
    mesh_ohttp_decapsulate_response, mesh_ohttp_encapsulate_request,
    mesh_ohttp_encapsulate_response, MeshX25519PublicKey,
};
use crate::actor::{Priority, Process, ProcessId};
use crate::bytes::{mesh_bytes_new, MeshBytes};
use crate::io::MeshResult;
use crate::secret::{
    as_test_actor, destroy_owned, insert_owned_resource, owned_secret_count_for_test,
    CryptoErrorTag, MeshSecretHandle, ResourceKind,
};
use std::ptr;
use zeroize::Zeroizing;

fn bytes(input: &[u8]) -> *mut MeshBytes {
    mesh_bytes_new(input.as_ptr(), input.len() as u64)
}

fn error(result: *mut MeshResult, tag: CryptoErrorTag) {
    assert!(!result.is_null());
    unsafe {
        assert_eq!((*result).tag, 1);
        assert!(!(*result).value.is_null());
        assert_eq!(*(*result).value, tag as u8);
    }
}

fn contexts() -> (ProcessId, *mut MeshSecretHandle, *mut MeshSecretHandle) {
    let actor = crate::actor::current_process().unwrap();
    let mut process = actor.lock();
    let private_bytes = [0x41; 32];
    let public = SystemProvider.x25519_public(&private_bytes);
    let private = insert_owned_resource(
        &mut process,
        ResourceKind::X25519PrivateKey,
        Zeroizing::new(private_bytes.to_vec().into_boxed_slice()),
    )
    .unwrap();
    let (wire, client) = encapsulate_request(
        &FixedProvider::with_random(&[0x51; 32]),
        &mut process,
        37,
        &public,
        b"request",
    )
    .unwrap();
    let (_, gateway) =
        decapsulate_request(&SystemProvider, &mut process, private, 37, &wire).unwrap();
    (process.pid, client, gateway)
}

#[test]
fn m3_ohttp_ffi_null_buffers_consume_only_valid_owned_response_roles() {
    as_test_actor(|_| {
        let (owner, client, gateway) = contexts();
        assert_eq!(owned_secret_count_for_test(owner), 3);
        // Wrong role precedes null-buffer parsing and preserves both contexts.
        error(
            mesh_ohttp_encapsulate_response(client, ptr::null_mut()),
            CryptoErrorTag::InvalidKey,
        );
        error(
            mesh_ohttp_decapsulate_response(gateway, ptr::null_mut()),
            CryptoErrorTag::InvalidKey,
        );
        assert_eq!(owned_secret_count_for_test(owner), 3);
        error(
            mesh_ohttp_encapsulate_response(gateway, ptr::null_mut()),
            CryptoErrorTag::InvalidLength,
        );
        assert_eq!(owned_secret_count_for_test(owner), 2);
        error(
            mesh_ohttp_encapsulate_response(gateway, ptr::null_mut()),
            CryptoErrorTag::SecretDestroyed,
        );
        error(
            mesh_ohttp_decapsulate_response(client, ptr::null_mut()),
            CryptoErrorTag::InvalidLength,
        );
        assert_eq!(owned_secret_count_for_test(owner), 1);
        error(
            mesh_ohttp_decapsulate_response(client, ptr::null_mut()),
            CryptoErrorTag::SecretDestroyed,
        );
    });
}

#[test]
fn m3_ohttp_ffi_oversize_buffers_cannot_leave_a_response_context_reusable() {
    as_test_actor(|_| {
        let (owner, client, gateway) = contexts();
        let oversized_plaintext = bytes(&vec![0x91; 1_048_529]);
        let oversized_wire = bytes(&vec![0x92; 1_048_577]);
        error(
            mesh_ohttp_encapsulate_response(gateway, oversized_plaintext),
            CryptoErrorTag::InvalidLength,
        );
        assert_eq!(owned_secret_count_for_test(owner), 2);
        error(
            mesh_ohttp_encapsulate_response(gateway, bytes(b"retry")),
            CryptoErrorTag::SecretDestroyed,
        );
        error(
            mesh_ohttp_decapsulate_response(client, oversized_wire),
            CryptoErrorTag::InvalidLength,
        );
        assert_eq!(owned_secret_count_for_test(owner), 1);
        error(
            mesh_ohttp_decapsulate_response(client, bytes(&[0; 48])),
            CryptoErrorTag::SecretDestroyed,
        );
    });
}

#[test]
fn m3_ohttp_ffi_key_id_is_checked_without_integer_truncation_or_context_creation() {
    as_test_actor(|owner| {
        let public = SystemProvider.x25519_public(&[0x41; 32]);
        let mut recipient = MeshX25519PublicKey {
            bytes: bytes(&public),
        };
        for key_id in [-1, 256, i64::MIN, i64::MAX] {
            error(
                mesh_ohttp_encapsulate_request(key_id, &mut recipient, bytes(b"request")),
                CryptoErrorTag::InvalidLength,
            );
            assert_eq!(owned_secret_count_for_test(owner), 0);
        }
    });
}

#[test]
fn m3_ohttp_ffi_foreign_actor_refusal_preserves_the_rightful_context_even_with_null_body() {
    struct Cleanup(ProcessId);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            destroy_owned(self.0);
        }
    }
    let owner = ProcessId(712_999);
    let _cleanup = Cleanup(owner);
    let mut process = Process::new(owner, Priority::Normal);
    let private_bytes = [0x41; 32];
    let public = SystemProvider.x25519_public(&private_bytes);
    let private = insert_owned_resource(
        &mut process,
        ResourceKind::X25519PrivateKey,
        Zeroizing::new(private_bytes.to_vec().into_boxed_slice()),
    )
    .unwrap();
    let (wire, client) = encapsulate_request(
        &FixedProvider::with_random(&[0x53; 32]),
        &mut process,
        37,
        &public,
        b"request",
    )
    .unwrap();
    let (_, gateway) =
        decapsulate_request(&SystemProvider, &mut process, private, 37, &wire).unwrap();
    as_test_actor(|_| {
        error(
            mesh_ohttp_encapsulate_response(gateway, ptr::null_mut()),
            CryptoErrorTag::SecretDestroyed,
        );
        error(
            mesh_ohttp_decapsulate_response(client, ptr::null_mut()),
            CryptoErrorTag::SecretDestroyed,
        );
    });
    assert_eq!(owned_secret_count_for_test(owner), 3);
    let reply = super::ohttp::encapsulate_response(
        &FixedProvider::with_random(&[0x62; 32]),
        &process,
        gateway,
        b"reply",
    )
    .unwrap();
    assert_eq!(
        super::ohttp::decapsulate_response(&SystemProvider, &process, client, &reply)
            .unwrap()
            .as_slice(),
        b"reply"
    );
    assert_eq!(destroy_owned(owner), 1);
}
