use mesh_typeck::{check, error::TypeError};

#[test]
fn export_declaration_accepts_the_stable_binary_boundary() {
    let parsed = mesh_parser::parse(
        "@export(\"mesh_mobile_echo\")\npub fn echo(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
    );
    let result = check(&parsed);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn export_declaration_rejects_unstable_abi_shapes() {
    for source in [
        "@export(\"mesh_echo\")\nfn echo(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
        "@export(\"mesh_echo\")\npub fn echo(request) -> Bytes!String do\n  Ok(request)\nend\n",
        "@export(\"mesh_echo\")\npub fn echo(request :: String) -> Bytes!String do\n  Ok(Bytes.from_utf8(request))\nend\n",
        "@export(\"mesh_echo\")\npub fn echo(request :: Bytes) -> Bytes do\n  request\nend\n",
        "@export(\"mesh-echo\")\npub fn echo(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
        // Symbols the host ABI, the C entry point, or C itself already own.
        "@export(\"mesh_library_init\")\npub fn echo(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
        "@export(\"main\")\npub fn echo(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
        "@export(\"static\")\npub fn echo(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
    ] {
        let parsed = mesh_parser::parse(source);
        let result = check(&parsed);
        assert!(
            result
                .errors
                .iter()
                .any(|error| matches!(error, TypeError::ExportDeclarationInvalid { .. })),
            "expected export ABI diagnostic for {source:?}, got {:?}",
            result.errors
        );
    }
}

#[test]
fn host_capabilities_are_bounded_binary_results() {
    let parsed = mesh_parser::parse(
        "pub fn load(request :: Bytes) -> Bytes ! String do\n  Host.secure_store_get(request)\nend\n",
    );
    let result = check(&parsed);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// An exported function is a plain function too: no type parameters,
/// `where` clause or guard (a guard made it a function clause, which went
/// unchecked), and a declared return type.
#[test]
fn export_declarations_are_plain_functions() {
    for (source, reason) in [
        (
            "@export(\"mesh_f\")\npub fn f<T>(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
            "generic exported functions are unsupported",
        ),
        (
            "@export(\"mesh_g\")\npub fn g(request :: Bytes) -> Bytes!String where Bytes: Eq do\n  Ok(request)\nend\n",
            "exported functions cannot have a where clause",
        ),
        (
            "@export(\"mesh_h\")\npub fn h(request :: Bytes) -> Bytes!String when true do\n  Ok(request)\nend\n",
            "exported functions cannot have a guard",
        ),
        (
            "@export(\"mesh_i\")\npub fn i(request :: Bytes) do\n  Ok(request)\nend\n",
            "exported functions require an explicit return type",
        ),
    ] {
        let result = check(&mesh_parser::parse(source));
        assert!(
            result.errors.iter().any(|error| error.to_string().contains(reason)),
            "{source:?}: {:?}",
            result.errors
        );
    }
}
