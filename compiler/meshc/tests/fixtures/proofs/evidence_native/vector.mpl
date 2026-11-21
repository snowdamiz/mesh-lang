logic do
  fn FitsTwo(size :: Nat) -> Prop do size == 2 end

  theorem fits_when_equal(size :: Nat, equal :: size == 2) do
    FitsTwo(size)
  end

  proof fits_when_equal do
    exact equal
  end

  pub theorem allowed_two() do FitsTwo(2) end

  proof allowed_two do
    exact fits_when_equal(2, Eq.refl(2))
  end
end

pub type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end

pub fn deliver<T, n :: Nat>(values :: Vec<T, Succ<n>>, witness :: Proof<FitsTwo<Succ<n>>>) -> T do
  case values do
    Push(head, _) -> head
  end
end
