# A heap string handed to Timer.send_after is delivered long after the sender
# dropped it and collected; the timer must carry its own copy.
fn show(name :: String) -> Int do
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
  Timer.send_after(pid, 400, "payload-${41 + 1}")
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
  # A pending timer does not keep the program alive, and once main returns
  # waiting actors are stopped. Outlast the timer.
  Timer.sleep(1200)
end
