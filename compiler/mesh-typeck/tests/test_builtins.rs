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
