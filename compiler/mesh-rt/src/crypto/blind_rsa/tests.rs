//! Known answers (RFC 9474 A.3 at 4,096 bits, RFC 9578 A.2 at 2,048 bits),
//! negative and bounds cases for the blind RSA helpers.

// `expect_err` would print the accepted value, which can be key material.
#![allow(clippy::err_expect)]

use super::*;
use crate::crypto::provider::{FixedProvider, SystemProvider};
use crypto_bigint::U4096;
use serde_json::Value;

const RFC9474_A3: &str =
    include_str!("../../../../../tests/vectors/blind-rsa/rfc9474-a3-pss-deterministic.json");
const RFC9578_TYPE2: &str =
    include_str!("../../../../../tests/vectors/blind-rsa/rfc9578-type2.json");

fn hex(value: &str) -> Vec<u8> {
    crate::bytes::decode_hex(value).expect("vector hex")
}

fn field(vector: &Value, name: &str) -> Vec<u8> {
    hex(vector[name].as_str().expect("vector field"))
}

fn tag(error: CryptoFailure) -> CryptoErrorTag {
    error.tag
}

/// The DER inside a PEM document.
fn pem_der(pem: &[u8]) -> Vec<u8> {
    let body: String = std::str::from_utf8(pem)
        .expect("PEM text")
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    crate::bytes::decode_base64(&body).expect("PEM base64")
}

/// `salt || r`: what blinding draws from its provider.
fn blinding_entropy(salt: &[u8], factor: &[u8]) -> Vec<u8> {
    [salt, factor].concat()
}

fn rfc9578_vectors() -> Vec<Value> {
    let document: Value = serde_json::from_str(RFC9578_TYPE2).expect("RFC 9578 vector file");
    assert_eq!(
        (document["suite"].as_str(), document["token_type"].as_str()),
        (Some("RSABSSA-SHA384-PSS-Deterministic"), Some("0x0002"))
    );
    document["vectors"].as_array().expect("vectors").clone()
}

/// RFC 9474 A.3 through every helper at the vector's 4,096 bits: encoding,
/// blinding with the vector's salt and factor, raw signing on AWS-LC,
/// finalizing and verifying.
#[test]
fn rfc9474_a3_pss_deterministic_vector_runs_through_the_helpers() {
    let document: Value = serde_json::from_str(RFC9474_A3).expect("RFC 9474 vector file");
    let vector = &document["vector"];
    let (message, salt) = (field(vector, "msg"), field(vector, "salt"));
    assert_eq!(
        field(vector, "prepared_msg"),
        message,
        "identity preparation"
    );
    let key = PublicKey::<{ U4096::LIMBS }>::from_modulus(&field(vector, "n")).expect("RFC key");

    let mut encoded = vec![0u8; 512];
    emsa_pss_encode(&SystemProvider, &message, &salt, &mut encoded);
    assert_eq!(encoded, field(vector, "encoded_msg"));

    // The vector gives r^-1; blinding draws r itself.
    let inverse = field(vector, "inv");
    let factor = Uint::<{ U4096::LIMBS }>::from_be_slice(&inverse)
        .invert_odd_mod(&key.modulus)
        .expect("an invertible r^-1");
    let entropy = blinding_entropy(&salt, factor.to_be_bytes().as_slice());
    let blinded = blind(&FixedProvider::with_random(&entropy), &key, &message).expect("blind");
    assert_eq!(blinded.blinded, field(vector, "blinded_msg"));
    assert_eq!(blinded.inverse[..], inverse[..]);

    let blind_signature = field(vector, "blind_sig");
    assert!(signature_opens_to(&key, &blind_signature, &blinded.blinded));
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let rsa = rsa_from_components(vector);
        let signed = signer::sign_raw(&rsa, &key, &blinded.blinded).expect("AWS-LC raw sign");
        assert_eq!(signed, blind_signature);
    }

    let signature = finalize(&key, &message, &blind_signature, &blinded.inverse).expect("final");
    assert_eq!(signature, field(vector, "sig"));
    assert!(verify(&key, &message, &signature));
}

