fn report(label :: String, passed :: Bool) do
  if passed do
    println(label <> ":ok")
  else
    println(label <> ":failed")
  end
end

fn invalid_length_is(error :: CryptoError, expected :: Int, actual :: Int) -> Bool do
  case error do
    InvalidLength(found_expected, found_actual) -> found_expected == expected and found_actual == actual
    _ -> false
  end
end

fn check_large_hashes() do
  case Bytes.repeat(165, 65537) do
    Ok(input) -> do
      let sha256 = Crypto.sha256(input)
      let sha512 = Crypto.sha512(input)
      report(
        "hash-large-input",
        Bytes.length(sha256) == 32 and
          Bytes.length(sha512) == 64 and
          Bytes.to_hex(sha256) == Crypto.sha256_hex(input) and
          Bytes.to_hex(sha512) == Crypto.sha512_hex(input)
      )
    end
    Err(_) -> report("hash-large-input", false)
  end
end

fn check_random_bytes() do
  case Crypto.random_bytes(0) do
    Ok(bytes) -> report("random-zero", Bytes.length(bytes) == 0)
    Err(_) -> report("random-zero", false)
  end
  case Crypto.random_bytes(32) do
    Ok(bytes) -> report("random-length", Bytes.length(bytes) == 32)
    Err(_) -> report("random-length", false)
  end
  case Crypto.random_bytes(65536) do
    Ok(bytes) -> report("random-max", Bytes.length(bytes) == 65536)
    Err(_) -> report("random-max", false)
  end
  case Crypto.random_bytes(-1) do
    Err(error) -> report("random-negative-length", invalid_length_is(error, 65536, -1))
    _ -> report("random-negative-length", false)
  end
  case Crypto.random_bytes(65537) do
    Err(error) -> report("random-excessive-length", invalid_length_is(error, 65536, 65537))
    _ -> report("random-excessive-length", false)
  end
end

fn check_secret_zero_length() do
  case Secret.random(0) do
    Err(error) -> report("secret-zero-length", invalid_length_is(error, 65536, 0))
    Ok(secret) -> do
      Secret.destroy(secret)
      report("secret-zero-length", false)
    end
  end
end

fn check_hmac_hkdf() -> Int ! CryptoError do
  let key = Secret.random(32) ?
  let message = Bytes.from_utf8("public Crypto V2 lifecycle")
  let salt = Bytes.from_utf8("mesh-msg/v1/test-salt")
  let info = Bytes.from_utf8("mesh-msg/v1/test-info")
  let tag = Crypto.hmac_sha256(key, message) ?
  let derived = Crypto.hkdf_sha256(tag, salt, info, 32) ?
  let second_tag = Crypto.hmac_sha256(key, message) ?

  Secret.destroy(second_tag)
  Secret.destroy(derived)
  Secret.destroy(tag)
  Secret.destroy(key)
  report("hmac-hkdf-lifecycle", true)

  let bounds_key = Secret.random(32) ?
  case Bytes.repeat(0, 65537) do
    Ok(too_long) -> case Crypto.hmac_sha256(bounds_key, too_long) do
      Err(error) -> report("hmac-message-length", invalid_length_is(error, 65536, 65537))
      Ok(unexpected) -> do
        Secret.destroy(unexpected)
        report("hmac-message-length", false)
      end
    end
    Err(_) -> report("hmac-message-length", false)
  end
  case Crypto.hkdf_sha256(bounds_key, Bytes.empty(), Bytes.empty(), 0) do
    Err(error) -> report("hkdf-zero-length", invalid_length_is(error, 8160, 0))
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("hkdf-zero-length", false)
    end
  end
  case Crypto.hkdf_sha256(bounds_key, Bytes.empty(), Bytes.empty(), 8161) do
    Err(error) -> report("hkdf-excessive-length", invalid_length_is(error, 8160, 8161))
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("hkdf-excessive-length", false)
    end
  end
  Secret.destroy(bounds_key)
  Ok(0)
end

