import File

fn bytes(value :: String) -> Bytes ! String do
  case Bytes.from_hex(value) do
    Err( _) -> Err("invalid test bytes")
    Ok( output) -> Ok(output)
  end
end

fn binary_file_proof() -> Bool ! String do
  let path = "/tmp/mesh_test_binary_ranges.bin"
  let first = bytes("00ff01") ?
  let second = bytes("0203") ?
  File.write_bytes(path, 0, first, true) ?
  File.write_bytes(path, 3, second, false) ?
  println("#{File.size(path) ?}")
  println(Bytes.to_hex(File.read_bytes(path, 2, 3) ?))
  case File.read_bytes(path, 0, 65537) do
    Err( _) -> println("bounded")
    Ok( _) -> println("unbounded")
  end
  case File.read_bytes(path, 16777216, 1) do
    Err( _) -> println("range bounded")
    Ok( _) -> println("range unbounded")
  end
  case File.write_bytes(path, 1, first, true) do
    Err( _) -> println("truncate bounded")
    Ok( _) -> println("truncate unbounded")
  end
  File.delete(path) ?
  Ok(true)
end

fn main() do
  case binary_file_proof() do
    Err( error) -> println(error)
    Ok( _) -> nil
  end
end
