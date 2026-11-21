pub type Tree<T> do
  Leaf(T)
  Branch(Tree<T>, Tree<T>)
end

pub struct Packet<T> do
  tree :: Tree<T>
end

pub fn mirror<T>(tree :: Tree<T>) -> Tree<T> do
  case tree do
    Leaf(value) -> Leaf(value)
    Branch(left, right) -> Branch(mirror(right), mirror(left))
  end
end

pub fn wrap<T>(value :: T) -> Packet<T> do
  Packet { tree: Leaf(value) }
end
