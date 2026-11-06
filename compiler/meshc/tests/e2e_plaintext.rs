//! The public source API and the disclosure report survive real code generation.
use std::{fs, path::PathBuf, process::Command};
#[path = "support/test_artifacts.rs"]
mod artifacts;

fn compiler() -> PathBuf {
    std::env::var_os("MESH_PLAINTEXT_TEST_COMPILER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(artifacts::meshc_bin)
}

#[test]
fn compiled_labels_are_erased_and_audit_all_source_sites_before_dead_code_elimination() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path();
    // The executed framing fixture is public. Unknown private-size seal failures
    // are rejected by the dedicated error regression below.
    fs::write(
        project.join("main.mpl"),
        r#"
struct Message do
  body :: Plaintext<String>
end
fn unused(value :: Plaintext<String>) do
  declassify(value, "unused reviewed source site")
end
@display("chat text displayed by the native host")
@export("plaintext_display")
pub fn display(value :: Plaintext<Bytes>) -> Result<Bytes, String> do Ok(value) end
fn reveal(value :: Plaintext<String>) do
  println(value |> declassify("user enabled preview"))
end
fn frame(value :: Bytes) -> Bytes do
  let builder = case BytesBuilder.new(128) do
    Err(_) -> return Bytes.empty()
    Ok(value) -> value
  end
  let _ = BytesBuilder.write_bytes(builder,value)
  case BytesBuilder.finish(builder) do
    Err(_) -> Bytes.empty()
    Ok(bytes) -> bytes
  end
end
fn sealed_frame() -> Bool!CryptoError do
  let key = Crypto.aead_key(Secret.random(32)?)?
  let nonce = Bytes.from_utf8("123456789012")
  let ciphertext = Crypto.aead_seal(key,nonce,Bytes.empty(),frame(Bytes.from_utf8("body")))?
  let opened = Crypto.aead_open(key,nonce,Bytes.empty(),ciphertext)?
  Ok(Bytes.secure_equals(opened,Bytes.from_utf8("body")))
end
fn main() do
  let message = Message { body: "reviewed preview" }
  reveal(message.body)
  println("explicit slot preview reason" |2> declassify(message.body))
  case sealed_frame() do
    Ok(true) -> println("framed-seal-ok")
    _ -> panic("framed seal failed")
  end
  if false do declassify("fixed", "unreachable reviewed source site") end
end
"#,
    )
    .unwrap();
    let mut previous = None;
    for level in ["0", "2"] {
        let binary = project.join(format!("proof-{level}"));
        let report = project.join(format!("audit-{level}.json"));
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
            "reviewed preview\nreviewed preview\nframed-seal-ok"
        );
        let bytes = fs::read(report).unwrap();
        if let Some(old) = &previous {
            assert_eq!(
                old, &bytes,
                "optimization must not prune or reorder audit sites"
            );
        }
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["policy"], "plaintext-v1");
        let modules = parsed["modules"].as_array().unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0]["file"], "main.mpl");
        assert_eq!(modules[0]["source_sha256"].as_str().unwrap().len(), 64);
        let sites = modules[0]["sites"].as_array().unwrap();
        assert_eq!(
            sites
                .iter()
                .map(|site| site["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "declassify",
                "display",
                "declassify",
                "declassify",
                "declassify"
            ]
        );
        assert!(sites.iter().all(|site| site["line"].as_u64().unwrap() > 0
            && site["column"].as_u64().unwrap() > 0
            && site["end"].as_u64() > site["start"].as_u64()));
        previous = Some(bytes);
    }
}

#[test]
fn compiled_network_exit_reports_dedicated_json_diagnostic_and_has_no_artifact() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("main.mpl"), "fn leak(value :: Plaintext<String>) do Http.build(:post, value) end\nfn main() do leak(\"private\") end").unwrap();
    let binary = temporary.path().join("proof");
    let result = Command::new(compiler())
        .args(["build", "--json"])
        .arg(temporary.path())
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!binary.exists());
    let diagnostics = String::from_utf8_lossy(&result.stderr);
    assert!(
        diagnostics
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|value| value["code"] == "E0093"),
        "{diagnostics}"
    );
}

#[test]
fn compiled_imported_projection_and_public_result_tags_preserve_sealing() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("messages.mpl"),
        r#"
pub struct Message do
  body :: Plaintext<U64>
  id :: String
