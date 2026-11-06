use mesh_typeck::error::TypeError;

fn checked(source: &str) -> mesh_typeck::TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

#[test]
fn sqlite_explicit_value_quota_has_public_typed_results_in_both_spellings() {
    for (write, read) in [
        ("Sqlite.execute_values_v2", "Sqlite.query_values_v2"),
        ("sqlite_execute_values_v2", "sqlite_query_values_v2"),
    ] {
        let source = format!("fn write(db::SqliteConn, data::Bytes)->Result<Int,String> do {write}(db,\"INSERT INTO archive VALUES (?)\",[Binary(data)],16797311) end\nfn read(db::SqliteConn)->Result<List<Map<String,DbValue>>,String> do {read}(db,\"SELECT payload FROM archive\",[],16797311) end");
        let result = checked(&source);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }
}

#[test]
fn sqlite_explicit_quota_does_not_disclose_private_arguments_or_control() {
    for (write, read) in [
        ("Sqlite.execute_values_v2", "Sqlite.query_values_v2"),
        ("sqlite_execute_values_v2", "sqlite_query_values_v2"),
    ] {
        let cases = [
            format!("fn probe(db::SqliteConn, value::Plaintext<Int>)->Result<Int,String> do {write}(db,\"SELECT 1\",[],value) end"),
            format!("fn probe(db::SqliteConn, value::Plaintext<Bytes>)->Result<Int,String> do {write}(db,\"SELECT ?\",[Binary(value)],32) end"),
            format!("fn probe(db::SqliteConn, value::Plaintext<String>)->Result<Int,String> do {write}(db,value,[],32) end"),
            format!("fn probe(db::SqliteConn, value::Plaintext<Int>)->Result<List<Map<String,DbValue>>,String> do {read}(db,\"SELECT 1\",[],value) end"),
            format!("fn probe(db::SqliteConn, value::Plaintext<Bool>)->Result<List<Map<String,DbValue>>,String> do if value do {read}(db,\"SELECT 1\",[],32) else Ok([]) end end"),
        ];
        for source in cases {
            let result = checked(&source);
            assert!(
                !result.errors.is_empty(),
                "private SQLite effect admitted: {source}"
            );
            assert!(
                result
                    .errors
                    .iter()
                    .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
                "{source}: {:?}",
                result.errors
            );
        }
    }
}
