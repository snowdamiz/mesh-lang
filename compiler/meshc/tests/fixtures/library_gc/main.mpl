type Payload do
  Kept(Bytes, Bool)
  Empty(Int)
end

type IndependentPayload do
  Left(Bytes, Bool)
  Right(Bool, Bytes)
end

struct IndependentEnvelope do
  leading :: Bool
  selected :: IndependentPayload
  trailing :: Int
end

fn make_envelope(seed :: Int) -> IndependentEnvelope do
  let bytes = Bytes.from_utf8("payload-#{seed}|")
  let selected = if seed % 2 == 0 do
    Left(bytes, true)
  else
    Right(false, bytes)
  end
  IndependentEnvelope { leading: seed % 2 == 0, selected: selected, trailing: seed }
end

fn make_capture(seed :: Int) do
  let bytes = Bytes.from_utf8("capture-#{seed}|")
  fn(extra :: Bytes) -> Bytes.concat(bytes, extra) end
end

fn map_value(index :: Int) -> Bytes do
  if pressure(4) != 4 do
    panic("map pressure")
  end
  Bytes.from_utf8("map-#{index}|")
end

@export("mesh_gc_independent_roots")
pub fn independent_roots(input :: Bytes) -> Bytes!String do
  let seed = Result.unwrap_or(Bytes.get(input, 0), 0)
  let envelope = make_envelope(seed)
  let capture = make_capture(seed)
  let mapped = List.map([0, 1], fn(index) -> map_value(index) end)
  let flat = List.flat_map([0, 1],
    fn(index) do
      if pressure(4) != 4 do
        panic("flat pressure")
      end
      let values = [Bytes.from_utf8("flat-#{index}|")]
      values
    end)
  if pressure(1024) != 1024 do
    return Err("independent pressure")
  end
  if envelope.leading != (seed % 2 == 0) || envelope.trailing != seed do
    return Err("independent scalar")
  end
  let payload = case envelope.selected do
    Left(bytes, true) -> bytes
    Right(false, bytes) -> bytes
    _ -> return Err("independent variant")
  end
  let bytes = case Bytes.concat_many([
      payload,
      capture(Bytes.empty())?,
      List.get(mapped, 0),
      List.get(mapped, 1),
      List.get(flat, 0),
      List.get(flat, 1)
    ],
    256) do
    Ok(value) -> value
    Err(_) -> return Err("independent join")
  end
  Host.secure_store_get(bytes)
end

struct Envelope do
  flag :: Bool
  payload :: Payload
end

fn pressure(count :: Int) -> Int do
  let lengths = for index in 0..count do
    let bytes = case Bytes.repeat(index % 251, 65536) do
      Ok(value) -> value
      Err(_) -> panic("pressure allocation")
    end
    Bytes.length(bytes)
  end
  List.length(lengths)
end

@export("mesh_gc_fixture")
pub fn fixture(input :: Bytes) -> Bytes!String do
  let envelope = Envelope { flag: true, payload: Kept(input, true) }
  let forward = fn(value) -> Bytes.concat(input, value) end
  let count = if Result.unwrap_or(Bytes.get(input, 0), 0) == 0 do
    8
  else
    1024
  end
  if pressure(count) != count do
    return Err("pressure count")
  end
  let mapped = List.flat_map([1, 2, 3],
    fn(index) do
      if pressure(8) != 8 do
        panic("callback count")
      end
      let values = [Bytes.from_utf8("#{index}")]
      values
    end)
  let kept = case envelope.payload do
    Kept(value, true) -> value
    _ -> return Err("aggregate root")
  end
  if !envelope.flag do
    return Err("flag root")
  end
  let returned = Host.secure_store_get(kept)?
  let joined = case Bytes.concat_many(mapped, 3) do
    Ok(value) -> value
    Err(_) -> return Err("join")
  end
  forward(Bytes.concat(returned, joined)?)
end
