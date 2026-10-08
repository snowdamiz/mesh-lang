//! Public-source end-to-end proof for the complete Crypto V2 classical API.

#![cfg(unix)]

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/test_artifacts.rs"]
mod artifacts;

#[derive(Deserialize)]
struct MlKemVector {
    schema_version: u64,
    suite: String,
    version: String,
    source: MlKemVectorSource,
    input: MlKemVectorInput,
    expected: MlKemVectorExpected,
    expected_errors: Vec<MlKemVectorError>,
}

#[derive(Deserialize)]
struct MlKemVectorSource {
    name: String,
    url: String,
    commit: String,
    test_group_id: u64,
    test_case_id: u64,
}

#[derive(Deserialize)]
struct MlKemVectorInput {
    d: String,
    z: String,
}

#[derive(Deserialize)]
struct MlKemVectorExpected {
    public_key: String,
}

#[derive(Deserialize)]
struct MlKemVectorError {
    input_length: usize,
    tag: String,
    expected_length: usize,
    source: String,
}

fn meshc_bin() -> PathBuf {
    artifacts::meshc_bin()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("e2e")
        .join(name)
}

fn mlkem_vector_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("vectors")
        .join("mlkem")
        .join("mlkem768-keygen-acvp-tc26.json")
}

#[test]
fn crypto_v2_public_api_compiles_and_executes_natively() {
    let temp = tempfile::tempdir().expect("failed to create temp directory");
    let project = temp.path().join("crypto-v2-public-api");
    fs::create_dir_all(&project).expect("failed to create project directory");

    let vector: MlKemVector = serde_json::from_str(
        &fs::read_to_string(mlkem_vector_fixture()).expect("failed to read ML-KEM vector"),
    )
    .expect("failed to parse ML-KEM vector");
    assert_eq!(vector.schema_version, 1);
    assert_eq!(vector.suite, "ML-KEM-768");
    assert_eq!(vector.version, "FIPS 203");
    assert_eq!(
        vector.source.name,
        "NIST ACVP ML-KEM keyGen FIPS203 internal projection"
    );
    assert_eq!(
        vector.source.url,
        "https://github.com/usnistgov/ACVP-Server/blob/65370b861b96efd30dfe0daae607bde26a78a5c8/gen-val/json-files/ML-KEM-keyGen-FIPS203/internalProjection.json"
    );
    assert_eq!(
        vector.source.commit,
        "65370b861b96efd30dfe0daae607bde26a78a5c8"
    );
    assert_eq!(vector.source.test_group_id, 2);
    assert_eq!(vector.source.test_case_id, 26);
    assert_eq!(vector.expected_errors.len(), 2);
    assert_eq!(
        (
            vector.expected_errors[0].input_length,
            vector.expected_errors[0].tag.as_str(),
            vector.expected_errors[0].expected_length,
            vector.expected_errors[1].input_length,
            vector.expected_errors[1].tag.as_str(),
            vector.expected_errors[1].expected_length,
            vector.expected_errors[0].source.as_str(),
            vector.expected_errors[1].source.as_str(),
        ),
        (
            63,
            "InvalidLength",
            64,
            65,
            "InvalidLength",
            64,
            "Mesh Crypto V2 public API seed-length contract",
            "Mesh Crypto V2 public API seed-length contract",
        )
    );

    let seed = format!("{}{}", vector.input.d, vector.input.z);
    let source = fs::read_to_string(fixture("crypto_v2.mpl"))
        .expect("failed to read Crypto V2 fixture")
        .replace("__MLKEM_SEED_HEX__", &seed)
        .replace("__MLKEM_PUBLIC_KEY_HEX__", &vector.expected.public_key)
        .replace(
            "__MLKEM_SHORT_SEED_LENGTH__",
            &vector.expected_errors[0].input_length.to_string(),
        )
        .replace(
            "__MLKEM_LONG_SEED_LENGTH__",
            &vector.expected_errors[1].input_length.to_string(),
        );
    fs::write(project.join("main.mpl"), source)
        .expect("failed to write generated Crypto V2 fixture");

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().expect("non-UTF-8 project path")])
        .output()
        .expect("failed to invoke meshc");
    assert!(
        build.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(project.join("crypto-v2-public-api"))
        .output()
        .expect("failed to execute compiled Mesh program");
    assert!(
        run.status.success(),
        "compiled program failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.stderr.is_empty(),
        "compiled program wrote to stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        concat!(
            "sha256-binary:ok\n",
            "sha512-binary:ok\n",
            "sha256-hex:ok\n",
            "sha512-hex:ok\n",
            "hash-large-input:ok\n",
            "random-zero:ok\n",
            "random-length:ok\n",
            "random-max:ok\n",
            "random-negative-length:ok\n",
            "random-excessive-length:ok\n",
            "secret-zero-length:ok\n",
            "hmac-hkdf-lifecycle:ok\n",
            "hmac-message-length:ok\n",
            "hkdf-zero-length:ok\n",
            "hkdf-excessive-length:ok\n",
            "argon2id-happy:ok\n",
            "argon2id-deterministic:ok\n",
            "argon2id-salt-bound:ok\n",
            "argon2id-memory-bound:ok\n",
            "argon2id-output-bound:ok\n",
            "x25519-public-abi:ok\n",
            "x25519-secret-derivation:ok\n",
            "x25519-invalid-public:ok\n",
            "x25519-noncontributory:ok\n",
            "x25519-agreement:ok\n",
            "aead-ciphertext-size:ok\n",
            "aead-roundtrip:ok\n",
            "aead-tamper:ok\n",
            "aead-wrong-key:ok\n",
            "aead-key-after-failure:ok\n",
            "aead-nonce-length:ok\n",
            "aead-plaintext-length:ok\n",
            "aead-ciphertext-bound:ok\n",
            "signing-public-abi:ok\n",
            "signature-abi:ok\n",
            "signature-valid:ok\n",
            "signature-mismatch:ok\n",
            "signature-malformed:ok\n",
            "signing-invalid-public:ok\n",
            "hpke-wire-size:ok\n",
            "hpke-roundtrip:ok\n",
            "hpke-info-binding:ok\n",
            "hpke-wire-bound:ok\n",
            "hpke-secret-roundtrip:ok\n",
            "mlkem-layout:ok\n",
            "mlkem-storage:ok\n",
            "mlkem-roundtrip:ok\n",
            "mlkem-seed-deterministic:ok\n",
            "mlkem-nist-acvp-keygen:ok\n",
            "mlkem-invalid-seed-short:ok\n",
            "mlkem-invalid-seed-long:ok\n",
        )
    );
}

