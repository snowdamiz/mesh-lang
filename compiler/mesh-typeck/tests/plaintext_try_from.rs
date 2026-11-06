//! `?` must analyze the selected error conversion, including its effects.
use mesh_typeck::{error::TypeError, ImportContext, ModuleExports, TypeckResult};

const TYPES: &str = r#"
type SourceError do First(detail :: String) Second end
type WrappedError do Wrapped(error :: SourceError) Other end
"#;
const WRAP: &str = r#"
impl From<SourceError> for WrappedError do
  fn from(error :: SourceError) -> WrappedError do Wrapped(error) end
end
"#;
const PROPAGATE: &str = r#"
fn propagate(value :: Result<Int,Plaintext<SourceError>>) -> Int!WrappedError do
  Ok(value?)
end
"#;
const OBSERVE: &str = r#"
fn observe(value :: Result<Int,Plaintext<SourceError>>) do
  case propagate(value) do
    Err(Other) -> println("other")
    Err(_) -> println("wrapped")
    Ok(_) -> println("ok")
  end
end
"#;

fn checked(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{source}\n{:?}", parse.errors());
    mesh_typeck::check(&parse)
}

fn allowed(source: &str) {
    let result = checked(source);
    assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
}

fn refused(source: &str) {
    let result = checked(source);
    assert!(!result.errors.is_empty(), "escaped: {source}");
    assert!(
        result
            .errors
            .iter()
            .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
        "unrelated error: {source}\n{:?}",
        result.errors
    );
}

#[test]
fn fixed_wrapper_variant_matches_explicit_conversion() {
    for body in [
        "Ok(value?)",
        "case value do Ok(number) -> Ok(number) Err(error) -> Err(Wrapped(error)) end",
    ] {
        allowed(&format!("{TYPES}{WRAP}fn propagate(value :: Result<Int,Plaintext<SourceError>>) -> Int!WrappedError do {body} end\n{OBSERVE}"));
    }
}

#[test]
fn wrapped_private_error_payload_is_still_refused() {
    refused(&format!(
        r#"{TYPES}{WRAP}{PROPAGATE}
fn observe(value :: Result<Int,Plaintext<SourceError>>) do
  case propagate(value) do
    Err(Wrapped(error)) -> case error do First(detail) -> println(detail) Second -> println("second") end
    _ -> nil
  end
end"#
    ));
}

#[test]
fn conversion_selected_variant_remains_private() {
    refused(&format!(
        r#"{TYPES}
impl From<SourceError> for WrappedError do
  fn from(error :: SourceError) -> WrappedError do
    case error do First(_) -> Wrapped(error) Second -> Other end
  end
end
{PROPAGATE}{OBSERVE}"#
    ));
}

#[test]
fn conversion_effects_receive_private_error_dependencies() {
    refused(&format!(
        r#"{TYPES}
impl From<SourceError> for WrappedError do
  fn from(error :: SourceError) -> WrappedError do
    case error do First(detail) -> println(detail) Second -> println("second") end
    Wrapped(error)
  end
end
{PROPAGATE}"#
    ));
}

#[test]
fn prior_selection_and_caller_control_survive_conversion() {
    for body in [
        "let selected = if secret do one else two end\npropagate(selected)",
        "if secret do propagate(one) else propagate(two) end",
    ] {
        refused(&format!(
            r#"{TYPES}{WRAP}{PROPAGATE}
fn selected(secret :: Plaintext<Bool>,one :: Result<Int,Plaintext<SourceError>>,two :: Result<Int,Plaintext<SourceError>>) -> Int!WrappedError do {body} end
fn observe(secret :: Plaintext<Bool>,one :: Result<Int,Plaintext<SourceError>>,two :: Result<Int,Plaintext<SourceError>>) do
  println(String.from(Result.is_ok(declassify_discriminant(selected(secret,one,two),"tag only"))))
end"#
        ));
    }
}

