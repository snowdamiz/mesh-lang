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
fn profile_a_ratchet_delivers_out_of_order_messages_within_the_bound() {
    artifacts::ensure_mesh_rt_staticlib();

    let root = workspace_root();
    let temp = tempfile::tempdir().expect("temporary project");
    let project = temp.path().join("messenger-ratchet-proof");
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
        "[package]\nname = \"messenger-ratchet-proof\"\nversion = \"0.1.0\"\n",
    )
    .expect("proof manifest");
    fs::write(
        project.join("main.mpl"),
        r#"
from Identity.Device import AccountKeys, DeviceKeys, IdentityError, VerificationPolicy, generate_account, generate_device, issue_device_credential
from Prekeys.Bundle import OneTimePrekeySecrets, PostQuantumPrekeySecrets, PrekeyError, SignedPrekeySecrets, build_prekey_bundle, generate_one_time_prekey, generate_post_quantum_prekey, generate_signed_prekey
from Protocol.V1 import AccountIdentity, DeviceCredential, InitialMessage, PrekeyBundle, encode_initial_message
from Session.Handshake import RatchetState, SessionError, initiate, receive_initial
from Session.Ratchet import DecryptOutcome, RatchetError, RatchetMessage, decode_ratchet_message, decrypt, encode_ratchet_message, encrypt
from Session.Snapshot import ReplacementOutcome, SnapshotError, SnapshotOutcome, replace_session, restore, snapshot

type ProofError do
  CryptoProblem(error :: CryptoError)
  IdentityProblem(error :: IdentityError)
  PrekeyProblem(error :: PrekeyError)
  SessionProblem(error :: SessionError)
  RatchetProblem(error :: RatchetError)
  SnapshotProblem(error :: SnapshotError)
  InvalidFixture
end

fn wide(value :: String) -> U64 ! ProofError do
  case U64.parse(value) do
    Err(_) -> Err(InvalidFixture)
    Ok(parsed) -> Ok(parsed)
  end
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
  expires_at :: U64
) -> DeviceCredential ! ProofError do
  case issue_device_credential(
    account_keys,
    device_keys,
    wide("1") ?,
    created_at,
    expires_at,
    wide("1") ?
  ) do
    Err(error) -> Err(IdentityProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn signed_prekey(
  device_keys :: borrow DeviceKeys,
  value :: DeviceCredential,
  expires_at :: U64
) -> SignedPrekeySecrets ! ProofError do
  case generate_signed_prekey(device_keys, value, wide("1") ?, expires_at) do
    Err(error) -> Err(PrekeyProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn one_time_prekey() -> OneTimePrekeySecrets ! ProofError do
  case generate_one_time_prekey(wide("2") ?) do
    Err(error) -> Err(PrekeyProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn post_quantum_prekey() -> PostQuantumPrekeySecrets ! ProofError do
  case generate_post_quantum_prekey() do
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

fn encoded_initial(value :: InitialMessage) -> Bytes ! ProofError do
  case encode_initial_message(value) do
    Err(_) -> Err(InvalidFixture)
    Ok(encoded) -> Ok(encoded)
  end
end

fn wire_message(value :: RatchetMessage) -> RatchetMessage ! ProofError do
  let encoded = case encode_ratchet_message(value) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(bytes) -> Ok(bytes)
  end ?
  let trailing = case Bytes.concat(encoded, Bytes.from_utf8("x")) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  let truncated = case Bytes.slice(encoded, 0, Bytes.length(encoded) - 1) do
    Err(_) -> Err(InvalidFixture)
    Ok(bytes) -> Ok(bytes)
  end ?
  let _ = case decode_ratchet_message(trailing) do
    Err(InvalidMessage) -> Ok(nil)
    Err(error) -> Err(RatchetProblem(error))
    Ok(_) -> Err(InvalidFixture)
  end ?
  let _ = case decode_ratchet_message(truncated) do
    Err(InvalidMessage) -> Ok(nil)
    Err(error) -> Err(RatchetProblem(error))
    Ok(_) -> Err(InvalidFixture)
  end ?
  case decode_ratchet_message(encoded) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(decoded) -> Ok(decoded)
  end
end

fn opened(state :: consume RatchetState, value :: Bytes, expected :: Bytes) -> RatchetState do
  if Bytes.secure_equals(value, expected) do
    println("ratchet:ok")
  else
    println("ratchet:wrong-plaintext")
  end
  state
end

fn rejected(state :: consume RatchetState, error :: RatchetError) -> RatchetState do
  println("ratchet:rejected")
  state
end

fn accept_message(state :: consume RatchetState,
message :: RatchetMessage,
associated_data :: Bytes,
expected :: Bytes) -> RatchetState do
  case decrypt(state, message, associated_data) do
    Opened(next, value) -> opened(next, value, expected)
    Rejected(next, error) -> rejected(next, error)
  end
end

fn expect_replay(state :: consume RatchetState,
message :: RatchetMessage,
associated_data :: Bytes) -> RatchetState do
  case decrypt(state, message, associated_data) do
    Opened(next, _) -> do
      println("replay:opened")
      next
    end
    Rejected(next, Replay) -> do
      println("replay:ok")
      next
    end
    Rejected(next, _) -> do
      println("replay:wrong-error")
      next
    end
  end
end

fn expect_jump_rejection(state :: consume RatchetState,
message :: RatchetMessage,
associated_data :: Bytes) -> RatchetState do
  case decrypt(state, message, associated_data) do
    Opened(next, _) -> do
      println("jump:opened")
      next
    end
    Rejected(next, ExcessiveJump) -> do
      println("jump:ok")
      next
    end
    Rejected(next, _) -> do
      println("jump:wrong-error")
      next
    end
  end
end

fn expect_authentication_rejection(state :: consume RatchetState,
message :: RatchetMessage,
associated_data :: Bytes) -> RatchetState do
  case decrypt(state, message, associated_data) do
    Opened(next, _) -> do
      println("authentication:opened")
      next
    end
    Rejected(next, AuthenticationRejected) -> do
      println("authentication:ok")
      next
    end
    Rejected(next, _) -> do
      println("authentication:wrong-error")
      next
    end
  end
end

fn storage_key() -> StorageKey ! ProofError do
  case StorageKey.ephemeral() do
    Err(error) -> Err(CryptoProblem(error))
    Ok(value) -> Ok(value)
  end
end

fn sealed(state :: consume RatchetState,
wrapping_key :: borrow StorageKey,
account_id :: Bytes,
device_id :: Bytes,
version :: U64) -> Result<(RatchetState, Bytes), ProofError> do
  case snapshot(state, wrapping_key, account_id, device_id, version) do
    SnapshotSealed(next, blob) -> Ok((next, blob))
    SnapshotRejected(rejected, _) -> do
      println("snapshot:seal-rejected")
      Ok((rejected, Bytes.empty()))
    end
  end
end

fn restored(blob :: Bytes,
wrapping_key :: borrow StorageKey,
account_id :: Bytes,
device_id :: Bytes,
minimum_version :: U64) -> RatchetState ! ProofError do
  case restore(blob, wrapping_key, account_id, device_id, minimum_version) do
    Err(error) -> Err(SnapshotProblem(error))
    Ok(state) -> do
      println("snapshot:restored")
      Ok(state)
    end
  end
end

fn replaced(current :: consume RatchetState,
blob :: Bytes,
wrapping_key :: borrow StorageKey,
account_id :: Bytes,
device_id :: Bytes) -> RatchetState ! ProofError do
  case replace_session(current, blob, wrapping_key, account_id, device_id) do
    ReplacementRejected(rejected, _) -> do
      println("snapshot:replace-rejected")
      Ok(rejected)
    end
    SessionReplaced(state) -> do
      println("snapshot:replaced")
      Ok(state)
    end
  end
end

fn rejects_rollback(current :: consume RatchetState,
blob :: Bytes,
wrapping_key :: borrow StorageKey,
account_id :: Bytes,
device_id :: Bytes) -> RatchetState do
  case replace_session(current, blob, wrapping_key, account_id, device_id) do
    SessionReplaced(state) -> do
      println("snapshot:rollback-replaced")
      state
    end
    ReplacementRejected(state, RollbackRejected) -> do
      println("snapshot:rollback-ok")
      state
    end
    ReplacementRejected(state, _) -> do
      println("snapshot:rollback-wrong-error")
      state
    end
  end
end

fn proof() -> Int ! ProofError do
  let created_at = wide("1700000000000") ?
  let expires_at = wide("1700604800000") ?
  let policy = VerificationPolicy {
    current_time: created_at,
    minimum_directory_sequence: wide("1") ?
  }
  let (alice_account_keys, alice_account) = account(created_at) ?
  let (bob_account_keys, bob_account) = account(created_at) ?
  let alice = device() ?
  let bob = device() ?
  let post_quantum = post_quantum_prekey() ?
  let alice_credential = credential(
    alice_account_keys,
    alice,
    created_at,
    expires_at
  ) ?
  let bob_credential = credential(
    bob_account_keys,
    bob,
    created_at,
    expires_at
  ) ?
  let signed = signed_prekey(bob, bob_credential, expires_at) ?
  let one_time = one_time_prekey() ?
  let published = bundle(bob_credential, signed, one_time) ?
  let (alice_session, initial) = case initiate(
    alice,
    alice_credential,
    bob_account,
    published,
    policy,
    1,
    Bytes.from_utf8("offline hello")
  ) do
    Err(error) -> Err(SessionProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let (bob_session, _) = case receive_initial(
    bob,
    bob_account,
    published,
    signed,
    one_time,
    post_quantum,
    alice_account,
    policy,
    policy,
    1,
    encoded_initial(initial) ?
  ) do
    Err(error) -> Err(SessionProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let first = Bytes.from_utf8("ratcheted one")
  let second = Bytes.from_utf8("ratcheted two")
  let third = Bytes.from_utf8("ratcheted three")
  let fourth = Bytes.from_utf8("ratcheted four")
  let associated_data = Bytes.from_utf8("conversation-1")
  let (alice_session, first_message) = case encrypt(
    alice_session,
    first,
    associated_data
  ) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let (alice_session, second_message) = case encrypt(
    alice_session,
    second,
    associated_data
  ) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let (alice_session, third_message) = case encrypt(
    alice_session,
    third,
    associated_data
  ) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let (alice_session, fourth_message) = case encrypt(
    alice_session,
    fourth,
    associated_data
  ) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let first_message = wire_message(first_message) ?
  let second_message = wire_message(second_message) ?
  let third_message = wire_message(third_message) ?
  let fourth_message = wire_message(fourth_message) ?
  let bob_session = accept_message(bob_session, third_message, associated_data, third)
  let wrapping_key = storage_key() ?
  let (persisted_session, blob_v1) = sealed(
    bob_session,
    wrapping_key,
    bob_account.account_id,
    bob_credential.device_id,
    wide("1") ?
  ) ?
  let bob_session = restored(
    blob_v1,
    wrapping_key,
    bob_account.account_id,
    bob_credential.device_id,
    wide("1") ?
  ) ?
  let bob_session = accept_message(bob_session, first_message, associated_data, first)
  let bob_session = accept_message(bob_session, second_message, associated_data, second)
  let (replacement_source, blob_v2) = sealed(
    bob_session,
    wrapping_key,
    bob_account.account_id,
    bob_credential.device_id,
    wide("2") ?
  ) ?
  let bob_session = replaced(
    persisted_session,
    blob_v2,
    wrapping_key,
    bob_account.account_id,
    bob_credential.device_id
  ) ?
  let bob_session = rejects_rollback(
    bob_session,
    blob_v1,
    wrapping_key,
    bob_account.account_id,
    bob_credential.device_id
  )
  let bob_session = expect_replay(bob_session, first_message, associated_data)
  let excessive = %{fourth_message | message_number: 100}
  let bob_session = expect_jump_rejection(bob_session, excessive, associated_data)
  let bob_session = expect_authentication_rejection(
    bob_session,
    fourth_message,
    Bytes.from_utf8("wrong-conversation")
  )
  let bob_session = accept_message(bob_session, fourth_message, associated_data, fourth)
  let response = Bytes.from_utf8("ratcheted response")
  let (_bob_session, response_message) = case encrypt(
    bob_session,
    response,
    associated_data
  ) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let _alice_session = accept_message(
    alice_session,
    response_message,
    associated_data,
    response
  )
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
    let run = Command::new(project.join("messenger-ratchet-proof"))
        .output()
        .expect("run ratchet proof");
    assert!(
        run.status.success(),
        "ratchet proof failed with {}:\nstdout:\n{}\nstderr:\n{}",
        run.status,
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(run.stderr.is_empty(), "unexpected stderr: {:?}", run.stderr);
    assert_eq!(
      String::from_utf8_lossy(&run.stdout),
        "ratchet:ok\nsnapshot:restored\nratchet:ok\nratchet:ok\nsnapshot:replaced\nsnapshot:rollback-ok\nreplay:ok\njump:ok\nauthentication:ok\nratchet:ok\nratchet:ok\n"
    );
}