end
pub fn forward(value :: Result<List<Message>,String>) -> Result<List<Message>,String> do
  return value
end
pub fn mapped(values :: List<Message>) -> List<Message> do
  [] <> for value in values do value end
end
fn accumulate(values :: List<Message>, index :: Int, output :: List<Message>) -> Result<List<Message>,String> do
  if index >= List.length(values) do
    Ok(output)
  else
    accumulate(values,index + 1,List.append(output,List.get(values,index)))
  end
end
pub fn accumulated(values :: List<Message>) -> Result<List<Message>,String> do
  accumulate(values,0,List.new())
end
"#,
    )
    .unwrap();
    fs::write(temporary.path().join("main.mpl"), r#"
import Messages
fn encrypt(input :: Result<List<Messages.Message>,String>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Result<Bytes,CryptoError>,String> do
  let messages = Messages.accumulated(Messages.mapped(Messages.forward(input)?))?
  let identifiers = [] ++ for message in messages do message.id end
  println(Int.to_string(List.length(messages)))
  for identifier in identifiers do println(identifier) end
  let first = List.get(messages,0)
  let encoded = case Bytes.write_u64_be(first.body) do
    Ok(bytes) -> bytes
    Err(_) -> return Err("integer")
  end
  Ok(Crypto.aead_seal(key,nonce,Bytes.empty(),encoded))
end
fn main() do
  let secret = case Secret.random(32) do Ok(value) -> value Err(_) -> panic("random") end
  let key = case Crypto.aead_key(secret) do Ok(value) -> value Err(_) -> panic("key") end
  let nonce = Bytes.from_utf8("123456789012")
  let word = case U64.parse("18446744073709551615") do Ok(value) -> value Err(_) -> panic("word") end
  let message = Messages.Message {body:word,id:"public-id"}
  let ciphertext = case encrypt(Ok([message]),key,nonce) do
    Ok(Ok(bytes)) -> bytes
    _ -> panic("seal")
  end
  let opened = case Crypto.aead_open(key,nonce,Bytes.empty(),ciphertext) do Ok(bytes) -> bytes Err(_) -> panic("open") end
  let restored = case Bytes.read_u64_be(opened,0) do Ok(value) -> value Err(_) -> panic("read") end
  if U64.to_string(restored) == "18446744073709551615" do println("projection-seal-ok") else panic("mismatch") end
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
        let result = Command::new(&binary).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "1\npublic-id\nprojection-seal-ok"
        );
    }
}

#[test]
fn compiled_full_width_private_u64_encoding_has_public_status_and_seals_exact_bits() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("framing.mpl"),
        "pub fn forward(value :: Bytes) -> Bytes do value end\n",
    )
    .unwrap();
    fs::write(temporary.path().join("main.mpl"), r#"
from Framing import forward
fn seal_word(value :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Result<Bytes,CryptoError>,BytesError> do
  let encoded = forward(Bytes.write_u64_be(value)?)
  println(Int.to_string(Bytes.length(encoded)))
  Ok(Crypto.aead_seal(key,nonce,Bytes.empty(),encoded))
end
fn main() do
  let secret = case Secret.random(32) do Ok(value) -> value Err(_) -> panic("random") end
  let key = case Crypto.aead_key(secret) do Ok(value) -> value Err(_) -> panic("key") end
  for text in ["0","1","4294967296","18446744073709551615"] do
    let nonce = case Crypto.random_bytes(12) do Ok(value) -> value Err(_) -> panic("nonce") end
    let word = case U64.parse(text) do Ok(value) -> value Err(_) -> panic("integer") end
    let ciphertext = case seal_word(word,key,nonce) do
      Ok(Ok(bytes)) -> bytes
      _ -> panic("seal")
    end
    let opened = case Crypto.aead_open(key,nonce,Bytes.empty(),ciphertext) do Ok(bytes) -> bytes Err(_) -> panic("open") end
    let restored = case Bytes.read_u64_be(opened,0) do Ok(value) -> value Err(_) -> panic("read") end
    if U64.to_string(restored) != text do panic("integer mismatch") else nil end
  end
  println("fixed-u64-seal-ok")
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
        let result = Command::new(&binary).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "8\n8\n8\n8\nfixed-u64-seal-ok"
        );
    }
}

