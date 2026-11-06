//! Check implicit conversion against the compiled, imported From body.
use std::{fs, process::Command};

#[path = "support/test_artifacts.rs"]
mod artifacts;

const ERRORS: &str = r#"
pub type SourceError do First(detail :: String) Second end
pub type WrappedError do Wrapped(error :: SourceError) Other end
impl From<SourceError> for WrappedError do
  fn from(error :: SourceError) -> WrappedError do Wrapped(error) end
end
impl From<Int> for WrappedError do
  fn from(error :: Int) -> WrappedError do Other end
end
"#;

const PROPAGATE: &str = r#"
from Errors import SourceError, WrappedError
fn implicit(value :: Result<Int,Plaintext<SourceError>>) -> Int!WrappedError do
  Ok(value?)
end
fn explicit(value :: Result<Int,Plaintext<SourceError>>) -> Int!WrappedError do
  case value do Ok(number) -> Ok(number) Err(error) -> Err(Wrapped(error)) end
end
"#;

#[test]
fn compiled_from_wrapper_matches_explicit_error_mapping() {
    let temporary = tempfile::tempdir().unwrap();
    let (types, implementations) = ERRORS.split_once("impl From").unwrap();
    fs::write(temporary.path().join("errors.mpl"), types).unwrap();
    fs::write(
        temporary.path().join("conversions.mpl"),
        format!("from Errors import SourceError, WrappedError\nimpl From{implementations}"),
    )
    .unwrap();
    fs::write(
        temporary.path().join("bridge.mpl"),
        "import Conversions\npub fn ready() do nil end\n",
    )
    .unwrap();
    fs::write(temporary.path().join("main.mpl"), format!(r#"import Bridge
{PROPAGATE}
fn observe(value :: Result<Int,Plaintext<SourceError>>) do
  case implicit(value) do Err(Other) -> println("other") Err(_) -> println("wrapped") Ok(number) -> println(Int.to_string(number)) end
  case explicit(value) do Err(Other) -> println("other") Err(_) -> println("wrapped") Ok(number) -> println(Int.to_string(number)) end
end
fn main() do
  observe(Ok(7))
  observe(Err(First("private detail")))
  observe(Err(Second))
end"#)).unwrap();
    let compiler = artifacts::meshc_bin();
    let mut prior = None;
    for level in ["0", "2"] {
        let binary = temporary.path().join(format!("proof-{level}"));
        let build = Command::new(&compiler)
            .args(["build", "--opt-level", level])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            artifacts::command_output_text(&build)
        );
        let run = Command::new(&binary).output().unwrap();
        assert!(
            run.status.success(),
            "{}",
            artifacts::command_output_text(&run)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "7\n7\nwrapped\nwrapped\nwrapped\nwrapped"
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
fn compiled_conversion_refuses_payload_control_and_side_effect_leaks() {
    let compiler = artifacts::meshc_bin();
    for (errors, observe) in [
        (ERRORS.to_string(), r#"fn observe(value :: Result<Int,Plaintext<SourceError>>) do case implicit(value) do Err(Wrapped(First(detail))) -> println(detail) _ -> nil end end"#),
        (ERRORS.replace("do Wrapped(error)", "do case error do First(_) -> Wrapped(error) Second -> Other end"), r#"fn observe(value :: Result<Int,Plaintext<SourceError>>) do case implicit(value) do Err(Other) -> println("other") _ -> nil end end"#),
        (ERRORS.replace("do Wrapped(error)", "do case error do First(detail) -> println(detail) Second -> nil end\nWrapped(error)"), ""),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join("errors.mpl"), errors).unwrap();
        fs::write(temporary.path().join("main.mpl"), format!("{PROPAGATE}\n{observe}\nfn main() do nil end")).unwrap();
        let binary = temporary.path().join("blocked");
        let build = Command::new(&compiler).args(["build", "--json"]).arg(temporary.path())
            .arg("-o").arg(&binary).output().unwrap();
        assert!(!build.status.success(), "private conversion escaped");
        assert!(!binary.exists() && !binary.with_extension("plaintext.json").exists());
        let diagnostics = String::from_utf8_lossy(&build.stderr).lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()).collect::<Vec<_>>();
        assert!(diagnostics.iter().any(|diagnostic| diagnostic["code"] == "E0093"), "{}", artifacts::command_output_text(&build));
        assert!(diagnostics.iter().all(|diagnostic| diagnostic["code"] == "E0093" || diagnostic["code"] == "C0001"));
    }
}

#[test]
fn compiled_primitive_from_matches_explicit_conversion() {
    let temporary = tempfile::tempdir().unwrap();
    let mut program = String::new();
    let mut main = String::new();
    let mut expected = String::new();
    for (id, source, target, values) in [
        ("int", "Int", "String", vec![("0", "0"), ("-7", "-7")]),
        (
            "float",
            "Float",
            "String",
            vec![("0.0", "0.0"), ("-2.5", "-2.5")],
        ),
        (
            "bool",
            "Bool",
            "String",
            vec![("false", "false"), ("true", "true")],
        ),
        ("widen", "Int", "Float", vec![("0", "0.0"), ("-7", "-7.0")]),
    ] {
        let printed = if target == "String" {
            "error"
        } else {
            "String.from(error)"
        };
        for (style, body) in [
            ("implicit", "Ok(value?)".to_string()),
            ("explicit", format!("case value do Ok(number) -> Ok(number) Err(error) -> Err({target}.from(error)) end")),
        ] {
            program.push_str(&format!(r#"
fn {id}_{style}(value :: Result<Int,{source}>) -> Int!{target} do {body} end
fn {id}_{style}_private(value :: Result<Int,Plaintext<{source}>>) -> Int!{target} do {body} end
fn {id}_{style}_observe(value :: Result<Int,Plaintext<{source}>>) do
  case {id}_{style}_private(value) do Ok(_) -> println("ok") Err(_) -> println("err") end
end
"#));
            program.push_str(&format!("fn {id}_{style}_error() -> Result<Int,{source}> do Err({}) end\n", values[0].0));
            main.push_str(&format!("case {id}_{style}({id}_{style}_error()) do Err(error) -> println({printed}) Ok(_) -> println(\"wrong\") end\n"));
            expected.push_str(&format!("{}\n", values[0].1));
            main.push_str(&format!("{id}_{style}_observe(Ok(23))\n"));
            expected.push_str("ok\n");
            for (literal, text) in &values {
                main.push_str(&format!(r#"
case {id}_{style}(Err({literal})) do Err(error) -> println({printed}) Ok(_) -> println("wrong") end
{id}_{style}_observe(Err({literal}))
"#));
                expected.push_str(&format!("{text}\nerr\n"));
            }
            main.push_str(&format!(r#"case {id}_{style}(Ok(23)) do Ok(number) -> println(Int.to_string(number)) Err(_) -> println("wrong") end
"#));
            expected.push_str("23\n");
        }
    }
    fs::write(
        temporary.path().join("main.mpl"),
        format!("{program}\nfn main() do\n{main}end"),
    )
    .unwrap();
    let compiler = artifacts::meshc_bin();
    let mut prior_report = None;
    for level in ["0", "2"] {
        let binary = temporary.path().join(format!("proof-{level}"));
        let build = Command::new(&compiler)
            .args(["build", "--opt-level", level])
            .arg(temporary.path())
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            artifacts::command_output_text(&build)
        );
        let run = Command::new(&binary).output().unwrap();
        assert!(
            run.status.success(),
            "{}",
            artifacts::command_output_text(&run)
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected);
        let report = fs::read(binary.with_extension("plaintext.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&report).unwrap();
        assert!(parsed["modules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|module| module["sites"].as_array().unwrap().is_empty()));
        if let Some(prior) = prior_report {
            assert_eq!(prior, report);
        }
        prior_report = Some(report);
    }
}

#[test]
fn compiled_primitive_from_refuses_payload_and_control_leaks() {
    let compiler = artifacts::meshc_bin();
    for source in ["Int", "Float", "Bool"] {
        for (input, body, observe, args) in [
            (format!("value :: Result<Int,Plaintext<{source}>>"), "Ok(value?)", "Err(error) -> println(error) Ok(_) -> nil", "value"),
            (format!("value :: Plaintext<Result<Int,{source}>>"), "Ok(value?)", "Err(_) -> println(\"err\") Ok(_) -> println(\"ok\")", "value"),
            (format!("secret :: Plaintext<Bool>, one :: Result<Int,{source}>, two :: Result<Int,{source}>"), "Ok((if secret do one else two end)?)", "Err(_) -> println(\"err\") Ok(_) -> println(\"ok\")", "secret,one,two"),
            (format!("secret :: Plaintext<Bool>, one :: Result<Int,{source}>, two :: Result<Int,{source}>"), "if secret do Ok(one?) else Ok(two?) end", "Err(_) -> println(\"err\") Ok(_) -> println(\"ok\")", "secret,one,two"),
        ] {
            let temporary = tempfile::tempdir().unwrap();
            fs::write(temporary.path().join("main.mpl"), format!(r#"
fn propagate({input}) -> Int!String do {body} end
fn observe({input}) do case propagate({args}) do {observe} end end
fn main() do nil end"#)).unwrap();
            let binary = temporary.path().join("blocked");
            let build = Command::new(&compiler).args(["build", "--json"]).arg(temporary.path()).arg("-o").arg(&binary).output().unwrap();
            assert!(!build.status.success(), "private {source} conversion escaped");
            assert!(!binary.exists() && !binary.with_extension("plaintext.json").exists());
            let diagnostics = String::from_utf8_lossy(&build.stderr).lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()).collect::<Vec<_>>();
            assert!(diagnostics.iter().any(|diagnostic| diagnostic["code"] == "E0093"), "{}", artifacts::command_output_text(&build));
            assert!(diagnostics.iter().all(|diagnostic| diagnostic["code"] == "E0093" || diagnostic["code"] == "C0001"));
        }
    }
}
