from Trees import Tree, Packet, mirror, wrap

fn main() do
  let ints = mirror(Branch(Leaf([17]), Leaf([23, 31])))
  let flags = mirror(Branch(Leaf([false]), Leaf([true, true, false])))
  let packet = wrap([-4, 8, 15, 16])
  let first = case ints do
    Branch(Leaf(left), Leaf(_)) -> List.length(left)
    _ -> 0
  end
  let second = case flags do
    Branch(Leaf(left), Leaf(_)) -> List.length(left)
    _ -> 0
  end
  let wrapped = case packet.tree do
    Leaf(value) -> List.length(value)
    _ -> 0
  end
  println("${first} ${second} ${wrapped}")
end
