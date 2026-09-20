from Bindings.Math import add, double

fn print_double(value :: Int) do
  case double(value) do
    Ok(result) -> println("#{result}")
    Err(error) -> println(error)
  end
end

fn main() do
  20 |> add(1) |> print_double()
  print_double(-1)
end
