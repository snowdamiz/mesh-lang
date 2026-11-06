//! A total typed integer write has public status and size, never public bits.
use mesh_typeck::error::TypeError;

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check(&parse)
}

fn allowed(source: &str) {
    let result = checked(source);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

fn refused(source: &str) {
    let result = checked(source);
    assert!(
        result
            .errors
            .iter()
            .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
        "unrelated errors: {:?}",
        result.errors
    );
    assert!(!result.errors.is_empty(), "plaintext escaped: {source}");
}

#[test]
fn full_width_u64_encoding_keeps_public_status_and_fixed_size_through_question_mark() {
    for writer in ["Bytes.write_u64_be", "bytes_write_u64_be"] {
        allowed(&format!("fn keep(value :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Result<Bytes,CryptoError>,BytesError> do let encoded = {writer}(value)?\nOk(Crypto.aead_seal(key,nonce,Bytes.empty(),encoded)) end"));
        allowed(&format!("fn footprint(value :: Plaintext<U64>) -> Result<Int,BytesError> do let encoded = {writer}(value)?\nprintln(Int.to_string(Bytes.length(encoded)))\nOk(0) end"));
        allowed(&format!("fn forwarded(value :: U64) -> Result<Bytes,BytesError> do {writer}(value) end\nfn footprint(value :: Plaintext<U64>) -> Result<Int,BytesError> do let encoded = forwarded(value)?\nprintln(Int.to_string(bytes_length(encoded)))\nOk(0) end"));
        allowed(&format!("fn footprint(value :: Plaintext<U64>) -> Result<Int,BytesError> do let encoded = forward({writer}(value)?)\nprintln(Int.to_string(Bytes.length(encoded)))\nOk(0) end\nfn forward(value :: Bytes) -> Bytes do value end"));
    }
}

#[test]
fn fixed_encoding_proofs_do_not_clear_bits_metadata_control_or_fresh_labels() {
    for tail in [
        "let encoded = Bytes.write_u64_be(value)?\nOk(Http.body_bytes(Http.build(:post,\"https://example.test\"),encoded))",
        "let encoded :: Plaintext<Bytes> = Bytes.write_u64_be(value)?\nprintln(Int.to_string(Bytes.length(encoded)))\nOk(0)",
        "let checked :: Plaintext<Result<Bytes,BytesError>> = Bytes.write_u64_be(value)\nlet encoded = checked?\nprintln(Int.to_string(Bytes.length(encoded)))\nOk(0)",
    ] {
        refused(&format!("fn leak(value :: Plaintext<U64>) -> Result<Int,BytesError> do {tail} end"));
    }
    refused("fn leak(value :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Result<Bytes,CryptoError>,BytesError> do let encoded = Bytes.write_u64_be(value)?\nOk(Crypto.aead_seal(key,nonce,encoded,Bytes.empty())) end");
    refused("fn leak(flag :: Plaintext<Bool>, value :: U64) -> Result<Int,BytesError> do if flag do let encoded = Bytes.write_u64_be(value)?\nprintln(Int.to_string(Bytes.length(encoded))) else nil end\nOk(0) end");
    refused("fn leak(flag :: Plaintext<Bool>, value :: U64) -> Result<Int,BytesError> do let chosen = if flag do Bytes.write_u64_be(value) else Bytes.write_u64_be(value) end\nlet encoded = chosen?\nprintln(Int.to_string(Bytes.length(encoded)))\nOk(0) end");
    refused("fn forward(value :: Bytes) -> Bytes do value end\nfn leak(value :: Plaintext<Bytes>) do println(Int.to_string(Bytes.length(forward(value)))) end");
}

#[test]
fn narrow_writes_and_same_name_untrusted_encoders_keep_dependent_failure() {
    refused("fn keep(value :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Result<Bytes,CryptoError>,BytesError> do let encoded = Bytes.write_u32_be(value)?\nOk(Crypto.aead_seal(key,nonce,Bytes.empty(),encoded)) end");
    refused("fn keep(value :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Result<Bytes,CryptoError>,String> do let encoded = Bytes.write_uint_le(U64.to_string(value),8)?\nOk(Crypto.aead_seal(key,nonce,Bytes.empty(),encoded)) end");
    refused("module Bytes do fn write_u64_be(value :: U64) -> Result<Bytes,BytesError> do let flag :: Plaintext<Bool> = true\nif flag do Ok(bytes_from_utf8(\"a\")) else Ok(bytes_from_utf8(\"bb\")) end end end\nfn leak(value :: U64) -> Result<Int,BytesError> do let encoded = Bytes.write_u64_be(value)?\nprintln(Int.to_string(bytes_length(encoded)))\nOk(0) end");
    refused("fn bytes_write_u64_be(value :: U64) -> Result<Bytes,BytesError> do let flag :: Plaintext<Bool> = true\nif flag do Ok(bytes_from_utf8(\"a\")) else Ok(bytes_from_utf8(\"bb\")) end end\nfn leak(value :: U64) -> Result<Int,BytesError> do let encoded = bytes_write_u64_be(value)?\nprintln(Int.to_string(bytes_length(encoded)))\nOk(0) end");
}
