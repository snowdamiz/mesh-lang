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
    fs::create_dir_all(destination.join(module).parent().unwrap()).unwrap();
    fs::copy(source.join("mesh.toml"), destination.join("mesh.toml")).unwrap();
    fs::copy(source.join(module), destination.join(module)).unwrap();
}

#[test]
fn canonical_profile_a_values_round_trip() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("messenger-protocol-m5");
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
        "[package]\nname = \"messenger-protocol-m5\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let credential_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m1/device-credential-v1.hex"),
    )
    .unwrap();
    let account_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m5/account-identity-v1.hex"),
    )
    .unwrap();
    let prekey_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m5/prekey-bundle-v1.hex"),
    )
    .unwrap();
    let inner_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m5/inner-envelope-v1.hex"),
    )
    .unwrap();
    let transcript_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m5/handshake-transcript-v1.hex"),
    )
    .unwrap();
    let transcript_hash_hex = fs::read_to_string(
        root.join("mesh-private-messenger/tests/fixtures/m5/handshake-transcript-hash-v1.hex"),
    )
    .unwrap();
    let source = r#"
from Protocol.V1 import AccountIdentity, HandshakeTranscript, InnerEnvelope, PrekeyBundle, ProtocolError, ProtocolExtension, decode_account_identity, decode_handshake_transcript, decode_inner_envelope, decode_prekey_bundle, encode_account_identity, encode_handshake_transcript, encode_inner_envelope, encode_prekey_bundle, hash_handshake_transcript, negotiate_profile_a

fn negotiation_proof() do
  case negotiate_profile_a([1], [1], 0) do
    Err(_) -> println("negotiation-error")
    Ok(suite) -> println("suite:${suite}")
  end
  case negotiate_profile_a([1, 1], [1], 0) do
    Err(DuplicateSuite) -> println("negotiation-duplicate")
    Err(_) -> println("negotiation-wrong-error")
    Ok(_) -> println("negotiation-duplicate-accepted")
  end
  case negotiate_profile_a([1], [2], 0) do
    Err(UnsupportedSuite) -> println("negotiation-unsupported")
    Err(_) -> println("negotiation-wrong-error")
    Ok(_) -> println("negotiation-unsupported-accepted")
  end
  case negotiate_profile_a([1], [1], 2) do
    Err(DowngradeDetected) -> println("negotiation-downgrade")
    Err(_) -> println("negotiation-wrong-error")
    Ok(_) -> println("negotiation-downgrade-accepted")
  end
  case negotiate_profile_a([1], [1], -1) do
    Err(InvalidSuiteHistory) -> println("negotiation-invalid-history")
    Err(_) -> println("negotiation-wrong-error")
    Ok(_) -> println("negotiation-invalid-history-accepted")
  end
end

fn account_proof() -> Int ! String do
  let account = AccountIdentity {
    version: 1,
    account_id: Bytes.from_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f") ?,
    authorization_public_key: Bytes.from_hex("202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f") ?,
    created_at: U64.parse("1700000000000") ?,
    directory_sequence: U64.parse("42") ?,
    extensions: [ProtocolExtension {
      id: 7,
      mandatory: false,
      value: Bytes.from_hex("a0a1a2") ?
    }]
  }
  case encode_account_identity(account) do
    Err(_) -> println("account-encode-error")
    Ok(encoded) -> if !Bytes.secure_equals(encoded, Bytes.from_hex("__ACCOUNT_HEX__") ?) do
        println("account-golden-mismatch")
      else
        case decode_account_identity(encoded) do
        Err(_) -> println("account-decode-error")
        Ok(decoded) -> case encode_account_identity(decoded) do
            Err(_) -> println("account-reencode-error")
            Ok(reencoded) -> if Bytes.secure_equals(encoded, reencoded) do
                println("account-roundtrip")
              else
                println("account-noncanonical")
              end
          end
      end
      end
  end
  Ok(0)
end

fn hostile_account_proof() -> Int ! String do
  let mandatory = Bytes.from_hex("014143540000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100070100000000") ?
  case decode_account_identity(mandatory) do
    Err(UnknownMandatoryExtension) -> println("account-mandatory")
    Err(_) -> println("account-mandatory-wrong-error")
    Ok(_) -> println("account-mandatory-accepted")
  end
  let duplicate = Bytes.from_hex("01414354000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020007000000000000070000000000") ?
  case decode_account_identity(duplicate) do
    Err(NonCanonicalEncoding) -> println("account-duplicate")
    Err(_) -> println("account-duplicate-wrong-error")
    Ok(_) -> println("account-duplicate-accepted")
  end
  case Bytes.repeat(0, 16583) do
    Err(_) -> println("account-limit-setup-error")
    Ok(oversized) -> case decode_account_identity(oversized) do
        Err(OversizedInput) -> println("account-limit")
        Err(_) -> println("account-limit-wrong-error")
        Ok(_) -> println("account-limit-accepted")
      end
  end
  Ok(0)
end

