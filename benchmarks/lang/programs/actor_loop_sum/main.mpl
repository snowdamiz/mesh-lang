# Same arithmetic loop, but inside an actor (reduction counting + yields active).
fn sum_to(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    sum_to(i + 1, n, acc + i % 7)
  end
end

actor worker() do
  receive do
    msg -> println("${sum_to(0, 300000000, 0)}")
  end
end

fn main() do
  let pid = spawn(worker)
  send(pid, 1)
end
