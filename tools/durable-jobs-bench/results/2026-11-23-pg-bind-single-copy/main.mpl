fn repeat(byte :: Int, count :: Int) -> Bytes!String do
  case Bytes.repeat(byte, count) do
    Ok(value) -> Ok(value)
    Err(_) -> Err("payload allocation failed")
  end
end

fn step(conn :: borrow PgConn, index :: Int, scalar :: String, label :: String, bytes :: Bytes) -> Unit!String do
  let id = String.from(index)
  let _ = Pg.begin(conn)?
  let scalar_count = Pg.execute(conn, "INSERT INTO bind_scalar VALUES ($1, $2)", [id, scalar])?
  let mixed_count = Pg.execute_values(conn, "INSERT INTO bind_mixed VALUES ($1, $2, $3, $4)",
    [Text(id), Binary(bytes), Text(label), Null])?
  let rows = Pg.query_values(conn,
    "SELECT needle, label, optional, octet_length(payload)::text AS byte_count FROM (SELECT substring($2::bytea from 1 for 3) AS needle, $3::text AS label, $4::text AS optional, payload FROM bind_mixed WHERE id = $1 AND payload = $2 AND bind_mixed.label = $3) AS found",
    [Text(id), Binary(bytes), Text(label), Null])?
  if scalar_count != 1 || mixed_count != 1 || List.length(rows) != 1 do
    return Err("unexpected write or query row count")
  end
  let row = List.head(rows)
  let needle = case Map.get(row, "needle") do
    Binary(value) -> Bytes.to_hex(value)
    _ -> "incorrect"
  end
  let label_matches = case Map.get(row, "label") do
    Text(value) -> value == label
    _ -> false
  end
  let optional_matches = case Map.get(row, "optional") do
    Null -> true
    _ -> false
  end
  let count_matches = case Map.get(row, "byte_count") do
    Text(value) -> value == String.from(Bytes.length(bytes))
    _ -> false
  end
  if needle != "a5a5a5" || !label_matches || !optional_matches || !count_matches do
    return Err("unexpected typed result")
  end
  Pg.commit(conn)
end

fn samples(conn :: borrow PgConn, i :: Int, count :: Int, scalar :: String, label :: String, bytes :: Bytes) -> Unit!String do
  if i > count do
    return Ok(())
  end
  let _ = step(conn, i, scalar, label, bytes)?
  samples(conn, i + 1, count, scalar, label, bytes)
end

fn exercise() -> Unit!String do
  let url = Env.get("BIND_DATABASE_URL", "")
  let scale = Env.get_int("BIND_SCALE", 0)
  let count = Env.get_int("BIND_COUNT", 0)
  let scalar_data = repeat(120, 21 * scale)?
  let scalar = Bytes.to_utf8(scalar_data)?
  let label_data = repeat(122, 7 * scale)?
  let label = Bytes.to_utf8(label_data)?
  let bytes = repeat(165, 17 * scale)?
  let conn = Pg.connect(url)?
  let rows = Pg.query(conn,
    "SELECT current_setting('server_version_num') AS version, ssl::text AS ssl, current_setting('fsync') AS fsync, current_setting('synchronous_commit') AS synchronous FROM pg_stat_ssl WHERE pid=pg_backend_pid()", [])?
  let row = List.head(rows)
  if Map.get(row, "version") != "170011" || Map.get(row, "ssl") != "true" || Map.get(row, "fsync") != "on" || Map.get(row, "synchronous") != "on" do
    return Err("unexpected PostgreSQL server identity")
  end
  let warmup = repeat(165, 17)?
  let _ = step(conn, 0, "warm", "warm", warmup)?
  println("ready")
  let _ = IO.read_line()?
  let _ = samples(conn, 1, count, scalar, label, bytes)?
  println("committed:" <> String.from(count))
  Pg.close(conn)
  Ok(())
end

fn main() do
  case exercise() do
    Ok(_) -> ()
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
