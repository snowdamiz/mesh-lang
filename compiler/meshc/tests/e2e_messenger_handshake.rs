#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("test executable")
        .parent()
        .expect("test executable directory")
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
        .expect("workspace root")
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create package directory");
    for entry in fs::read_dir(source).expect("read package directory") {
        let entry = entry.expect("package entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().expect("package entry type").is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("copy package file");
        }
    }
}

#[test]
fn classical_offline_handshake_is_entirely_mesh_and_fail_closed() {
    artifacts::ensure_mesh_rt_staticlib();

    let root = workspace_root();
    let temp = tempfile::tempdir().expect("temporary project");
    let project = temp.path().join("messenger-handshake-proof");
    let installed = project.join(".mesh/packages");
    copy_tree(
        &root.join("mesh-lang/packages/mesh-binary"),
        &installed.join("mesh-binary@0.1.0"),
    );
    copy_tree(
        &root.join("mesh-private-messenger/packages/messenger-protocol"),
        &installed.join("messenger-protocol@0.1.0"),
    );
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"messenger-handshake-proof\"\nversion = \"0.1.0\"\n",
    )
    .expect("proof manifest");
    fs::write(
        project.join("main.mpl"),
        r#"
from Identity.Device import AccountKeys, DeviceKeys, IdentityError, VerificationPolicy, credential_signing_bytes, generate_account, generate_device, issue_device_credential
from Prekeys.Bundle import OneTimePrekeySecrets, PrekeyError, SignedPrekeySecrets, build_prekey_bundle, generate_one_time_prekey, generate_signed_prekey
from Protocol.V1 import AccountIdentity, DeviceCredential, InitialMessage, PrekeyBundle, ProtocolError, decode_initial_message, encode_initial_message
from Session.Handshake import RatchetState, SessionError, initiate, receive_initial

type ProofError do
  IdentityProblem(error :: IdentityError)
  PrekeyProblem(error :: PrekeyError)
  SessionProblem(error :: SessionError)
  InvalidFixture
end

fn wide(value :: String) -> U64 ! ProofError do
  case U64.parse(value) do
    Err(_) -> Err(InvalidFixture)
    Ok(parsed) -> Ok(parsed)
  end
end

fn policy(current_time :: U64, minimum_directory_sequence :: U64) -> VerificationPolicy do
  VerificationPolicy {
    current_time: current_time,
    minimum_directory_sequence: minimum_directory_sequence
  }
end

fn account(created_at :: U64) -> Result < (AccountKeys, AccountIdentity), ProofError > do
  case generate_account(created_at, wide("1") ?) do
    Err(error) -> Err(IdentityProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn device() -> DeviceKeys ! ProofError do
  case generate_device() do
    Err(error) -> Err(IdentityProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn credential(
  account_keys :: borrow AccountKeys,
  device_keys :: borrow DeviceKeys,
  created_at :: U64,
  expires_at :: U64,
  directory_sequence :: U64
) -> DeviceCredential ! ProofError do
  case issue_device_credential(
    account_keys,
    device_keys,
    wide("1") ?,
    created_at,
    expires_at,
    directory_sequence
  ) do
    Err(error) -> Err(IdentityProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn account_sequence(value :: AccountIdentity, directory_sequence :: U64) -> AccountIdentity do
  AccountIdentity {
    version: value.version,
    account_id: value.account_id,
    authorization_public_key: value.authorization_public_key,
    created_at: value.created_at,
    directory_sequence: directory_sequence,
    extensions: value.extensions
  }
end

fn signing_bytes(value :: DeviceCredential) -> Bytes ! ProofError do
  case credential_signing_bytes(value) do
    Err(error) -> Err(IdentityProblem(error))
    Ok(bytes) -> Ok(bytes)
  end
end

fn encode_initial(value :: InitialMessage) -> Bytes ! ProofError do
  case encode_initial_message(value) do
    Err(error) -> Err(SessionProblem(ProtocolFailure(error)))
    Ok(bytes) -> Ok(bytes)
  end
end

fn signed_prekey(
  device_keys :: borrow DeviceKeys,
  value :: DeviceCredential,
  id :: U64,
  expires_at :: U64
) -> SignedPrekeySecrets ! ProofError do
  case generate_signed_prekey(device_keys, value, id, expires_at) do
    Err(error) -> Err(PrekeyProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn one_time_prekey(id :: U64) -> OneTimePrekeySecrets ! ProofError do
  case generate_one_time_prekey(id) do
    Err(error) -> Err(PrekeyProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn bundle(
  value :: DeviceCredential,
  signed :: borrow SignedPrekeySecrets,
  one_time :: borrow OneTimePrekeySecrets
) -> PrekeyBundle ! ProofError do
  case build_prekey_bundle(value, signed, one_time) do
    Err(error) -> Err(PrekeyProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn start(
  initiator :: borrow DeviceKeys,
  initiator_credential :: DeviceCredential,
  responder_account :: AccountIdentity,
  responder_bundle :: PrekeyBundle,
  responder_policy :: VerificationPolicy,
  plaintext :: Bytes
) -> Result < (RatchetState, InitialMessage), ProofError > do
  case initiate(
    initiator,
    initiator_credential,
    responder_account,
    responder_bundle,
    responder_policy,
    1,
    plaintext
  ) do
    Err(error) -> Err(SessionProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn accept_handshake(
  responder :: borrow DeviceKeys,
  responder_account :: AccountIdentity,
  responder_bundle :: PrekeyBundle,
  signed :: borrow SignedPrekeySecrets,
  one_time :: consume OneTimePrekeySecrets,
  initiator_account :: AccountIdentity,
  responder_policy :: VerificationPolicy,
  initiator_policy :: VerificationPolicy,
  message :: Bytes
) -> Result < (RatchetState, Bytes), ProofError > do
  case receive_initial(
    responder,
    responder_account,
    responder_bundle,
    signed,
    one_time,
    initiator_account,
    responder_policy,
    initiator_policy,
    message
  ) do
    Err(error) -> Err(SessionProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn accepted_start(value :: consume (RatchetState, InitialMessage), message :: String) do
  println(message)
end

fn accepted_receive(value :: consume (RatchetState, Bytes), message :: String) do
  println(message)
end

fn invalid_signature_bundle(value :: PrekeyBundle) -> PrekeyBundle ! ProofError do
  let zero_signature = case Bytes.repeat(0, 64) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  Ok(PrekeyBundle {
    version: value.version,
    suite: value.suite,
    device_credential: value.device_credential,
    identity_dh_public_key: value.identity_dh_public_key,
    signing_public_key: value.signing_public_key,
    signed_prekey_id: value.signed_prekey_id,
    signed_prekey: value.signed_prekey,
    signed_prekey_signature: zero_signature,
    one_time_prekey_id: value.one_time_prekey_id,
    one_time_prekey: value.one_time_prekey,
    supported_suites: value.supported_suites,
    expires_at: value.expires_at,
    extensions: value.extensions
  })
end

fn rejected_ciphertext(value :: InitialMessage) -> InitialMessage ! ProofError do
  let ciphertext = case Bytes.repeat(0, Bytes.length(value.ciphertext)) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  Ok(InitialMessage {
    version: value.version,
    suite: value.suite,
    signed_prekey_id: value.signed_prekey_id,
    one_time_prekey_id: value.one_time_prekey_id,
    initiator_credential: value.initiator_credential,
    initiator_identity_public_key: value.initiator_identity_public_key,
    initiator_ephemeral_public_key: value.initiator_ephemeral_public_key,
    transcript_hash: value.transcript_hash,
    nonce: value.nonce,
    ciphertext: ciphertext
  })
end

fn ciphertext_size(value :: InitialMessage, size :: Int) -> InitialMessage ! ProofError do
  let ciphertext = case Bytes.repeat(0, size) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  Ok(InitialMessage {
    version: value.version,
    suite: value.suite,
    signed_prekey_id: value.signed_prekey_id,
    one_time_prekey_id: value.one_time_prekey_id,
    initiator_credential: value.initiator_credential,
    initiator_identity_public_key: value.initiator_identity_public_key,
    initiator_ephemeral_public_key: value.initiator_ephemeral_public_key,
    transcript_hash: value.transcript_hash,
    nonce: value.nonce,
    ciphertext: ciphertext
  })
end

fn hostile_magic(value :: Bytes) -> Bytes ! ProofError do
  let tail = case Bytes.slice(value, 4, Bytes.length(value) - 4) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  let version = case Bytes.from_list([1]) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  let prefix = case Bytes.concat(version, Bytes.from_utf8("BAD")) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  case Bytes.concat(prefix, tail) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end
end

fn initial_codec_proof(value :: InitialMessage) -> Int ! ProofError do
  let encoded = encode_initial(value) ?
  let truncated = case Bytes.slice(encoded, 0, Bytes.length(encoded) - 1) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  case decode_initial_message(truncated) do
    Err(_) -> println("initial-truncated:rejected")
    Ok(_) -> println("initial-truncated:accepted")
  end
  case decode_initial_message(hostile_magic(encoded) ?) do
    Err(_) -> println("initial-hostile:rejected")
    Ok(_) -> println("initial-hostile:accepted")
  end
  case Bytes.repeat(0, 65537) do
    Err(_) -> println("initial-limit:setup-error")
    Ok(oversized) -> case decode_initial_message(oversized) do
        Err(OversizedInput) -> println("initial-limit:rejected")
        Err(_) -> println("initial-limit:wrong-error")
        Ok(_) -> println("initial-limit:accepted")
      end
  end
  case encode_initial_message(ciphertext_size(value, 65187) ?) do
    Err(_) -> println("initial-ciphertext-boundary:rejected")
    Ok(boundary) -> if Bytes.length(boundary) == 65536 do
        println("initial-ciphertext-boundary:accepted")
      else
        println("initial-ciphertext-boundary:wrong-size")
      end
  end
  case encode_initial_message(ciphertext_size(value, 65188) ?) do
    Err(OversizedInput) -> println("initial-ciphertext-limit:rejected")
    Err(_) -> println("initial-ciphertext-limit:wrong-error")
    Ok(_) -> println("initial-ciphertext-limit:accepted")
  end
  Ok(0)
end

fn proof() -> Int ! ProofError do
  let created_at = wide("1700000000000") ?
  let current_time = wide("1700000000001") ?
  let future_time = wide("1700000000002") ?
  let expires_at = wide("1700604800000") ?
  let first_sequence = wide("1") ?
  let second_sequence = wide("2") ?
  let verification = policy(current_time, first_sequence)
  let strict_verification = policy(current_time, second_sequence)
  let (alice_account_keys, alice_account) = account(created_at) ?
  let (bob_account_keys, bob_account) = account(created_at) ?
  let alice = device() ?
  let bob = device() ?
  let alice_credential = credential(
    alice_account_keys,
    alice,
    created_at,
    expires_at,
    first_sequence
  ) ?
  let bob_credential = credential(
    bob_account_keys,
    bob,
    created_at,
    expires_at,
    first_sequence
  ) ?
  let signed = signed_prekey(bob, bob_credential, wide("1") ?, expires_at) ?
  let one_time = one_time_prekey(wide("2") ?) ?
  let published = bundle(bob_credential, signed, one_time) ?
  let plaintext = Bytes.from_utf8("offline hello")

  case Bytes.slice(signing_bytes(alice_credential) ?, 0, 29) do
    Err(_) -> println("credential-domain:error")
    Ok(prefix) -> if Bytes.secure_equals(prefix, Bytes.from_utf8("mesh-msg/v1/device-credential")) do
        println("credential-domain:bound")
      else
        println("credential-domain:missing")
      end
  end

  case Bytes.repeat(0, 65172) do
    Err(_) -> println("initial-plaintext-limit:setup-error")
    Ok(oversized_plaintext) -> case initiate(
        alice,
        alice_credential,
        bob_account,
        published,
        verification,
        1,
        oversized_plaintext
      ) do
        Err(_) -> println("initial-plaintext-limit:rejected")
        Ok(value) -> accepted_start(value, "initial-plaintext-limit:accepted")
      end
  end

  let current_bob_credential = credential(
    bob_account_keys,
    bob,
    created_at,
    expires_at,
    second_sequence
  ) ?
  let account_rollback_signed = signed_prekey(bob,
  current_bob_credential,
  wide("8") ?,
  expires_at) ?
  let account_rollback_one_time = one_time_prekey(wide("9") ?) ?
  let account_rollback_bundle = bundle(current_bob_credential,
  account_rollback_signed,
  account_rollback_one_time) ?
  case initiate(
    alice,
    alice_credential,
    bob_account,
    account_rollback_bundle,
    strict_verification,
    1,
    plaintext
  ) do
    Err(_) -> println("account-rollback:rejected")
    Ok(value) -> accepted_start(value, "account-rollback:accepted")
  end

  let expired_signed = signed_prekey(bob, bob_credential, wide("10") ?, created_at) ?
  let expired_one_time = one_time_prekey(wide("11") ?) ?
  let expired_bundle = bundle(bob_credential, expired_signed, expired_one_time) ?
  case initiate(
    alice,
    alice_credential,
    bob_account,
    expired_bundle,
    verification,
    1,
    plaintext
  ) do
    Err(_) -> println("expired-bundle:rejected")
    Ok(value) -> accepted_start(value, "expired-bundle:accepted")
  end

  let future_credential = credential(
    bob_account_keys,
    bob,
    future_time,
    expires_at,
    first_sequence
  ) ?
  let future_signed = signed_prekey(bob, future_credential, wide("12") ?, expires_at) ?
  let future_one_time = one_time_prekey(wide("13") ?) ?
  let future_bundle = bundle(future_credential, future_signed, future_one_time) ?
  case initiate(
    alice,
    alice_credential,
    bob_account,
    future_bundle,
    verification,
    1,
    plaintext
  ) do
    Err(_) -> println("future-credential:rejected")
    Ok(value) -> accepted_start(value, "future-credential:accepted")
  end

  case initiate(
    alice,
    alice_credential,
    bob_account,
    invalid_signature_bundle(published) ?,
    verification,
    1,
    plaintext
  ) do
    Err(_) -> println("invalid-signature:rejected")
    Ok(value) -> accepted_start(value, "invalid-signature:accepted")
  end

  let (alice_session, initial) = start(
    alice,
    alice_credential,
    bob_account,
    published,
    verification,
    plaintext
  ) ?
  let _ = initial_codec_proof(initial) ?
  let (bob_session, opened) = accept_handshake(
    bob,
    bob_account,
    published,
    signed,
    one_time,
    alice_account,
    verification,
    verification,
    encode_initial(initial) ?
  ) ?
  if Bytes.secure_equals(opened, plaintext) and Bytes.secure_equals(alice_session.session_id, bob_session.session_id) do
    println("offline-handshake:ok")
  else
    println("offline-handshake:failed")
  end

  let expired_credential = credential(
    alice_account_keys,
    alice,
    created_at,
    created_at,
    first_sequence
  ) ?
  let expired_credential_one_time = one_time_prekey(wide("14") ?) ?
  let expired_credential_bundle = bundle(bob_credential, signed, expired_credential_one_time) ?
  let (_expired_sender, expired_message) = start(
    alice,
    expired_credential,
    bob_account,
    expired_credential_bundle,
    verification,
    plaintext
  ) ?
  case receive_initial(
    bob,
    bob_account,
    expired_credential_bundle,
    signed,
    expired_credential_one_time,
    alice_account,
    verification,
    verification,
    encode_initial(expired_message) ?
  ) do
    Err(_) -> println("expired-credential:rejected")
    Ok(value) -> accepted_receive(value, "expired-credential:accepted")
  end


  let credential_rollback_one_time = one_time_prekey(wide("15") ?) ?
  let credential_rollback_bundle = bundle(bob_credential,
  signed,
  credential_rollback_one_time) ?
  let (_rollback_sender, rollback_message) = start(
    alice,
    alice_credential,
    bob_account,
    credential_rollback_bundle,
    verification,
    plaintext
  ) ?
  case receive_initial(
    bob,
    bob_account,
    credential_rollback_bundle,
    signed,
    credential_rollback_one_time,
    account_sequence(alice_account, second_sequence),
    verification,
    strict_verification,
    encode_initial(rollback_message) ?
  ) do
    Err(_) -> println("credential-rollback:rejected")
    Ok(value) -> accepted_receive(value, "credential-rollback:accepted")
  end

  let rejected_one_time = one_time_prekey(wide("3") ?) ?
  let rejected_bundle = bundle(bob_credential, signed, rejected_one_time) ?
  let (_candidate_sender, candidate_message) = start(
    alice,
    alice_credential,
    bob_account,
    rejected_bundle,
    verification,
    plaintext
  ) ?
  case receive_initial(
    bob,
    bob_account,
    rejected_bundle,
    signed,
    rejected_one_time,
    alice_account,
    verification,
    verification,
    encode_initial(rejected_ciphertext(candidate_message) ?) ?
  ) do
    Err(AuthenticationRejected) -> println("failed-auth:no-session")
    Err(_) -> println("failed-auth:wrong-error")
    Ok(value) -> accepted_receive(value, "failed-auth:session-created")
  end
  Ok(0)
end

fn main() do
  case proof() do
    Err(_) -> println("proof:error")
    Ok(_) -> nil
  end
end
"#,
    )
    .expect("proof source");

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().expect("project path")])
        .output()
        .expect("run meshc");
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("messenger-handshake-proof"))
        .output()
        .expect("run handshake proof");
    assert!(
        run.status.success(),
        "handshake proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stderr.is_empty(), "unexpected stderr: {:?}", run.stderr);
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "credential-domain:bound\ninitial-plaintext-limit:rejected\naccount-rollback:rejected\nexpired-bundle:rejected\nfuture-credential:rejected\ninvalid-signature:rejected\ninitial-truncated:rejected\ninitial-hostile:rejected\ninitial-limit:rejected\ninitial-ciphertext-boundary:accepted\ninitial-ciphertext-limit:rejected\noffline-handshake:ok\nexpired-credential:rejected\ncredential-rollback:rejected\nfailed-auth:no-session\n"
    );
}
