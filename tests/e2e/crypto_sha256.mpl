fn main() do
  let hash = Crypto.sha256_hex(Bytes.from_utf8("hello"))
  println(hash)
end
