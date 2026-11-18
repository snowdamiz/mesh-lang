fn verify_status(ok :: Bool, message :: String) do
  if !ok do
    panic(message)
  end
end

actor rotate_words(parent :: Pid<Int>, a :: Int, b :: Int, remaining :: Int) do
  receive do
    delta ->
      verify_status(send(parent, a) == 0, "reply failed")
      if remaining > 1 do
        rotate_words(parent, b + delta, a, remaining - 1)
      end
  end
end

actor rotate_cleanup(parent :: Pid<Int>, n :: Int) do
  receive do
    delta ->
      send(parent, n + delta)
      if n > 1 do
        rotate_cleanup(parent, n - 1)
      end
  end
  terminate do
    0
  end
end

actor driver() do
  let fast = spawn grouped(rotate_words, self(), 1, 3, 4)
  let shared = spawn(rotate_words, self(), 1, 3, 4)
  let cleanup = spawn grouped(rotate_cleanup, self(), 2)
  for i in 1..5 do
    send(fast, i)
    let first = receive do x -> x + 0 after 3000 -> -1 end
    send(shared, i)
    let second = receive do x -> x + 0 after 3000 -> -2 end
    println(first.to_string())
    verify_status(first == second, "frame mismatch")
  end
  send(cleanup, 8)
  let reply = receive do x -> x + 0 after 3000 -> -3 end
  println(reply.to_string())
  send(cleanup, 9)
  let last = receive do x -> x + 0 after 3000 -> -4 end
  println(last.to_string())
end

fn main() do
  spawn(driver)
end
