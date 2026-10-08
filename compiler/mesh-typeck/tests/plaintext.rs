//! `Plaintext<T>`: message content that may leave the program only through
//! a seal, an `@display` export or a `declassify` with a written reason.

use mesh_typeck::diagnostics::{render_diagnostic, DiagnosticOptions};
use mesh_typeck::error::TypeError;
use mesh_typeck::{ImportContext, ModuleExports, TypeckResult};

fn check_source(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "parse errors: {:?}", parse.errors());
    mesh_typeck::check(&parse)
}

fn violations(result: &TypeckResult) -> Vec<&str> {
    result
        .errors
        .iter()
        .filter_map(|error| match error {
            TypeError::PlaintextViolation { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect()
}

fn rendered(source: &str, result: &TypeckResult) -> String {
    result
        .errors
        .iter()
        .map(|error| {
            render_diagnostic(
                error,
                source,
                "main.mpl",
                &DiagnosticOptions::colorless(),
                None,
            )
        })
        .collect()
}

fn assert_clean(source: &str) -> TypeckResult {
    let result = check_source(source);
    assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
    result
}

/// `source` is refused, with a diagnostic that says what may take plaintext.
fn assert_refused(label: &str, source: &str) {
    let result = check_source(source);
    assert!(!result.errors.is_empty(), "{label}: accepted\n{source}");
    let text = rendered(source, &result);
    assert!(
        text.contains("Plaintext") && text.contains("declassify"),
        "{label}: the diagnostic does not explain plaintext's exits\n{text}"
    );
}

const BODY: &str = "fn body() -> Plaintext<String> do\n  Plaintext.from(\"hello\")\nend\nfn bytes() -> Plaintext<Bytes> do\n  Plaintext.from(Bytes.from_utf8(\"hello\"))\nend\n";

/// Helpers the derivations below go through.
const DERIVING: &str = "struct Note do\n  text :: Plaintext<String>\nend\nfn field_of(note :: Note) do\n  note.text\nend\nfn through_closure(text) do\n  let later = fn() -> Plaintext.map(text, fn(value) -> value <> \"!\" end) end\n  later()\nend\n";

#[test]
fn every_ordinary_exit_refuses_plaintext() {
    for (label, body) in [
        ("log", "println(body())"),
        ("stderr", "IO.eprintln(body())"),
        ("host log", "Host.log_redacted(bytes())"),
        ("panic", "panic(body())"),
        ("interpolation", "let shown = \"body: #{body()}\"\n  shown"),
        ("json", "Json.encode(body())"),
        ("http url", "Http.build(:get, body())"),
        (
            "http header",
            "Http.build(:post, \"https://example.com\") |> Http.header(\"x-body\", body())",
        ),
        (
            "http body",
            "Http.build(:post, \"https://example.com\") |> Http.body(body())",
        ),
        (
            "push payload",
            "Http.build(:post, \"https://push.example.com\") |> Http.json(body())",
        ),
        ("websocket server", "Ws.send(1, body())"),
        ("websocket client", "WsClient.send_bytes(1, bytes())"),
        ("file", "File.write(\"/tmp/x\", body())"),
        ("comparison", "body() == body()"),
        ("hash", "Crypto.sha256(bytes())"),
    ] {
        let source = format!("{BODY}fn leak() do\n  {body}\nend\n");
        assert_refused(label, &source);
    }
}

/// A generic type's derived impls, a map's keys and `List.contains` would
/// show or compare plaintext without a type error of their own.
#[test]
fn plaintext_is_not_shown_or_compared_behind_a_generic() {
    let boxed = "struct Boxed<T> do\n  value :: T\nend deriving(Display, Eq, Hash)\n";
    for (label, body) in [
        ("a derived Display", "\"#{Boxed { value: body() }}\""),
        (
            "a derived Eq",
            "Boxed { value: body() } == Boxed { value: body() }",
        ),
        (
            "a map keyed by plaintext",
            "let keys = Map.from_list([(body(), 1)])\n  Map.has_key(keys, body())",
        ),
        (
            "a map literal keyed by plaintext",
            "let keys = %{body() => 1}\n  Map.size(keys)",
        ),
        (
            "a set of plaintext",
            "let seen = Set.from_list([body()])\n  Set.size(seen)",
        ),
        ("List.contains", "List.contains([body()], body())"),
    ] {
        let source = format!("{BODY}{boxed}fn leak() do\n  {body}\nend\n");
        let result = check_source(&source);
        assert!(
            !violations(&result).is_empty(),
            "{label}: {:?}",
            result.errors
        );
    }
    // A generic type holding plaintext is fine where nothing shows it.
    assert_clean(&format!(
        "{BODY}{boxed}fn keep() do\n  let boxed = Boxed {{ value: body() }}\n  boxed.value\nend\n"
    ));
}

#[test]
fn a_type_holding_plaintext_derives_nothing_that_shows_or_serializes_it() {
    for derive in ["Json", "Display", "Debug", "Row", "Eq", "Ord", "Hash"] {
        let source =
            format!("struct Note do\n  text :: Plaintext<String>\nend deriving({derive})\n");
        let result = check_source(&source);
        assert!(
            violations(&result)
                .iter()
                .any(|reason| reason.contains(&format!("cannot derive `{derive}`"))),
            "{derive}: {:?}",
            result.errors
        );
    }
    let sum = "type Event do\n  Said(Plaintext<String>)\n  Left\nend deriving(Json)\n";
    assert_eq!(
        violations(&check_source(sum)),
        ["`Event` holds plaintext and cannot derive `Json`"]
    );
    // Through another type: a record holding a record that holds plaintext.
    let nested = "struct Note do\n  text :: Plaintext<String>\nend\nstruct Thread do\n  first :: Note\nend deriving(Display)\n";
    assert_eq!(
        violations(&check_source(nested)),
        ["`Thread` holds plaintext and cannot derive `Display`"]
    );
}

#[test]
fn a_type_holding_plaintext_gets_no_default_derives() {
    let declaration = "struct Note do\n  text :: Plaintext<String>\n  id :: Int\nend\n";
    assert_clean(declaration);
    for (label, body) in [
        ("interpolated", "\"#{note}\""),
        ("inspected", "inspect(note)"),
        ("compared", "note == note"),
        ("encoded", "Json.encode(note)"),
    ] {
        let source = format!("{declaration}fn leak(note :: Note) do\n  {body}\nend\n");
        let result = check_source(&source);
        assert!(!result.errors.is_empty(), "{label}: accepted");
    }
}

#[test]
fn values_derived_from_plaintext_stay_plaintext() {
    let derivations = [
        (
            "sliced",
            "Plaintext.map(body(), fn(text) -> String.slice(text, 0, 2) end)",
        ),
        (
            "concatenated",
            "Plaintext.map2(body(), body(), fn(a, b) -> a <> b end)",
        ),
        (
            "trimmed",
            "Plaintext.map(body(), fn(text) -> String.trim(text) end)",
        ),
        (
            "split",
            "Plaintext.map(body(), fn(text) -> String.split(text, \" \") end)",
        ),
        (
            "measured",
            "Plaintext.map(body(), fn(text) -> String.length(text) end)",
        ),
        (
            "compared",
            "Plaintext.map2(body(), body(), fn(a, b) -> a == b end)",
        ),
        ("in a record", "Note { text: body() }"),
        ("from a record's field", "field_of(Note { text: body() })"),
        ("through a closure", "through_closure(body())"),
        ("in a list", "[body()]"),
        ("in an option", "Some(body())"),
    ];
    for (label, derived) in derivations {
        let source = format!(
            "{BODY}{DERIVING}fn leak() do\n  let derived = {derived}\n  println(\"#{{derived}}\")\nend\n"
        );
        assert_refused(label, &source);
    }
    // The same derivations check when nothing leaves.
    let kept: String = derivations
        .iter()
        .enumerate()
        .map(|(index, (_, derived))| format!("  let kept_{index} = {derived}\n"))
        .collect();
    assert_clean(&format!("{BODY}{DERIVING}fn keep() do\n{kept}  0\nend\n"));
}

#[test]
fn lifted_functions_must_have_no_exits() {
    let helpers = "fn shout(text :: String) -> String do\n  String.to_upper(text)\nend\nfn noisy(text :: String) -> String do\n  println(text)\n  text\nend\nfn louder(text :: String) -> String do\n  shout(noisy(text))\nend\n";
    assert_clean(&format!(
        "{BODY}{helpers}fn lift() -> Plaintext<String> do\n  Plaintext.map(body(), shout)\nend\n"
    ));
    for (label, lifted, reason) in [
        (
            "a closure that logs",
            "Plaintext.map(body(), fn(text) -> noisy(text) end)",
            "calls `noisy`, which has an exit",
        ),
        (
            "a named function that logs",
            "Plaintext.map(body(), noisy)",
            "`noisy` has an exit: it calls `println`",
        ),
        (
            "a function that calls one that logs",
            "Plaintext.map(body(), louder)",
            "`louder` has an exit: it calls `noisy`, which has an exit",
        ),
        (
            "a closure that panics",
            "Plaintext.map(body(), fn(text) -> panic(text) end)",
            "calls `panic`",
        ),
        (
            "a closure that sends",
            "Plaintext.map(body(), fn(text) -> send(self(), text) end)",
            "sends a message",
        ),
        (
            "a closure that writes a file",
            "Plaintext.map(body(), fn(text) -> File.write(\"/tmp/x\", text) end)",
            "calls `File.write`",
        ),
        (
            "a standard library function named directly",
            "Plaintext.map(body(), String.trim)",
            "the function `Plaintext.map` applies must be a closure written here or a named function",
        ),
    ] {
        let source = format!("{BODY}{helpers}fn lift() do\n  {lifted}\nend\n");
        let result = check_source(&source);
        assert!(
            violations(&result)
                .iter()
                .any(|found| found.contains(reason)),
            "{label}: expected `{reason}`, got {:?}",
            result.errors
        );
    }
}

#[test]
fn lifted_functions_cannot_run_code_the_checker_cannot_see() {
    for (label, source) in [
        (
            "a function passed in",
            "fn apply(f, text :: String) -> String do\n  f(text)\nend\nfn lift(f) do\n  Plaintext.map(body(), fn(text) -> apply(f, text) end)\nend\n",
        ),
        (
            "a user impl of Display",
            "struct Shown do\n  text :: String\nend\nimpl Display for Shown do\n  fn to_string(self) -> String do\n    println(self.text)\n    self.text\n  end\nend\nfn lift() do\n  Plaintext.map(body(), fn(text) -> \"#{Shown { text: text }}\" end)\nend\n",
        ),
        (
            "Plaintext.map as a value",
            "fn lift() do\n  let apply = Plaintext.map\n  apply(body(), fn(text) -> text end)\nend\n",
        ),
    ] {
        let source = format!("{BODY}{source}");
        let result = check_source(&source);
        assert!(
            !violations(&result).is_empty(),
            "{label}: {:?}",
            result.errors
        );
    }
}

#[test]
fn lifted_functions_from_other_modules_carry_their_purity() {
    let module = "pub fn shout(text :: String) -> String do\n  String.to_upper(text)\nend\npub fn noisy(text :: String) -> String do\n  println(text)\n  text\nend\n";
    let module_parse = mesh_parser::parse(module);
    let module_check = mesh_typeck::check(&module_parse);
    assert!(module_check.errors.is_empty(), "{:?}", module_check.errors);
    let exports = mesh_typeck::collect_exports(&module_parse, &module_check);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Words".to_string(),
        ModuleExports::new("Words".to_string(), &exports),
    );
    let check = |body: &str| {
        let source = format!("import Words\n{BODY}fn lift() do\n  {body}\nend\n");
        mesh_typeck::check_with_imports(&mesh_parser::parse(&source), &imports)
    };
    let pure = check("Plaintext.map(body(), Words.shout)");
    assert!(pure.errors.is_empty(), "{:?}", pure.errors);
    let impure = check("Plaintext.map(body(), fn(text) -> Words.noisy(text) end)");
    assert_eq!(
        violations(&impure),
        ["`Plaintext.map` needs a function with no exits: it calls `Words.noisy`, which has an exit"]
    );
    // Imported by name, the same.
    let check_from = |body: &str| {
        let source = format!("from Words import shout, noisy\n{BODY}fn lift() do\n  {body}\nend\n");
        mesh_typeck::check_with_imports(&mesh_parser::parse(&source), &imports)
    };
    let pure = check_from("Plaintext.map(body(), shout)");
    assert!(pure.errors.is_empty(), "{:?}", pure.errors);
    assert_eq!(
        violations(&check_from("Plaintext.map(body(), noisy)")),
        ["`Plaintext.map` needs a function with no exits: `noisy` has an exit"]
    );
}

const KEEPER: &str = "actor keeper(text :: Plaintext<String>) do\n  receive do\n    next -> keeper(next)\n  end\nend\n";

#[test]
fn plaintext_goes_only_to_the_programs_own_actors() {
    assert_clean(&format!(
        "{BODY}{KEEPER}fn keep() do\n  let pid = spawn(keeper, body())\n  send(pid, body())\n  Timer.send_after(pid, 10, body())\nend\n"
    ));
    // Registering or monitoring such a pid hands out no way to send to it:
    // what a lookup gives back is untyped, and takes no plaintext.
    assert_clean(&format!(
        "{BODY}{KEEPER}actor watcher() do\n  let pid = spawn(keeper, body())\n  Process.register(\"keeper\", pid)\n  Global.register(\"keeper\", pid)\n  Process.monitor(pid, 1)\n  receive do\n    n -> watcher()\n  end\nend\n"
    ));
    for (label, body, reason) in [
        (
            "an untyped pid",
            "send(Process.whereis(\"keeper\"), body())",
            "not to an untyped `Pid`",
        ),
        (
            "a pid looked up in the global registry",
            "let pid :: Pid<Plaintext<String>> = Global.whereis(\"keeper\")\n  send(pid, body())",
            "do not convert",
        ),
        (
            "a pid looked up locally",
            "let pid :: Pid<Plaintext<String>> = Process.whereis(\"keeper\")\n  send(pid, body())",
            "do not convert",
        ),
        (
            "a remote actor",
            "Node.spawn(\"n@h\", keeper, body())",
            "`Node.spawn` starts an actor on another node",
        ),
        (
            "a remote actor, piped",
            "body() |3> Node.spawn(\"n@h\", keeper)",
            "`Node.spawn` starts an actor on another node",
        ),
    ] {
        let source = format!("{BODY}{KEEPER}fn leak() do\n  {body}\nend\n");
        let result = check_source(&source);
        assert!(
            violations(&result)
                .iter()
                .any(|found| found.contains(reason)),
            "{label}: expected `{reason}`, got {:?}",
            result.errors
        );
    }
}

#[test]
fn a_service_taking_plaintext_is_reached_only_through_its_own_pid() {
    let vault = "service Vault do\n  fn init() -> Int do\n    0\n  end\n  cast Put(text :: Plaintext<String>) do |count|\n    count + 1\n  end\nend\n";
    assert_clean(&format!(
        "{BODY}{vault}fn keep() do\n  let pid = Vault.start()\n  Vault.put(pid, body())\nend\n"
    ));
    let source = format!(
        "{BODY}{vault}fn leak() do\n  Vault.put(Process.whereis(\"vault\"), body())\nend\n"
    );
    let result = check_source(&source);
    assert!(
        violations(&result)
            .iter()
            .any(|found| found.contains("do not convert")),
        "{:?}",
        result.errors
    );
}

#[test]
fn declassify_needs_a_written_reason() {
    for (label, call, reason) in [
        (
            "no reason",
            "declassify(body())",
            "`declassify` takes the value and a reason",
        ),
        (
            "a computed reason",
            "let why = \"because\"\n  declassify(body(), why)",
            "the reason given to `declassify` must be a string literal",
        ),
        (
            "an empty reason",
            "declassify(body(), \"  \")",
            "the reason given to `declassify` must not be empty",
        ),
        (
            "an interpolated reason",
            "let n = 1\n  declassify(body(), \"case #{n}\")",
            "the reason given to `declassify` must be a string literal",
        ),
        (
            "piped",
            "body() |> declassify(\"piped\")",
            "`declassify` is called directly",
        ),
        (
            "as a value",
            "let reveal = declassify\n  reveal(body(), \"hidden\")",
            "`declassify` is called directly",
        ),
    ] {
        let source = format!("{BODY}fn reveal() do\n  {call}\nend\n");
        let result = check_source(&source);
        assert!(
            violations(&result)
                .iter()
                .any(|found| found.contains(reason)),
            "{label}: expected `{reason}`, got {:?}",
            result.errors
        );
    }
    let defined = check_source("fn declassify(x :: Int) -> Int do\n  x\nend\n");
    assert_eq!(
        violations(&defined),
        ["`declassify` is the language's declassification and cannot be redefined"]
    );
    for binding in [
        "fn f() do\n  let declassify = 1\n  declassify\nend\n",
        "fn f(declassify :: Int) -> Int do\n  declassify\nend\n",
        "fn f(pair :: (Int, Int)) -> Int do\n  let (declassify, _) = pair\n  declassify\nend\n",
    ] {
        assert_eq!(
            violations(&check_source(binding)),
            ["`declassify` is the language's declassification and cannot be redefined"],
            "{binding}"
        );
    }
}

#[test]
fn declassify_with_a_reason_is_recorded() {
    let result = assert_clean(&format!(
        "{BODY}fn bucket() -> Int do\n  declassify(Plaintext.map(body(), fn(text) -> String.length(text) / 16 end), \"padding bucket\")\nend\n"
    ));
    let sites: Vec<(&str, &str)> = result
        .plaintext
        .declassify_sites
        .iter()
        .map(|site| (site.function.as_str(), site.reason.as_str()))
        .collect();
    assert_eq!(sites, [("bucket", "padding bucket")]);
}

#[test]
fn seals_are_the_exits_for_content() {
    assert_clean(
        r#"
fn seal(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, body :: Plaintext<Bytes>) -> Result<Bytes, CryptoError> do
  Crypto.aead_seal_plaintext(key, nonce, aad, body)
end
fn open(key :: borrow AeadKey, nonce :: Bytes, aad :: Bytes, sealed :: Bytes) -> Result<Plaintext<Bytes>, CryptoError> do
  Crypto.aead_open_plaintext(key, nonce, aad, sealed)
end
fn seal_to(recipient :: X25519PublicKey, info :: Bytes, aad :: Bytes, body :: Plaintext<Bytes>) -> Result<Bytes, CryptoError> do
  Crypto.hpke_seal_plaintext(recipient, info, aad, body)
end
fn open_from(private_key :: borrow X25519PrivateKey, info :: Bytes, aad :: Bytes, sealed :: Bytes) -> Result<Plaintext<Bytes>, CryptoError> do
  Crypto.hpke_open_plaintext(private_key, info, aad, sealed)
end
fn store(body :: Plaintext<Bytes>, key :: borrow StorageKey, context :: Bytes) -> Result<Bytes, CryptoError> do
  Plaintext.seal_for_storage(body, key, context)
end
fn load(sealed :: Bytes, key :: borrow StorageKey, context :: Bytes) -> Result<Plaintext<Bytes>, CryptoError> do
  Plaintext.unseal_from_storage(sealed, key, context)
end
"#,
    );
    // What an open gives back is plaintext too.
    assert_refused(
        "opened",
        "fn open(key :: borrow AeadKey, sealed :: Bytes) do\n  case Crypto.aead_open_plaintext(key, sealed, sealed, sealed) do\n    Ok(body) -> Host.log_redacted(body)\n    Err(_) -> Err(\"no\")\n  end\nend\n",
    );
}

#[test]
fn only_display_exports_carry_plaintext_to_the_host() {
    let result = assert_clean(
        "@display\n@export(\"mesh_show\")\npub fn show(request :: Bytes) -> Plaintext<Bytes>!String do\n  Ok(Plaintext.from(request))\nend\n@display\n@export(\"mesh_compose\")\npub fn compose(body :: Plaintext<Bytes>) -> Bytes!String do\n  Ok(Bytes.empty())\nend\n",
    );
    let exports: Vec<(&str, &str)> = result
        .plaintext
        .display_exports
        .iter()
        .map(|export| (export.function.as_str(), export.symbol.as_str()))
        .collect();
    assert_eq!(
        exports,
        [("show", "mesh_show"), ("compose", "mesh_compose")]
    );

    for (label, source, reason) in [
        (
            "an export without @display",
            "@export(\"mesh_show\")\npub fn show(request :: Bytes) -> Plaintext<Bytes>!String do\n  Ok(Plaintext.from(request))\nend\n",
            "`show` carries plaintext across the library boundary: mark it `@display`",
        ),
        (
            "@display on a function that is not exported",
            "@display\npub fn show(request :: Bytes) -> Plaintext<Bytes> do\n  Plaintext.from(request)\nend\n",
            "`@display` marks an `@export`",
        ),
        (
            "@display on an export without plaintext",
            "@display\n@export(\"mesh_echo\")\npub fn echo(request :: Bytes) -> Bytes!String do\n  Ok(request)\nend\n",
            "`echo` carries no plaintext",
        ),
    ] {
        let result = check_source(source);
        assert!(
            violations(&result).iter().any(|found| found.contains(reason)),
            "{label}: expected `{reason}`, got {:?}",
            result.errors
        );
    }
}