#[test]
fn same_error_propagation_does_not_call_from() {
    let source = r#"
impl From<String> for String do
  fn from(error :: String) -> String do println(error)
error end
end
fn propagate(value :: Result<Int,Plaintext<String>>) -> Int!String do Ok(value?) end
"#;
    allowed(source);
    allowed(&format!(
        r#"{TYPES}
impl From<SourceError> for SourceError do
  fn from(error :: SourceError) -> SourceError do
    case error do First(detail) -> println(detail) Second -> nil end
    error
  end
end
fn propagate(value :: Result<Int,Plaintext<SourceError>>) -> Int!SourceError do Ok(value?) end"#
    ));
    refused(&format!(
        r#"{source}
fn observe(value :: Result<Int,Plaintext<String>>) do
  case propagate(value) do Err(error) -> println(error) _ -> nil end
end"#
    ));
}

#[test]
fn conversion_identity_includes_source_and_target_types() {
    let sibling = r#"
impl From<Int> for WrappedError do
  fn from(error :: Int) -> WrappedError do
    println(Int.to_string(error))
    Other
  end
end
type AnotherError do Another end
impl From<SourceError> for AnotherError do
  fn from(error :: SourceError) -> AnotherError do
    case error do First(detail) -> println(detail) Second -> nil end
    Another
  end
end
"#;
    for implementations in [format!("{WRAP}{sibling}"), format!("{sibling}{WRAP}")] {
        allowed(&format!("{TYPES}{implementations}{PROPAGATE}{OBSERVE}"));
        refused(&format!("{TYPES}{implementations}fn leak(value :: Result<Int,Plaintext<Int>>) -> Int!WrappedError do Ok(value?) end"));
        refused(&format!("{TYPES}{implementations}fn leak(value :: Result<Int,Plaintext<SourceError>>) -> Int!AnotherError do Ok(value?) end"));
    }
}

#[test]
fn imported_conversion_keeps_its_shape_and_effects_and_unknown_proof_is_refused() {
    for effect in [
        "",
        "case error do First(detail) -> println(detail) Second -> nil end",
    ] {
        let dependency = mesh_parser::parse(&format!(
            "{}{}",
            TYPES.replace("type ", "pub type "),
            WRAP.replace("do Wrapped(error)", &format!("do {effect}\nWrapped(error)"))
        ));
        let result = mesh_typeck::check(&dependency);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let exports = mesh_typeck::collect_exports(&dependency, &result);
        for import in [
            "import Errors",
            "from Errors import SourceError, WrappedError",
        ] {
            for retain_proof in [true, false] {
                let mut module = ModuleExports::new("Errors".into(), &exports);
                if !retain_proof {
                    module.plaintext_functions.clear();
                }
                let mut imports = ImportContext::empty();
                imports.all_trait_impls.extend(exports.trait_impls.clone());
                imports.module_exports.insert("Errors".into(), module);
                let parse = mesh_parser::parse(&format!("{import}\n{PROPAGATE}{OBSERVE}"));
                assert!(parse.ok(), "{:?}", parse.errors());
                let result = mesh_typeck::check_with_imports(&parse, &imports);
                assert!(
                    result
                        .errors
                        .iter()
                        .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
                    "{:?}",
                    result.errors
                );
                assert_eq!(
                    result.errors.is_empty(),
                    effect.is_empty() && retain_proof,
                    "{import}: proof={retain_proof}\n{:?}",
                    result.errors
                );
            }
        }
    }
}

#[test]
fn conversion_effects_observe_private_error_tag_and_caller_control() {
    let wrap = WRAP.replace(
        "do Wrapped(error)",
        "do println(\"conversion ran\")\nWrapped(error)",
    );
    allowed(&format!("{TYPES}{wrap}{PROPAGATE}"));
    for body in [
        "Ok((if secret do one else two end)?)",
        "if secret do Ok(one?) else Ok(two?) end",
    ] {
        refused(&format!("{TYPES}{wrap}fn leak(secret :: Plaintext<Bool>,one :: Result<Int,SourceError>,two :: Result<Int,SourceError>) -> Int!WrappedError do {body} end"));
    }
}

#[test]
fn alias_and_closure_use_the_selected_conversion() {
    for body in [
        "let same = fn(input :: Result<Int,SourceError>) -> Ok(input?) end\nOk(same(value)?)",
        "Ok(value?)",
    ] {
        let implementation = WRAP.replace("From<SourceError>", "From<Alias>");
        allowed(&format!("{TYPES}type Alias = SourceError\n{implementation}\nfn propagate(value :: Result<Int,Plaintext<SourceError>>) -> Int!WrappedError do {body} end\n{OBSERVE}"));
    }
}