/// The RFC 9474 key as an AWS-LC `RSA` built from its components.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rsa_from_components(vector: &Value) -> signer::Rsa {
    use aws_lc_sys as lc;
    use signer::Bignum;
    let number = |bytes: &[u8]| unsafe {
        Bignum(lc::BN_bin2bn(
            bytes.as_ptr(),
            bytes.len(),
            std::ptr::null_mut(),
        ))
    };
    let (p, q, d) = (field(vector, "p"), field(vector, "q"), field(vector, "d"));
    let (p, q, d, n, e) = (
        number(&p),
        number(&q),
        number(&d),
        number(&field(vector, "n")),
        number(&field(vector, "e")),
    );
    unsafe {
        let context = lc::BN_CTX_new();
        let one = lc::BN_value_one();
        let (p_minus_one, q_minus_one) = (Bignum(lc::BN_new()), Bignum(lc::BN_new()));
        let (dmp1, dmq1, iqmp) = (
            Bignum(lc::BN_new()),
            Bignum(lc::BN_new()),
            Bignum(lc::BN_new()),
        );
        assert_eq!(lc::BN_sub(p_minus_one.0, p.0, one), 1);
        assert_eq!(lc::BN_sub(q_minus_one.0, q.0, one), 1);
        assert_eq!(
            lc::BN_div(ptr::null_mut(), dmp1.0, d.0, p_minus_one.0, context),
            1
        );
        assert_eq!(
            lc::BN_div(ptr::null_mut(), dmq1.0, d.0, q_minus_one.0, context),
            1
        );
        assert!(!lc::BN_mod_inverse(iqmp.0, q.0, p.0, context).is_null());
        lc::BN_CTX_free(context);
        let rsa = lc::RSA_new_private_key(n.0, e.0, d.0, p.0, q.0, dmp1.0, dmq1.0, iqmp.0);
        assert!(!rsa.is_null());
        signer::Rsa(rsa)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::ptr;

/// Every RFC 9578 A.2 token through the helpers and AWS-LC: the issuer key
/// imports to the vector's own PKCS#8 and SPKI, blinding with the vector's
/// salt and r gives its token request, signing its response, finalizing its
/// token, and the token verifies.
#[test]
fn rfc9578_type2_vectors_run_through_the_helpers_and_signer() {
    let vectors = rfc9578_vectors();
    assert_eq!(vectors.len(), 5);
    for (index, vector) in vectors.iter().enumerate() {
        let spki = field(vector, "pkI");
        let key = public_key_from_spki(&spki).expect("RFC 9578 issuer key");
        let key_id = SystemProvider.sha256(&spki);
        let token_request = field(vector, "token_request");
        let token = field(vector, "token");
        assert_eq!(
            token_request[..3],
            [0x00, 0x02, key_id[31]],
            "vector {index}"
        );

        let mut token_input = vec![0x00, 0x02];
        token_input.extend(field(vector, "nonce"));
        token_input.extend(SystemProvider.sha256(&field(vector, "token_challenge")));
        token_input.extend(key_id);
        assert_eq!(token[..98], token_input[..], "vector {index}");

        let entropy = blinding_entropy(&field(vector, "salt"), &field(vector, "blind"));
        let blinded =
            blind(&FixedProvider::with_random(&entropy), &key, &token_input).expect("blind");
        assert_eq!(blinded.blinded, token_request[3..], "vector {index}");

        let token_response = field(vector, "token_response");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let pkcs8 = pem_der(&field(vector, "skI"));
            let imported = signer::import(&pkcs8).expect("RFC 9578 issuer key imports");
            assert_eq!(imported[..], pkcs8[..], "already canonical");
            let modulus = signer::public_modulus(&imported).expect("modulus");
            assert_eq!(spki_for_modulus(&modulus), spki, "vector {index}");
            let signed = signer::sign(&imported, &blinded.blinded).expect("sign");
            assert_eq!(signed, token_response, "vector {index}");
        }

        let authenticator =
            finalize(&key, &token_input, &token_response, &blinded.inverse).expect("finalize");
        assert_eq!(authenticator, token[98..], "vector {index}");
        assert!(verify(&key, &token_input, &authenticator));
    }
}

fn rfc9578_key() -> (Vec<u8>, Br1PublicKey) {
    let spki = field(&rfc9578_vectors()[0], "pkI");
    let key = public_key_from_spki(&spki).expect("RFC 9578 issuer key");
    (spki, key)
}

