from Solana.Read import Pubkey
from Solana.Address import ProgramDerivedAddress, create_program_address, find_program_address

fn require(condition :: Bool) do
  if !condition do
    panic("Solana SDK oracle mismatch")
  end
end

fn bytes(object :: Json, field :: String) -> Bytes!String do
  Bytes.from_hex(Json.as_string(Json.object_get(object, field)?)?)
end

fn proof() -> Bool!String do
  let curve_check = Crypto.ed25519_is_on_curve
  case curve_check(Bytes.from_hex(String.repeat("00", 32))?) do
    Ok(true) -> nil
    _ -> return Err("first-class curve check failed")
  end
  case crypto_ed25519_is_on_curve(Bytes.from_hex(String.repeat("00", 32))?) do
    Ok(true) -> nil
    _ -> return Err("flat curve check failed")
  end
  let fixture = Json.parse(File.read(Env.get("MESH_SOLANA_PDA_FIXTURE", ""))?)?
  let curves = Json.object_get(fixture, "curves")?
  for index in 0..Json.array_length(curves)? do
    let item = Json.array_get(curves, index)?
    let expected = Json.as_bool(Json.object_get(item, "on_curve")?)?
    let result = case Crypto.ed25519_is_on_curve(bytes(item, "bytes")?) do
      Err(_) -> return Err("curve membership failed")
      Ok(value) -> value
    end
    require(result == expected)
  end
  let cases = Json.object_get(fixture, "addresses")?
  for index in 0..Json.array_length(cases)? do
    let item = Json.array_get(cases, index)?
    let program = Pubkey { bytes: bytes(item, "program")? }
    let raw_seeds = Json.object_get(item, "seeds")?
    let seeds = for n in 0..Json.array_length(raw_seeds)? do
      Bytes.from_hex(Json.as_string(Json.array_get(raw_seeds, n)?)?)?
    end
    let found = find_program_address(program, seeds)?
    require(Bytes.secure_equals(found.address.bytes, bytes(item, "address")?))
    require(found.bump == Json.as_int(Json.object_get(item, "bump")?)?)
    let bump = case Bytes.from_list([found.bump]) do
      Err(_) -> return Err("invalid bump")
      Ok(value) -> value
    end
    require(Bytes.secure_equals(create_program_address(program, List.append(seeds, bump))?.bytes,
      found.address.bytes))
    for higher in (found.bump + 1)..256 do
      let bump = case Bytes.from_list([higher]) do
        Err(_) -> return Err("invalid higher bump")
        Ok(value) -> value
      end
      case create_program_address(program, List.append(seeds, bump)) do
        Ok(_) -> require(false)
        Err(_) -> nil
      end
    end
  end
  let program = Pubkey { bytes: Bytes.from_hex(String.repeat("00", 32))? }
  for invalid in [
    for _ in 0..16 do
      Bytes.empty()
    end,
    [Bytes.from_utf8(String.repeat("a", 33))]
  ] do
    case find_program_address(program, invalid) do
      Ok(_) -> require(false)
      Err(_) -> nil
    end
  end
  case create_program_address(program,
    for _ in 0..17 do
      Bytes.empty()
    end) do
    Ok(_) -> require(false)
    Err(_) -> nil
  end
  case find_program_address(Pubkey { bytes: Bytes.empty() }, []) do
    Ok(_) -> require(false)
    Err(_) -> nil
  end
  for invalid in [
    Bytes.empty(),
    Bytes.from_utf8(String.repeat("a", 31)),
    Bytes.from_utf8(String.repeat("a", 33))
  ] do
    case Crypto.ed25519_is_on_curve(invalid) do
      Err(InvalidPublicKey) -> nil
      _ -> require(false)
    end
  end
  Ok(true)
end

fn main() do
  case proof() do
    Err(error) -> panic(error)
    Ok(_) -> println("solana-pda:ok")
  end
end