#[test]
fn compiled_nested_container_sources_are_refused_before_codegen() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("main.mpl"),
        r#"
fn leaked_map(values :: Map<String,Plaintext<Bytes>>) do
  Http.body_bytes(Http.build(:post,"https://example.test"),Map.get(values,"body"))
end
fn leaked_queue(values :: Queue<Plaintext<Bytes>>) do
  Http.body_bytes(Http.build(:post,"https://example.test"),Queue.peek(values))
end
fn main() do nil end
"#,
    )
    .unwrap();
    let binary = temporary.path().join("must-not-exist");
    let output = Command::new(compiler())
        .args(["build", "--json"])
        .arg(temporary.path())
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!binary.exists());
    let diagnostics = artifacts::command_output_text(&output);
    let entries = diagnostics
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>();
    assert_eq!(
        entries
            .iter()
            .filter(|item| item["code"] == "E0093")
            .count(),
        2,
        "{diagnostics}"
    );
    assert!(
        entries
            .iter()
            .all(|item| item["code"] == "E0093" || item["code"] == "C0001"),
        "{diagnostics}"
    );
}

#[test]
fn compiled_seal_errors_cannot_publish_private_length_or_failure_control() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("main.mpl"),
        r#"
fn log_error(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do
  case Crypto.aead_seal(key,nonce,Bytes.empty(),body) do
    Err(InvalidLength(_,actual)) -> println(Int.to_string(actual))
    _ -> nil
  end
end
fn network_on_failure(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do
  if Result.is_err(Crypto.aead_seal(key,nonce,Bytes.empty(),body)) do
    let _ = Http.body_bytes(Http.build(:post,"https://example.test"),Bytes.from_utf8("failed"))
  end
end
@export("private_seal_error")
pub fn native_error(input :: Bytes) -> Bytes!String do
  let body :: Plaintext<Bytes> = input
  let material = case Secret.random(32) do Ok(value) -> value Err(_) -> return Err("key") end
  let key = case Crypto.aead_key(material) do Ok(value) -> value Err(_) -> return Err("key") end
  case Crypto.aead_seal(key,Bytes.from_utf8("123456789012"),Bytes.empty(),body) do
    Err(InvalidLength(_,actual)) -> Ok(Bytes.from_utf8(Int.to_string(actual)))
    Err(_) -> Err("seal")
    Ok(value) -> Ok(value)
  end
end
fn main() do nil end
"#,
    )
    .unwrap();
    let binary = temporary.path().join("must-not-exist");
    let output = Command::new(compiler())
        .args(["build", "--json"])
        .arg(temporary.path())
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!binary.exists());
    let text = artifacts::command_output_text(&output);
    let diagnostics = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>();
    for sink in ["println", "Http.body_bytes", "native export"] {
        assert!(
            diagnostics.iter().any(|item| item["code"] == "E0093"
                && item["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(sink))),
            "missing {sink}: {text}"
        );
    }
    assert!(
        diagnostics
            .iter()
            .all(|item| item["code"] == "E0093" || item["code"] == "C0001"),
        "{text}"
    );
}

#[test]
fn compiled_genuine_hash_width_is_public_but_digest_bits_require_a_seal() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("main.mpl"), r#"
fn forward(bytes :: Bytes) -> Bytes do bytes end
fn encrypt_digest(value :: Plaintext<Bytes>, wide :: Bool, key :: borrow AeadKey, nonce :: Bytes) -> Bytes!CryptoError do
  let digest = forward(if wide do Crypto.sha512(value) else Crypto.sha256(value) end)
  println(Int.to_string(Bytes.length(digest)))
  let encrypted = Crypto.aead_seal(key,nonce,Bytes.empty(),digest)?
  let _request = Http.body_bytes(Http.build(:post,"https://never-contact.invalid"),encrypted)
  Ok(encrypted)
end
fn main() do
  let material = case Secret.random(32) do Ok(value) -> value Err(_) -> panic("key") end
  let key = case Crypto.aead_key(material) do Ok(value) -> value Err(_) -> panic("key") end
  for count in [0,1,65537,131073] do
    let input = case Bytes.repeat(7,count) do Ok(value) -> value Err(_) -> panic("fixture") end
    for wide in [false,true] do
      let nonce = case Crypto.random_bytes(12) do Ok(value) -> value Err(_) -> panic("nonce") end
      let ciphertext = case encrypt_digest(input,wide,key,nonce) do Ok(value) -> value Err(_) -> panic("encrypt") end
      let opened = case Crypto.aead_open(key,nonce,Bytes.empty(),ciphertext) do Ok(value) -> value Err(_) -> panic("open") end
      let expected = if wide do Crypto.sha512(input) else Crypto.sha256(input) end
      if !Bytes.secure_equals(opened,expected) do panic("digest mismatch") end
    end
  end
  println("fixed-hash-seal-ok")
end
"#).unwrap();
    for level in ["0", "2"] {
        let binary = temporary.path().join(format!("proof-{level}"));
        let output = Command::new(compiler())
            .args(["build", "--opt-level", level])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            artifacts::command_output_text(&output)
        );
        let execution = Command::new(&binary).output().unwrap();
        assert!(
            execution.status.success(),
            "{}",
            artifacts::command_output_text(&execution)
        );
        assert_eq!(
            String::from_utf8_lossy(&execution.stdout).trim(),
            "32\n64\n32\n64\n32\n64\n32\n64\nfixed-hash-seal-ok"
        );
    }
}

