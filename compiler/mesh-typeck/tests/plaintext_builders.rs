//! Local binary framing remains labeled through an affine builder; an escaped
//! or unproven builder never acquires the right to receive message plaintext.
use mesh_typeck::{error::TypeError, ImportContext, ModuleExports};

fn checked(source: &str, context: &ImportContext) -> mesh_typeck::TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check_with_imports(&parse, context)
}
fn allowed(source: &str) {
    let result = checked(source, &ImportContext::empty());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}
fn refused(source: &str, context: &ImportContext) {
    let result = checked(source, context);
    assert!(
        result
            .errors
            .iter()
            .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
        "unrelated type error: {:?}",
        result.errors
    );
    assert!(!result.errors.is_empty(), "plaintext escaped: {source}");
    if source.contains("Http.body_bytes") {
        assert!(result.errors.iter().any(|error| matches!(error,TypeError::PlaintextViolation {sink,..} if sink == "Http.body_bytes")),"builder contents did not reach the actual HTTP sink: {:?}",result.errors);
    }
}
const FRAME: &str = r#"
fn frame(value :: Plaintext<Bytes>) -> Bytes!BinaryError do
  let builder = BytesBuilder.new(128)?
  let _ = BytesBuilder.write_bytes(builder,value)
  BytesBuilder.finish(builder)
end
"#;

#[test]
fn a_local_frame_can_be_sealed_but_cannot_be_used_as_an_http_body() {
    allowed(&format!("{FRAME}\nfn framed(value :: Plaintext<Bytes>) -> Bytes do case frame(value) do Err(_) -> Bytes.empty()\nOk(body) -> body end end\nfn work(value :: Plaintext<Bytes>, key :: borrow AeadKey, nonce :: Bytes) do\nCrypto.aead_seal(key,nonce,Bytes.empty(),framed(value)) end"));
    refused(&format!("{FRAME}\nfn work(value :: Plaintext<Bytes>) -> Int!BinaryError do\nlet body = frame(value)?\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),body)\nOk(1) end"),&ImportContext::empty());
}

#[test]
fn moves_and_borrowed_writer_helpers_preserve_contents_until_finish() {
    let helper = "fn append(builder :: borrow BytesBuilder, value :: Bytes) -> Result<(),BinaryError> do BytesBuilder.write_bytes(builder,value) end";
    allowed(&format!("{helper}\nfn frame(value :: Plaintext<Bytes>) -> Bytes!BinaryError do\nlet builder = BytesBuilder.new(128)?\nlet moved = builder\nlet _ = append(moved,value)\nBytesBuilder.finish(moved) end"));
    refused(&format!("{helper}\nfn leak(value :: Plaintext<Bytes>) -> Int!BinaryError do\nlet builder = BytesBuilder.new(128)?\nlet moved = builder\nlet _ = append(moved,value)\nlet body = BytesBuilder.finish(moved)?\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),body)\nOk(1) end"),&ImportContext::empty());
}

#[test]
fn writes_in_either_branch_taint_the_finished_frame() {
    refused("fn leak(flag :: Bool, value :: Plaintext<Bytes>) -> Int!BinaryError do\nlet builder = BytesBuilder.new(128)?\nif flag do let _ = BytesBuilder.write_bytes(builder,value)\nnil else nil end\nlet body = BytesBuilder.finish(builder)?\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),body)\nOk(1) end",&ImportContext::empty());
}

#[test]
fn an_empty_builder_observed_by_an_unproven_import_cannot_receive_plaintext_later() {
    let dependency =
        mesh_parser::parse("pub fn observe(builder :: borrow BytesBuilder) do nil end");
    let check = mesh_typeck::check(&dependency);
    assert!(check.errors.is_empty(), "{:?}", check.errors);
    let mut exports = mesh_typeck::collect_exports(&dependency, &check);
    // Model an embedding/native boundary without a checked flow summary.
    exports.plaintext_functions.clear();
    let mut context = ImportContext::empty();
    context.module_exports.insert(
        "Unproven".into(),
        ModuleExports::new("Unproven".into(), &exports),
    );
    refused("from Unproven import observe\nfn leak(value :: Plaintext<Bytes>) -> Bytes!BinaryError do\nlet builder = BytesBuilder.new(128)?\nobserve(builder)\nBytesBuilder.write_bytes(builder,value)?\nBytesBuilder.finish(builder) end",&context);
    refused("from Unproven import observe\nfn leak(value :: Plaintext<Bytes>) -> Bytes!BinaryError do\nlet factory = fn() -> BytesBuilder.new(128) end\nlet first = factory()?\nobserve(first)\nlet second = factory()?\nBytesBuilder.write_bytes(first,value)?\nlet _ = BytesBuilder.finish(second)?\nBytesBuilder.finish(first) end",&context);
}

#[test]
fn imported_borrowed_writers_export_their_mutation_effects() {
    let dependency = mesh_parser::parse("pub fn append(builder :: borrow BytesBuilder, value :: Bytes) -> Result<(),BinaryError> do BytesBuilder.write_bytes(builder,value) end");
    let check = mesh_typeck::check(&dependency);
    assert!(check.errors.is_empty(), "{:?}", check.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &check);
    let mut context = ImportContext::empty();
    context.module_exports.insert(
        "Writer".into(),
        ModuleExports::new("Writer".into(), &exports),
    );
    refused("from Writer import append\nfn leak(value :: Plaintext<Bytes>) -> Int!BinaryError do\nlet builder = BytesBuilder.new(128)?\nlet _ = append(builder,value)\nlet body = BytesBuilder.finish(builder)?\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),body)\nOk(1) end",&context);
}
