fn binary_hex(value :: DbValue) -> String do
  case value do
    Binary(bytes) -> Bytes.to_hex(bytes)
    Text(_) -> "unexpected-text"
    Null -> "unexpected-null"
  end
end

fn text_value(value :: DbValue) -> String do
  case value do
    Text(text) -> text
    Binary(_) -> "unexpected-binary"
    Null -> "unexpected-null"
  end
end

fn null_value(value :: DbValue) -> String do
  case value do
    Null -> "null"
    Text(_) -> "unexpected-text"
    Binary(_) -> "unexpected-binary"
  end
end

fn run_db() -> Int ! String do
  let db = Sqlite.open(":memory:")?
  let _ = Sqlite.execute(db,
  "CREATE TABLE values_test (label TEXT NOT NULL, payload BLOB NOT NULL, optional BLOB, empty_payload BLOB NOT NULL) STRICT",
  [])?
  let payload = Bytes.from_hex("00ff80")?
  let _ = Sqlite.execute_values(db,
  "INSERT INTO values_test (label, payload, optional, empty_payload) VALUES (?, ?, ?, ?)",
  [Text("typed"), Binary(payload), Null, Binary(Bytes.empty())])?
  let rows = Sqlite.query_values(db,
  "SELECT label, payload, optional, empty_payload FROM values_test WHERE label = ?",
  [Text("typed")])?
  let row = List.head(rows)
  println("binary:" <> binary_hex(Map.get(row, "payload")))
  println("empty:" <> binary_hex(Map.get(row, "empty_payload")))
  println("text:" <> text_value(Map.get(row, "label")))
  println("null:" <> null_value(Map.get(row, "optional")))
  let legacy = Sqlite.query(db,
  "SELECT typeof(payload) AS payload_type, typeof(empty_payload) AS empty_type FROM values_test",
  [])?
  let legacy_row = List.head(legacy)
  println("legacy:" <> Map.get(legacy_row, "payload_type") <> ":" <> Map.get(legacy_row, "empty_type"))
  Sqlite.close(db)
  Ok(0)
end

fn main() do
  case run_db() do
    Ok(_) -> println("done")
    Err(error) -> println("error:" <> error)
  end
end
