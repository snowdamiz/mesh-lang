fn require(value :: Bool) do
  if !value do
    panic("hash preimage assertion failed")
  end
end

fn context() -> Bytes do
  case Bytes.from_hex("01"
    <> String.repeat("31", 32)
    <> String.repeat("00", 48)
    <> String.repeat("52", 32)
    <> "00140000000000000001") do
    Ok(value) -> value
    Err(_) -> Bytes.empty()
  end
end

fn exercise() -> Bool!CryptoError do
  let key = StorageKey.ephemeral()?
  let value = HashPreimage.generate()?
  let commitment = HashPreimage.commitment(value)?
  let blob = HashPreimage.seal_for_storage(value, key, context())?
  let second_blob = HashPreimage.seal_for_storage(value, key, context())?
  require(!Bytes.secure_equals(blob, second_blob))
  let disclosed = HashPreimage.reveal(value)?
  require(Bytes.length(disclosed) == 32)
  require(Bytes.secure_equals(commitment, Crypto.sha256(disclosed)))
  let restored = HashPreimage.unseal_from_storage(blob, key, context())?
  require(Bytes.secure_equals(commitment, HashPreimage.commitment(restored)?))
  require(Bytes.secure_equals(disclosed, HashPreimage.reveal(restored)?))
  let dropped = HashPreimage.generate()?
  Secret.destroy(dropped)
  Ok(true)
end

fn main() do
  case exercise() do
    Ok(value) -> require(value)
    Err(_) -> panic("hash preimage exercise failed")
  end
  println("hash preimage verified")
end
