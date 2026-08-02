fn binary_hex(value :: DbValue) -> String do
  case value do
    Binary( bytes) -> Bytes.to_hex(bytes)
    Text( _) -> "unexpected-text"
    Null -> "unexpected-null"
  end
end

fn text_value(value :: DbValue) -> String do
  case value do
    Text( text) -> text
    Binary( _) -> "unexpected-binary"
    Null -> "unexpected-null"
  end
end

fn null_value(value :: DbValue) -> String do
  case value do
    Null -> "null"
    Text( _) -> "unexpected-text"
    Binary( _) -> "unexpected-binary"
  end
end

fn run_db() -> Int ! String do
  let url = Env.get("MESH_TEST_DATABASE_URL",
  "postgres://mesh_test:mesh_test@localhost:5432/mesh_test?sslmode=disable")
  let pool = Pool.open(url, 1, 1, 5000) ?
  let _ = Pool.execute(pool, "DROP TABLE IF EXISTS mesh_db_value_e2e", []) ?
  let _ = Pool.execute(pool,
  "CREATE TABLE mesh_db_value_e2e (id SERIAL PRIMARY KEY, payload BYTEA, label TEXT NOT NULL, optional BYTEA)",
  []) ?
  let _ = Pool.execute(pool,
  "INSERT INTO mesh_db_value_e2e (payload, label) VALUES (NULL, $1)",
  ["legacy"]) ?
  let payload = case Bytes.from_hex("00ff80") do
    Ok( bytes) -> bytes
    Err( _) -> Bytes.empty()
  end
  let _ = Pool.execute_values(pool,
  "INSERT INTO mesh_db_value_e2e (payload, label, optional) VALUES ($1, $2, $3)",
  [Binary(payload), Text("typed"), Null]) ?
  let rows = Pool.query_values(pool,
  "SELECT payload, label, optional FROM mesh_db_value_e2e WHERE payload = $1",
  [Binary(payload)]) ?
  let row = List.head(rows)
  println("binary:" <> binary_hex(Map.get(row, "payload")))
  println("text:" <> text_value(Map.get(row, "label")))
  println("null:" <> null_value(Map.get(row, "optional")))
  let legacy_rows = Pool.query(pool,
  "SELECT label FROM mesh_db_value_e2e WHERE label = $1",
  ["legacy"]) ?
  println("legacy:" <> Map.get(List.head(legacy_rows), "label"))
  Pool.close(pool)
  Ok(0)
end

fn main() do
  case run_db() do
    Ok( _) -> println("done")
    Err( error) -> println("error:" <> error)
  end
end
