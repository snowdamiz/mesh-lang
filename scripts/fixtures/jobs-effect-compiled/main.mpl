fn hold(path :: String, stage :: Int) -> Unit!String do
  let _ = File.write(path <> "/committed-${stage}", "committed")?
  let deadline = Monotonic.now_nanos() + 15_000_000_000
  while !File.exists(path <> "/release-${stage}") && Monotonic.now_nanos() < deadline do
    Timer.sleep(10)
  end
  if File.exists(path <> "/release-${stage}") do Ok(()) else Err("proof hold elapsed") end
end

fn run_handler(context :: String) -> String!String do
  let args = Json.object_get(Json.parse(context)?, "args")?
  let path = Json.as_string(Json.object_get(args, "path")?)?
  let label = Json.as_string(Json.object_get(args, "label")?)?
  let actor_reply = Job.await(Job.async(fn () -> "child-" <> label end))?
  if actor_reply != "child-" <> label do
    return Err("the first actor returned the wrong value")
  end
  let values = Json.object_get(json { values: [label] }, "values")?
  let _ = Jobs.effect("first", values)?
  hold(path, 1)?
  case Jobs.effect("second", values) do
    Ok(_) -> hold(path, 2)?
    Err(error) ->
      if error == "Jobs effect outcome is unconfirmed" do
        let _ = File.write(path <> "/ignored-unknown", error)?
        ()
      else
        return Err(error)
      end
  end
  Ok("{\"action\":\"complete\"}")
end

fn handler(context :: String) -> String do
  case run_handler(context) do
    Ok(outcome) -> outcome
    Err(error) -> Json.encode(json { action: "retry", reason: error })
  end
end

fn check_local_control(runner :: Int, listen :: Bool) -> Unit!String do
  let _ = case Jobs.local_control(runner, "{\"queue\":\"default\",\"action\":\"scale\",\"limit\":2}") do
    Ok(_) -> Err("effect runner accepted a scale-up")
    Err(_) -> Ok(())
  end?
  let _ = case Jobs.local_control(runner, "{\"queue\":\"second\",\"action\":\"start\"}") do
    Ok(_) -> Err("effect runner accepted a second queue")
    Err(_) -> Ok(())
  end?
  if !listen do
    let _ = Jobs.local_control(runner, "{\"queue\":\"default\",\"action\":\"pause\"}")?
    let _ = Jobs.local_control(runner, "{\"queue\":\"default\",\"action\":\"resume\"}")?
    ()
  end
  Ok(())
end

fn serve(pool :: PoolHandle) -> String!String do
  let registry = Json.parse("[{\"name\":\"first\",\"routine\":\"compiled_demo.first\",\"args\":[\"text\"]},{\"name\":\"second\",\"routine\":\"compiled_demo.second\",\"args\":[\"text\"]}]")?
  let listen = Env.get("MESH_EFFECT_LISTEN", "") == "1"
  let wakeup_config = if listen do
    "\"listen\":true,\"poll_ms\":60000"
  else
    "\"listen\":false,\"poll_ms\":25"
  end
  let config = "{\"namespace\":\"compiled-effect-proof\",\"queues\":{\"default\":1},\"workers\":[{\"worker\":\"proof\",\"version\":1}],\"claim_batch\":true,\"cron_interval_ms\":0,\"lease_ms\":60000," <> wakeup_config <> "}"
  let runner = Jobs.start_effects(pool, config, handler, Env.get("MESH_EFFECT_APP_URL", ""), registry)?
  check_local_control(runner, listen)?
  let marks = Env.get("MESH_EFFECT_MARKS", "")
  let _ = File.write(marks <> "/ready", Jobs.status(runner)?)?
  let deadline = Monotonic.now_nanos() + 45_000_000_000
  while !File.exists(marks <> "/stop") && Monotonic.now_nanos() < deadline do
    let _ = File.write(marks <> "/status", Jobs.status(runner)?)?
    Timer.sleep(25)
  end
  let _ = Jobs.stop(runner, 1000)?
  Jobs.join(runner)
end

fn run() -> String!String do
  let pool = Pool.open(Env.get("MESH_EFFECT_JOBS_URL", ""), 1, 1, 5000)?
  let result = if Env.get("MESH_EFFECT_MIGRATE", "") == "1" do
    Jobs.migrate(pool)?
    Ok("migrated")
  else
    serve(pool)
  end
  Pool.close(pool)
  result
end

fn main() do
  case run() do
    Ok(result) -> println(result)
    Err(error) ->
      println("compiled effect proof: " <> error)
      Process.exit(1)
  end
end
