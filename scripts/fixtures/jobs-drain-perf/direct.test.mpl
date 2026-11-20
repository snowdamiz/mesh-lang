fn probe(context :: String) -> String do
  if String.contains(context, "direct-probe") do
    """{"action":"complete"}"""
  else
    """{"action":"discard","reason":"missing probe input"}"""
  end
end

fn invoke(input :: String, remaining :: Int) -> Unit!String do
  if remaining == 0 do
    return Ok(())
  end
  let outcome = Test.perform_job(input, 1000, probe)?
  if outcome != """{"action":"complete"}""" do
    return Err("unexpected direct handler outcome: " <> outcome)
  end
  invoke(input, remaining - 1)
end

fn measure_direct(input :: String, samples :: Int) -> Unit!String do
  if samples == 0 do
    return Ok(())
  end
  let iterations = 1000
  let started = Monotonic.now_nanos()
  let _ = invoke(input, iterations)?
  let elapsed = Monotonic.now_nanos() - started
  println("J5_DIRECT " <> Json.encode(json { elapsed_ns: elapsed, completed: iterations }))
  measure_direct(input, samples - 1)
end

fn run_direct() -> Unit!String do
  let input = """{"worker":"proof","args":{"label":"direct-probe"}}"""
  let _ = invoke(input, 1000)?
  measure_direct(input, 7)
end

test("measure the compiled database-free direct handler helper") do
  case run_direct() do
    Ok(_) -> assert(true)
    Err(error) -> panic(error)
  end
end
