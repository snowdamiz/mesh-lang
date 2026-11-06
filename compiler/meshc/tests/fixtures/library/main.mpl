@export("mesh_fixture_echo")
pub fn echo(request :: Bytes) -> Bytes ! String do
  Host.secure_store_get(request)
end

fn storage_context() -> Bytes ! String do
  Bytes.from_hex("0111111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222233333333333333333333333333333333333333333333333333333333333333334444444444444444444444444444444444444444444444444444444444444444000e0000000000000001")
end

@export("mesh_fixture_storage_roundtrip")
pub fn storage_roundtrip(request :: Bytes) -> Bytes ! String do
  let key = case StorageKey.platform() do
    Err(_) -> Err("platform storage key failed")
    Ok(value) -> Ok(value)
  end ?
  let sealed = case StorageKey.seal_bytes(request, key, storage_context() ?) do
    Err(_) -> Err("storage seal failed")
    Ok(value) -> Ok(value)
  end ?
  case StorageKey.unseal_bytes(sealed, key, storage_context() ?) do
    Err(_) -> Err("storage open failed")
    Ok(value) -> Ok(value)
  end
end
@export("mesh_fixture_large", max_input: 16_782_000, max_output: 16_782_000)
pub fn large(request :: Bytes) -> Bytes ! String do
  Ok(request)
end

@export("mesh_fixture_input_only", max_input: 16_782_000)
pub fn input_only(request :: Bytes) -> Bytes ! String do
  Ok(request)
end

@export("mesh_fixture_small_output", max_output: 2)
pub fn small_output(request :: Bytes) -> Bytes ! String do
  Ok(request)
end
