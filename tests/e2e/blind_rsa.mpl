fn report(label :: String, passed :: Bool) do
  if passed do
    println(label <> ":ok")
  else
    println(label <> ":failed")
  end
end

fn invalid_length_is(error :: CryptoError, expected :: Int, actual :: Int) -> Bool do
  case error do
    InvalidLength(found_expected, found_actual) -> found_expected == expected
      and found_actual == actual
    _ -> false
  end
end

fn vector_bytes(value :: String) -> Bytes!CryptoError do
  case Bytes.from_hex(value) do
    Err(_) -> Err(InternalFailure)
    Ok(value)
  end
end

fn part(value :: Bytes, start :: Int, length :: Int) -> Bytes!CryptoError do
  case Bytes.slice(value, start, length) do
    Err(_) -> Err(InternalFailure)
    Ok(value)
  end
end

fn join(first :: Bytes, second :: Bytes) -> Bytes!CryptoError do
  case Bytes.concat(first, second) do
    Err(_) -> Err(InternalFailure)
    Ok(value)
  end
end

# The input RFC 9578 signs for token type 0x0002: token_type || nonce ||
# SHA-256(TokenChallenge) || token_key_id.
fn token_input(nonce :: Bytes, challenge :: Bytes, token_key_id :: Bytes) -> Bytes!CryptoError do
  let token_type = vector_bytes("0002")?
  let with_nonce = join(token_type, nonce)?
  let with_digest = join(with_nonce, Crypto.sha256(challenge))?
  join(with_digest, token_key_id)
end

# The same bytes with the byte at `index` changed.
fn tampered(value :: Bytes, index :: Int) -> Bytes!CryptoError do
  let before = part(value, 0, index)?
  let rest = part(value, index + 1, Bytes.length(value) - index - 1)?
  let original = part(value, index, 1)?
  let replacement = if Bytes.secure_equals(original, vector_bytes("00")?) do
    vector_bytes("01")?
  else
    vector_bytes("00")?
  end
  let head = join(before, replacement)?
  join(head, rest)
end

fn issuer_storage_context() -> Bytes!CryptoError do
  vector_bytes("01000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000012000000000000000a")
end

fn discard_key(key :: consume BlindRsaSecretKey) do
  nil
end

fn check_vector(index :: Int,
  issuer :: borrow BlindRsaSecretKey,
  spki_hex :: String,
  challenge_hex :: String,
  nonce_hex :: String,
  request_hex :: String,
  response_hex :: String,
  token_hex :: String) -> Int!CryptoError do
  let label = "rfc9578-type2-#{index}"
  let spki = vector_bytes(spki_hex)?
  let public_key = Crypto.blind_rsa_public_from_spki(spki)?
  let derived = Crypto.blind_rsa_public(issuer)?
  let token_key_id = Crypto.sha256(spki)
  let input = token_input(vector_bytes(nonce_hex)?, vector_bytes(challenge_hex)?, token_key_id)?
  let request = vector_bytes(request_hex)?
  let token = vector_bytes(token_hex)?
  let request_prefix = part(request, 0, 3)?
  let expected_prefix = join(vector_bytes("0002")?, part(token_key_id, 31, 1)?)?
  let response = Crypto.blind_rsa_sign(issuer, part(request, 3, 256)?)?
  let authenticator = part(token, 98, 256)?
  let verified = Crypto.blind_rsa_verify(public_key, input, authenticator)?
  report("#{label}-key",
    Bytes.secure_equals(public_key.bytes, spki) and Bytes.secure_equals(derived.bytes, spki))
  report("#{label}-request", Bytes.secure_equals(request_prefix, expected_prefix))
  report("#{label}-input", Bytes.secure_equals(part(token, 0, 98)?, input))
  report("#{label}-sign", Bytes.secure_equals(response, vector_bytes(response_hex)?))
  report("#{label}-verify", verified)
  Ok(0)
end

