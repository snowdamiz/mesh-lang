fn database_url() -> String do
  Env.get("JOBS_UPGRADE_DB_URL", "")
end

fn namespace() -> String do
  Env.get("JOBS_UPGRADE_NAMESPACE", "")
end

fn enqueue_in(conn :: borrow PgConn, spec :: String) -> String!String do
  let _ = Pg.begin(conn)?
  case Jobs.enqueue_in(conn, namespace(), spec) do
    Ok(receipt) ->
      let _ = Pg.commit(conn)?
      Ok(receipt)
    Err(error) ->
      let _ = Pg.rollback(conn)?
      Err(error)
  end
end

fn enqueue() -> String!String do
  let spec = File.read(Env.get("JOBS_UPGRADE_SPEC", ""))?
  let conn = Pg.connect(database_url())?
  let result = enqueue_in(conn, spec)
  Pg.close(conn)
  result
end

fn perform(context :: String) -> String!String do
  let job = Json.parse(context)?
  let args = Json.object_get(job, "args")?
  let label = Json.as_string(Json.object_get(args, "label")?)?
  let id = Json.as_string(Json.object_get(job, "id")?)?
  let effect = Json.as_string(Json.object_get(job, "effect_key")?)?
  let attempt = Json.as_int(Json.object_get(job, "attempt")?)?
  let conn = Pg.connect(database_url())?
  let recorded = Pg.execute(conn,
    """
    INSERT INTO public.upgrade_attempts (job_id, effect_key, attempt, worker_role)
    VALUES ($1::uuid, $2, $3::integer, current_user)
    """,
    [id, effect, String.from(attempt)])
  Pg.close(conn)
  let _ = recorded?

  if label == "retryable" && attempt == 1 do
    return Ok("""{"action":"retry_after","delay_ms":15000,"reason":"upgrade proof"}""")
  end
  if label == "held" do
    let gate = Env.get("JOBS_UPGRADE_GATE", "")
    while !File.exists(gate) do
      Timer.sleep(10)
    end
  end
  let conn = Pg.connect(database_url())?
  let written = Pg.execute(conn,
    """
    INSERT INTO public.upgrade_effects (effect_key, job_id, worker_role)
    VALUES ($1, $2::uuid, current_user)
    """,
    [effect, id])
  Pg.close(conn)
  let _ = written?
  Ok("""{"action":"complete"}""")
end

fn handle(context :: String) -> String do
  case perform(context) do
    Ok(outcome) -> outcome
    Err(error) ->
      IO.eprintln("upgrade handler: " <> error)
      Json.encode(json { action: "fail", reason: error })
  end
end

fn serve(pool :: PoolHandle) -> String!String do
  let queues = Json.parse(Env.get("JOBS_UPGRADE_QUEUES", ""))?
  let workers = Json.parse("""[{"worker":"upgrade","version":1}]""")?
  let base = Json.encode(json {
    namespace: namespace(), queues: queues, workers: workers, lease_ms: 6000,
    poll_ms: 50, shutdown_grace_ms: 10000
  })
  let config = case Env.get("JOBS_UPGRADE_MODE", "legacy") do
    "batch" ->
      String.slice(base, 0, String.length(base) - 1) <>
        ",\"claim_batch\":true,\"cron_interval_ms\":0}"
    "single" ->
      String.slice(base, 0, String.length(base) - 1) <> ",\"cron_interval_ms\":0}"
    _ -> base
  end
  Process.install_shutdown_signals()
  let runner = Jobs.start(pool, config, handle)?
  println("READY")
  while !Process.shutdown_requested() do
    Timer.sleep(25)
  end
  let _ = Jobs.stop(runner, 10000)?
  Jobs.join(runner)
end

fn run() -> String!String do
  let args = Env.args()
  if List.length(args) != 2 do
    return Err("usage: jobs-upgrade migrate|enqueue|run")
  end
  let action = List.get(args, 1)
  if action == "enqueue" do
    return enqueue()
  end
  if action != "migrate" && action != "run" do
    return Err("usage: jobs-upgrade migrate|enqueue|run")
  end
  let pool = Pool.open(database_url(), 1, 6, 5000)?
  let result = if action == "migrate" do migrate(pool) else serve(pool) end
  Pool.close(pool)
  result
end

fn migrate(pool :: PoolHandle) -> String!String do
  let _ = Jobs.migrate(pool)?
  Ok("MIGRATED")
end

fn main() do
  case run() do
    Ok(result) -> println(result)
    Err(error) ->
      IO.eprintln("upgrade fixture: " <> error)
      Process.exit(1)
  end
end
