//! Explicit file quotas must remain public effect arguments, in both spellings.
use mesh_typeck::error::TypeError;

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check(&parse)
}

#[test]
fn explicit_file_quota_calls_have_public_typed_results() {
    for (read, write) in [
        ("File.read_bytes_v2", "File.write_bytes_v2"),
        ("file_read_bytes_v2", "file_write_bytes_v2"),
    ] {
        let source = format!("fn read(path :: String) -> Result<Bytes,String> do {read}(path,16777216,19648,16796864) end\nfn write(path :: String, data :: Bytes) -> Result<(),String> do {write}(path,16777216,data,false,16796864) end");
        let result = checked(&source);
        assert!(result.errors.is_empty(), "{source}: {:?}", result.errors);
    }
}

#[test]
fn explicit_file_quota_arguments_and_caller_control_remain_private() {
    let reads = [
        ("String", "value,0,1,16"),
        ("Int", "\"fixture\",value,1,16"),
        ("Int", "\"fixture\",0,value,16"),
        ("Int", "\"fixture\",0,1,value"),
    ];
    let writes = [
        ("String", "value,0,Bytes.empty(),false,16"),
        ("Int", "\"fixture\",value,Bytes.empty(),false,16"),
        ("Bytes", "\"fixture\",0,value,false,16"),
        ("Bool", "\"fixture\",0,Bytes.empty(),value,16"),
        ("Int", "\"fixture\",0,Bytes.empty(),false,value"),
    ];
    for (read, write) in [
        ("File.read_bytes_v2", "File.write_bytes_v2"),
        ("file_read_bytes_v2", "file_write_bytes_v2"),
    ] {
        let mut sources = Vec::new();
        for (ty, args) in reads {
            sources.push(format!(
                "fn probe(value :: Plaintext<{ty}>) -> Result<Bytes,String> do {read}({args}) end"
            ));
        }
        for (ty, args) in writes {
            sources.push(format!(
                "fn probe(value :: Plaintext<{ty}>) -> Result<(),String> do {write}({args}) end"
            ));
        }
        sources.push(format!("fn probe(value :: Plaintext<Bool>) -> Result<Bytes,String> do if value do {read}(\"fixture\",0,1,16) else Ok(Bytes.empty()) end end"));
        sources.push(format!("fn probe(value :: Plaintext<Bool>) -> Result<(),String> do if value do {write}(\"fixture\",0,Bytes.empty(),false,16) else Ok(nil) end end"));
        for source in sources {
            let result = checked(&source);
            assert!(
                !result.errors.is_empty(),
                "private effect was admitted: {source}"
            );
            assert!(
                result
                    .errors
                    .iter()
                    .all(|e| matches!(e, TypeError::PlaintextViolation { .. })),
                "unrelated diagnostics for {source}: {:?}",
                result.errors
            );
        }
    }
}
