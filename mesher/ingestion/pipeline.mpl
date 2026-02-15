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

# Start the full ingestion pipeline.
# 1. Start RateLimiter
# 2. Start EventProcessor
# 3. Start StorageWriter
# 4. Start PipelineRegistry (stores all PIDs)
# 5. Register PipelineRegistry by name for handler lookup
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

  registry_pid
end
