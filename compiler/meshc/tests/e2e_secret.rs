#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn write_project(root: &Path, name: &str, source: &str) -> PathBuf {
    let project = root.join(name);
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("mesh.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    fs::write(project.join("main.mpl"), source).unwrap();
    project
}

fn build(project: &Path) -> Output {
    Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap()
}

#[test]
fn secret_random_is_typed_and_destroyable_without_revelation() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "secret-proof",
        r#"
fn proof() -> Int ! CryptoError do
  case Secret.random(-1) do
    Err(_) -> println("invalid")
    Ok(secret) -> do
      Secret.destroy(secret)
      println("unexpected")
    end
  end
  let secret = Secret.random(32) ?
  Secret.destroy(secret)
  println("ok")
  Ok(0)
end

fn main() do
  case proof() do
    Err(_) -> println("failed")
    Ok(_) -> nil
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("secret-proof")).output().unwrap();
    assert!(
        run.status.success(),
        "secret proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "invalid\nok\n");
}

/// Two secrets join into one as long as both, which is only observed
/// through what takes a secret of an exact length; a join longer than a
/// secret may be is refused.
#[test]
fn secret_concat_joins_two_secrets_into_one() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "secret-concat",
        r#"
fn proof() -> Int ! CryptoError do
  let joined = Secret.concat(Secret.random(16) ?, Secret.random(16) ?) ?
  let key = Crypto.aead_key(joined) ?
  println("thirty_two:key")
  let short = Secret.concat(Secret.random(16) ?, Secret.random(8) ?) ?
  case Crypto.aead_key(short) do
    Err(InvalidKey) -> println("twenty_four:invalid_key")
    Err(_) -> println("twenty_four:other")
    Ok(unexpected) -> println("twenty_four:unexpected")
  end
  case Secret.concat(Secret.random(65536) ?, Secret.random(1) ?) do
    Err(InvalidLength(maximum, actual)) -> println("too_long:#{maximum},#{actual}")
    Ok(unexpected) -> println("too_long:unexpected")
    Err(_) -> println("too_long:other")
  end
  Ok(0)
end

fn main() do
  case proof() do
    Err(_) -> println("failed")
    Ok(_) -> nil
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("secret-concat"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "thirty_two:key\ntwenty_four:invalid_key\ntoo_long:65536,65537\n"
    );
}

/// A parameter written as a pattern owns the resources it binds, like a
/// `case` arm, and one annotated `borrow` lends them back to the caller.
/// Clauses and arities of one name take resources like any function.
#[test]
fn resources_bind_through_parameter_patterns() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "secret-params",
        r#"
fn tag((key, message)) -> Int ! CryptoError do
  let mac = Crypto.hmac_sha256(key, message) ?
  println("tagged")
  Ok(0)
end

fn peek((key, n) :: borrow (SecretBytes, Int)) -> Int ! CryptoError do
  let mac = Crypto.hmac_sha256(key, Bytes.from_utf8("peek")) ?
  Ok(n)
end

fn open(Ok(key) :: Result<SecretBytes, CryptoError>) -> String = "opened"
fn open(Err(_) :: Result<SecretBytes, CryptoError>) = "failed"

# One name at two arities: the two-argument one borrows.
fn lend(key :: borrow SecretBytes, n :: Int) -> Int ! CryptoError do
  let mac = Crypto.hmac_sha256(key, Bytes.from_utf8("lend")) ?
  Ok(n)
end
fn lend(key :: SecretBytes) -> Int = 0

fn left_in_arm() -> Int do
  case Secret.random(1) do
    Ok(secret) -> 1
    Err(_) -> 0
  end
end

fn skipped_in_arm() -> Int do
  case Secret.random(1) do
    Ok(_) -> 1
    Err(_) -> 0
  end
end

fn skipped_in_let(pair :: (SecretBytes, Int)) -> Int do
  let (_, n) = pair
  n
end

fn skipped_in_clause((_, n) :: (SecretBytes, Int)) -> Int = n

