type Message do
  Note(String)
  Ready(Int)
end

actor reader() do
  send(self(), Note("kept for later"))
  send(self(), Ready(42))
  receive matching do
    Ready(value) when value > 0 -> println("ready ${value}")
    after 1000 -> println("no ready message")
  end
  receive do
    Note(text) -> println(text)
    Ready(value) -> println("another ${value}")
  end
end

fn main() do
  let reader :: Pid<Message> = spawn(reader)
end
