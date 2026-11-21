from Vector import Vec, deliver, allowed_two

fn main() do
  let single :: Vec<Int, 1> = Push(7, Empty)
  let double :: Vec<Int, 2> = Push(42, single)
  println("${deliver(double, allowed_two)}")
end