// ── Blind RSA (profile BR1) ────────────────────────────────────────────

#[derive(Deserialize)]
struct BlindRsaVectorFile {
    schema_version: u64,
    suite: String,
    profile: String,
    token_type: String,
    source: BlindRsaVectorSource,
    vectors: Vec<BlindRsaVector>,
}

#[derive(Deserialize)]
struct BlindRsaVectorSource {
    name: String,
    url: String,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct BlindRsaVector {
    skI: String,
    pkI: String,
    token_challenge: String,
    nonce: String,
    salt: String,
    blind: String,
    token_request: String,
    token_response: String,
    token: String,
}

fn blind_rsa_vectors() -> BlindRsaVectorFile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/vectors/blind-rsa/rfc9578-type2.json");
    serde_json::from_str(&fs::read_to_string(path).expect("failed to read RFC 9578 vectors"))
        .expect("failed to parse RFC 9578 vectors")
}

fn decode_hex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("hex"))
        .collect()
}

fn encode_hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The DER inside a PEM document, as hex.
fn pem_der_hex(pem_hex: &str) -> String {
    use base64::Engine as _;
    let pem = String::from_utf8(decode_hex(pem_hex)).expect("PEM text");
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    encode_hex(
        &base64::engine::general_purpose::STANDARD
            .decode(body)
            .expect("PEM base64"),
    )
}

/// Build `source` as a project and run it with `envs`: its stdout.
fn build_and_run(name: &str, source: &str, envs: &[(&str, &str)]) -> String {
    let temp = tempfile::tempdir().expect("failed to create temp directory");
    let project = temp.path().join(name);
    fs::create_dir_all(&project).expect("failed to create project directory");
    fs::write(project.join("main.mpl"), source).expect("failed to write fixture");
    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().expect("non-UTF-8 project path")])
        .output()
        .expect("failed to invoke meshc");
    assert!(
        build.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join(name))
        .envs(envs.iter().copied())
        .output()
        .expect("failed to execute compiled Mesh program");
    assert!(
        run.status.success() && run.stderr.is_empty(),
        "compiled program failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout).expect("UTF-8 output")
}

