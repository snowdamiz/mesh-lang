//! Hash outputs keep private bits while their genuine fixed byte width is public.
use mesh_typeck::error::TypeError;
use mesh_typeck::{ImportContext, ModuleExports};

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check(&parse)
}

#[test]
fn genuine_hash_width_can_be_observed_and_sealed_through_forwarding() {
    for hash in [
        "Crypto.sha256",
        "crypto_sha256",
        "Crypto.sha512",
        "crypto_sha512",
    ] {
        let source = format!(
            r#"
fn forward(bytes :: Bytes) -> Bytes do bytes end
fn encrypt(value :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Int,CryptoError> do
  let digest = forward({hash}(value))
  println(Int.to_string(Bytes.length(digest)))
  let encrypted = Crypto.aead_seal(key,nonce,Bytes.empty(),digest)?
  Ok(Http.body_bytes(Http.build(:post,"https://example.test"),encrypted))
end
"#
        );
        let result = checked(&source);
        assert!(result.errors.is_empty(), "{hash}: {:?}", result.errors);
    }
}

#[test]
fn hash_bits_metadata_and_private_algorithm_choice_remain_private() {
    let sources = [
        "fn leak(value :: Plaintext<Bytes>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),Crypto.sha256(value)) end",
        "fn leak(value :: Plaintext<Bytes>) do println(Bytes.to_hex(Crypto.sha512(value))) end",
        "fn leak(value :: Plaintext<Bytes>) do Json.encode_string(Bytes.to_hex(Crypto.sha256(value))) end",
        "fn leak(value :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do Crypto.aead_seal(key,nonce,Crypto.sha256(value),Bytes.empty()) end",
        "fn leak(value :: Plaintext<Bytes>, key :: borrow AeadKey) do Crypto.aead_seal(key,Crypto.sha256(value),Bytes.empty(),Bytes.empty()) end",
        "fn leak(choice :: Plaintext<Bool>) do let digest = if choice do Crypto.sha256(Bytes.empty()) else Crypto.sha512(Bytes.empty()) end\nprintln(Int.to_string(Bytes.length(digest))) end",
        "@export(\"private_hash\") pub fn leak(input :: Bytes) -> Bytes!String do let value :: Plaintext<Bytes> = input\nOk(Crypto.sha256(value)) end",
        "fn leak(value :: Plaintext<Bytes>) do let digest :: Plaintext<Bytes> = Crypto.sha256(value)\nprintln(Int.to_string(Bytes.length(digest))) end",
        "module Crypto do pub fn sha256(value :: Bytes) -> Bytes do value end end\nfn leak(value :: Plaintext<Bytes>) do println(Int.to_string(Bytes.length(Crypto.sha256(value)))) end",
    ];
    for (index, source) in sources.iter().enumerate() {
        let result = checked(source);
        assert!(!result.errors.is_empty(), "case {index} escaped");
        assert!(
            result
                .errors
                .iter()
                .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
            "case {index}: {:?}",
            result.errors
        );
    }
}

#[test]
fn imported_hash_lookalikes_do_not_receive_a_constant_width_proof() {
    let dependency = mesh_parser::parse("pub fn sha256(value :: Bytes) -> Bytes do value end");
    let result = mesh_typeck::check(&dependency);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &result);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Untrusted.Crypto".into(),
        ModuleExports::new("Untrusted.Crypto".into(), &exports),
    );
    let source = mesh_parser::parse("import Untrusted.Crypto\nfn leak(value :: Plaintext<Bytes>) do println(Int.to_string(Bytes.length(Crypto.sha256(value)))) end");
    assert!(source.ok(), "{:?}", source.errors());
    let result = mesh_typeck::check_with_imports(&source, &imports);
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
