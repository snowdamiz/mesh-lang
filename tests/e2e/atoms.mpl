fn check(a) do
  case a do
    :ok -> "matched ok"
    :error -> "matched error"
    _ -> "other"
  end
end

actor waiter() do
  receive do
    :stop -> println("stopped by atom")
    _ -> println("something else")
  end
end

fn main() do
  println(check(:ok))
  println(check(:error))
  println(check(:nope))
  let a = :ready
  println("equal: #{a == :ready}, different: #{a != :ready}")
  println("shown: #{a}")
  let m = Map.put(%{}, :key, 1)
  println("map: #{Map.get(m, :key)}")
  let pid = spawn(waiter)
  send(pid, :stop)
  Timer.sleep(50)
end
