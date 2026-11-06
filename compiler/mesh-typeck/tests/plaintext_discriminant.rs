//! Reviewed Result tags do not review either payload or private control.
use mesh_typeck::{error::TypeError, ImportContext, ModuleExports, TypeckResult};

fn checked(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check(&parse)
}
fn allowed(source: &str) {
    let result = checked(source);
    assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
}
fn refused(source: &str) {
    let result = checked(source);
    assert!(!result.errors.is_empty(), "escaped: {source}");
    assert!(
        result
            .errors
            .iter()
            .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
        "unrelated error: {source}\n{:?}",
        result.errors
    );
}

#[test]
fn separate_reviewed_bool_does_not_implicitly_review_the_original_result() {
    refused(
        r#"fn blocked(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do
  let encrypted = Crypto.aead_seal(key,nonce,Bytes.empty(),body)
  let admitted = declassify(Result.is_ok(encrypted),"test-only review of seal status")
  let bytes = Result.unwrap_or(encrypted,Bytes.empty())
  if admitted do Http.body_bytes(Http.build(:post,"https://example.invalid"),bytes) else 0 end
end"#,
    );
}

#[test]
fn reviewed_discriminant_allows_successful_ciphertext_selection_direct_and_pipe() {
    for review in [
        "declassify_discriminant(encrypted,\"test-only seal admission review\")",
        "encrypted |> declassify_discriminant(\"test-only seal admission review\")",
        "\"test-only seal admission review\" |2> declassify_discriminant(encrypted)",
    ] {
        allowed(&format!(
            r#"fn transmit(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do
  let encrypted = Crypto.aead_seal(key,nonce,Bytes.empty(),body)
  let reviewed = {review}
  Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty()))
end"#
        ));
    }
    allowed(
        r#"fn status(value :: Result<Int,String>) do
  println(String.from(Result.is_ok(declassify_discriminant(value,"test-only Result tag"))))
end"#,
    );
    allowed(
        r#"fn review(value :: Result<Bytes,CryptoError>) -> Result<Bytes,CryptoError> do
declassify_discriminant(value,"fixed helper seal admission") end
fn route(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(review(Crypto.aead_seal(key,nonce,Bytes.empty(),body)),Bytes.empty())) end"#,
    );
}

#[test]
fn reviewed_result_keeps_ok_err_nested_crypto_and_aggregate_payloads_private() {
    for source in [
        r#"fn leak(value :: Result<Plaintext<Bytes>,String>) do
let reviewed = declassify_discriminant(value,"tag only")
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty())) end"#,
        r#"fn leak(value :: Result<Int,Plaintext<String>>) do
case declassify_discriminant(value,"tag only") do Err(error) -> println(error)
Ok(_) -> nil end end"#,
        r#"fn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do
case declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"tag only") do
Err(InvalidLength(_,actual)) -> println(Int.to_string(actual))
_ -> nil end end"#,
        r#"struct Record do body :: Plaintext<Bytes> end
fn leak(value :: Result<Record,String>) do
case declassify_discriminant(value,"tag only") do Ok(record) -> Http.body_bytes(Http.build(:post,"https://example.invalid"),record.body)
Err(_) -> 0 end end"#,
        r#"fn leak(value :: Plaintext<Result<Bytes,String>>) do
case declassify_discriminant(value,"tag only") do Ok(bytes) -> Http.body_bytes(Http.build(:post,"https://example.invalid"),bytes)
Err(_) -> 0 end end"#,
        r#"fn leak(value :: Plaintext<Result<Bytes,String>>) do
case declassify_discriminant(value,"tag only") do Err(error) -> println(error)
Ok(_) -> nil end end"#,
        r#"fn leak(value :: Result<Plaintext<Result<Int,String>>,String>) do
case declassify_discriminant(value,"outer tag only") do
Ok(inner) -> println(String.from(Result.is_ok(inner)))
Err(_) -> nil end end"#,
        r#"fn leak(value :: Result<Plaintext<Result<Int,String>>,String>) do
case declassify_discriminant(value,"outer tag only") do
Ok(inner) -> println(String.from(Result.is_ok(declassify_discriminant(inner,"inner tag only"))))
Err(_) -> nil end end"#,
        r#"fn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Plaintext<Bytes>) do
let reviewed = declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"tag only")
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty())) end"#,
        r#"fn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes, aad :: Plaintext<Bytes>) do
