fn exercise(channel :: Int) do
  let _ = channel |> Channel.try_send(10)
  let _ = channel |> Channel.try_send(20)
  let _ = channel |> Channel.try_send(30)
  println("${Channel.depth(channel)}")
  println("${Channel.byte_depth(channel)}")
  println("${Channel.dropped(channel)}")

  case Channel.recv(channel, 0) do
    Ok(value) -> println("${value}")
    Err(error) -> println(error)
  end
end

fn exercise_byte_bound(channel :: Int) do
  let _ = channel |> Channel.try_send(10)
  let _ = channel |> Channel.try_send(20)
  case channel |> Channel.try_send(30) do
    Ok(_) -> println("accepted")
    Err(error) -> println(error)
  end
  println("${Channel.depth(channel)}")
  println("${Channel.byte_depth(channel)}")
  println("${Channel.dropped(channel)}")
end

fn main() do
  case Channel.bounded(2, :latest_only) do
    Ok(channel) -> exercise(channel)
    Err(error) -> println(error)
  end
  case Channel.bounded_bytes(3, 16, :reject_newest) do
    Ok(channel) -> exercise_byte_bound(channel)
    Err(error) -> println(error)
  end
end
