from Bindings import native_text, text_calls, from_utf8, consume, abi_from_utf8, invalid_utf8

actor worker() do
  let direct = Bytes.length(Bytes.from_utf8(native_text()))
  let calls = text_calls()
  let text = native_text()
  let shadowed = Bytes.length(from_utf8(text))
  let through_abi = Bytes.length(abi_from_utf8(text))
  let kept = Bytes.from_utf8(text)
  let churn = String.repeat("é🙂", 87382)
  println("#{direct} #{calls} #{String.length(text)} #{shadowed} #{through_abi} #{consume(kept)} #{Bytes.length(Bytes.from_utf8(churn))}")
  let raw = invalid_utf8()
  let raw_length = Bytes.length(Bytes.from_utf8(raw))
  let raw_copy = abi_from_utf8(raw)
  println("#{raw_length} #{Bytes.to_hex(raw_copy)}")
end

fn main() do
  let _ = spawn(worker)
end
