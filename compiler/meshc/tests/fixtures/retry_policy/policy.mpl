pub fn retry(status :: Int, left :: Int) -> Bool do
  (status == 503 || status == 429) && left > 0
end
