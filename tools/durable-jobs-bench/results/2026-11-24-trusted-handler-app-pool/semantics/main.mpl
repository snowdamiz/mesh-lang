fn trusted(namespace :: String) -> String!String do
  let workers = Json.parse("""[{"worker":"arbitrary","version":1}]""")?
  Ok(Json.encode(json {
    namespace: namespace, queues: json { default: 1 },
    workers: workers,
    execution: "trusted_shared_process", poll_ms: 20
  }))
end

fn protected_config() -> String do
  """{"namespace":"protected","queues":{"default":1},"workers":[{"worker":"arbitrary","version":1}]}"""
end

fn perform(context :: String, app :: PoolHandle) -> String!String do
  let args = Json.object_get(Json.parse(context)?, "args")?
  let value = Json.as_string(Json.object_get(args, "value")?)?
  let _ = Pool.execute(app, "INSERT INTO seen (value) VALUES ($1)", [value])?
  Ok("""{"action":"complete"}""")
end

fn handle(context :: String, app :: PoolHandle) -> String do
  case perform(context, app) do
    Ok(outcome) -> outcome
    Err(_) -> """{"action":"fail","reason":"application pool write failed"}"""
  end
end

fn enqueue(queue :: PoolHandle, namespace :: String, value :: String) -> Unit!String do
  let _ = Jobs.enqueue(queue, namespace, Json.encode(json {
    worker: "arbitrary", args: json { value: value }, request_key: value
  }))?
  Ok(())
end

fn completed(runner :: Int, count :: Int, tries :: Int) -> Unit!String do
  let report = Json.parse(Jobs.status(runner)?)?
  let outcomes = Json.object_get(report, "outcomes")?
  if Json.as_int(Json.object_get(outcomes, "completed")?)? == count do
    return Ok(())
  end
  if tries == 0 do return Err("scoped runner did not complete its jobs") end
  Timer.sleep(25)
  completed(runner, count, tries - 1)
end

fn stop(runner :: Int) -> Unit!String do
  let _ = Jobs.stop(runner, 5000)?
  let _ = Jobs.join(runner)?
  Ok(())
end

fn verify(app :: PoolHandle, expected :: String) -> Unit!String do
  let rows = Pool.query(app, "SELECT string_agg(value, ',' ORDER BY value) AS seen_values FROM seen", [])?
  if Map.get(List.head(rows), "seen_values") == expected do Ok(()) else Err("app pool crossed runner boundary") end
end

fn run() -> Unit!String do
  let queue = Pool.open(Env.get("PROOF_QUEUE", ""), 1, 2, 5000)?
  let a = Pool.open(Env.get("PROOF_APP_A", ""), 1, 1, 5000)?
  let b = Pool.open(Env.get("PROOF_APP_B", ""), 1, 1, 5000)?
  let _ = Pool.execute(a, "CREATE TABLE seen (value text NOT NULL)", [])?
  let _ = Pool.execute(b, "CREATE TABLE seen (value text NOT NULL)", [])?
  let _ = Jobs.migrate(queue)?
  case Jobs.start_with_app_pool(queue, protected_config(), handle, a) do
    Ok(_) -> return Err("protected runner acquired application pool")
    Err(error) ->
      if error != "Jobs.start_with_app_pool requires trusted_shared_process execution" do return Err(error) end
  end
  case Jobs.start_with_app_pool(queue, trusted("same")?, handle, queue) do
    Ok(_) -> return Err("queue pool accepted as application pool")
    Err(error) ->
      if error != "runner requires a distinct, open application pool" do return Err(error) end
  end
  let runner_a = Jobs.start_with_app_pool(queue, trusted("app_a")?, handle, a)?
  let runner_b = Jobs.start_with_app_pool(queue, trusted("app_b")?, handle, b)?
  let _ = enqueue(queue, "app_a", "only_a")?
  let _ = enqueue(queue, "app_b", "only_b")?
  let _ = completed(runner_a, 1, 200)?
  let _ = completed(runner_b, 1, 200)?
  let _ = stop(runner_a)?
  let _ = verify(a, "only_a")?
  Pool.close(a)
  case Jobs.start_with_app_pool(queue, trusted("closed")?, handle, a) do
    Ok(_) -> return Err("closed pool accepted")
    Err(error) ->
      if error != "runner requires a distinct, open application pool" do return Err(error) end
  end
  let _ = enqueue(queue, "app_b", "after_a")?
  let _ = completed(runner_b, 2, 200)?
  let _ = stop(runner_b)?
  let _ = verify(b, "after_a,only_b")?
  Pool.close(b)
  Pool.close(queue)
  Ok(())
end

fn main() do
  case run() do
    Ok(_) -> println("APP_POOL_SCOPE_OK")
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
