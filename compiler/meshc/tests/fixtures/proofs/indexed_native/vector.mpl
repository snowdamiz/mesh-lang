pub type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end

pub fn first<T, n :: Nat>(values :: Vec<T, Succ<n>>) -> T do
  case values do
    Push(head, _) -> head
  end
end

pub fn only(values :: Vec<Int, 1>) -> Int = first(values)

pub fn copy<T, n :: Nat>(values :: Vec<T, n>) -> Vec<T, n> do
  case values do
    Empty -> Empty
    Push(head, tail) -> Push(head, copy(tail))
  end
end
