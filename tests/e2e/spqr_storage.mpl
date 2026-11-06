##! Approved typed SPQR storage public API fixture. Synthetic opaque keys only.
fn hex(value :: String) -> Bytes!CryptoError do
  case Bytes.from_hex(value) do
    Err(_) -> Err(InternalFailure)
    Ok(value)
  end
end

fn check(value :: Bool) -> Unit!CryptoError do
  if value do
    Ok(nil)
  else
    Err(InternalFailure)
  end
end

fn proof() -> Unit!CryptoError do
  let key = StorageKey.ephemeral()?
  let chains = SecretMap.new(7)?
  let skipped = SecretMap.new(64)?
  let root_name = hex("00")?
  let skip_name = hex("0200000000000000000000000000000001")?
  let root = Secret.random(32)?
  let skipped_key = Secret.random(32)?
  let challenge = Bytes.from_utf8("SPQR public storage fixture v1")
  let root_tag = Crypto.hmac_sha256_tag(root, challenge)?
  let skip_tag = Crypto.hmac_sha256_tag(skipped_key, challenge)?
  SecretMap.insert(chains, root_name, root)?
  SecretMap.insert(chains, hex("01000000000000000000")?, Secret.random(32)?)?
  SecretMap.insert(chains, hex("01000000000000000001")?, Secret.random(32)?)?
  SecretMap.insert(skipped, skip_name, skipped_key)?
  let chain_context = hex("011111111111111111111111111111111111111111111111111111111111111111222222222222222222222222222222223333333333333333333333333333333333333333333333333333333333333333e11dfb36e0576a6b1354b3a7fe1e665433aeaecbba44fc033a69f38d80a4187f00180000000000000009010000000000000000000000000000000000555555555555555555555555555555555555555555555555555555555555555566666666666666666666666666666666666666666666666666666666666666660100000000000000000100000000000000000100000000000000020100000000000000000000000000000001")?
  let skip_context = hex("011111111111111111111111111111111111111111111111111111111111111111222222222222222222222222222222223333333333333333333333333333333333333333333333333333333333333333a4415b51e05deb79c5799625c21212bc9879e386e478e88f95d993312c86e9fa00190000000000000009010000000000000000000000000000000000555555555555555555555555555555555555555555555555555555555555555566666666666666666666666666666666666666666666666666666666666666660100000000000000000100000000000000000100000000000000020100000000000000000000000000000001")?
  let chain_blob = SecretMap.seal_for_storage(chains, key, chain_context)?
  let skip_blob = SecretMap.seal_for_storage(skipped, key, skip_context)?
  check(Bytes.length(chain_blob) == 206 && Bytes.length(skip_blob) == 126)?
  let restored_chains = SecretMap.unseal_from_storage(chain_blob, key, chain_context)?
  let restored_skipped = SecretMap.unseal_from_storage(skip_blob, key, skip_context)?
  let restored_root = SecretMap.copy(restored_chains, root_name)?
  let restored_skip = SecretMap.copy(restored_skipped, skip_name)?
  check(Bytes.secure_equals(root_tag, Crypto.hmac_sha256_tag(restored_root, challenge)?))?
  check(Bytes.secure_equals(skip_tag, Crypto.hmac_sha256_tag(restored_skip, challenge)?))?
  check(case SecretMap.unseal_from_storage(chain_blob, key, skip_context) do
    Err(AuthenticationFailed) -> true
    _ -> false
  end)?
  check(case Secret.seal_for_storage(restored_root, key, chain_context) do
    Err(_) -> true
    Ok(_) -> false
  end)?
  # Public storage borrows its sources; use them again without exporting values.
  let original_root = SecretMap.copy(chains, root_name)?
  check(Bytes.secure_equals(root_tag, Crypto.hmac_sha256_tag(original_root, challenge)?))?
  Secret.destroy(original_root)
  Secret.destroy(restored_root)
  Secret.destroy(restored_skip)
  Secret.destroy(chains)
  Secret.destroy(skipped)
  Secret.destroy(restored_chains)
  Secret.destroy(restored_skipped)
  Secret.destroy(key)
  Ok(nil)
end

fn main() do
  case proof() do
    Ok(_) -> println("SPQR_STORAGE_PUBLIC_API_EXECUTED")
    Err(_) -> println("SPQR_STORAGE_PUBLIC_API_FAILED")
  end
end
