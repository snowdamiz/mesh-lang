struct Payload do
  number :: Int
  bytes :: Bytes
end

type Target do
  Saved(Payload)
  Count(Int)
  Flag(Bool)
  Pair(Bool, String, Int)
end

struct Carrier do
  enabled :: Bool
  target :: Target
  ending :: Bool
end

fn create(index :: Int) -> Carrier do
  let target = if index == 8 do
    Flag(true)
  else if index == 9 do
    Count(99)
  else if index == 10 do
    Pair(true, "pair-#{index}", index)
  else
    Saved(Payload { number: index, bytes: Bytes.from_utf8("payload-#{index}") })
  end
  Carrier { enabled: true, target: target, ending: false }
end

fn churn(left :: Int) -> Int do
  if left == 0 do
    0
  else
    let bytes = Bytes.from_utf8(String.repeat("#{left}", 200))
    Bytes.length(bytes) + churn(left - 1)
  end
end

fn inspect(value :: Carrier) -> String do
  if !value.enabled || value.ending do
    return "carrier-flags-corrupted"
  end
  case value.target do
    Saved(payload) -> "#{payload.number}:#{Bytes.to_hex(payload.bytes)}"
    Count(index) -> "count-#{index}"
    Flag(value) -> "flag-#{value}"
    Pair(flag, label, number) -> "#{flag}:#{label}:#{number}"
  end
end

fn main() do
  let values = for index in 0..11 do
    create(index)
  end
  println("#{churn(8000)}")
  for value in values do
    println(inspect(value))
  end
end
