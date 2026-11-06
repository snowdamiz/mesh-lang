//! Result tag review changes source policy, not runtime values or ownership.
use std::{fs, path::PathBuf, process::Command};
#[path = "support/test_artifacts.rs"]
mod artifacts;

fn compiler() -> PathBuf {
    std::env::var_os("MESH_PLAINTEXT_TEST_COMPILER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(artifacts::meshc_bin)
}

#[test]
fn reviewed_tags_preserve_ciphertext_resources_and_evaluate_once_at_both_optimizations() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path();
    fs::write(project.join("cipher.mpl"), r#"
pub fn seal(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) -> Bytes do
  let reviewed = declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"test-only imported seal admission")
  Result.unwrap_or(reviewed,Bytes.empty())
end
"#).unwrap();
    fs::write(project.join("review.mpl"), r#"
pub fn declassify_discriminant(value :: Result<Int,String>,reason :: String) -> Result<Int,String> do
  case value do Ok(number) -> Ok(number + 200) Err(error) -> Err(error) end
end
"#).unwrap();
    fs::write(project.join("main.mpl"), r#"
import Review
import Cipher
fn tick() -> Result<Int,String> do println("once")
  Ok(7)
end
fn local_identity_tests() do
  println(Int.to_string(Result.unwrap_or(declassify_discriminant(tick(),"test-only direct tag"),0)))
  println(Int.to_string(Result.unwrap_or(tick() |> declassify_discriminant("test-only pipe tag"),0)))
  println(Int.to_string(Result.unwrap_or("test-only slot two tag" |2> declassify_discriminant(tick()),0)))
end
fn local_lookalike(value :: Result<Int,String>) -> Result<Int,String> do
  let declassify_discriminant = fn(input,reason) ->
    case input do Ok(number) -> Ok(number + 100) Err(error) -> Err(error) end
  end
  declassify_discriminant(value,"ordinary closure")
end
fn keep_secret(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do
  value |> declassify_discriminant("test-only affine Result tag")
end
fn keep_error_secret(value :: Result<Int,SecretBytes>) -> Result<Int,SecretBytes> do
  "test-only affine error tag" |2> declassify_discriminant(value)
end
fn cipher_direct(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) -> Bytes do
  Result.unwrap_or(declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"test-only direct seal tag"),Bytes.empty())
end
fn cipher_pipe(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) -> Bytes do
  Result.unwrap_or(Crypto.aead_seal(key,nonce,Bytes.empty(),body) |> declassify_discriminant("test-only pipe seal tag"),Bytes.empty())
end
fn crypto_proof() -> Bool!CryptoError do
  let original = Secret.random(32)?
  let wrapped :: Result<SecretBytes,String> = Ok(original)
  let restored = case keep_secret(wrapped) do
    Ok(secret) -> secret
    Err(_) -> return Err(InvalidKey)
  end
  let derived = Crypto.hkdf_sha256(restored,Bytes.empty(),Bytes.from_utf8("ownership"),32)?
  Secret.destroy(restored)
  let error_secret = Secret.random(32)?
  let restored_error = case keep_error_secret(Err(error_secret)) do
    Err(secret) -> secret
    Ok(_) -> return Err(InvalidKey)
  end
  let tag = Crypto.hmac_sha256(restored_error,Bytes.empty())?
  Secret.destroy(tag)
  Secret.destroy(restored_error)
  let key = Crypto.aead_key(derived)?
  let nonce = Bytes.from_utf8("123456789012")
  let body = Bytes.from_utf8("private message body")
  let first = Crypto.aead_open(key,nonce,Bytes.empty(),cipher_direct(body,key,nonce))?
  let second = Crypto.aead_open(key,nonce,Bytes.empty(),cipher_pipe(body,key,nonce))?
  let third = Crypto.aead_open(key,nonce,Bytes.empty(),Cipher.seal(body,key,nonce))?
  Ok(Bytes.secure_equals(first,body) && Bytes.secure_equals(second,body) && Bytes.secure_equals(third,body))
end
fn unused(value :: Result<Int,String>) do
  declassify_discriminant(value,"unused reviewed tag")
end
fn main() do
  local_identity_tests()
  println(Int.to_string(Result.unwrap_or(local_lookalike(Ok(1)),0)))
  println(Int.to_string(Result.unwrap_or(Review.declassify_discriminant(Ok(1),"ordinary imported call"),0)))
  case crypto_proof() do Ok(true) -> println("ciphertext-and-resource-ok") _ -> panic("crypto proof failed") end
  if false do declassify_discriminant(Ok(1),"unreachable reviewed tag") end
end
"#).unwrap();
    let mut previous_report = None;
    for level in ["0", "2"] {
        let binary = project.join(format!("proof-{level}"));
        let report = project.join(format!("report-{level}.json"));
        let result = Command::new(compiler())
            .args(["build", "--opt-level", level, "--plaintext-report"])
            .arg(&report)
            .arg(project)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        let execution = Command::new(&binary).output().unwrap();
        assert!(
            execution.status.success(),
            "{}",
            artifacts::command_output_text(&execution)
        );
        assert_eq!(
            String::from_utf8_lossy(&execution.stdout).trim(),
            "once\n7\nonce\n7\nonce\n7\n101\n201\nciphertext-and-resource-ok"
        );
        let bytes = fs::read(report).unwrap();
        if let Some(previous) = &previous_report {
            assert_eq!(previous, &bytes);
        }
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let sites = parsed["modules"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|module| module["sites"].as_array().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(sites.len(), 10);
        assert!(sites
            .iter()
            .all(|site| site["kind"] == "declassify_discriminant"));
        assert!(!sites
            .iter()
            .any(|site| site["reason"].as_str().unwrap().starts_with("ordinary")));
        previous_report = Some(bytes);
    }
}

#[test]
fn compiled_payload_control_and_metadata_leaks_have_no_artifact() {
    for source in [
        r#"fn leak(value :: Result<Plaintext<Bytes>,String>) do Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(declassify_discriminant(value,"tag only"),Bytes.empty())) end"#,
        r#"fn leak(value :: Result<Int,Plaintext<String>>) do case declassify_discriminant(value,"tag only") do Err(error) -> println(error) Ok(_) -> nil end end"#,
        r#"fn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do case declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"tag only") do Err(InvalidLength(_,actual)) -> println(Int.to_string(actual)) _ -> nil end end"#,
        r#"fn leak(choice :: Plaintext<Bool>,value :: Result<Int,String>) do if choice do println(String.from(Result.is_ok(declassify_discriminant(value,"tag only")))) end end"#,
        r#"fn leak(choice :: Plaintext<Bool>,first :: Result<Bytes,String>,second :: Result<Bytes,String>) do let selected = if choice do first else second end
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(declassify_discriminant(selected,"tag only"),Bytes.empty())) end"#,
        r#"fn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Plaintext<Bytes>) do Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"tag only"),Bytes.empty())) end"#,
        r#"fn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes,aad :: Plaintext<Bytes>) do Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(declassify_discriminant(Crypto.aead_seal(key,nonce,aad,body),"tag only"),Bytes.empty())) end"#,
        r#"fn leak(value :: Plaintext<Result<Int,String>>) do println(String.from(Result.is_ok(declassify_discriminant(value,"tag only")))) end"#,
        r#"fn leak(secret :: Plaintext<Bool>,first :: Result<Int,String>,second :: Result<Int,String>) do let selected = if secret do first else second end
println(String.from(Result.is_ok(declassify_discriminant(selected,"tag only")))) end"#,
        r#"fn review(value :: Result<Int,String>) -> Result<Int,String> do declassify_discriminant(value,"local tag only") end
fn leak(secret :: Plaintext<Bool>,first :: Result<Int,String>,second :: Result<Int,String>) do let selected = if secret do first else second end
println(String.from(Result.is_ok(review(selected)))) end"#,
        r#"import Foreign
struct Holder do value :: Result<Int,String> end
fn leak(secret :: Plaintext<Bool>,first :: Result<Int,String>,second :: Result<Int,String>) do let selected = if secret do first else second end
println(String.from(Result.is_ok(Foreign.review(Holder { value: selected }.value)))) end"#,
        r#"fn leak(value :: Result<Int,String>) do let review = declassify_discriminant
review(value,"indirect review") end"#,
    ] {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join("foreign.mpl"), "pub fn review(value :: Result<Int,String>) -> Result<Int,String> do declassify_discriminant(value,\"imported tag only\") end").unwrap();
        fs::write(
            temporary.path().join("main.mpl"),
            format!("{source}\nfn main() do nil end"),
        )
        .unwrap();
        let binary = temporary.path().join("blocked");
        let report = temporary.path().join("blocked.json");
        let result = Command::new(compiler())
            .args(["build", "--json", "--plaintext-report"])
            .arg(&report)
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(!result.status.success(), "escaped: {source}");
        assert!(!binary.exists());
        assert!(!report.exists());
        let diagnostics = String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .collect::<Vec<_>>();
        assert!(
            !diagnostics.is_empty(),
            "{}",
            artifacts::command_output_text(&result)
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == "E0093")
                && diagnostics.iter().all(
                    |diagnostic| diagnostic["code"] == "E0093" || diagnostic["code"] == "C0001"
                ),
            "{source}\n{}",
            artifacts::command_output_text(&result)
        );
    }
}

