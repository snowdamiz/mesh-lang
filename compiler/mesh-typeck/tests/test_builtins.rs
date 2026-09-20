use mesh_parser::parse;
use mesh_typeck::{check_with_imports, ImportContext};

#[test]
fn in_memory_secure_store_builtin_is_test_mode_only() {
    let parsed = parse("fn install() -> Bool do\n  Test.install_in_memory_secure_store()\nend\n");

    let ordinary = check_with_imports(&parsed, &ImportContext::default());
    assert!(!ordinary.errors.is_empty());

    let test_mode = check_with_imports(
        &parsed,
        &ImportContext {
            test_builtins: true,
            ..ImportContext::default()
        },
    );
    assert!(test_mode.errors.is_empty(), "{:?}", test_mode.errors);
}

#[test]
fn push_token_fixture_builtin_has_selector_and_token_bytes_signature_in_test_mode() {
    let test_imports = ImportContext {
        test_builtins: true,
        ..ImportContext::default()
    };
    let bytes = parse(
        "fn install(selector :: Bytes, token :: Bytes) -> Bool do\n  Test.set_push_token(selector, token)\nend\n",
    );
    let bytes_result = check_with_imports(&bytes, &test_imports);
    assert!(bytes_result.errors.is_empty(), "{:?}", bytes_result.errors);

    let string = parse(
        "fn install() -> Bool do\n  Test.set_push_token(Bytes.from_utf8(\"selector\"), \"token\")\nend\n",
    );
    let string_result = check_with_imports(&string, &test_imports);
    assert!(!string_result.errors.is_empty());
}