let reviewed = declassify_discriminant(Crypto.aead_seal(key,nonce,aad,body),"tag only")
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty())) end"#,
    ] {
        refused(source);
    }
}

#[test]
fn caller_and_earlier_shape_control_are_not_erased() {
    for source in [
        // A whole-value source label also labels its shape control. This
        // narrow operation never erases that stronger source contract.
        r#"fn leak(value :: Plaintext<Result<Int,String>>) do
println(String.from(Result.is_ok(declassify_discriminant(value,"tag only")))) end"#,
        r#"fn leak(choice :: Plaintext<Bool>, value :: Result<Int,String>) do
if choice do println(String.from(Result.is_ok(declassify_discriminant(value,"tag only")))) end end"#,
        r#"fn leak(choice :: Plaintext<Bool>, first :: Result<Int,String>, second :: Result<Int,String>) do
let selected = if choice do declassify_discriminant(first,"tag only") else second end
println(String.from(Result.is_ok(selected))) end"#,
        r#"fn leak(choice :: Plaintext<Bool>, first :: Result<Bytes,String>, second :: Result<Bytes,String>) do
let selected = if choice do first else second end
let reviewed = declassify_discriminant(selected,"tag only")
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty())) end"#,
        r#"fn leak(choice :: Plaintext<Bool>, value :: Result<Int,String>) do
if choice do return nil end
println(String.from(Result.is_ok(declassify_discriminant(value,"tag only")))) end"#,
        r#"fn leak(value :: Result<Int,String>) do
let reviewed :: Plaintext<Result<Int,String>> = declassify_discriminant(value,"tag only")
println(String.from(Result.is_ok(reviewed))) end"#,
    ] {
        refused(source);
    }
}

#[test]
fn invalid_reasons_and_first_class_escape_are_rejected() {
    for call in [
        "declassify_discriminant(value,\"\")",
        "declassify_discriminant(value,\"   \")",
        "declassify_discriminant(value,\"\\n\")",
        "declassify_discriminant(value,reason)",
    ] {
        refused(&format!(
            "fn bad(value :: Result<Int,String>, reason :: String) do {call} end"
        ));
    }
    refused("fn bad(value :: Result<Int,String>) do let review = declassify_discriminant\nreview(value,\"indirect review\") end");
    assert!(
        !checked("fn bad(value :: Result<Int,String>) do declassify_discriminant(value) end")
            .errors
            .is_empty()
    );
    assert!(!checked(
        "fn bad(value :: Bool) do declassify_discriminant(value,\"not a Result\") end"
    )
    .errors
    .is_empty());
}

#[test]
fn local_and_imported_lookalikes_are_ordinary_calls() {
    refused(
        r#"fn declassify_discriminant(value :: Result<Bytes,CryptoError>,reason :: String) -> Result<Bytes,CryptoError> do value end
fn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do
let reviewed = declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"not the builtin")
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty())) end"#,
    );
    refused(
        r#"fn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do
let declassify_discriminant = fn(value,reason) -> value end
let reviewed = declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),"not the builtin")
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty())) end"#,
    );
    let dependency = mesh_parser::parse("pub fn declassify_discriminant(value :: Result<Bytes,CryptoError>,reason :: String) -> Result<Bytes,CryptoError> do value end");
    let result = mesh_typeck::check(&dependency);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &result);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Untrusted.Review".into(),
        ModuleExports::new("Untrusted.Review".into(), &exports),
    );
    for source in [
        "import Untrusted.Review\nfn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do let reviewed=Review.declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),\"not the builtin\")\nHttp.body_bytes(Http.build(:post,\"https://example.invalid\"),Result.unwrap_or(reviewed,Bytes.empty())) end",
        "from Untrusted.Review import declassify_discriminant\nfn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do let reviewed=declassify_discriminant(Crypto.aead_seal(key,nonce,Bytes.empty(),body),\"not the builtin\")\nHttp.body_bytes(Http.build(:post,\"https://example.invalid\"),Result.unwrap_or(reviewed,Bytes.empty())) end",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let result = mesh_typeck::check_with_imports(&parse, &imports);
        assert!(!result.errors.is_empty());
        assert!(result.errors.iter().all(|e| matches!(e, TypeError::PlaintextViolation { .. })), "{:?}", result.errors);
        assert!(result.plaintext.sites.is_empty());
    }
}

