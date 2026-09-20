use mesh_typeck::TypeckResult;

fn check(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "parse errors: {:?}", parse.errors());
    mesh_typeck::check(&parse)
}

#[test]
fn postgres_binary_values_are_typed_without_changing_text_calls() {
    let result = check(
        r#"
fn keep(value :: DbValue) -> DbValue do value end

fn direct(conn :: PgConn, payload :: Bytes) do
  let _ = Pg.execute_values(conn, "INSERT INTO messages (body) VALUES ($1)", [Binary(payload)])
  let _ = Pg.query_values(conn, "SELECT body FROM messages WHERE label = $1", [Text("inbox")])
  let _ = Pg.execute(conn, "INSERT INTO labels (name) VALUES ($1)", ["legacy"])
  Pg.query(conn, "SELECT name FROM labels", [])
end

fn pooled(pool :: PoolHandle, payload :: Bytes) do
  let _ = Pool.execute_values(pool, "INSERT INTO messages (body) VALUES ($1)", [Binary(payload)])
  Pool.query_values(pool, "SELECT body FROM messages WHERE deleted_at IS $1", [Null])
end
"#,
    );

    assert!(
        result.errors.is_empty(),
        "typed PostgreSQL values should type-check: {:?}",
        result.errors
    );
}