fn skip_all(n :: Int) -> Int ! CryptoError do
  let a = skipped_in_arm()
  let b = skipped_in_let((Secret.random(1) ?, 1))
  let c = skipped_in_clause((Secret.random(1) ?, 1))
  Ok(a + b + c + n)
end

# More secrets than a process may hold at once: each must be destroyed
# where its arm, `let` or clause ends, bound by name or by `_`.
fn churn(0) do nil end
fn churn(count :: Int) do
  left_in_arm()
  open(Secret.random(1))
  case skip_all(0) do
    Ok(_) -> nil
    Err(_) -> println("skip failed")
  end
  churn(count - 1)
end

fn proof() -> Int ! CryptoError do
  tag((Secret.random(32) ?, Bytes.from_utf8("message"))) ?
  let pair = (Secret.random(32) ?, 7)
  println("peeked:#{peek(pair) ?}")
  let (key, n) = pair
  Secret.destroy(key)
  println(open(Secret.random(16)))
  println(open(Secret.random(0)))
  churn(4200)
  println(open(Secret.random(16)))
  let lent = Secret.random(32) ?
  let first = lend(lent, 2) ?
  let piped = (lent |> lend(3)) ?
  let mac = Crypto.hmac_sha256(lent, Bytes.from_utf8("after")) ?
  println("lent:#{first + piped + (lent |> lend)}")
  Ok(n)
end

fn main() do
  case proof() do
    Ok(n) -> println("done:#{n}")
    Err(_) -> println("failed")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("secret-params"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "tagged\npeeked:7\nopened\nfailed\nopened\nlent:5\ndone:7\n"
    );
}

#[test]
fn secret_map_keeps_bounded_keys_affine_across_the_native_abi() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "secret-map-proof",
        r#"
fn proof() -> Int ! CryptoError do
  let committed = SecretMap.new(2) ?
  let first_key = Bytes.from_utf8("first")
  let first = Secret.random(32) ?
  SecretMap.insert(committed, first_key, first) ?
  let copied = SecretMap.copy(committed, first_key) ?
  Secret.destroy(copied)

  let forked = SecretMap.fork(committed) ?
  SecretMap.delete(forked, first_key) ?
  if !SecretMap.contains(committed, first_key) || SecretMap.contains(forked, first_key) do
    println("fork-aliased")
  end

  let candidate = SecretMap.new(1) ?
  let second_key = Bytes.from_utf8("second")
  let second = Secret.random(32) ?
  SecretMap.insert(candidate, second_key, second) ?
  SecretMap.merge(committed, candidate) ?
  if SecretMap.contains(committed, second_key) do
    SecretMap.delete(committed, first_key) ?
    if SecretMap.contains(committed, first_key) do
      println("delete-failed")
    else
      println("secret-map-ok")
    end
  else
    println("merge-failed")
  end
  Ok(0)
end

fn main() do
  case proof() do
    Ok(_) -> nil
    Err(_) -> println("secret-map-error")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("secret-map-proof"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "secret map proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "secret-map-ok\n");
}

/// Each SecretMap request refused for what is wrong with it: a capacity
/// out of range, a key too short or long, taken or missing, a full map, an
/// entry too big to encode, and a merge of a key both maps hold. A missing
/// key is no error to `delete`, and no match for `contains`.
#[test]
fn secret_map_refuses_each_request_for_what_is_wrong_with_it() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "secret-map-refusals",
        r#"
fn name(error :: CryptoError) -> String do
  case error do
    InvalidKey -> "InvalidKey"
    ResourceLimitExceeded -> "ResourceLimitExceeded"
    InvalidLength(_, _) -> "InvalidLength"
    _ -> "other"
  end
end

fn show(label :: String, result :: Result<Unit, CryptoError>) do
  case result do
    Ok(_) -> println(label <> ":ok")
    Err(error) -> println(label <> ":" <> name(error))
  end
end

fn letters(length :: Int) -> String do
  if length == 0 do
    ""
  else
    "k" <> letters(length - 1)
  end
end

fn key(length :: Int) -> Bytes do
  Bytes.from_utf8(letters(length))
end

