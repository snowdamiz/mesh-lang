fn print_bool(value :: Bool) do
  println("#{value}")
end

fn inspect_list_bytes(bytes :: Bytes) do
  bytes |> Bytes.to_hex() |> println()
  let values = Bytes.to_list(bytes)
  println("#{List.length(values)}")
  println("#{List.get(values, 3)}")
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

  case Bytes.from_list([0, 18, 52, 255]) do
    Ok(bytes) -> inspect_list_bytes(bytes)
    Err(_) -> println("unexpected-list-error")
  end
  case Bytes.from_list([256]) do
    Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
    Err(_) -> println("invalid-byte")
  end
  case Bytes.repeat(171, 3) do
    Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
    Err(_) -> println("unexpected-repeat-error")
  end
  case Bytes.repeat(-1, 1) do
    Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
    Err(_) -> println("invalid-byte")
  end

  case Bytes.from_hex("123456789abcdef0") do
    Ok(bytes) -> inspect_endian_reads(bytes)
    Err(error) -> println(error)
  end

  case Bytes.write_u16_be(4660) do
    Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
    Err(_) -> println("unexpected-write-error")
  end
  case U64.parse("305419896") do
    Ok(value) -> case Bytes.write_u32_be(value) do
        Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
        Err(_) -> println("unexpected-write-error")
      end
    Err(error) -> println(error)
  end
  case U64.parse("18446744073709551615") do
    Ok(value) -> case Bytes.write_u64_be(value) do
        Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
        Err(_) -> println("unexpected-write-error")
      end
    Err(error) -> println(error)
  end
  case Bytes.write_u16_be(65536) do
    Ok(bytes) -> bytes |> Bytes.to_hex() |> println()
    Err(_) -> println("write-error")
  end
end

fn inspect_endian_reads(bytes :: Bytes) do
  case Bytes.read_u16_be(bytes, 0) do
    Ok(value) -> println("#{value}")
    Err(_) -> println("unexpected-read-error")
  end
  case Bytes.read_u16_le(bytes, 0) do
    Ok(value) -> println("#{value}")
    Err(_) -> println("unexpected-read-error")
  end
  case Bytes.read_u32_be(bytes, 0) do
    Ok(value) -> value |> U64.to_string() |> println()
    Err(_) -> println("unexpected-read-error")
  end
  case Bytes.read_u32_le(bytes, 0) do
    Ok(value) -> value |> U64.to_string() |> println()
    Err(_) -> println("unexpected-read-error")
  end
  case Bytes.read_u64_be(bytes, 0) do
    Ok(value) -> value |> U64.to_string() |> println()
    Err(_) -> println("unexpected-read-error")
  end
  case Bytes.read_u64_le(bytes, 0) do
    Ok(value) -> value |> U64.to_string() |> println()
    Err(_) -> println("unexpected-read-error")
  end
  case Bytes.read_u64_be(bytes, 1) do
    Ok(value) -> value |> U64.to_string() |> println()
    Err(_) -> println("read-error")
  end
end
