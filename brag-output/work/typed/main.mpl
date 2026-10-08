actor counter(total :: Int) do
  receive do
    amount -> counter(total + amount)
  end
end

fn main() do
  let pid = spawn(counter, 0)
  send(pid, 5)
  send(pid, "five")
end
