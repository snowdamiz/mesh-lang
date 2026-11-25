from WorkerIdentity import realtime_us, worker_pid
from Writes import record_attempt, record_effect

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
  let max_attempts = if Env.get("JOBS_PARITY_MODE", "") == "final_attempt" do 1 else 3 end
  Ok(Json.encode(json {
    worker: "parity", args: args, request_key: key, max_attempts: max_attempts,
    backoff_base_ms: 1000, backoff_max_ms: 1000
  }))
end

fn collect_specs(jobs :: Json, index :: Int, stop :: Int, specs :: List<String>, batch_bytes :: Int, limit :: Int) -> List<String>!String do
  if index == stop do
    return Ok(specs)
  end
  let spec = job_spec(Json.array_get(jobs, index)?)?
  let spec_bytes = Bytes.length(Bytes.from_utf8(spec))
  let empty = List.length(specs) == 0
  let comma_bytes = if empty do 0 else 1 end
  let next_bytes = batch_bytes + spec_bytes + comma_bytes
  if next_bytes > limit do
    if empty do
      return Err("encoded job exceeds the Jobs.enqueue_in input limit")
    end
    return Ok(specs)
  end
  collect_specs(jobs, index + 1, stop, List.append(specs, spec), next_bytes, limit)
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

fn enqueue_specs(conn :: borrow PgConn, specs :: List<String>, size :: Int) -> Json!String do
  let batch = "[" <> String.join(specs, ",") <> "]"
  let receipts = Json.parse(Jobs.enqueue_in(conn, namespace(), batch)?)?
  if Json.array_length(receipts)? != size do
    return Err("enqueue did not return one receipt per input")
  end
  Ok(receipts)
end

fn enqueue_batches(conn :: borrow PgConn, jobs :: Json, index :: Int, count :: Int, ids :: Map<String, String>) -> Map<String, String>!String do
  if index < count do
    let stop = if count - index > 128 do index + 128 else count end
    let specs = collect_specs(jobs, index, stop, [], 2, 1_048_576)?
    let size = List.length(specs)
    let receipts = enqueue_specs(conn, specs, size)?
    let ids = collect_ids(receipts, jobs, index, 0, size, ids)?
    enqueue_batches(conn, jobs, index + size, count, ids)
  else
    Ok(ids)
  end
end

fn transaction_id(conn :: borrow PgConn) -> String!String do
  let rows = Pg.query(conn, "SELECT pg_current_xact_id()::text AS xid", [])?
  Ok(Map.get(List.head(rows), "xid"))
end

fn write_batch(conn :: borrow PgConn, jobs :: Json, count :: Int, request_id :: String, mode :: String) -> String!String do
  let notify_proof = mode == "notify_prepare" || mode == "notify_commit"
  if Env.get("JOBS_PARITY_MESH_LISTEN", "0") == "1" do
    let _ = Pg.execute(conn, "SET LOCAL mesh_jobs.publish_wakeups = on", [])?
    ()
  else if notify_proof do
    let _ = Pg.execute(conn, "SET LOCAL mesh_jobs.publish_wakeups = off", [])?
    ()
  end
  let _ = Pg.execute(conn,
    "INSERT INTO public.mesh_jobs_parity_product (engine, namespace, request_id) VALUES ('mesh', $1, $2) ON CONFLICT DO NOTHING",
    [namespace(), request_id])?
  if mode == "producer_commit" || notify_proof do
    if count < 1 || count > 128 do
      return Err("producer commit probe requires one nonempty enqueue batch")
    end
    let specs = collect_specs(jobs, 0, count, [], 2, 1_048_576)?
    if List.length(specs) != count do
      return Err("producer commit probe input exceeds the enqueue batch limit")
    end
    let receipts = enqueue_specs(conn, specs, count)?
    let ids = collect_ids(receipts, jobs, 0, 0, count, Map.new())?
    let xid = transaction_id(conn)?
    return Ok(Json.encode(json { count: count, xid: xid, ids: ids, receipts: receipts }))
  end
  let ids = if count == 1024 && Env.get("MESH_JOBS_DENSE_PRODUCER_1024", "0") == "1" do
    let specs = collect_specs(jobs, 0, count, [], 2, 2_097_152)?
    if List.length(specs) != count do
      return Err("private dense enqueue requires one complete frame")
    end
    let receipts = enqueue_specs(conn, specs, count)?
    collect_ids(receipts, jobs, 0, 0, count, Map.new())?
  else
    enqueue_batches(conn, jobs, 0, count, Map.new())?
  end
  let xid = transaction_id(conn)?
  Ok(Json.encode(json { count: count, xid: xid, ids: ids }))
end

