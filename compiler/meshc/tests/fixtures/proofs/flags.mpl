pub fn drop_false_prefix(xs :: List<Bool>) -> List<Bool> do
  case xs do
    [] -> []
    head :: tail ->
      if head do
        xs
      else
        drop_false_prefix(tail)
      end
  end
end
