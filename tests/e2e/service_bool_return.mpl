# Service Bool return E2E test.
# Verifies: service calls returning and accepting Bool values.
# Exercises the Bool truncation path (i64 -> i1) in codegen_service_call_helper.
# Also exercises Bool argument passing to a service handler (i64 -> trunc -> i1).
# Expected output: true\ntrue\nfalse\nenabled:true\ndisabled:false\n

service Limiter do
  fn init(remaining :: Int) -> Int do
    remaining
  end

  call Check() :: Bool do |remaining|
    if remaining > 0 do
      (remaining - 1, true)
    else
      (remaining, false)
    end
  end

  call SetEnabled(enabled :: Bool) :: Bool do |remaining|
    (remaining, enabled)
  end
end

fn main() do
  let pid = Limiter.start(2)
  let r1 = Limiter.check(pid)
  if r1 do
    println("true")
  else
    println("false")
  end
  let r2 = Limiter.check(pid)
  if r2 do
    println("true")
  else
    println("false")
  end
  # Third call: no capacity remains, so the result is false.
  let r3 = Limiter.check(pid)
  if r3 do
    println("true")
  else
    println("false")
  end
  # Test Bool argument passing: SetEnabled(true) should return true
  let r4 = Limiter.set_enabled(pid, true)
  if r4 do
    println("enabled:true")
  else
    println("enabled:false")
  end
  # Test Bool argument passing: SetEnabled(false) should return false
  let r5 = Limiter.set_enabled(pid, false)
  if r5 do
    println("disabled:true")
  else
    println("disabled:false")
  end
end
