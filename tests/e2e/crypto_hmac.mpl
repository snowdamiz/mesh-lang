fn hmac_v2() -> Int ! CryptoError do
  let key = Secret.random(32) ?
  let tag = Crypto.hmac_sha256(key, Bytes.from_utf8("what do ya want for nothing?")) ?
  Secret.destroy(tag)
  Secret.destroy(key)
  Ok(0)
end

fn main() do
  case hmac_v2() do
    Ok(_) -> println("ok")
    Err(_) -> println("error")
  end
  let h512 = Crypto.hmac_sha512("Jefe", "what do ya want for nothing?")
  println(h512)
end
