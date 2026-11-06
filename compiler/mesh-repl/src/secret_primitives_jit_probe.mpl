fn c1_secret_primitives_probe() -> Int!CryptoError do
  let recipient=Crypto.mlkem_generate()?
  let (ct,first)=Crypto.mlkem_encapsulate(recipient.public_key)?
  let second=Crypto.mlkem_decapsulate(recipient.private_key,ct)?
  let info=Bytes.from_utf8("C1 opaque prerequisite fixture v1")
  let refused=case Crypto.hkdf_sha256_secret_salt(first,second,info,0) do Err(InvalidLength(maximum,actual)) -> maximum==8160 && actual==0 _ -> false end
  let wide=crypto_hkdf_sha256_secret_salt(first,second,info,64)?
  let prefix=Crypto.hkdf_sha256_secret_salt(second,first,info,32)?
  let expected=Crypto.hmac_sha256_tag(wide,info)?
  let (left,right)=Secret.split_at(wide,32)?
  let left_tag=Crypto.hmac_sha256_tag(left,info)?
  let prefix_tag=crypto_hmac_sha256_tag(prefix,info)?
  let joined=Secret.concat(left,right)?
  let joined_tag=Crypto.hmac_sha256_tag(joined,info)?
  let invalid=Secret.random(32)?
  let split_refused=case secret_split_at(invalid,32) do Err(InvalidKey) -> true _ -> false end
  let too_long=case Bytes.repeat(0,65537) do Ok(value) -> value Err(_) -> return Err(InternalFailure) end
  let public_bound=case Crypto.hmac_sha256_tag(first,too_long) do Err(InvalidLength(maximum,actual)) -> maximum==65536 && actual==65537 _ -> false end
  let key_still_usable=Crypto.hmac_sha256_tag(first,info)?
  let same_shared_tag=Crypto.hmac_sha256_tag(second,info)?
  Secret.destroy(first)
  Secret.destroy(second)
  Secret.destroy(prefix)
  Secret.destroy(joined)
  Secret.destroy(recipient.private_key)
  Ok(if refused && split_refused && public_bound && Bytes.length(expected)==32 && Bytes.secure_equals(expected,joined_tag) && Bytes.secure_equals(left_tag,prefix_tag) && Bytes.secure_equals(key_still_usable,same_shared_tag) do 73 else 0 end)
end
