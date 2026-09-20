# Non-tail recursion: call overhead + reduction checks.
fn fib(n :: Int) -> Int do
  if n < 2 do
    n
  else
    fib(n - 1) + fib(n - 2)
  end
end

fn main() do
  println("${fib(35)}")
end
