//! Projection precision must not turn a public field/tag into a laundering
//! channel when its actual value was selected or computed from message content.
use mesh_typeck::{error::TypeError, ImportContext, ModuleExports};

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check(&parse)
}
fn allowed(source: &str) {
    let result = checked(source);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}
fn refused(source: &str) {
    let result = checked(source);
    assert!(
        result
            .errors
            .iter()
            .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
        "unrelated errors: {:?}",
        result.errors
    );
    assert!(!result.errors.is_empty(), "plaintext escaped: {source}");
}
const MESSAGE: &str = "struct Message do body :: Plaintext<Bytes>\nid :: Bytes end\n";
const ROUTE: &str = "fn route(value :: Message) do Http.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end\n";

const PREPARED_RESULT: &str = r#"
resource struct PreparedResult do
  path :: String
  image :: Plaintext<Bytes>
end
fn refuse_prepared() -> PreparedResult!String do
  Err("unavailable")
end
fn retire_prepared(value :: consume PreparedResult) do
  nil
end
"#;

fn prepared_result_case(refusal: &str, choice: &str, output: &str) -> String {
    format!(
        r#"{PREPARED_RESULT}
fn prepare(choice :: Bool, path :: String, image :: Plaintext<Bytes>) -> PreparedResult!String do
  if choice do
    {refusal}
  else
    Ok(PreparedResult {{ path: path, image: image }})
  end
end
fn route(choice :: {choice}, path :: String, image :: Plaintext<Bytes>) -> Unit!String do
  let prepared = prepare(choice, path, image)?
  {output}
  retire_prepared(prepared)
  Ok(nil)
end"#
    )
}

#[test]
fn prepared_result_refusal_helper_keeps_a_successful_resource_public_field() {
    allowed(&prepared_result_case(
        "refuse_prepared()",
        "Bool",
        "let _public = File.write(prepared.path, \"public constant\")",
    ));
}

#[test]
fn prepared_result_inline_refusal_keeps_a_successful_resource_public_field() {
    allowed(&prepared_result_case(
        "Err(\"unavailable\")",
        "Bool",
        "let _public = File.write(prepared.path, \"public constant\")",
    ));
}

#[test]
fn prepared_result_keeps_private_payload_and_prior_choice_refusals() {
    refused(&prepared_result_case(
        "refuse_prepared()",
        "Bool",
        "let _private = File.write_bytes(path, 0, prepared.image, false)",
    ));
    refused(&prepared_result_case(
        "refuse_prepared()",
        "Plaintext<Bool>",
        "let _private_choice = File.write(path, \"public constant\")",
    ));
}

#[test]
fn prepared_result_alias_refusal_keeps_a_successful_resource_public_field() {
    let source = prepared_result_case(
        "alias_refusal()",
        "Bool",
        "let _public = File.write(prepared.path, \"public constant\")",
    );
    allowed(&format!(
        "{source}\ntype PreparedAlias = Result<PreparedResult,String>\nfn alias_refusal() -> PreparedAlias do refuse_prepared() end"
    ));
}

#[test]
fn prepared_result_recursive_refusal_keeps_a_successful_resource_public_field() {
    let source = prepared_result_case(
        "recursive_refusal(false)",
        "Bool",
        "let _public = File.write(prepared.path, \"public constant\")",
    );
    allowed(&format!(
        "{source}\nfn recursive_refusal(stop :: Bool) -> PreparedResult!String do if stop do refuse_prepared() else recursive_refusal(true) end end"
    ));
}

#[test]
fn prepared_result_recursive_success_keeps_its_private_field_dependency() {
    refused(&format!(
        r#"{PREPARED_RESULT}
fn recursive_success(stop :: Bool, image :: Plaintext<Bytes>) -> PreparedResult!String do
  if stop do
    Ok(PreparedResult {{ path: Bytes.to_hex(image), image: image }})
  else
    recursive_success(true, image)
  end
end
fn route(image :: Plaintext<Bytes>) -> Unit!String do
  let prepared = recursive_success(false, image)?
  let _private = File.write(prepared.path, "public constant")
  retire_prepared(prepared)
  Ok(nil)
end"#
    ));
}

#[test]
fn prepared_result_global_callable_alias_keeps_a_successful_public_field() {
    let source = prepared_result_case(
        "refuse_alias()",
        "Bool",
        "let _public = File.write(prepared.path, \"public constant\")",
    );
    allowed(&format!("{source}\nlet refuse_alias = refuse_prepared"));
}

