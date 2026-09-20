fn main() do
  let hash = Crypto.sha512_hex(Bytes.from_utf8("hello"))
  println(hash)
end
