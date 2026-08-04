struct Entry do
  id :: U64
  score :: Int
end

fn entry_id(entry :: Entry) -> U64 do
  entry.id
end

fn proof() -> Bool ! String do
  let seven = U64.parse("7") ?
  let nine = U64.parse("9") ?
  let entries = [Entry {
    id : seven,
    score : 7
  }, Entry {
    id : nine,
    score : 9
  }]
  let named = List.map(entries, entry_id)
  let inline = entries
    |> List.map(fn (entry) do entry.id end)
  let method = entries.map(fn (entry) do entry.score end)
  let offset = 1
  let shifted = List.map(entries, fn (entry) do entry.score + offset end)
  let copied = List.map(entries,
  fn (entry) do
    Entry {
      id : entry.id,
      score : entry.score
    }
  end)
  let floats = List.map([1.5], fn (value) do value + 0.5 end)
  let tuple_sums = List.map([(3, 4)], fn (pair) do Tuple.first(pair) + Tuple.second(pair) end)
  let tuples = List.map([1], fn (value) do (value, value + 1) end)
  let branch_tuples = List.map([1], fn 1 -> (1, 2)| _ -> (0, 0) end)
  let adders = List.map([1], fn (offset) do fn (value) do value + offset end end)
  let first_adder = List.get(adders, 0)
  let applied = List.map(adders, fn (adder) do adder(2) end)
  Ok(U64.compare(List.get(named, 0), seven) == 0 and U64.compare(List.get(inline, 1), nine) == 0 and List.get(shifted,
  1) == 10 and List.get(method, 1) == 9 and List.get(copied, 1).score == 9 and List.get(floats, 0) == 2.0 and List.get(tuple_sums,
  0) == 7 and Tuple.second(List.get(tuples, 0)) == 2 and Tuple.second(List.get(branch_tuples, 0)) == 2 and first_adder(3) == 4 and List.get(applied,
  0) == 3)
end

fn main() do
  case proof() do
    Ok( result) -> println("${result}")
    Err( error) -> println(error)
  end
end