fn proof() -> Int ! CryptoError do
  case SecretMap.new(0) do
    Ok(_) -> println("capacity_zero:ok")
    Err(error) -> println("capacity_zero:" <> name(error))
  end
  case SecretMap.new(65) do
    Ok(_) -> println("capacity_65:ok")
    Err(error) -> println("capacity_65:" <> name(error))
  end
  let map = SecretMap.new(2) ?
  show("insert_empty_key", SecretMap.insert(map, key(0), Secret.random(8) ?))
  show("insert_long_key", SecretMap.insert(map, key(129), Secret.random(8) ?))
  show("insert", SecretMap.insert(map, key(1), Secret.random(8) ?))
  show("insert_taken_key", SecretMap.insert(map, key(1), Secret.random(8) ?))
  show("insert_too_big", SecretMap.insert(map, key(2), Secret.random(65530) ?))
  show("insert_second", SecretMap.insert(map, key(2), Secret.random(8) ?))
  show("insert_full", SecretMap.insert(map, key(3), Secret.random(8) ?))
  println("contains_invalid:#{SecretMap.contains(map, key(0))}")
  println("contains_missing:#{SecretMap.contains(map, key(3))}")
  case SecretMap.copy(map, key(3)) do
    Ok(copied) -> println("copy_missing:ok")
    Err(error) -> println("copy_missing:" <> name(error))
  end
  show("delete_missing", SecretMap.delete(map, key(3)))
  show("delete_invalid", SecretMap.delete(map, key(0)))
  let other = SecretMap.new(1) ?
  SecretMap.insert(other, key(2), Secret.random(8) ?) ?
  show("merge_taken_key", SecretMap.merge(map, other))
  Ok(0)
end

fn main() do
  case proof() do
    Ok(_) -> println("done")
    Err(error) -> println("error:" <> name(error))
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("secret-map-refusals"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "capacity_zero:ResourceLimitExceeded\ncapacity_65:ResourceLimitExceeded\ninsert_empty_key:InvalidKey\ninsert_long_key:InvalidKey\ninsert:ok\ninsert_taken_key:InvalidKey\ninsert_too_big:ResourceLimitExceeded\ninsert_second:ok\ninsert_full:ResourceLimitExceeded\ncontains_invalid:false\ncontains_missing:false\ncopy_missing:InvalidKey\ndelete_missing:ok\ndelete_invalid:InvalidKey\nmerge_taken_key:InvalidKey\ndone\n"
    );
}

#[test]
fn secret_values_are_rejected_at_public_data_boundaries() {
    let cases = [
        (
            "secret-interpolation",
            r##"fn misuse(secret :: SecretBytes) do println("#{secret}") end
fn main() do nil end"##,
        ),
        (
            "secret-json",
            r#"fn misuse(secret :: SecretBytes) do Json.encode(secret) end
fn main() do nil end"#,
        ),
        (
            "secret-equality",
            r#"fn misuse(secret :: SecretBytes) -> Bool do secret == secret end
fn main() do nil end"#,
        ),
        (
            "secret-send",
            r#"fn misuse(pid :: Pid, secret :: SecretBytes) do send(pid, secret) end
fn main() do nil end"#,
        ),
        (
            "secret-list",
            r#"fn misuse(secret :: SecretBytes) do
  let values = [secret]
  List.length(values)
end
fn main() do nil end"#,
        ),
        (
            "secret-struct",
            r##"struct Leaky do
  secret :: SecretBytes
end
fn misuse(secret :: SecretBytes) do
  let leaky = Leaky { secret: secret }
  println("#{leaky}")
end
fn main() do nil end"##,
        ),
    ];

    for (name, source) in cases {
        let temp = tempfile::tempdir().unwrap();
        let project = write_project(temp.path(), name, source);
        let output = build(&project);
        assert!(
            !output.status.success(),
            "{name} unexpectedly compiled; SecretBytes crossed a public boundary"
        );
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        assert!(
            stderr.contains("secret") || stderr.contains("resource"),
            "{name} failed without a secret/resource diagnostic:\n{stderr}"
        );
    }
}

