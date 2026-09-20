# Latency of the first message after a quiet spell: 300 service calls from
# `main`, each after the system has been idle for 5 ms, and the time spent
# inside the calls. `main` is not a scheduler worker, so the service's worker
# has to be woken from another thread: it used to sleep for up to a millisecond
# between looks at its actors, and nothing told it one had become ready.
service Echo do
  fn init(start :: Int) -> Int do
    start
  end

  call Ping(n :: Int) :: Int do |count|
    (count + 1, n)
  end
end

fn round(echo, i :: Int, n :: Int, spent :: Int) -> Int do
  if i >= n do
    spent
  else
    Timer.sleep(5)
    let before = Monotonic.now_nanos()
    let _ = Echo.ping(echo, i)
    round(echo, i + 1, n, spent + Monotonic.now_nanos() - before)
  end
end

fn main() do
  let spent = round(Echo.start(0), 0, 300, 0)
  println("calls done")
  IO.eprintln("mean call latency after idle: ${spent / 300 / 1000} us")
end
