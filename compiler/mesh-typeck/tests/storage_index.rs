fn check(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn storage_index_tag_borrows_an_opaque_key_and_returns_public_bytes() {
    let result = check(
        r#"
fn indexes(key :: borrow StorageKey, namespace :: Bytes, label :: Bytes) -> Bool!CryptoError do
  let first = StorageKey.index_tag(key, namespace, label)?
  let again = storage_key_index_tag(key, namespace, label)?
  let piped = key |> StorageKey.index_tag(namespace, label)?
  Ok(first == again && again == piped)
end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn storage_index_tag_never_retypes_or_reuses_a_destroyed_key() {
    for source in [
        "fn bad(key :: SecretBytes) do StorageKey.index_tag(key, Bytes.empty(), Bytes.empty()) end",
        "fn bad(key :: StorageKey) do Secret.destroy(key)\nStorageKey.index_tag(key, Bytes.empty(), Bytes.empty()) end",
        "fn bad(key :: StorageKey) do StorageKey.index_tag(key, \"namespace\", Bytes.empty()) end",
        "fn bad(key :: StorageKey) -> Bytes do StorageKey.index_tag(key, Bytes.empty(), Bytes.empty()) end",
    ] {
        assert!(!check(source).errors.is_empty(), "accepted {source}");
    }
}

#[test]
fn storage_index_tag_is_not_a_plaintext_declassification() {
    let result = check("fn leak(key :: borrow StorageKey, label :: Plaintext<Bytes>) -> Result<Int,CryptoError> do let tag = StorageKey.index_tag(key,Bytes.from_utf8(\"index\"),label)?\nOk(Http.body_bytes(Http.build(:post,\"https://example.test\"),tag)) end");
    assert!(
        result
            .errors
            .iter()
            .any(|error| format!("{error:?}").contains("Plaintext")),
        "{:?}",
        result.errors
    );
}
