//! Bounded joining must not disclose content, extents, limits or caller control.
use mesh_typeck::error::TypeError;

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn public_join_and_private_computation_without_a_sink_are_allowed() {
    for operation in ["Bytes.concat_many", "bytes_concat_many"] {
        let result = checked(&format!(
            "fn keep(parts :: List<Plaintext<Bytes>>, limit :: Int) -> Bytes!BytesError do {operation}(parts, limit) end\nfn public() do let joined = Result.unwrap_or({operation}([Bytes.from_utf8(\"public\")],6), Bytes.empty())\nHttp.body_bytes(Http.build(:post,\"https://example.test\"), joined) end"
        ));
        assert!(result.errors.is_empty(), "{operation}: {:?}", result.errors);
    }
}

#[test]
fn private_inputs_and_callers_cannot_escape_through_join_projections() {
    for operation in ["Bytes.concat_many", "bytes_concat_many"] {
        let sources = [
            format!("fn leak(parts :: List<Plaintext<Bytes>>) do let joined = Result.unwrap_or({operation}(parts,100),Bytes.empty())\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),joined) end"),
            format!("fn leak(parts :: Plaintext<List<Bytes>>) do println(String.from(Result.is_ok({operation}(parts,100)))) end"),
            format!("fn leak(value :: Plaintext<Bytes>) do let joined = Result.unwrap_or({operation}([value],100),Bytes.empty())\nprintln(Int.to_string(Bytes.length(joined))) end"),
            format!("fn leak(limit :: Plaintext<Int>) do println(String.from(Result.is_err({operation}([Bytes.from_utf8(\"public\")],limit)))) end"),
            format!("fn leak(count :: Plaintext<Int>) do let parts = for index in 0..count do Bytes.empty() end\nprintln(String.from(Result.is_ok({operation}(parts,0)))) end"),
            format!("fn leak(choice :: Plaintext<Bool>) do let result = if choice do {operation}([Bytes.from_utf8(\"a\")],1) else {operation}([Bytes.from_utf8(\"b\")],1) end\nprintln(String.from(Result.is_ok(result))) end"),
            format!("fn join(parts :: List<Bytes>, limit :: Int) -> Bytes!BytesError do {operation}(parts,limit) end\nfn leak(value :: Plaintext<Bytes>) do println(String.from(Result.is_ok(join([value],100)))) end"),
        ];
        for (index, source) in sources.iter().enumerate() {
            let result = checked(source);
            assert!(
                !result.errors.is_empty(),
                "{operation} case {index} escaped"
            );
            assert!(
                result
                    .errors
                    .iter()
                    .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
                "{operation} case {index}: {:?}",
                result.errors
            );
        }
    }
}
