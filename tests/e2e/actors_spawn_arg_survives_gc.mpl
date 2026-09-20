# A heap value passed to spawn must stay valid while the spawner, which has
# dropped its own reference, keeps collecting.
#
# The string is built at run time so it lives on the spawner's heap, and is the
# same size as the churned garbage so a freed copy would be overwritten.
# Build with --opt-level 2: unoptimized code leaves the value in a dead stack
# slot, which the conservative collector would still treat as a root.
fn show(name :: String) -> Int do
  println(name)
  0
end

actor child(name :: String) do
  receive do
    msg -> show(name)
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

fn run_parent() -> Int do
  let pid = spawn(child, "payload-${41 + 1}")
  let total = churn(0, 200000, 0)
  send(pid, 1)
  total
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
