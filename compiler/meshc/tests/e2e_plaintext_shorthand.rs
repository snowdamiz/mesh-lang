//! A shorthand arm has the same data/control and ownership as its explicit value.
use std::{fs, path::PathBuf, process::Command};
#[path = "support/test_artifacts.rs"]
mod artifacts;
fn compiler() -> PathBuf {
    std::env::var_os("MESH_PLAINTEXT_TEST_COMPILER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(artifacts::meshc_bin)
}

#[test]
fn compiled_shorthand_imports_question_marks_and_affine_payloads_match_explicit_arms() {
    artifacts::ensure_mesh_rt_staticlib();
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("transform.mpl"),
        r#"
pub fn direct(value :: Result<Bytes,Plaintext<Bytes>>) -> Bytes!String do
  case value do
    Ok(output)
    Err(_) -> Err("fixed error")
  end
end
pub fn with_try(value :: Result<Bytes,Plaintext<Bytes>>) -> Bytes!String do
  let bytes = case value do
    Ok(output)
    Err(_) -> Err("fixed error")
  end?
  Ok(bytes)
end
pub fn owned(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do
  case value do
    Ok(secret)
    Err(error)
  end
end
"#,
    )
    .unwrap();
    fs::write(temporary.path().join("main.mpl"),r#"
import Transform
fn check_secret() -> Bool!CryptoError do
  let secret = case Transform.owned(Ok(Secret.random(32)?)) do
    Ok(value) -> value
    Err(_) -> return Err(InvalidKey)
  end
  let tag = Crypto.hmac_sha256(secret,Bytes.empty())?
  Secret.destroy(tag)
  Secret.destroy(secret)
  Ok(true)
end
fn observe(value :: Result<Bytes,Plaintext<Bytes>>) do
  case Transform.direct(value) do Ok(output) -> println(Int.to_string(Bytes.length(output))) Err(error) -> println(error) end
  case Transform.with_try(value) do Ok(output) -> println(Int.to_string(Bytes.length(output))) Err(error) -> println(error) end
end
fn main() do
  observe(Ok(Bytes.from_utf8("body")))
  observe(Err(Bytes.from_utf8("private unused error bytes")))
  case check_secret() do Ok(true) -> println("shorthand-resource-ok") _ -> panic("resource lost") end
end
"#).unwrap();
    let mut prior = None;
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
        let run = Command::new(&binary).output().unwrap();
        assert!(
            run.status.success(),
            "{}",
            artifacts::command_output_text(&run)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "4\n4\nfixed error\nfixed error\nshorthand-resource-ok"
        );
        let report = fs::read(binary.with_extension("plaintext.json")).unwrap();
        if let Some(prior) = &prior {
            assert_eq!(prior, &report);
        }
        let parsed: serde_json::Value = serde_json::from_slice(&report).unwrap();
        assert!(parsed["modules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|module| module["sites"].as_array().unwrap().is_empty()));
        prior = Some(report);
    }
}

#[test]
fn compiled_shorthand_cannot_discard_selected_private_data_or_control() {
    for source in [
        r#"fn leak(value :: Result<Plaintext<Bytes>,String>) do let rebuilt=case value do Ok(bytes)
Err(error) end
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(rebuilt,Bytes.empty())) end"#,
        r#"fn leak(value :: Result<Int,Plaintext<String>>) do let rebuilt=case value do Ok(number)
Err(error) end
case rebuilt do Err(error) -> println(error) Ok(_) -> nil end end"#,
        r#"fn leak(secret :: Plaintext<Bool>,first :: Result<Int,String>,second :: Result<Int,String>) do let input=if secret do first else second end
let rebuilt=case input do Ok(number)
Err(error) end
println(String.from(Result.is_ok(declassify_discriminant(rebuilt,"tag only")))) end"#,
    ] {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("main.mpl"),
            format!("{source}\nfn main() do nil end"),
        )
        .unwrap();
        let binary = temporary.path().join("blocked");
        let output = Command::new(compiler())
            .args(["build", "--json"])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(!output.status.success(), "escaped: {source}");
        assert!(!binary.exists() && !binary.with_extension("plaintext.json").exists());
        let diagnostics = String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .collect::<Vec<_>>();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == "E0093"),
            "{}",
            artifacts::command_output_text(&output)
        );
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic["code"] == "E0093" || diagnostic["code"] == "C0001"));
    }
}
