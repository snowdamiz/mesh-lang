# BR1 public API proof. Hex placeholders are PUBLIC RFC 9578 test material.
# Private key import stays in Env.get_secret_hex; no secret is printed/exported.
fn report(label :: String, passed :: Bool) do
  if passed do
    println(label <> ":ok")
  else
    println(label <> ":failed")
  end
end

fn vector_bytes(value :: String) -> Bytes!CryptoError do
  case Bytes.from_hex(value) do
    Ok(bytes) -> Ok(bytes)
    Err(_) -> Err(InternalFailure)
  end
end

fn vector_repeat(value :: Int, length :: Int) -> Bytes!CryptoError do
  case Bytes.repeat(value, length) do
    Ok(bytes) -> Ok(bytes)
    Err(_) -> Err(InternalFailure)
  end
end

fn corrupt_last_byte(value :: Bytes) -> Bytes!CryptoError do
  let size = Bytes.length(value)
  let prefix = case Bytes.slice(value, 0, size - 1) do
    Ok(bytes) -> bytes
    Err(_) -> return Err(InternalFailure)
  end
  let last = case Bytes.get(value, size - 1) do
    Ok(byte) -> byte
    Err(_) -> return Err(InternalFailure)
  end
  let replacement = if last == 0 do
    1
  else
    0
  end
  case Bytes.concat(prefix, vector_repeat(replacement, 1)?) do
    Ok(bytes) -> Ok(bytes)
    Err(_) -> Err(InternalFailure)
  end
end

fn invalid_length(error :: CryptoError, expected :: Int, actual :: Int) -> Bool do
  case error do
    InvalidLength(wanted, found) -> wanted == expected and found == actual
    _ -> false
  end
end

fn verify_vector(key :: BlindRsaPublicKey,
  signer :: borrow BlindRsaSecretKey,
  message_hex :: String,
  signature_hex :: String,
  blinded_hex :: String,
  response_hex :: String,
  label :: String) -> Int!CryptoError do
  let message = vector_bytes(message_hex)?
  let signature = vector_bytes(signature_hex)?
  let blinded = vector_bytes(blinded_hex)?
  let expected_response = vector_bytes(response_hex)?
  let response = Crypto.blind_rsa_sign(signer, blinded)?
  report(label,
    Crypto.blind_rsa_verify(key, message, signature)?
      and Bytes.secure_equals(response, expected_response))
  Ok(0)
end

fn check_client_vectors() -> Int!CryptoError do
  let key = Crypto.blind_rsa_public_from_spki(vector_bytes("__SPKI_HEX__")?)?
  __CLIENT_VECTOR_CALLS__
  let message = Bytes.from_utf8("portable blind RSA client")
  let pending = Crypto.blind_rsa_blind(key, message)?
  let request = pending.blinded
  report("br1-client-blind", Bytes.length(request) == 256)
  let wrapping = StorageKey.ephemeral()?
  let context = vector_bytes("__STATE_CONTEXT_HEX__")?
  let blob = BlindRsaBlindingState.seal_for_storage(pending.state, wrapping, context)?
  Secret.destroy(pending.state)
  let restored = BlindRsaBlindingState.unseal_from_storage(blob, wrapping, context)?
  case Crypto.blind_rsa_finalize(key, message, vector_repeat(0, 256)?, restored) do
    Err(AuthenticationFailed) -> report("br1-client-finalize", true)
    _ -> report("br1-client-finalize", false)
  end
  Secret.destroy(wrapping)
  Ok(0)
end

