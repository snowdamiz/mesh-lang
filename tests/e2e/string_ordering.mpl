# `<`, `>`, `<=`, `>=` and `compare` on strings.
#
# The type checker accepted them and lowering always generates
# `Ord__compare__String` with `<`, but codegen had no case for them: a build
# that used one failed with "Unsupported binop type: String", and the REPL,
# which prunes nothing, failed on every evaluation.

fn order_name(o :: Ordering) -> String do
  case o do
    Less -> "less"
    Equal -> "equal"
    Greater -> "greater"
  end
end

fn by_name(x :: String, y :: String) -> Int do
  if x < y do
    -1
  else
    if x > y do
      1
    else
      0
    end
  end
end

fn main() do
  let a = "apple-${1}"
  let b = "banana-${2}"
  println("${a < b} ${a > b} ${a <= a} ${b >= a} ${a < a}")
  println("${order_name(compare(a, b))} ${order_name(compare(b, a))} ${order_name(compare(a, a))}")
  let sorted = List.sort(["pear", "apple", "fig"], by_name)
  println(String.join(sorted, ","))
end
