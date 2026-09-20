# Tuple.first / Tuple.second / Tuple.nth return the element's own type.
#
# They are declared `-> Int`, all an untyped `Tuple` can promise, so on a
# tuple holding anything else they were a type error ("expected String, found
# Int") or handed back the element's address. A call that knows the tuple's
# type now takes the element's type from it, and decodes the slot the way
# tuples store it: an aggregate of one word or less inline, a larger one boxed.

type Color do
  Red
  Green
end

struct Tag do
  label :: String
end

fn color_name(c :: Color) -> String do
  case c do
    Red -> "red"
    Green -> "green"
  end
end

fn main() do
  let pair = ("name-${1 + 1}", 7)
  println(Tuple.first(pair))
  println("${Tuple.second(pair) + 1}")

  let triple = (1, 2.5, true)
  println("${Tuple.nth(triple, 1)} ${Tuple.nth(triple, 2)}")

  # A payload-free variant sits in its slot inline, a struct in a box.
  let mixed = (Green, Tag { label: "tag-${2 + 3}" })
  let tag = Tuple.second(mixed)
  println("${color_name(Tuple.first(mixed))} ${tag.label}")

  let nested = ((1, "inner-${4 + 4}"), 9)
  let inner = Tuple.first(nested)
  println("${Tuple.second(inner)} ${Tuple.second(nested)}")

  println(pair |> Tuple.first)

  # A tuple of Ints behind an unannotated parameter works as it always did.
  let sums = List.map([(3, 4)], fn (p) do Tuple.first(p) + Tuple.second(p) end)
  println("${List.get(sums, 0)}")
end
