fn main() do
  let a = Bytes.from_utf8("ab")
  let b = case Bytes.from_hex("00ff") do Ok(value) -> value Err(_) -> panic("fixture") end
  let parts = [a, Bytes.empty(), b, a]
  case Bytes.concat_many(parts, 6) do
    Ok(value) -> println(Bytes.to_hex(value))
    Err(_) -> panic("join")
  end
  println(Bytes.to_hex(a))
  println("#{List.length(parts)}")
  let extended = List.append(parts, b)
  println("#{List.length(parts)}/#{List.length(extended)}")
  case Bytes.concat_many([], 0) do
    Ok(value) -> println("empty/#{Bytes.length(value)}")
    Err(_) -> panic("empty")
  end
  case Bytes.concat_many(parts, 5) do Ok(_) -> panic("bound") Err(_) -> println("too-large") end
  case Bytes.concat_many([], -1) do Ok(_) -> panic("negative") Err(_) -> println("invalid-limit") end
  let many = for index in 0..8192 do
    case Bytes.write_u16_be(index) do Ok(value) -> value Err(_) -> panic("word") end
  end
  case Bytes.concat_many(many, 16384) do
    Ok(value) -> do
      println("#{Bytes.length(value)}")
      println(Bytes.to_hex(Result.unwrap_or(Bytes.slice(value, 0, 4), Bytes.empty())))
      println(Bytes.to_hex(Result.unwrap_or(Bytes.slice(value, 16380, 4), Bytes.empty())))
    end
    Err(_) -> panic("many")
  end
end
