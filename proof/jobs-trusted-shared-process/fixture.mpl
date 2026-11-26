fn probe(context :: String) -> String!String do
  let job = Json.parse(context)?
  let args = Json.object_get(job, "args")?
  let key = Json.as_string(Json.object_get(args, "receipt_key")?)?
  let attempt = Json.as_int(Json.object_get(job, "attempt")?)?
  println("handler:" <> key)
  if key == "panic" && attempt == 1 do
    panic("trusted attempt panic")
  end
  if (key == "parked-cancel" || key == "timeout") && attempt == 1 do
    Timer.sleep(10_000)
  end
  if key == "gc" do
    for _ in 0..32 do
      let _ = String.repeat("é", 8192)
      Timer.sleep(1)
    end
  end
  let reply = handle_trusted_receipt(context)
  if key == "lost-owner" && attempt == 1 do
    Timer.sleep(10_000)
  end
  Ok(reply)
end

fn proof_handler(context :: String) -> String do
  case probe(context) do
    Ok(reply) -> reply
    Err(error) -> Json.encode(json { action: "discard", reason: error })
  end
end

fn serve(pool :: PoolHandle) -> String!String do
  let config = Env.get("RUNNER_CONFIG", "")
  let runner = if config == "example" do
    start_trusted_receipts(pool)?
  else
    Jobs.start(pool, config, proof_handler)?
  end
  println("ready")
  while !File.exists(Env.get("STOP_FILE", "")) do
    Timer.sleep(10)
  end
  let report = Jobs.stop(runner, 0)?
  println("stop:" <> report)
  Jobs.join(runner)
end

fn run() -> String!String do
  let pool = Pool.open(Env.get("QUEUE_DATABASE_URL", ""), 1, 2, 1000)?
  let command = Env.get("ACTION", "")
  let result = case command do
    "serve" -> serve(pool)
    "migrate" -> do
      let _ = Jobs.migrate(pool)?
      Ok("migrated")
    end
    "enqueue" -> Jobs.enqueue(pool, "tenant-a", Env.get("SPEC", ""))
    "control" -> Jobs.control(pool, "tenant-a", Env.get("SPEC", ""))
    "drain" -> Jobs.drain(pool, Env.get("RUNNER_CONFIG", ""), proof_handler,
      """{"max_jobs":1,"recursive":false,"on_error":"stop"}""")
    _ -> Err("unknown proof command")
  end
  Pool.close(pool)
  result
end

fn main() do
  case run() do
    Ok(value) -> println(value)
    Err(error) -> do
      IO.eprintln(error)
      Process.exit(1)
    end
  end
end
