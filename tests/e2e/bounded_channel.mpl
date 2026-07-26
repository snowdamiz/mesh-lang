fn exercise(channel :: Int) do
  let _ = Channel.try_send(channel, 10)
  let _ = Channel.try_send(channel, 20)
  let _ = Channel.try_send(channel, 30)
  println("${Channel.depth(channel)}")
  println("${Channel.dropped(channel)}")

  case Channel.recv(channel, 0) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end
end

fn main() do
  case Channel.bounded(2, :latest_only) do
    Ok(channel) -> exercise(channel)
    Err(error) -> println(error)
  end
end
