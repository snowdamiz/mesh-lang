use mesh_typeck::{check, error::TypeError};

#[test]
fn native_declaration_has_the_annotated_public_function_type() {
    let parsed = mesh_parser::parse(
        "@native(\"mesh_math_add\")\npub fn add(left :: Int, right :: Int) -> Int\n",
    );
    let result = check(&parsed);
    assert!(result.errors.is_empty(), "{:?}", result.errors);

    let exports = mesh_typeck::collect_exports(&parsed, &result);
    assert!(exports.functions.contains_key("add"));
}

#[test]
fn native_declaration_rejects_implicit_or_private_abi_shapes() {
    for source in [
        "@native(\"mesh_value\")\nfn value() -> Int\n",
        "@native(\"mesh_value\")\npub fn value(input) -> Int\n",
        "@native(\"mesh_value\")\npub fn value(input :: Int)\n",
        "@native(\"mesh-value\")\npub fn value() -> Int\n",
        "@native(\"mesh_value\")\npub fn value(input :: List<Int>) -> Int\n",
    ] {
        let parsed = mesh_parser::parse(source);
        let result = check(&parsed);
        assert!(
            result
                .errors
                .iter()
                .any(|error| matches!(error, TypeError::NativeDeclarationInvalid { .. })),
            "expected native ABI diagnostic for {source:?}, got {:?}",
            result.errors
        );
    }
}
