fn namespace() -> String do
  Env.get("MESH_JOBS_PROOF_NAMESPACE", "")
end

fn database_url() -> String do
  Env.get("MESH_TEST_DATABASE_URL", "")
end

fn commit_job(conn :: borrow PgConn) -> String!String do
  let ns = namespace()
  let _ = Pg.execute(conn, "INSERT INTO " <> ns <> ".business VALUES ('committed')", [])?
  Jobs.enqueue_in(conn,
    ns,
    """{"worker":"proof","args":{"label":"committed"},"request_key":"commit-once"}""")
end

fn rollback_job(conn :: borrow PgConn) -> String!String do
  let ns = namespace()
  let _ = Pg.execute(conn, "INSERT INTO " <> ns <> ".business VALUES ('rolled-back')", [])?
  let _ = Jobs.enqueue_in(conn,
    ns,
    """{"worker":"proof","args":{"label":"rolled-back"},"request_key":"rollback-once"}""")?
  Err("intentional rollback after enqueue")
end

fn seed(pool :: PoolHandle) -> String!String do
  let _ = Jobs.migrate(pool)?
  let _ = Jobs.migrate(pool)?
  let receipt = Repo.transaction(pool, commit_job)?
  case Repo.transaction(pool, rollback_job) do
    Err(error) ->
      if error == "intentional rollback after enqueue" do
        Ok(receipt)
      else
        Err(error)
      end
    Ok(_) -> Err("Repo.transaction committed the forced rollback")
  end
end

fn record_effect(context :: String) -> String!String do
  let decoded = Json.parse(context)?
  let args = Json.object_get(decoded, "args")?
  let label = Json.as_string(Json.object_get(args, "label")?)?
  let attempt = Json.as_int(Json.object_get(decoded, "attempt")?)?
  let worker = Env.get("MESH_JOBS_PROOF_WORKER", "")
  let conn = Pg.connect(database_url())?
  let result = Pg.execute(conn,
    "INSERT INTO " <> namespace() <> ".effects(worker, label, context) VALUES ($1, $2, $3::jsonb)",
    [worker, label, context])
  Pg.close(conn)
  let _ = result?
  let gate = Env.get("MESH_JOBS_PROOF_GATE", "")
  if gate != "" do
    while !File.exists(gate) do
      Timer.sleep(10)
    end
  end
  if label == "poison" do
    Ok("""{"action":"fail","reason":"proof worker rejected poison"}""")
  else if label == "retry" && attempt == 1 do
    Ok("""{"action":"retry_after","delay_ms":400,"reason":"proof worker requested a retry"}""")
  else
    Ok("""{"action":"complete"}""")
  end
end

fn handle(context :: String) -> String do
  case record_effect(context) do
    Ok(outcome) -> outcome
    Err(error) ->
      println("handler error: " <> error)
      """{"action":"discard","reason":"proof handler failed"}"""
  end
end

fn serve(pool :: PoolHandle, config :: String) -> String!String do
  Process.install_shutdown_signals()
  let runner = Jobs.start(pool, config, handle)?
  let markers = Env.get("MESH_JOBS_PROOF_MARKERS", "")
  let status = Jobs.status(runner)?
  let _ = File.write(markers <> "/ready", status)?
  while !Process.shutdown_requested() do
    Timer.sleep(10)
  end
  let _ = File.write(markers <> "/signal", "received")?
  let _ = Jobs.stop(runner, 8000)?
  Jobs.join(runner)
end

fn run() -> String!String do
  let pool = Pool.open(database_url(), 1, 8, 5000)?
  let payload = Env.get("MESH_JOBS_PROOF_PAYLOAD", "")
  let result = case Env.get("MESH_JOBS_PROOF_COMMAND", "") do
    "seed" -> seed(pool)
    "enqueue" -> Jobs.enqueue(pool, namespace(), payload)
    "control" -> Jobs.control(pool, namespace(), payload)
    "serve" -> serve(pool, payload)
    _ -> Err("unknown proof command")
  end
  Pool.close(pool)
  result
end

fn main() do
  case run() do
    Ok(result) -> println(result)
    Err(error) ->
      println("proof error: " <> error)
      Process.exit(1)
  end
end