fn check_vectors() -> Int!CryptoError do
  let key = Crypto.blind_rsa_public_from_spki(vector_bytes("__SPKI_HEX__")?)?
  let signer = Crypto.blind_rsa_from_secret(Env.get_secret_hex("MESH_TEST_BLIND_RSA_PKCS8_HEX")?)?
  report("br1-import-public",
    Bytes.secure_equals((Crypto.blind_rsa_public(signer)?).bytes, key.bytes))
  __VECTOR_CALLS__
  let message = vector_bytes("__MESSAGE_HEX__")?
  let signature = vector_bytes("__SIGNATURE_HEX__")?
  report("br1-message-tamper",
    not (Crypto.blind_rsa_verify(key, Bytes.from_utf8("wrong message"), signature)?))
  let tampered = vector_bytes("__TAMPERED_SIGNATURE_HEX__")?
  report("br1-signature-tamper", not (Crypto.blind_rsa_verify(key, message, tampered)?))
  case Crypto.blind_rsa_public_from_spki(Bytes.empty()) do
    Err(InvalidPublicKey) -> report("br1-spki-empty", true)
    _ -> report("br1-spki-empty", false)
  end
  case Crypto.blind_rsa_public_from_spki(vector_bytes("__BAD_SPKI_HEX__")?) do
    Err(InvalidPublicKey) -> report("br1-spki-template", true)
    _ -> report("br1-spki-template", false)
  end
  case Crypto.blind_rsa_public_from_spki(vector_bytes("__BAD_EXPONENT_SPKI_HEX__")?) do
    Err(InvalidPublicKey) -> report("br1-spki-exponent", true)
    _ -> report("br1-spki-exponent", false)
  end
  case Crypto.blind_rsa_public_from_spki(vector_bytes("__SHORT_MODULUS_SPKI_HEX__")?) do
    Err(InvalidPublicKey) -> report("br1-spki-modulus", true)
    _ -> report("br1-spki-modulus", false)
  end
  case Crypto.blind_rsa_sign(signer, vector_repeat(0, 255)?) do
    Err(error) -> report("br1-sign-length", invalid_length(error, 256, 255))
    _ -> report("br1-sign-length", false)
  end
  case Crypto.blind_rsa_sign(signer, vector_bytes("__MODULUS_HEX__")?) do
    Err(InvalidSignature) -> report("br1-sign-range", true)
    _ -> report("br1-sign-range", false)
  end
  let excessive = vector_repeat(0, 65537)?
  case Crypto.blind_rsa_blind(key, excessive) do
    Err(error) -> report("br1-message-bound", invalid_length(error, 65536, 65537))
    Ok(unexpected) -> do
      Secret.destroy(unexpected.state)
      report("br1-message-bound", false)
    end
  end
  case Crypto.blind_rsa_from_secret(Secret.random(32)?) do
    Err(InvalidKey) -> report("br1-import-malformed", true)
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("br1-import-malformed", false)
    end
    Err(_) -> report("br1-import-malformed", false)
  end
  Secret.destroy(signer)
  Ok(0)
end

fn check_roundtrip() -> Int!CryptoError do
  let signer = Crypto.blind_rsa_generate()?
  let public_key = Crypto.blind_rsa_public(signer)?
  let imported = Crypto.blind_rsa_public_from_spki(public_key.bytes)?
  let message = Bytes.from_utf8("mesh BR1 public client/server roundtrip")
  let first = Crypto.blind_rsa_blind(imported, message)?
  let second = Crypto.blind_rsa_blind(imported, message)?
  report("br1-fresh-blinding", not Bytes.secure_equals(first.blinded, second.blinded))
  Secret.destroy(second.state)
  let response = Crypto.blind_rsa_sign(signer, first.blinded)?
  let signature = Crypto.blind_rsa_finalize(imported, message, response, first.state)?
  report("br1-roundtrip", Crypto.blind_rsa_verify(imported, message, signature)?)
  let wrong_signer = Crypto.blind_rsa_generate()?
  let wrong_key = Crypto.blind_rsa_public(wrong_signer)?
  report("br1-wrong-key", not (Crypto.blind_rsa_verify(wrong_key, message, signature)?))
  Secret.destroy(wrong_signer)
  let failed = Crypto.blind_rsa_blind(imported, message)?
  case Crypto.blind_rsa_finalize(imported, message, vector_repeat(0, 256)?, failed.state) do
    Err(_) -> report("br1-finalize-tamper", true)
    _ -> report("br1-finalize-tamper", false)
  end
  let empty = Crypto.blind_rsa_blind(imported, Bytes.empty())?
  let empty_response = Crypto.blind_rsa_sign(signer, empty.blinded)?
  let empty_signature = Crypto.blind_rsa_finalize(imported,
    Bytes.empty(),
    empty_response,
    empty.state)?
  report("br1-empty-message", Crypto.blind_rsa_verify(imported, Bytes.empty(), empty_signature)?)
  let maximum = vector_repeat(65, 65536)?
  let maximum_blind = Crypto.blind_rsa_blind(imported, maximum)?
  let maximum_response = Crypto.blind_rsa_sign(signer, maximum_blind.blinded)?
  let maximum_signature = Crypto.blind_rsa_finalize(imported,
    maximum,
    maximum_response,
    maximum_blind.state)?
  report("br1-maximum-message", Crypto.blind_rsa_verify(imported, maximum, maximum_signature)?)
  println("BR1_SPKI=" <> Bytes.to_hex(public_key.bytes))
  println("BR1_MESSAGE=" <> Bytes.to_hex(message))
  println("BR1_SIGNATURE=" <> Bytes.to_hex(signature))
  Secret.destroy(signer)
  Ok(0)
