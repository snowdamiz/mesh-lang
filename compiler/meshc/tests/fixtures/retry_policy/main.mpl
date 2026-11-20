from Policy import retry

fn main() do
  println("#{retry(503, 1)}")
  println("#{retry(503, 0)}")
  println("#{retry(429, 1)}")
  println("#{retry(429, 0)}")
  println("#{retry(500, 1)}")
end
