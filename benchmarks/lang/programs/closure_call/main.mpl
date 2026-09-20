# 200M calls through a closure value held in a parameter, so nothing is inlined.
# Guards the environment check every function-value call now makes.
fn run(step :: Fun(Int) -> Int, i :: Int, n :: Int, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    run(step, i + 1, n, step(acc))
  end
end

fn main() do
  let k = 3
  let step = fn (x :: Int) -> (x + k) % 1000003 end
  println("${run(step, 0, 200000000, 0)}")
end
