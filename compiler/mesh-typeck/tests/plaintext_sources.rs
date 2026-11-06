//! Unmodelled containers must retain nested source labels conservatively.
use mesh_typeck::error::TypeError;

#[test]
fn nested_labels_in_unmodelled_containers_cannot_disappear_at_function_boundaries() {
    let public = mesh_parser::parse("fn ship(values :: Map<String,Bytes>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),Map.get(values,\"body\")) end");
    assert!(public.ok(), "{:?}", public.errors());
    assert!(mesh_typeck::check(&public).errors.is_empty());
    for source in [
        "fn leak(values :: Map<String,Plaintext<Bytes>>) do let body = Map.get(values,\"body\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),body) end",
        "struct Message do body :: Plaintext<Bytes> end\nfn leak(values :: Map<String,Message>) do let value = Map.get(values,\"body\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),value.body) end",
        "fn leak(values :: Queue<Plaintext<Bytes>>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),Queue.peek(values)) end",
        "fn leak(values :: Set<Plaintext<String>>) do println(String.from(Set.contains(values,\"guess\"))) end",
        "type Bag<T> = Map<String,T>\nfn forward(values :: Bag<Plaintext<Bytes>>) -> Bag<Plaintext<Bytes>> do values end\nfn leak(values :: Bag<Plaintext<Bytes>>) do Http.body_bytes(Http.build(:post,\"https://example.test\"),Map.get(forward(values),\"body\")) end",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.iter().all(|error| matches!(error, TypeError::PlaintextViolation { .. })), "unrelated errors: {:?}", result.errors);
        assert!(!result.errors.is_empty(), "nested source label disappeared: {source}");
    }
}
