fn database_url() -> String do
  Env.get("J5_DATABASE_URL", "")
end

fn record_effect(context :: String) -> String!String do
  let job = Json.parse(context)?
  let id = Json.as_string(Json.object_get(job, "id")?)?
  let conn = Pg.connect(database_url())?
  let written = Pg.execute(conn,
    "INSERT INTO public.j5_effects (job_id, context) VALUES ($1::uuid, $2::jsonb)",
    [id, context])
  Pg.close(conn)
  let _ = written?
  Ok("""{"action":"complete"}""")
end

fn handle(context :: String) -> String do
  case record_effect(context) do
    Ok(outcome) -> outcome
    Err(error) -> Json.encode(json { action: "discard", reason: error })
  end
end

fn measure(pool :: PoolHandle) -> String!String do
  let config = Env.get("J5_CONFIG", "")
  let options = Env.get("J5_OPTIONS", "")
  let started = Monotonic.now_nanos()
  let report = drain(pool, config, options)?
  let elapsed = Monotonic.now_nanos() - started
  Ok(Json.encode(json { elapsed_ns: elapsed, report: Json.parse(report)? }))
end

fn run(pool :: PoolHandle, action :: String) -> String!String do
  case action do
    "seed" ->
      let _ = Jobs.migrate(pool)?
      Jobs.enqueue(pool, "drain_perf", Env.get("J5_SEED", ""))
    "drain" -> measure(pool)
    _ -> Err("expected seed or drain")
  end
end

fn main() do
  let action = List.get(Env.args(), 1)
  case Pool.open(database_url(), 1, 1, 5000) do
    Ok(pool) ->
      let result = run(pool, action)
      Pool.close(pool)
      case result do
        Ok(output) -> println(output)
        Err(error) ->
          println("fixture error: " <> error)
          Process.exit(1)
      end
    Err(error) ->
      println("pool error: " <> error)
      Process.exit(1)
  end
end
