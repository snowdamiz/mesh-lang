fn operand() -> String do
  println("before panic")
  panic("byte length operand failed")
end

fn byte_count() -> Int do
  Bytes.length(Bytes.from_utf8(operand()))
end

fn main() do
  println("#{byte_count()}")
  println("after panic")
end
