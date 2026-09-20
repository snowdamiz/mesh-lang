# Spawn + message throughput: 200k short-lived actors.
actor tiny() do
  receive do
    msg -> 0
  end
end

fn spawn_batch(n :: Int) -> Int do
  if n <= 0 do
    0
  else
    let pid = spawn(tiny)
    send(pid, 1)
    spawn_batch(n - 1)
  end
end

fn spawn_many(batches :: Int) -> Int do
  if batches <= 0 do
    0
  else
    spawn_batch(1000)
    spawn_many(batches - 1)
  end
end

fn main() do
  spawn_many(200)
  println("done")
end
