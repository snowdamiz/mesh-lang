type Message do
  Add(Int)
  Reset
end

fn apply(total :: Int, message :: Message) -> Int do
  case message do
    Add(value) -> total + value
  end
end

fn main() do
  println("#{apply(0, Add(42))}")
end
