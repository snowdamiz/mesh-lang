fn c1_jit_context() -> Bytes!CryptoError do
  case Bytes.from_hex("010303030303030303030303030303030303030303030303030303030303030303040404040404040404040404040404040505050505050505050505050505050505050505050505050505050505050505060606060606060606060606060606060606060606060606060606060606060600150000000000000001") do Ok(value) -> Ok(value) Err(_) -> Err(InternalFailure) end
end

#--definition--
fn c1_jit_concat(first :: Bytes, second :: Bytes) -> Bytes!CryptoError do
  case Bytes.concat(first, second) do Ok(value) -> Ok(value) Err(_) -> Err(InternalFailure) end
end

#--definition--
fn c1_jit_challenge(secret :: consume SecretBytes) -> Bytes!CryptoError do
  let key = Crypto.aead_key(secret)?
  let value = Crypto.aead_seal(key, Bytes.from_utf8("0123456789ab"), Bytes.empty(), Bytes.from_utf8("C1 public JIT challenge"))?
  Secret.destroy(key)
  Ok(value)
end

#--definition--
fn c1_incremental_probe() -> Int!CryptoError do
  let recipient = Crypto.mlkem_generate()?
  let (header, vector) = crypto_mlkem_public_parts(recipient.private_key)?
  Crypto.mlkem_validate_public_parts(header, vector)?
  crypto_mlkem_validate_public_parts(header, vector)?
  let operation = Crypto.mlkem_encapsulate1
  let (state, first, shared) = operation(header)?
  let wrong_bytes = case Bytes.repeat(0, 1152) do Ok(value) -> value Err(_) -> return Err(InternalFailure) end
  let validation_refused = case Crypto.mlkem_validate_public_parts(header, MlKemPublicVector1152 { bytes: wrong_bytes }) do Err(InvalidPublicKey) -> true _ -> false end
  if !validation_refused do return Err(InternalFailure) end
  let refused = case Crypto.mlkem_encapsulate2(state, MlKemPublicVector1152 { bytes: wrong_bytes }) do Err(InvalidPublicKey) -> true _ -> false end
  let second = crypto_mlkem_encapsulate2(state, vector)?
  let wrapping = StorageKey.ephemeral()?
  let scope = c1_jit_context()?
  let blob = MlKemEncapsulationState.seal_for_storage(state, wrapping, scope)?
  Secret.destroy(state)
  let restored = mlkem_encapsulation_state_unseal_from_storage(blob, wrapping, scope)?
  let retried = Crypto.mlkem_encapsulate2(restored, vector)?
  let complete = MlKemCiphertext { bytes: c1_jit_concat(first.bytes, second.bytes)? }
  let full_shared = Crypto.mlkem_decapsulate(recipient.private_key, complete)?
  let parts_shared = Crypto.mlkem_decapsulate_parts(recipient.private_key, first, second)?
  let expected = c1_jit_challenge(shared)?
  let full_output = c1_jit_challenge(full_shared)?
  let parts_output = c1_jit_challenge(parts_shared)?
  let widths = Bytes.length(header.bytes) == 64 && Bytes.length(vector.bytes) == 1152 && Bytes.length(first.bytes) == 960 && Bytes.length(second.bytes) == 128
  let agrees = Bytes.secure_equals(expected, full_output) && Bytes.secure_equals(expected, parts_output) && Bytes.secure_equals(second.bytes, retried.bytes)
  Secret.destroy(restored)
  Secret.destroy(wrapping)
  Secret.destroy(recipient.private_key)
  Ok(if widths && refused && agrees do 71 else 0 end)
end
