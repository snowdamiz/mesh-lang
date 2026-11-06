//! A bodyless constructor arm rebuilds its matched variant, not every variant.
use mesh_typeck::error::TypeError;

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{source}\n{:?}", parse.errors());
    mesh_typeck::check(&parse)
}

#[test]
fn shorthand_rebuilds_only_matched_payloads_in_direct_and_question_mark_results() {
    let source = r#"
fn direct(value :: Result<Bytes,Plaintext<Bytes>>) -> Bytes!String do
  case value do Ok(output)
    Err(_) -> Err("fixed error") end
end
fn with_try(value :: Result<Bytes,Plaintext<Bytes>>) -> Bytes!String do
  let bytes = case value do Ok(output)
    Err(_) -> Err("fixed error") end?
  Ok(bytes)
end
fn observe(value :: Result<Bytes,Plaintext<Bytes>>) do
  case direct(value) do Err(error) -> println(error) Ok(_) -> nil end
  case with_try(value) do Err(error) -> println(error) Ok(_) -> nil end
end
"#;
    let result = checked(source);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.plaintext.sites.is_empty());
}

#[test]
fn nested_and_nullary_shorthand_variants_drop_only_unreachable_payloads() {
    let source = r#"
type Outcome do Success
  Failure(Plaintext<Bytes>) end
fn nullary(value :: Outcome) -> Outcome do
  case value do Success
    Failure(_) -> Success end
end
fn nested(value :: Result<Result<Bytes,Plaintext<Bytes>>,String>) -> Result<Result<Bytes,String>,String> do
  case value do Ok(Ok(output))
    Ok(Err(_)) -> Ok(Err("fixed inner error"))
    Err(error) end
end
fn observe(value :: Result<Result<Bytes,Plaintext<Bytes>>,String>,outcome :: Outcome) do
  case nested(value) do Ok(Err(error)) -> println(error) Err(error) -> println(error) Ok(Ok(_)) -> nil end
  case nullary(outcome) do Success -> println("success") Failure(_) -> nil end
end
"#;
    let result = checked(source);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn shorthand_retains_selected_payloads_private_tags_guards_and_prior_control() {
    for source in [
        r#"fn leak(value :: Result<Plaintext<Bytes>,String>) do let rebuilt = case value do Ok(bytes)
Err(error) end
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(rebuilt,Bytes.empty())) end"#,
        r#"fn leak(value :: Result<Int,Plaintext<String>>) do let rebuilt = case value do Ok(number)
Err(error) end
case rebuilt do Err(error) -> println(error) Ok(_) -> nil end end"#,
        r#"fn leak(value :: Plaintext<Result<Int,String>>) do let rebuilt = case value do Ok(number)
Err(error) end
println(String.from(Result.is_ok(declassify_discriminant(rebuilt,"tag only")))) end"#,
        r#"fn leak(secret :: Plaintext<Bool>,first :: Result<Int,String>,second :: Result<Int,String>) do let selected = if secret do first else second end
let rebuilt = case selected do Ok(number)
Err(error) end
println(String.from(Result.is_ok(declassify_discriminant(rebuilt,"tag only")))) end"#,
        r#"fn leak(secret :: Plaintext<Bool>,value :: Result<Int,String>) do let rebuilt = case value do Ok(number) when secret
Ok(_) -> Err("denied")
Err(error) end
println(String.from(Result.is_ok(rebuilt))) end"#,
        r#"fn leak(value :: Result<Result<Plaintext<Bytes>,String>,String>) do let rebuilt = case value do Ok(Ok(bytes))
Ok(Err(error))
Err(error) end
case rebuilt do Ok(Ok(bytes)) -> Http.body_bytes(Http.build(:post,"https://example.invalid"),bytes) _ -> 0 end end"#,
    ] {
        let result = checked(source);
        assert!(!result.errors.is_empty(), "escaped: {source}");
        assert!(
            result
                .errors
                .iter()
                .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
            "{source}\n{:?}",
            result.errors
        );
    }
}

#[test]
fn shorthand_keeps_affine_payload_ownership() {
    let result = checked("fn forward(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do case value do Ok(secret)\nErr(error) end end");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let result = checked("fn twice(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do let rebuilt = case value do Ok(secret)\nErr(error) end\nvalue\nrebuilt end");
    assert!(
        result
            .errors
            .iter()
            .any(|error| matches!(error, TypeError::ResourceViolation { .. })),
        "{:?}",
        result.errors
    );
}
