#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

fn copy_package(source: &Path, destination: &Path, module: &str) {
    fs::create_dir_all(destination.join("protocol").parent().unwrap()).unwrap();
    fs::create_dir_all(destination.join(module).parent().unwrap()).unwrap();
    fs::copy(source.join("mesh.toml"), destination.join("mesh.toml")).unwrap();
    fs::copy(source.join(module), destination.join(module)).unwrap();
}

#[test]
fn messenger_protocol_matches_golden_v1_fixtures() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("messenger-protocol-proof");
    let installed = project.join(".mesh/packages");
    copy_package(
        &root.join("mesh-lang/packages/mesh-binary"),
        &installed.join("mesh-binary@0.1.0"),
        "binary/reader.mpl",
    );
    copy_package(
        &root.join("mesh-private-messenger/packages/messenger-protocol"),
        &installed.join("messenger-protocol@0.1.0"),
        "protocol/v1.mpl",
    );
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"messenger-protocol-proof\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let outer_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m1/outer-envelope-v1.hex"),
    )
    .unwrap();
    let credential_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m1/device-credential-v1.hex"),
    )
    .unwrap();
    let outer_oversized_hex =
        fs::read_to_string(root.join(
            "mesh-private-messenger/tests/fixtures/m1/outer-envelope-v1-oversized-vector.hex",
        ))
        .unwrap();
    let credential_oversized_hex = fs::read_to_string(root.join(
        "mesh-private-messenger/tests/fixtures/m1/device-credential-v1-oversized-vector.hex",
    ))
    .unwrap();
    let source = r##"
from Protocol.V1 import DeviceCredential, OuterEnvelope, decode_device_credential, decode_outer_envelope, encode_device_credential, encode_outer_envelope

fn check_outer(value :: OuterEnvelope, expected :: Bytes) do
  case encode_outer_envelope(value) do
    Err(_) -> println("outer-encode-error")
    Ok(encoded) -> if Bytes.secure_equals(encoded, expected) do
        println("outer-encode")
      else
        println("outer-encode-mismatch")
      end
  end
  case decode_outer_envelope(expected) do
    Err(_) -> println("outer-decode-error")
    Ok(decoded) -> if decoded.version == 1 && decoded.suite == 1 && decoded.padding_bucket == 256 && U64.compare(decoded.expiration, value.expiration) == 0 && Bytes.secure_equals(decoded.envelope_id, value.envelope_id) && Bytes.secure_equals(decoded.mailbox_token, value.mailbox_token) && Bytes.secure_equals(decoded.ciphertext, value.ciphertext) do
        println("outer-decode")
      else
        println("outer-decode-mismatch")
      end
  end
end

fn check_credential(value :: DeviceCredential, expected :: Bytes) do
  case encode_device_credential(value) do
    Err(_) -> println("credential-encode-error")
    Ok(encoded) -> if Bytes.secure_equals(encoded, expected) do
        println("credential-encode")
      else
        println("credential-encode-mismatch")
      end
  end
  case decode_device_credential(expected) do
    Err(_) -> println("credential-decode-error")
    Ok(decoded) -> if decoded.version == 1 && decoded.suite == 1 && U64.compare(decoded.capabilities, value.capabilities) == 0 && U64.compare(decoded.created_at, value.created_at) == 0 && U64.compare(decoded.expires_at, value.expires_at) == 0 && U64.compare(decoded.directory_sequence, value.directory_sequence) == 0 && Bytes.secure_equals(decoded.account_id, value.account_id) && Bytes.secure_equals(decoded.device_id, value.device_id) && Bytes.secure_equals(decoded.signing_public_key, value.signing_public_key) && Bytes.secure_equals(decoded.dh_public_key, value.dh_public_key) && Bytes.secure_equals(decoded.post_quantum_public_key, value.post_quantum_public_key) && Bytes.secure_equals(decoded.signature, value.signature) do
        println("credential-decode")
      else
        println("credential-decode-mismatch")
      end
  end
end

fn proof() -> Int ! String do
  let outer_bytes = Bytes.from_hex("__OUTER_HEX__") ?
  check_outer(OuterEnvelope {
    version: 1,
    envelope_id: Bytes.from_hex("000102030405060708090a0b0c0d0e0f") ?,
    mailbox_token: Bytes.from_hex("202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f") ?,
    suite: 1,
    expiration: U64.parse("1700000000000") ?,
    padding_bucket: 256,
    ciphertext: Bytes.from_hex("a0a1a2a3a4a5a6a7") ?
  }, outer_bytes)
  let suffix = Bytes.from_hex("00") ?
  case decode_outer_envelope(Bytes.concat(outer_bytes, suffix) ?) do
    Err(_) -> println("outer-trailing")
    Ok(_) -> println("outer-trailing-accepted")
  end
  case decode_outer_envelope(Bytes.from_hex("__OUTER_OVERSIZED_HEX__") ?) do
    Err(_) -> println("outer-oversized")
    Ok(_) -> println("outer-oversized-accepted")
  end
  case decode_outer_envelope(Bytes.empty()) do
    Err(_) -> println("outer-hostile")
    Ok(_) -> println("outer-hostile-accepted")
  end

  let credential_bytes = Bytes.from_hex("__CREDENTIAL_HEX__") ?
  check_credential(DeviceCredential {
    version: 1,
    suite: 1,
    account_id: Bytes.from_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f") ?,
    device_id: Bytes.from_hex("202122232425262728292a2b2c2d2e2f") ?,
    signing_public_key: Bytes.from_hex("303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f") ?,
    dh_public_key: Bytes.from_hex("505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f") ?,
    post_quantum_public_key: Bytes.empty(),
    capabilities: U64.parse("5") ?,
    created_at: U64.parse("1700000000000") ?,
    expires_at: U64.parse("1731536000000") ?,
    directory_sequence: U64.parse("42") ?,
    signature: Bytes.from_hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf") ?
  }, credential_bytes)
  case decode_device_credential(Bytes.concat(credential_bytes, suffix) ?) do
    Err(_) -> println("credential-trailing")
    Ok(_) -> println("credential-trailing-accepted")
  end
  case decode_device_credential(Bytes.from_hex("__CREDENTIAL_OVERSIZED_HEX__") ?) do
    Err(_) -> println("credential-oversized")
    Ok(_) -> println("credential-oversized-accepted")
  end
  case decode_device_credential(Bytes.empty()) do
    Err(_) -> println("credential-hostile")
    Ok(_) -> println("credential-hostile-accepted")
  end
  Ok(0)
end

fn main() do
  case proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
end
"##
    .replace("__OUTER_HEX__", outer_hex.trim())
    .replace("__CREDENTIAL_HEX__", credential_hex.trim())
    .replace("__OUTER_OVERSIZED_HEX__", outer_oversized_hex.trim())
    .replace(
        "__CREDENTIAL_OVERSIZED_HEX__",
        credential_oversized_hex.trim(),
    );
    fs::write(project.join("main.mpl"), source).unwrap();

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("messenger-protocol-proof"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "messenger protocol proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "outer-encode\nouter-decode\nouter-trailing\nouter-oversized\nouter-hostile\ncredential-encode\ncredential-decode\ncredential-trailing\ncredential-oversized\ncredential-hostile\n"
    );
}
