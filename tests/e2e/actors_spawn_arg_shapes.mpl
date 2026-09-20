# Aggregate spawn arguments: a closure with a captured heap string, a struct,
# and a string relayed through an actor that exits at once. Each must reach its
# actor intact (they are wider than one argument slot) and outlive the
# spawner's collections and its exit. Values are built at run time, so they
# live on the spawner's heap rather than in static data. The children sleep
# before reading what they were given; meanwhile the spawner churns and exits.
# Build with --opt-level 2; see actors_spawn_arg_survives_gc.mpl.
struct Job do
  name :: String
  tries :: Int
end

# One word wide: it travels in its argument slot, not in a box.
struct Batch do
  names :: List<String>
end

fn report(label :: String, value :: String) -> Int do
  Timer.sleep(400)
  println("${label}=${value}")
  0
end

actor takes_string(name :: String) do
  report("relayed", name)
end

actor takes_closure(make :: Fun(Int) -> String) do
  report("closure", make(7))
end

actor takes_struct(job :: Job) do
  report("struct", "${job.name}x${job.tries}")
end

actor takes_batch(batch :: Batch) do
  report("batch", String.join(batch.names, "+"))
end

# Hands the value it was given straight on to an actor of its own, then exits.
actor relay(name :: String) do
  spawn(takes_string, name)
end

fn churn(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let s = "garbage-${i}"
    churn(i + 1, n, acc + String.length(s))
  end
end

fn run_parent() -> Int do
  let prefix = "item-${10 + 1}"
  spawn(takes_closure, fn(n :: Int) -> "${prefix}/${n}" end)
  spawn(takes_struct, Job { name: "job-${30 + 3}", tries: 3 })
  spawn(takes_batch, Batch { names: ["batch-${40 + 4}", "batch-${50 + 5}"] })
  spawn(relay, "relay-${20 + 2}")
  churn(0, 200000, 0)
end

actor parent() do
  receive do
    msg -> run_parent()
  end
end

fn main() do
  let pid = spawn(parent)
  send(pid, 1)
end
