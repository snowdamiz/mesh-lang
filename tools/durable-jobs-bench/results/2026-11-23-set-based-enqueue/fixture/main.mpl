fn url() -> String do
  Env.get("BULK_TEST_URL", "")
end

fn namespace() -> String do
  Env.get("BULK_TEST_NS", "")
end

fn typed_write(conn :: borrow PgConn, token :: String) -> Unit!String do
  let bytes = case Bytes.from_hex("00ff80") do
    Ok(bytes) -> bytes
    Err(_) -> Bytes.empty()
  end
  let _ = Pg.execute_values(conn,
    "INSERT INTO public.bulk_app(token, body, optional) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    [Text(token), Binary(bytes), Null])?
  Ok(())
end

fn in_transaction(conn :: borrow PgConn, spec :: String, mode :: String) -> String!String do
  let _ = Pg.begin(conn)?
  if mode == "repeatable" do
    let _ = Pg.execute(conn, "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ", [])?
    ()
  end
  let token = Env.get("BULK_TEST_TOKEN", "")
  let _ = Pg.query(conn, "SELECT set_config('application_name', $1, true)", [token])?
  typed_write(conn, token)?
  case Jobs.enqueue_in(conn, namespace(), spec) do
    Ok(receipt) ->
      if mode == "expect-error" do
        let _ = Pg.rollback(conn)?
        return Err("expected enqueue error")
      end
      if mode == "rollback" do
        let _ = Pg.rollback(conn)?
        return Ok(receipt)
      end
      if mode == "hold" do
        println("HELD")
        let _ = IO.read_line()?
        ()
      end
      let _ = Pg.commit(conn)?
      Ok(receipt)
    Err(error) ->
      if mode == "expect-error" do
        typed_write(conn, token <> "-after")?
        let _ = Pg.commit(conn)?
        return Ok(Json.encode(json {error: error}))
      end
      Err(error)
  end
end

fn handle(context :: String) -> String do
  case Json.parse(context) do
    Ok(ctx) ->
      case Json.object_get(ctx, "effect_key") do
        Ok(key) ->
          case Pg.connect(url()) do
            Ok(conn) ->
              let result = typed_write(conn, Json.encode(key))
              Pg.close(conn)
              case result do
                Ok(_) -> """{"action":"complete"}"""
                Err(error) -> Json.encode(json {action: "discard", reason: error})
              end
            Err(error) -> Json.encode(json {action: "discard", reason: error})
          end
        Err(error) -> Json.encode(json {action: "discard", reason: error})
      end
    Err(error) -> Json.encode(json {action: "discard", reason: error})
  end
end

fn pooled(mode :: String) -> String!String do
  let pool = Pool.open(url(), 1, 2, 5000)?
  if mode == "migrate" do
    let _ = Jobs.migrate(pool)?
    let _ = Pool.close(pool)
    return Ok("MIGRATED")
  end
  if mode == "drain" do
    let queues = Json.parse("""{"default":1}""")?
    let workers = Json.parse("""[{"worker":"Proof","version":1}]""")?
    let config = Json.encode(json {
      namespace: namespace(), queues: queues, workers: workers,
      lease_ms: 6000, poll_ms: 10, shutdown_grace_ms: 5000
    })
    let result = Jobs.drain(pool, config, handle, """{"max_jobs":128,"recursive":false,"on_error":"continue"}""")
    let _ = Pool.close(pool)
    return result
  end
  let spec = File.read(Env.get("BULK_TEST_SPEC", ""))?
  let result = Jobs.enqueue(pool, namespace(), spec)
  let _ = Pool.close(pool)
  result
end

fn run() -> String!String do
  let mode = List.get(Env.args(), 1)
  if mode == "migrate" || mode == "pool" || mode == "drain" do
    return pooled(mode)
  end
  let spec = File.read(Env.get("BULK_TEST_SPEC", ""))?
  let conn = Pg.connect(url())?
  let result = in_transaction(conn, spec, mode)
  Pg.close(conn)
  result
end

fn main() do
  case run() do
    Ok(value) -> println(value)
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