#[test]
fn a_named_user_function_is_not_compiled_as_the_review_builtin() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("main.mpl"), r#"
fn declassify_discriminant(value :: Result<Int,String>,reason :: String) -> Result<Int,String> do
  case value do Ok(number) -> Ok(number + 9) Err(error) -> Err(error) end
end
fn main() do println(Int.to_string(Result.unwrap_or(Ok(1) |> declassify_discriminant("ordinary user call"),0))) end
"#).unwrap();
    for level in ["0", "2"] {
        let binary = temporary.path().join(format!("proof-{level}"));
        let result = Command::new(compiler())
            .args(["build", "--opt-level", level])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        let execution = Command::new(&binary).output().unwrap();
        assert!(
            execution.status.success(),
            "{}",
            artifacts::command_output_text(&execution)
        );
        assert_eq!(String::from_utf8_lossy(&execution.stdout).trim(), "10");
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(binary.with_extension("plaintext.json")).unwrap())
                .unwrap();
        assert!(report["modules"][0]["sites"].as_array().unwrap().is_empty());
    }
}

#[test]
fn qualified_module_functions_are_not_replaced_by_same_named_local_closures() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("review.mpl"), r#"
pub fn identity(value :: Int) -> Int do value + 90 end
pub fn declassify_discriminant(value :: Result<Int,String>,reason :: String) -> Result<Int,String> do
  case value do Ok(number) -> Ok(number + 90) Err(error) -> Err(error) end
