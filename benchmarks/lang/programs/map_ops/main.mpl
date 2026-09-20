# Map put/get with a few thousand int keys.
fn fill(i :: Int, n :: Int, m :: Map<Int, Int>) -> Map<Int, Int> do
  if i >= n do
    m
  else
    fill(i + 1, n, Map.put(m, i, i * 2))
  end
end

fn probe(i :: Int, n :: Int, m :: Map<Int, Int>, acc :: Int) -> Int do
  if i >= n do
    acc
  else
    probe(i + 1, n, m, acc + Map.get(m, i % 4000))
  end
end

fn main() do
  let m = fill(0, 4000, Map.new())
  println("${probe(0, 400000, m, 0)}")
end