fn finish_batch(conn :: borrow PgConn, rollback :: Bool, mode :: String) -> Unit!String do
  if rollback do
    return Pg.rollback(conn)
  end
  if mode == "notify_prepare" do
    case Pg.execute(conn, "PREPARE TRANSACTION 'jobs_parity_notify'", []) do
      Ok(_) -> Ok(())
      Err(error) -> Err("PREPARE TRANSACTION: " <> error)
    end
  else if mode == "notify_commit" do
    case Pg.commit(conn) do
      Ok(_) -> Ok(())
      Err(error) -> Err("COMMIT: " <> error)
    end
  else
    Pg.commit(conn)
  end
end

fn transact_batch(conn :: borrow PgConn, input :: Json) -> String!String do
  let jobs = Json.object_get(input, "jobs")?
  let count = Json.array_length(jobs)?
  let request_id = Json.as_string(Json.object_get(input, "request_id")?)?
  let rollback = Json.as_bool(Json.object_get(input, "rollback")?)?
  let mode = Env.get("JOBS_PARITY_MODE", "")
  let _ = Pg.begin(conn)?
  case write_batch(conn, jobs, count, request_id, mode) do
    Ok(receipt) ->
      let _ = finish_batch(conn, rollback, mode)?
      Ok(receipt)
    Err(error) ->
      let _ = Pg.rollback(conn)?
      Err(error)
  end
end

fn read_batch(path :: String) -> Json!String do
  if !String.starts_with(path, "/") do
    return Err("JOBS_PARITY_INPUT must be an absolute path")
  end
  Json.parse(File.read(path)?)
end

fn enqueue(path :: String) -> String!String do
  let input = read_batch(path)?
  let conn = Pg.connect(database_url())?
  let result = transact_batch(conn, input)
  Pg.close(conn)
  result
end

fn enqueue_paths() -> String!String do
  let path = IO.read_line()?
  if path == "stop" do
    return Ok("{}")
  end
  let receipt = enqueue(path)?
  println(receipt)
  enqueue_paths()
end

fn enqueue_stream() -> String!String do
  println("PRODUCER_READY")
  enqueue_paths()
end

service ApplicationPool do
  fn init(pool :: PoolHandle) -> PoolHandle do
    pool
  end

  call Current() :: PoolHandle do |pool|
    (pool, pool)
  end
end

fn perform(context :: String, entered_us :: Int) -> String!String do
  let job = Json.parse(context)?
  let args = Json.object_get(job, "args")?
  let ns = namespace()
  let job_id = Json.as_string(Json.object_get(job, "id")?)?
  let run_key = Json.as_string(Json.object_get(job, "effect_key")?)?
  let key = Json.as_string(Json.object_get(args, "key")?)?
  let _ = Json.as_string(Json.object_get(args, "payload")?)?
  let attempt = Json.as_int(Json.object_get(job, "attempt")?)?
  let hold = Json.as_int(Json.object_get(args, "hold_ms")?)?
  let fail_first = Json.as_bool(Json.object_get(args, "fail_first")?)?
  if key == "" || hold < 0 || entered_us < 0 do
    return Err("invalid trusted application input")
  end
  let pool = ApplicationPool.current(Process.whereis("parity-app-pool"))
  let _ = record_attempt(pool, ns, key, job_id, run_key, attempt, entered_us, worker_pid())?
  if hold > 0 do Timer.sleep(hold) end
  if fail_first && attempt == 1 do
    return Ok("""{"action":"retry_after","delay_ms":10,"reason":"fixture requested retry"}""")
  end
  let _ = record_effect(pool, ns, key)?
  Ok("""{"action":"complete"}""")
end

fn handle(context :: String) -> String do
  let entered_us = realtime_us()
  case perform(context, entered_us) do
    Ok(outcome) -> outcome
    Err(error) ->
      IO.eprintln("trusted application write failed: " <> error)
      Json.encode(json { action: "fail", reason: "trusted application write failed or commit was unconfirmed" })
  end
end

fn session_loop(conn :: borrow PgConn) -> String!String do
  let line = IO.read_line()?
  if line == "stop" do return Ok("{}") end
  let request = Json.parse(line)?
  let verb = Json.as_string(Json.object_get(request, "verb")?)?
  if verb == "enqueue" do
    let receipt = transact_batch(conn, Json.object_get(request, "batch")?)?
    println(receipt)
  else if verb == "query" do
    let sql = Json.as_string(Json.object_get(request, "sql")?)?
    let rows = Pg.query(conn, sql, [])?
    if List.length(rows) != 1 do return Err("observer SQL must return one row") end
    println(Map.get(List.head(rows), "value"))
  else
    return Err("unknown producer command")
  end
  session_loop(conn)
end

fn session() -> String!String do
  let conn = Pg.connect(database_url())?
  println("SESSION_READY")
  let result = session_loop(conn)
  Pg.close(conn)
  result
end