fn tamper_last_byte(ciphertext :: Bytes) -> Bytes ! String do
  let length = Bytes.length(ciphertext)
  let last = Bytes.get(ciphertext, length - 1) ?
  let prefix = Bytes.slice(ciphertext, 0, length - 1) ?
  let replacement = if last == 120 do Bytes.from_utf8("y") else Bytes.from_utf8("x") end
  Bytes.concat(prefix, replacement)
end

fn check_x25519_and_aead() -> Int ! CryptoError do
  let alice = Crypto.x25519_generate() ?
  let bob = Crypto.x25519_generate() ?
  let alice_public = Crypto.x25519_public(alice.private_key) ?
  report(
    "x25519-public-abi",
    Bytes.length(alice.public_key.bytes) == 32 and
      Bytes.length(alice_public.bytes) == 32 and
      Bytes.secure_equals(alice.public_key.bytes, alice_public.bytes)
  )

  case Bytes.repeat(0, 31) do
    Ok(short_public) -> case Crypto.x25519_shared(
      alice.private_key,
      X25519PublicKey { bytes: short_public }
    ) do
      Err(InvalidPublicKey) -> report("x25519-invalid-public", true)
      Ok(unexpected) -> do
        Secret.destroy(unexpected)
        report("x25519-invalid-public", false)
      end
      _ -> report("x25519-invalid-public", false)
    end
    Err(_) -> report("x25519-invalid-public", false)
  end
  case Bytes.repeat(0, 32) do
    Ok(zero_public) -> case Crypto.x25519_shared(
      alice.private_key,
      X25519PublicKey { bytes: zero_public }
    ) do
      Err(InvalidPublicKey) -> report("x25519-noncontributory", true)
      Ok(unexpected) -> do
        Secret.destroy(unexpected)
        report("x25519-noncontributory", false)
      end
      _ -> report("x25519-noncontributory", false)
    end
    Err(_) -> report("x25519-noncontributory", false)
  end

  let alice_shared = Crypto.x25519_shared(alice.private_key, bob.public_key) ?
  let bob_shared = Crypto.x25519_shared(bob.private_key, alice.public_key) ?
  let alice_key = Crypto.aead_key(alice_shared) ?
  let bob_key = Crypto.aead_key(bob_shared) ?
  let nonce = Bytes.from_utf8("123456789012")
  let associated_data = Bytes.from_utf8("mesh-msg/v1/aad")
  let plaintext = Bytes.from_utf8("private payload")
  let ciphertext = Crypto.aead_seal(alice_key, nonce, associated_data, plaintext) ?
  let opened = Crypto.aead_open(bob_key, nonce, associated_data, ciphertext) ?
  let agreed = Bytes.secure_equals(opened, plaintext)
  report("x25519-agreement", agreed)
  report("aead-ciphertext-size", Bytes.length(ciphertext) == Bytes.length(plaintext) + 16)
  report("aead-roundtrip", agreed)

  case tamper_last_byte(ciphertext) do
    Ok(tampered) -> case Crypto.aead_open(bob_key, nonce, associated_data, tampered) do
      Err(AuthenticationFailed) -> report("aead-tamper", true)
      _ -> report("aead-tamper", false)
    end
    Err(_) -> report("aead-tamper", false)
  end
  let wrong_material = Secret.random(32) ?
  let wrong_key = Crypto.aead_key(wrong_material) ?
  case Crypto.aead_open(wrong_key, nonce, associated_data, ciphertext) do
    Err(AuthenticationFailed) -> report("aead-wrong-key", true)
    _ -> report("aead-wrong-key", false)
  end
  let reopened = Crypto.aead_open(bob_key, nonce, associated_data, ciphertext) ?
  report("aead-key-after-failure", Bytes.secure_equals(reopened, plaintext))

  let short_material = Secret.random(31) ?
  case Crypto.aead_key(short_material) do
    Err(InvalidKey) -> report("aead-invalid-key", true)
    _ -> report("aead-invalid-key", false)
  end
  case Crypto.aead_seal(
    alice_key,
    Bytes.from_utf8("12345678901"),
    associated_data,
    plaintext
  ) do
    Err(error) -> report("aead-nonce-length", invalid_length_is(error, 12, 11))
    _ -> report("aead-nonce-length", false)
  end
  case Bytes.repeat(0, 65537) do
    Ok(too_long) -> case Crypto.aead_seal(alice_key, nonce, associated_data, too_long) do
      Err(error) -> report("aead-plaintext-length", invalid_length_is(error, 65536, 65537))
      _ -> report("aead-plaintext-length", false)
    end
    Err(_) -> report("aead-plaintext-length", false)
  end
  case Bytes.repeat(0, 65553) do
    Ok(too_long) -> case Crypto.aead_open(bob_key, nonce, associated_data, too_long) do
      Err(error) -> report("aead-ciphertext-bound", invalid_length_is(error, 65552, 65553))
      _ -> report("aead-ciphertext-bound", false)
    end
    Err(_) -> report("aead-ciphertext-bound", false)
  end
  Ok(0)