fn all_m5_decoders_reject(input :: Bytes) -> Bool do
  case decode_account_identity(input) do
    Ok(_) -> false
    Err(_) -> case decode_prekey_bundle(input) do
        Ok(_) -> false
        Err(_) -> case decode_inner_envelope(input) do
            Ok(_) -> false
            Err(_) -> case decode_handshake_transcript(input) do
                Ok(_) -> false
                Err(_) -> true
              end
          end
      end
  end
end

fn hostile_decoder_lengths(version :: Bytes, length :: Int) -> Bool do
  if length > 64 do
    true
  else
    case Bytes.repeat(length, length) do
      Err(_) -> false
      Ok(tail) -> case Bytes.concat(version, tail) do
          Err(_) -> false
          Ok(input) -> all_m5_decoders_reject(input) && hostile_decoder_lengths(version, length + 1)
        end
    end
  end
end

fn hostile_decoder_proof() -> Int ! String do
  if hostile_decoder_lengths(Bytes.from_hex("01") ?, 0) do
    println("decoder-hostile-matrix")
  else
    println("decoder-hostile-accepted")
  end
  Ok(0)
end

fn prekey_proof() -> Int ! String do
  let bundle = PrekeyBundle {
    version: 1,
    suite: 1,
    device_credential: Bytes.from_hex("__CREDENTIAL_HEX__") ?,
    identity_dh_public_key: Bytes.from_hex("505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f") ?,
    signing_public_key: Bytes.from_hex("303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f") ?,
    signed_prekey_id: U64.parse("7") ?,
    signed_prekey: Bytes.from_utf8("pppppppppppppppppppppppppppppppp"),
    signed_prekey_signature: Bytes.from_utf8("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
    one_time_prekey_id: U64.parse("9") ?,
    one_time_prekey: Bytes.from_utf8("oooooooooooooooooooooooooooooooo"),
    post_quantum_prekey: Bytes.empty(),
    supported_suites: [1],
    expires_at: U64.parse("1700604800000") ?,
    extensions: [ProtocolExtension {
      id: 11,
      mandatory: false,
      value: Bytes.from_utf8("optional-proof")
    }]
  }
  case encode_prekey_bundle(bundle) do
    Err(_) -> println("prekey-encode-error")
    Ok(encoded) -> if !Bytes.secure_equals(encoded, Bytes.from_hex("__PREKEY_HEX__") ?) do
        println("prekey-golden-mismatch")
      else
        case decode_prekey_bundle(encoded) do
        Err(_) -> println("prekey-decode-error")
        Ok(decoded) -> case encode_prekey_bundle(decoded) do
            Err(_) -> println("prekey-reencode-error")
            Ok(reencoded) -> if Bytes.secure_equals(encoded, reencoded) && Bytes.secure_equals(decoded.one_time_prekey, bundle.one_time_prekey) && List.length(decoded.supported_suites) == 1 && List.get(decoded.supported_suites, 0) == 1 do
                println("prekey-roundtrip")
              else
                println("prekey-noncanonical")
              end
          end
      end
      end
  end
  let invalid = PrekeyBundle {
    version: bundle.version,
    suite: bundle.suite,
    device_credential: bundle.device_credential,
    identity_dh_public_key: bundle.identity_dh_public_key,
    signing_public_key: bundle.signing_public_key,
    signed_prekey_id: bundle.signed_prekey_id,
    signed_prekey: bundle.signed_prekey,
    signed_prekey_signature: bundle.signed_prekey_signature,
    one_time_prekey_id: bundle.one_time_prekey_id,
    one_time_prekey: Bytes.from_utf8("short"),
    post_quantum_prekey: bundle.post_quantum_prekey,
    supported_suites: bundle.supported_suites,
    expires_at: bundle.expires_at,
    extensions: bundle.extensions
  }
  case encode_prekey_bundle(invalid) do
    Err(InvalidFieldLength) -> println("prekey-invalid-length")
    Err(_) -> println("prekey-invalid-wrong-error")
    Ok(_) -> println("prekey-invalid-accepted")
  end
  Ok(0)
end

fn inner_value(body :: Bytes, timestamp :: U64) -> InnerEnvelope do
  InnerEnvelope {
    version: 1,
    sender_account_id: Bytes.from_utf8("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    sender_device_id: Bytes.from_utf8("ssssssssssssssss"),
    recipient_device_id: Bytes.from_utf8("rrrrrrrrrrrrrrrr"),
    conversation_id: Bytes.from_utf8("cccccccccccccccc"),
    client_message_id: Bytes.from_utf8("mmmmmmmmmmmmmmmm"),
    client_timestamp: timestamp,
    message_type: 1,
    body: body,
    reply_reference: Bytes.empty(),
    attachment_manifest: Bytes.empty(),
    receipt_policy: 1,
    disappearing_seconds: 0,
    extensions: []
  }
end

fn inner_round_trip(size :: Int, timestamp :: U64) -> Bool do
  case Bytes.repeat(165, size) do
    Err(_) -> false
    Ok(body) -> case encode_inner_envelope(inner_value(body, timestamp)) do
        Err(_) -> false
        Ok(encoded) -> case decode_inner_envelope(encoded) do
            Err(_) -> false
            Ok(decoded) -> case encode_inner_envelope(decoded) do
                Err(_) -> false
                Ok(reencoded) -> Bytes.secure_equals(encoded, reencoded) && Bytes.length(decoded.body) == size
              end
          end
      end
  end
end

fn inner_properties(sizes :: List < Int >, index :: Int, timestamp :: U64) -> Bool do
  if index >= List.length(sizes) do
    true
  else
    inner_round_trip(List.get(sizes, index), timestamp) && inner_properties(sizes, index + 1, timestamp)
  end
end

fn inner_proof() -> Int ! String do
  let timestamp = U64.parse("1700000000000") ?
  if inner_properties([0, 1, 255, 4096, 32768], 0, timestamp) do
    println("inner-properties")
  else
    println("inner-property-failed")
  end
  case Bytes.from_hex("a5") do
    Err(error) -> println(error)
    Ok(body) -> case encode_inner_envelope(inner_value(body, timestamp)) do
        Err(_) -> println("inner-golden-encode-error")
        Ok(encoded) -> if Bytes.secure_equals(encoded, Bytes.from_hex("__INNER_HEX__") ?) do
            println("inner-golden")
          else
            println("inner-golden-mismatch")
          end
      end
  end
  case Bytes.repeat(0, 32769) do
    Err(_) -> println("inner-limit-setup-error")
    Ok(body) -> case encode_inner_envelope(inner_value(body, timestamp)) do
        Err(OversizedInput) -> println("inner-limit")
        Err(_) -> println("inner-limit-wrong-error")
        Ok(_) -> println("inner-limit-accepted")
      end
  end
  Ok(0)
end

fn transcript_proof() -> Int ! String do
  let transcript = HandshakeTranscript {
    version: 1,
    suite: 1,
    initiator_credential_hash: Bytes.from_utf8("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    responder_prekey_bundle_hash: Bytes.from_utf8("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    initiator_ephemeral_public_key: Bytes.from_utf8("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
    signed_prekey_id: U64.parse("7") ?,
    responder_signed_prekey: Bytes.from_utf8("pppppppppppppppppppppppppppppppp"),
    one_time_prekey_id: U64.parse("0") ?,
    responder_one_time_prekey: Bytes.empty(),
    responder_post_quantum_prekey: Bytes.empty(),
    extensions: [ProtocolExtension {
      id: 13,
      mandatory: false,
      value: Bytes.from_utf8("transcript")
    }]
  }
  case encode_handshake_transcript(transcript) do
    Err(_) -> println("transcript-encode-error")
    Ok(encoded) -> if !Bytes.secure_equals(encoded, Bytes.from_hex("__TRANSCRIPT_HEX__") ?) do
        println("transcript-golden-mismatch")
      else
        case decode_handshake_transcript(encoded) do
        Err(_) -> println("transcript-decode-error")
        Ok(decoded) -> case encode_handshake_transcript(decoded) do
            Err(_) -> println("transcript-reencode-error")
            Ok(reencoded) -> case hash_handshake_transcript(transcript) do
                Err(_) -> println("transcript-hash-error")
                Ok(first_hash) -> case hash_handshake_transcript(decoded) do
                    Err(_) -> println("transcript-hash-error")
                    Ok(second_hash) -> if Bytes.secure_equals(encoded, reencoded) && Bytes.secure_equals(first_hash, second_hash) && Bytes.secure_equals(first_hash, Bytes.from_hex("__TRANSCRIPT_HASH_HEX__") ?) do
                        println("transcript-roundtrip")
                      else
                        println("transcript-noncanonical")
                      end
                  end
              end
          end
      end
      end
  end
  Ok(0)
end

fn main() do
  negotiation_proof()
  case account_proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
  case hostile_account_proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
  case hostile_decoder_proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
  case prekey_proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
  case inner_proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
  case transcript_proof() do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
end
"#
    .replace("__CREDENTIAL_HEX__", credential_hex.trim())
    .replace("__ACCOUNT_HEX__", account_hex.trim())
    .replace("__PREKEY_HEX__", prekey_hex.trim())
    .replace("__INNER_HEX__", inner_hex.trim())
    .replace("__TRANSCRIPT_HEX__", transcript_hex.trim())
    .replace("__TRANSCRIPT_HASH_HEX__", transcript_hash_hex.trim());
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
    let run = Command::new(project.join("messenger-protocol-m5"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "messenger protocol proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stderr), "");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "suite:1\nnegotiation-duplicate\nnegotiation-unsupported\nnegotiation-downgrade\nnegotiation-invalid-history\naccount-roundtrip\naccount-mandatory\naccount-duplicate\naccount-limit\ndecoder-hostile-matrix\nprekey-roundtrip\nprekey-invalid-length\ninner-properties\ninner-golden\ninner-limit\ntranscript-roundtrip\n"
    );
}
