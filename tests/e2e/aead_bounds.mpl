fn fill(length :: Int) -> Bytes!CryptoError do
  case Bytes.repeat(113, length) do
    Err(_) -> Err(InternalFailure)
    Ok(value)
  end
end

fn invalid_length(value :: CryptoError, maximum :: Int, actual :: Int) -> Bool do
  case value do
    InvalidLength(found_maximum, found_actual) -> found_maximum == maximum && found_actual == actual
    _ -> false
  end
end

fn proof() -> Bool!CryptoError do
  let key = Crypto.aead_key(Secret.random(32)?)?
  for length in [65536, 65537, 65606, 131072] do
    let nonce = Crypto.random_bytes(12)?
    let plaintext = fill(length)?
    let sealed = Crypto.aead_seal(key, nonce, Bytes.from_utf8("bounded transport"), plaintext)?
    if Bytes.length(sealed) != length + 16 do
      return Ok(false)
    end
    if !Bytes.secure_equals(Crypto.aead_open(key,
        nonce,
        Bytes.from_utf8("bounded transport"),
        sealed)?,
      plaintext) do
      return Ok(false)
    end
    case Crypto.aead_open(key, nonce, Bytes.from_utf8("wrong associated data"), sealed) do
      Err(AuthenticationFailed) -> nil
      _ -> return Ok(false)
    end
  end
  for length in [65536, 65537, 131072] do
    let aad = fill(length)?
    let tampered_aad = case Bytes.concat(fill(length - 1)?, Bytes.from_utf8("r")) do
      Ok(value) -> value
      Err(_) -> return Err(InternalFailure)
    end
    let plaintext = Bytes.from_utf8("authenticated metadata boundary")
    let nonce = Crypto.random_bytes(12)?
    let extended_nonce = Crypto.random_bytes(24)?
    let sealed = Crypto.aead_seal(key, nonce, aad, plaintext)?
    let extended_sealed = Crypto.xchacha20poly1305_seal(key, extended_nonce, aad, plaintext)?
    if !Bytes.secure_equals(Crypto.aead_open(key, nonce, aad, sealed)?, plaintext) do
      return Ok(false)
    end
    if !Bytes.secure_equals(Crypto.xchacha20poly1305_open(key,
        extended_nonce,
        aad,
        extended_sealed)?,
      plaintext) do
      return Ok(false)
    end
    case Crypto.aead_open(key, nonce, tampered_aad, sealed) do
      Err(AuthenticationFailed) -> nil
      _ -> return Ok(false)
    end
    case Crypto.xchacha20poly1305_open(key, extended_nonce, tampered_aad, extended_sealed) do
      Err(AuthenticationFailed) -> nil
      _ -> return Ok(false)
    end
  end
  let nonce = Crypto.random_bytes(12)?
  let extended_nonce = Crypto.random_bytes(24)?
  case Crypto.aead_seal(key, nonce, Bytes.empty(), fill(131073)?) do
    Err(error) -> if invalid_length(error, 131072, 131073) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  case Crypto.aead_open(key, nonce, Bytes.empty(), fill(131089)?) do
    Err(error) -> if invalid_length(error, 131088, 131089) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  case Crypto.aead_seal(key, nonce, fill(131073)?, Bytes.empty()) do
    Err(error) -> if invalid_length(error, 131072, 131073) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  case Crypto.aead_open(key, nonce, fill(131073)?, fill(16)?) do
    Err(error) -> if invalid_length(error, 131072, 131073) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  case Crypto.xchacha20poly1305_seal(key, extended_nonce, fill(131073)?, Bytes.empty()) do
    Err(error) -> if invalid_length(error, 131072, 131073) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  case Crypto.xchacha20poly1305_open(key, extended_nonce, fill(131073)?, fill(16)?) do
    Err(error) -> if invalid_length(error, 131072, 131073) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  let recipient = Crypto.x25519_generate()?
  case Crypto.hpke_seal(recipient.public_key, Bytes.empty(), fill(65537)?, Bytes.empty()) do
    Err(error) -> if invalid_length(error, 65536, 65537) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  case Crypto.hpke_seal(recipient.public_key, Bytes.empty(), Bytes.empty(), fill(65537)?) do
    Err(error) -> if invalid_length(error, 65536, 65537) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  case Crypto.hpke_open(recipient.private_key, Bytes.empty(), Bytes.empty(), fill(65585)?) do
    Err(error) -> if invalid_length(error, 65584, 65585) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  let secret = Secret.random(32)?
  case Crypto.hmac_sha256(secret, fill(65537)?) do
    Err(error) -> if invalid_length(error, 65536, 65537) do
      nil
    else
      return Ok(false)
    end
    _ -> return Ok(false)
  end
  Ok(true)
end

fn main() do
  case proof() do
    Ok(true) -> println("aead-bounds:ok")
    _ -> do
      println("aead-bounds:failed")
      Process.exit(1)
    end
  end
end
