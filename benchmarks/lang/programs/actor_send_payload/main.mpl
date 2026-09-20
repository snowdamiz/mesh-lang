# 100k service calls that each carry two heap strings and return a heap
# string. Both builds copy the strings between caller and service (the
# baseline through the String-only path this work replaced), so this compares
# the two copies. Calls, not casts: a mailbox holds 1,024 messages and rejects
# the rest, so a caller that does not wait for replies loses messages.
service Sink do
  fn init(start :: Int) -> Int do
    start
  end

  call Add(name :: String, label :: String) :: String do |total|
    (total + String.length(name) + String.length(label), "${name}/${label}")
  end

  call Total() :: Int do |total|
    (total, total)
  end
end

fn feed(sink, i :: Int, n :: Int, seen :: Int) -> Int do
  if i >= n do
    seen
  else
    let echoed = Sink.add(sink, "job-${i}", "label-${i % 7}")
    feed(sink, i + 1, n, seen + String.length(echoed))
  end
end

fn main() do
  let sink = Sink.start(0)
  let seen = feed(sink, 0, 100000, 0)
  println("${Sink.total(sink)} ${seen}")
end
