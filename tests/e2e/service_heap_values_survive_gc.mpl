# Service arguments and replies cross between the caller and the service
# actor. Both sides drop the values and keep collecting, so each side must
# hold its own copy. Strings are built at run time, the same size as the
# churned garbage. Build with --opt-level 2.
struct Entry do
  label :: String
  count :: Int
end

# One word wide: it travels in its argument slot, not in a box.
struct Batch do
  names :: List<String>
end

service Shelf do
  fn init(seed :: Int) -> List<String> do
    List.new()
  end

  # A list argument kept in the service's state, a string reply.
  call Stock(items :: List<String>) :: String do |_state|
    (items, "stocked-${List.length(items)}")
  end

  # A struct argument; the reply is a list built from the service's state.
  call Tag(entry :: Entry) :: List<String> do |state|
    (state, List.append(state, "${entry.label}#${entry.count}"))
  end

  call Load(batch :: Batch) :: Int do |_state|
    (batch.names, List.length(batch.names))
  end

  cast Restock(items :: List<String>) do |_state|
    items
  end

  call First() :: String do |state|
    (state, List.get(state, 0))
  end
end

fn churn(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let s = "garbage-${i}"
    churn(i + 1, n, acc + String.length(s))
  end
end

fn run_client() -> Int do
  let shelf = Shelf.start(0)
  let reply = Shelf.stock(shelf, ["item-a-${100 + 1}", "item-b-${100 + 2}"])
  let tagged = Shelf.tag(shelf, Entry { label: "entry-${100 + 3}", count: 7 })
  churn(0, 200000, 0)
  println("reply=${reply}")
  println("tagged=${List.get(tagged, 0)},${List.get(tagged, 1)},${List.get(tagged, 2)}")
  let loaded = Shelf.load(shelf, Batch { names: ["batch-${100 + 5}", "batch-${100 + 6}"] })
  churn(0, 200000, 0)
  println("loaded=${loaded}:${Shelf.first(shelf)}")
  Shelf.restock(shelf, ["cast-x-${100 + 4}"])
  churn(0, 200000, 0)
  println("first=${Shelf.first(shelf)}")
  0
end

actor client() do
  receive do
    msg -> run_client()
  end
end

fn main() do
  let pid = spawn(client)
  send(pid, 1)
end
