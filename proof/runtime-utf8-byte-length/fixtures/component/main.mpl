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

fn prepare_specs(jobs :: Json, index :: Int, specs :: List<String>) -> List<String>!String do
  if index == 64 do
    return Ok(specs)
  end
  let spec = job_spec(Json.array_get(jobs, index)?)?
  prepare_specs(jobs, index + 1, List.append(specs, spec))
end

fn measure(specs :: List<String>, index :: Int, total :: Int) -> Int do
  if index == 63 do
    return total + Bytes.length(Bytes.from_utf8(List.get(specs, index)))
  end
  let accepted = Bytes.length(Bytes.from_utf8(List.get(specs, index)))
  let rejected = Bytes.length(Bytes.from_utf8(List.get(specs, index + 1)))
  measure(specs, index + 1, total + accepted + rejected)
end

fn run() -> String!String do
  let input = Json.parse(File.read(Env.get("JOBS_PARITY_INPUT", ""))?)?
  let jobs = Json.object_get(input, "jobs")?
  let specs = prepare_specs(jobs, 0, [])?
  let start = Monotonic.now_nanos()
  let sum = measure(specs, 0, 0)
  let elapsed = Monotonic.now_nanos() - start
  Ok("#{sum} #{elapsed}")
end

fn main() do
  case run() do
    Ok(result) -> println(result)
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
