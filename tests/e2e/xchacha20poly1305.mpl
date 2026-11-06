fn fail(error :: CryptoError) -> Bool do
  case error do
    AuthenticationFailed -> true
    _ -> false
  end
end
fn proof() -> Bool!CryptoError do
  let key=Crypto.aead_key(Secret.random(32)?)?
  let nonce=Crypto.random_bytes(24)?
  let aad=Bytes.from_utf8("header direction and version")
  let body=Bytes.from_utf8("private header\0")
  let encrypted=Crypto.xchacha20poly1305_seal(key,nonce,aad,body)?
  let opened=Crypto.xchacha20poly1305_open(key,nonce,aad,encrypted)?
  if !Bytes.secure_equals(opened,body) || Bytes.length(encrypted)!=Bytes.length(body)+16 do return Ok(false) end
  let wrong=case Crypto.xchacha20poly1305_open(key,nonce,Bytes.empty(),encrypted) do
    Err(error) -> fail(error)
    Ok(_) -> false
  end
  let nonce_refused=case Crypto.xchacha20poly1305_seal(key,Bytes.empty(),aad,body) do
    Err(InvalidLength(expected,actual)) -> expected==24 && actual==0
    _ -> false
  end
  let again=crypto_xchacha20poly1305_seal(key,nonce,aad,body)?
  let borrowed=Bytes.secure_equals(Crypto.xchacha20poly1305_open(key,nonce,aad,again)?,body)
  Secret.destroy(key)
  Ok(wrong && nonce_refused && borrowed)
end
fn main() do
  if Result.unwrap_or(proof(),false) do println("xchacha20poly1305:ok") else println("xchacha20poly1305:failed") end
end
