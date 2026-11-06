fn create_then_drop() -> (String, Bytes)!CryptoError do
  let (key, code) = RecoveryCode.generate()?
  let tag = Crypto.hmac_sha256_tag(key, Bytes.from_utf8("public recovery agreement"))?
  Secret.destroy(key)
  Ok((code, tag))
end

fn recovery_roundtrip() -> Bool!CryptoError do
  let (code, expected) = create_then_drop()?
  let restored = recovery_code_parse(code)?
  let actual = Crypto.hmac_sha256_tag(restored, Bytes.from_utf8("public recovery agreement"))?
  let refused = case RecoveryCode.parse(String.replace(code, "MRC1", "MRC2")) do
    Err(InvalidKey) -> true
    _ -> false
  end
  let retry = Crypto.hmac_sha256_tag(restored, Bytes.from_utf8("public recovery agreement"))?
  Secret.destroy(restored)
  Ok(String.length(code) == 85
    && Bytes.secure_equals(actual, expected)
    && Bytes.secure_equals(retry, expected)
    && refused)
end

fn main() do
  case recovery_roundtrip() do
    Ok(true) -> println("recovery-code:ok")
    _ -> println("recovery-code:failed")
  end
end
