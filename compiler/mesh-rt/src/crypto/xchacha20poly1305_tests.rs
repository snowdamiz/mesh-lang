//! Published XChaCha draft A.3.1 vector; the API uses the existing opaque AeadKey.
//! https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03#appendix-A.3.1
//! The source is an expired Internet-Draft, not an IETF standard.
use super::*;
use crate::actor::{Priority, ProcessId};
use crate::secret::destroy_owned;
fn vector() -> ([u8; 32], [u8; 24], Vec<u8>, Vec<u8>, Vec<u8>) {
    let key = std::array::from_fn(|i| 0x80 + i as u8);
    let nonce = std::array::from_fn(|i| 0x40 + i as u8);
    let aad = decode_hex("50515253c0c1c2c3c4c5c6c7").unwrap();
    let body=b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".to_vec();
    let expected = decode_hex(concat!(
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb",
        "731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452",
        "2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9",
        "21f9664c97637da9768812f615c68b13b52e",
        "c0875924c1c7987947deafd8780acf49"
    ))
    .unwrap();
    (key, nonce, aad, body, expected)
}
#[test]
fn xchacha20poly1305_provider_matches_published_known_answer_and_authenticates_all_inputs() {
    let (key, nonce, aad, body, expected) = vector();
    assert_eq!(
        SystemProvider
            .xchacha20poly1305_seal(&key, &nonce, &aad, &body)
            .unwrap(),
        expected
    );
    let mut opened = Zeroizing::new(expected.clone());
    SystemProvider
        .xchacha20poly1305_open(&key, &nonce, &aad, &mut opened)
        .unwrap();
    assert_eq!(&*opened, &body);
    for which in 0..5 {
        let mut changed = expected.clone();
        let mut n = nonce;
        let mut k = key;
        let mut a = aad.clone();
        match which {
            0 => changed[0] ^= 1,
            1 => n[23] ^= 1,
            2 => k[0] ^= 1,
            3 => a[0] ^= 1,
            _ => {
                changed.truncate(15);
            }
        }
        let mut output = Zeroizing::new(changed);
        assert_eq!(
            SystemProvider.xchacha20poly1305_open(&k, &n, &a, &mut output),
            Err(ProviderError::AuthenticationFailed)
        );
        assert!(
            output.iter().all(|v| *v == 0),
            "unauthenticated output retained"
        );
    }
}
#[test]
fn xchacha20poly1305_provider_preserves_bounds_empty_input_and_failure_zeroization() {
    for length in [0, 1, 65536, MAX_AEAD_PLAINTEXT_BYTES] {
        let input = vec![7; length];
        let encrypted = SystemProvider
            .xchacha20poly1305_seal(&[9; 32], &[3; 24], b"aad", &input)
            .unwrap();
        assert_eq!(encrypted.len(), length + 16);
        let mut output = Zeroizing::new(encrypted);
        SystemProvider
            .xchacha20poly1305_open(&[9; 32], &[3; 24], b"aad", &mut output)
            .unwrap();
        assert_eq!(&*output, &input);
    }
    assert_eq!(
        SystemProvider.xchacha20poly1305_seal(
            &[9; 32],
            &[3; 24],
            b"aad",
            &vec![0; MAX_AEAD_PLAINTEXT_BYTES + 1]
        ),
        Err(ProviderError::InvalidLength)
    );
    let mut output = Zeroizing::new(vec![0xa5; MAX_AEAD_CIPHERTEXT_BYTES + 1]);
    assert_eq!(
        SystemProvider.xchacha20poly1305_open(&[9; 32], &[3; 24], b"aad", &mut output),
        Err(ProviderError::InvalidLength)
    );
    assert!(output.iter().all(|v| *v == 0));
    let mut output = Zeroizing::new(vec![0xa5; 16]);
    assert_eq!(
        SystemProvider.xchacha20poly1305_open(&[9; 32], &[3; 24], &vec![0; 131073], &mut output),
        Err(ProviderError::InvalidLength)
    );
    assert!(output.iter().all(|v| *v == 0));
}
fn bytes(input: &[u8]) -> *mut MeshBytes {
    mesh_bytes_new(input.as_ptr(), input.len() as u64)
}
#[test]
fn xchacha20poly1305_public_runtime_requires_24_byte_nonce_and_live_borrowed_owner_key() {
    crate::mesh_rt_init();
    let owner = ProcessId(78_524);
    let mut process = Process::new(owner, Priority::Normal);
    let (key, nonce, aad, body, expected) = vector();
    let resource = insert_owned_resource(
        &mut process,
        ResourceKind::AeadKey,
        Zeroizing::new(key.to_vec().into_boxed_slice()),
    )
    .unwrap();
    let encrypted = xchacha20poly1305_seal_for_process(
        &process,
        &SystemProvider,
        resource,
        bytes(&nonce),
        bytes(&aad),
        bytes(&body),
    )
    .unwrap();
    assert_eq!(encrypted, expected);
    assert_eq!(
        &*xchacha20poly1305_open_for_process(
            &process,
            &SystemProvider,
            resource,
            bytes(&nonce),
            bytes(&aad),
            bytes(&expected)
        )
        .unwrap(),
        &body
    );
    for length in [0, 12, 23, 25] {
        for opening in [false, true] {
            let result = if opening {
                xchacha20poly1305_open_for_process(
                    &process,
                    &SystemProvider,
                    resource,
                    bytes(&vec![0; length]),
                    bytes(&aad),
                    bytes(&expected),
                )
                .map(|v| v.to_vec())
            } else {
                xchacha20poly1305_seal_for_process(
                    &process,
                    &SystemProvider,
                    resource,
                    bytes(&vec![0; length]),
                    bytes(&aad),
                    bytes(&body),
                )
            };
            assert!(
                matches!(result,Err(CryptoFailure{tag:CryptoErrorTag::InvalidLength,expected:24,actual,..}) if actual==length as i64)
            );
        }
    }
    let wrong = Process::new(ProcessId(78_525), Priority::Normal);
    assert!(matches!(
        xchacha20poly1305_seal_for_process(
            &wrong,
            &SystemProvider,
            resource,
            bytes(&nonce),
            bytes(&aad),
            bytes(&body)
        ),
        Err(CryptoFailure {
            tag: CryptoErrorTag::SecretDestroyed,
            ..
        })
    ));
    assert_eq!(destroy_owned(owner), 1);
    assert!(matches!(
        xchacha20poly1305_open_for_process(
            &process,
            &SystemProvider,
            resource,
            bytes(&nonce),
            bytes(&aad),
            bytes(&expected)
        ),
        Err(CryptoFailure {
            tag: CryptoErrorTag::SecretDestroyed,
            ..
        })
    ));
}
