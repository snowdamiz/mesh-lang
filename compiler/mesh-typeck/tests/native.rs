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

/// A native declaration is a plain function: no type parameters (whose
/// types it then reported as `?12`), `where` clause or guard (a guard made
/// it a function clause, which went unchecked), and a return type the ABI
/// carries. Each is its one error.
#[test]
fn native_declarations_are_plain_functions() {
    for (source, reason) in [
        (
            "@native(\"mesh_a\")\npub fn a<T>(x :: T) -> Int\n",
            "generic native functions are unsupported",
        ),
        (
            "@native(\"mesh_b\")\npub fn b(x :: Int) -> Int where Int: Eq\n",
            "native function declarations cannot have a where clause",
        ),
        (
            "@native(\"mesh_c\")\npub fn c(x :: Int) -> Int when x > 0\n",
            "native function declarations cannot have a guard",
        ),
        (
            "@native(\"mesh_d\")\npub fn d(x :: Int) -> List<Int>\n",
            "return type `List<Int>` is not ABI-safe",
        ),
    ] {
        let result = check(&mesh_parser::parse(source));
        let errors: Vec<String> = result.errors.iter().map(|e| e.to_string()).collect();
        assert!(
            errors.len() == 1 && errors[0].contains(reason),
            "{source:?}: {errors:?}"
        );
    }
}
