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

fn seed_transaction(pool :: PoolHandle) -> String!String do
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

fn enable_wakeups(conn :: borrow PgConn) -> Int!String do
  Pg.execute(conn, "SET mesh_jobs.publish_wakeups = on", [])
end

fn staged_wake_job(conn :: borrow PgConn) -> String!String do
  let _ = Pg.execute(conn, "SET LOCAL mesh_jobs.publish_wakeups = on", [])?
  let spec = Env.get("MESH_JOBS_PROOF_PAYLOAD", "")
  let receipt = Jobs.enqueue_in(conn, namespace(), spec)?
  let markers = Env.get("MESH_JOBS_PROOF_MARKERS", "")
  let _ = File.write(markers <> "/staged", receipt)?
  while !File.exists(markers <> "/commit") do
    Timer.sleep(10)
  end
  if Env.get("MESH_JOBS_PROOF_COMMAND", "") == "stage_rollback" do
    Err("intentional wake rollback")
  else
    Ok(receipt)
  end
end

fn stage_wake_rollback(pool :: PoolHandle) -> String!String do
  case Repo.transaction(pool, staged_wake_job) do
    Err(error) ->
      if error == "intentional wake rollback" do
        Ok("null")
      else
        Err(error)
      end
    Ok(_) -> Err("staged wake transaction should have rolled back")
  end
end

fn migrate(pool :: PoolHandle) -> String!String do
  let _ = Jobs.migrate(pool)?
  Ok("{}")
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
  if gate != "" && label == Env.get("MESH_JOBS_PROOF_GATE_LABEL", label) do
    while !File.exists(gate) do
      Timer.sleep(10)
    end
  end
  if label == "poison" || label == "j6-held-retry" do
    Ok("""{"action":"fail","reason":"proof worker rejected poison"}""")
  else if (label == "retry" || label == "event-exhaustion" || label == "wake-retry") && attempt == 1 do
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

fn chain_job(context :: String) -> String!String do
  let args = Json.object_get(Json.parse(context)?, "args")?
  let depth = Json.as_int(Json.object_get(args, "depth")?)?
  if depth > 0 do
    let pool = Pool.open(database_url(), 1, 1, 5000)?
    let result = Jobs.enqueue(pool,
      namespace(),
      json { worker: "proof", queue: "chain", args: json { label: "chain", depth: depth - 1 } })
    Pool.close(pool)
    let _ = result?
    ()
  end
  Ok("""{"action":"complete"}""")
end

fn bounded_outcome(context :: String) -> String!String do
  let queue = Json.as_string(Json.object_get(Json.parse(context)?, "queue")?)?
  if queue == "chain" do
    chain_job(context)
  else
    Ok(handle(context))
  end
end

fn bounded_handler(context :: String) -> String do
  case bounded_outcome(context) do
    Ok(outcome) -> outcome
    Err(error) -> Json.encode(json { action: "discard", reason: error })
  end
end

fn bounded_drain(pool :: PoolHandle, payload :: String) -> String!String do
  let payload = Json.parse(payload)?
  let config = Json.encode(Json.object_get(payload, "config")?)
  let options = Json.encode(Json.object_get(payload, "options")?)
  Jobs.drain(pool, config, bounded_handler, options)
end

fn hold_checkout(conn :: borrow PgConn) -> Int!String do
  let _ = Pg.query(conn, "SELECT set_config('application_name', $1, true)", [namespace()])?
  let markers = Env.get("MESH_JOBS_PROOF_MARKERS", "")
  let _ = File.write(markers <> "/pool-held", "held")?
  while !File.exists(markers <> "/pool-release") do
    Timer.sleep(10)
  end
  Ok(0)
end

actor exhaust_pool(pool :: PoolHandle) do
  case Repo.transaction(pool, hold_checkout) do
    Ok(_) -> ()
    Err(error) -> println("pool checkout failed: " <> error)
  end
