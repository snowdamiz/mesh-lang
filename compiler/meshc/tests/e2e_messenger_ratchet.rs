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
fn profile_a_ratchet_delivers_an_in_order_message() {
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
from Prekeys.Bundle import OneTimePrekeySecrets, PrekeyError, SignedPrekeySecrets, build_prekey_bundle, generate_one_time_prekey, generate_signed_prekey
from Protocol.V1 import AccountIdentity, DeviceCredential, InitialMessage, PrekeyBundle, encode_initial_message
from Session.Handshake import RatchetState, SessionError, initiate, receive_initial
from Session.Ratchet import DecryptOutcome, RatchetError, decrypt, encrypt

type ProofError do
  IdentityProblem(error :: IdentityError)
  PrekeyProblem(error :: PrekeyError)
  SessionProblem(error :: SessionError)
  RatchetProblem(error :: RatchetError)
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

fn opened(state :: consume RatchetState, value :: Bytes, expected :: Bytes) do
  if Bytes.secure_equals(value, expected) do
    println("in-order:ok")
  else
    println("in-order:wrong-plaintext")
  end
end

fn rejected(state :: consume RatchetState, error :: RatchetError) do
  println("in-order:rejected")
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
    alice_account,
    policy,
    policy,
    encoded_initial(initial) ?
  ) do
    Err(error) -> Err(SessionProblem(error))
    Ok(value) -> Ok(value)
  end ?
  let plaintext = Bytes.from_utf8("ratcheted hello")
  let associated_data = Bytes.from_utf8("conversation-1")
  let (_alice_session, message) = case encrypt(
    alice_session,
    plaintext,
    associated_data
  ) do
    Err(error) -> Err(RatchetProblem(error))
    Ok(value) -> Ok(value)
  end ?
  case decrypt(bob_session, message, associated_data) do
    Opened(state, value) -> opened(state, value, plaintext)
    Rejected(state, error) -> rejected(state, error)
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
    let run = Command::new(project.join("messenger-ratchet-proof"))
        .output()
        .expect("run ratchet proof");
    assert!(
        run.status.success(),
        "ratchet proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stderr.is_empty(), "unexpected stderr: {:?}", run.stderr);
    assert_eq!(String::from_utf8_lossy(&run.stdout), "in-order:ok\n");
}
