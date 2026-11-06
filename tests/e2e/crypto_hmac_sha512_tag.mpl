fn check() -> String!String do
  let key = case Env.get_secret_hex("MESH_HMAC_TEST_KEY") do
    Ok(value) -> value
    Err(_) -> return Err("key_import_failed")
  end
  let body = "nonce=1616492376594&ordertype=limit&pair=XBTUSD&price=37500&type=buy&volume=1.25"
  let digest = Crypto.sha256(Bytes.from_utf8("1616492376594" <> body))
  let message = Bytes.concat(Bytes.from_utf8("/0/private/AddOrder"), digest)?
  let first = case Crypto.hmac_sha512_tag(key, message) do
    Ok(tag) -> tag
    Err(_) -> return Err("hmac_failed")
  end
  let second = case Crypto.hmac_sha512_tag(key, message) do
    Ok(tag) -> tag
    Err(_) -> return Err("borrow_failed")
  end
  if !Bytes.secure_equals(first, second) || Bytes.length(first) != 64 do
    return Err("tag_mismatch")
  end
  let oversized = case Bytes.repeat(0, 65537) do
    Ok(value) -> value
    Err(_) -> return Err("fixture_bytes_failed")
  end
  case Crypto.hmac_sha512_tag(key, oversized) do
    Err(InvalidLength(65536, 65537)) -> nil
    _ -> return Err("missing_bound")
  end
  Secret.destroy(key)
  Ok(Bytes.to_base64(first))
end

fn main() do
  case check() do
    Ok(tag) -> println(tag)
    Err(error) -> println(error)
  end
end