end
pub fn borrowed(key :: borrow SecretBytes) -> Int!CryptoError do
  let tag = Crypto.hmac_sha256(key,Bytes.empty())?
  Secret.destroy(tag)
  Ok(91)
end
"#).unwrap();
    fs::write(temporary.path().join("main.mpl"), r#"
import Review
fn apply(callback, value :: Int) -> Int do callback(value) end
fn borrowed_proof() -> Bool!CryptoError do
  let borrowed = fn(value) -> value + 80 end
  let key = Secret.random(32)?
  let answer = Review.borrowed(key)?
  let tag = Crypto.hmac_sha256(key,Bytes.empty())?
  Secret.destroy(tag)
  Secret.destroy(key)
  Ok(answer == 91 && borrowed(1) == 81)
end
fn main() do
  let identity = fn(value) -> value + 80 end
  println(Int.to_string(identity(1)))
  println(Int.to_string(Review.identity(1)))
  println(Int.to_string(1 |> Review.identity()))
  let stored = Review.identity
  println(Int.to_string(stored(1)))
  println(Int.to_string(apply(Review.identity,1)))
  println(Int.to_string(apply(identity,1)))
  let declassify_discriminant = fn(value,reason) ->
    case value do Ok(number) -> Ok(number + 80) Err(error) -> Err(error) end
  end
  println(Int.to_string(Result.unwrap_or(declassify_discriminant(Ok(1),"ordinary local"),0)))
  println(Int.to_string(Result.unwrap_or(Review.declassify_discriminant(Ok(1),"ordinary qualified"),0)))
  let stored_review = Review.declassify_discriminant
  println(Int.to_string(Result.unwrap_or(stored_review(Ok(1),"ordinary stored"),0)))
  case borrowed_proof() do Ok(true) -> println("borrowed-function-ok") _ -> panic("borrow mode lost") end
end
"#).unwrap();
    for level in ["0", "2"] {
        let binary = temporary.path().join(format!("proof-{level}"));
        let result = Command::new(compiler())
            .args(["build", "--opt-level", level])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        let execution = Command::new(&binary).output().unwrap();
        assert!(
            execution.status.success(),
            "{}",
            artifacts::command_output_text(&execution)
        );
        assert_eq!(
            String::from_utf8_lossy(&execution.stdout).trim(),
            "81\n91\n91\n91\n91\n81\n81\n91\n91\nborrowed-function-ok"
        );
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(binary.with_extension("plaintext.json")).unwrap())
                .unwrap();
        assert!(report["modules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|module| module["sites"].as_array().unwrap().is_empty()));
    }
}