/// RFC 9578 Appendix A.2 through the public Mesh API in compiled Mesh: the
/// issuer key imports from secret bytes to the vector's SPKI, signing each
/// token request gives the vector's response, each token verifies, and a
/// fresh blind/sign/finalize round trip, the refusals and sealed storage of
/// the key hold.
#[test]
fn blind_rsa_rfc9578_type2_vectors_run_through_the_public_mesh_api() {
    let file = blind_rsa_vectors();
    assert_eq!(
        (
            file.schema_version,
            file.suite.as_str(),
            file.profile.as_str(),
            file.token_type.as_str(),
            file.source.name.as_str(),
            file.source.url.as_str(),
            file.vectors.len(),
        ),
        (
            1,
            "RSABSSA-SHA384-PSS-Deterministic",
            "BR1",
            "0x0002",
            "RFC 9578 Appendix A.2, Issuance Protocol 2 - Blind RSA (2048-bit)",
            "https://www.rfc-editor.org/rfc/rfc9578.txt",
            5,
        )
    );
    let issuer = pem_der_hex(&file.vectors[0].skI);
    let mut checks = String::new();
    let mut expected = String::new();
    for (index, vector) in file.vectors.iter().enumerate() {
        assert_eq!(pem_der_hex(&vector.skI), issuer, "one issuer key");
        assert_eq!((vector.salt.len(), vector.blind.len()), (96, 512));
        checks.push_str(&format!(
            "check_vector({index}, issuer, \"{}\", \"{}\", \"{}\", \"{}\", \"{}\", \"{}\") ?\n  ",
            vector.pkI,
            vector.token_challenge,
            vector.nonce,
            vector.token_request,
            vector.token_response,
            vector.token
        ));
        for check in ["key", "request", "input", "sign", "verify"] {
            expected.push_str(&format!("rfc9578-type2-{index}-{check}:ok\n"));
        }
    }
    for check in [
        "blind-rsa-round-trip",
        "blind-rsa-unlinkable-requests",
        "blind-rsa-verify-tampered",
        "blind-rsa-finalize-tampered",
        "blind-rsa-spki-length",
        "blind-rsa-spki-exponent",
        "blind-rsa-sign-length",
        "blind-rsa-sign-range",
        "blind-rsa-verify-length",
        "blind-rsa-import-invalid",
        "blind-rsa-storage",
    ] {
        expected.push_str(&format!("{check}:ok\n"));
    }
    let first = &file.vectors[0];
    let source = fs::read_to_string(fixture("blind_rsa.mpl"))
        .expect("failed to read blind RSA fixture")
        .replace("__RFC9578_VECTOR_CHECKS__", &checks)
        .replace("__RFC9578_SPKI_HEX__", &first.pkI)
        .replace("__RFC9578_NONCE_HEX__", &first.nonce)
        .replace("__RFC9578_CHALLENGE_HEX__", &first.token_challenge);

    let output = build_and_run(
        "blind-rsa-public-api",
        &source,
        &[("MESH_BLIND_RSA_ISSUER_PKCS8_HEX", issuer.as_str())],
    );
    assert_eq!(output, expected);
}

/// One RFC 9578 token issued and redeemed through compiled Mesh on a server
/// target: generate, publish, blind, sign, finalize, verify, nullify.
#[test]
fn blind_rsa_token_issue_and_redeem_round_trip() {
    let source = fs::read_to_string(fixture("blind_rsa_token_round_trip.mpl"))
        .expect("failed to read round-trip fixture");
    let output = build_and_run("blind-rsa-token-round-trip", &source, &[]);
    assert_eq!(output, "token:354\nnullifier:32\nforged:refused\n");
}

/// Finalizing twice with one blinding state does not compile: the state is
/// affine and finalize consumes it.
#[test]
fn blind_rsa_finalize_with_a_consumed_state_is_a_compile_error() {
    let temp = tempfile::tempdir().expect("failed to create temp directory");
    let project = temp.path().join("blind-rsa-consumed-state");
    fs::create_dir_all(&project).expect("failed to create project directory");
    fs::write(
        project.join("main.mpl"),
        "fn twice(key :: BlindRsaPublicKey, message :: Bytes, response :: Bytes) -> Bytes ! CryptoError do\n\
         \x20 let blinded = Crypto.blind_rsa_blind(key, message) ?\n\
         \x20 let first = Crypto.blind_rsa_finalize(key, message, response, blinded.state) ?\n\
         \x20 Crypto.blind_rsa_finalize(key, message, response, blinded.state)\n\
         end\n\
         fn main() do\n\
         \x20 nil\n\
         end\n",
    )
    .expect("failed to write fixture");
    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().expect("non-UTF-8 project path")])
        .output()
        .expect("failed to invoke meshc");
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(!build.status.success(), "{diagnostics}");
    assert!(
        diagnostics.contains("resource `blinded` was used after it moved"),
        "{diagnostics}"
    );
}

fn openssl() -> String {
    std::env::var("MESH_OPENSSL").unwrap_or_else(|_| "openssl".to_string())
}

fn run_openssl(arguments: &[&str]) -> std::process::Output {
    Command::new(openssl())
        .args(arguments)
        .output()
        .expect("the OpenSSL CLI is required for the blind RSA differential test")
}