fn imported_prepared_result(unknown: bool) -> (String, ImportContext) {
    let dependency = PREPARED_RESULT
        .replace(
            "resource struct PreparedResult",
            "pub resource struct PreparedResult",
        )
        .replace("fn refuse_prepared", "pub fn refuse_prepared");
    let parse = mesh_parser::parse(&dependency);
    assert!(parse.ok(), "{:?}", parse.errors());
    let result = mesh_typeck::check(&parse);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&parse, &result);
    let mut module = ModuleExports::new("Refusal".into(), &exports);
    assert!(module.plaintext_functions.contains_key("refuse_prepared"));
    if unknown {
        // An older embedding can supply an effect summary without constructor
        // proof. The absence of proof must stay unknown, never empty/bottom.
        module
            .plaintext_functions
            .insert("refuse_prepared".into(), Default::default());
    }
    let mut imports = ImportContext::empty();
    imports.module_exports.insert("Refusal".into(), module);
    let source = prepared_result_case(
        "refuse_prepared()",
        "Bool",
        "let _public = File.write(prepared.path, \"public constant\")",
    )
    .replace(
        PREPARED_RESULT,
        "from Refusal import PreparedResult, refuse_prepared\nfn retire_prepared(value :: consume PreparedResult) do nil end\n",
    );
    (source, imports)
}

