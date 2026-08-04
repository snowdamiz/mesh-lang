# Job async/await E2E test.
# Verifies: Job.await selects the requested job when completions arrive out of order.
# Expected output: 1\n2\n

fn main() do
  let slow_job = Job.async(fn () do
    Timer.sleep(100)
    1
  end)
  let fast_job = Job.async(fn () -> 2 end)
  case Job.await(slow_job) do
    Ok( val) -> println("${val}")
    Err( msg) -> println(msg)
  end
  case Job.await(fast_job) do
    Ok( val) -> println("${val}")
    Err( msg) -> println(msg)
  end
end
