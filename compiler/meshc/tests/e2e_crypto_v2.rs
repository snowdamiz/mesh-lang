//! Public-source end-to-end proof for the complete Crypto V2 classical API.

#[cfg(unix)]
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/test_artifacts.rs"]
mod artifacts;

#[cfg(unix)]
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

#[cfg(unix)]
#[derive(Deserialize)]
struct MlKemVectorSource {
    name: String,
    url: String,
    commit: String,
    test_group_id: u64,
    test_case_id: u64,
}

#[cfg(unix)]
#[derive(Deserialize)]
struct MlKemVectorInput {
    d: String,
    z: String,
}

#[cfg(unix)]
#[derive(Deserialize)]
struct MlKemVectorExpected {
    public_key: String,
}

#[cfg(unix)]
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

#[cfg(unix)]
fn mlkem_vector_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("vectors")
        .join("mlkem")
        .join("mlkem768-keygen-acvp-tc26.json")
}

#[cfg(unix)]
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

fn blind_rsa_vectors() -> serde_json::Value {
    let path = artifacts::repo_root().join("tests/vectors/blind-rsa/rfc9578-type2.json");
    let vectors: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).expect("read RFC9578 vectors"))
            .expect("parse RFC9578 vectors");
    assert_eq!(vectors["schema_version"], 1);
    assert_eq!(vectors["profile"], "BR1");
    assert_eq!(vectors["source"]["section"], "Appendix A.2");
    assert_eq!(vectors["vectors"].as_array().unwrap().len(), 5);
    vectors
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2));
    (0..value.len())
        .step_by(2)
        .map(|offset| {
            u8::from_str_radix(&value[offset..offset + 2], 16).expect("valid fixture hex")
        })
        .collect()
}