end

fn serve(pool :: PoolHandle, config :: String, observe_after_join :: Bool) -> String!String do
  Process.install_shutdown_signals()
  let runner = Jobs.start(pool, config, handle)?
  let markers = Env.get("MESH_JOBS_PROOF_MARKERS", "")
  let capture_events = Env.get("MESH_JOBS_PROOF_EVENTS", "") == "true"
  let status = Jobs.status(runner)?
  let _ = File.write(markers <> "/ready", status)?
  while !Process.shutdown_requested() do
    if File.exists(markers <> "/pool-hold") do
      let _ = File.rename(markers <> "/pool-hold", markers <> "/pool-hold-started")?
      let _ = spawn(exhaust_pool, pool)
      ()
    end
    let command = markers <> "/local-control"
    if File.exists(command) do
      let _ = File.rename(command, markers <> "/local-control-read")?
      let action = File.read(markers <> "/local-control-read")?
      let response = case Jobs.local_control(runner, action) do
        Ok(report) -> report
        Err(error) -> Json.encode(json { error: error })
      end
      let _ = File.write(markers <> "/local-control-result", response)?
      ()
    end
    let _ = File.write(markers <> "/status", Jobs.status(runner)?)?
    if capture_events do
      let batch = Jobs.read_events(runner, 128)?
      File.append(markers <> "/events", batch <> "\n")?
    end
    Timer.sleep(10)
  end
  let _ = File.write(markers <> "/signal", "received")?
  let grace = Json.as_int(Json.object_get(Json.parse(config)?, "shutdown_grace_ms")?)?
  let stop_start = Monotonic.now_nanos()
  let stopping = Jobs.stop(runner, grace)?
  let stop_ns = Monotonic.now_nanos() - stop_start
  let _ = File.write(markers <> "/stop", stopping)?
  let _ = File.write(markers <> "/stop-ns", "${stop_ns}\n")?
  let joined = Jobs.join(runner)?
  let _ = File.write(markers <> "/joined", joined)?
  if capture_events do
    let batch = Jobs.read_events(runner, 128)?
    File.append(markers <> "/events", batch <> "\n")?
  end
  if observe_after_join do
    let finish = markers <> "/finish-post-join"
    let deadline = Monotonic.now_nanos() + 60_000_000_000
    while !File.exists(finish) && Monotonic.now_nanos() < deadline do
      let _ = File.write(markers <> "/post-join-status", Jobs.status(runner)?)?
      Timer.sleep(25)
    end
    if File.exists(finish) do
      Jobs.status(runner)
    else
      Err("post-join status observation timed out")
    end
  else
    Ok(joined)
  end
end

fn run() -> String!String do
  let pool_size = if Env.get("MESH_JOBS_PROOF_ONE_SLOT", "") == "true" do 1 else 8 end
  let pool = Pool.open(database_url(), 1, pool_size, 5000)?
  if Env.get("MESH_JOBS_PROOF_PUBLISH", "") == "on" do
    let _ = Repo.transaction(pool, enable_wakeups)?
    ()
  end
  let payload = Env.get("MESH_JOBS_PROOF_PAYLOAD", "")
  let result = case Env.get("MESH_JOBS_PROOF_COMMAND", "") do
    "seed_transaction" -> seed_transaction(pool)
    "migrate" -> migrate(pool)
    "enqueue" -> Jobs.enqueue(pool, namespace(), payload)
    "enqueue_file" -> Jobs.enqueue(pool, namespace(), File.read(payload)?)
    "control" -> Jobs.control(pool, namespace(), payload)
    "stage_commit" -> Repo.transaction(pool, staged_wake_job)
    "stage_rollback" -> stage_wake_rollback(pool)
    "serve" -> serve(pool, payload, false)
    "serve_with_post_join_status" -> serve(pool, payload, true)
    "drain" -> bounded_drain(pool, payload)
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
