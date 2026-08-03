//! Public-source end-to-end proof for the complete Crypto V2 classical API.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

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

#[test]
fn crypto_v2_public_api_compiles_and_executes_natively() {
    let temp = tempfile::tempdir().expect("failed to create temp directory");
    let project = temp.path().join("crypto-v2-public-api");
    std::fs::create_dir_all(&project).expect("failed to create project directory");
    std::fs::copy(fixture("crypto_v2.mpl"), project.join("main.mpl"))
        .expect("failed to copy Crypto V2 fixture");

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
            "x25519-public-abi:ok\n",
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
            "mlkem-layout:ok\n",
            "mlkem-roundtrip:ok\n",
            "mlkem-seed-deterministic:ok\n",
        )
    );
}
