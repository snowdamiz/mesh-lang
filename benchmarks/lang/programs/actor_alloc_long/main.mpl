# Short-lived allocation churn inside an actor: allocator fast path + GC of garbage.
struct Point do
  x :: Int
  y :: Int
end

fn churn(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let p = Some(Point { x: i, y: i + 1 })
    let v = case p do
      Some(q) -> q.x + q.y
      None -> 0
    end
    churn(i + 1, n, acc + v % 3)
  end
end

actor worker() do
  receive do
    msg -> println("${churn(0, 5000000, 0)}")
  end
end

fn main() do
  let pid = spawn(worker)
  send(pid, 1)
end