/// Only the RFC 9578 template with a 2,048-bit odd modulus and e = 65,537
/// parses: every other length, every changed template byte, another
/// exponent, a shorter or even modulus is `InvalidPublicKey`.
#[test]
fn spki_parsing_accepts_only_the_exact_template() {
    let (spki, _) = rfc9578_key();
    for length in [0, SPKI_BYTES - 1, SPKI_BYTES + 1] {
        let mut resized = spki.clone();
        resized.resize(length, 0);
        let error = public_key_from_spki(&resized).err().expect("refused");
        assert_eq!(
            (error.tag, error.expected, error.actual),
            (
                CryptoErrorTag::InvalidPublicKey,
                SPKI_BYTES as i64,
                length as i64
            )
        );
    }
    let template = (0..SPKI_PREFIX.len()).chain(SPKI_BYTES - SPKI_SUFFIX.len()..SPKI_BYTES);
    for index in template {
        let mut changed = spki.clone();
        changed[index] ^= 0x01;
        assert!(
            public_key_from_spki(&changed).is_err(),
            "byte {index} was not checked"
        );
    }
    let mut exponent_three = spki.clone();
    exponent_three[SPKI_BYTES - 3..].copy_from_slice(&[0x00, 0x00, 0x03]);
    let mut short_modulus = spki.clone();
    short_modulus[SPKI_PREFIX.len()] &= 0x7f;
    let mut even_modulus = spki.clone();
    even_modulus[SPKI_PREFIX.len() + MODULUS_BYTES - 1] &= 0xfe;
    for refused in [exponent_three, short_modulus, even_modulus] {
        let error = public_key_from_spki(&refused).err().expect("refused");
        assert_eq!(tag(error), CryptoErrorTag::InvalidPublicKey);
    }
    let modulus: [u8; MODULUS_BYTES] = spki[SPKI_PREFIX.len()..][..MODULUS_BYTES]
        .try_into()
        .unwrap();
    assert_eq!(spki_for_modulus(&modulus), spki);
}

/// Blinding refuses to run without entropy, and a source that only ever
/// yields an unusable r (zero, or not below n) is a broken source.
#[test]
fn blinding_needs_entropy_and_a_usable_factor() {
    let (_, key) = rfc9578_key();
    let error = blind(&FixedProvider::entropy_failure(), &key, b"message")
        .err()
        .expect("no entropy");
    assert_eq!(tag(error), CryptoErrorTag::EntropyUnavailable);
    for factor in [[0u8; MODULUS_BYTES], [0xff; MODULUS_BYTES]] {
        let entropy = blinding_entropy(&[0x11; SALT_BYTES], &factor);
        let error = blind(&FixedProvider::with_random(&entropy), &key, b"message")
            .err()
            .expect("unusable r");
        assert_eq!(tag(error), CryptoErrorTag::EntropyUnavailable);
    }
    let entropy = blinding_entropy(&[0x11; SALT_BYTES], &key.modulus_bytes());
    assert!(blind(&FixedProvider::with_random(&entropy), &key, b"message").is_err());
}

/// Finalize releases a signature only when it verifies: a blind signature of
/// the wrong length, not below n, tampered, for another message, or
/// unblinded with the wrong state is `InvalidSignature`.
#[test]
fn finalize_refuses_everything_that_does_not_verify() {
    let vector = &rfc9578_vectors()[0];
    let (_, key) = rfc9578_key();
    let entropy = blinding_entropy(&field(vector, "salt"), &field(vector, "blind"));
    let token = field(vector, "token");
    let token_input = &token[..98];
    let blinded = blind(&FixedProvider::with_random(&entropy), &key, token_input).expect("blind");
    let response = field(vector, "token_response");
    let refused = |message: &[u8], blind_signature: &[u8], inverse: &[u8]| {
        let error = finalize(&key, message, blind_signature, inverse)
            .err()
            .expect("refused");
        (error.tag, error.expected, error.actual)
    };
    let signature = CryptoErrorTag::InvalidSignature;
    assert_eq!(
        refused(token_input, &response[1..], &blinded.inverse),
        (signature, 256, 255)
    );
    assert_eq!(
        refused(
            token_input,
            &[response.clone(), vec![0]].concat(),
            &blinded.inverse
        ),
        (signature, 256, 257)
    );
    assert_eq!(
        refused(token_input, &key.modulus_bytes(), &blinded.inverse),
        (signature, 256, 256)
    );
    let mut tampered = response.clone();
    tampered[200] ^= 0x01;
    assert_eq!(
        refused(token_input, &tampered, &blinded.inverse).0,
        signature
    );
    assert_eq!(
        refused(b"another message", &response, &blinded.inverse).0,
        signature
    );
    let other = blind(&SystemProvider, &key, token_input).expect("another blinding");
    assert_eq!(refused(token_input, &response, &other.inverse).0, signature);
    assert_eq!(refused(token_input, &response, &[0; 255]).0, signature);
    assert_eq!(
        refused(token_input, &response, &key.modulus_bytes()).0,
        signature
    );
    assert!(finalize(&key, token_input, &response, &blinded.inverse).is_ok());
}

