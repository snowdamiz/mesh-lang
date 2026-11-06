//! Explicit extended-nonce AEAD preserves opaque borrowed keys and plaintext boundaries.
use mesh_typeck::error::TypeError;
fn check(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}
#[test]
fn extended_nonce_aead_types_and_borrowed_key_survive_direct_flat_and_named_forwarding() {
    for (seal, open) in [
        (
            "Crypto.xchacha20poly1305_seal",
            "Crypto.xchacha20poly1305_open",
        ),
        (
            "crypto_xchacha20poly1305_seal",
            "crypto_xchacha20poly1305_open",
        ),
    ] {
        let source=format!("fn decrypt_forward(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Bytes) -> Bytes!CryptoError do {open}(key,nonce,aad,body) end\nfn roundtrip(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, body :: Bytes) -> Bytes!CryptoError do\nlet encrypted = {seal}(key,nonce,aad,body)?\nlet decrypted = decrypt_forward(key,nonce,aad,encrypted)?\nlet again = {seal}(key,nonce,aad,decrypted)?\n{open}(key,nonce,aad,again) end");
        let result = check(&source);
        assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
    }
}
#[test]
fn extended_nonce_aead_rejects_wrong_types_and_destroyed_or_consumed_keys() {
    for operation in [
        "Crypto.xchacha20poly1305_seal",
        "crypto_xchacha20poly1305_seal",
        "Crypto.xchacha20poly1305_open",
        "crypto_xchacha20poly1305_open",
    ] {
        for source in [format!("fn bad(key :: SecretBytes, nonce :: Bytes, body :: Bytes) do {operation}(key,nonce,Bytes.empty(),body) end"),format!("fn bad(key :: AeadKey, nonce :: String, body :: Bytes) do {operation}(key,nonce,Bytes.empty(),body) end"),format!("fn bad(key :: AeadKey, nonce :: Bytes, body :: Bytes) -> Bytes do {operation}(key,nonce,Bytes.empty(),body) end"),format!("fn bad(key :: AeadKey, nonce :: Bytes, body :: Bytes) do Secret.destroy(key)\n{operation}(key,nonce,Bytes.empty(),body) end")] {
            assert!(!check(&source).errors.is_empty(),"accepted {source}");
        }
    }
}
#[test]
fn extended_nonce_ciphertext_can_leave_but_private_nonce_aad_and_caller_control_cannot() {
    for operation in [
        "Crypto.xchacha20poly1305_seal",
        "crypto_xchacha20poly1305_seal",
    ] {
        let source=format!("fn seal(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Plaintext<Bytes>) do\nlet encrypted=declassify_discriminant({operation}(key,nonce,aad,body),\"test-only finite encryption status\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(encrypted,Bytes.empty())) end");
        let result = check(&source);
        assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
        for args in ["key,private,aad,body", "key,nonce,private,body"] {
            let source=format!("fn leak(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Bytes,private :: Plaintext<Bytes>) do\nlet encrypted=declassify_discriminant({operation}({args}),\"test-only finite encryption status\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(encrypted,Bytes.empty())) end");
            let result = check(&source);
            assert!(
                result
                    .errors
                    .iter()
                    .any(|e| matches!(e, TypeError::PlaintextViolation { .. })),
                "escaped {source}\n{:?}",
                result.errors
            );
        }
        let source=format!("fn leak(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Bytes,choice :: Plaintext<Bool>) do\nif choice do\nlet encrypted={operation}(key,nonce,aad,body)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(encrypted,Bytes.empty())) end end");
        assert!(
            check(&source)
                .errors
                .iter()
                .any(|e| matches!(e, TypeError::PlaintextViolation { .. })),
            "caller control escaped"
        );
    }
}
#[test]
fn extended_nonce_open_keeps_private_dependencies_through_named_forwarding_and_result_tag() {
    for operation in [
        "Crypto.xchacha20poly1305_open",
        "crypto_xchacha20poly1305_open",
    ] {
        for tail in ["println(String.from(Result.is_ok(opened)))","Http.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(opened,Bytes.empty()))"] {
            let source=format!("fn decrypt_forward(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Bytes) -> Bytes!CryptoError do {operation}(key,nonce,aad,body) end\nfn leak(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Plaintext<Bytes>) do\nlet opened=decrypt_forward(key,nonce,aad,body)\n{tail} end");
            let result=check(&source);assert!(result.errors.iter().any(|e|matches!(e,TypeError::PlaintextViolation{..})),"escaped {source}\n{:?}",result.errors);
        }
    }
}

#[test]
fn extended_nonce_aead_preserves_existing_affine_function_value_refusal() {
    for operation in [
        "Crypto.aead_seal",
        "Crypto.aead_open",
        "Crypto.xchacha20poly1305_seal",
        "Crypto.xchacha20poly1305_open",
    ] {
        let source=format!("fn indirect(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, body :: Bytes) -> Bytes!CryptoError do\nlet operation_value={operation}\noperation_value(key,nonce,aad,body) end");
        let result = check(&source);
        assert!(
            result
                .errors
                .iter()
                .all(|e| matches!(e, TypeError::ResourceViolation { .. })),
            "unexpected {operation}: {:?}",
            result.errors
        );
        assert!(result.errors.iter().any(|e|matches!(e,TypeError::ResourceViolation{reason,..} if reason.contains("generic or indirect call"))),"accepted {operation}");
    }
}

#[test]
fn extended_nonce_named_lookalikes_cannot_claim_an_encryption_boundary() {
    for (definition, operation) in [
        ("module Crypto do pub fn xchacha20poly1305_seal(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Bytes) -> Bytes!CryptoError do Ok(body) end end", "Crypto.xchacha20poly1305_seal"),
        ("fn crypto_xchacha20poly1305_seal(key :: borrow AeadKey,nonce :: Bytes,aad :: Bytes,body :: Bytes) -> Bytes!CryptoError do Ok(body) end", "crypto_xchacha20poly1305_seal"),
    ] {
        assert!(check(definition).errors.is_empty(), "{definition}");
        let source = format!("{definition}\nfn leak(key :: borrow AeadKey,body :: Plaintext<Bytes>) do\nlet result = declassify_discriminant({operation}(key,Bytes.empty(),Bytes.empty(),body),\"test-only result tag\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(result,Bytes.empty())) end");
        let result = check(&source);
        assert!(result.errors.iter().any(|e| matches!(e, TypeError::PlaintextViolation { .. })), "{source}: {:?}", result.errors);
        assert!(result.errors.iter().all(|e| matches!(e, TypeError::PlaintextViolation { .. })), "unrelated refusal: {:?}", result.errors);
    }
}