#[test]
fn prepared_result_checked_import_keeps_proven_refusal_constructors() {
    let (source, imports) = imported_prepared_result(false);
    let parse = mesh_parser::parse(&source);
    assert!(parse.ok(), "{:?}", parse.errors());
    let result = mesh_typeck::check_with_imports(&parse, &imports);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn prepared_result_unknown_import_does_not_gain_a_constructor_proof() {
    let (source, imports) = imported_prepared_result(true);
    let parse = mesh_parser::parse(&source);
    assert!(parse.ok(), "{:?}", parse.errors());
    let result = mesh_typeck::check_with_imports(&parse, &imports);
    assert!(!result.errors.is_empty(), "unknown result was narrowed");
    assert!(result
        .errors
        .iter()
        .all(|error| matches!(error, TypeError::PlaintextViolation { .. })));
}

#[test]
fn prepared_result_arbitrary_parameter_result_keeps_private_metadata() {
    refused(&format!(
        r#"{PREPARED_RESULT}
fn unknown_result(value :: Result<PreparedResult,String>) -> PreparedResult!String do
  value
end
fn route(choice :: Bool, path :: String, image :: Plaintext<Bytes>) -> Unit!String do
  let selected = if choice do
    unknown_result(Ok(PreparedResult {{ path: Bytes.to_hex(image), image: image }}))
  else
    Ok(PreparedResult {{ path: path, image: image }})
  end
  let prepared = selected?
  let _private = File.write(prepared.path, "public constant")
  retire_prepared(prepared)
  Ok(nil)
end"#
    ));
}

#[test]
fn a_declared_public_field_is_independent_of_the_private_payload() {
    allowed(&format!("{MESSAGE}{ROUTE}"));
    refused(&format!("{MESSAGE}fn leak(value :: Message) do Http.body_bytes(Http.build(:post,\"https://example.test\"),value.body) end"));
    refused(&format!("{MESSAGE}fn leak(value :: Plaintext<Message>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end"));
}

#[test]
fn derived_public_fields_remain_private_across_construction_forwarding_and_updates() {
    for tail in [
        "fn leak(body :: Plaintext<Bytes>) do route(Message { body: body, id: body }) end",
        "fn forward(value :: Message) -> Message do value end\nfn leak(body :: Plaintext<Bytes>) do route(forward(Message { body: body, id: body })) end",
        "fn leak(value :: Message, body :: Plaintext<Bytes>) do route(%{value | id: body}) end",
        "fn leak(flag :: Plaintext<Bool>, body :: Bytes) do let id = if flag do Bytes.from_utf8(\"yes\") else Bytes.from_utf8(\"no\") end\nroute(Message {body:body,id:id}) end",
    ] { refused(&format!("{MESSAGE}{ROUTE}{tail}")); }
}

#[test]
fn a_public_result_tag_does_not_disclose_its_labeled_payload() {
    allowed("fn seal(body :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) -> Bytes!CryptoError do\nlet value :: Result<Bytes,CryptoError> = Ok(body)\ncase value do Ok(bytes) -> Crypto.aead_seal(key,nonce,Bytes.empty(),bytes)\nErr(error) -> Err(error) end end");
    allowed("fn seal(value :: Result<Plaintext<Bytes>,String>, key :: borrow AeadKey, nonce :: Bytes) -> Result<Result<Bytes,CryptoError>,String> do\nlet body = value?\nOk(Crypto.aead_seal(key,nonce,Bytes.empty(),body)) end");
}

#[test]
fn a_data_derived_result_tag_remains_private_through_forwarding_and_question_mark() {
    for source in [
        "fn choose(flag :: Plaintext<Bool>) -> Result<Int,String> do if flag do Ok(1) else Err(\"fixed\") end end\nfn leak(flag :: Plaintext<Bool>) -> Result<Int,String> do let _ = choose(flag)?\nprintln(\"fixed\")\nOk(0) end",
        "fn leak(value :: Plaintext<Result<Int,String>>) -> Result<Int,String> do let _ = value?\nprintln(\"fixed\")\nOk(0) end",
        "fn leak(value :: Result<Plaintext<String>,String>) do case value do Ok(\"guess\") -> println(\"matched\")\n_ -> nil end end",
    ] { refused(source); }
}

#[test]
fn projecting_through_lists_and_variant_payloads_preserves_field_dependencies() {
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do\nlet value = List.head(values)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end"));
    refused(&format!("{MESSAGE}{ROUTE}fn leak(body :: Plaintext<Bytes>) do\nlet values = [Message {{body:body,id:body}}]\ncase Some(List.head(values)) do Some(value) -> route(value)\nNone -> 0 end end"));
}

#[test]
fn question_mark_control_survives_nested_calls_and_blocks() {
    for source in [
        "fn leak(value :: Plaintext<Result<String,String>>) -> Result<Int,String> do let _ = String.length(value?)\nprintln(\"fixed\")\nOk(0) end",
        "fn leak(value :: Plaintext<Result<Int,String>>, flag :: Bool) -> Result<Int,String> do if flag do let _ = value?\n0 else 1 end\nprintln(\"fixed\")\nOk(0) end",
        "fn leak(value :: Option<Int>, flag :: Plaintext<Bool>) do case value do Some(_) when flag -> nil\n_ -> println(\"fallback\") end end",
        "fn ignore(first :: Int, request :: Int) -> Int do 0 end\nfn leak(value :: Plaintext<Result<Int,String>>) -> Result<Int,String> do let _ = ignore(value?,Http.build(:post,\"https://example.test\"))\nOk(0) end",
        "fn leak(value :: Plaintext<Result<Int,String>>, flag :: Bool) -> Result<Int,String> do if flag do 0 else let _ = value?\n0 end\nprintln(\"fixed\")\nOk(0) end",
    ] { refused(source); }
}

#[test]
fn nested_generic_aliases_and_explicit_returns_keep_projection_proofs() {
    allowed(&format!("{MESSAGE}type Wrapped<T> = Result<List<T>,String>\nfn forward(value :: Wrapped<Message>) -> Wrapped<Message> do return value end\nfn route(values :: Wrapped<Message>) do case forward(values) do Ok(items) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),List.head(items).id)\nErr(_) -> 0 end end"));
    refused(&format!("{MESSAGE}{ROUTE}fn forward(value :: Message) -> Message do return value end\nfn leak(body :: Plaintext<Bytes>) do route(forward(Message {{body:body,id:body}})) end"));
}

#[test]
fn imported_projections_are_proofs_about_actual_arguments_not_declared_field_names() {
    let parse = mesh_parser::parse(&format!("pub {MESSAGE}pub type Wrapped<T> = Result<List<T>,String>\npub fn identifier(value :: Message) -> Bytes do value.id end\npub fn forward(value :: Wrapped<Message>) -> Wrapped<Message> do value end"));
    assert!(parse.ok(), "{:?}", parse.errors());
    let result = mesh_typeck::check(&parse);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&parse, &result);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Messages".into(),
        ModuleExports::new("Messages".into(), &exports),
    );
    for (source, private) in [
        ("import Messages\nfn route(value :: Messages.Message) do Http.body_bytes(Http.build(:post,\"https://example.test\"),Messages.identifier(value)) end", false),
        ("from Messages import Message, identifier\nfn leak(body :: Plaintext<Bytes>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),identifier(Message {body:body,id:body})) end", true),
        ("import Messages\nfn route(value :: Messages.Wrapped<Messages.Message>) do case Messages.forward(value) do Ok(items) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),List.head(items).id)\nErr(_) -> 0 end end", false),
    ] {
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let result = mesh_typeck::check_with_imports(&parse, &imports);
        assert!(result.errors.iter().all(|error| matches!(error,TypeError::PlaintextViolation {..})), "{:?}", result.errors);
        assert_eq!(!result.errors.is_empty(),private,"{source}: {:?}",result.errors);
    }
}

