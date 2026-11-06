//! OHTTP plaintext-flow and genuine-builtin summary contracts.
use mesh_typeck::{error::TypeError, ImportContext, ModuleExports};

fn check(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{source}\n{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

fn private_refusal(source: &str) {
    let result = check(source);
    assert!(
        result
            .errors
            .iter()
            .any(|e| matches!(e, TypeError::PlaintextViolation { .. })),
        "escaped: {source}\n{:?}",
        result.errors
    );
    assert!(
        result
            .errors
            .iter()
            .all(|e| matches!(e, TypeError::PlaintextViolation { .. })),
        "unrelated failure: {source}\n{:?}",
        result.errors
    );
}

#[test]
fn ohttp_only_successful_ciphertext_field_can_cross_a_reviewed_finite_status_boundary() {
    for prefix in ["Ohttp.", "ohttp_"] {
        let source = format!(
            r#"
fn send_packet(key :: X25519PublicKey, body :: Plaintext<Bytes>) do
  let result = declassify_discriminant({prefix}encapsulate_request(37, key, body), "test-only finite OHTTP encryption status")
  case result do
    Ok(packet) -> Http.body_bytes(Http.build(:post, "https://example.test"), packet.wire)
    Err(_) -> Http.body_bytes(Http.build(:post, "https://example.test"), Bytes.empty())
  end
end
"#
        );
        let result = check(&source);
        assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
    }
}

#[test]
fn ohttp_successful_response_ciphertext_can_leave_after_only_finite_status_review() {
    for prefix in ["Ohttp.", "ohttp_"] {
        let source = format!(
            r#"
fn send_packet(context :: OhttpGatewayContext, body :: Plaintext<Bytes>) do
  let encrypted = declassify_discriminant({prefix}encapsulate_response(context, body), "test-only finite response encryption status")
  Http.body_bytes(Http.build(:post, "https://example.test"), Result.unwrap_or(encrypted, Bytes.empty()))
end
"#
        );
        let result = check(&source);
        assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
        private_refusal(&format!(
            r#"
fn leak(context :: OhttpGatewayContext, body :: Plaintext<Bytes>) do
  let encrypted = {prefix}encapsulate_response(context, body)
  println(String.from(Result.is_ok(encrypted)))
end
"#
        ));
        private_refusal(&format!(
            r#"
fn leak(context :: OhttpGatewayContext, body :: Plaintext<Bytes>) do
  let encrypted = declassify_discriminant({prefix}encapsulate_response(context, body), "test-only response tag")
  case encrypted do
    Err(InvalidLength(_, actual)) -> println(String.from(actual))
    Err(_) -> nil
    Ok(_) -> nil
  end
end
"#
        ));
        private_refusal(&format!(
            r#"
fn leak(context :: OhttpGatewayContext, private :: Plaintext<Bool>) do
  if private do
    let encrypted = declassify_discriminant({prefix}encapsulate_response(context, Bytes.empty()), "test-only response tag cannot erase caller control")
    let _ = Http.body_bytes(Http.build(:post, "https://example.test"), Result.unwrap_or(encrypted, Bytes.empty()))
    nil
  end
end
"#
        ));
    }
}

#[test]
fn ohttp_finite_tag_review_does_not_release_private_error_length_payloads() {
    for prefix in ["Ohttp.", "ohttp_"] {
        private_refusal(&format!(
            r#"
fn leak(key :: X25519PublicKey, body :: Plaintext<Bytes>) do
  let result = declassify_discriminant({prefix}encapsulate_request(37, key, body), "test-only finite OHTTP status")
  case result do
    Err(InvalidLength(_, actual)) -> println(String.from(actual))
    Err(_) -> nil
    Ok(_) -> nil
  end
end
"#
        ));
    }
}

#[test]
fn ohttp_private_key_id_recipient_metadata_and_caller_control_are_not_ciphertext() {
    for prefix in ["Ohttp.", "ohttp_"] {
        for (parameters, key_id, recipient) in [
            (
                "key :: X25519PublicKey, private :: Plaintext<Int>",
                "private",
                "key",
            ),
            (
                "private :: Plaintext<Bytes>",
                "37",
                "X25519PublicKey { bytes: private }",
            ),
        ] {
            private_refusal(&format!(
                r#"
fn leak({parameters}) do
  let result = declassify_discriminant({prefix}encapsulate_request({key_id}, {recipient}, Bytes.empty()), "test-only finite status")
  case result do
    Ok(packet) -> Http.body_bytes(Http.build(:post, "https://example.test"), packet.wire)
    Err(_) -> Http.body_bytes(Http.build(:post, "https://example.test"), Bytes.empty())
  end
end
"#
            ));
        }
        private_refusal(&format!(
            r#"
fn leak(key :: X25519PublicKey, private :: Plaintext<Bool>) do
  if private do
    let result = declassify_discriminant({prefix}encapsulate_request(37, key, Bytes.empty()), "test-only status cannot erase caller control")
    let _ = case result do
      Ok(packet) -> Http.body_bytes(Http.build(:post, "https://example.test"), packet.wire)
      Err(_) -> Http.body_bytes(Http.build(:post, "https://example.test"), Bytes.empty())
    end
    nil
  end
end
"#
        ));
    }
}

#[test]
fn ohttp_opening_payload_and_tag_remain_private_through_named_forwarding() {
    for prefix in ["Ohttp.", "ohttp_"] {
        private_refusal(&format!(
            r#"
fn forward(key :: borrow X25519PrivateKey, body :: Bytes) -> OhttpRequestOpening!CryptoError do
  {prefix}decapsulate_request(key, 37, body)
end
fn leak(key :: borrow X25519PrivateKey, body :: Plaintext<Bytes>) do
  let result = declassify_discriminant(forward(key, body), "test-only finite opening status")
  case result do
    Ok(packet) -> Http.body_bytes(Http.build(:post, "https://example.test"), packet.plaintext)
    Err(_) -> Http.body_bytes(Http.build(:post, "https://example.test"), Bytes.empty())
  end
end
"#
        ));
        private_refusal(&format!(
            r#"
fn leak(context :: OhttpClientContext, body :: Plaintext<Bytes>) do
  let result = {prefix}decapsulate_response(context, body)
  println(String.from(Result.is_ok(result)))
end
"#
        ));
    }
}

#[test]
fn ohttp_local_and_flat_lookalikes_keep_their_body_dependencies() {
    for (definition, operation) in [
        ("module Ohttp do pub fn encapsulate_response(context :: OhttpGatewayContext, body :: Bytes) -> Bytes!CryptoError do Secret.destroy(context)\nOk(body) end end", "Ohttp.encapsulate_response"),
        ("fn ohttp_encapsulate_response(context :: OhttpGatewayContext, body :: Bytes) -> Bytes!CryptoError do Secret.destroy(context)\nOk(body) end", "ohttp_encapsulate_response"),
    ] {
        let definitions = check(definition);
        assert!(definitions.errors.is_empty(), "{definition}\n{:?}", definitions.errors);
        private_refusal(&format!("{definition}\nfn leak(context :: OhttpGatewayContext, body :: Plaintext<Bytes>) do let encrypted = declassify_discriminant({operation}(context, body), \"test-only status\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"), Result.unwrap_or(encrypted, Bytes.empty())) end"));
    }
}

#[test]
fn ohttp_imported_lookalikes_use_their_own_body_summary() {
    for (body, should_allow) in [("Bytes.empty()", true), ("body", false)] {
        let dependency = mesh_parser::parse(&format!(
            "pub fn encapsulate_response(context :: OhttpGatewayContext, body :: Bytes) -> Bytes!CryptoError do Secret.destroy(context)\nOk({body}) end\npub fn ohttp_encapsulate_response(context :: OhttpGatewayContext, body :: Bytes) -> Bytes!CryptoError do Secret.destroy(context)\nOk({body}) end"
        ));
        assert!(dependency.ok(), "{:?}", dependency.errors());
        let checked = mesh_typeck::check(&dependency);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let exports = mesh_typeck::collect_exports(&dependency, &checked);
        let mut imports = ImportContext::empty();
        imports.module_exports.insert(
            "Untrusted.Ohttp".into(),
            ModuleExports::new("Untrusted.Ohttp".into(), &exports),
        );
        for (import, operation) in [
            ("import Untrusted.Ohttp", "Ohttp.encapsulate_response"),
            (
                "from Untrusted.Ohttp import ohttp_encapsulate_response",
                "ohttp_encapsulate_response",
            ),
        ] {
            let source = format!(
                "{import}\nfn route(context :: OhttpGatewayContext, body :: Plaintext<Bytes>) do let encrypted = declassify_discriminant({operation}(context, body), \"test-only status\")\nHttp.body_bytes(Http.build(:post, \"https://example.test\"), Result.unwrap_or(encrypted, Bytes.empty())) end"
            );
            let parsed = mesh_parser::parse(&source);
            assert!(parsed.ok(), "{source}\n{:?}", parsed.errors());
            let checked = mesh_typeck::check_with_imports(&parsed, &imports);
            if should_allow {
                assert!(checked.errors.is_empty(), "{source}\n{:?}", checked.errors);
            } else {
                assert!(
                    checked
                        .errors
                        .iter()
                        .any(|e| matches!(e, TypeError::PlaintextViolation { .. })),
                    "escaped: {source}\n{:?}",
                    checked.errors
                );
                assert!(
                    checked
                        .errors
                        .iter()
                        .all(|e| matches!(e, TypeError::PlaintextViolation { .. })),
                    "unrelated failure: {source}\n{:?}",
                    checked.errors
                );
            }
        }
    }
}
