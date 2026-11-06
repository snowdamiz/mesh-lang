fn proof() -> String!CryptoError do
  let key = StorageKey.ephemeral()?
  let namespace = Bytes.from_utf8("public-index-test/v1")
  let value = Bytes.from_utf8("profile/v1")
  let first = StorageKey.index_tag(key, namespace, value)?
  let again = storage_key_index_tag(key, namespace, value)?
  let piped = key
    |> StorageKey.index_tag(namespace, value)?
  if !(Bytes.length(first) == 32) do
    return Ok("width")
  end
  if !(first == again && again == piped) do
    return Ok("determinism-pipe")
  end
  if !(first != StorageKey.index_tag(key, Bytes.from_utf8("public-index-test/v2"), value)?) do
    return Ok("namespace")
  end
  if !(first != StorageKey.index_tag(key, namespace, Bytes.empty())?) do
    return Ok("empty-value")
  end
  if !(Result.is_ok(StorageKey.index_tag(key,
    Bytes.repeat(1, 256)
      |> Result.unwrap_or(Bytes.empty()),
    Bytes.empty()))) do
    return Ok("namespace-max")
  end
  if !(Result.is_err(StorageKey.index_tag(key, Bytes.empty(), value))) do
    return Ok("namespace-empty")
  end
  if !(Result.is_err(StorageKey.index_tag(key,
    Bytes.repeat(1, 257)
      |> Result.unwrap_or(Bytes.empty()),
    value))) do
    return Ok("namespace-too-long")
  end
  if !(Result.is_err(StorageKey.index_tag(key,
    namespace,
    Bytes.repeat(1, 65537)
      |> Result.unwrap_or(Bytes.empty())))) do
    return Ok("value-too-long")
  end
  Secret.destroy(key)
  Ok("ok")
end

fn main() do
  case proof() do
    Ok(result) -> println("storage-index:" <> result)
    Err(SecretDestroyed) -> println("storage-index:destroyed")
    Err(InvalidKey) -> println("storage-index:invalid-key")
    Err(_) -> println("storage-index:error")
  end
end
