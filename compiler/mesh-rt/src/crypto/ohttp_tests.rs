//! Runtime-only OHTTP contract tests. All key material here is synthetic.
//! These tests intentionally precede the internal implementation and expose no
//! Mesh/FFI API, resource serialization, or cross-actor transport.

use super::ohttp::{
    decapsulate_request, decapsulate_response, encapsulate_request, encapsulate_response,
};
use super::provider::{CryptoProvider, FixedProvider, ProviderError, SystemProvider};
use crate::actor::{Priority, Process, ProcessId};
use crate::secret::{
    as_test_actor, destroy_owned, insert_owned_resource, owned_secret_count_for_test,
    with_owned_resource, CryptoErrorTag, MeshSecretHandle, ResourceKind,
};
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroizing;

const WIRE_LIMIT: usize = 1_048_576;
const REQUEST_OVERHEAD: usize = 55;
const RESPONSE_OVERHEAD: usize = 48;
const KEY_ID: u8 = 37;
const HEADER: [u8; 7] = [KEY_ID, 0, 32, 0, 1, 0, 3];
static NEXT_OWNER: AtomicU64 = AtomicU64::new(712_000);

struct Owner(Process);

impl Owner {
    fn new() -> Self {
        Self(Process::new(
            ProcessId(NEXT_OWNER.fetch_add(1, Ordering::Relaxed)),
            Priority::Normal,
        ))
    }

    fn private_key(&mut self) -> (*mut MeshSecretHandle, [u8; 32]) {
        let private = [0x41; 32];
        let public = SystemProvider.x25519_public(&private);
        let handle = insert_owned_resource(
            &mut self.0,
            ResourceKind::X25519PrivateKey,
            Zeroizing::new(private.to_vec().into_boxed_slice()),
        )
        .unwrap();
        (handle, public)
    }

    fn count(&self) -> usize {
        owned_secret_count_for_test(self.0.pid)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        destroy_owned(self.0.pid);
    }
}

fn request(
    client: &mut Owner,
    public: &[u8; 32],
    seed: u8,
    plaintext: &[u8],
) -> (Vec<u8>, *mut MeshSecretHandle) {
    encapsulate_request(
        &FixedProvider::with_random(&[seed; 32]),
        &mut client.0,
        KEY_ID,
        public,
        plaintext,
    )
    .unwrap()
}

