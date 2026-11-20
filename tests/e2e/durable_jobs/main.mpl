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
  if label == "panic" do
    panic("proof worker panicked")
  end
  let worker = Env.get("MESH_JOBS_PROOF_WORKER", "")
  let conn = Pg.connect(database_url())?
  let result = Pg.execute(conn,
    "INSERT INTO " <> namespace() <> ".effects(worker, label, context) VALUES ($1, $2, $3::jsonb)",
    [worker, label, context])
  Pg.close(conn)
  let _ = result?
  if (label == "native-timeout" || label == "native-owner-paused") && attempt == 1 do
    let blocker = Pg.connect(database_url())?
    let _ = Pg.query(blocker, "SELECT set_config('application_name', $1, false)", [label])?
    let blocked = Pg.query(blocker, "SELECT pg_sleep(60)", [])
    let resumed = Pg.execute(blocker,
      "INSERT INTO "
        <> namespace()
        <> ".effects(worker, label, context) VALUES ($1, 'after-timeout', $2::jsonb)",
      [worker, context])
    Pg.close(blocker)
    let _ = blocked?
    let _ = resumed?
    ()
  end
  let gate = Env.get("MESH_JOBS_PROOF_GATE", "")
  if gate != "" do
    while !File.exists(gate) do
      Timer.sleep(10)
    end
  end
  if label == "poison" do
    Ok("""{"action":"fail","reason":"proof worker rejected poison"}""")
  else if (label == "retry" || label == "event-exhaustion") && attempt == 1 do
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
  let capture_events = Env.get("MESH_JOBS_PROOF_EVENTS", "") == "true"
  let status = Jobs.status(runner)?
  let _ = File.write(markers <> "/ready", status)?
  while !Process.shutdown_requested() do
    let _ = File.write(markers <> "/status", Jobs.status(runner)?)?
    if capture_events do
      let batch = Jobs.read_events(runner, 128)?
      File.append(markers <> "/events", batch <> "\n")?
    end
    Timer.sleep(10)
  end
  let _ = File.write(markers <> "/signal", "received")?
  let grace = Json.as_int(Json.object_get(Json.parse(config)?, "shutdown_grace_ms")?)?
  let stopping = Jobs.stop(runner, grace)?
  let _ = File.write(markers <> "/stop", stopping)?
  let joined = Jobs.join(runner)?
  if capture_events do
    let batch = Jobs.read_events(runner, 128)?
    File.append(markers <> "/events", batch <> "\n")?
  end
  let _ = File.write(markers <> "/joined", joined)?
  Ok(joined)
end

fn run() -> String!String do
  let pool = Pool.open(database_url(), 1, 8, 5000)?
  let payload = Env.get("MESH_JOBS_PROOF_PAYLOAD", "")
  let result = case Env.get("MESH_JOBS_PROOF_COMMAND", "") do
    "seed" -> seed(pool)
    "enqueue" -> Jobs.enqueue(pool, namespace(), payload)
    "enqueue_file" -> Jobs.enqueue(pool, namespace(), File.read(payload)?)
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
