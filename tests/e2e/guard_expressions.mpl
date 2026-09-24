fn classify(n :: Int) -> String do
  case n do
    x when x * 2 > 10 -> "big"
    x when x > -1 -> "small"
    _ -> "negative"
  end
end

actor waiter(limit :: Int) do
  receive do
    n when n + 1 > limit -> println("over #{n}")
    n -> println("under #{n}")
  end
end

fn main() do
  println(classify(9))
  println(classify(2))
  println(classify(-4))
  let pid = spawn(waiter, 5)
  send(pid, 7)
  Timer.sleep(50)
end
