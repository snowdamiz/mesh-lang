#[test]
fn process_lookup_returns_an_optional_pid_and_requires_a_name() {
    let parsed = mesh_parser::parse(
        r#"
fn optional() -> Option<Pid> do
  Process.lookup("mailbox")
end
fn main() do
  case optional() do
    None -> nil
    Some(pid) -> do
      Process.register("second-name", pid)
      nil
    end
  end
end
"#,
    );
    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    let result = mesh_typeck::check(&parsed);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let invalid = mesh_parser::parse("fn main() do Process.lookup(42) end");
    assert!(!mesh_typeck::check(&invalid).errors.is_empty());
}
