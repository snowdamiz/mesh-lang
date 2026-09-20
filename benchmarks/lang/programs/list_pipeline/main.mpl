# List map/filter/reduce over a range-built list, repeated.
fn run(i :: Int, n :: Int, xs :: List<Int>, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let total = xs
      |> List.map(fn(x) -> x * 3 end)
      |> List.filter(fn(x) -> x % 2 == 0 end)
      |> List.reduce(0, fn(a, x) -> a + x end)
    run(i + 1, n, xs, acc + total % 1000)
  end
end

fn main() do
  let xs = for i in 0..100000 do
    i
  end
  println("${run(0, 200, xs, 0)}")
end
