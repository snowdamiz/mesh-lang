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

fn check_argon2id() -> Int ! CryptoError do
  let password = Secret.random(16) ?
  let salt = Bytes.from_utf8("12345678")
  let first = Crypto.argon2id(password, salt, 32, 1, 1, 32) ?
  let second = Crypto.argon2id(password, salt, 32, 1, 1, 32) ?
  let first_key = Crypto.aead_key(first) ?
  let second_key = Crypto.aead_key(second) ?
  let nonce = Bytes.from_utf8("argon2-nonce")
  let aad = Bytes.from_utf8("mesh-msg/v1/argon2id")
  let plaintext = Bytes.from_utf8("derived secret")
  let first_ciphertext = Crypto.aead_seal(first_key, nonce, aad, plaintext) ?
  let second_ciphertext = Crypto.aead_seal(second_key, nonce, aad, plaintext) ?

  report("argon2id-happy", Bytes.length(first_ciphertext) == Bytes.length(plaintext) + 16)
  report("argon2id-deterministic", Bytes.secure_equals(first_ciphertext, second_ciphertext))

  case Crypto.argon2id(password, Bytes.from_utf8("short"), 32, 1, 1, 32) do
    Err(error) -> report("argon2id-salt-bound", invalid_length_is(error, 8, 5))
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("argon2id-salt-bound", false)
    end
  end
  case Crypto.argon2id(password, salt, 7, 1, 1, 32) do
    Err(error) -> report("argon2id-memory-bound", invalid_length_is(error, 8, 7))
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("argon2id-memory-bound", false)
    end
  end
  case Crypto.argon2id(password, salt, 32, 1, 1, 65) do
    Err(error) -> report("argon2id-output-bound", invalid_length_is(error, 64, 65))
    Ok(unexpected) -> do
      Secret.destroy(unexpected)
      report("argon2id-output-bound", false)
    end
  end
  Secret.destroy(password)
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

  let derived = Crypto.x25519_from_secret(Secret.random(32) ?) ?
  let derived_shared = Crypto.x25519_shared(derived.private_key, bob.public_key) ?
  let bob_derived_shared = Crypto.x25519_shared(bob.private_key, derived.public_key) ?
  let derived_key = Crypto.aead_key(derived_shared) ?
  let bob_derived_key = Crypto.aead_key(bob_derived_shared) ?
  let derivation_nonce = Bytes.from_utf8("derive-key12")
  let derivation_aad = Bytes.from_utf8("mesh-treekem/v1")
  let derivation_plaintext = Bytes.from_utf8("secret-derived X25519")
  let derivation_ciphertext = Crypto.aead_seal(derived_key,
  derivation_nonce,
  derivation_aad,
  derivation_plaintext) ?
  let derivation_opened = Crypto.aead_open(bob_derived_key,
  derivation_nonce,
  derivation_aad,
  derivation_ciphertext) ?
  report("x25519-secret-derivation", Bytes.secure_equals(derivation_opened, derivation_plaintext))

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
      Err(_) -> report("x25519-invalid-public", false)
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
      Err(_) -> report("x25519-noncontributory", false)
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

fn check_hpke() -> Int ! CryptoError do
  let recipient = Crypto.x25519_generate() ?
  let info = Bytes.from_utf8("mesh-mls/v1/welcome")
  let associated_data = Bytes.from_utf8("group-and-epoch")
  let plaintext = Bytes.from_utf8("epoch secret")
  let sealed = Crypto.hpke_seal(recipient.public_key, info, associated_data, plaintext) ?
  let opened = Crypto.hpke_open(recipient.private_key, info, associated_data, sealed) ?
  report("hpke-wire-size", Bytes.length(sealed) == Bytes.length(plaintext) + 48)
  report("hpke-roundtrip", Bytes.secure_equals(opened, plaintext))

  case Crypto.hpke_open(
    recipient.private_key,
    Bytes.from_utf8("wrong info"),
    associated_data,
    sealed
  ) do
    Err(AuthenticationFailed) -> report("hpke-info-binding", true)
    _ -> report("hpke-info-binding", false)
  end
  case Bytes.repeat(0, 47) do
    Ok(short) -> case Crypto.hpke_open(recipient.private_key, info, associated_data, short) do
      Err(error) -> report("hpke-wire-bound", invalid_length_is(error, 48, 47))
      _ -> report("hpke-wire-bound", false)
    end
    Err(_) -> report("hpke-wire-bound", false)
  end

  let secret = Secret.random(32) ?
  let sealed_secret = Crypto.hpke_seal_secret(recipient.public_key, info, associated_data, secret) ?
  let opened_secret = Crypto.hpke_open_secret(recipient.private_key, info, associated_data, sealed_secret) ?
  let sender_key = Crypto.aead_key(secret) ?
  let recipient_key = Crypto.aead_key(opened_secret) ?
  let nonce = Bytes.from_utf8("123456789012")
  let secret_ciphertext = Crypto.aead_seal(sender_key, nonce, associated_data, plaintext) ?
  let secret_opened = Crypto.aead_open(recipient_key, nonce, associated_data, secret_ciphertext) ?
  report("hpke-secret-roundtrip", Bytes.secure_equals(secret_opened, plaintext))
  Ok(0)
