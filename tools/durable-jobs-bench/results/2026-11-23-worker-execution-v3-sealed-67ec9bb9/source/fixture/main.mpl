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
  let max_attempts = if Env.get("JOBS_PARITY_MODE", "") == "final_attempt" do 1 else 3 end
  Ok(Json.encode(json {
    worker: "parity", args: args, request_key: key, max_attempts: max_attempts,
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
    let specs = collect_specs(jobs, index, stop, [], 2)?
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
    let specs = collect_specs(jobs, 0, count, [], 2)?
    if List.length(specs) != count do
      return Err("producer commit probe input exceeds the enqueue batch limit")
    end
    let receipts = enqueue_specs(conn, specs, count)?
    let ids = collect_ids(receipts, jobs, 0, 0, count, Map.new())?
    let xid = transaction_id(conn)?
    return Ok(Json.encode(json { count: count, xid: xid, ids: ids, receipts: receipts }))
  end
  let ids = enqueue_batches(conn, jobs, 0, count, Map.new())?
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

fn wait_mark(path :: String) -> Unit!String do
  let deadline = Monotonic.now_nanos() + 15_000_000_000
  while !File.exists(path) && Monotonic.now_nanos() < deadline do
    Timer.sleep(5)
  end
  if File.exists(path) do Ok(()) else Err("proof hold elapsed") end
end

fn effect_result(name :: String, values :: Json) -> String do
  case Jobs.effect(name, values) do
    Ok(applied) -> if applied do "applied" else "prior" end
    Err(error) -> error
  end
end

fn complete() -> String do
  """{"action":"complete"}"""
end

fn perform(context :: String, entered_us :: Int) -> String!String do
  let job = Json.parse(context)?
  let args = Json.object_get(job, "args")?
  let key = Json.as_string(Json.object_get(args, "key")?)?
  let attempt = Json.as_int(Json.object_get(job, "attempt")?)?
  let hold = Json.as_int(Json.object_get(args, "hold_ms")?)?
  let fail_first = Json.as_bool(Json.object_get(args, "fail_first")?)?
  if entered_us < 0 || fail_first || Env.get("LD_PRELOAD", "") != "" || Env.get("JOBS_PARITY_DB_URL", "") != "" || Env.get("MESH_EFFECT_APP_URL", "") != "" do
    return Err("unexpected handler input or inherited runner environment")
  end
  let proof = case Json.object_get(args, "proof_path") do
    Ok(path) -> Json.as_string(path)?
    Err(_) -> ""
  end
  let marks = if proof == "" do "" else proof <> "/" <> String.from(attempt) end
  let is_proof = marks != ""
  let pid = worker_pid()
  if is_proof do
    let _ = File.write(marks <> "/worker", String.from(pid))?
    wait_mark(marks <> "/call")?
  end
  let encoded_key = Json.encode(key)
  let witness = if is_proof do "true" else "false" end
  let values = Json.parse("[${encoded_key},${entered_us},${pid},${witness}]")?
  if is_proof do
    let _ = File.write(marks <> "/calling", String.from(Monotonic.now_nanos()))?
    ()
  end
  let first = effect_result("first", values)
  if is_proof do
    let _ = File.write(marks <> "/first", Json.encode(json {
      outcome: first, monotonic_ns: Monotonic.now_nanos()
    }))?
    if first == "Jobs effect outcome is unconfirmed" do
      wait_mark(marks <> "/finish")?
      let _ = File.write(marks <> "/returned-complete", first)?
      return Ok(complete())
    end
    wait_mark(marks <> "/second-call")?
  end
  if first != "applied" && first != "prior" do return Err(first) end
  if hold > 0 do Timer.sleep(hold) end
  let second_values = Json.parse("[${encoded_key},${witness}]")?
  let second = effect_result("second", second_values)
  if second != "applied" && second != "prior" do return Err(second) end
  if is_proof do
    let _ = File.write(marks <> "/second", second)?
    wait_mark(marks <> "/finish")?
  end
  Ok(complete())
end

fn handle(context :: String) -> String do
  let entered_us = realtime_us()
  case perform(context, entered_us) do
    Ok(outcome) -> outcome
    Err(error) ->
      IO.eprintln("named effect handler: " <> error)
      Json.encode(json { action: "fail", reason: "named effect handler failed" })
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
  let lanes = Env.get_int("MESH_EFFECT_LANES", 0)
  if lanes < 1 || lanes > slots || Env.get("JOBS_PARITY_MESH_BATCH", "") != "1" do
    return Err("named effects require prewarmed lanes and batch claims")
  end
  let effects_base_config = String.slice(config, 0, String.length(config) - 1) <> ",\"effect_sessions\":" <> String.from(lanes) <> "}"
  let effects_config = if Env.get("JOBS_PARITY_MESH_EXECUTION", "strict") == "trusted-persistent" do
    String.slice(effects_base_config, 0, String.length(effects_base_config) - 1) <>
      ""","trusted_persistent_workers":[{"worker":"parity","version":1}]}"""
  else
    effects_base_config
  end
  let registry = Json.parse("""[{"name":"first","routine":"compiled_demo.first","args":["text","int","int","bool"]},{"name":"second","routine":"compiled_demo.second","args":["text","bool"]}]""")?
  let runner = Jobs.start_effects(pool, effects_config, handle, Env.get("MESH_EFFECT_APP_URL", ""), registry)?
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
