# Eight actors churn short-lived allocations at the same time: allocator scaling across cores.
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

fn outer(k :: Int, acc :: Int) -> Int do
  if k <= 0 do
    acc
  else
    let v = churn(0, 5000, 0)
    outer(k - 1, acc + v)
  end
end

fn work() -> Int do
  let total = outer(400, 0)
  0
end

actor worker() do
  receive do
    msg -> work()
  end
end

fn start(n :: Int) -> Int do
  if n <= 0 do
    0
  else
    let pid = spawn(worker)
    send(pid, 1)
    start(n - 1)
  end
end

fn main() do
  start(8)
  println("started")
end