end

fn crypto_vector_bytes(value :: String) -> Bytes ! CryptoError do
  case Bytes.from_hex(value) do
    Err(_) -> Err(InternalFailure)
    Ok(value) -> Ok(value)
  end
end

fn mlkem_storage_context() -> Bytes ! CryptoError do
  case Bytes.from_hex("0100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f0000000000000001") do
    Err(_) -> Err(InvalidLength(123, 0))
    Ok(value) -> Ok(value)
  end
end

fn consume_unexpected_mlkem_pair(pair :: consume MlKemKeyPair) do
  nil
end

fn check_mlkem() -> Int ! CryptoError do
  let pair = Crypto.mlkem_generate() ?
  let (ciphertext, sender_secret) = Crypto.mlkem_encapsulate(pair.public_key) ?
  let wrapping_key = StorageKey.ephemeral() ?
  let storage_context = mlkem_storage_context() ?
  let private_blob = MlKemPrivateKey.seal_for_storage(pair.private_key, wrapping_key, storage_context) ?
  let restored_private = MlKemPrivateKey.unseal_from_storage(private_blob, wrapping_key, storage_context) ?
  let receiver_secret = Crypto.mlkem_decapsulate(restored_private, ciphertext) ?
  let sender_key = Crypto.aead_key(sender_secret) ?
  let receiver_key = Crypto.aead_key(receiver_secret) ?
  let nonce = Bytes.from_utf8("123456789012")
  let aad = Bytes.from_utf8("mesh-mlkem768-v1")
  let plaintext = Bytes.from_utf8("post-quantum round trip")
  let sealed = Crypto.aead_seal(sender_key, nonce, aad, plaintext) ?
  let opened = Crypto.aead_open(receiver_key, nonce, aad, sealed) ?
  report(
    "mlkem-layout",
    Bytes.length(pair.public_key.bytes) == 1184 and Bytes.length(ciphertext.bytes) == 1088
  )
  report("mlkem-storage", Bytes.length(private_blob) == 131)
  report("mlkem-roundtrip", Bytes.secure_equals(opened, plaintext))

  let seed = Bytes.from_utf8("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
  let seeded_first = Crypto.mlkem_from_seed(seed) ?
  let seeded_second = Crypto.mlkem_from_seed(seed) ?
  report(
    "mlkem-seed-deterministic",
    Bytes.secure_equals(seeded_first.public_key.bytes, seeded_second.public_key.bytes)
  )

  let vector_seed = crypto_vector_bytes("__MLKEM_SEED_HEX__") ?
  let expected_public_key = crypto_vector_bytes("__MLKEM_PUBLIC_KEY_HEX__") ?
  let vector_pair = Crypto.mlkem_from_seed(vector_seed) ?
  report(
    "mlkem-nist-acvp-keygen",
    Bytes.secure_equals(vector_pair.public_key.bytes, expected_public_key)
  )

  case Bytes.repeat(0, __MLKEM_SHORT_SEED_LENGTH__) do
    Ok(short_seed) -> case Crypto.mlkem_from_seed(short_seed) do
      Err(error) -> report(
        "mlkem-invalid-seed-short",
        invalid_length_is(error, 64, __MLKEM_SHORT_SEED_LENGTH__)
      )
      Ok(unexpected) -> do
        consume_unexpected_mlkem_pair(unexpected)
        report("mlkem-invalid-seed-short", false)
      end
    end
    Err(_) -> report("mlkem-invalid-seed-short", false)
  end
  case Bytes.repeat(0, __MLKEM_LONG_SEED_LENGTH__) do
    Ok(long_seed) -> case Crypto.mlkem_from_seed(long_seed) do
      Err(error) -> report(
        "mlkem-invalid-seed-long",
        invalid_length_is(error, 64, __MLKEM_LONG_SEED_LENGTH__)
      )
      Ok(unexpected) -> do
        consume_unexpected_mlkem_pair(unexpected)
        report("mlkem-invalid-seed-long", false)
      end
    end
    Err(_) -> report("mlkem-invalid-seed-long", false)
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
  case check_argon2id() do
    Ok(_) -> nil
    Err(_) -> report("argon2id-happy", false)
  end
  case check_x25519_and_aead() do
    Ok(_) -> nil
    Err(_) -> report("x25519-public-abi", false)
  end
  case check_signing() do
    Ok(_) -> nil
    Err(_) -> report("signing-public-abi", false)
  end
  case check_hpke() do
    Ok(_) -> nil
    Err(_) -> report("hpke-roundtrip", false)
  end
  case check_mlkem() do
    Ok(_) -> nil
    Err(_) -> report("mlkem-roundtrip", false)
  end
end
