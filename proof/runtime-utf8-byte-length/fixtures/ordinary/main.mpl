from Bytes import from_utf8

fn noisy() -> String do
  println("evaluated")
  "é🙂é"
end

fn direct(text :: String) -> Int do
  Bytes.length(Bytes.from_utf8(text))
end

fn indirect(text :: String) -> Int do
  let encode = Bytes.from_utf8
  let length_of = Bytes.length
  length_of(encode(text))
end

fn escaped(text :: String) -> Int do
  let bytes = Bytes.from_utf8(text)
  println(Bytes.to_hex(bytes))
  Bytes.length(bytes)
end

fn shadowed(text :: String) -> Int do
  let from_utf8 = fn(_ :: String) do Bytes.empty() end
  Bytes.length(from_utf8(text))
end

actor worker() do
  let s = String.repeat("é🙂", 87382)
  println("#{direct(s)} #{String.length(s)} #{direct(String.repeat("🙂", 17))} #{direct(s)}")
end

fn main() do
  println("#{Bytes.length(Bytes.from_utf8(noisy()))}")
  println("#{direct("")} #{String.length("é🙂é")} #{Bytes.length(from_utf8("é🙂"))}")
  println("#{indirect("é🙂")} #{shadowed("é🙂")} #{escaped("é🙂")}")
  let _ = spawn(worker)
end
