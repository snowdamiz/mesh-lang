//! Public-source end-to-end proof for the complete Crypto V2 classical API.

#![cfg(unix)]

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let mut path = std::env::current_exe()
        .expect("cannot locate test executable")
        .parent()
        .expect("test executable has no parent")
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
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