end

fn churn_rejected_aead_keys(0) -> Bool do true end
fn churn_rejected_aead_keys(count :: Int) -> Bool do
  case Secret.random(31) do
    Ok(material) -> case Crypto.aead_key(material) do
      Err(InvalidKey) -> churn_rejected_aead_keys(count - 1)
      _ -> false
    end
    Err(_) -> false
  end
end

fn check_resource_baseline() do
  let rejected_cleanly = churn_rejected_aead_keys(4097)
  case Secret.random(1) do
    Ok(secret) -> do
      Secret.destroy(secret)
      report("resource-baseline", rejected_cleanly)
    end
    Err(_) -> report("resource-baseline", false)
  end
end

fn check_signing() -> Int ! CryptoError do
  let signer = Crypto.signing_generate() ?
  let message = Bytes.from_utf8("nominal signature ABI")
  let signature = Crypto.sign(signer.private_key, message) ?
  report("signing-public-abi", Bytes.length(signer.public_key.bytes) == 32)
  report("signature-abi", Bytes.length(signature.bytes) == 64)

  let valid = Crypto.verify(signer.public_key, message, signature) ?
  report("signature-valid", valid)
  let mismatch = Crypto.verify(
    signer.public_key,
    Bytes.from_utf8("different message"),
    signature
  ) ?
  report("signature-mismatch", mismatch == false)

  case Bytes.repeat(0, 63) do
    Ok(short_signature) -> case Crypto.verify(
      signer.public_key,
      message,
      Signature { bytes: short_signature }
    ) do
      Err(InvalidSignature) -> report("signature-malformed", true)
      _ -> report("signature-malformed", false)
    end
    Err(_) -> report("signature-malformed", false)
  end
  case Bytes.repeat(0, 31) do
    Ok(short_public) -> case Crypto.verify(
      SigningPublicKey { bytes: short_public },
      message,
      signature
    ) do
      Err(InvalidPublicKey) -> report("signing-invalid-public", true)
      _ -> report("signing-invalid-public", false)
    end
    Err(_) -> report("signing-invalid-public", false)
  end
  Ok(0)
end

fn main() do
  let input = Bytes.from_utf8("abc")
  let sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
  let sha512 = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"

  report("sha256-binary", Bytes.to_hex(Crypto.sha256(input)) == sha256)
  report("sha512-binary", Bytes.to_hex(Crypto.sha512(input)) == sha512)
  report("sha256-hex", Crypto.sha256_hex(input) == sha256)
  report("sha512-hex", Crypto.sha512_hex(input) == sha512)
  check_large_hashes()
  check_random_bytes()
  check_secret_zero_length()
  case check_hmac_hkdf() do
    Ok(_) -> nil
    Err(_) -> report("hmac-hkdf-lifecycle", false)
  end
  case check_x25519_and_aead() do
    Ok(_) -> nil
    Err(_) -> report("x25519-public-abi", false)
  end
  case check_signing() do
    Ok(_) -> nil
    Err(_) -> report("signing-public-abi", false)
  end
  check_resource_baseline()
end
