from Policy import retry

test("retry eligibility across HTTP statuses and remaining budgets") do
  for status in 100..600 do
    let eligible = case status do
      429 -> true
      503 -> true
      _ -> false
    end
    assert_eq(retry(status, -1), false)
    assert_eq(retry(status, 0), false)
    assert_eq(retry(status, 1), eligible)
    assert_eq(retry(status, 2), eligible)
    assert_eq(retry(status, 8), eligible)
    0
  end
end
