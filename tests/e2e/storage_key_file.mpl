fn proof() -> Bool!String do
  let directory = Env.get("MESH_STORAGE_DIRECTORY", "")
  let output = Env.get("MESH_STORAGE_OUTPUT", "")
  let wrapping = case StorageKey.file(directory) do Err(_) -> return Err("file key failed") Ok(value) -> value end
  let key_context = Bytes.from_hex("__KEY_CONTEXT__")?
  let state_context = Bytes.from_hex("__STATE_CONTEXT__")?
  let message = Bytes.from_utf8("public Mesh BR1 restart custody proof")
  if Env.get("MESH_STORAGE_MODE", "") == "write" do
    let signer = case Crypto.blind_rsa_generate() do Err(_) -> return Err("generation failed") Ok(value) -> value end
    let public = case Crypto.blind_rsa_public(signer) do Err(_) -> return Err("public failed") Ok(value) -> value end
    let pending = case Crypto.blind_rsa_blind(public, message) do Err(_) -> return Err("blind failed") Ok(value) -> value end
    let response = case Crypto.blind_rsa_sign(signer, pending.blinded) do Err(_) -> return Err("sign failed") Ok(value) -> value end
    let state_blob = case BlindRsaBlindingState.seal_for_storage(pending.state, wrapping, state_context) do Err(_) -> return Err("state seal failed") Ok(value) -> value end
    let key_blob = case BlindRsaSecretKey.seal_for_storage(signer, wrapping, key_context) do Err(_) -> return Err("key seal failed") Ok(value) -> value end
    File.write(output <> "/key", Bytes.to_hex(key_blob))?
    File.write(output <> "/state", Bytes.to_hex(state_blob))?
    File.write(output <> "/public", Bytes.to_hex(public.bytes))?
    File.write(output <> "/response", Bytes.to_hex(response))?
  else
    let key_blob = Bytes.from_hex(File.read(output <> "/key")?)?
    let state_blob = Bytes.from_hex(File.read(output <> "/state")?)?
    let response = Bytes.from_hex(File.read(output <> "/response")?)?
    let public_bytes = Bytes.from_hex(File.read(output <> "/public")?)?
    let signer = case BlindRsaSecretKey.unseal_from_storage(key_blob, wrapping, key_context) do Err(_) -> return Err("key restore failed") Ok(value) -> value end
    let public = case Crypto.blind_rsa_public(signer) do Err(_) -> return Err("restored public failed") Ok(value) -> value end
    if !Bytes.secure_equals(public.bytes, public_bytes) do return Err("public changed") end
    let state = case BlindRsaBlindingState.unseal_from_storage(state_blob, wrapping, state_context) do Err(_) -> return Err("state restore failed") Ok(value) -> value end
    let signature = case Crypto.blind_rsa_finalize(public, message, response, state) do Err(_) -> return Err("finalize failed") Ok(value) -> value end
    let verified = case Crypto.blind_rsa_verify(public, message, signature) do Err(_) -> return Err("verify failed") Ok(value) -> value end
    if !verified do return Err("invalid signature") end
    Secret.destroy(signer)
  end
  Secret.destroy(wrapping)
  Ok(true)
end

fn main() do
  case proof() do
    Ok(true) -> println("storage-file-restart:ok")
    _ -> println("storage-file-restart:failed")
  end
end
