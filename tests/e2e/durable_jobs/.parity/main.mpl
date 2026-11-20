from WorkerIdentity import realtime_us, worker_pid

fn database_url() -> String do
  Env.get("JOBS_PARITY_DB_URL", "")
end

fn namespace() -> String do
  Env.get("JOBS_PARITY_NAMESPACE", "")
end

fn migrate(pool :: PoolHandle) -> String!String do
  let capacity = Env.get_int("JOBS_PARITY_TOTAL_SLOTS", 0)
  if capacity < 1 || capacity > 1024 do
    return Err("JOBS_PARITY_TOTAL_SLOTS must be between 1 and 1024")
  end
  let _ = Jobs.migrate(pool)?
  Jobs.control(pool, namespace(), json {
    action: "configure", queue: "default", concurrency: capacity
  })
end

fn job_spec(args :: Json) -> String!String do
  let key = Json.as_string(Json.object_get(args, "key")?)?
  let _ = Json.as_string(Json.object_get(args, "payload")?)?
  let hold = Json.as_int(Json.object_get(args, "hold_ms")?)?
  let _ = Json.as_bool(Json.object_get(args, "fail_first")?)?
  if hold < 0 do
    return Err("hold_ms must be nonnegative")
  end
  Ok(Json.encode(json {
    worker: "parity", args: args, request_key: key, max_attempts: 3,
    backoff_base_ms: 1000, backoff_max_ms: 1000
  }))
end

fn collect_specs(jobs :: Json, index :: Int, stop :: Int, specs :: List<String>, batch_bytes :: Int) -> List<String>!String do
  if index == stop do
    return Ok(specs)
  end
  let spec = job_spec(Json.array_get(jobs, index)?)?
  let spec_bytes = Bytes.length(Bytes.from_utf8(spec))
  let empty = List.length(specs) == 0
  let comma_bytes = if empty do 0 else 1 end
  let next_bytes = batch_bytes + spec_bytes + comma_bytes
  if next_bytes > 1_048_576 do
    if empty do
      return Err("encoded job exceeds the Jobs.enqueue_in input limit")
    end
    return Ok(specs)
  end
  collect_specs(jobs, index + 1, stop, List.append(specs, spec), next_bytes)
end

fn collect_ids(receipts :: Json, jobs :: Json, start :: Int, index :: Int, count :: Int, ids :: Map<String, String>) -> Map<String, String>!String do
  if index == count do
    return Ok(ids)
  end
  let job = Json.array_get(jobs, start + index)?
  let key = Json.as_string(Json.object_get(job, "key")?)?
  let receipt = Json.array_get(receipts, index)?
  let id = Json.as_string(Json.object_get(receipt, "id")?)?
  collect_ids(receipts, jobs, start, index + 1, count, Map.put(ids, key, id))
end

fn enqueue_batches(conn :: borrow PgConn, jobs :: Json, index :: Int, count :: Int, ids :: Map<String, String>) -> Map<String, String>!String do
  if index < count do
    let stop = if count - index > 128 do index + 128 else count end
    let specs = collect_specs(jobs, index, stop, [], 2)?
    let batch = "[" <> String.join(specs, ",") <> "]"
    let receipts = Json.parse(Jobs.enqueue_in(conn, namespace(), batch)?)?
    let size = List.length(specs)
    if Json.array_length(receipts)? != size do
      return Err("enqueue did not return one receipt per input")
    end
    let ids = collect_ids(receipts, jobs, index, 0, size, ids)?
    enqueue_batches(conn, jobs, index + size, count, ids)
  else
    Ok(ids)
  end
end

fn write_batch(conn :: borrow PgConn, jobs :: Json, count :: Int, request_id :: String) -> String!String do
  if Env.get("JOBS_PARITY_MESH_LISTEN", "0") == "1" do
    let _ = Pg.execute(conn, "SET LOCAL mesh_jobs.publish_wakeups = on", [])?
    ()
  end
  let _ = Pg.execute(conn,
    "INSERT INTO public.mesh_jobs_parity_product (engine, namespace, request_id) VALUES ('mesh', $1, $2) ON CONFLICT DO NOTHING",
    [namespace(), request_id])?
  let ids = enqueue_batches(conn, jobs, 0, count, Map.new())?
  let rows = Pg.query(conn, "SELECT pg_current_xact_id()::text AS xid", [])?
  let xid = Map.get(List.head(rows), "xid")
  Ok(Json.encode(json { count: count, xid: xid, ids: ids }))
end

fn transact_batch(conn :: borrow PgConn, input :: Json) -> String!String do
  let jobs = Json.object_get(input, "jobs")?
  let count = Json.array_length(jobs)?
  let request_id = Json.as_string(Json.object_get(input, "request_id")?)?
  let rollback = Json.as_bool(Json.object_get(input, "rollback")?)?
  let _ = Pg.begin(conn)?
  case write_batch(conn, jobs, count, request_id) do
    Ok(receipt) ->
      let _ = (if rollback do Pg.rollback(conn) else Pg.commit(conn) end)?
      Ok(receipt)
    Err(error) ->
      let _ = Pg.rollback(conn)?
      Err(error)
  end
end

fn enqueue() -> String!String do
  let path = Env.get("JOBS_PARITY_INPUT", "")
  if !String.starts_with(path, "/") do
    return Err("JOBS_PARITY_INPUT must be an absolute path")
  end
  let input = Json.parse(File.read(path)?)?
  let conn = Pg.connect(database_url())?
  let result = transact_batch(conn, input)
  Pg.close(conn)
  result
end

