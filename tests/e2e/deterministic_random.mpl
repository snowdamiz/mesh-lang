fn main() do
  let initial = Random.seed(42)
  let first = Random.next_int(initial, 1, 100)
  let repeated = Random.next_int(initial, 1, 100)
  println("${Tuple.second(first)}")
  println("${Tuple.second(first) == Tuple.second(repeated)}")
  println("${Tuple.first(first) != initial}")

  let unit = Random.next_unit_ppm(Tuple.first(first))
  println("${Tuple.second(unit) >= 0 && Tuple.second(unit) < 1000000}")
end