#[test]
fn generic_source_arguments_select_distinct_conversion_bodies() {
    let definitions = r#"
type Wrapped do Ints(values :: List<Int>) Strings(values :: List<String>) end
impl From<List<Int>> for Wrapped do
  fn from(error :: List<Int>) -> Wrapped do Ints(error) end
end
impl From<List<String>> for Wrapped do
  fn from(error :: List<String>) -> Wrapped do
    if List.length(error) > 0 do Strings(error) else Ints([]) end
  end
end
"#;
    for (source, public) in [("Int", true), ("String", false)] {
        let program = format!(
            r#"{definitions}
fn propagate(value :: Result<Int,Plaintext<List<{source}>>>) -> Int!Wrapped do Ok(value?) end
fn observe(value :: Result<Int,Plaintext<List<{source}>>>) do
  case propagate(value) do Err(Ints(_)) -> println("ints") _ -> println("other") end
end"#
        );
        if public {
            allowed(&program);
        } else {
            refused(&program);
        }
    }
}

#[test]
fn primitive_from_preserves_private_payload_and_public_tag() {
    for (source, target) in [
        ("Int", "String"),
        ("Float", "String"),
        ("Bool", "String"),
        ("Int", "Float"),
    ] {
        for body in ["Ok(value?)".to_string(), format!("case value do Ok(number) -> Ok(number) Err(error) -> Err({target}.from(error)) end")] {
            let propagate = format!("fn propagate(value :: Result<Int,Plaintext<{source}>>) -> Int!{target} do {body} end");
            let printed = if target == "String" { "error" } else { "String.from(error)" };
            allowed(&format!(r#"{propagate}
fn observe(value :: Result<Int,Plaintext<{source}>>) do
  case propagate(value) do Ok(_) -> println("ok") Err(_) -> println("err") end
end"#));
            refused(&format!(r#"{propagate}
fn observe(value :: Result<Int,Plaintext<{source}>>) do
  case propagate(value) do Ok(_) -> nil Err(error) -> println({printed}) end
end"#));
        }
    }
}

#[test]
fn primitive_from_preserves_private_selection_and_caller_control() {
    for source in ["Int", "Float", "Bool"] {
        for (input, body) in [
            (format!("value :: Plaintext<Result<Int,{source}>>"), "Ok(value?)"),
            (format!("secret :: Plaintext<Bool>, one :: Result<Int,{source}>, two :: Result<Int,{source}>"), "Ok((if secret do one else two end)?)"),
            (format!("secret :: Plaintext<Bool>, one :: Result<Int,{source}>, two :: Result<Int,{source}>"), "if secret do Ok(one?) else Ok(two?) end"),
        ] {
            let args = if input.starts_with("value") { "value" } else { "secret,one,two" };
            refused(&format!(r#"
fn propagate({input}) -> Int!String do {body} end
fn observe({input}) do
  case propagate({args}) do Ok(_) -> println("ok") Err(_) -> println("err") end
end"#));
        }
        refused(&format!(
            r#"
type Custom do Converted end
impl From<{source}> for Custom do
  fn from(error :: {source}) -> Custom do println(String.from(error))
Converted end
end
fn propagate(value :: Result<Int,Plaintext<{source}>>) -> Int!Custom do Ok(value?) end"#
        ));
    }
}

#[test]
fn user_code_cannot_replace_primitive_from() {
    for (source, target) in [
        ("Int", "String"),
        ("Float", "String"),
        ("Bool", "String"),
        ("Int", "Float"),
    ] {
        let result = checked(&format!(
            r#"
impl From<{source}> for {target} do
  fn from(error :: {source}) -> {target} do println("user conversion")
{target}.from(error) end
end"#
        ));
        assert!(
            result
                .errors
                .iter()
                .any(|error| matches!(error, TypeError::DuplicateImpl { .. })),
            "{:?}",
            result.errors
        );
    }
}
