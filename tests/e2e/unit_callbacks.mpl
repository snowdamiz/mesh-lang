# A callback declared to return () accepts a function returning anything;
# the result is discarded. A struct result exercises the adapter that keeps the
# caller from seeing a return value it has no slot for.
struct Report do
  title :: String
  lines :: List<String>
  score :: Int
end

fn each_twice(f :: Fun(Int) -> ()) do
  f(1)
  f(2)
end

fn tally(n :: Int) -> Int do
  println("tally #{n}")
  n * 10
end

fn report(n :: Int) -> Report do
  println("report #{n}")
  Report { title: "r", lines: ["a", "b"], score: n }
end

fn main() do
  let base = 100
  each_twice(tally)
  each_twice(report)
  each_twice(fn n -> base + n end)
  each_twice(fn n -> println("closure #{base + n}") end)
end
