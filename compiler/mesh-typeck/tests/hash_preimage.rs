fn check(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn random_preimage_can_be_committed_persisted_and_explicitly_revealed() {
    let result = check(
        r#"
fn create() -> HashPreimage!CryptoError do HashPreimage.generate() end
fn commit(value :: borrow HashPreimage) -> Bytes!CryptoError do HashPreimage.commitment(value) end
fn persist(value :: borrow HashPreimage, key :: borrow StorageKey, context :: Bytes) -> Bytes!CryptoError do HashPreimage.seal_for_storage(value, key, context) end
fn restore(value :: Bytes, key :: borrow StorageKey, context :: Bytes) -> HashPreimage!CryptoError do HashPreimage.unseal_from_storage(value, key, context) end
fn publish(value :: HashPreimage) -> Bytes!CryptoError do HashPreimage.reveal(value) end
fn raw(value :: HashPreimage) -> Bytes!CryptoError do hash_preimage_reveal(value) end
fn discard(value :: HashPreimage) do Secret.destroy(value) end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn preimages_cannot_import_other_secrets_escape_ownership_or_reveal_twice() {
    for source in [
        "fn bad(value :: SecretBytes) do HashPreimage.reveal(value) end",
        "fn bad(value :: Bytes) do HashPreimage.from_bytes(value) end",
        "fn bad(value :: SecretBytes) do HashPreimage.from_secret(value) end",
        "fn bad(value :: HashPreimage) do HashPreimage.reveal(value)\nHashPreimage.reveal(value) end",
        "fn bad(value :: borrow HashPreimage) do HashPreimage.reveal(value) end",
        "fn bad(value :: HashPreimage) do println(value) end",
        "fn bad(value :: HashPreimage) do Json.encode(value) end",
        "fn bad(value :: HashPreimage) do [value] end",
        "fn bad(value :: HashPreimage, pid :: Pid<HashPreimage>) do send(pid, value) end",
    ] {
        assert!(!check(source).errors.is_empty(), "accepted {source}");
    }
}
