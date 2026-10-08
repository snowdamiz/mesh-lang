fn storage_context() -> Bytes!String do
  Bytes.from_hex("0111111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222233333333333333333333333333333333333333333333333333333333333333334444444444444444444444444444444444444444444444444444444444444444000e0000000000000001")
end

fn louder(bytes :: Bytes) -> Bytes do
  case Bytes.to_utf8(bytes) do
    Ok(text) -> Bytes.from_utf8(String.to_upper(text))
    Err(_) -> bytes
  end
end

# The host hands the core content and gets content back: both cross the
# boundary only because the export is marked `@display`.
@display
@export("mesh_plaintext_fixture_shout")
pub fn shout(request :: Plaintext<Bytes>) -> Plaintext<Bytes>!String do
  let key = case StorageKey.ephemeral() do
    Err(_) -> Err("storage key failed")
    Ok(value)
  end?
  let loud = Plaintext.map(request, louder)
  let sealed = case Plaintext.seal_for_storage(loud, key, storage_context()?) do
    Err(_) -> Err("storage seal failed")
    Ok(value)
  end?
  case Plaintext.unseal_from_storage(sealed, key, storage_context()?) do
    Err(_) -> Err("storage open failed")
    Ok(value)
  end
end