#[test]
fn m3_ohttp_request_reply_is_bound_and_each_role_is_consumed_once() {
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    let plaintext = b"\0binary request\xff";
    let (wire, client_context) = request(&mut client, &public, 0x51, plaintext);
    assert_eq!(&wire[..7], &HEADER);
    assert_eq!(wire.len(), plaintext.len() + REQUEST_OVERHEAD);
    assert_eq!(ResourceKind::MlKemEncapsulationState as u32, 12);
    assert_eq!(ResourceKind::OhttpClientContext as u32, 13);
    assert_eq!(ResourceKind::OhttpGatewayContext as u32, 14);
    let (opened, gateway_context) =
        decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &wire).unwrap();
    assert_eq!(opened.as_slice(), plaintext);
    for (owner, context, kind) in [
        (&client, client_context, ResourceKind::OhttpClientContext),
        (&gateway, gateway_context, ResourceKind::OhttpGatewayContext),
    ] {
        assert_eq!(
            with_owned_resource(&owner.0, context, kind, |material| material.len()).unwrap(),
            64,
            "only the response secret and encapsulated public key are retained"
        );
    }
    let response = b"\0binary reply\xfe";
    let reply = encapsulate_response(
        &FixedProvider::with_random(&[0x62; 32]),
        &gateway.0,
        gateway_context,
        response,
    )
    .unwrap();
    assert_eq!(reply.len(), response.len() + RESPONSE_OVERHEAD);
    assert_eq!(gateway.count(), 1, "the borrowed gateway key remains");
    assert_eq!(
        decapsulate_response(&SystemProvider, &client.0, client_context, &reply)
            .unwrap()
            .as_slice(),
        response
    );
    assert_eq!(client.count(), 0);
    assert_eq!(
        encapsulate_response(&SystemProvider, &gateway.0, gateway_context, response)
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(
        decapsulate_response(&SystemProvider, &client.0, client_context, &reply)
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
}

#[test]
fn m3_ohttp_foreign_actor_and_wrong_role_do_not_consume_authorized_contexts() {
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let outsider = Owner::new();
    let (private, public) = gateway.private_key();
    let (wire, client_context) = request(&mut client, &public, 0x52, b"request");
    let (_, gateway_context) =
        decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &wire).unwrap();
    assert_eq!(
        encapsulate_response(&SystemProvider, &outsider.0, gateway_context, b"reply")
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(
        decapsulate_response(&SystemProvider, &gateway.0, gateway_context, b"")
            .unwrap_err()
            .tag,
        CryptoErrorTag::InvalidKey
    );
    assert_eq!(
        encapsulate_response(&SystemProvider, &client.0, client_context, b"reply")
            .unwrap_err()
            .tag,
        CryptoErrorTag::InvalidKey
    );
    assert_eq!(
        (client.count(), gateway.count(), outsider.count()),
        (1, 2, 0)
    );
    let reply = encapsulate_response(
        &FixedProvider::with_random(&[0x63; 32]),
        &gateway.0,
        gateway_context,
        b"reply",
    )
    .unwrap();
    assert_eq!(
        decapsulate_response(&SystemProvider, &outsider.0, client_context, &reply)
            .unwrap_err()
            .tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(
        decapsulate_response(&SystemProvider, &client.0, client_context, &reply)
            .unwrap()
            .as_slice(),
        b"reply"
    );
}

#[test]
fn m3_ohttp_valid_contexts_are_consumed_before_all_fallible_response_work() {
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    for (index, malformed) in [vec![], vec![0; 47], vec![0; WIRE_LIMIT + 1]]
        .into_iter()
        .enumerate()
    {
        let (_, context) = request(&mut client, &public, index as u8 + 1, b"request");
        assert_eq!(
            decapsulate_response(&SystemProvider, &client.0, context, &malformed)
                .unwrap_err()
                .tag,
            CryptoErrorTag::InvalidLength
        );
        assert_eq!(client.count(), 0);
        assert_eq!(
            decapsulate_response(&SystemProvider, &client.0, context, &malformed)
                .unwrap_err()
                .tag,
            CryptoErrorTag::SecretDestroyed
        );
    }
    for (seed, oversized) in [(11, false), (12, true)] {
        let (wire, client_context) = request(&mut client, &public, seed, b"request");
        let (_, context) =
            decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &wire).unwrap();
        let reply = vec![
            0;
            if oversized {
                WIRE_LIMIT - RESPONSE_OVERHEAD + 1
            } else {
                1
            }
        ];
        let error = encapsulate_response(
            &FixedProvider::entropy_failure(),
            &gateway.0,
            context,
            &reply,
        )
        .unwrap_err();
        assert_eq!(
            error.tag,
            if oversized {
                CryptoErrorTag::InvalidLength
            } else {
                CryptoErrorTag::EntropyUnavailable
            }
        );
        assert_eq!(gateway.count(), 1);
        assert_eq!(
            encapsulate_response(&SystemProvider, &gateway.0, context, b"retry")
                .unwrap_err()
                .tag,
            CryptoErrorTag::SecretDestroyed
        );
        assert_eq!(
            decapsulate_response(&SystemProvider, &client.0, client_context, &[])
                .unwrap_err()
                .tag,
            CryptoErrorTag::InvalidLength
        );
    }
}

#[test]
fn m3_ohttp_rejects_mismatched_key_suite_and_tampered_requests_without_context() {
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    let (wire, _) = request(&mut client, &public, 0x53, b"request");
    for (position, expected) in [
        (0, CryptoErrorTag::InvalidKey),
        (1, CryptoErrorTag::UnsupportedOperation),
        (2, CryptoErrorTag::UnsupportedOperation),
        (3, CryptoErrorTag::UnsupportedOperation),
        (4, CryptoErrorTag::UnsupportedOperation),
        (5, CryptoErrorTag::UnsupportedOperation),
        (6, CryptoErrorTag::UnsupportedOperation),
        (7, CryptoErrorTag::AuthenticationFailed),
        (wire.len() - 1, CryptoErrorTag::AuthenticationFailed),
    ] {
        let mut tampered = wire.clone();
        tampered[position] ^= 1;
        assert_eq!(
            decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &tampered)
                .err()
                .unwrap()
                .tag,
            expected,
            "tampered byte {position}"
        );
        assert_eq!(gateway.count(), 1, "failure published a gateway context");
    }
    let mut zero_enc = wire.clone();
    zero_enc[7..39].fill(0);
    assert_eq!(
        decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &zero_enc)
            .err()
            .unwrap()
            .tag,
        CryptoErrorTag::InvalidPublicKey
    );
    for malformed in [vec![], wire[..54].to_vec(), vec![0; WIRE_LIMIT + 1]] {
        assert_eq!(
            decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &malformed)
                .err()
                .unwrap()
                .tag,
            CryptoErrorTag::InvalidLength
        );
        assert_eq!(gateway.count(), 1);
    }
}