#[test]
fn aggregate_result_scope_cleanup_releases_live_secret_resources() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "secret-result-cleanup",
        r#"
fn allocate_and_drop() do
  let pending = Secret.random(1)
  nil
end

fn churn(0) do nil end
fn churn(count :: Int) do
  allocate_and_drop()
  churn(count - 1)
end

fn main() do
  churn(4200)
  case Secret.random(1) do
    Ok(secret) -> do
      println("clean")
      Secret.destroy(secret)
    end
    Err(_) -> println("leaked")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("secret-result-cleanup"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "secret cleanup proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "clean\n");
}

#[test]
fn crypto_nominal_public_structs_cross_the_native_pointer_abi() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "crypto-nominal-abi",
        r#"
fn exercise_crypto() -> Int ! CryptoError do
  let alice = Crypto.x25519_generate() ?
  let bob = Crypto.x25519_generate() ?
  let alice_shared = Crypto.x25519_shared(alice.private_key, bob.public_key) ?
  let bob_shared = Crypto.x25519_shared(bob.private_key, alice.public_key) ?

  let signer = Crypto.signing_generate() ?
  let message = Bytes.from_utf8("nominal ABI")
  let signature = Crypto.sign(signer.private_key, message) ?
  let valid = Crypto.verify(signer.public_key, message, signature) ?

  Secret.destroy(alice_shared)
  Secret.destroy(bob_shared)
  if valid do println("crypto-ok") else println("verify-failed") end
  Ok(0)
end

fn main() do
  case exercise_crypto() do
    Ok(_) -> nil
    Err(_) -> println("crypto-error")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("crypto-nominal-abi"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "native crypto ABI proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "crypto-ok\n");
}

#[test]
fn signing_seed_constructor_is_deterministic() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "crypto-signing-seed",
        r#"
fn proof() -> Bool ! CryptoError do
  let seed = Bytes.from_utf8("0123456789abcdef0123456789abcdef")
  let first = Crypto.signing_from_seed(seed) ?
  let second = Crypto.signing_from_seed(seed) ?
  let message = Bytes.from_utf8("stable checkpoint")
  let signature = Crypto.sign(first.private_key, message) ?
  Crypto.verify(second.public_key, message, signature)
end

fn main() do
  case proof() do
    Ok(true) -> println("seed-ok")
    _ -> println("seed-error")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("crypto-signing-seed"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "seeded signing proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "seed-ok\n");
}

#[test]
fn secret_hex_environment_values_feed_private_key_constructors_without_mesh_values() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "crypto-secret-env",
        r#"
fn proof() -> Bool ! CryptoError do
  let signer = Crypto.signing_from_secret(Env.get_secret_hex("MESH_TEST_SIGNING_SEED_HEX") ?) ?
  let message = Bytes.from_utf8("secret environment boundary")
  let signature = Crypto.sign(signer.private_key, message) ?
  let signature_valid = Crypto.verify(signer.public_key, message, signature) ?

  let first_mlkem = Crypto.mlkem_from_secret(Env.get_secret_hex("MESH_TEST_MLKEM_SEED_HEX") ?) ?
  let second_mlkem = Crypto.mlkem_from_secret(Env.get_secret_hex("MESH_TEST_MLKEM_SEED_HEX") ?) ?
  let malformed_rejected = case Env.get_secret_hex("MESH_TEST_INVALID_SEED_HEX") do
    Err(InvalidKey) -> true
    Ok(secret) -> do
      Secret.destroy(secret)
      false
    end
    Err(_) -> false
  end
  let missing_rejected = case Env.get_secret_hex("MESH_TEST_MISSING_SEED_HEX") do
    Err(InvalidKey) -> true
    Ok(secret) -> do
      Secret.destroy(secret)
      false
    end
    Err(_) -> false
  end

  Ok(
    signature_valid and
      Bytes.to_hex(signer.public_key.bytes) == "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a" and
      Bytes.secure_equals(first_mlkem.public_key.bytes, second_mlkem.public_key.bytes) and
      malformed_rejected and
      missing_rejected
  )
