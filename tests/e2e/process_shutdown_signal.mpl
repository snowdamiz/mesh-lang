fn main() do
  Process.install_shutdown_signals()
  println("${Process.shutdown_requested()}")
  Process.request_shutdown()
  println("${Process.shutdown_requested()}")
end
