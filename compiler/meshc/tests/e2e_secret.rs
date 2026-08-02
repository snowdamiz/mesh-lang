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
