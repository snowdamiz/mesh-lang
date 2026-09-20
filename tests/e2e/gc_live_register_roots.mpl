struct Entry do
  index :: Int
  payload :: Bytes
end

fn burn_reductions(remaining :: Int) -> Int do
  if remaining <= 0 do
    0
  else
    burn_reductions(remaining - 1)
  end
end

fn identity(entries :: List < Entry >) -> List < Entry > do
  entries
end

fn verify_live_entries() do
  let inflated = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let inflated = inflated <> inflated
  let entries = [Entry { index : 0, payload : Bytes.from_utf8("zero") }, Entry { index : 1, payload : Bytes.from_utf8("one") }, Entry { index : 2, payload : Bytes.from_utf8("two") }]
  let _ = burn_reductions(3999)
  let entries = identity(entries)
  let inflated_ok = string_length(inflated) == 262144
  let entries_ok = (List.get(entries, 2)).index == 2
  if inflated_ok && entries_ok do
    println("gc live register roots preserved")
  else if !inflated_ok do
    println("gc live string root corrupted")
  else
    println("gc live list root corrupted")
  end
end

actor verifier() do
  receive do
    msg -> verify_live_entries()
  end
end

fn main() do
  let pid = spawn(verifier)
  send(pid, 1)
end