#[test]
fn unused_and_unreachable_discriminant_sites_are_distinct_and_deterministic() {
    let source = "fn make() -> Result<Int,String> do Ok(1) end\nfn unused(value :: Result<Int,String>) do declassify_discriminant(value,\"unused tag review\") end\nfn main() do if false do declassify_discriminant(make(),\"unreachable tag review\") end end";
    let result = checked(source);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result
            .plaintext
            .sites
            .iter()
            .map(|s| (s.kind.as_str(), s.reason.as_str()))
            .collect::<Vec<_>>(),
        [
            ("declassify_discriminant", "unused tag review"),
            ("declassify_discriminant", "unreachable tag review")
        ]
    );
}

#[test]
fn result_review_moves_affine_payload_exactly_once() {
    for expression in [
        "declassify_discriminant(value,\"tag review keeps ownership\")",
        "value |> declassify_discriminant(\"tag review keeps ownership\")",
        "\"tag review keeps ownership\" |2> declassify_discriminant(value)",
    ] {
        allowed(&format!("fn review(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do {expression} end"));
        allowed(&format!("fn review(value :: Result<Int,SecretBytes>) -> Result<Int,SecretBytes> do {expression} end"));
    }
    let result = checked("fn bad(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do let reviewed = declassify_discriminant(value,\"tag review keeps ownership\")\nvalue\nreviewed end");
    assert!(
        result
            .errors
            .iter()
            .any(|e| matches!(e, TypeError::ResourceViolation { .. })),
        "{:?}",
        result.errors
    );
    let result = checked("fn bad(value :: borrow Result<SecretBytes,String>) -> Result<SecretBytes,String> do declassify_discriminant(value,\"tag review cannot move a borrow\") end");
    assert!(
        result
            .errors
            .iter()
            .any(|e| matches!(e, TypeError::ResourceViolation { .. })),
        "{:?}",
        result.errors
    );
}

#[test]
fn imported_review_preserves_the_reviewed_tag_and_private_payloads() {
    let dependency = mesh_parser::parse(
        r#"pub fn review(value :: Result<Bytes,CryptoError>) -> Result<Bytes,CryptoError> do
declassify_discriminant(value,"test-only imported seal admission") end"#,
    );
    let checked_dependency = mesh_typeck::check(&dependency);
    assert!(
        checked_dependency.errors.is_empty(),
        "{:?}",
        checked_dependency.errors
    );
    let exports = mesh_typeck::collect_exports(&dependency, &checked_dependency);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Trusted.Review".into(),
        ModuleExports::new("Trusted.Review".into(), &exports),
    );
    for (source, is_allowed) in [
        (
            r#"import Trusted.Review
fn transmit(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do
let reviewed = Review.review(Crypto.aead_seal(key,nonce,Bytes.empty(),body))
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(reviewed,Bytes.empty())) end"#,
            true,
        ),
        (
            r#"import Trusted.Review
fn leak(body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do
case Review.review(Crypto.aead_seal(key,nonce,Bytes.empty(),body)) do
Err(InvalidLength(_,actual)) -> println(Int.to_string(actual))
_ -> nil end end"#,
            false,
        ),
        (
            r#"import Trusted.Review
fn leak(choice :: Plaintext<Bool>,body :: Plaintext<Bytes>,key :: borrow AeadKey,nonce :: Bytes) do
if choice do
Http.body_bytes(Http.build(:post,"https://example.invalid"),Result.unwrap_or(Review.review(Crypto.aead_seal(key,nonce,Bytes.empty(),body)),Bytes.empty()))
else 0 end end"#,
            false,
        ),
    ] {
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let result = mesh_typeck::check_with_imports(&parse, &imports);
        if is_allowed {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
        } else {
            assert!(!result.errors.is_empty());
            assert!(
                result
                    .errors
                    .iter()
                    .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
                "{:?}",
                result.errors
            );
        }
    }
}

