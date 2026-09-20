use mesh_typeck::error::TypeError;
use mesh_typeck::TypeckResult;

fn check(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "parse errors: {:?}", parse.errors());
    mesh_typeck::check(&parse)
}

fn resource_violations(result: &TypeckResult) -> Vec<&str> {
    result
        .errors
        .iter()
        .filter_map(|error| match error {
            TypeError::ResourceViolation { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn pg_conn_is_affine_and_cannot_cross_actor_boundaries() {
    let result = check(
        r#"
fn close_twice(conn :: PgConn) do
  Pg.close(conn)
  Pg.close(conn)
end

fn send_alias(pid :: Pid<PgConn>, conn :: PgConn) do
  send(pid, conn)
end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        [
            "resource `conn` was used after it moved",
            "resource `conn` cannot cross an actor mailbox boundary",
        ]
    );
}

#[test]
fn pool_manual_leases_are_not_public_mesh_api() {
    let checkout = check("fn manual(pool :: PoolHandle) do\n  Pool.checkout(pool)\nend");
    let checkin =
        check("fn manual(pool :: PoolHandle, conn :: PgConn) do\n  Pool.checkin(pool, conn)\nend");

    let pool_is_unbound = |result: &TypeckResult| {
        result
            .errors
            .iter()
            .any(|error| matches!(error, TypeError::UnboundVariable { name, .. } if name == "Pool"))
    };
    assert!(pool_is_unbound(&checkout), "errors: {:?}", checkout.errors);
    assert!(pool_is_unbound(&checkin), "errors: {:?}", checkin.errors);
}

#[test]
fn pg_operations_borrow_until_close_consumes() {
    let result = check(
        r#"
fn decode(_row :: Map<String, String>) -> Int ! String do
  Ok(1)
end

fn in_transaction(conn :: borrow PgConn) do
  let _ = Pg.execute(conn, "SELECT 1", [])
  Ok(nil)
end

fn exercise(conn :: PgConn, payload :: Bytes) do
  let _ = Pg.execute(conn, "SELECT 1", [])
  let _ = Pg.query(conn, "SELECT 1", [])
  let _ = Pg.execute_values(conn, "SELECT $1", [Binary(payload)])
  let _ = Pg.query_values(conn, "SELECT $1", [Binary(payload)])
  let _ = Pg.begin(conn)
  let _ = Pg.commit(conn)
  let _ = Pg.rollback(conn)
  let _ = Pg.transaction(conn, in_transaction)
  let _ = Pg.query_as(conn, "SELECT 1", [], decode)
  Pg.close(conn)
end
"#,
    );

    assert!(
        result.errors.is_empty(),
        "Pg operations must borrow the unique connection until close: {:?}",
        result.errors
    );
}

#[test]
fn transaction_callback_must_borrow_connection_alias() {
    let result = check(
        r#"
fn unsafe_transaction(conn :: PgConn) do
  let _ = Pg.transaction(conn, fn(inner_conn :: PgConn) do
    Pg.close(inner_conn)
    Ok(nil)
  end)
  Pg.close(conn)
end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        ["Pg.transaction callback must borrow its PgConn parameter"]
    );
}

#[test]
fn named_transaction_callback_must_borrow_connection_alias() {
    let result = check(
        r#"
fn unsafe_body(inner_conn :: PgConn) do
  Pg.close(inner_conn)
  Ok(nil)
end

fn unsafe_transaction(conn :: PgConn) do
  let _ = Pg.transaction(conn, unsafe_body)
  Pg.close(conn)
end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        ["Pg.transaction callback must borrow its PgConn parameter"]
    );
}

#[test]
fn repo_transaction_exposes_only_a_scoped_borrow() {
    let result = check(
        r#"
fn repo_body(conn :: borrow PgConn) do
  let _ = Pg.execute(conn, "SELECT 1", [])
  Ok(1)
end

fn safe_transaction(pool :: PoolHandle) do
  Repo.transaction(pool, repo_body)
end
"#,
    );

    assert!(
        result.errors.is_empty(),
        "Repo.transaction must expose a borrowed PgConn and preserve the callback result: {:?}",
        result.errors
    );
}

#[test]
fn repo_transaction_rejects_an_owned_connection_alias() {
    let result = check(
        r#"
fn unsafe_transaction(pool :: PoolHandle) do
  Repo.transaction(pool, fn(inner_conn :: PgConn) do
    Pg.close(inner_conn)
    Ok(1)
  end)
end
"#,
    );

    assert_eq!(
        resource_violations(&result),
        ["Repo.transaction callback must borrow its PgConn parameter"]
    );
}
