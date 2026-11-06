use mesh_typeck::check;

#[test]
fn explicit_http_roots_use_the_bytes_builder_contract() {
    let parsed = mesh_parser::parse(
        r##"
fn main() do
  let request :: Int = Http.build(:get, "https://localhost/")
    |> Http.trust_root_pem(Bytes.from_utf8("certificate"))
    |> Http.max_redirects(0)
  let same :: Int = http_trust_root_pem(request, Bytes.empty())
  case Http.send(same) do
    Ok(response) -> println("#{response.status}")
    Err(error) -> println(error)
  end
end
"##,
    );
    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    let checked = check(&parsed);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
}

#[test]
fn explicit_http_roots_reject_text_and_wrong_handle_types() {
    for expression in [
        "Http.trust_root_pem(1, \"certificate\")",
        "Http.trust_root_pem(\"handle\", Bytes.empty())",
        "http_trust_root_pem(1, \"certificate\")",
    ] {
        let parsed = mesh_parser::parse(&format!("fn main() do\n {expression}\nend"));
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        assert!(!check(&parsed).errors.is_empty(), "accepted {expression}");
    }
}