#[test]
fn iteration_uses_public_length_and_retains_the_element_field_shape() {
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do for value in values do Http.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end end"));
    refused(&format!("{MESSAGE}fn leak(values :: List<Message>) do for value in values do Http.body_bytes(Http.build(:post,\"https://example.test\"),value.body) end end"));
    refused(&format!("{MESSAGE}fn leak(values :: Plaintext<List<Message>>) do for value in values do Http.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end end"));
}

#[test]
fn for_in_results_preserve_public_lengths_and_nested_item_projections() {
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do let mapped = for value in values do value end\nprintln(Int.to_string(List.length(mapped)))\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(mapped).id) end"));
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do let bodies = for value in values when Bytes.length(value.id) > 0 do value.body end\nprintln(Int.to_string(List.length(bodies))) end"));
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do let nested = for value in values do [value] end\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(List.head(nested)).id) end"));
}

#[test]
fn for_in_shape_precision_keeps_payload_filter_and_early_exit_dependencies() {
    for tail in [
        "fn leak(values :: List<Message>) do let bodies = for value in values do value.body end\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(bodies)) end",
        "fn leak(values :: List<Message>) do let mapped = for value in values do %{value | id: value.body} end\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(mapped).id) end",
        "fn leak(values :: List<Message>) do let filtered = for value in values when Bytes.length(value.body) > 0 do value.id end\nprintln(Int.to_string(List.length(filtered))) end",
        "fn leak(values :: Plaintext<List<Message>>) do let mapped = for value in values do value.id end\nprintln(Int.to_string(List.length(mapped))) end",
        "fn leak(values :: List<Message>) do let mapped = for value in values do if Bytes.length(value.body) > 0 do break end\nvalue end\nprintln(Int.to_string(List.length(mapped))) end",
        "fn leak(values :: List<Message>) do let mapped = for value in values do if Bytes.length(value.body) > 0 do continue end\nvalue end\nprintln(Int.to_string(List.length(mapped))) end",
        "fn leak(values :: List<Message>) -> Result<Int,String> do let mapped = for value in values do if Bytes.length(value.body) > 0 do return Err(\"fixed\") end\nvalue end\nprintln(Int.to_string(List.length(mapped)))\nOk(0) end",
    ] { refused(&format!("{MESSAGE}{tail}")); }
    refused("fn leak(values :: List<Plaintext<Result<Int,String>>>) -> Result<Int,String> do let mapped = for value in values do value? end\nprintln(Int.to_string(List.length(mapped)))\nOk(0) end");
}

#[test]
fn imported_comprehension_summaries_keep_items_and_selection_control_separate() {
    let parse = mesh_parser::parse(&format!("pub {MESSAGE}pub fn mapped(values :: List<Message>) -> List<Message> do for value in values do value end end\npub fn filtered(values :: List<Message>) -> List<Message> do for value in values when Bytes.length(value.body) > 0 do value end end"));
    assert!(parse.ok(), "{:?}", parse.errors());
    let result = mesh_typeck::check(&parse);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&parse, &result);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Messages".into(),
        ModuleExports::new("Messages".into(), &exports),
    );
    for (tail, private) in [
        ("let mapped = Messages.mapped(values)\nprintln(Int.to_string(List.length(mapped)))\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(mapped).id)", false),
        ("let mapped = Messages.mapped(values)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(mapped).body)", true),
        ("let original = List.head(values)\nlet mapped = Messages.mapped([%{original | id: original.body}])\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(mapped).id)", true),
        ("let filtered = Messages.filtered(values)\nprintln(Int.to_string(List.length(filtered)))", true),
    ] {
        let parse = mesh_parser::parse(&format!("import Messages\nfn route(values :: List<Messages.Message>) do {tail} end"));
        assert!(parse.ok(), "{:?}", parse.errors());
        let result = mesh_typeck::check_with_imports(&parse, &imports);
        assert!(result.errors.iter().all(|error| matches!(error, TypeError::PlaintextViolation { .. })), "{:?}", result.errors);
        assert_eq!(!result.errors.is_empty(), private, "{tail}: {:?}", result.errors);
    }
}

