# A heap string sent in a message must stay valid after the sender has dropped
# it and collected: the receiver gets its own copy.
#
# The string is built at run time so it lives on the sender's heap, and is the
# same size as the churned garbage so a freed copy would be overwritten.
# Build with --opt-level 2; see actors_spawn_arg_survives_gc.mpl.
fn show(name :: String) -> Int do
  Timer.sleep(400)
  println(name)
  0
end

actor receiver() do
  receive do
    name -> show(name)
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

fn run_sender() -> Int do
  let pid = spawn(receiver)
  send(pid, "payload-${41 + 1}")
  churn(0, 200000, 0)
end

actor sender() do
  receive do
    msg -> run_sender()
  end
end

fn main() do
  let pid = spawn(sender)
  send(pid, 1)
end