end

fn check_storage() -> Int!CryptoError do
  let signer = Crypto.blind_rsa_from_secret(Env.get_secret_hex("MESH_TEST_BLIND_RSA_PKCS8_HEX")?)?
  let key = Crypto.blind_rsa_public(signer)?
  let wrapping = StorageKey.ephemeral()?
  let key_context = vector_bytes("__KEY_CONTEXT_HEX__")?
  let state_context = vector_bytes("__STATE_CONTEXT_HEX__")?
  let other_context = vector_bytes("__OTHER_CONTEXT_HEX__")?
  let key_blob = BlindRsaSecretKey.seal_for_storage(signer, wrapping, key_context)?
  Secret.destroy(signer)
  let restored_key = BlindRsaSecretKey.unseal_from_storage(key_blob, wrapping, key_context)?
  report("br1-key-storage",
    Bytes.secure_equals((Crypto.blind_rsa_public(restored_key)?).bytes, key.bytes))
  let message = Bytes.from_utf8("persist pending issuance before sending payment")
  let pending = Crypto.blind_rsa_blind(key, message)?
  let request = pending.blinded
  let state_blob = BlindRsaBlindingState.seal_for_storage(pending.state, wrapping, state_context)?
  Secret.destroy(pending.state)
  let restored_state = BlindRsaBlindingState.unseal_from_storage(state_blob,
    wrapping,
    state_context)?
  let response = Crypto.blind_rsa_sign(restored_key, request)?
  let signature = Crypto.blind_rsa_finalize(key, message, response, restored_state)?
  report("br1-state-storage", Crypto.blind_rsa_verify(key, message, signature)?)
  case BlindRsaBlindingState.unseal_from_storage(state_blob, wrapping, other_context) do
    Err(AuthenticationFailed) -> report("br1-storage-context", true)
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("br1-storage-context", false)
    end
    _ -> report("br1-storage-context", false)
  end
  case BlindRsaSecretKey.unseal_from_storage(state_blob, wrapping, key_context) do
    Err(_) -> report("br1-storage-type", true)
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("br1-storage-type", false)
    end
  end
  let tampered = corrupt_last_byte(state_blob)?
  case BlindRsaBlindingState.unseal_from_storage(tampered, wrapping, state_context) do
    Err(AuthenticationFailed) -> report("br1-storage-tamper", true)
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("br1-storage-tamper", false)
    end
    Err(_) -> report("br1-storage-tamper", false)
  end
  let rebound = BlindRsaBlindingState.unseal_from_storage(state_blob, wrapping, state_context)?
  case Crypto.blind_rsa_finalize(key, Bytes.from_utf8("another issuance"), response, rebound) do
    Err(_) -> report("br1-state-message-binding", true)
    _ -> report("br1-state-message-binding", false)
  end
  let rebound_key = BlindRsaBlindingState.unseal_from_storage(state_blob, wrapping, state_context)?
  let another_issuer = Crypto.blind_rsa_generate()?
  let another_public = Crypto.blind_rsa_public(another_issuer)?
  case Crypto.blind_rsa_finalize(another_public, message, response, rebound_key) do
    Err(AuthenticationFailed) -> report("br1-state-key-binding", true)
    _ -> report("br1-state-key-binding", false)
  end
  Secret.destroy(another_issuer)
  Secret.destroy(restored_key)
  Secret.destroy(wrapping)
  Ok(0)
end

fn main() do
  case check_vectors() do
    Ok(_) -> nil
    Err(_) -> report("br1-vector-error", false)
  end
  case check_roundtrip() do
    Ok(_) -> nil
    Err(_) -> report("br1-roundtrip-error", false)
  end
  case check_storage() do
    Ok(_) -> nil
    Err(_) -> report("br1-storage-error", false)
  end
end
