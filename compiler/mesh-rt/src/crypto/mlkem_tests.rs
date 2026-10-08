//! ML-KEM-768 provider vectors: every NIST ACVP FIPS 203 ML-KEM-768 key
//! generation, encapsulation and decapsulation case, and a differential
//! against OpenSSL, run through the runtime's own ML-KEM helpers.
//! `scripts/generate-mlkem-vectors.sh` regenerates both files.

use super::provider::FixedProvider;
use super::{
    mlkem_decapsulate_expanded, mlkem_decapsulate_material, mlkem_encapsulate_material,
    mlkem_expand, mlkem_public_key,
};
use libcrux_ml_kem::mlkem768::MlKem768PrivateKey;
use serde_json::Value;

fn document(json: &str) -> Value {
    serde_json::from_str(json).expect("ML-KEM vector file")
}

fn hex(value: &Value) -> Vec<u8> {
    crate::bytes::decode_hex(value.as_str().expect("a hex string")).expect("hex")
}

fn encapsulate(public_key: &[u8], m: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (ciphertext, shared_secret) = mlkem_encapsulate_material(
        &FixedProvider::with_random(m),
        public_key.try_into().expect("a 1,184-byte public key"),
    )
    .unwrap_or_else(|_| panic!("encapsulation failed"));
    (ciphertext, shared_secret.to_vec())
}

fn decapsulate(seed: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    mlkem_decapsulate_material(
        seed,
        ciphertext.try_into().expect("a 1,088-byte ciphertext"),
    )
    .to_vec()
}

#[test]
fn nist_acvp_mlkem768_key_generation_encapsulation_and_decapsulation_match() {
    let vectors = document(include_str!(
        "../../../../tests/vectors/mlkem/mlkem768-acvp-fips203.json"
    ));
    assert_eq!(vectors["suite"], "ML-KEM-768");

    let key_generation = vectors["key_generation"].as_array().expect("cases");
    assert_eq!(key_generation.len(), 25);
    for case in key_generation {
        let seed = [hex(&case["d"]), hex(&case["z"])].concat();
        let expanded = mlkem_expand(&seed);
        assert_eq!(
            expanded.public_key.as_slice()[..],
            hex(&case["ek"])[..],
            "tcId {}",
            case["tcId"]
        );
        assert_eq!(
            expanded.private_key.as_slice()[..],
            hex(&case["dk"])[..],
            "tcId {}",
            case["tcId"]
        );
        assert_eq!(mlkem_public_key(&seed), hex(&case["ek"]));
    }

    let encapsulation = vectors["encapsulation"].as_array().expect("cases");
    assert_eq!(encapsulation.len(), 25);
    for case in encapsulation {
        assert_eq!(
            encapsulate(&hex(&case["ek"]), &hex(&case["m"])),
            (hex(&case["c"]), hex(&case["k"])),
            "tcId {}",
            case["tcId"]
        );
    }

    let decapsulation = &vectors["decapsulation"];
    let private_key = MlKem768PrivateKey::try_from(&hex(&decapsulation["dk"])[..]).expect("dk");
    let cases = decapsulation["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 10);
    assert!(cases
        .iter()
        .any(|case| case["reason"] == "modify ciphertext"));
    for case in cases {
        let ciphertext = hex(&case["c"]);
        let shared_secret = mlkem_decapsulate_expanded(
            &private_key,
            ciphertext[..].try_into().expect("a 1,088-byte ciphertext"),
        );
        assert_eq!(
            shared_secret[..],
            hex(&case["k"])[..],
            "tcId {}",
            case["tcId"]
        );
    }
}

/// OpenSSL made these from random seeds: its public key for each seed, its
/// ciphertext and shared secret for a fixed m, and what it decapsulates a
/// ciphertext with one flipped bit to (implicit rejection).
#[test]
fn mlkem768_agrees_with_openssl() {
    let vectors = document(include_str!(
        "../../../../tests/vectors/mlkem/mlkem768-openssl.json"
    ));
    let cases = vectors["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let seed = hex(&case["seed"]);
        let public_key = mlkem_public_key(&seed);
        assert_eq!(public_key, hex(&case["ek"]));
        let (ciphertext, shared_secret) = encapsulate(&public_key, &hex(&case["m"]));
        assert_eq!(
            (ciphertext.clone(), shared_secret.clone()),
            (hex(&case["c"]), hex(&case["k"]))
        );
        assert_eq!(decapsulate(&seed, &ciphertext), shared_secret);
        let rejected = decapsulate(&seed, &hex(&case["tampered_c"]));
        assert_eq!(rejected, hex(&case["rejected_k"]));
        assert_ne!(rejected, shared_secret);
    }
}