#[test]
fn list_concat_operators_preserve_lengths_and_items_without_clearing_private_choices() {
    for operator in ["++", "<>"] {
        allowed(&format!("{MESSAGE}fn route(values :: List<Message>, value :: Message) do let joined = values {operator} [value]\nprintln(Int.to_string(List.length(joined)))\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(joined).id) end"));
        for tail in [
            format!("fn leak(values :: List<Message>, value :: Message) do let joined = values {operator} [value]\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(joined).body) end"),
            format!("fn leak(values :: List<Message>, value :: Message) do let joined = values {operator} [%{{value | id: value.body}}]\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(joined).id) end"),
            format!("fn leak(values :: Plaintext<List<Message>>, value :: Message) do let joined = values {operator} [value]\nprintln(Int.to_string(List.length(joined))) end"),
            format!("fn leak(flag :: Plaintext<Bool>, value :: Message) do let chosen = if flag do [value] else [] end\nlet joined = chosen {operator} [value]\nprintln(Int.to_string(List.length(joined))) end"),
        ] { refused(&format!("{MESSAGE}{tail}")); }
        refused(&format!(
            "fn leak(body :: Plaintext<String>) do println(\"prefix\" {operator} body) end"
        ));
    }
}

#[test]
fn inferred_parameters_and_destructuring_preserve_public_fields() {
    allowed(&format!("{MESSAGE}fn identifier(value) -> Bytes do let Message {{id}} = value\nid end\nfn route(value :: Message) do Http.body_bytes(Http.build(:post,\"https://example.test\"),identifier(value)) end"));
    allowed(&format!("{MESSAGE}fn route(Message {{id}}) do Http.body_bytes(Http.build(:post,\"https://example.test\"),id) end"));
    refused(&format!("{MESSAGE}fn leak(Message {{body}}) do Http.body_bytes(Http.build(:post,\"https://example.test\"),body) end"));
}

#[test]
fn a_nullary_variant_has_no_private_payload_but_does_not_clear_private_control() {
    allowed(&format!("{MESSAGE}fn empty() -> Option<Message> do None end\nfn route() do case empty() do None -> println(\"empty\")\nSome(_) -> nil end end"));
    refused(&format!("{MESSAGE}fn choose(flag :: Plaintext<Bool>) -> Option<Message> do if flag do None else Some(Message {{body:Bytes.empty(),id:Bytes.empty()}}) end end\nfn leak(flag :: Plaintext<Bool>) do case choose(flag) do None -> println(\"empty\")\nSome(_) -> nil end end"));
}

#[test]
fn forward_declarations_and_global_aliases_do_not_invent_private_field_dependencies() {
    allowed(&format!("{MESSAGE}fn route() do let value = make()\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end\nfn make() -> Message do Message {{body:Bytes.empty(),id:Bytes.empty()}} end\nfn use_route() do route() end"));
    allowed(&format!("{MESSAGE}let value = make()\nfn route() do Http.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end\nfn make() -> Message do Message {{body:Bytes.empty(),id:Bytes.empty()}} end\nfn use_route() do route() end"));
    refused(&format!("{MESSAGE}fn route(body :: Plaintext<Bytes>) do let value = make(body)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end\nfn make(body :: Bytes) -> Message do Message {{body:body,id:body}} end"));
}

#[test]
fn unused_closures_check_their_own_declared_field_labels() {
    allowed(&format!("{MESSAGE}fn work() do let view = fn(value :: Message) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end\nnil end"));
    refused(&format!("{MESSAGE}fn work() do let leak = fn(value :: Message) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),value.body) end\nnil end"));
    refused(
        "fn work() do let leak = fn(value :: Plaintext<String>) -> println(value) end\nnil end",
    );
}

#[test]
fn structural_list_operations_preserve_public_lengths_and_item_fields() {
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do println(Int.to_string(List.length(values)))\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.get(values,0).id) end"));
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>, value :: Message) do let updated = List.append(values,value)\nprintln(Int.to_string(List.length(updated)))\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.get(List.reverse(List.take(List.drop(updated,0),1)),0).id) end"));
    refused(&format!("{MESSAGE}fn leak(values :: List<Message>, index :: Plaintext<Int>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),List.get(values,index).id) end"));
    refused(&format!("{MESSAGE}fn leak(values :: Plaintext<List<Message>>) do println(Int.to_string(List.length(values))) end"));
    refused(&format!("{MESSAGE}fn leak(body :: Plaintext<Bytes>, values :: List<Message>) do let updated = List.concat(values,[Message {{body:body,id:body}}])\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.get(updated,0).id) end"));
}

