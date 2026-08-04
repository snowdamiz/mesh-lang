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
