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

fn repo_commit_body(conn :: borrow PgConn) -> Int ! String do
  let _ = Pg.execute(conn,
  "INSERT INTO mesh_repo_transaction_runtime (value) VALUES ('committed')", []) ?
  Ok(1)
end

fn repo_rollback_body(conn :: borrow PgConn) -> Int ! String do
  let _ = Pg.execute(conn,
  "INSERT INTO mesh_repo_transaction_runtime (value) VALUES ('rolled-back')", []) ?
  Err("forced rollback")
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
  let _ = Pg.transaction(conn, transaction_body) ?
  println("direct-transaction:ok")
  Pg.close(conn)
  let pool = Pool.open(url, 1, 2, 5000) ?
  let _ = Pool.execute(pool,
  "CREATE TABLE IF NOT EXISTS mesh_repo_transaction_runtime (value TEXT NOT NULL)", []) ?
  let _ = Pool.execute(pool, "DELETE FROM mesh_repo_transaction_runtime", []) ?
  let committed = Repo.transaction(pool, repo_commit_body) ?
  println("transaction-commit:#{committed}")
  case Repo.transaction(pool, repo_rollback_body) do
    Err( reason) -> println("transaction-rollback:#{reason}")
    Ok( _) -> println("transaction-rollback:unexpected-commit")
  end
  let transaction_rows = Pool.query(pool,
  "SELECT count(*)::text AS value FROM mesh_repo_transaction_runtime", []) ?
  println("transaction-count:" <> Map.get(List.head(transaction_rows), "value"))
  Pool.close(pool)
  Ok(0)
end

fn main() do
  case run_db() do
    Ok(_) -> println("done")
    Err(error) -> println("error:" <> error)
  end
end
