# One Privacy Pass token of type 0x0002 (RFC 9578) through Crypto.BlindRsa:
# the issuer publishes a key, the client blinds a token input, the issuer
# signs without seeing it, the client unblinds it into a token, and a
# redeemer holding only the public key verifies the token and spends its
# nullifier.

fn bytes_of(hex :: String) -> Bytes!CryptoError do
  case Bytes.from_hex(hex) do
    Ok(value)
    Err(_) -> Err(InternalFailure)
  end
end

fn joined(first :: Bytes, second :: Bytes) -> Bytes!CryptoError do
  case Bytes.concat(first, second) do
    Ok(value)
    Err(_) -> Err(InternalFailure)
  end
end

fn slice(value :: Bytes, start :: Int, length :: Int) -> Bytes!CryptoError do
  case Bytes.slice(value, start, length) do
    Ok(part)
    Err(_) -> Err(InvalidLength(start + length, Bytes.length(value)))
  end
end

# token_type (0x0002) || nonce || SHA-256(TokenChallenge) || token_key_id
fn token_input(nonce :: Bytes, challenge :: Bytes, token_key_id :: Bytes) -> Bytes!CryptoError do
  let typed = joined(bytes_of("0002")?, nonce)?
  let challenged = joined(typed, Crypto.sha256(challenge))?
  joined(challenged, token_key_id)
end

# A token is its 98-byte input and a 256-byte authenticator. It is valid when
# the authenticator verifies; its nullifier is SHA-256(token input).
fn redeem(issuer_key :: BlindRsaPublicKey, token :: Bytes) -> Bytes!CryptoError do
  let input = slice(token, 0, 98)?
  let authenticator = slice(token, 98, 256)?
  if Crypto.blind_rsa_verify(issuer_key, input, authenticator)? do
    Ok(Crypto.sha256(input))
  else
    Err(InvalidSignature)
  end
end

fn round_trip() -> Int!CryptoError do
  # Issuer: a fresh key; only its SPKI is published.
  let issuer = Crypto.blind_rsa_generate()?
  let published = Crypto.blind_rsa_public(issuer)?
  # Client: accepts the key by its SPKI bytes and blinds a token input.
  let issuer_key = Crypto.blind_rsa_public_from_spki(published.bytes)?
  let nonce = Crypto.random_bytes(32)?
  let challenge = Bytes.from_utf8("morse credits token challenge")
  let input = token_input(nonce, challenge, Crypto.sha256(issuer_key.bytes))?
  let blinded = Crypto.blind_rsa_blind(issuer_key, input)?
  let request = blinded.blinded
  # Issuer: signs the blinded request; it never sees the token input.
  let response = Crypto.blind_rsa_sign(issuer, request)?
  # Client: unblinds (the state is consumed) and assembles the token.
  let authenticator = Crypto.blind_rsa_finalize(issuer_key, input, response, blinded.state)?
  let token = joined(input, authenticator)?
  println("token:#{Bytes.length(token)}")
  # Redeemer: verifies and records the nullifier.
  let nullifier = redeem(issuer_key, token)?
  println("nullifier:#{Bytes.length(nullifier)}")
  let forged = joined(slice(token, 0, 97)?, bytes_of("ff")?)?
  case redeem(issuer_key, joined(forged, authenticator)?) do
    Err(InvalidSignature) -> println("forged:refused")
    _ -> println("forged:accepted")
  end
  Ok(0)
end

fn main() do
  case round_trip() do
    Ok(_) -> nil
    Err(_) -> println("round trip failed")
  end
end
