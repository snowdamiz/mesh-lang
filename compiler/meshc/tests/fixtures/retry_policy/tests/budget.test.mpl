from Policy import retry

test("retry eligibility across HTTP statuses and remaining budgets") do
  for status in 100..600 do
    let eligible = case status do
      429 -> true
      503 -> true
      _ -> false
    end
    for (left, has_budget) in [(-1, false), (0, false), (1, true), (2, true), (8, true)] do
      let input = "#{status}/#{left}"
      assert_eq("#{input}: #{retry(status, left)}", "#{input}: #{eligible && has_budget}")
    end
  end
end
