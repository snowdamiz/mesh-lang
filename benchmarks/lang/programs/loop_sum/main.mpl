# Tail-recursive arithmetic loop: back-edge cost.
fn sum_to(i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    sum_to(i + 1, n, acc + i % 7)
  end
end

fn main() do
  println("${sum_to(0, 300000000, 0)}")
end
