# Values the OpenSSL differential test checks: a Mesh-generated key's SPKI
# with a finalized signature, and raw signing with an OpenSSL-generated key.

fn bytes_of(hex :: String) -> Bytes!CryptoError do
  case Bytes.from_hex(hex) do
    Ok(value)
    Err(_) -> Err(InternalFailure)
  end
end

fn hex_line(label :: String, value :: Bytes) do
  println(label <> ":" <> Bytes.to_hex(value))
end

fn run() -> Int!CryptoError do
  let issuer = Crypto.blind_rsa_generate()?
  let public_key = Crypto.blind_rsa_public(issuer)?
  let message = Bytes.from_utf8("mesh blind rsa differential")
  let blinded = Crypto.blind_rsa_blind(public_key, message)?
  let request = blinded.blinded
  let response = Crypto.blind_rsa_sign(issuer, request)?
  let signature = Crypto.blind_rsa_finalize(public_key, message, response, blinded.state)?
  hex_line("spki", public_key.bytes)
  hex_line("message", message)
  hex_line("signature", signature)
  let imported = Crypto.blind_rsa_from_secret(Env.get_secret_hex("MESH_BLIND_RSA_OPENSSL_PKCS8_HEX")?)?
  let imported_public = Crypto.blind_rsa_public(imported)?
  hex_line("imported-spki", imported_public.bytes)
  hex_line("raw-signature", Crypto.blind_rsa_sign(imported, bytes_of("__RAW_INPUT_HEX__")?)?)
  Ok(0)
end

fn main() do
  case run() do
    Ok(_) -> nil
    Err(_) -> println("differential:failed")
  end
end
