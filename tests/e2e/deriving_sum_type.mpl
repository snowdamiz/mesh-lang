type Color do
  Red
  Green
  Blue
end deriving(Eq, Ord, Display, Debug, Hash)

# Variants that carry values derive the same protocols.
type Shape do
  Circle(Float)
  Rect(Int, Int)
  Named(String)
  Empty
end deriving(Eq, Display, Debug)

fn main() do
  let r = Red
  let g = Green
  let b = Blue
  println("${r}")
  println("${g}")
  println("${b}")
  println("${r == r}")
  println("${r == g}")
  let named = Named("box-${1 + 1}")
  println("${Circle(1.5)} ${Rect(2, 3)} ${named} ${Empty}")
  println("${Rect(2, 3) == Rect(2, 3)} ${Rect(2, 3) == Rect(2, 4)} ${named == Named("box-2")}")
end