fn encode_hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn blind_rsa_fixture_source(vectors: &serde_json::Value) -> String {
    let first = &vectors["vectors"][0];
    let spki = first["pkI"].as_str().unwrap();
    let mut bad_spki = decode_hex(spki);
    bad_spki[13] ^= 1; // RSASSA-PSS OID, keeping all lengths intact.
    let mut bad_exponent = decode_hex(spki);
    *bad_exponent.last_mut().unwrap() = 3;
    let mut short_modulus = decode_hex(spki);
    // The template's INTEGER sign octet precedes its fixed 256-byte modulus.
    let modulus_offset = short_modulus.len() - 5 - 256;
    short_modulus[modulus_offset] &= 0x7f;
    let mut tampered_signature = decode_hex(first["authenticator"].as_str().unwrap());
    tampered_signature[128] ^= 1;
    let mut context = [0u8; 123];
    context[0] = 1;
    context[114] = 18;
    context[122] = 1;
    let key_context = encode_hex(&context);
    context[114] = 19;
    let state_context = encode_hex(&context);
    context[112] = 1;
    let other_context = encode_hex(&context);
    let calls: String = vectors["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            assert_eq!(v["pkI"].as_str().unwrap(), spki);
            format!(
                "let _ = verify_vector(key, signer, \"{}\", \"{}\", \"{}\", \"{}\", \"br1-rfc9578-{}\") ?\n",
                v["token_input"].as_str().unwrap(),
                v["authenticator"].as_str().unwrap(),
                v["blinded_msg"].as_str().unwrap(),
                v["token_response"].as_str().unwrap(),
                v["id"].as_u64().unwrap()
            )
        })
        .collect();
    let client_calls: String = vectors["vectors"].as_array().unwrap().iter().map(|v| format!(
        "report(\"br1-client-vector-{}\", Crypto.blind_rsa_verify(key, vector_bytes(\"{}\")?, vector_bytes(\"{}\")?)?)\n",
        v["id"].as_u64().unwrap(), v["token_input"].as_str().unwrap(), v["authenticator"].as_str().unwrap()
    )).collect();
    fs::read_to_string(fixture("blind_rsa.mpl"))
        .expect("read BR1 Mesh fixture")
        .replace("__SPKI_HEX__", spki)
        .replace("__VECTOR_CALLS__", &calls)
        .replace("__CLIENT_VECTOR_CALLS__", &client_calls)
        .replace("__MESSAGE_HEX__", first["token_input"].as_str().unwrap())
        .replace(
            "__SIGNATURE_HEX__",
            first["authenticator"].as_str().unwrap(),
        )
        .replace(
            "__TAMPERED_SIGNATURE_HEX__",
            &encode_hex(&tampered_signature),
        )
        .replace("__BAD_SPKI_HEX__", &encode_hex(&bad_spki))
        .replace("__BAD_EXPONENT_SPKI_HEX__", &encode_hex(&bad_exponent))
        .replace("__SHORT_MODULUS_SPKI_HEX__", &encode_hex(&short_modulus))
        .replace(
            "__MODULUS_HEX__",
            &spki[modulus_offset * 2..(modulus_offset + 256) * 2],
        )
        .replace("__KEY_CONTEXT_HEX__", &key_context)
        .replace("__STATE_CONTEXT_HEX__", &state_context)
        .replace("__OTHER_CONTEXT_HEX__", &other_context)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_blind_rsa_public_api() -> String {
    let vectors = blind_rsa_vectors();
    let temp = tempfile::tempdir().expect("BR1 project directory");
    let project = temp.path().join("blind-rsa-public-api");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("main.mpl"), blind_rsa_fixture_source(&vectors)).unwrap();
    let build = Command::new(meshc_bin())
        .arg("build")
        .arg(&project)
        .output()
        .expect("compile public BR1 program");
    assert!(
        build.status.success(),
        "BR1 compilation failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let output = Command::new(project.join("blind-rsa-public-api"))
        .env(
            "MESH_TEST_BLIND_RSA_PKCS8_HEX",
            vectors["vectors"][0]["pkcs8_der"].as_str().unwrap(),
        )
        .output()
        .expect("execute public BR1 program");
    assert!(
        output.status.success(),
        "BR1 executable failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "BR1 emitted diagnostics: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains(":failed"), "BR1 check failed:\n{stdout}");
    for label in [
        "br1-import-public",
        "br1-rfc9578-1",
        "br1-rfc9578-2",
        "br1-rfc9578-3",
        "br1-rfc9578-4",
        "br1-rfc9578-5",
        "br1-message-tamper",
        "br1-signature-tamper",
        "br1-spki-empty",
        "br1-spki-template",
        "br1-spki-exponent",
        "br1-spki-modulus",
        "br1-sign-length",
        "br1-sign-range",
        "br1-message-bound",
        "br1-import-malformed",
        "br1-fresh-blinding",
        "br1-roundtrip",
        "br1-wrong-key",
        "br1-finalize-tamper",
        "br1-empty-message",
        "br1-maximum-message",
        "br1-key-storage",
        "br1-state-storage",
        "br1-storage-context",
        "br1-storage-type",
        "br1-storage-tamper",
        "br1-state-message-binding",
        "br1-state-key-binding",
    ] {
        assert!(
            stdout.lines().any(|line| line == format!("{label}:ok")),
            "missing {label}:\n{stdout}"
        );
    }
    // The RFC private key never enters stdout or diagnostics, including errors.
    assert!(!stdout.contains(vectors["vectors"][0]["pkcs8_der"].as_str().unwrap()));
    stdout
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn blind_rsa_public_api_rfc9578_and_roundtrip() {
    run_blind_rsa_public_api();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn blind_rsa_openssl_differential() {
    let stdout = run_blind_rsa_public_api();
    let temp = tempfile::tempdir().unwrap();
    let path = |name: &str| temp.path().join(name);
    for (prefix, name) in [
        ("BR1_SPKI=", "public.der"),
        ("BR1_MESSAGE=", "message.bin"),
        ("BR1_SIGNATURE=", "signature.bin"),
    ] {
        let value = stdout
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .expect("public differential output");
        fs::write(path(name), decode_hex(value)).unwrap();
    }
    let openssl = std::env::var_os("MESH_OPENSSL_BIN").unwrap_or_else(|| "openssl".into());
    let checked = |args: &[&str]| {
        let output = Command::new(&openssl)
            .current_dir(temp.path())
            .args(args)
            .output()
            .expect("OpenSSL CLI required for BR1 differential test (MESH_OPENSSL_BIN)");
        assert!(
            output.status.success(),
            "OpenSSL {args:?} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let version = checked(&["version"]);
    println!(
        "BR1_OPENSSL={}",
        String::from_utf8_lossy(&version.stdout).trim()
    );
    checked(&[
        "pkey",
        "-pubin",
        "-inform",
        "DER",
        "-in",
        "public.der",
        "-out",
        "public.pem",
    ]);
    checked(&[
        "dgst",
        "-sha384",
        "-sigopt",
        "rsa_padding_mode:pss",
        "-sigopt",
        "rsa_pss_saltlen:48",
        "-verify",
        "public.pem",
        "-signature",
        "signature.bin",
        "message.bin",
    ]);
    let vectors = blind_rsa_vectors();
    let first = &vectors["vectors"][0];
    fs::write(
        path("test-key.der"),
        decode_hex(first["pkcs8_der"].as_str().unwrap()),
    )
    .unwrap();
    fs::write(
        path("blinded.bin"),
        decode_hex(first["blinded_msg"].as_str().unwrap()),
    )
    .unwrap();
    // RFC test key uses rsaEncryption PKCS#8, so raw RSA is not PSS-key restricted.
    // OpenSSL pkeyutl -sign imposes an EVP_MAX_MD_SIZE input cap even with
    // padding:none (upstream #30777). -decrypt performs the same m^d mod n
    // without hashing/padding and accepts the complete modulus-width input.
    checked(&[
        "pkeyutl",
        "-decrypt",
        "-keyform",
        "DER",
        "-inkey",
        "test-key.der",
        "-pkeyopt",
        "rsa_padding_mode:none",
        "-in",
        "blinded.bin",
        "-out",
        "response.bin",
    ]);
    assert_eq!(
        fs::read(path("response.bin")).unwrap(),
        decode_hex(first["token_response"].as_str().unwrap())
    );
}

#[test]
fn blind_rsa_client_public_api_all_targets() {
    let vectors = blind_rsa_vectors();
    let full_source = blind_rsa_fixture_source(&vectors);
    let source = full_source.rsplit_once("\nfn main() do").unwrap().0;
    let gated_server_check = if cfg!(any(target_os = "linux", target_os = "macos")) {
        ""
    } else {
        "case Crypto.blind_rsa_generate() do\n Err(UnsupportedTarget) -> report(\"br1-client-server-gate\", true)\n Ok(key) -> do\n Secret.destroy(key)\n report(\"br1-client-server-gate\", false)\n end\n _ -> report(\"br1-client-server-gate\", false)\n end\n"
    };
    let source = format!("{source}\nfn main() do\ncase check_client_vectors() do\n Ok(_) -> nil\n Err(_) -> report(\"br1-client-error\", false)\nend\n{gated_server_check}end\n");
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("blind-rsa-client");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("main.mpl"), source).unwrap();
    let build = Command::new(meshc_bin())
        .arg("build")
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "BR1 client build failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = if cfg!(windows) {
        "blind-rsa-client.exe"
    } else {
        "blind-rsa-client"
    };
    let output = Command::new(project.join(binary)).output().unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "BR1 client execution failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains(":failed"), "{stdout}");
    for label in [
        "br1-client-vector-1",
        "br1-client-vector-2",
        "br1-client-vector-3",
        "br1-client-vector-4",
        "br1-client-vector-5",
        "br1-client-blind",
        "br1-client-finalize",
    ] {
        assert!(
            stdout.lines().any(|line| line == format!("{label}:ok")),
            "missing {label}:\n{stdout}"
        );
    }
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        assert!(stdout.contains("br1-client-server-gate:ok"));
    }
}

#[test]
fn aead_bounded_transport_public_api_preserves_unrelated_limits() {
    for optimization in ["0", "2"] {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("aead-bounds");
        fs::create_dir(&project).unwrap();
        fs::copy(fixture("aead_bounds.mpl"), project.join("main.mpl")).unwrap();
        let build = Command::new(meshc_bin())
            .args(["build", "--opt-level", optimization])
            .arg(&project)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let binary = if cfg!(windows) {
            "aead-bounds.exe"
        } else {
            "aead-bounds"
        };
        let run = Command::new(project.join(binary)).output().unwrap();
        assert!(
            run.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8(run.stdout).unwrap(), "aead-bounds:ok\n");
    }
}

#[test]
fn storage_index_public_api_executes_at_both_optimization_levels() {
    for optimization in ["0", "2"] {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("storage-index");
        fs::create_dir(&project).unwrap();
        fs::copy(fixture("storage_index.mpl"), project.join("main.mpl")).unwrap();
        let build = Command::new(meshc_bin())
            .args(["build", "--opt-level", optimization])
            .arg(&project)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let binary = if cfg!(windows) {
            "storage-index.exe"
        } else {
            "storage-index"
        };
        let run = Command::new(project.join(binary)).output().unwrap();
        assert!(
            run.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8(run.stdout).unwrap(), "storage-index:ok\n");
    }
}

#[test]
fn xchacha20poly1305_public_api_executes_at_both_optimization_levels() {
    for optimization in ["0", "2"] {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("xchacha20poly1305");
        fs::create_dir(&project).unwrap();
        fs::copy(fixture("xchacha20poly1305.mpl"), project.join("main.mpl")).unwrap();
        let build = Command::new(meshc_bin())
            .args(["build", "--opt-level", optimization])
            .arg(&project)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let binary = if cfg!(windows) {
            "xchacha20poly1305.exe"
        } else {
            "xchacha20poly1305"
        };
        let run = Command::new(project.join(binary)).output().unwrap();
        assert!(
            run.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8(run.stdout).unwrap(),
            "xchacha20poly1305:ok\n"
        );
    }
}
