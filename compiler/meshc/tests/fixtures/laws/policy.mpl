pub fn retry(status :: Int, remaining :: Int) -> Bool do
  (status == 429 || status == 503) && remaining > 0
end
