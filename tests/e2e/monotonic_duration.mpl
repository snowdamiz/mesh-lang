fn main() do
  let started = Monotonic.now_nanos()
  Timer.sleep(5)
  let finished = Monotonic.now_nanos()

  case Monotonic.elapsed(started, finished) do
    Ok(elapsed) -> println("${elapsed > 0}")
    Err(error) -> println(error)
  end

  case Duration.seconds(2) do
    Ok(duration) -> println("${duration == 2000000000}")
    Err(error) -> println(error)
  end
end
