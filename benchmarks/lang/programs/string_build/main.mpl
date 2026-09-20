# String interpolation + concat + length in a loop.
fn go(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    let s = "user-${i}:" <> "value-${i * 2}"
    go(i + 1, n, acc + String.length(s))
  end
end

fn main() do
  println("${go(0, 3000000, 0)}")
end
