# Short-lived allocation churn inside an actor: allocator fast path + GC of garbage.
# The inner loop is restarted every 5000 iterations so runtimes that leak stack
# per loop trip can still finish; see actor_alloc_long for the single long loop.
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

actor worker() do
  receive do
    msg -> println("${outer(1000, 0)}")
  end
end

fn main() do
  let pid = spawn(worker)
  send(pid, 1)
end