#[test]
fn qualified_import_linker_alias_does_not_shadow_or_borrow_the_builtin() {
    let dependency = mesh_parser::parse("pub fn declassify_discriminant(value :: borrow Result<SecretBytes,String>,reason :: String) -> Int do 7 end");
    let checked_dependency = mesh_typeck::check(&dependency);
    assert!(
        checked_dependency.errors.is_empty(),
        "{:?}",
        checked_dependency.errors
    );
    let exports = mesh_typeck::collect_exports(&dependency, &checked_dependency);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.Review".into(),
        ModuleExports::new("Foreign.Review".into(), &exports),
    );
    for expression in [
        "declassify_discriminant(value,\"genuine builtin\")",
        "value |> declassify_discriminant(\"genuine builtin\")",
    ] {
        let source = format!("import Foreign.Review\nfn move_once(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do {expression} end");
        let result = mesh_typeck::check_with_imports(&mesh_parser::parse(&source), &imports);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let bad_source = format!("import Foreign.Review\nfn move_twice(value :: Result<SecretBytes,String>) -> Result<SecretBytes,String> do let reviewed = {expression}\nvalue\nreviewed end");
        let result = mesh_typeck::check_with_imports(&mesh_parser::parse(&bad_source), &imports);
        assert!(
            result
                .errors
                .iter()
                .any(|error| matches!(error, TypeError::ResourceViolation { .. })),
            "{:?}",
            result.errors
        );
    }
    let source = "from Foreign.Review import declassify_discriminant\nfn ordinary(value :: borrow Result<SecretBytes,String>) -> Int do declassify_discriminant(value,\"ordinary imported function\") end";
    let result = mesh_typeck::check_with_imports(&mesh_parser::parse(source), &imports);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.plaintext.sites.is_empty());
}

#[test]
fn prior_control_survives_local_imported_and_nested_review_helpers() {
    let dependency = mesh_parser::parse(
        r#"pub fn review(value :: Result<Int,String>) -> Result<Int,String> do declassify_discriminant(value,"imported tag only") end"#,
    );
    let checked_dependency = mesh_typeck::check(&dependency);
    assert!(
        checked_dependency.errors.is_empty(),
        "{:?}",
        checked_dependency.errors
    );
    let exports = mesh_typeck::collect_exports(&dependency, &checked_dependency);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.Review".into(),
        ModuleExports::new("Foreign.Review".into(), &exports),
    );
    for (definitions, expression) in [
        ("", "declassify_discriminant(selected,\"direct tag only\")"),
        ("fn review(value :: Result<Int,String>) -> Result<Int,String> do declassify_discriminant(value,\"local tag only\") end", "review(selected)"),
        ("import Foreign.Review", "Review.review(selected)"),
        ("struct Holder do value :: Result<Int,String> end", "declassify_discriminant(Holder { value: selected }.value,\"record tag only\")"),
        ("fn review_tuple(pair :: (Result<Int,String>,Int)) -> Result<Int,String> do let (value,_) = pair\ndeclassify_discriminant(value,\"tuple tag only\") end", "review_tuple((selected,1))"),
        ("", "declassify_discriminant(List.get([selected],0),\"list tag only\")"),
        ("import Foreign.Review\nstruct Holder do value :: Result<Int,String> end", "Review.review(Holder { value: selected }.value)"),
        ("fn captured(value :: Result<Int,String>) -> Result<Int,String> do let review = fn() -> declassify_discriminant(value,\"captured tag only\") end\nreview() end", "captured(selected)"),
    ] {
        let source = format!("{definitions}\nfn leak(secret :: Plaintext<Bool>,one :: Result<Int,String>,two :: Result<Int,String>) do let selected = if secret do one else two end\nprintln(String.from(Result.is_ok({expression}))) end");
        let parse = mesh_parser::parse(&source);
        assert!(parse.ok(), "{source}\n{:?}", parse.errors());
        let result = mesh_typeck::check_with_imports(&parse,&imports);
        assert!(!result.errors.is_empty(), "escaped: {source}");
        assert!(result.errors.iter().all(|error| matches!(error,TypeError::PlaintextViolation { .. })), "{source}\n{:?}", result.errors);
    }
}

#[test]
fn unknown_control_beyond_the_shape_budget_is_conservative() {
    let mut definitions = "struct Layer0 do value :: Result<Int,String> end\n".to_string();
    for depth in 1..15 {
        definitions.push_str(&format!(
            "struct Layer{depth} do value :: Layer{} end\n",
            depth - 1
        ));
    }
    let mut constructor = "Layer0 { value: selected }".to_string();
    for depth in 1..15 {
        constructor = format!("Layer{depth} {{ value: {constructor} }}");
    }
    let projection = format!("value{}", ".value".repeat(15));
    refused(&format!("{definitions}\nfn review_deep(value :: Layer14) do println(String.from(Result.is_ok(declassify_discriminant({projection},\"deep tag only\")))) end\nfn leak(secret :: Plaintext<Bool>,one :: Result<Int,String>,two :: Result<Int,String>) do let selected = if secret do one else two end\nreview_deep({constructor}) end"));
}
