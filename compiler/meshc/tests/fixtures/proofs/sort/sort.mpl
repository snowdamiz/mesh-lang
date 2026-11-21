pub fn insert(value :: Int, xs :: List<Int>) -> List<Int> do
  case xs do
    [] -> [value]
    head :: tail ->
      if value <= head do
        List.concat([value], xs)
      else
        List.concat([head], insert(value, tail))
      end
  end
end

pub fn sort(xs :: List<Int>) -> List<Int> do
  case xs do
    [] -> []
    head :: tail -> insert(head, sort(tail))
  end
end
