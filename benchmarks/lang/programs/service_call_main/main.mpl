# 100k synchronous service calls made from `main`, which is not a coroutine and
# so polls for each reply. Guards the pacing of that wait: sleeping from the
# first miss cost about 85 microseconds a call.
service Sink do
  fn init(start :: Int) -> Int do
    start
  end

  call Add(a :: Int, b :: Int) :: Int do |total|
    (total + a + b, a + b)
  end

  call Total() :: Int do |total|
    (total, total)
  end
end

fn feed(sink, i :: Int, n :: Int, seen :: Int) -> Int do
  if i >= n do
    seen
  else
    let echoed = Sink.add(sink, i, i % 7)
    feed(sink, i + 1, n, seen + echoed)
  end
end

fn main() do
  let sink = Sink.start(0)
  let seen = feed(sink, 0, 100000, 0)
  println("${Sink.total(sink)} ${seen}")
end
