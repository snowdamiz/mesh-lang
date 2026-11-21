from Vector import Vec, copy, first, only

fn main() do
  let single :: Vec<Int, 1> = Push(7, Empty)
  let double :: Vec<Int, 2> = Push(42, single)
  let flags :: Vec<Bool, 1> = Push(true, Empty)
  let empty :: Vec<Int, 0> = copy(Empty)
  println("${only(copy(single))} ${first(copy(double))} ${first(copy(flags))} ${only(Push(0, empty))}")
end
