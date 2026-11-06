//! Public plaintext-flow contracts. An unrelated ordinary type error never
//! satisfies a refused-exit assertion: the dedicated flow diagnostic is required.

use mesh_typeck::{ImportContext, ModuleExports, TypeckResult};

fn checked(source: &str) -> TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(
        parsed.ok(),
        "parse errors for {source:?}: {:?}",
        parsed.errors()
    );
    mesh_typeck::check(&parsed)
}

fn refused(source: &str) {
    let result = checked(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| format!("{error:?}").contains("PlaintextViolation")),
        "expected a plaintext-flow diagnostic for {source:?}, got {:?}",
        result.errors
    );
}

fn allowed(source: &str) {
    let result = checked(source);
    assert!(
        result.errors.is_empty(),
        "rejected {source:?}: {:?}",
        result.errors
    );
}

#[test]
fn logging_panics_serialization_push_and_unknown_native_are_refused_exits() {
    for source in [
        "fn leak(value :: Plaintext<String>) do println(value) end",
        "fn leak(value :: Plaintext<String>) do panic(value) end",
        "fn leak(value :: Plaintext<String>) do Json.encode_string(value) end",
        "fn leak(value :: Plaintext<Bytes>) do Host.background_schedule(value) end",
        "@native(\"plain_native_sink\") pub fn native_sink(value :: String) -> Int\nfn leak(value :: Plaintext<String>) do native_sink(value) end",
    ] {
        refused(source);
    }
}

#[test]
fn network_urls_headers_bodies_and_remote_actor_messages_are_refused() {
    for source in [
        "fn leak(value :: Plaintext<String>) do Http.build(:post, value) end",
        "fn leak(value :: Plaintext<String>) do Http.header(Http.build(:post, \"https://example.test\"), \"X-Text\", value) end",
        "fn leak(value :: Plaintext<Bytes>) do Http.body_bytes(Http.build(:post, \"https://example.test\"), value) end",
        "fn leak(options :: Int, value :: Plaintext<String>) do WsClient.header(options, \"Authorization\", value) end",
        "fn leak(pid :: Pid<String>, value :: Plaintext<String>) do send(pid, value) end",
    ] {
        refused(source);
    }
}

#[test]
fn slices_interpolation_and_local_forwarding_keep_the_label() {
    for source in [
        "fn leak(value :: Plaintext<String>) do println(String.slice(value, 0, 1)) end",
        "fn leak(value :: Plaintext<String>) do let text :: String = \"body: #{value}\"\nprintln(text) end",
        "fn forward(value :: String) do String.slice(value, 0, 1) end\nfn leak(value :: Plaintext<String>) do println(forward(value)) end",
        "fn encode(value :: String) do Json.encode_string(value) end\nfn leak(value :: Plaintext<String>) do encode(value) end",
        "fn aead_seal(value :: String) do println(value) end\nfn leak(value :: Plaintext<String>) do aead_seal(value) end",
    ] {
        refused(source);
    }
}

#[test]
fn control_flow_cannot_disclose_plaintext_through_public_constants() {
    for source in [
        "fn leak(flag :: Plaintext<Bool>) do let answer = if flag do \"yes\" else \"no\" end\nprintln(answer) end",
        "fn leak(flag :: Plaintext<Bool>) do if flag do println(\"fixed\") else nil end end",
        "fn leak(value :: Plaintext<String>) do for index in 0..String.length(value) do println(\"fixed\") end end",
    ] {
        refused(source);
    }
}

#[test]
fn nested_records_variants_and_destructuring_keep_the_label() {
    refused(
        r#"
struct Message do
  body :: Plaintext<String>
end
fn leak(value :: Option<List<Message>>) do
  let text = case value do
    Some(messages) -> List.head(messages).body
    None -> "empty"
  end
  println(text)
end
"#,
    );
    refused(
        r#"
fn leak(value :: Plaintext<String>) do
  let wrapped = Some(([value], 7))
  case wrapped do
    Some((items, count)) -> println(List.head(items))
    None -> nil
  end
end
"#,
    );
}

