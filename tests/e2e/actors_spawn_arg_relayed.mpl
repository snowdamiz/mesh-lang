# A spawn argument handed on to a grandchild must stay valid after the actor in
# the middle has exited and while the original spawner keeps collecting.
#
# The string is built at run time so it lives on the spawner's heap, and is the
# same size as the churned garbage so a freed copy would be overwritten.
# Build with --opt-level 2; see actors_spawn_arg_survives_gc.mpl.
fn show(name :: String) -> Int do
  Timer.sleep(400)
  println(name)
  0
end

actor leaf(name :: String) do
  show(name)
end

# Passes on what it was given and exits at once.
actor relay(name :: String) do
  spawn(leaf, name)
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
  spawn(relay, "payload-${41 + 1}")
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
