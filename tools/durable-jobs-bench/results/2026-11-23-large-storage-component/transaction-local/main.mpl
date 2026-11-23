fn configured(conn :: borrow PgConn) -> String!String do
  let rows = Pg.query(conn, "SELECT current_setting('default_toast_compression') AS value", [])?
  Ok(Map.get(List.head(rows), "value"))
end

fn write_job(conn :: borrow PgConn, key :: String, body :: String, lz4 :: Bool, rollback :: Bool) -> String!String do
  let _ = Pg.begin(conn)?
  if lz4 do
    let _ = Pg.execute(conn, "SET LOCAL default_toast_compression = 'lz4'", [])?
    ()
  end
  let policy = configured(conn)?
  let xid_rows = Pg.query(conn, "SELECT pg_current_xact_id()::text AS xid", [])?
  let xid = Map.get(List.head(xid_rows), "xid")
  let spec = Json.encode(json {worker: "archive", args: json {key: key, payload: body}, request_key: key})
  let receipt = Jobs.enqueue_in(conn, "storage", spec)?
  let _ = Pg.execute(conn, "INSERT INTO public.storage_products (key) VALUES ($1)", [key])?
  if rollback do
    let _ = Pg.rollback(conn)?
    ()
  else
    let _ = Pg.commit(conn)?
    ()
  end
  let restored = configured(conn)?
  println(Json.encode(json {key: key, policy: policy, restored: restored, receipt: receipt, rollback: rollback, xid: xid}))
  Ok("")
end

fn run() -> String!String do
  let pool = Pool.open(Env.get("MESH_STORAGE_OWNER_URL", ""), 1, 1, 2000)?
  let _ = Jobs.migrate(pool)?
  Pool.close(pool)
  if Env.get("MESH_STORAGE_MIGRATE_ONLY", "") == "yes" do return Ok("") end
  let file = File.read(Env.get("MESH_STORAGE_DATA", ""))?
  let conn = Pg.connect(Env.get("MESH_STORAGE_PRODUCER_URL", ""))?
  let _ = write_job(conn, "default", file, false, false)?
  let _ = write_job(conn, "local-rollback", file, true, true)?
  let _ = write_job(conn, "local-commit", file, true, false)?
  let _ = write_job(conn, "default-again", file, false, false)?
  Pg.close(conn)
  Ok("")
end

fn main() do
  case run() do
    Ok(_) -> ()
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
