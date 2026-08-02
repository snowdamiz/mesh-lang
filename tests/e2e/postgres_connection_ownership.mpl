fn decode_row(_row :: Map<String, String>) -> Int ! String do
  Ok(1)
end

fn transaction_body(conn :: borrow PgConn) do
  let _ = Pg.execute(conn, "SELECT 1", [])
  Ok(nil)
end

fn repo_transaction_body(conn :: borrow PgConn) do
  let _ = Pg.query(conn, "SELECT 1", [])
  Ok(1)
end

fn exercise_connection(conn :: PgConn, payload :: Bytes) do
  let _ = Pg.execute(conn, "SELECT 1", [])
  let _ = Pg.query(conn, "SELECT 1", [])
  let _ = Pg.execute_values(conn, "SELECT $1", [Binary(payload)])
  let _ = Pg.query_values(conn, "SELECT $1", [Binary(payload)])
  let _ = Pg.begin(conn)
  let _ = Pg.commit(conn)
  let _ = Pg.rollback(conn)
  let _ = Pg.transaction(conn, transaction_body)
  let _ = Pg.query_as(conn, "SELECT 1", [], decode_row)
  Pg.close(conn)
end

fn exercise_repo_transaction(pool :: PoolHandle) do
  let _ = Repo.transaction(pool, repo_transaction_body)
  nil
end

fn discard_connection_result(url :: String) do
  let connection = Pg.connect(url)
  nil
end

fn forward_connection_result(url :: String) -> PgConn ! String do
  let connection = Pg.connect(url)
  connection
end

fn run_db() -> Int ! String do
  let url = Env.get("MESH_TEST_DATABASE_URL",
  "postgres://mesh_test:mesh_test@localhost:5432/mesh_test?sslmode=disable")
  discard_connection_result(url)
  let conn = forward_connection_result(url) ?
  let rows = Pg.query(conn, "SELECT 'ok' AS value", []) ?
  println("direct:" <> Map.get(List.head(rows), "value"))
  Pg.close(conn)
  Ok(0)
end

fn main() do
  case run_db() do
    Ok(_) -> println("done")
    Err(error) -> println("error:" <> error)
  end
end
