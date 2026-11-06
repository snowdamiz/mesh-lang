//! Only successful ciphertext is an encrypted exit; failures retain input facts.
use mesh_typeck::error::TypeError;

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check(&parse)
}

#[test]
fn seal_error_payloads_tags_and_public_returns_retain_private_length() {
    let sources = [
        "fn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do case Crypto.aead_seal(key,nonce,Bytes.empty(),body) do Err(InvalidLength(_,actual)) -> println(Int.to_string(actual))\n_ -> nil end end",
        "fn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do case Crypto.aead_seal(key,nonce,Bytes.empty(),body) do Err(InvalidLength(_,actual)) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),Bytes.from_utf8(Int.to_string(actual)))\n_ -> 0 end end",
        "fn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do if Result.is_err(Crypto.aead_seal(key,nonce,Bytes.empty(),body)) do println(\"private failure\") end end",
        "fn leak(body :: Plaintext<Bytes>, key :: borrow StorageKey, context :: Bytes) do case StorageKey.seal_bytes(body,key,context) do Err(InvalidLength(_,actual)) -> println(Int.to_string(actual))\n_ -> nil end end",
        "fn leak(body :: Plaintext<Bytes>, key :: X25519PublicKey) do if Result.is_ok(Crypto.hpke_seal(key,Bytes.empty(),Bytes.empty(),body)) do println(\"private success\") end end",
        "fn encrypt(body :: Bytes, key :: borrow AeadKey, nonce :: Bytes) -> Result<Bytes,CryptoError> do Crypto.aead_seal(key,nonce,Bytes.empty(),body) end\nfn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do case encrypt(body,key,nonce) do Err(InvalidLength(_,actual)) -> println(Int.to_string(actual))\n_ -> nil end end",
        "@export(\"private_error_return\") pub fn leak(input :: Bytes) -> Bytes!String do let body :: Plaintext<Bytes> = input\nlet material = case Secret.random(32) do Err(_) -> return Err(\"key\")\nOk(value) -> value end\nlet key = case Crypto.aead_key(material) do Err(_) -> return Err(\"key\")\nOk(value) -> value end\ncase Crypto.aead_seal(key,Bytes.from_utf8(\"123456789012\"),Bytes.empty(),body) do Err(InvalidLength(_,actual)) -> Ok(Bytes.from_utf8(Int.to_string(actual)))\nErr(_) -> Err(\"seal\")\nOk(value) -> Ok(value) end end",
        "fn leak(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Int,CryptoError> do let ciphertext = Crypto.aead_seal(key,nonce,Bytes.empty(),body)?\nOk(Http.body_bytes(Http.build(:post,\"https://example.test\"),ciphertext)) end",
    ];
    let mut escaped = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let result = checked(source);
        assert!(
            result
                .errors
                .iter()
                .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
            "unrelated errors in case {index}: {:?}",
            result.errors
        );
        if result.errors.is_empty() {
            escaped.push(index);
        }
    }
    assert!(
        escaped.is_empty(),
        "seal failure facts escaped in cases {escaped:?}"
    );
}

#[test]
fn known_public_encoded_size_keeps_ciphertext_transport_allowed() {
    for seal in [
        "Crypto.aead_seal(key,nonce,Bytes.empty(),body)",
        "Crypto.hpke_seal(public_key,Bytes.empty(),Bytes.empty(),body)",
        "StorageKey.seal_bytes(body,storage,context)",
    ] {
        let source = format!("fn encrypt(value :: Plaintext<U64>, key :: borrow AeadKey, nonce :: Bytes, public_key :: X25519PublicKey, storage :: borrow StorageKey, context :: Bytes) -> Result<Int,CryptoError> do let body = case Bytes.write_u64_be(value) do Err(_) -> return Err(InternalFailure)\nOk(bytes) -> bytes end\nlet ciphertext = {seal}?\nOk(Http.body_bytes(Http.build(:post,\"https://example.test\"),ciphertext)) end");
        let result = checked(&source);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }
}