end

fn main() do
  case proof() do
    Ok(true) -> println("secret-env-ok")
    _ -> println("secret-env-error")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("crypto-secret-env"))
        .env(
            "MESH_TEST_SIGNING_SEED_HEX",
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        )
        .env(
            "MESH_TEST_MLKEM_SEED_HEX",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
        )
        .env("MESH_TEST_INVALID_SEED_HEX", "not-hex")
        .env_remove("MESH_TEST_MISSING_SEED_HEX")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "secret environment proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "secret-env-ok\n");
}

#[test]
fn x25519_seed_constructor_matches_rfc7748() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "crypto-x25519-seed",
        r#"
fn proof() -> Bool ! String do
  let seed = Bytes.from_hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a") ?
  let pair = case Crypto.x25519_from_seed(seed) do
    Err(_) -> Err("x25519 failed")
    Ok(value) -> Ok(value)
  end ?
  Ok(Bytes.to_hex(pair.public_key.bytes) == "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
end

fn main() do
  case proof() do
    Ok(true) -> println("seed-ok")
    _ -> println("seed-error")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("crypto-x25519-seed"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "seeded X25519 proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "seed-ok\n");
}

#[test]
fn result_tuple_destructuring_binds_and_consumes_resource_elements() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "secret-result-tuple",
        r#"
fn allocate() -> Result < (SecretBytes, Int), CryptoError > do
  let secret = Secret.random(1) ?
  Ok((secret, 42))
end

fn proof() -> Int ! CryptoError do
  let (secret, value) = allocate() ?
  Secret.destroy(secret)
  Ok(value)
end

fn main() do
  case proof() do
    Ok(value) -> println("${value}")
    Err(_) -> println("failed")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("secret-result-tuple"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "secret tuple proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "42\n");
}

#[test]
fn moved_nested_resource_sum_payload_cleanup_does_not_dereference_null() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "nested-resource-cleanup",
        r#"
pub resource struct Boxed do
  key :: SecretBytes
end
pub type Keys do
  Pair(first :: SecretBytes, second :: SecretBytes)
end
fn update(base :: consume Boxed, keys :: consume Keys) -> Boxed do
  case keys do
    Pair(first, second) -> do
      Secret.destroy(second)
      %{base | key: first}
    end
  end
end
fn finish(base :: consume Boxed, keys :: Option<Keys>) -> Boxed do
  let material = base
  let material = case keys do
    None -> material
    Some(value) -> update(material, value)
  end
  material
end
fn run() -> Int ! CryptoError do
  let base = Boxed {key: Secret.random(32) ?}
  let keys = Pair(Secret.random(32) ?, Secret.random(32) ?)
  let next = finish(base, Some(keys))
  let derived = Crypto.hkdf_sha256(next.key, Bytes.empty(), Bytes.from_utf8("check"), 32) ?
  Secret.destroy(derived)
  Ok(1)
end
fn main() do
  case run() do
    Ok(value) -> println(Int.to_string(value))
    Err(_) -> println("error")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("nested-resource-cleanup"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "nested resource cleanup crashed: {:?}",
        run.status
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "1\n");
}

#[test]
fn conditional_resource_tuple_preserves_runtime_layout_and_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let project = write_project(
        temp.path(),
        "conditional-resource-tuple",
        r#"
resource struct Owned do
  key :: SecretBytes
  number :: Int
end
fn choose(reverse :: Bool) -> Int ! CryptoError do
  let one = Owned { key : Secret.random(16) ?, number : 1 }
  let two = Owned { key : Secret.random(32) ?, number : 2 }
  let (left, right) = if reverse do (two, one) else (one, two) end
  let result = left.number * 10 + right.number
  Ok(result)
end
fn main() do
  case choose(false) do
    Ok(value) -> println(Int.to_string(value))
    Err(_) -> println("failed")
  end
  case choose(true) do
    Ok(value) -> println(Int.to_string(value))
    Err(_) -> println("failed")
  end
end
"#,
    );
    let output = build(&project);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("conditional-resource-tuple"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "12\n21\n");
}