#[test]
fn genuine_seals_accept_only_the_plaintext_data_argument() {
    allowed("fn sealed(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, value :: Plaintext<Bytes>) -> Result<Bytes, CryptoError> do Crypto.aead_seal(key, nonce, aad, value) end");
    allowed("fn sealed(key :: X25519PublicKey, info :: Bytes, aad :: Bytes, value :: Plaintext<Bytes>) -> Result<Bytes, CryptoError> do Crypto.hpke_seal(key, info, aad, value) end");
    allowed("fn stored(value :: Plaintext<Bytes>, key :: borrow StorageKey, context :: Bytes) -> Result<Bytes, CryptoError> do StorageKey.seal_bytes(value, key, context) end");
    refused("fn leak(key :: borrow AeadKey, nonce :: Plaintext<Bytes>, value :: Bytes) do Crypto.aead_seal(key, nonce, Bytes.empty(), value) end");
    refused("fn leak(key :: borrow AeadKey, nonce :: Bytes, aad :: Plaintext<Bytes>, value :: Bytes) do Crypto.aead_seal(key, nonce, aad, value) end");
    refused("fn leak(key :: X25519PublicKey, info :: Plaintext<Bytes>, value :: Bytes) do Crypto.hpke_seal(key, info, Bytes.empty(), value) end");
    refused("fn leak(value :: Bytes, key :: borrow StorageKey, context :: Plaintext<Bytes>) do StorageKey.seal_bytes(value, key, context) end");
}

#[test]
fn an_import_with_the_same_base_name_as_crypto_is_not_a_sealing_capability() {
    let dependency = mesh_parser::parse("pub fn aead_seal(value :: String) do println(value) end");
    let result = mesh_typeck::check(&dependency);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &result);
    let mut context = ImportContext::empty();
    context.module_exports.insert(
        "Untrusted.Crypto".into(),
        ModuleExports::new("Untrusted.Crypto".into(), &exports),
    );
    let source = mesh_parser::parse("import Untrusted.Crypto\nfn leak(value :: Plaintext<String>) do Crypto.aead_seal(value) end");
    assert!(source.ok(), "{:?}", source.errors());
    let result = mesh_typeck::check_with_imports(&source, &context);
    assert!(
        result
            .errors
            .iter()
            .any(|error| format!("{error:?}").contains("PlaintextViolation")),
        "{:?}",
        result.errors
    );
}

#[test]
fn declassification_requires_an_explicit_nonempty_literal_reason() {
    for source in [
        "fn leak(value :: Plaintext<String>) do declassify(value) end",
        "fn leak(value :: Plaintext<String>) do declassify(value, \"\") end",
        "fn leak(value :: Plaintext<String>) do declassify(value, \"   \") end",
        "fn leak(value :: Plaintext<String>, reason :: String) do declassify(value, reason) end",
        "fn leak(value :: Plaintext<String>) do declassify(value, \"\\n\") end",
        "fn leak(value :: Plaintext<String>) do let disclose = declassify\ndisclose(value, \"indirect disclosure\") end",
    ] {
        refused(source);
    }
    allowed("fn preview(value :: Plaintext<String>) do println(declassify(value, \"user enabled notification preview\")) end");
    allowed("struct Message do body :: Plaintext<String> end\nfn preview(value :: Message) do println(declassify(value, \"user enabled full record preview\").body) end");
}

#[test]
fn labels_are_copyable_and_pure_computation_can_return_labeled_values() {
    allowed("fn copy(value :: Plaintext<String>) do (value, value) end");
    allowed(
        "fn derive(value :: Plaintext<String>) do String.length(String.slice(value, 0, 1)) end",
    );
    allowed("fn derive(value :: Plaintext<String>) do value.slice(0, 1).length() end");
}

#[test]
fn unlabeled_programs_keep_their_existing_behavior() {
    allowed("fn message(value :: String) do println(value) end\nfn payload(value :: String) do Json.encode_string(value) end");
    allowed("fn encode(prefix :: String, value :: String) do Json.encode_string(prefix <> value) end\nfn payload(value :: String) do value |2> encode(\"prefix\") end");
}

#[test]
fn while_loops_slot_pipes_and_captured_function_values_do_not_launder_labels() {
    for source in [
        "fn leak(flag :: Plaintext<Bool>) do while flag do println(\"fixed\") end end",
        "fn forward(prefix :: String, value :: String) do println(prefix <> value) end\nfn leak(value :: Plaintext<String>) do value |2> forward(\"prefix\") end",
        "fn leak(value :: Plaintext<String>) do let read = fn() -> value end\nprintln(read()) end",
        "fn leak(flag :: Plaintext<Bool>) do if flag do return nil end\nprintln(\"fixed\") end",
        "fn leak(value :: Plaintext<Result<Int, String>>) -> Result<Int, String> do let _ = value?\nprintln(\"fixed\")\nOk(0) end",
    ] {
        refused(source);
    }
}

#[test]
fn higher_order_exit_keeps_plaintext_captured_inside_a_closure() {
    refused(
        r#"
@native("host_callback") pub fn callback(value :: String) -> Int
fn leak(value :: Plaintext<String>) do
  let secret = fn() -> callback(value) end
  secret()
end
"#,
    );
}

#[test]
fn local_namespace_lookalikes_do_not_acquire_builtin_sealing_trust() {
    refused(
        r#"
module Crypto do
  pub fn aead_seal(key :: Int, nonce :: Int, aad :: Int, value :: String) do
    println(value)
  end
end
fn leak(value :: Plaintext<String>) do Crypto.aead_seal(0, 0, 0, value) end
"#,
    );
}

