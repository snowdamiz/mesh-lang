service Blocker do
  fn init(n :: Int) -> Int do
    n
  end

  call Block() :: Int do |n|
    Timer.sleep(100000)
    (n, n)
  end
end

actor caller() do
  receive do
    go -> Blocker.block(Blocker.start(1))
  end
  println("unreachable")
end

fn main() do
  let pid = spawn(caller)
  send(pid, 1)
  Timer.sleep(200)
  println("main done")
end
