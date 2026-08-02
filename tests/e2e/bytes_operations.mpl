fn print_bool(value :: Bool) do
  println("#{value}")
end

fn inspect_raw(raw :: Bytes) do
  println("#{Bytes.length(raw)}")
  case raw |> Bytes.get(0) do
    Ok(value) -> println("#{value}")
    Err(error) -> println(error)
  end
  case raw |> Bytes.get(3) do
    Ok(value) -> println("#{value}")
    Err(error) -> println(error)
  end
  case raw |> Bytes.slice(1, 2) do
    Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
    Err(error) -> println(error)
  end
  case raw |> Bytes.to_utf8() do
    Ok(value) -> println(value)
    Err(error) -> println(error)
  end
  raw |> Bytes.to_base64() |> println()
end

fn inspect_hello(bytes :: Bytes) do
  case bytes |> Bytes.to_utf8() do
    Ok(value) -> println(value)
    Err(error) -> println(error)
  end
  bytes |> Bytes.secure_equals(Bytes.from_utf8("Hello World")) |> print_bool()
end

fn roundtrip_base58(bytes :: Bytes) do
  case bytes |> Bytes.to_base58() |> Bytes.from_base58() do
    Ok(decoded) -> inspect_hello(decoded)
    Err(error) -> println(error)
  end
end

fn main() do
  println("#{Bytes.length(Bytes.empty())}")

  case Bytes.from_hex("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000") do
    Ok(bytes) -> Bytes.empty() |> Bytes.secure_equals(bytes) |> print_bool()
    Err(error) -> println(error)
  end

  case "ff0041" |> Bytes.from_hex() do
    Err(error) -> println(error)
    Ok(raw) -> inspect_raw(raw)
  end

  case Bytes.from_utf8("World") |2> Bytes.concat(Bytes.from_utf8("Hello ")) do
    Ok(bytes) -> roundtrip_base58(bytes)
    Err(error) -> println(error)
  end

  case "78563412" |> Bytes.from_hex() do
    Ok(bytes) -> case bytes |> Bytes.read_uint_le(0, 4) do
        Ok(value) -> println(value)
        Err(error) -> println(error)
      end
    Err(error) -> println(error)
  end

  case "18446744073709551615" |> Bytes.write_uint_le(8) do
    Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
    Err(error) -> println(error)
  end
end
