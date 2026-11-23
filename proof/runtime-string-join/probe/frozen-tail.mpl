fn prepare(jobs :: Json, index :: Int, count :: Int, batches :: List<List<String>>) -> List<List<String>>!String do
  if index >= count do
    return Ok(batches)
  end
  let stop = if count - index > 128 do index + 128 else count end
  let specs = collect_specs(jobs, index, stop, [], 2)?
  prepare(jobs, index + List.length(specs), count, List.append(batches, specs))
end

fn audit(batches :: List<List<String>>, i :: Int, total :: Int) -> Int do
  if i == List.length(batches) do
    return total
  end
  let specs = List.get(batches, i)
  let batch = BATCH_EXPRESSION
  println("#{List.length(specs)} #{Bytes.length(Bytes.from_utf8(batch))} #{Bytes.to_hex(Crypto.sha256(Bytes.from_utf8(batch)))}")
  audit(batches, i + 1, total + List.length(specs))
end

fn measure(batches :: List<List<String>>, rounds :: Int, index :: Int, total :: Int, acc :: Int) -> Int do
  if rounds == 0 do
    return acc
  end
  if index == total do
    return measure(batches, rounds - 1, 0, total, acc)
  end
  let specs = List.get(batches, index)
  let batch = BATCH_EXPRESSION
  measure(batches, rounds, index + 1, total, checksum(batch, acc))
end

fn run() -> Int!String do
  let input = Json.parse(File.read(Env.get("PROBE_INPUT", ""))?)?
  let jobs = Json.object_get(input, "jobs")?
  let count = Json.array_length(jobs)?
  let batches = prepare(jobs, 0, count, [])?
  if audit(batches, 0, 0) != count do
    return Err("batch did not account for every input")
  end
  let rounds = Env.get_int("PROBE_ROUNDS", 1)
  let total = List.length(batches)
  let start = cpu_ns()
  let sum = measure(batches, rounds, 0, total, 0)
  let elapsed = cpu_ns() - start
  println("#{sum} #{elapsed}")
  Ok(0)
end

actor produce() do
  case run() do
    Ok(_) -> ()
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end

fn main() do
  let _ = spawn(produce)
end
