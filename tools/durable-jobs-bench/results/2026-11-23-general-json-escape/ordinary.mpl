fn emit(trees :: Json, index :: Int, count :: Int) -> Unit!String do
  if index == count do return Ok(()) end
  println(Json.encode(Json.array_get(trees, index)?))
  emit(trees, index + 1, count)
end

fn encode_rows(trees :: Json, index :: Int, count :: Int, bytes :: Int) -> Int!String do
  if index == count do return Ok(bytes) end
  let encoded = Json.encode(Json.array_get(trees, index)?)
  encode_rows(trees, index + 1, count, bytes + Bytes.length(Bytes.from_utf8(encoded)))
end

fn encode_rounds(trees :: Json, rounds :: Int, count :: Int, bytes :: Int) -> Int!String do
  if rounds == 0 do return Ok(bytes) end
  let bytes = encode_rows(trees, 0, count, bytes)?
  encode_rounds(trees, rounds - 1, count, bytes)
end

fn run() -> Unit!String do
  let trees = Json.parse(File.read(Env.get("INPUT", ""))?)?
  let count = Json.array_length(trees)?
  if Env.get("TRACE", "") == "1" do return emit(trees, 0, count) end
  let rounds = Env.get_int("ROUNDS", 0)
  let start = Monotonic.now_nanos()
  let bytes = encode_rounds(trees, rounds, count, 0)?
  let elapsed = Monotonic.now_nanos() - start
  println(Json.encode(json { bytes: bytes, elapsed_ns: elapsed, count: count }))
  Ok(())
end

fn main() do
  case run() do
    Ok(_) -> ()
    Err(error) ->
      IO.eprintln(error)
      Process.exit(1)
  end
end
