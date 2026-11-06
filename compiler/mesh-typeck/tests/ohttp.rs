//! OHTTP public API ownership contracts.
use mesh_typeck::error::TypeError;

fn check(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{source}\n{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn ohttp_both_names_have_role_specific_affine_results_and_borrow_the_gateway_key() {
    for prefix in ["Ohttp.", "ohttp_"] {
        let source = format!(
            r#"
fn request(id :: Int, recipient :: X25519PublicKey, body :: Bytes) -> OhttpRequest!CryptoError do
  {prefix}encapsulate_request(id, recipient, body)
end
fn open(key :: borrow X25519PrivateKey, id :: Int, wire :: Bytes) -> OhttpRequestOpening!CryptoError do
  let first = {prefix}decapsulate_request(key, id, wire)?
  Secret.destroy(first.response_context)
  {prefix}decapsulate_request(key, id, wire)
end
fn reply(context :: OhttpGatewayContext, body :: Bytes) -> Bytes!CryptoError do
  {prefix}encapsulate_response(context, body)
end
fn receive_reply(context :: OhttpClientContext, wire :: Bytes) -> Bytes!CryptoError do
  {prefix}decapsulate_response(context, wire)
end
"#
        );
        let result = check(&source);
        assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
        for name in [
            "OhttpClientContext",
            "OhttpGatewayContext",
            "OhttpRequest",
            "OhttpRequestOpening",
        ] {
            assert!(
                result.type_registry.is_resource_name(name),
                "not affine: {name}"
            );
        }
    }
}

#[test]
fn ohttp_contexts_move_on_both_success_and_error_paths_and_cannot_be_borrow_consumed() {
    for (context, operation) in [
        ("OhttpGatewayContext", "encapsulate_response"),
        ("OhttpClientContext", "decapsulate_response"),
    ] {
        for prefix in ["Ohttp.", "ohttp_"] {
            for source in [
                format!("fn bad(context :: {context}, body :: Bytes) do {prefix}{operation}(context, body)\nSecret.destroy(context) end"),
                format!("fn bad(context :: {context}, body :: Bytes) do case {prefix}{operation}(context, body) do Err(_) -> Secret.destroy(context) Ok(_) -> nil end end"),
            ] {
                let result = check(&source);
                assert!(result.errors.iter().any(|e| matches!(e, TypeError::ResourceViolation { reason, .. } if reason.contains("after it moved"))), "{source}\n{:?}", result.errors);
                assert!(result.errors.iter().all(|e| matches!(e, TypeError::ResourceViolation { .. })), "unrelated failure: {:?}", result.errors);
            }
            let source = format!("fn bad(context :: borrow {context}, body :: Bytes) do {prefix}{operation}(context, body) end");
            let result = check(&source);
            assert!(result.errors.iter().any(|e| matches!(e, TypeError::ResourceViolation { reason, .. } if reason.contains("borrowed resource"))), "{source}\n{:?}", result.errors);
        }
    }
}

#[test]
fn ohttp_context_roles_key_types_and_key_id_types_are_not_interchangeable() {
    for prefix in ["Ohttp.", "ohttp_"] {
        for source in [
            format!("fn bad(context :: OhttpClientContext, body :: Bytes) do {prefix}encapsulate_response(context, body) end"),
            format!("fn bad(context :: OhttpGatewayContext, body :: Bytes) do {prefix}decapsulate_response(context, body) end"),
            format!("fn bad(key :: SecretBytes, wire :: Bytes) do {prefix}decapsulate_request(key, 37, wire) end"),
            format!("fn bad(key :: X25519PublicKey, body :: Bytes) do {prefix}encapsulate_request(\"37\", key, body) end"),
        ] {
            let result = check(&source);
            assert!(result.errors.iter().any(|e| matches!(e, TypeError::Mismatch { .. })), "not a real type mismatch: {source}\n{:?}", result.errors);
        }
    }
}

#[test]
fn ohttp_contexts_cannot_format_serialize_cross_actors_or_use_secret_storage() {
    for resource in ["OhttpClientContext", "OhttpGatewayContext"] {
        for body in [
            "println(\"#{context}\")",
            "Json.encode(context)",
            "send(pid, context)",
        ] {
            let source = format!("fn bad(context :: {resource}, pid :: Pid) do {body} end");
            let result = check(&source);
            assert!(
                result
                    .errors
                    .iter()
                    .any(|e| matches!(e, TypeError::ResourceViolation { .. })),
                "escaped: {source}\n{:?}",
                result.errors
            );
            assert!(
                result.errors.iter().all(|e| match e {
                    TypeError::ResourceViolation { .. } => true,
                    TypeError::TraitNotSatisfied { ty, trait_name, .. } =>
                        ty.con_name() == Some(resource)
                            && matches!(trait_name.as_str(), "Display" | "Json"),
                    _ => false,
                }),
                "unrelated failure: {:?}",
                result.errors
            );
        }
        let source = format!("fn bad(context :: {resource}, wrapping :: borrow StorageKey, aad :: Bytes) do Secret.seal_for_storage(context, wrapping, aad) end");
        assert!(
            check(&source)
                .errors
                .iter()
                .any(|e| matches!(e, TypeError::Mismatch { .. })),
            "context became storage-sealable: {source}"
        );
    }
}

#[test]
fn ohttp_contexts_do_not_gain_an_indirect_affine_call_escape() {
    for prefix in ["Ohttp.", "ohttp_"] {
        let source = format!("fn bad(context :: OhttpGatewayContext, body :: Bytes) do let operation = {prefix}encapsulate_response\noperation(context, body) end");
        let result = check(&source);
        assert!(result.errors.iter().any(|e| matches!(e, TypeError::ResourceViolation { reason, .. } if reason.contains("generic or indirect call"))), "{source}\n{:?}", result.errors);
    }
}
