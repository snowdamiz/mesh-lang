fn payload(rows :: List<Map<String, DbValue>>) -> Bytes!String do
  if List.length(rows) != 1 do
    return Err("wrong_row_count")
  end
  case Map.get(List.head(rows), "payload") do
    Binary(value) -> Ok(value)
    _ -> Err("wrong_value_type")
  end
end

fn exercise() -> Bool!String do
  let db = Sqlite.open(":memory:")?
  let _ = Sqlite.execute(db, "CREATE TABLE archive(payload BLOB NOT NULL) STRICT", [])?
  let input = payload(Sqlite.query_values_v2(db,
    "SELECT zeroblob(16797311) AS payload",
    [],
    16797311)?)?
  if Bytes.length(input) != 16797311 do
    return Err("wrong_payload_extent")
  end
  let first = Sqlite.execute_values_v2(db,
    "INSERT INTO archive VALUES (?)",
    [Binary(input)],
    16797311)?
  let second = sqlite_execute_values_v2(db,
    "INSERT INTO archive VALUES (?)",
    [Binary(input)],
    16797311)?
  let actual = payload(sqlite_query_values_v2(db,
    "SELECT payload FROM archive LIMIT 1",
    [],
    16797311)?)?
  let bounded = Result.is_err(Sqlite.query_values_v2(db,
    "SELECT payload FROM archive LIMIT 1",
    [],
    16797310))
  let invalid = Result.is_err(sqlite_execute_values_v2(db,
    "INSERT INTO archive VALUES (X'01')",
    [],
    0))
  let legacy = Result.is_err(Sqlite.query_values(db, "SELECT payload FROM archive LIMIT 1", []))
  let rows = Sqlite.query(db, "SELECT count(*) AS count FROM archive", [])?
  let unchanged = Map.get(List.head(rows), "count") == "2"
  Sqlite.close(db)
  Ok(first == 1
    && second == 1
    && Bytes.secure_equals(input, actual)
    && bounded
    && invalid
    && legacy
    && unchanged)
end

fn main() do
  case exercise() do
    Ok(true) -> println("sqlite-v2:ok")
    _ -> println("sqlite-v2:failed")
  end
end