#[test]
fn map_callbacks_receive_the_item_shape_and_keep_their_result_dependencies() {
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do let _ = List.map(values,fn(value) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end)\nnil end"));
    allowed(&format!("{MESSAGE}fn route(values :: List<Message>) do let ids = List.map(values,fn(value) -> value.id end)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(ids)) end"));
    refused(&format!("{MESSAGE}fn leak(values :: List<Message>) do let bodies = List.map(values,fn(value) -> value.body end)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(bodies)) end"));
    refused(&format!("{MESSAGE}fn leak(values :: List<Message>) do let _ = List.map(values,fn(value) -> Http.body_bytes(Http.build(:post,\"https://example.test\"),value.body) end)\nnil end"));
}

#[test]
fn callback_returns_are_local_but_argument_question_marks_return_from_the_caller() {
    allowed("fn work(value :: Plaintext<Result<Int,String>>) do let callback = fn() do let _ = value?\nOk(0) end\nlet _ = callback()\nprintln(\"fixed\") end");
    refused("fn leak(value :: Plaintext<Result<Int,String>>) -> Result<Int,String> do let callback = fn(number :: Int) -> 0 end\nlet _ = callback(value?)\nprintln(\"fixed\")\nOk(0) end");
}

#[test]
fn a_known_closure_can_seal_its_capture_but_a_private_function_choice_stays_unproven() {
    allowed("fn seal(key :: X25519PublicKey, value :: Plaintext<Bytes>) do let encrypt = fn() -> Crypto.hpke_seal(key,Bytes.empty(),Bytes.empty(),value) end\nencrypt() end");
    refused("fn leak(flag :: Plaintext<Bool>) do let transmit = if flag do fn() -> println(\"yes\") end else fn() -> println(\"no\") end end\ntransmit() end");
}

#[test]
fn discriminant_queries_and_default_payloads_do_not_read_an_unused_error_payload() {
    allowed("fn route(value :: Result<Plaintext<Bytes>,String>) do if Result.is_ok(value) do println(\"ok\") else println(\"error\") end end");
    allowed("fn route(value :: Result<Bytes,Plaintext<String>>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(value,Bytes.empty())) end");
    refused("fn leak(value :: Plaintext<Result<Int,String>>) do if Result.is_err(value) do println(\"error\") else nil end end");
    refused("fn leak(value :: Result<Plaintext<Bytes>,String>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(value,Bytes.empty())) end");
}

#[test]
fn a_whole_value_label_also_covers_projections_beyond_the_shape_expansion_budget() {
    let leaf = "struct Leaf do a :: Bytes\nb :: Bytes\nc :: Bytes\nd :: Bytes\ne :: Bytes\nf :: Bytes\ng :: Bytes\nid :: Bytes end\n";
    let fields = (0..80)
        .map(|index| format!("f{index} :: Leaf"))
        .collect::<Vec<_>>()
        .join("\n");
    let values = (0..80)
        .map(|index| format!("f{index}: leaf"))
        .collect::<Vec<_>>()
        .join(",");
    refused(&format!("{leaf}struct Wide do {fields} end\nfn leak(leaf :: Leaf) do let value :: Plaintext<Wide> = Wide {{{values}}}\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),value.f79.id) end"));
}

#[test]
fn an_explicit_label_is_a_new_boundary_after_a_reviewed_declassification() {
    refused("fn leak(value :: Plaintext<Bytes>) do let fresh :: Plaintext<Bytes> = declassify(value,\"test disclosure\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),fresh) end");
    refused("fn fresh(value :: Bytes) -> Plaintext<Bytes> do declassify(value,\"test disclosure\") end\nfn leak(value :: Bytes) do Http.body_bytes(Http.build(:post,\"https://example.test\"),fresh(value)) end");
}