fn serve_started(pool :: PoolHandle) -> String!String do
  let slots = Env.get_int("JOBS_PARITY_SLOTS", 0)
  let poll = Env.get_int("JOBS_PARITY_POLL_MS", 0)
  if Env.get("JOBS_PARITY_RUNNER", "") == "" do
    return Err("JOBS_PARITY_RUNNER is required")
  end
  let lease = case Env.get("JOBS_PARITY_MODE", "") do
    "dense" -> 30000
    "sparse" -> 30000
    "fault" -> 1500
    "final_attempt" -> 10000
    _ -> 0
  end
  if lease == 0 do
    return Err("JOBS_PARITY_MODE must be dense, sparse, fault, or final_attempt")
  end
  let grace = 15000
  let workers = Json.parse("""[{"worker":"parity","version":1}]""")?
  Process.install_shutdown_signals()
  let base_config = Json.encode(json {
    namespace: namespace(), queues: json { default: slots },
    workers: workers,
    lease_ms: lease, poll_ms: poll, shutdown_grace_ms: grace, cron_interval_ms: 0
  })
  let batch = if Env.get("JOBS_PARITY_MESH_BATCH", "0") == "1" do ",\"claim_batch\":true" else "" end
  let listen = if Env.get("JOBS_PARITY_MESH_LISTEN", "0") == "1" do ",\"listen\":true" else "" end
  let events = case Env.get("JOBS_PARITY_MESH_EVENTS", "") do
    "logged" -> ",\"event_log\":true"
    "backpressured" -> ",\"event_buffer\":128"
    _ -> ""
  end
  let config = String.slice(base_config, 0, String.length(base_config) - 1) <> batch <> listen <> events <> "}"
  let trusted = String.slice(config, 0, String.length(config) - 1) <> ",\"execution\":\"trusted_shared_process\"}"
  let policy = Env.get("TRUSTED_ADMISSION_POLICY", "stock")
  if policy != "stock" && policy != "set_based" && policy != "success_cohort" do
    return Err("unknown ordinary admission policy")
  end
  let claims = if policy == "stock" do "" else ",\"claim_batch_strategy\":\"set_based\"" end
  let groups = if policy == "success_cohort" do ",\"group_success\":true" else "" end
  let ordinary_config = String.slice(trusted, 0, String.length(trusted) - 1) <> claims <> groups <> "}"
  let runner = Jobs.start(pool, ordinary_config, handle)?
  let status_path = Env.get("JOBS_PARITY_STATUS_PATH", "")
  println("READY")
  while !Process.shutdown_requested() do
    if status_path != "" && File.exists(status_path <> ".request") do
      let status = Jobs.status(runner)?
      let _ = File.rename(status_path <> ".request", status_path <> ".pending")?
      let _ = File.write(status_path <> ".pending", status)?
      let _ = File.rename(status_path <> ".pending", status_path)?
      ()
    end
    Timer.sleep(25)
  end
  let _ = Jobs.stop(runner, grace)?
  Jobs.join(runner)
end

fn serve(pool :: PoolHandle) -> String!String do
  let lanes = Env.get_int("MESH_EFFECT_LANES", 0)
  let slots = Env.get_int("JOBS_PARITY_SLOTS", 0)
  if lanes < 1 || lanes > slots do
    return Err("require one to slots warmed application connections")
  end
  let app_pool = Pool.open(Env.get("MESH_EFFECT_APP_URL", ""), lanes, lanes, 5000)?
  let owner = ApplicationPool.start(app_pool)
  let registered = Process.register("parity-app-pool", owner)
  let result = if registered == 0 do serve_started(pool) else Err("cannot register application pool") end
  Pool.close(app_pool)
  result
end

fn run() -> String!String do
  let argv = Env.args()
  if List.length(argv) != 2 do
    return Err("usage: jobs-parity-mesh migrate|enqueue|run")
  end
  let action = List.get(argv, 1)
  if action != "migrate" && action != "enqueue" && action != "run" && action != "session" do
    return Err("usage: jobs-parity-mesh migrate|enqueue|run")
  end
  if !Regex.is_match(~r/^[a-z][a-z0-9_]{0,62}$/, namespace()) do
    return Err("JOBS_PARITY_NAMESPACE must match [a-z][a-z0-9_]{0,62}")
  end
  if database_url() == "" do
    return Err("JOBS_PARITY_DB_URL is required")
  end
  if action == "session" do return session() end
  if action == "enqueue" do
    let path = Env.get("JOBS_PARITY_INPUT", "")
    if path == "-" do
      return enqueue_stream()
    end
    return enqueue(path)
  end
  let relay_url = Env.get("JOBS_PARITY_POOL_URL", "")
  let pool_url = if action == "run" && relay_url != "" do relay_url else database_url() end
  let pool = Pool.open(pool_url, 1, 1, 5000)?
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
