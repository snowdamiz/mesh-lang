# Large live set + garbage churn inside an actor: GC mark/sweep cost.
fn build(i :: Int, n :: Int, acc :: List<String>) -> List<String> do
  if i >= n do
    acc
  else
    build(i + 1, n, List.append(acc, "item-${i}"))
  end
end

fn churn(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let s = "garbage-${i}"
    churn(i + 1, n, acc + String.length(s))
  end
end

fn work() -> Int do
  let live = build(0, 3000, List.new())
  let total = churn(0, 300000, 0)
  println("${List.length(live)} ${total}")
  0
end

actor worker() do
  receive do
    msg -> work()
  end
end

fn main() do
  let pid = spawn(worker)
  send(pid, 1)
end
