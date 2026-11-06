use mesh_typeck::error::TypeError;
fn refuses_consuming_lookalike(definition: &str, call: &str) {
    let source = format!("{definition}\nfn caller(key :: borrow X25519PrivateKey, wire :: Bytes) do {call}(key,37,wire) end");
    let parsed = mesh_parser::parse(&source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    let checked = mesh_typeck::check(&parsed);
    assert!(checked.errors.iter().any(|e| matches!(e,TypeError::ResourceViolation{reason,..} if reason.contains("borrowed resource"))),"a consuming user body received a borrowed key: {source}\n{:?}",checked.errors);
    assert!(
        checked
            .errors
            .iter()
            .all(|e| matches!(e, TypeError::ResourceViolation { .. })),
        "unrelated error: {:?}",
        checked.errors
    );
}
#[test]
fn local_flat_ohttp_lookalike_keeps_its_declared_consuming_key_parameter() {
    refuses_consuming_lookalike("fn ohttp_decapsulate_request(key :: X25519PrivateKey, id :: Int, wire :: Bytes) -> OhttpRequestOpening!CryptoError do Secret.destroy(key)\nErr(InvalidKey) end","ohttp_decapsulate_request");
}
#[test]
fn local_qualified_ohttp_lookalike_keeps_its_declared_consuming_key_parameter() {
    refuses_consuming_lookalike("module Ohttp do pub fn decapsulate_request(key :: X25519PrivateKey, id :: Int, wire :: Bytes) -> OhttpRequestOpening!CryptoError do Secret.destroy(key)\nErr(InvalidKey) end end","Ohttp.decapsulate_request");
}
#[test]
fn imported_flat_and_qualified_aliases_keep_their_consuming_key_parameter() {
    use mesh_typeck::{ImportContext, ModuleExports};
    let dependency=mesh_parser::parse("pub fn decapsulate_request(key :: X25519PrivateKey,id :: Int,wire :: Bytes) -> OhttpRequestOpening!CryptoError do Secret.destroy(key)\nErr(InvalidKey) end\npub fn ohttp_decapsulate_request(key :: X25519PrivateKey,id :: Int,wire :: Bytes) -> OhttpRequestOpening!CryptoError do Secret.destroy(key)\nErr(InvalidKey) end");
    assert!(dependency.ok(), "{:?}", dependency.errors());
    let checked = mesh_typeck::check(&dependency);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &checked);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Untrusted.Ohttp".into(),
        ModuleExports::new("Untrusted.Ohttp".into(), &exports),
    );
    for (import, call) in [
        ("import Untrusted.Ohttp", "Ohttp.decapsulate_request"),
        (
            "from Untrusted.Ohttp import ohttp_decapsulate_request",
            "ohttp_decapsulate_request",
        ),
    ] {
        let source=format!("{import}\nfn caller(key :: borrow X25519PrivateKey,wire :: Bytes) do {call}(key,37,wire) end");
        let parsed = mesh_parser::parse(&source);
        assert!(parsed.ok(), "{:?}", parsed.errors());
        let checked = mesh_typeck::check_with_imports(&parsed, &imports);
        assert!(checked.errors.iter().any(|e| matches!(e,TypeError::ResourceViolation{reason,..} if reason.contains("borrowed resource"))),"imported consuming body received borrowed key: {source}\n{:?}",checked.errors);
        assert!(
            checked
                .errors
                .iter()
                .all(|e| matches!(e, TypeError::ResourceViolation { .. })),
            "unrelated errors: {:?}",
            checked.errors
        );
    }
}
