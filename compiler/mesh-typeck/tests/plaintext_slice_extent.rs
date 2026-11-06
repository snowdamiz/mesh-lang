//! Slicing discloses no byte values through its bounds status or output extent.
use mesh_typeck::error::TypeError;
use mesh_typeck::{ImportContext, ModuleExports};

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn genuine_public_range_slices_of_private_fixed_width_data_can_be_sealed() {
    for slice in ["Bytes.slice", "bytes_slice"] {
        let source = format!(
            r#"
struct Frame do payload :: Bytes end
fn cut(value :: Bytes, start :: Int, count :: Int) -> Frame!String do
  Ok(Frame {{ payload: {slice}(value,start,count)? }})
end
fn encrypt(value :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes, start :: Int, count :: Int) -> Result<Int,String> do
  let frame = cut(Crypto.sha256(value),start,count)?
  println(Int.to_string(Bytes.length(frame.payload)))
  let encrypted = case Crypto.aead_seal(key,nonce,Bytes.empty(),frame.payload) do
    Ok(output) -> output
    Err(_) -> return Err("seal")
  end
  Ok(Http.body_bytes(Http.build(:post,"https://example.test"),encrypted))
end
fn public_bounds_error(value :: Plaintext<Bytes>) do
  case {slice}(Crypto.sha256(value),33,0) do
    Err(error) -> println(error)
    Ok(_) -> nil
  end
end
"#
        );
        let result = checked(&source);
        assert!(result.errors.is_empty(), "{slice}: {:?}", result.errors);
    }
}

#[test]
fn private_extents_ranges_payloads_and_control_remain_private() {
    let sources = [
        "fn leak(value :: Plaintext<Bytes>) do let part :: Plaintext<Bytes> = Result.unwrap_or(Bytes.slice(Crypto.sha256(value),0,12),Bytes.empty())\nprintln(Int.to_string(Bytes.length(part))) end",
        "fn leak(value :: Plaintext<Bytes>) do let sliced :: Plaintext<Result<Bytes,String>> = Bytes.slice(Crypto.sha256(value),0,12)\nprintln(String.from(Result.is_ok(sliced))) end",
        "fn leak(value :: Plaintext<Bytes>) do println(String.from(Result.is_ok(Bytes.slice(value,0,1)))) end",
        "fn leak(value :: Plaintext<Bytes>) do case Bytes.slice(value,0,1) do Err(error) -> println(error)\nOk(_) -> nil end end",
        "fn leak(value :: Plaintext<Bytes>, start :: Plaintext<Int>) do println(String.from(Result.is_ok(Bytes.slice(Crypto.sha256(value),start,1)))) end",
        "fn leak(value :: Plaintext<Bytes>, count :: Plaintext<Int>) do println(String.from(Result.is_ok(Bytes.slice(Crypto.sha256(value),0,count)))) end",
        "fn leak(value :: Plaintext<Bytes>) do let bytes = Result.unwrap_or(Bytes.slice(Crypto.sha256(value),0,12),Bytes.empty())\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),bytes) end",
        "fn leak(value :: Plaintext<Bytes>, key :: borrow AeadKey) do let bytes = Result.unwrap_or(Bytes.slice(Crypto.sha256(value),0,12),Bytes.empty())\nCrypto.aead_seal(key,bytes,Bytes.empty(),Bytes.empty()) end",
        "fn leak(value :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do let bytes = Result.unwrap_or(Bytes.slice(Crypto.sha256(value),0,12),Bytes.empty())\nCrypto.aead_seal(key,nonce,bytes,Bytes.empty()) end",
        "fn leak(choice :: Plaintext<Bool>) do let result = if choice do Bytes.slice(Bytes.from_utf8(\"ab\"),0,1) else Bytes.slice(Bytes.from_utf8(\"cd\"),0,1) end\nprintln(String.from(Result.is_err(result))) end",
        "module Bytes do pub fn slice(value :: Bytes,start :: Int,count :: Int) -> Bytes!String do if Result.unwrap_or(bytes_get(value,0),0) == 0 do Err(\"selected\") else Ok(value) end end end\nfn leak(value :: Plaintext<Bytes>) do println(String.from(Result.is_ok(Bytes.slice(Crypto.sha256(value),0,1)))) end",
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
fn imported_slice_lookalike_cannot_hide_content_dependent_errors() {
    let dependency = mesh_parser::parse("pub fn slice(value :: Bytes,start :: Int,count :: Int) -> Bytes!String do if Result.unwrap_or(bytes_get(value,0),0) == 0 do Err(\"selected\") else Ok(value) end end");
    let result = mesh_typeck::check(&dependency);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &result);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Untrusted.Bytes".into(),
        ModuleExports::new("Untrusted.Bytes".into(), &exports),
    );
    let parsed = mesh_parser::parse("import Untrusted.Bytes\nfn leak(value :: Plaintext<Bytes>) do println(String.from(Result.is_ok(Bytes.slice(Crypto.sha256(value),0,1)))) end");
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
