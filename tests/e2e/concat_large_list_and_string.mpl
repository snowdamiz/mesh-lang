# `List.concat` and string interpolation size their result from the length
# fields of their inputs and then copy into it. A result allocated too small
# writes over whatever the heap handed out next, which is how an overrun in
# one of them shows up: not in the value that overran, but in its neighbour.
# So each check below reads back an object allocated *after* the concat as
# well as the concat's own two ends.

fn grow(i :: Int, n :: Int, acc :: String) -> String do
  if i >= n do
    acc
  else
    grow(i + 1, n, "${acc}${i % 10}")
  end
end

fn main() do
  let left = for i in 0..1000 do "l-${i}" end
  let right = for i in 0..1000 do "r-${i}" end
  let both = List.concat(left, right)
  # Allocated after the concat, so an undersized result lands on it.
  let neighbour = for i in 0..1000 do "a-${i}" end
  println("${List.length(both)} ${List.get(both, 0)} ${List.get(both, 999)} ${List.get(both, 1000)} ${List.get(both, 1999)}")
  println("${List.length(neighbour)} ${List.get(neighbour, 0)} ${List.get(neighbour, 999)}")

  # 400 concatenations, each allocating a longer string than the last.
  let text = grow(0, 400, "")
  println("${String.length(text)} ${String.starts_with(text, "0123456789")} ${String.ends_with(text, "6789")}")
end
