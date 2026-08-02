//! Type-system and affine-ownership contract tests for Mesh Crypto V2.

use mesh_typeck::error::TypeError;
use mesh_typeck::infer::VariantFieldInfo;
use mesh_typeck::ty::{Ty, TyCon};
use mesh_typeck::TypeckResult;

fn check_source(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "parse errors: {:?}", parse.errors());
    mesh_typeck::check(&parse)
}

fn resource_violations(result: &TypeckResult) -> Vec<&str> {
    result
        .errors
        .iter()
        .filter_map(|error| match error {
            TypeError::ResourceViolation { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect()
}

fn con(name: &str) -> Ty {
    Ty::Con(TyCon::new(name))
}

#[test]
fn crypto_error_uses_the_public_plan_order_and_payload() {
    let result = check_source("");
    let definition = result
        .type_registry
        .sum_type_defs
        .get("CryptoError")
        .expect("CryptoError must be a builtin sum type");

    assert!(definition.generic_params.is_empty());
    assert_eq!(
        definition
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        [
            "InvalidLength",
            "InvalidKey",
            "InvalidPublicKey",
            "InvalidSignature",
            "AuthenticationFailed",
            "EntropyUnavailable",
            "SecretDestroyed",
            "ResourceLimitExceeded",
            "UnsupportedOperation",
            "InternalFailure",
        ]
    );
    assert!(matches!(
        definition.variants[0].fields.as_slice(),
        [
            VariantFieldInfo::Named(expected, expected_ty),
            VariantFieldInfo::Named(actual, actual_ty),
        ] if expected == "expected"
            && actual == "actual"
            && *expected_ty == Ty::int()
            && *actual_ty == Ty::int()
    ));
    assert!(definition
        .variants
        .iter()
        .skip(1)
        .all(|variant| variant.fields.is_empty()));
}

#[test]
fn crypto_public_structs_have_exact_fields_and_keypairs_are_affine() {
    let result = check_source("");
    let registry = &result.type_registry;

    for (name, fields) in [
        ("X25519PublicKey", vec![("bytes", Ty::bytes())]),
        ("SigningPublicKey", vec![("bytes", Ty::bytes())]),
        ("Signature", vec![("bytes", Ty::bytes())]),
        (
            "X25519KeyPair",
            vec![
                ("private_key", con("X25519PrivateKey")),
                ("public_key", con("X25519PublicKey")),
            ],
        ),
        (
            "SigningKeyPair",
            vec![
                ("private_key", con("SigningPrivateKey")),
                ("public_key", con("SigningPublicKey")),
            ],
        ),
    ] {
        let definition = registry
            .struct_defs
            .get(name)
            .unwrap_or_else(|| panic!("missing builtin struct {name}"));
        assert!(definition.generic_params.is_empty());
        assert_eq!(
            definition
                .fields
                .iter()
                .map(|(field, ty)| (field.as_str(), ty.clone()))
                .collect::<Vec<_>>(),
            fields
        );
    }

    for resource in [
        "SecretBytes",
        "X25519PrivateKey",
        "SigningPrivateKey",
        "AeadKey",
        "X25519KeyPair",
        "SigningKeyPair",
    ] {
        assert!(
            registry.is_resource_name(resource),
            "{resource} must be affine"
        );
    }
    for value in ["X25519PublicKey", "SigningPublicKey", "Signature"] {
        assert!(
            !registry.is_resource_name(value),
            "{value} must remain an ordinary value"
        );
    }
}

#[test]
fn crypto_public_value_structs_work_in_source_literals_and_calls() {
    let result = check_source(
        r#"
fn use_wrappers(private_key :: borrow X25519PrivateKey, message :: Bytes) -> Result<Bool, CryptoError> do
  let peer = X25519PublicKey { bytes: Bytes.empty() }
  let signing_key = SigningPublicKey { bytes: Bytes.empty() }
  let signature = Signature { bytes: Bytes.empty() }
  let peer_bytes :: Bytes = peer.bytes
  let signing_bytes :: Bytes = signing_key.bytes
  let signature_bytes :: Bytes = signature.bytes
  Crypto.x25519_shared(private_key, peer)
  Crypto.verify(signing_key, message, signature)
end
"#,
    );

    assert!(
        result.errors.is_empty(),
        "synthetic public structs were not usable from Mesh source: {:?}",
        result.errors
    );
}

#[test]
fn qualified_and_prefixed_crypto_v2_signatures_typecheck() {
    let result = check_source(
        r#"
fn hash(input :: Bytes) -> Bytes do Crypto.sha256(input) end
fn hash_raw(input :: Bytes) -> Bytes do crypto_sha256(input) end
fn hash512(input :: Bytes) -> Bytes do Crypto.sha512(input) end
fn hash512_raw(input :: Bytes) -> Bytes do crypto_sha512(input) end
fn hash_hex(input :: Bytes) -> String do Crypto.sha256_hex(input) end
fn hash_hex_raw(input :: Bytes) -> String do crypto_sha256_hex(input) end
fn hash512_hex(input :: Bytes) -> String do Crypto.sha512_hex(input) end
fn hash512_hex_raw(input :: Bytes) -> String do crypto_sha512_hex(input) end
fn random_bytes() -> Result<Bytes, CryptoError> do Crypto.random_bytes(32) end
fn random_bytes_raw() -> Result<Bytes, CryptoError> do crypto_random_bytes(32) end

fn hmac(key :: borrow SecretBytes, message :: Bytes) -> Result<SecretBytes, CryptoError> do
  Crypto.hmac_sha256(key, message)
end
fn hmac_raw(key :: borrow SecretBytes, message :: Bytes) -> Result<SecretBytes, CryptoError> do
  crypto_hmac_sha256(key, message)
end
fn hkdf(key :: borrow SecretBytes, salt :: Bytes, info :: Bytes) -> Result<SecretBytes, CryptoError> do
  Crypto.hkdf_sha256(key, salt, info, 32)
end
fn hkdf_raw(key :: borrow SecretBytes, salt :: Bytes, info :: Bytes) -> Result<SecretBytes, CryptoError> do
  crypto_hkdf_sha256(key, salt, info, 32)
end

fn x_generate() -> Result<X25519KeyPair, CryptoError> do Crypto.x25519_generate() end
fn x_generate_raw() -> Result<X25519KeyPair, CryptoError> do crypto_x25519_generate() end
fn x_public(key :: borrow X25519PrivateKey) -> Result<X25519PublicKey, CryptoError> do
  Crypto.x25519_public(key)
end
fn x_public_raw(key :: borrow X25519PrivateKey) -> Result<X25519PublicKey, CryptoError> do
  crypto_x25519_public(key)
end
fn x_shared(key :: borrow X25519PrivateKey, peer :: X25519PublicKey) -> Result<SecretBytes, CryptoError> do
  Crypto.x25519_shared(key, peer)
end
fn x_shared_raw(key :: borrow X25519PrivateKey, peer :: X25519PublicKey) -> Result<SecretBytes, CryptoError> do
  crypto_x25519_shared(key, peer)
end

fn signing_generate() -> Result<SigningKeyPair, CryptoError> do Crypto.signing_generate() end
fn signing_generate_raw() -> Result<SigningKeyPair, CryptoError> do crypto_signing_generate() end
fn sign(key :: borrow SigningPrivateKey, message :: Bytes) -> Result<Signature, CryptoError> do
  Crypto.sign(key, message)
end
fn sign_raw(key :: borrow SigningPrivateKey, message :: Bytes) -> Result<Signature, CryptoError> do
  crypto_sign(key, message)
end
fn verify(key :: SigningPublicKey, message :: Bytes, signature :: Signature) -> Result<Bool, CryptoError> do
  Crypto.verify(key, message, signature)
end
fn verify_raw(key :: SigningPublicKey, message :: Bytes, signature :: Signature) -> Result<Bool, CryptoError> do
  crypto_verify(key, message, signature)
end

fn aead_key(material :: SecretBytes) -> Result<AeadKey, CryptoError> do Crypto.aead_key(material) end
fn aead_key_raw(material :: SecretBytes) -> Result<AeadKey, CryptoError> do crypto_aead_key(material) end
fn seal(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, plaintext :: Bytes) -> Result<Bytes, CryptoError> do
  Crypto.aead_seal(key, nonce, aad, plaintext)
end
fn seal_raw(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, plaintext :: Bytes) -> Result<Bytes, CryptoError> do
  crypto_aead_seal(key, nonce, aad, plaintext)
end
fn open(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, ciphertext :: Bytes) -> Result<Bytes, CryptoError> do
  Crypto.aead_open(key, nonce, aad, ciphertext)
end
fn open_raw(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, ciphertext :: Bytes) -> Result<Bytes, CryptoError> do
  crypto_aead_open(key, nonce, aad, ciphertext)
end
"#,
    );

    assert!(
        result.errors.is_empty(),
        "unexpected Crypto V2 signature errors: {:?}",
        result.errors
    );
}

#[test]
fn colliding_legacy_string_calls_are_rejected() {
    let result = check_source(
        r#"
fn bad_sha() do Crypto.sha256("hello") end
fn bad_sha_raw() do crypto_sha256("hello") end
fn bad_sha512() do Crypto.sha512("hello") end
fn bad_sha512_raw() do crypto_sha512("hello") end
fn bad_hmac() do Crypto.hmac_sha256("key", "message") end
fn bad_hmac_raw() do crypto_hmac_sha256("key", "message") end
"#,
    );

    let mismatch_count = result
        .errors
        .iter()
        .filter(|error| {
            matches!(
                error,
                TypeError::Mismatch { expected, found, .. }
                    if found == &Ty::string()
                        && matches!(expected, Ty::Con(name) if matches!(name.name.as_str(), "Bytes" | "SecretBytes"))
            )
        })
        .count();
    assert_eq!(mismatch_count, 6, "unexpected errors: {:?}", result.errors);
}

#[test]
fn qualified_and_prefixed_borrowing_calls_are_reusable() {
    let result = check_source(
        r#"
fn secret_borrows(key :: SecretBytes, message :: Bytes) -> Result<SecretBytes, CryptoError> do
  Crypto.hmac_sha256(key, message)
  crypto_hkdf_sha256(key, message, message, 32)
end
fn x25519_borrows(key :: X25519PrivateKey, peer :: X25519PublicKey) -> Result<X25519PublicKey, CryptoError> do
  Crypto.x25519_shared(key, peer)
  crypto_x25519_public(key)
end
fn signing_borrows(key :: SigningPrivateKey, message :: Bytes) -> Result<Signature, CryptoError> do
  Crypto.sign(key, message)
  crypto_sign(key, message)
end
fn aead_borrows(key :: AeadKey, nonce :: Bytes, aad :: Bytes, body :: Bytes) -> Result<Bytes, CryptoError> do
  Crypto.aead_seal(key, nonce, aad, body)
  crypto_aead_open(key, nonce, aad, body)
end
"#,
    );

    assert!(
        resource_violations(&result).is_empty(),
        "borrow calls moved a resource: {:?}",
        result.errors
    );
}

#[test]
fn aead_key_is_consume_for_qualified_and_prefixed_calls() {
    let result = check_source(
        r#"
fn qualified(material :: borrow SecretBytes) do Crypto.aead_key(material) end
fn prefixed(material :: borrow SecretBytes) do crypto_aead_key(material) end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        [
            "borrowed resource `material` cannot be moved",
            "borrowed resource `material` cannot be moved",
        ]
    );
}

#[test]
fn use_after_aead_key_consume_is_rejected() {
    let result = check_source(
        r#"
fn qualified(material :: SecretBytes) do
  Crypto.aead_key(material)
  Secret.destroy(material)
end
fn prefixed(material :: SecretBytes) do
  crypto_aead_key(material)
  Secret.destroy(material)
end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        [
            "resource `material` was used after it moved",
            "resource `material` was used after it moved",
        ]
    );
}

#[test]
fn crypto_resource_results_pattern_bind_owned_values() {
    let result = check_source(
        r#"
fn take_aead(key :: consume AeadKey) do nil end
fn take_pair(pair :: consume X25519KeyPair) do nil end
fn misuse(material :: SecretBytes) do
  case Crypto.aead_key(material) do
    Ok(key) -> (take_aead(key), take_aead(key))
    Err(_) -> (nil, nil)
  end
end
fn misuse_pair() do
  case Crypto.x25519_generate() do
    Ok(pair) -> (take_pair(pair), take_pair(pair))
    Err(_) -> (nil, nil)
  end
end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        [
            "resource `key` was used after it moved",
            "resource `pair` was used after it moved",
        ]
    );
    assert!(
        !resource_violations(&result)
            .iter()
            .any(|reason| reason.contains("unsupported")),
        "Result<R, CryptoError> must be accepted for resource outputs"
    );
}
