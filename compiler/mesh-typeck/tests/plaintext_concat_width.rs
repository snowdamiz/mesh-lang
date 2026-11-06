//! Concatenation status/extent depend on lengths, while all bytes stay private.
use mesh_typeck::error::TypeError;
use mesh_typeck::{ImportContext, ModuleExports};

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn unequal_private_words_with_public_widths_can_be_framed_and_sealed() {
    for concat in ["Bytes.concat", "bytes_concat"] {
        let source = format!(
            r#"
fn forward(value :: Bytes) -> Bytes do value end
fn encrypt(left :: Plaintext<U64>, right :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Int,String> do
 let a = case Bytes.write_u64_be(left) do Ok(value) -> value Err(_) -> return Err("word") end
 let b = case Bytes.write_u64_be(right) do Ok(value) -> value Err(_) -> return Err("word") end
 let framed = forward({concat}(a,b)?)
 println(Int.to_string(Bytes.length(framed)))
 let encrypted = case Crypto.aead_seal(key,nonce,Bytes.empty(),framed) do Ok(value) -> value Err(_) -> return Err("seal") end
 Ok(Http.body_bytes(Http.build(:post,"https://example.test"),encrypted))
end
"#
        );
        let result = checked(&source);
        assert!(result.errors.is_empty(), "{concat}: {:?}", result.errors);
    }
    let source = "fn encrypt(value :: Plaintext<Bytes>, key :: borrow StorageKey, context :: Bytes) -> Result<Int,String> do let framed = Bytes.concat(Bytes.from_utf8(\"frame\"),Crypto.sha256(value))?\nlet encrypted = case StorageKey.seal_bytes(framed,key,context) do Ok(value) -> value Err(_) -> return Err(\"seal\") end\nOk(Http.body_bytes(Http.build(:post,\"https://example.test\"),encrypted)) end";
    let result = checked(source);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn private_extent_error_payload_contents_and_branch_control_are_refused() {
    let sources = [
        "fn leak(value :: Plaintext<Bytes>, other :: Bytes) do if Result.is_err(Bytes.concat(value,other)) do println(\"overflow\") end end",
        "fn leak(value :: Plaintext<Bytes>, other :: Bytes) do case Bytes.concat(value,other) do Err(error) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),Bytes.from_utf8(error))\nOk(_) -> 0 end end",
        "fn append(a :: Bytes,b :: Bytes) -> Bytes!String do Bytes.concat(a,b) end\nfn leak(value :: Plaintext<Bytes>) do if Result.is_ok(append(value,Bytes.empty())) do println(\"status\") end end",
        "fn leak(count :: Plaintext<Int>) do let value = Result.unwrap_or(Bytes.repeat(1,count),Bytes.empty())\nif Result.is_err(Bytes.concat(value,value)) do println(\"overflow\") end end",
        "fn leak(value :: Plaintext<U64>) -> Result<Int,BytesError> do let a = Bytes.write_u64_be(value)?\nlet result = Bytes.concat(a,a)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(result,Bytes.empty()))\nOk(0) end",
        "fn leak(choice :: Plaintext<Bool>) do let result = if choice do Bytes.concat(Bytes.from_utf8(\"a\"),Bytes.from_utf8(\"b\")) else Bytes.concat(Bytes.from_utf8(\"c\"),Bytes.from_utf8(\"d\")) end\nprintln(String.from(Result.is_err(result))) end",
        "fn leak(value :: Plaintext<U64>, key :: borrow AeadKey) -> Result<Bytes,BytesError> do let bytes = Bytes.write_u64_be(value)?\nlet framed = Result.unwrap_or(Bytes.concat(bytes,bytes),Bytes.empty())\ncase Crypto.aead_seal(key,Bytes.from_utf8(\"123456789012\"),framed,Bytes.empty()) do Ok(output) -> Ok(output) Err(_) -> Ok(Bytes.empty()) end end",
        "module Bytes do pub fn concat(a :: Bytes,b :: Bytes) -> Bytes!String do if Result.unwrap_or(bytes_get(a,0),0) == 0 do Err(\"selected\") else Ok(b) end end end\nfn leak(value :: Plaintext<U64>) -> Result<Int,BytesError> do let bytes = bytes_write_u64_be(value)?\nprintln(String.from(Result.is_ok(Bytes.concat(bytes,bytes))))\nOk(0) end",
    ];
    for (index, source) in sources.iter().enumerate() {
        let result = checked(source);
        assert!(!result.errors.is_empty(), "case {index} escaped");
        assert!(
            result
                .errors
                .iter()
                .all(|e| matches!(e, TypeError::PlaintextViolation { .. })),
            "case {index}: {:?}",
            result.errors
        );
    }
}

#[test]
fn imported_concat_lookalike_cannot_hide_content_dependent_status() {
    let dependency = mesh_parser::parse("pub fn concat(a :: Bytes,b :: Bytes) -> Bytes!String do if Result.unwrap_or(bytes_get(a,0),0) == 0 do Err(\"selected\") else Ok(b) end end");
    let result = mesh_typeck::check(&dependency);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &result);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Untrusted.Bytes".into(),
        ModuleExports::new("Untrusted.Bytes".into(), &exports),
    );
    let parsed = mesh_parser::parse("import Untrusted.Bytes\nfn leak(value :: Plaintext<U64>) -> Result<Int,BytesError> do let bytes = bytes_write_u64_be(value)?\nprintln(String.from(Result.is_ok(Bytes.concat(bytes,bytes))))\nOk(0) end");
    assert!(parsed.ok(), "{:?}", parsed.errors());
    let result = mesh_typeck::check_with_imports(&parsed, &imports);
    assert!(!result.errors.is_empty());
    assert!(
        result
            .errors
            .iter()
            .all(|e| matches!(e, TypeError::PlaintextViolation { .. })),
        "{:?}",
        result.errors
    );
}
