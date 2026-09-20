# Service String return E2E test.
# Verifies pointer-valued replies and String state across synchronous calls.
# Expected output: hello\nworld\n

service Greeter do
  fn init(initial :: String) -> String do
    initial
  end

  call Get() :: String do |state|
    (state, state)
  end

  call Put(value :: String) :: String do |_state|
    (value, value)
  end
end

fn main() do
  let pid = Greeter.start("hello")
  println(Greeter.get(pid))
  println(Greeter.put(pid, "world"))
end