/// Verification is a plain yes or no for any bytes.
#[test]
fn verify_rejects_tampered_misplaced_and_malformed_signatures() {
    let vector = &rfc9578_vectors()[0];
    let (_, key) = rfc9578_key();
    let token = field(vector, "token");
    let (token_input, authenticator) = token.split_at(98);
    assert!(verify(&key, token_input, authenticator));
    let mut tampered = authenticator.to_vec();
    tampered[0] ^= 0x01;
    for (message, signature) in [
        (token_input, &tampered[..]),
        (&token[1..98], authenticator),
        (token_input, &authenticator[1..]),
        (token_input, &key.modulus_bytes()[..]),
        (token_input, &[][..]),
    ] {
        assert!(!verify(&key, message, signature));
    }
}

/// A blinded message is exactly 256 bytes and below n: RSASP1 never runs on
/// anything else.
#[test]
fn a_blinded_message_must_be_an_element_below_the_modulus() {
    let (_, key) = rfc9578_key();
    let invalid = |blinded: &[u8]| {
        let error = blinded_element(&key, blinded).err().expect("refused");
        (error.tag, error.expected, error.actual)
    };
    let length = CryptoErrorTag::InvalidLength;
    assert_eq!(invalid(&[1; 255]), (length, 256, 255));
    assert_eq!(invalid(&[1; 257]), (length, 256, 257));
    assert_eq!(invalid(&key.modulus_bytes()), (length, 256, 256));
    assert_eq!(invalid(&[0xff; 256]), (length, 256, 256));
    assert!(blinded_element(&key, &[0; 256]).is_ok());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod server {
    use super::*;

    fn pkcs8_for(bits: u32, exponent: u64) -> Zeroizing<Box<[u8]>> {
        let rsa = signer::generate_rsa(bits, exponent).expect("test key");
        signer::encode(&rsa).expect("PKCS#8")
    }

    /// A generated key is a BR1 key: canonical PKCS#8 within the length
    /// rule, whose public key round-trips through blind, sign, finalize and
    /// verify.
    #[test]
    fn generated_keys_are_br1_keys_that_sign_blinded_messages() {
        for _ in 0..4 {
            let pkcs8 = signer::generate().expect("generate");
            assert!((MIN_SECRET_KEY_BYTES..=MAX_SECRET_KEY_BYTES).contains(&pkcs8.len()));
            assert_eq!(signer::import(&pkcs8).expect("reimport")[..], pkcs8[..]);
            let spki = spki_for_modulus(&signer::public_modulus(&pkcs8).expect("modulus"));
            let key = public_key_from_spki(&spki).expect("an RFC 9578 SPKI");
            let blinded = blind(&SystemProvider, &key, b"token input").expect("blind");
            let blind_signature = signer::sign(&pkcs8, &blinded.blinded).expect("sign");
            let signature = finalize(&key, b"token input", &blind_signature, &blinded.inverse)
                .expect("finalize");
            assert!(verify(&key, b"token input", &signature));
        }
    }

    /// Import accepts only an RSA PKCS#8 key of exactly 2,048 bits with
    /// e = 65,537 whose parts agree, with nothing after it.
    #[test]
    fn import_refuses_other_sizes_exponents_encodings_and_broken_keys() {
        let pkcs8 = pem_der(&field(&rfc9578_vectors()[0], "skI"));
        let mut trailing = pkcs8.clone();
        trailing.push(0);
        let mut broken = pkcs8.clone();
        // A byte inside the private exponent: the key no longer checks out.
        broken[400] ^= 0x01;
        let refused = [
            pkcs8[..pkcs8.len() - 1].to_vec(),
            trailing,
            broken,
            Vec::new(),
            pkcs8_for(1024, 65_537).to_vec(),
            pkcs8_for(3072, 65_537).to_vec(),
            pkcs8_for(2048, 3).to_vec(),
            pkcs8_for(2048, 65_539).to_vec(),
        ];
        for (index, input) in refused.iter().enumerate() {
            let error = signer::import(input).err().expect("refused");
            assert_eq!(tag(error), CryptoErrorTag::InvalidKey, "case {index}");
            assert_eq!(unsafe { aws_lc_sys::ERR_peek_error() }, 0, "case {index}");
        }
        assert!(signer::sign(&pkcs8_for(1024, 65_537), &[0; 128]).is_err());
    }

    /// Signing refuses a blinded message of the wrong length or not below n
    /// before AWS-LC sees it.
    #[test]
    fn signing_refuses_out_of_range_blinded_messages() {
        let pkcs8 = pem_der(&field(&rfc9578_vectors()[0], "skI"));
        let (_, key) = rfc9578_key();
        for (blinded, expected) in [
            (vec![1; 255], (256, 255)),
            (vec![1; 257], (256, 257)),
            (key.modulus_bytes(), (256, 256)),
            (vec![0xff; 256], (256, 256)),
        ] {
            let error = signer::sign(&pkcs8, &blinded).err().expect("refused");
            assert_eq!(
                (error.tag, error.expected, error.actual),
                (CryptoErrorTag::InvalidLength, expected.0, expected.1)
            );
        }
        assert_eq!(
            tag(signer::sign(b"not a key", &[0; 256])
                .err()
                .expect("refused")),
            CryptoErrorTag::InvalidKey
        );
    }

    /// Release-mode timing of `blind_rsa_sign` for one fixed blinded message
    /// against fresh random ones, beside a control of the fixed message in a
    /// second buffer. RSA blinding makes the time independent of the input.
    /// Run by `scripts/verify-crypto-timing.sh`.
    #[test]
    #[ignore = "release-mode timing distribution; run by scripts/verify-crypto-timing.sh"]
    fn blind_rsa_sign_timing_distribution() {
        use crate::bytes::tests::{welch_t, TimingStats};
        use std::hint::black_box;
        use std::time::Instant;
        const THRESHOLD: f64 = 10.0;

        let samples = std::env::var("MESH_TIMING_SAMPLES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(600)
            .max(200);
        let pkcs8 = pem_der(&field(&rfc9578_vectors()[0], "skI"));
        let (_, key) = rfc9578_key();
        let fixed = blind(&SystemProvider, &key, b"fixed")
            .expect("blind")
            .blinded;
        let control = fixed.clone();
        let time = |blinded: &[u8]| {
            let start = Instant::now();
            black_box(signer::sign(black_box(&pkcs8), black_box(blinded)).expect("sign"));
            start.elapsed().as_nanos() as f64
        };
        for _ in 0..32 {
            time(&fixed);
        }
        let (mut fixed_stats, mut random_stats, mut control_stats) = (
            TimingStats::default(),
            TimingStats::default(),
            TimingStats::default(),
        );
        for sample in 0..samples {
            let random = blind(&SystemProvider, &key, &sample.to_be_bytes())
                .expect("blind")
                .blinded;
            let mut order: [(&mut TimingStats, &[u8]); 3] = [
                (&mut fixed_stats, &fixed),
                (&mut random_stats, &random),
                (&mut control_stats, &control),
            ];
            order.rotate_left(sample % 3);
            for (stats, blinded) in order {
                stats.record(time(blinded));
            }
        }
        let t_score = welch_t(&fixed_stats, &random_stats);
        let control_t = welch_t(&fixed_stats, &control_stats);
        let inconclusive = control_t >= THRESHOLD;
        let passed = inconclusive || t_score < THRESHOLD;
        println!(
            "MESH_TIMING_JSON={{\"schema_version\":2,\"boundary\":\"Crypto.blind_rsa_sign\",\"modulus_bits\":2048,\"samples_per_group\":{samples},\"repetitions_per_sample\":1,\"fixed_mean_ns\":{:.3},\"random_mean_ns\":{:.3},\"control_mean_ns\":{:.3},\"welch_t\":{t_score:.6},\"control_t\":{control_t:.6},\"threshold\":{THRESHOLD:.1},\"inconclusive\":{inconclusive},\"passed\":{passed}}}",
            fixed_stats.mean(),
            random_stats.mean(),
            control_stats.mean(),
        );
        assert!(
            passed,
            "blind RSA signing time depends on the blinded message: |t|={t_score:.3} \
             with a control separation of only |t|={control_t:.3}"
        );
    }
}

/// Off the servers every private-key operation is `UnsupportedTarget`.
#[test]
fn the_unsupported_signer_refuses_every_operation() {
    let failures = [
        unsupported::generate().err(),
        unsupported::import(&[0; 1217]).err(),
        unsupported::public_modulus(&[0; 1217]).err(),
    ];
    for failure in failures {
        assert_eq!(failure.map(tag), Some(CryptoErrorTag::UnsupportedTarget));
    }
    assert_eq!(
        unsupported::sign(&[0; 1217], &[0; 256]).err().map(tag),
        Some(CryptoErrorTag::UnsupportedTarget)
    );
    assert_eq!(
        server_target().is_ok(),
        cfg!(any(target_os = "linux", target_os = "macos"))
    );
}