#[test]
fn an_early_return_payload_does_not_control_the_public_success_path() {
    allowed("fn route(value :: Result<Bytes,Plaintext<String>>) -> Result<Int,String> do let body = case value do Err(error) -> return Err(error)\nOk(bytes) -> bytes end\nlet _ = Http.body_bytes(Http.build(:post,\"https://example.test\"),body)\nOk(0) end");
    refused("fn leak(value :: Plaintext<Result<Bytes,String>>) -> Result<Int,String> do let body = case value do Err(error) -> return Err(error)\nOk(bytes) -> bytes end\nlet _ = Http.body_bytes(Http.build(:post,\"https://example.test\"),body)\nOk(0) end");
}

#[test]
fn repeated_closure_calls_do_not_reuse_a_public_projection_for_private_arguments() {
    refused(&format!("{MESSAGE}fn forward(value :: Message) -> Message do value end\nfn leak(body :: Plaintext<Bytes>) do let route = fn(value :: Message) do let forwarded = forward(value)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),forwarded.id) end\nlet _ = route(Message {{body:body,id:Bytes.empty()}})\nroute(Message {{body:Bytes.empty(),id:body}}) end"));
}

#[test]
fn recursive_and_global_forwarding_converge_before_a_public_projection_is_trusted() {
    refused(&format!("{MESSAGE}fn entry(body :: Plaintext<Bytes>) do let value = first(2,Message {{body:body,id:Bytes.empty()}})\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),value.id) end\nfn first(left :: Int,value :: Message) -> Message do if left > 0 do second(left - 1,value) else value end end\nfn second(left :: Int,value :: Message) -> Message do first(left,%{{value | id: value.body}}) end"));
    refused(&format!("{MESSAGE}let current = first()\nfn entry() do Http.body_bytes(Http.build(:post,\"https://example.test\"),current.id) end\nfn first() -> Message do second() end\nfn second() -> Message do let body :: Plaintext<Bytes> = Bytes.empty()\nMessage {{body:body,id:body}} end"));
}

#[test]
fn local_secret_destruction_is_not_a_plaintext_exit_or_a_privilege_for_a_lookalike() {
    allowed("fn cleanup(flag :: Plaintext<Bool>, key :: consume SecretBytes) do if flag do Secret.destroy(key) else Secret.destroy(key) end end");
    refused("module Secret do fn destroy(key :: consume SecretBytes) do println(\"fixed\") end end\nfn leak(flag :: Plaintext<Bool>, key :: consume SecretBytes) do if flag do Secret.destroy(key) else Secret.destroy(key) end end");
    refused("fn leak(flag :: Plaintext<Bool>, key :: consume SecretBytes) do let done = if flag do Secret.destroy(key) else Secret.destroy(key) end\nif done == nil do println(\"fixed\") else nil end end");
}

#[test]
fn empty_list_constructors_keep_public_size_and_fields_through_typed_accumulators() {
    for constructor in ["List.new()", "list_new()", "[]"] {
        allowed(&format!("{MESSAGE}fn route(value :: Message) do let empty :: List<Message> = {constructor}\nprintln(Int.to_string(List.length(empty)))\nlet values = List.append(empty,value)\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(values).id) end"));
        allowed(&format!("{MESSAGE}fn parsed(value :: Bytes) -> Message!String do Ok(Message {{body:value,id:Bytes.empty()}}) end\nfn decode(values :: List<Bytes>, index :: Int, output :: List<Message>) -> List<Message>!String do if index >= List.length(values) do Ok(output) else decode(values,index + 1,List.append(output,parsed(List.get(values,index))?)) end end\nfn route(values :: List<Bytes>) -> Result<Int,String> do let decoded = decode(values,0,{constructor})?\nprintln(Int.to_string(List.length(decoded)))\nOk(0) end"));
    }
    refused(&format!("{MESSAGE}fn leak(value :: Message, flag :: Plaintext<Bool>) do let values = if flag do List.new() else [value] end\nprintln(Int.to_string(List.length(values))) end"));
    refused(&format!("{MESSAGE}fn leak(value :: Message) do let empty :: Plaintext<List<Message>> = List.new()\nprintln(Int.to_string(List.length(empty))) end"));
    refused(&format!("{MESSAGE}fn leak(body :: Plaintext<Bytes>) do let empty :: List<Message> = List.new()\nlet values = List.append(empty,Message {{body:body,id:body}})\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),List.head(values).id) end"));
    refused("module List do fn new() -> List<Int> do let flag :: Plaintext<Bool> = true\nif flag do [1] else [] end end end\nfn leak() do println(Int.to_string(List.length(List.new()))) end");
}