#[test]
fn compiled_concat_preserves_unequal_private_words_and_refuses_private_extent() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("main.mpl"), r#"
fn encrypt_pair(left :: Plaintext<U64>, right :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes) -> Bytes!String do
  let a = case Bytes.write_u64_be(left) do Ok(value) -> value Err(_) -> return Err("word") end
  let b = case Bytes.write_u64_be(right) do Ok(value) -> value Err(_) -> return Err("word") end
  let framed = Bytes.concat(a,b)?
  println(Int.to_string(Bytes.length(framed)))
  let encrypted = case Crypto.aead_seal(key,nonce,Bytes.empty(),framed) do Ok(value) -> value Err(_) -> return Err("seal") end
  let _request = Http.body_bytes(Http.build(:post,"https://never-contact.invalid"),encrypted)
  Ok(encrypted)
end
fn main() do
  let material = case Secret.random(32) do Ok(value) -> value Err(_) -> panic("key") end
  let key = case Crypto.aead_key(material) do Ok(value) -> value Err(_) -> panic("key") end
  for pair in [("0","18446744073709551615"),("18446744073709551615","0"),("1","4294967296")] do
    let (left_text,right_text) = pair
    let left = case U64.parse(left_text) do Ok(value) -> value Err(_) -> panic("word") end
    let right = case U64.parse(right_text) do Ok(value) -> value Err(_) -> panic("word") end
    let nonce = case Crypto.random_bytes(12) do Ok(value) -> value Err(_) -> panic("nonce") end
    let encrypted = case encrypt_pair(left,right,key,nonce) do Ok(value) -> value Err(_) -> panic("encrypt") end
    let opened = case Crypto.aead_open(key,nonce,Bytes.empty(),encrypted) do Ok(value) -> value Err(_) -> panic("open") end
    let restored_left = case Bytes.read_u64_be(opened,0) do Ok(value) -> value Err(_) -> panic("read") end
    let restored_right = case Bytes.read_u64_be(opened,8) do Ok(value) -> value Err(_) -> panic("read") end
    if U64.to_string(restored_left) != left_text || U64.to_string(restored_right) != right_text do panic("pair mismatch") end
  end
  println("concat-private-pair-ok")
end
"#).unwrap();
    for level in ["0", "2"] {
        let binary = temporary.path().join(format!("proof-{level}"));
        let output = Command::new(compiler())
            .args(["build", "--opt-level", level])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            artifacts::command_output_text(&output)
        );
        let result = Command::new(&binary).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "16\n16\n16\nconcat-private-pair-ok"
        );
    }
    fs::write(temporary.path().join("main.mpl"), "fn leak(count :: Plaintext<Int>) do let bytes = Result.unwrap_or(Bytes.repeat(1,count),Bytes.empty())\nif Result.is_err(Bytes.concat(bytes,bytes)) do println(\"overflow\") end end\nfn main() do nil end").unwrap();
    let forbidden = temporary.path().join("must-not-exist");
    let output = Command::new(compiler())
        .args(["build", "--json"])
        .arg(temporary.path())
        .arg("-o")
        .arg(&forbidden)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!forbidden.exists());
    let text = artifacts::command_output_text(&output);
    let entries = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>();
    assert!(entries.iter().any(|item| item["code"] == "E0093"), "{text}");
    assert!(
        entries
            .iter()
            .all(|item| item["code"] == "E0093" || item["code"] == "C0001"),
        "{text}"
    );
}