#[test]
fn m3_ohttp_reply_cannot_move_between_requests_and_auth_failure_consumes_context() {
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    let (first, first_context) = request(&mut client, &public, 0x54, b"same body");
    let (second, second_context) = request(&mut client, &public, 0x55, b"same body");
    assert_ne!(&first[7..39], &second[7..39]);
    let (_, gateway_context) =
        decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &first).unwrap();
    let reply = encapsulate_response(
        &FixedProvider::with_random(&[0x64; 32]),
        &gateway.0,
        gateway_context,
        b"response",
    )
    .unwrap();
    assert_eq!(
        decapsulate_response(&SystemProvider, &client.0, second_context, &reply)
            .unwrap_err()
            .tag,
        CryptoErrorTag::AuthenticationFailed
    );
    assert_eq!(client.count(), 1);
    let mut tampered = reply;
    tampered[0] ^= 1;
    assert_eq!(
        decapsulate_response(&SystemProvider, &client.0, first_context, &tampered)
            .unwrap_err()
            .tag,
        CryptoErrorTag::AuthenticationFailed
    );
    assert_eq!(client.count(), 0);
}

#[test]
fn m3_ohttp_complete_wire_caps_are_exact_and_do_not_widen_existing_crypto() {
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    let large_request = vec![0xa5; WIRE_LIMIT - REQUEST_OVERHEAD];
    let (wire, client_context) = request(&mut client, &public, 0x56, &large_request);
    assert_eq!(wire.len(), WIRE_LIMIT);
    let (opened, gateway_context) =
        decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &wire).unwrap();
    assert_eq!(opened.as_slice(), large_request);
    let large_response = vec![0x5a; WIRE_LIMIT - RESPONSE_OVERHEAD];
    let reply = encapsulate_response(
        &FixedProvider::with_random(&[0x65; 32]),
        &gateway.0,
        gateway_context,
        &large_response,
    )
    .unwrap();
    assert_eq!(reply.len(), WIRE_LIMIT);
    assert_eq!(
        decapsulate_response(&SystemProvider, &client.0, client_context, &reply)
            .unwrap()
            .as_slice(),
        large_response
    );
    assert_eq!(
        encapsulate_request(
            &FixedProvider::entropy_failure(),
            &mut client.0,
            KEY_ID,
            &public,
            &vec![0; WIRE_LIMIT - REQUEST_OVERHEAD + 1],
        )
        .err()
        .unwrap()
        .tag,
        CryptoErrorTag::InvalidLength,
        "preflight refuses before entropy and before publishing a context"
    );
    assert_eq!(client.count(), 0);
    assert_eq!(super::MAX_INPUT_BYTES, 65_536);
    assert_eq!(super::MAX_HPKE_SEALED_BYTES, 65_584);
    assert_eq!(super::provider::MAX_AEAD_PLAINTEXT_BYTES, 131_072);
    assert_eq!(
        SystemProvider
            .chacha20poly1305_seal(&[0; 32], &[0; 12], &[], &vec![0; 131_073])
            .unwrap_err(),
        ProviderError::InvalidLength
    );
    assert_eq!(
        super::hpke_open_material(&SystemProvider, &[0x41; 32], &[], &[], &vec![0; 65_585])
            .unwrap_err()
            .tag,
        CryptoErrorTag::InvalidLength
    );
}

