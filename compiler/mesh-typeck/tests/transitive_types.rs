//! Public signatures carry reachable type layouts without re-exporting names.
use mesh_typeck::{
    check_with_imports, collect_exports, error::TypeError, ImportContext, ModuleExports,
};

fn module(name: &str, source: &str, imports: &ImportContext) -> ModuleExports {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    let checked = check_with_imports(&parse, imports);
    assert!(checked.errors.is_empty(), "{name}: {:?}", checked.errors);
    ModuleExports::new(name.to_string(), &collect_exports(&parse, &checked))
}

#[test]
fn public_nested_types_follow_function_results_through_reexports() {
    let source = module(
        "Source",
        r#"
pub struct Leaf do bytes :: Bytes end
pub struct Entry do leaf :: Leaf end
pub type Answer do
Found(Entry)
Missing
end
pub fn answer() -> Answer do Found(Entry { leaf: Leaf { bytes: Bytes.empty() } }) end
pub fn entries() -> Result<List<Entry>, String> do Ok([Entry { leaf: Leaf { bytes: Bytes.empty() } }]) end
"#,
        &ImportContext::empty(),
    );
    let mut imports = ImportContext::empty();
    imports.module_exports.insert("Source".into(), source);
    let relay = module("Relay", "from Source import answer, entries\npub fn relay() do entries() end\npub fn relay_answer() do answer() end", &imports);
    // Supporting layouts must not become names the intermediate module exports.
    assert!(!relay.struct_defs.contains_key("Leaf"));
    assert!(!relay.struct_defs.contains_key("Entry"));
    let mut imports = ImportContext::empty();
    imports.module_exports.insert("Relay".into(), relay);
    let consumer = mesh_parser::parse("from Relay import relay\nfn value() -> Bytes do case relay() do Ok(entries) -> List.head(entries).leaf.bytes Err(_) -> Bytes.empty() end end");
    let checked = check_with_imports(&consumer, &imports);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    assert!(checked.type_registry.sum_type_defs.contains_key("Answer"));
    let forbidden = check_with_imports(
        &mesh_parser::parse(
            "from Relay import Leaf\nfn value() do Leaf { bytes: Bytes.empty() } end",
        ),
        &imports,
    );
    assert!(!forbidden.errors.is_empty());
}

#[test]
fn private_layouts_do_not_escape_through_public_function_results() {
    let source = module(
        "Source",
        "struct Hidden do secret :: Int end\npub fn opaque() -> Hidden do Hidden { secret: 1 } end",
        &ImportContext::empty(),
    );
    let mut imports = ImportContext::empty();
    imports.module_exports.insert("Source".into(), source);
    let relay = module(
        "Relay",
        "from Source import opaque\npub fn relay() do opaque() end",
        &imports,
    );
    let mut imports = ImportContext::empty();
    imports.module_exports.insert("Relay".into(), relay);
    let checked = check_with_imports(
        &mesh_parser::parse("from Relay import relay\nfn value() do relay().secret end"),
        &imports,
    );
    assert!(
        checked
            .errors
            .iter()
            .any(|error| matches!(error, TypeError::NoSuchField { .. })),
        "{:?}",
        checked.errors
    );
}

#[test]
fn transitive_public_resource_layouts_preserve_affine_field_ownership() {
    let source = module("Source", "pub resource Handle\npub struct Envelope do handle :: Handle end\npub fn wrap(handle :: consume Handle) -> Envelope do Envelope { handle: handle } end", &ImportContext::empty());
    let mut imports = ImportContext::empty();
    imports.module_exports.insert("Source".into(), source);
    let relay = module(
        "Relay",
        "from Source import wrap\npub fn relay(handle :: consume Handle) do wrap(handle) end",
        &imports,
    );
    let mut imports = ImportContext::empty();
    imports.module_exports.insert("Relay".into(), relay);
    let checked = check_with_imports(&mesh_parser::parse("from Relay import relay\nfn take(handle :: consume Handle) do nil end\nfn misuse(handle :: Handle) do let envelope = relay(handle)\ntake(envelope.handle)\ntake(envelope.handle) end"), &imports);
    assert_eq!(checked.errors.len(), 1, "{:?}", checked.errors);
    assert!(
        matches!(&checked.errors[0], TypeError::ResourceViolation { reason, .. } if reason.contains("used after it moved")),
        "{:?}",
        checked.errors
    );
}
