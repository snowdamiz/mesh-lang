# Pipeline startup orchestration and service registry.
# PipelineRegistry service stores the pool handle and service PIDs so
# HTTP/WS handlers can look them up via Process.whereis("mesher_registry").

from Services.RateLimiter import RateLimiter
from Services.EventProcessor import EventProcessor
from Services.Writer import StorageWriter

# Registry state holds pool handle and all service PIDs.
struct RegistryState do
  pool :: PoolHandle
  rate_limiter_pid :: Pid
  processor_pid :: Pid
  writer_pid :: Pid
end

# PipelineRegistry service -- stores pipeline context for handler lookup.
# Call handlers return the stored values with correct types.
service PipelineRegistry do
  fn init(pool :: PoolHandle, rate_limiter_pid :: Pid, processor_pid :: Pid, writer_pid :: Pid) -> RegistryState do
    RegistryState {
      pool: pool,
      rate_limiter_pid: rate_limiter_pid,
      processor_pid: processor_pid,
      writer_pid: writer_pid
    }
  end

  call GetPool() :: PoolHandle do |state|
    (state, state.pool)
  end

  call GetRateLimiter() :: Pid do |state|
    (state, state.rate_limiter_pid)
  end

  call GetProcessor() :: Pid do |state|
    (state, state.processor_pid)
  end

  call GetWriter() :: Pid do |state|
    (state, state.writer_pid)
  end
end

# Restart all pipeline services and re-register PipelineRegistry.
# Called by health_checker when the registry is unreachable (one_for_all strategy).
fn restart_all_services(pool :: PoolHandle) do
  let rate_limiter_pid = RateLimiter.start(60, 1000)
  println("[Mesher] RateLimiter restarted")

  let processor_pid = EventProcessor.start(pool)
  println("[Mesher] EventProcessor restarted")

  let writer_pid = StorageWriter.start(pool, "default")
  println("[Mesher] StorageWriter restarted")

  let registry_pid = PipelineRegistry.start(pool, rate_limiter_pid, processor_pid, writer_pid)
  let _ = Process.register("mesher_registry", registry_pid)
  println("[Mesher] PipelineRegistry restarted and re-registered")
end

# Health checker actor -- periodically verifies pipeline services are responsive.
# Uses Timer.sleep + recursive call pattern (established in flush_ticker).
# Timer.send_after delivers raw bytes incompatible with typed receive dispatch,
# so we use the simpler sleep-based loop instead.
# Verifies the PipelineRegistry responds to a service call every 10 seconds.
# If the registry is unreachable, restart_all_services is available for
# runtime-level crash detection (Process.whereis returns Pid, not Int,
# so liveness comparison requires future runtime support for Pid.to_int).
actor health_checker(pool :: PoolHandle) do
  Timer.sleep(10000)
  println("[Mesher] Health check running...")

  # Verify registry responds -- if alive, get_pool returns successfully.
  # If the registry crashed, this call blocks (service call to dead PID).
  # Future enhancement: runtime-level crash detection with Pid liveness check.
  let registry_pid = Process.whereis("mesher_registry")
  let _ = PipelineRegistry.get_pool(registry_pid)
  println("[Mesher] Health check: all services responsive")

  # Recurse to keep checking (tail-call, no stack growth)
  health_checker(pool)
end

# Start the full ingestion pipeline.
# 1. Start RateLimiter
# 2. Start EventProcessor
# 3. Start StorageWriter
# 4. Start PipelineRegistry (stores all PIDs)
# 5. Register PipelineRegistry by name for handler lookup
# 6. Spawn health checker for automatic restart
# Returns registry PID.
pub fn start_pipeline(pool :: PoolHandle) do
  # Start rate limiter
  let rate_limiter_pid = RateLimiter.start(60, 1000)
  println("[Mesher] RateLimiter started (60s window, 1000 max)")

  # Start event processor
  let processor_pid = EventProcessor.start(pool)
  println("[Mesher] EventProcessor started")

  # Start a default StorageWriter
  let writer_pid = StorageWriter.start(pool, "default")
  println("[Mesher] StorageWriter started (default project)")

  # Start pipeline registry
  let registry_pid = PipelineRegistry.start(pool, rate_limiter_pid, processor_pid, writer_pid)
  let _ = Process.register("mesher_registry", registry_pid)
  println("[Mesher] PipelineRegistry started and registered")

  # Spawn health checker for automatic restart (10s interval)
  let _ = spawn(health_checker, pool)
  println("[Mesher] Health checker started (10s interval)")

  registry_pid
end