#[test]
fn m3_ohttp_empty_messages_and_measured_resolve_sized_reply_are_supported() {
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    for size in [0, 570_418] {
        let (wire, context) = request(&mut client, &public, 0x57, &[]);
        assert_eq!(wire.len(), REQUEST_OVERHEAD);
        let (opened, gateway_context) =
            decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &wire).unwrap();
        assert!(opened.is_empty());
        let body: Vec<_> = (0..size).map(|i| (i % 251) as u8).collect();
        let reply = encapsulate_response(
            &FixedProvider::with_random(&[0x66; 32]),
            &gateway.0,
            gateway_context,
            &body,
        )
        .unwrap();
        assert_eq!(reply.len(), size + RESPONSE_OVERHEAD);
        assert_eq!(
            decapsulate_response(&SystemProvider, &client.0, context, &reply)
                .unwrap()
                .as_slice(),
            body
        );
    }
}

#[test]
fn m3_ohttp_private_key_destroy_during_provider_call_cannot_publish_plaintext_or_context() {
    struct DestroyDuringDh(ProcessId);
    impl CryptoProvider for DestroyDuringDh {
        fn fill_random(&self, output: &mut [u8]) -> Result<(), ProviderError> {
            SystemProvider.fill_random(output)
        }
        fn x25519_shared(
            &self,
            private: &[u8; 32],
            public: &[u8; 32],
            output: &mut [u8; 32],
        ) -> Result<(), ProviderError> {
            let result = SystemProvider.x25519_shared(private, public, output);
            destroy_owned(self.0);
            result
        }
    }
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    let (wire, _) = request(&mut client, &public, 0x58, b"must not escape");
    let owner = gateway.0.pid;
    assert_eq!(
        decapsulate_request(
            &DestroyDuringDh(owner),
            &mut gateway.0,
            private,
            KEY_ID,
            &wire
        )
        .err()
        .unwrap()
        .tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(gateway.count(), 0);
}

#[test]
fn m3_ohttp_contexts_follow_existing_actor_exit_erasure() {
    let owner = as_test_actor(|owner| {
        let actor = crate::actor::current_process().unwrap();
        let mut process = actor.lock();
        let public = SystemProvider.x25519_public(&[0x41; 32]);
        let (_, context) = encapsulate_request(
            &FixedProvider::with_random(&[0x59; 32]),
            &mut process,
            KEY_ID,
            &public,
            b"request",
        )
        .unwrap();
        assert!(with_owned_resource(
            &process,
            context,
            ResourceKind::OhttpClientContext,
            |bytes| bytes.len() == 64,
        )
        .unwrap());
        assert_eq!(owned_secret_count_for_test(owner), 1);
        owner
    });
    assert_eq!(owned_secret_count_for_test(owner), 0);
}

#[test]
fn m3_ohttp_both_context_roles_use_the_existing_idempotent_resource_destructor() {
    let owner = as_test_actor(|owner| {
        let actor = crate::actor::current_process().unwrap();
        let (client_context, gateway_context) = {
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
                &FixedProvider::with_random(&[0x70; 32]),
                &mut process,
                KEY_ID,
                &public,
                b"request",
            )
            .unwrap();
            let (_, gateway) =
                decapsulate_request(&SystemProvider, &mut process, private, KEY_ID, &wire).unwrap();
            (client, gateway)
        };
        assert_eq!(owned_secret_count_for_test(owner), 3);
        crate::secret::mesh_resource_destroy(client_context);
        crate::secret::mesh_resource_destroy(client_context);
        assert_eq!(owned_secret_count_for_test(owner), 2);
        crate::secret::mesh_resource_destroy(gateway_context);
        crate::secret::mesh_resource_destroy(gateway_context);
        assert_eq!(owned_secret_count_for_test(owner), 1);
        owner
    });
    assert_eq!(owned_secret_count_for_test(owner), 0);
}

