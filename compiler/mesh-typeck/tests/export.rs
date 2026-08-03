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