#[test]
fn compiled_slice_checks_public_bounds_without_disclosing_private_bytes() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("main.mpl"), r#"
fn encrypt_slice(value :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes, start :: Int, count :: Int) -> Bytes!String do
  let part = Bytes.slice(Crypto.sha256(value),start,count)?
  println(Int.to_string(Bytes.length(part)))
  let encrypted = case Crypto.aead_seal(key,nonce,Bytes.empty(),part) do
    Ok(output) -> output
    Err(_) -> return Err("seal")
  end
  let _request = Http.body_bytes(Http.build(:post,"https://never-contact.invalid"),encrypted)
  Ok(encrypted)
end
fn public_bounds(value :: Plaintext<Bytes>) -> Bool do
  let digest = Crypto.sha256(value)
  for range in [(-1,0),(0,-1),(33,0),(32,1),(9223372036854775807,9223372036854775807)] do
    let (start,count) = range
    case bytes_slice(digest,start,count) do
      Err(error) -> if error != "byte slice out of bounds" do return false end
      Ok(_) -> return false
    end
  end
  true
end
fn main() do
  let material = case Secret.random(32) do Ok(value) -> value Err(_) -> panic("key") end
  let key = case Crypto.aead_key(material) do Ok(value) -> value Err(_) -> panic("key") end
  for size in [0,1,65537,131073] do
    let input = case Bytes.repeat(19,size) do Ok(value) -> value Err(_) -> panic("input") end
    if !public_bounds(input) do panic("public bounds mismatch") end
    for range in [(0,0),(4,12),(32,0),(0,32)] do
      let (start,count) = range
      let nonce = case Crypto.random_bytes(12) do Ok(value) -> value Err(_) -> panic("nonce") end
      let encrypted = case encrypt_slice(input,key,nonce,start,count) do Ok(value) -> value Err(_) -> panic("seal") end
      let opened = case Crypto.aead_open(key,nonce,Bytes.empty(),encrypted) do Ok(value) -> value Err(_) -> panic("open") end
      let expected = case Bytes.slice(Crypto.sha256(input),start,count) do Ok(value) -> value Err(_) -> panic("expected") end
      if !Bytes.secure_equals(opened,expected) do panic("private slice mismatch") end
    end
  end
  println("private-slice-bounds-ok")
end
"#).unwrap();
    for level in ["0", "2"] {
        let binary = temporary.path().join(format!("proof-{level}"));
        let output = Command::new(compiler())
            .args(["build", "--opt-level", level])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            artifacts::command_output_text(&output)
        );
        let result = Command::new(&binary).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "0\n12\n0\n32\n0\n12\n0\n32\n0\n12\n0\n32\n0\n12\n0\n32\nprivate-slice-bounds-ok"
        );
    }
    fs::write(
        temporary.path().join("main.mpl"),
        r#"
fn private_extent(value :: Plaintext<Bytes>) do
 if Result.is_err(Bytes.slice(value,0,1)) do println("bounds") end
end
fn private_range(count :: Plaintext<Int>) do
 if Result.is_err(Bytes.slice(Bytes.from_utf8("fixed"),0,count)) do println("bounds") end
end
fn private_bytes(value :: Plaintext<Bytes>) do
 let part = Result.unwrap_or(Bytes.slice(Crypto.sha256(value),0,12),Bytes.empty())
 Http.body_bytes(Http.build(:post,"https://never-contact.invalid"),part)
end
fn main() do nil end
"#,
    )
    .unwrap();
    let forbidden = temporary.path().join("must-not-exist");
    let output = Command::new(compiler())
        .args(["build", "--json"])
        .arg(temporary.path())
        .arg("-o")
        .arg(&forbidden)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!forbidden.exists());
    let text = artifacts::command_output_text(&output);
    let entries = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>();
    assert_eq!(
        entries
            .iter()
            .filter(|item| item["code"] == "E0093")
            .count(),
        3,
        "{text}"
    );
    assert!(
        entries
            .iter()
            .all(|item| item["code"] == "E0093" || item["code"] == "C0001"),
        "{text}"
    );
}