#[test]
fn m3_ohttp_provider_failures_never_publish_or_retain_response_contexts() {
    enum Fault {
        Hmac,
        Seal,
        Open,
    }
    impl CryptoProvider for Fault {
        fn fill_random(&self, output: &mut [u8]) -> Result<(), ProviderError> {
            output.fill(0x71);
            Ok(())
        }
        fn hmac_sha256(
            &self,
            key: &[u8],
            message: &[u8],
            output: &mut [u8; 32],
        ) -> Result<(), ProviderError> {
            if matches!(self, Self::Hmac) {
                output.fill(0);
                Err(ProviderError::InternalFailure)
            } else {
                SystemProvider.hmac_sha256(key, message, output)
            }
        }
        fn ohttp_chacha20poly1305_seal(
            &self,
            key: &[u8; 32],
            nonce: &[u8; 12],
            plaintext: &[u8],
        ) -> Result<Vec<u8>, ProviderError> {
            if matches!(self, Self::Seal) {
                Err(ProviderError::InternalFailure)
            } else {
                SystemProvider.ohttp_chacha20poly1305_seal(key, nonce, plaintext)
            }
        }
        fn ohttp_chacha20poly1305_open(
            &self,
            key: &[u8; 32],
            nonce: &[u8; 12],
            ciphertext: &mut Zeroizing<Vec<u8>>,
        ) -> Result<(), ProviderError> {
            if matches!(self, Self::Open) {
                Err(ProviderError::InternalFailure)
            } else {
                SystemProvider.ohttp_chacha20poly1305_open(key, nonce, ciphertext)
            }
        }
    }
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let (private, public) = gateway.private_key();
    assert_eq!(
        encapsulate_request(&Fault::Seal, &mut client.0, KEY_ID, &public, b"request")
            .err()
            .unwrap()
            .tag,
        CryptoErrorTag::InternalFailure
    );
    assert_eq!(client.count(), 0);
    assert_eq!(
        encapsulate_request(
            &FixedProvider::entropy_failure(),
            &mut client.0,
            KEY_ID,
            &public,
            b"request",
        )
        .err()
        .unwrap()
        .tag,
        CryptoErrorTag::EntropyUnavailable
    );
    assert_eq!(client.count(), 0);
    let (wire, context) = request(&mut client, &public, 0x72, b"request");
    assert_eq!(
        decapsulate_request(&Fault::Open, &mut gateway.0, private, KEY_ID, &wire)
            .err()
            .unwrap()
            .tag,
        CryptoErrorTag::InternalFailure
    );
    assert_eq!(gateway.count(), 1);
    decapsulate_response(&SystemProvider, &client.0, context, &[]).unwrap_err();
    for (seed, fault) in [
        (0x73, Fault::Hmac),
        (0x74, Fault::Seal),
        (0x75, Fault::Open),
    ] {
        let (wire, client_context) = request(&mut client, &public, seed, b"request");
        let (_, gateway_context) =
            decapsulate_request(&SystemProvider, &mut gateway.0, private, KEY_ID, &wire).unwrap();
        if matches!(fault, Fault::Open) {
            let reply = encapsulate_response(
                &FixedProvider::with_random(&[0x76; 32]),
                &gateway.0,
                gateway_context,
                b"reply",
            )
            .unwrap();
            assert_eq!(
                decapsulate_response(&fault, &client.0, client_context, &reply)
                    .unwrap_err()
                    .tag,
                CryptoErrorTag::InternalFailure
            );
        } else {
            assert_eq!(
                encapsulate_response(&fault, &gateway.0, gateway_context, b"reply")
                    .unwrap_err()
                    .tag,
                CryptoErrorTag::InternalFailure
            );
            decapsulate_response(&SystemProvider, &client.0, client_context, &[]).unwrap_err();
        }
        assert_eq!((client.count(), gateway.count()), (0, 1));
    }
}

#[test]
fn m3_ohttp_dedicated_provider_refuses_oversize_and_clears_failed_open_buffers() {
    let key = [0x81; 32];
    let nonce = [0x82; 12];
    let plaintext = vec![0x83; WIRE_LIMIT - RESPONSE_OVERHEAD];
    let mut ciphertext = Zeroizing::new(
        SystemProvider
            .ohttp_chacha20poly1305_seal(&key, &nonce, &plaintext)
            .unwrap(),
    );
    assert_eq!(ciphertext.len(), WIRE_LIMIT - 32);
    ciphertext[0] ^= 1;
    assert_eq!(
        SystemProvider.ohttp_chacha20poly1305_open(&key, &nonce, &mut ciphertext),
        Err(ProviderError::AuthenticationFailed)
    );
    assert!(ciphertext.iter().all(|b| *b == 0));
    for (length, expected) in [
        (15, ProviderError::AuthenticationFailed),
        (WIRE_LIMIT - 31, ProviderError::InvalidLength),
    ] {
        let mut ciphertext = Zeroizing::new(vec![0x84; length]);
        assert_eq!(
            SystemProvider.ohttp_chacha20poly1305_open(&key, &nonce, &mut ciphertext),
            Err(expected)
        );
        assert!(ciphertext.iter().all(|b| *b == 0));
    }
    assert_eq!(
        SystemProvider
            .ohttp_chacha20poly1305_seal(
                &key,
                &nonce,
                &vec![0x85; WIRE_LIMIT - RESPONSE_OVERHEAD + 1],
            )
            .unwrap_err(),
        ProviderError::InvalidLength
    );
}

