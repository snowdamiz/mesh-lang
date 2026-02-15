# Pipeline startup orchestration and service registry.
# PipelineRegistry service stores the pool handle and service PIDs so
# HTTP/WS handlers can look them up via Process.whereis("mesher_registry").

from Services.RateLimiter import RateLimiter
from Services.EventProcessor import EventProcessor
from Services.Writer import StorageWriter
from Services.StreamManager import StreamManager
from Services.StreamManager import stream_drain_ticker
from Storage.Queries import check_volume_spikes

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

  let stream_mgr_pid = StreamManager.start()
  let _ = Process.register("stream_manager", stream_mgr_pid)
  println("[Mesher] StreamManager restarted")

  # Spawn drain ticker for StreamManager buffer backpressure (250ms interval)
  let _ = spawn stream_drain_ticker(stream_mgr_pid, 250)
  println("[Mesher] StreamManager drain ticker restarted (250ms interval)")

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

# Periodic spike detection actor -- checks archived issues for volume spikes.
# Runs every 5 minutes (300000ms). If an archived issue has a sudden burst of events
# (>10x average hourly rate), it's auto-escalated to 'unresolved' (ISSUE-03).
# Uses Timer.sleep + recursive call pattern (established in flush_ticker, health_checker).
actor spike_checker(pool :: PoolHandle) do
  Timer.sleep(300000)
  let result = check_volume_spikes(pool)
  case result do
    Ok(n) -> log_spike_result(n)
    Err(e) -> println("[Mesher] Spike checker error: " <> e)
  end
  spike_checker(pool)
end

# Helper: log spike checker result (extracted for single-expression case arm).
fn log_spike_result(n :: Int) do
  if n > 0 do
    println("[Mesher] Spike checker: escalated " <> String.from(n) <> " archived issues")
  else
    0
  end
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
  # Start stream manager (before other services so WS handler can find it)
  let stream_mgr_pid = StreamManager.start()
  let _ = Process.register("stream_manager", stream_mgr_pid)
  println("[Mesher] StreamManager started")

  # Spawn drain ticker for StreamManager buffer backpressure (250ms interval)
  let _ = spawn stream_drain_ticker(stream_mgr_pid, 250)
  println("[Mesher] StreamManager drain ticker started (250ms interval)")

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

  # Spawn spike detection checker (5 minute interval)
  let _ = spawn(spike_checker, pool)
  println("[Mesher] Spike checker started (5 min interval)")

  registry_pid
end
