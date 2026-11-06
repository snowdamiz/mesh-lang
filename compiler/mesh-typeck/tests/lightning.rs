fn check(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn verified_invoice_details_are_public_bytes_or_a_bounded_error() {
    let result = check("fn inspect(input :: String) -> Bytes!String do Lightning.invoice_details(input) end\nfn raw(input :: String) -> Bytes!String do lightning_invoice_details(input) end\nfn indirect(input :: String) -> Bytes!String do let parse = Lightning.invoice_details\nparse(input) end");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    for source in [
        "fn bad() do Lightning.invoice_details(Bytes.empty()) end",
        "fn bad(value :: String) -> Bytes do Lightning.invoice_details(value) end",
        "fn bad() do Lightning.invoice_details() end",
    ] {
        assert!(!check(source).errors.is_empty(), "accepted {source}");
    }
}