#[test]
fn m3_ohttp_matches_independent_selected_suite_complete_wires() {
    // Independent OHTTP commit fbfa023be6191658d410e7583c7d5378aaed1e2e,
    // rust-hpke0.14.0; synthetic fixture generated before Mesh comparison.
    // Full fixture SHA256: f99d1fb9c8222332628ff235f16c5d143e5a8c9b14cb1b827a2f4b6f85ed9b2b.
    let bytes = |hex: &str| crate::bytes::decode_hex(hex).unwrap();
    let gateway_private: [u8; 32] =
        bytes("ef0106ca50fd40bd29f8e1cfb6079fd37b4052d3bf4ce20b616aff8dd2bd8c5f")
            .try_into()
            .unwrap();
    let gateway_public: [u8; 32] =
        bytes("fd2c4dd1c8a6b88fe1fc59ce441398f5ea83a9296e210997ac63bed970b86028")
            .try_into()
            .unwrap();
    let ephemeral_ikm: [u8; 32] =
        bytes("5c0303235d2c0d4cc6f7426feb81674ce933994fe45d3dde42b0a98ee2bd71c2")
            .try_into()
            .unwrap();
    let response_nonce: [u8; 32] =
        bytes("6262626262626262626262626262626262626262626262626262626262626262")
            .try_into()
            .unwrap();
    let request_plaintext = bytes("00696e646570656e64656e742062696e6172792072657175657374ff");
    let response_plaintext = bytes("00696e646570656e64656e742062696e61727920726573706f6e7365fe");
    let request_wire = bytes("25002000010003f0475110cd1e58fa47525136677c4a9febd94b41a123f4adcd1f7eaaef469159ac8dfab44fa4ecc1f50d28e64a4b9b6f97a1ede786789fb17cb6c5835b8bed25f82e069581c300b3ea82e8fb");
    let response_wire = bytes("62626262626262626262626262626262626262626262626262626262626262622b926a4f9bd5a54318844606885532f5207bca9b70fa93ee2839ed0ca695c1d8a14e6c2d1fb0fca3f3e8bf35ac");
    assert_eq!(
        SystemProvider.x25519_public(&gateway_private),
        gateway_public
    );
    let mut client = Owner::new();
    let mut gateway = Owner::new();
    let private = insert_owned_resource(
        &mut gateway.0,
        ResourceKind::X25519PrivateKey,
        Zeroizing::new(gateway_private.to_vec().into_boxed_slice()),
    )
    .unwrap();
    let (actual_request, client_context) = encapsulate_request(
        &FixedProvider::with_random(&ephemeral_ikm),
        &mut client.0,
        KEY_ID,
        &gateway_public,
        &request_plaintext,
    )
    .unwrap();
    assert_eq!(
        actual_request, request_wire,
        "independent complete request wire"
    );
    let (opened, gateway_context) = decapsulate_request(
        &SystemProvider,
        &mut gateway.0,
        private,
        KEY_ID,
        &request_wire,
    )
    .unwrap();
    assert_eq!(opened.as_slice(), request_plaintext);
    let actual_response = encapsulate_response(
        &FixedProvider::with_random(&response_nonce),
        &gateway.0,
        gateway_context,
        &response_plaintext,
    )
    .unwrap();
    assert_eq!(
        actual_response, response_wire,
        "independent complete response wire"
    );
    assert_eq!(
        decapsulate_response(&SystemProvider, &client.0, client_context, &response_wire)
            .unwrap()
            .as_slice(),
        response_plaintext,
    );
    assert_eq!((client.count(), gateway.count()), (0, 1));
}