fn record_attempt(conn :: borrow PgConn, entered_us :: Int, key :: String, effect :: String, attempt :: Int) -> Int!String do
  Pg.execute(conn,
    """
    INSERT INTO public.mesh_jobs_parity_attempts
      (engine, namespace, job_key, effect_key, attempt, entered_at, worker_pid)
    VALUES ('mesh', $1, $2, $3, $4::integer,
      to_timestamp(($5::bigint / 1000000)::double precision)
        + ($5::bigint % 1000000) * interval '1 microsecond',
      $6::integer)
    """,
    [namespace(), key, effect, String.from(attempt), String.from(entered_us), String.from(worker_pid())])
end

fn write_effect(conn :: borrow PgConn, key :: String, effect :: String) -> Int!String do
  Pg.execute(conn,
    """
    INSERT INTO public.mesh_jobs_parity_effects
      (engine, namespace, job_key, effect_key, effected_at)
    VALUES ('mesh', $1, $2, $3, clock_timestamp())
    ON CONFLICT DO NOTHING
    """,
    [namespace(), key, effect])
end

fn perform(context :: String, entered_us :: Int) -> String!String do
  if entered_us < 0 do
    return Err("cannot capture handler entry time")
  end
  let job = Json.parse(context)?
  let args = Json.object_get(job, "args")?
  let key = Json.as_string(Json.object_get(args, "key")?)?
  let effect = Json.as_string(Json.object_get(job, "effect_key")?)?
  let attempt = Json.as_int(Json.object_get(job, "attempt")?)?
  let hold = Json.as_int(Json.object_get(args, "hold_ms")?)?
  let fail_first = Json.as_bool(Json.object_get(args, "fail_first")?)?

  let conn = Pg.connect(database_url())?
  let recorded = record_attempt(conn, entered_us, key, effect, attempt)
  let result = case recorded do
    Ok(_) ->
      if !fail_first && hold > 0 do
        Timer.sleep(hold)
      end
      write_effect(conn, key, effect)
    Err(error) -> Err(error)
  end
  Pg.close(conn)
  let _ = result?

  if fail_first && attempt == 1 do
    if hold > 0 do
      Timer.sleep(hold)
    end
    Ok("""{"action":"retry_after","delay_ms":1000,"reason":"parity fail_first"}""")
  else
    let gate = Env.get("JOBS_PARITY_RETRY_GATE", "")
    if fail_first && gate != "" do
      while !File.exists(gate) do
        Timer.sleep(10)
      end
    end
    Ok("""{"action":"complete"}""")
  end
end

fn handle(context :: String) -> String do
  let entered_us = realtime_us()
  case perform(context, entered_us) do
    Ok(outcome) -> outcome
    Err(error) ->
      IO.eprintln("parity handler " <> Env.get("JOBS_PARITY_RUNNER", "") <> ": " <> error)
      Json.encode(json { action: "fail", reason: "parity handler write failed" })
  end
end

fn serve(pool :: PoolHandle) -> String!String do
  let slots = Env.get_int("JOBS_PARITY_SLOTS", 0)
  let poll = Env.get_int("JOBS_PARITY_POLL_MS", 0)
  if Env.get("JOBS_PARITY_RUNNER", "") == "" do
    return Err("JOBS_PARITY_RUNNER is required")
  end
  let lease = case Env.get("JOBS_PARITY_MODE", "") do
    "dense" -> 30000
    "sparse" -> 30000
    "fault" -> 1500
    _ -> 0
  end
  if lease == 0 do
    return Err("JOBS_PARITY_MODE must be dense, sparse, or fault")
  end
  let grace = 15000
  let workers = Json.parse("""[{"worker":"parity","version":1}]""")?
  Process.install_shutdown_signals()
  let base_config = Json.encode(json {
    namespace: namespace(), queues: json { default: slots },
    workers: workers,
    lease_ms: lease, poll_ms: poll, shutdown_grace_ms: grace
  })
  let batch = if Env.get("JOBS_PARITY_MESH_BATCH", "0") == "1" do ",\"claim_batch\":true" else "" end
  let listen = if Env.get("JOBS_PARITY_MESH_LISTEN", "0") == "1" do ",\"listen\":true" else "" end
  let events = case Env.get("JOBS_PARITY_MESH_EVENTS", "") do
    "logged" -> ",\"event_log\":true"
    "backpressured" -> ",\"event_buffer\":128"
    _ -> ""
  end
  let config = String.slice(base_config, 0, String.length(base_config) - 1) <> batch <> listen <> events <> "}"
  let runner = Jobs.start(pool, config, handle)?
  println("READY")
  while !Process.shutdown_requested() do
    Timer.sleep(25)
  end
  let _ = Jobs.stop(runner, grace)?
  Jobs.join(runner)
end

fn run() -> String!String do
  let argv = Env.args()
  if List.length(argv) != 2 do
    return Err("usage: jobs-parity-mesh migrate|enqueue|run")
  end
  let action = List.get(argv, 1)
  if action != "migrate" && action != "enqueue" && action != "run" do
    return Err("usage: jobs-parity-mesh migrate|enqueue|run")
  end
  if !Regex.is_match(~r/^[a-z][a-z0-9_]{0,62}$/, namespace()) do
    return Err("JOBS_PARITY_NAMESPACE must match [a-z][a-z0-9_]{0,62}")
  end
  if database_url() == "" do
    return Err("JOBS_PARITY_DB_URL is required")
  end
  if action == "enqueue" do
    return enqueue()
  end
  let pool = Pool.open(database_url(), 1, 4, 5000)?
  let result = if action == "migrate" do migrate(pool) else serve(pool) end
  Pool.close(pool)
  result
end

fn main() do
  case run() do
    Ok(result) -> println(result)
    Err(error) ->
      IO.eprintln("parity error: " <> error)
      Process.exit(1)
  end
end
