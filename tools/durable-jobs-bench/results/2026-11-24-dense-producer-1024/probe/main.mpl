fn enqueue(conn :: borrow PgConn, ns :: String, specs :: String, tail_path :: String) -> String!String do
  let first = Jobs.enqueue_in(conn, ns, specs)?
  if tail_path == "" do
    return Ok(first)
  end
  let tail = Jobs.enqueue_in(conn, ns, File.read(tail_path)?)?
  Ok(Json.encode(json { first: Json.parse(first)?, tail: Json.parse(tail)? }))
end

fn submit() -> String!String do
  let conn = Pg.connect(Env.get("JOBS_PARITY_DB_URL", ""))?
  let specs = File.read(Env.get("DENSE_PROBE_INPUT", ""))?
  let ns = Env.get("JOBS_PARITY_NAMESPACE", "")
  let _ = Pg.begin(conn)?
  let tail_path = Env.get("DENSE_PROBE_TAIL", "")
  let result = case enqueue(conn, ns, specs, tail_path) do
    Ok(receipt) ->
      let _ = if Env.get("DENSE_PROBE_ROLLBACK", "") == "1" do Pg.rollback(conn) else Pg.commit(conn) end?
      Ok(receipt)
    Err(error) ->
      let _ = Pg.execute(conn, "SELECT 1", [])?
      let _ = if tail_path == "" do Pg.commit(conn) else Pg.rollback(conn) end?
      Ok(Json.encode(json { error: error, transaction_usable: true }))
  end
  Pg.close(conn)
  result
end

fn main() do
  case submit() do
    Ok(receipt) -> println(receipt)
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
