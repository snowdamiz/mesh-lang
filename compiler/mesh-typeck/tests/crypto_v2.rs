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

#[test]
fn file_storage_key_is_an_affine_string_to_crypto_result() {
    let result = check_source(
        "fn open(directory :: String) -> StorageKey ! CryptoError do StorageKey.file(directory) end\n\
         fn raw(directory :: String) -> StorageKey ! CryptoError do storage_key_file(directory) end",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    for source in [
        "fn bad() do StorageKey.file(Bytes.empty()) end",
        "fn bad(path :: String) -> StorageKey do StorageKey.file(path) end",
        "fn bad(path :: String) do let key = StorageKey.file(path)?\nSecret.destroy(key)\nSecret.destroy(key) end",
    ] {
        assert!(!check_source(source).errors.is_empty(), "accepted {source}");
    }
}

#[test]
fn ed25519_curve_membership_is_a_public_bytes_to_boolean_result() {
    let result = check_source(
        r#"
fn point(value :: Bytes) -> Result<Bool, CryptoError> do
  Crypto.ed25519_is_on_curve(value)
end
fn raw(value :: Bytes) -> Result<Bool, CryptoError> do
  crypto_ed25519_is_on_curve(value)
end
fn indirect(value :: Bytes) -> Result<Bool, CryptoError> do
  let check = Crypto.ed25519_is_on_curve
  check(value)
end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    for source in [
        "fn wrong() do Crypto.ed25519_is_on_curve(\"key\") end",
        "fn wrong(key :: SigningPublicKey) do Crypto.ed25519_is_on_curve(key) end",
        "fn wrong(key :: Bytes) -> Bool do Crypto.ed25519_is_on_curve(key) end",
    ] {
        assert!(!check_source(source).errors.is_empty());
    }
}

#[test]
fn signing_seed_constructor_is_typed_as_an_affine_key_pair_result() {
    let result = check_source(
        r#"
fn seeded(value :: Bytes) -> Result<SigningKeyPair, CryptoError> do
  Crypto.signing_from_seed(value)
end
fn seeded_raw(value :: Bytes) -> Result<SigningKeyPair, CryptoError> do
  crypto_signing_from_seed(value)
end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn x25519_seed_constructor_is_typed_as_an_affine_key_pair_result() {
    let result = check_source(
        r#"
fn seeded(value :: Bytes) -> Result<X25519KeyPair, CryptoError> do
  Crypto.x25519_from_seed(value)
end
fn seeded_raw(value :: Bytes) -> Result<X25519KeyPair, CryptoError> do
  crypto_x25519_from_seed(value)
end
fn from_secret(value :: SecretBytes) -> Result<X25519KeyPair, CryptoError> do
  Crypto.x25519_from_secret(value)
end
fn from_secret_raw(value :: SecretBytes) -> Result<X25519KeyPair, CryptoError> do
  crypto_x25519_from_secret(value)
end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn secret_key_constructors_consume_private_material() {
    let result = check_source(
        r#"
fn qualified(material :: SecretBytes) do
  Crypto.x25519_from_secret(material)
  Secret.destroy(material)
end
fn prefixed(material :: SecretBytes) do
  crypto_x25519_from_secret(material)
  Secret.destroy(material)
end
fn signing_qualified(material :: SecretBytes) do
  Crypto.signing_from_secret(material)
  Secret.destroy(material)
end
fn signing_prefixed(material :: SecretBytes) do
  crypto_signing_from_secret(material)
  Secret.destroy(material)
end
fn mlkem_qualified(material :: SecretBytes) do
  Crypto.mlkem_from_secret(material)
  Secret.destroy(material)
end
fn mlkem_prefixed(material :: SecretBytes) do
  crypto_mlkem_from_secret(material)
  Secret.destroy(material)
end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        [
            "resource `material` was used after it moved",
            "resource `material` was used after it moved",
            "resource `material` was used after it moved",
            "resource `material` was used after it moved",
            "resource `material` was used after it moved",
            "resource `material` was used after it moved",
        ]
    );
}

#[test]
fn argon2id_returns_an_affine_secret_and_borrows_the_password() {
    let result = check_source(
        r#"
fn derive(password :: SecretBytes, salt :: Bytes) -> Result<SecretBytes, CryptoError> do
  Crypto.argon2id(password, salt, 32, 3, 1, 32)
  crypto_argon2id(password, salt, 32, 3, 1, 32)
end
"#,
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn mlkem_private_keys_can_be_sealed_and_restored_through_the_public_module() {
    let result = check_source(
        r#"
fn persist(key :: borrow MlKemPrivateKey, wrapping_key :: borrow StorageKey, context :: Bytes) -> Result<Bytes, CryptoError> do
  MlKemPrivateKey.seal_for_storage(key, wrapping_key, context)
end
fn restore(blob :: Bytes, wrapping_key :: borrow StorageKey, context :: Bytes) -> Result<MlKemPrivateKey, CryptoError> do
  MlKemPrivateKey.unseal_from_storage(blob, wrapping_key, context)
end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
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
            "UnsupportedTarget",
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
        ("MlKemPublicKey", vec![("bytes", Ty::bytes())]),
        ("MlKemCiphertext", vec![("bytes", Ty::bytes())]),
        ("SigningPublicKey", vec![("bytes", Ty::bytes())]),
        ("Signature", vec![("bytes", Ty::bytes())]),
        ("BlindRsaPublicKey", vec![("bytes", Ty::bytes())]),
        (
            "BlindRsaBlinded",
            vec![
                ("blinded", Ty::bytes()),
                ("state", con("BlindRsaBlindingState")),
            ],
        ),
        (
            "X25519KeyPair",
            vec![
                ("private_key", con("X25519PrivateKey")),
                ("public_key", con("X25519PublicKey")),
            ],
        ),
        (
            "MlKemKeyPair",
            vec![
                ("private_key", con("MlKemPrivateKey")),
                ("public_key", con("MlKemPublicKey")),
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
        "MlKemPrivateKey",
        "SigningPrivateKey",
        "AeadKey",
        "BlindRsaSecretKey",
        "BlindRsaBlindingState",
        "BlindRsaBlinded",
        "X25519KeyPair",
        "MlKemKeyPair",
        "SigningKeyPair",
    ] {
        assert!(
            registry.is_resource_name(resource),
            "{resource} must be affine"
        );
    }
    for value in [
        "X25519PublicKey",
        "MlKemPublicKey",
        "MlKemCiphertext",
        "SigningPublicKey",
        "Signature",
        "BlindRsaPublicKey",
    ] {
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
fn mlkem768_public_round_trip_signature_typechecks() {
    let result = check_source(
        r#"
fn round_trip() -> Result<SecretBytes, CryptoError> do
  let pair = Crypto.mlkem_generate()?
  let (ciphertext, sender_secret) = Crypto.mlkem_encapsulate(pair.public_key)?
  Secret.destroy(sender_secret)
  Crypto.mlkem_decapsulate(pair.private_key, ciphertext)
end

fn deterministic(seed :: Bytes) -> Result<MlKemKeyPair, CryptoError> do
  Crypto.mlkem_from_seed(seed)
end
"#,
    );

    assert!(
        result.errors.is_empty(),
        "unexpected ML-KEM signature errors: {:?}",
        result.errors
    );
}

#[test]
fn hpke_base_mode_public_signatures_typecheck() {
    let result = check_source(
        r#"
fn seal(recipient :: X25519PublicKey, info :: Bytes, aad :: Bytes, plaintext :: Bytes) -> Result<Bytes, CryptoError> do
  Crypto.hpke_seal(recipient, info, aad, plaintext)
end
fn open(private_key :: borrow X25519PrivateKey, info :: Bytes, aad :: Bytes, sealed :: Bytes) -> Result<Bytes, CryptoError> do
  Crypto.hpke_open(private_key, info, aad, sealed)
end
"#,
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn hpke_secret_signatures_keep_plaintext_actor_owned() {
    let result = check_source(
        r#"
fn seal(recipient :: X25519PublicKey, info :: Bytes, aad :: Bytes, plaintext :: borrow SecretBytes) -> Result<Bytes, CryptoError> do
  Crypto.hpke_seal_secret(recipient, info, aad, plaintext)
end
fn open(private_key :: borrow X25519PrivateKey, info :: Bytes, aad :: Bytes, sealed :: Bytes) -> Result<SecretBytes, CryptoError> do
  Crypto.hpke_open_secret(private_key, info, aad, sealed)
end
"#,
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
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

#[test]
fn blind_rsa_public_signatures_and_persistent_resources_typecheck() {
    for prefix in ["Crypto.", "crypto_"] {
        let source = format!(
            r#"
fn generate() -> BlindRsaSecretKey ! CryptoError do {prefix}blind_rsa_generate() end
fn import_key(material :: SecretBytes) -> BlindRsaSecretKey ! CryptoError do {prefix}blind_rsa_from_secret(material) end
fn public(key :: borrow BlindRsaSecretKey) -> BlindRsaPublicKey ! CryptoError do {prefix}blind_rsa_public(key) end
fn parse_key(spki :: Bytes) -> BlindRsaPublicKey ! CryptoError do {prefix}blind_rsa_public_from_spki(spki) end
fn blind(key :: BlindRsaPublicKey, message :: Bytes) -> BlindRsaBlinded ! CryptoError do {prefix}blind_rsa_blind(key, message) end
fn sign(key :: borrow BlindRsaSecretKey, message :: Bytes) -> Bytes ! CryptoError do {prefix}blind_rsa_sign(key, message) end
fn finalize(key :: BlindRsaPublicKey, message :: Bytes, signature :: Bytes, state :: BlindRsaBlindingState) -> Bytes ! CryptoError do {prefix}blind_rsa_finalize(key, message, signature, state) end
fn verify(key :: BlindRsaPublicKey, message :: Bytes, signature :: Bytes) -> Bool ! CryptoError do {prefix}blind_rsa_verify(key, message, signature) end
fn reuse(key :: borrow BlindRsaSecretKey, message :: Bytes) do
  {prefix}blind_rsa_public(key)
  {prefix}blind_rsa_sign(key, message)
  {prefix}blind_rsa_public(key)
end
fn indirect(key :: BlindRsaPublicKey, message :: Bytes, signature :: Bytes) -> Bool ! CryptoError do
  let verify = {prefix}blind_rsa_verify
  verify(key, message, signature)
end
"#
        );
        let result = check_source(&source);
        assert!(result.errors.is_empty(), "{prefix}: {:?}", result.errors);
    }
    for (module, prefix) in [
        ("BlindRsaSecretKey", "blind_rsa_secret_key"),
        ("BlindRsaBlindingState", "blind_rsa_blinding_state"),
    ] {
        for function_prefix in [format!("{module}."), format!("{prefix}_")] {
            let source = format!(
                r#"
fn persist(value :: borrow {module}, wrapping_key :: borrow StorageKey, context :: Bytes) -> Bytes ! CryptoError do
  {function_prefix}seal_for_storage(value, wrapping_key, context)
  {function_prefix}seal_for_storage(value, wrapping_key, context)
end
fn restore(blob :: Bytes, wrapping_key :: borrow StorageKey, context :: Bytes) -> {module} ! CryptoError do
  {function_prefix}unseal_from_storage(blob, wrapping_key, context)
end
"#
            );
            let result = check_source(&source);
            assert!(
                result.errors.is_empty(),
                "{function_prefix}: {:?}",
                result.errors
            );
        }
    }
}

#[test]
fn blind_rsa_private_import_and_finalize_consume_on_qualified_and_flat_calls() {
    for prefix in ["Crypto.", "crypto_"] {
        let result = check_source(&format!(
            r#"
fn import_twice(material :: SecretBytes) do
  {prefix}blind_rsa_from_secret(material)
  {prefix}blind_rsa_from_secret(material)
end
fn finalize_twice(key :: BlindRsaPublicKey, message :: Bytes, signature :: Bytes, state :: BlindRsaBlindingState) do
  {prefix}blind_rsa_finalize(key, message, signature, state)
  {prefix}blind_rsa_finalize(key, message, signature, state)
end
fn borrowed_state(key :: BlindRsaPublicKey, message :: Bytes, signature :: Bytes, state :: borrow BlindRsaBlindingState) do
  {prefix}blind_rsa_finalize(key, message, signature, state)
end
"#
        ));
        assert_eq!(
            resource_violations(&result),
            [
                "resource `material` was used after it moved",
                "resource `state` was used after it moved",
                "borrowed resource `state` cannot be moved",
            ],
            "{prefix}: {:?}",
            result.errors
        );
    }
}

#[test]
fn blind_rsa_result_aggregate_and_projected_state_are_affine() {
    let result = check_source(
        r#"
fn take(value :: consume BlindRsaBlinded) do nil end
fn take_state(value :: consume BlindRsaBlindingState) do nil end
fn duplicate(key :: BlindRsaPublicKey, message :: Bytes) do
  case Crypto.blind_rsa_blind(key, message) do
    Ok(blinded) -> (take(blinded), take(blinded))
    Err(_) -> (nil, nil)
  end
end
fn duplicate_state(blinded :: BlindRsaBlinded) do
  take_state(blinded.state)
  take_state(blinded.state)
end
"#,
    );
    let violations = resource_violations(&result);
    assert_eq!(violations.len(), 2, "{:?}", result.errors);
    assert!(
        violations[0].contains("used after it moved"),
        "{violations:?}"
    );
    assert!(
        violations[1].contains("used after it moved"),
        "{violations:?}"
    );
}

#[test]
fn blind_rsa_rejects_fabrication_exposure_and_wrong_argument_order() {
    for source in [
        "fn bad(value :: Bytes) do Crypto.blind_rsa_from_secret(value) end",
        "fn bad(key :: BlindRsaSecretKey) -> Bytes do key end",
        "fn bad(state :: BlindRsaBlindingState) -> Bytes do state end",
        "fn bad() do BlindRsaSecretKey {} end",
        "fn bad() do BlindRsaBlindingState {} end",
        "fn bad(key :: BlindRsaPublicKey, message :: Bytes, signature :: Bytes, state :: BlindRsaBlindingState) do Crypto.blind_rsa_finalize(key, message, state, signature) end",
        "fn bad(key :: BlindRsaPublicKey, message :: Bytes, signature :: Bytes) -> Bool do Crypto.blind_rsa_verify(key, message, signature) end",
    ] {
        assert!(!check_source(source).errors.is_empty(), "accepted: {source}");
    }
}

#[test]
fn blind_rsa_request_bytes_and_sealed_state_can_precede_finalize() {
    let result = check_source(
        r#"
fn round_trip(key :: borrow BlindRsaSecretKey, message :: Bytes, wrapping :: borrow StorageKey) -> Bytes ! CryptoError do
  let public = Crypto.blind_rsa_public(key)?
  let blinded = Crypto.blind_rsa_blind(public, message)?
  let request = blinded.blinded
  let _recovery = BlindRsaBlindingState.seal_for_storage(blinded.state, wrapping, message)?
  let signature = Crypto.blind_rsa_sign(key, request)?
  Crypto.blind_rsa_finalize(public, message, signature, blinded.state)
end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn secret_destroy_accepts_only_the_builtin_opaque_secret_handle_family() {
    for resource in [
        "SecretBytes",
        "X25519PrivateKey",
        "SigningPrivateKey",
        "AeadKey",
        "StorageKey",
        "SecretMap",
        "MlKemPrivateKey",
        "BlindRsaSecretKey",
        "BlindRsaBlindingState",
    ] {
        for call in [
            "Secret.destroy(value)",
            "secret_destroy(value)",
            "value |> Secret.destroy",
        ] {
            let source = format!("fn discard(value :: {resource}) do {call} end");
            let result = check_source(&source);
            assert!(result.errors.is_empty(), "{source}: {:?}", result.errors);
        }
    }
    for source in [
        "fn bad(value :: Bytes) do Secret.destroy(value) end",
        "fn bad(value :: BlindRsaBlinded) do Secret.destroy(value) end",
        "fn bad(value :: X25519KeyPair) do secret_destroy(value) end",
        "fn bad(value :: BytesBuilder) do secret_destroy(value) end",
        "fn bad(value :: PgConn) do Secret.destroy(value) end",
        "resource Custom\nfn bad(value :: Custom) do Secret.destroy(value) end",
        "fn bad(value :: BlindRsaSecretKey) do let destroy = Secret.destroy\ndestroy(value) end",
    ] {
        assert!(
            !check_source(source).errors.is_empty(),
            "accepted: {source}"
        );
    }
}

#[test]
fn secret_destroy_consumes_blind_rsa_handles_and_rejects_borrows() {
    let result = check_source(
        r#"
fn duplicate(key :: BlindRsaSecretKey) do Secret.destroy(key)
secret_destroy(key) end
fn borrow_state(state :: borrow BlindRsaBlindingState) do Secret.destroy(state) end
"#,
    );
    assert_eq!(
        resource_violations(&result),
        [
            "resource `key` was used after it moved",
            "borrowed resource `state` cannot be moved"
        ],
        "{:?}",
        result.errors
    );
}
