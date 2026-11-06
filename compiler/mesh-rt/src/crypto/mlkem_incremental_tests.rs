//! Synthetic public deterministic inputs only; private data never gains a Mesh Bytes API.
use super::super::provider::FixedProvider;
use super::super::{mlkem_decapsulate_material, mlkem_encapsulate_material, mlkem_public_key};
use super::{begin, decapsulate_parts, finish, parts, CONTINUATION_BYTES, WORKSPACE_BYTES};
use crate::secret::CryptoErrorTag;

#[test]
fn c1_incremental_parts_and_two_stages_match_full_kem() {
    assert_eq!(WORKSPACE_BYTES, 2080);
    assert_eq!(CONTINUATION_BYTES, 100);
    for seed in [[0x69; 64], [0; 64], [0xff; 64]] {
        let (header, vector) = parts(&seed);
        assert_eq!(&mlkem_public_key(&seed)[..1152], vector.as_slice());
        assert_eq!(&mlkem_public_key(&seed)[1152..], &header[..32]);
        let first = begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap();
        assert_eq!(first.state.len(), 100);
        assert_eq!(first.ciphertext.len(), 960);
        let ct2 = finish(&first.state, &vector).unwrap();
        assert_eq!(ct2.len(), 128);
        let mut ciphertext = first.ciphertext.to_vec();
        ciphertext.extend_from_slice(&ct2);
        let (full, shared) = mlkem_encapsulate_material(
            &FixedProvider::with_random(&[0x73; 32]),
            mlkem_public_key(&seed).as_slice().try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(ciphertext, full);
        assert_eq!(&*first.shared, shared.as_ref());
        assert_eq!(
            &*first.shared,
            &*decapsulate_parts(&seed, &first.ciphertext, &ct2).unwrap()
        );
        assert_eq!(
            &*first.shared,
            mlkem_decapsulate_material(&seed, full.as_slice().try_into().unwrap()).as_ref()
        );
        assert_eq!(finish(&first.state, &vector).unwrap(), ct2);
    }
}

#[test]
fn c1_header_or_vector_mismatch_refuses_and_preserves_exact_retry() {
    let (header, vector) = parts(&[0x69; 64]);
    let (other_header, other_vector) = parts(&[0x6a; 64]);
    let first = begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap();
    let expected = finish(&first.state, &vector).unwrap();
    assert_eq!(
        finish(&first.state, &other_vector).unwrap_err().tag,
        CryptoErrorTag::InvalidPublicKey
    );
    let other = begin(&FixedProvider::with_random(&[0x73; 32]), &other_header).unwrap();
    assert_eq!(
        finish(&other.state, &vector).unwrap_err().tag,
        CryptoErrorTag::InvalidPublicKey
    );
    assert_eq!(finish(&first.state, &vector).unwrap(), expected);
}

#[test]
fn c1_lengths_precede_entropy_and_ciphertext_rejection_stays_implicit() {
    for length in [0, 63, 65, 1184] {
        assert_eq!(
            begin(&FixedProvider::entropy_failure(), &vec![0; length])
                .err()
                .unwrap()
                .tag,
            CryptoErrorTag::InvalidLength
        );
    }
    let seed = [0x69; 64];
    let (header, vector) = parts(&seed);
    assert_eq!(
        begin(&FixedProvider::entropy_failure(), &header)
            .err()
            .unwrap()
            .tag,
        CryptoErrorTag::EntropyUnavailable
    );
    let first = begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap();
    let ct2 = finish(&first.state, &vector).unwrap();
    for (one, two) in [
        (first.ciphertext[..959].to_vec(), ct2.to_vec()),
        (first.ciphertext.to_vec(), ct2[..127].to_vec()),
    ] {
        assert_eq!(
            decapsulate_parts(&seed, &one, &two).err().unwrap().tag,
            CryptoErrorTag::InvalidLength
        );
    }
    for position in [0, 959, 960, 1087] {
        let mut altered = first.ciphertext.to_vec();
        altered.extend_from_slice(&ct2);
        altered[position] ^= 1;
        let rejected = decapsulate_parts(&seed, &altered[..960], &altered[960..]).unwrap();
        assert_ne!(&*rejected, &*first.shared);
        assert_eq!(
            &*rejected,
            &*decapsulate_parts(&seed, &altered[..960], &altered[960..]).unwrap()
        );
    }
}

#[test]
fn c1_correct_hash_does_not_admit_noncanonical_public_coefficients() {
    let header: [u8; 64] = [
        166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166,
        166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 54, 70, 48, 243, 0,
        47, 29, 148, 80, 15, 62, 216, 118, 53, 46, 160, 236, 1, 154, 105, 127, 228, 208, 160, 125,
        143, 247, 23, 80, 219, 201, 145,
    ];
    let first = begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap();
    assert_eq!(
        finish(&first.state, &[0xff; 1152]).unwrap_err().tag,
        CryptoErrorTag::InvalidPublicKey
    );
}

#[test]
fn c1_nist_keygen_fixture_parts_and_incremental_ciphertext_match_full_path() {
    let vector: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/vectors/mlkem/mlkem768-keygen-acvp-tc26.json"
    )))
    .unwrap();
    fn decode(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    let seed: [u8; 64] = decode(
        &(vector["input"]["d"].as_str().unwrap().to_owned()
            + vector["input"]["z"].as_str().unwrap()),
    )
    .try_into()
    .unwrap();
    let expected_public = decode(vector["expected"]["public_key"].as_str().unwrap());
    let (header, public_vector) = parts(&seed);
    assert_eq!(public_vector.as_slice(), &expected_public[..1152]);
    assert_eq!(&header[..32], &expected_public[1152..]);
    for random in [[0; 32], [0xff; 32], std::array::from_fn(|i| i as u8)] {
        let first = begin(&FixedProvider::with_random(&random), &header).unwrap();
        let second = finish(&first.state, &public_vector).unwrap();
        let (full, shared) = mlkem_encapsulate_material(
            &FixedProvider::with_random(&random),
            expected_public.as_slice().try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(first.ciphertext.as_slice(), &full[..960]);
        assert_eq!(second.as_slice(), &full[960..]);
        assert_eq!(&*first.shared, shared.as_ref());
    }
}

#[test]
fn c1_incremental_roundtrip_and_retry_fit_the_runtime_actor_stack() {
    use corosensei::{stack::DefaultStack, Coroutine, CoroutineResult};
    let stack = DefaultStack::new(crate::actor::DEFAULT_STACK_SIZE).unwrap();
    let mut actor = Coroutine::<(), (), bool, _>::with_stack(stack, |_, ()| {
        let seed = [0x69; 64];
        let (header, vector) = parts(&seed);
        let first = begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap();
        let ct2 = finish(&first.state, &vector).unwrap();
        let (_, wrong) = parts(&[0x6a; 64]);
        assert_eq!(
            finish(&first.state, &wrong).err().unwrap().tag,
            CryptoErrorTag::InvalidPublicKey
        );
        assert_eq!(finish(&first.state, &vector).unwrap(), ct2);
        &*decapsulate_parts(&seed, &first.ciphertext, &ct2).unwrap() == &*first.shared
    });
    assert!(matches!(actor.resume(()), CoroutineResult::Return(true)));
}

#[test]
fn c1_public_validation_exact_parts_lengths_binding_and_modulus() {
    use super::validate_public_parts;
    for seed in [[0; 64], [0x69; 64], [0xff; 64]] {
        let (header, vector) = parts(&seed);
        validate_public_parts(&header, &vector).unwrap();
        for n in [0, 63, 65, 1184] {
            assert_eq!(
                validate_public_parts(&vec![0; n], &vector).unwrap_err().tag,
                CryptoErrorTag::InvalidLength
            );
        }
        for n in [0, 1151, 1153, 1184] {
            assert_eq!(
                validate_public_parts(&header, &vec![0; n]).unwrap_err().tag,
                CryptoErrorTag::InvalidLength
            );
        }
        let (_, other) = parts(&[0x6a; 64]);
        assert_eq!(
            validate_public_parts(&header, &other).unwrap_err().tag,
            CryptoErrorTag::InvalidPublicKey
        );
    }
    // Same correct-hash corpus as c1_correct_hash_does_not_admit_noncanonical_public_coefficients.
    // rho32=0xa6, vector1152=0xff; coefficients4095 fail even though H(ek) matches.
    let header: [u8; 64] = [
        166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166,
        166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 54, 70, 48, 243, 0,
        47, 29, 148, 80, 15, 62, 216, 118, 53, 46, 160, 236, 1, 154, 105, 127, 228, 208, 160, 125,
        143, 247, 23, 80, 219, 201, 145,
    ];
    assert_eq!(
        validate_public_parts(&header, &[0xff; 1152])
            .unwrap_err()
            .tag,
        CryptoErrorTag::InvalidPublicKey
    );
}