#[test]
fn only_reviewed_display_exports_can_return_plaintext_to_the_native_host() {
    refused("@export(\"message_body\") pub fn body(value :: Plaintext<Bytes>) -> Result<Bytes, String> do Ok(value) end");
    for source in [
        "@display(\"chat history text shown to the user\")\n@export(\"message_body\") pub fn body(value :: Plaintext<Bytes>) -> Result<Bytes, String> do Ok(value) end",
        "@export(\"message_body\")\n@display(\"chat history text shown to the user\") pub fn body(value :: Plaintext<Bytes>) -> Result<Bytes, String> do Ok(value) end",
    ] {
        let result = checked(source);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.plaintext.sites.len(), 1);
        assert_eq!(result.plaintext.sites[0].kind, "display");
        assert_eq!(result.plaintext.sites[0].reason, "chat history text shown to the user");
    }
    refused("@display(\"   \")\n@export(\"message_body\") pub fn body(value :: Plaintext<Bytes>) -> Result<Bytes, String> do Ok(value) end");
    refused(
        "@display(\"not an export\") fn body(value :: Plaintext<String>) -> String do value end",
    );
    refused("@display(\"display cannot authorize logging\")\n@export(\"message_body\") pub fn body(value :: Plaintext<Bytes>) -> Result<Bytes, String> do Json.encode_string(Bytes.to_hex(value))\nOk(value) end");
}

#[test]
fn audit_sites_include_unused_functions_and_unreachable_calls_deterministically() {
    let result = checked("fn unused(value :: Plaintext<String>) do declassify(value, \"unused reviewed site\") end\nfn main() do if false do declassify(\"fixed\", \"unreachable reviewed site\") end end");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result
            .plaintext
            .sites
            .iter()
            .map(|site| site.reason.as_str())
            .collect::<Vec<_>>(),
        ["unused reviewed site", "unreachable reviewed site"]
    );
}

#[test]
fn global_labels_and_higher_order_callbacks_cannot_skip_flow_analysis() {
    for source in [
        "let text :: Plaintext<String> = \"private\"\nprintln(text)",
        "let text :: Plaintext<String> = \"private\"\nfn main() do println(text) end",
        "fn log(value :: String) do println(value) end\nfn leak(value :: Plaintext<List<String>>) do List.map(value, log) end",
        "fn leak(value :: Plaintext<List<String>>) do List.map(value, fn(item) -> println(item) end) end",
        "module Private do let text :: Plaintext<String> = \"private\"\nfn leak() do println(text) end end",
    ] { refused(source); }
    allowed("fn length(value :: Plaintext<List<String>>) do List.map(value, fn(item) -> String.length(item) end) end");
    allowed("fn length(value :: Plaintext<String>) do let size = fn(item) -> String.length(item) end\nsize(value) end");
}

#[test]
fn proven_local_actor_messages_are_checked_against_the_receiver_body() {
    allowed("actor sink() do receive do text -> String.length(text) end end\nfn work(value :: Plaintext<String>) do let pid = spawn(sink)\nsend(pid, value) end");
    refused("actor sink() do receive do text -> println(text) end end\nfn work(value :: Plaintext<String>) do let pid = spawn(sink)\nsend(pid, value) end");
    refused("actor sink(text :: String) do println(text) end\nfn work(value :: Plaintext<String>) do spawn(sink, value) end");
}

#[test]
fn imported_actor_locality_requires_the_checked_mailbox_effect_proof() {
    for (body, expected) in [("String.length(text)", false), ("println(text)", true)] {
        let dependency = mesh_parser::parse(&format!(
            "actor sink() do receive do text -> {body} end end"
        ));
        let check = mesh_typeck::check(&dependency);
        assert!(check.errors.is_empty(), "{:?}", check.errors);
        let exports = mesh_typeck::collect_exports(&dependency, &check);
        let mut context = ImportContext::empty();
        context.module_exports.insert(
            "Workers".into(),
            ModuleExports::new("Workers".into(), &exports),
        );
        let source = mesh_parser::parse("from Workers import sink\nfn work(value :: Plaintext<String>) do let pid = spawn(sink)\nsend(pid,value) end");
        assert!(source.ok(), "{:?}", source.errors());
        let check = mesh_typeck::check_with_imports(&source, &context);
        assert_eq!(
            check.errors.iter().any(|error| matches!(
                error,
                mesh_typeck::error::TypeError::PlaintextViolation { .. }
            )),
            expected,
            "{:?}",
            check.errors
        );
        if !expected {
            assert!(check.errors.is_empty(), "{:?}", check.errors);
        }
    }
}
