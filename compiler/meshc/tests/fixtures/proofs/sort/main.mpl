from Sort import sort

fn print_values(xs :: List<Int>) do
  case xs do
    [] -> println("end")
    head :: tail ->
      println("${head}")
      print_values(tail)
  end
end

fn main() do
  let least = -9223372036854775808
  let greatest = 9223372036854775807
  print_values(sort([]))
  print_values(sort([greatest, 0, least, -1, greatest, -1, least, 1, 0]))
end