fn check_round_trip(issuer :: borrow BlindRsaSecretKey,
  spki :: Bytes,
  input :: Bytes) -> Int!CryptoError do
  let public_key = Crypto.blind_rsa_public_from_spki(spki)?
  let first = Crypto.blind_rsa_blind(public_key, input)?
  let second = Crypto.blind_rsa_blind(public_key, input)?
  let first_request = first.blinded
  let second_request = second.blinded
  let first_response = Crypto.blind_rsa_sign(issuer, first_request)?
  let second_response = Crypto.blind_rsa_sign(issuer, second_request)?
  let first_signature = Crypto.blind_rsa_finalize(public_key, input, first_response, first.state)?
  let second_signature = Crypto.blind_rsa_finalize(public_key,
    input,
    second_response,
    second.state)?
  let verified = Crypto.blind_rsa_verify(public_key, input, first_signature)?
  report("blind-rsa-round-trip",
    verified and Bytes.length(first_request) == 256 and Bytes.length(first_signature) == 256)
  report("blind-rsa-unlinkable-requests",
    not Bytes.secure_equals(first_request, second_request)
      and not Bytes.secure_equals(first_signature, second_signature))
  let tampered_signature = tampered(first_signature, 17)?
  let other_input = tampered(input, 40)?
  let tampered_verified = Crypto.blind_rsa_verify(public_key, input, tampered_signature)?
  let other_verified = Crypto.blind_rsa_verify(public_key, other_input, first_signature)?
  report("blind-rsa-verify-tampered", not tampered_verified and not other_verified)
  let third = Crypto.blind_rsa_blind(public_key, input)?
  let third_request = third.blinded
  let third_response = Crypto.blind_rsa_sign(issuer, third_request)?
  case Crypto.blind_rsa_finalize(public_key, input, tampered(third_response, 100)?, third.state) do
    Err(InvalidSignature) -> report("blind-rsa-finalize-tampered", true)
    _ -> report("blind-rsa-finalize-tampered", false)
  end
  Ok(0)
end

fn check_refusals(issuer :: borrow BlindRsaSecretKey,
  spki :: Bytes,
  input :: Bytes) -> Int!CryptoError do
  let public_key = Crypto.blind_rsa_public_from_spki(spki)?
  case Crypto.blind_rsa_public_from_spki(part(spki, 1, 341)?) do
    Err(InvalidPublicKey) -> report("blind-rsa-spki-length", true)
    _ -> report("blind-rsa-spki-length", false)
  end
  case Crypto.blind_rsa_public_from_spki(tampered(spki, 340)?) do
    Err(InvalidPublicKey) -> report("blind-rsa-spki-exponent", true)
    _ -> report("blind-rsa-spki-exponent", false)
  end
  case Crypto.blind_rsa_sign(issuer, part(spki, 82, 255)?) do
    Err(error) -> report("blind-rsa-sign-length", invalid_length_is(error, 256, 255))
    _ -> report("blind-rsa-sign-length", false)
  end
  case Crypto.blind_rsa_sign(issuer, part(spki, 81, 256)?) do
    Err(error) -> report("blind-rsa-sign-range", invalid_length_is(error, 256, 256))
    _ -> report("blind-rsa-sign-range", false)
  end
  case Crypto.blind_rsa_verify(public_key, input, part(spki, 82, 255)?) do
    Err(InvalidSignature) -> report("blind-rsa-verify-length", true)
    _ -> report("blind-rsa-verify-length", false)
  end
  case Crypto.blind_rsa_from_secret(Secret.random(1217)?) do
    Err(InvalidKey) -> report("blind-rsa-import-invalid", true)
    Ok(unexpected) -> do
      discard_key(unexpected)
      report("blind-rsa-import-invalid", false)
    end
    Err(_) -> report("blind-rsa-import-invalid", false)
  end
  Ok(0)
end

fn check_storage(issuer :: borrow BlindRsaSecretKey,
  input :: Bytes,
  spki :: Bytes) -> Int!CryptoError do
  let wrapping_key = StorageKey.ephemeral()?
  let context = issuer_storage_context()?
  let blob = BlindRsaSecretKey.seal_for_storage(issuer, wrapping_key, context)?
  let restored = BlindRsaSecretKey.unseal_from_storage(blob, wrapping_key, context)?
  let public_key = Crypto.blind_rsa_public_from_spki(spki)?
  let blinded = Crypto.blind_rsa_blind(public_key, input)?
  let request = blinded.blinded
  let original = Crypto.blind_rsa_sign(issuer, request)?
  let reopened = Crypto.blind_rsa_sign(restored, request)?
  let restored_public = Crypto.blind_rsa_public(restored)?
  report("blind-rsa-storage",
    Bytes.secure_equals(original, reopened) and Bytes.secure_equals(restored_public.bytes, spki))
  Ok(0)
end

fn run() -> Int!CryptoError do
  let issuer = Crypto.blind_rsa_from_secret(Env.get_secret_hex("MESH_BLIND_RSA_ISSUER_PKCS8_HEX")?)?
  __RFC9578_VECTOR_CHECKS__
  let spki = vector_bytes("__RFC9578_SPKI_HEX__")?
  let input = token_input(vector_bytes("__RFC9578_NONCE_HEX__")?,
    vector_bytes("__RFC9578_CHALLENGE_HEX__")?,
    Crypto.sha256(spki))?
  check_round_trip(issuer, spki, input)?
  check_refusals(issuer, spki, input)?
  check_storage(issuer, input, spki)?
  Ok(0)
end

fn main() do
  case run() do
    Ok(_) -> nil
    Err(_) -> report("blind-rsa-run", false)
  end
end