/// Differential against the OpenSSL CLI: a Mesh-generated key's SPKI and a
/// finalized signature verify with `openssl dgst -sha384` under RSASSA-PSS
/// with a 48-byte salt (and a changed signature does not), and raw signing
/// with an OpenSSL-generated key equals OpenSSL's own raw private-key
/// operation on the same input.
#[test]
fn blind_rsa_agrees_with_the_openssl_cli() {
    let temp = tempfile::tempdir().expect("failed to create temp directory");
    let path = |name: &str| temp.path().join(name).to_str().unwrap().to_string();
    let generated = run_openssl(&[
        "genpkey",
        "-algorithm",
        "RSA",
        "-pkeyopt",
        "rsa_keygen_bits:2048",
        "-outform",
        "DER",
        "-out",
        &path("key.der"),
    ]);
    assert!(generated.status.success(), "{generated:?}");
    // Some OpenSSL releases write a DER key as PKCS#1: Mesh imports PKCS#8.
    let converted = run_openssl(&[
        "pkcs8",
        "-topk8",
        "-nocrypt",
        "-inform",
        "DER",
        "-in",
        &path("key.der"),
        "-outform",
        "DER",
        "-out",
        &path("key.p8"),
    ]);
    assert!(converted.status.success(), "{converted:?}");
    let pkcs8_hex = encode_hex(&fs::read(path("key.p8")).expect("OpenSSL key"));
    // Below every 2,048-bit modulus: a leading zero byte.
    let raw_input: Vec<u8> = std::iter::once(0)
        .chain((1..256u32).map(|index| (index * 131 + 7) as u8))
        .collect();
    let source = fs::read_to_string(fixture("blind_rsa_differential.mpl"))
        .expect("failed to read differential fixture")
        .replace("__RAW_INPUT_HEX__", &encode_hex(&raw_input));
    let output = build_and_run(
        "blind-rsa-differential",
        &source,
        &[("MESH_BLIND_RSA_OPENSSL_PKCS8_HEX", pkcs8_hex.as_str())],
    );
    let value = |label: &str| {
        let prefix = format!("{label}:");
        decode_hex(
            output
                .lines()
                .find_map(|line| line.strip_prefix(prefix.as_str()))
                .unwrap_or_else(|| panic!("no {label} in {output}")),
        )
    };

    fs::write(path("spki.der"), value("spki")).unwrap();
    fs::write(path("message.bin"), value("message")).unwrap();
    let signature = value("signature");
    fs::write(path("signature.bin"), &signature).unwrap();
    let mut changed = signature.clone();
    changed[7] ^= 1;
    fs::write(path("changed.bin"), &changed).unwrap();
    let verify = |signature: &str| {
        run_openssl(&[
            "dgst",
            "-sha384",
            "-sigopt",
            "rsa_padding_mode:pss",
            "-sigopt",
            "rsa_pss_saltlen:48",
            "-keyform",
            "DER",
            "-verify",
            &path("spki.der"),
            "-signature",
            &path(signature),
            &path("message.bin"),
        ])
    };
    let accepted = verify("signature.bin");
    assert!(
        accepted.status.success()
            && String::from_utf8_lossy(&accepted.stdout).contains("Verified OK"),
        "{accepted:?}"
    );
    assert!(!verify("changed.bin").status.success());

    let modulus = run_openssl(&[
        "rsa",
        "-inform",
        "DER",
        "-in",
        &path("key.der"),
        "-noout",
        "-modulus",
    ]);
    let modulus = String::from_utf8(modulus.stdout).expect("modulus");
    assert_eq!(
        modulus
            .trim()
            .strip_prefix("Modulus=")
            .expect("modulus line")
            .to_lowercase(),
        encode_hex(&value("imported-spki")[81..337])
    );

    fs::write(path("raw.bin"), &raw_input).unwrap();
    let key = path("key.p8");
    let raw_sign = |operation: &str| {
        run_openssl(&[
            "pkeyutl",
            operation,
            "-inkey",
            &key,
            "-keyform",
            "DER",
            "-pkeyopt",
            "rsa_padding_mode:none",
            "-in",
            &path("raw.bin"),
            "-out",
            &path("openssl-raw.bin"),
        ])
    };
    // OpenSSL 3.0-3.4 sign raw input of the modulus length; 3.5 and later
    // refuse it as an oversized digest, and the same private-key operation
    // (RSASP1 = RSADP) is `-decrypt` with no padding.
    if !raw_sign("-sign").status.success() {
        let decrypted = raw_sign("-decrypt");
        assert!(decrypted.status.success(), "{decrypted:?}");
    }
    assert_eq!(
        fs::read(path("openssl-raw.bin")).expect("OpenSSL raw signature"),
        value("raw-signature")
    );
}
