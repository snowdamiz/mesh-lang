# Mesher monitoring platform entry point.
# Connects to PostgreSQL, creates schema and partitions, starts all services.
# Entity services (Org, Project, User) live in mesher/services/*.mpl.
# StorageWriter batch logic lives here until Plan 02 extracts it.

from Storage.Schema import create_schema, create_partitions_ahead
from Storage.Writer import insert_event
from Services.Org import OrgService
from Services.Project import ProjectService
from Services.User import UserService

# WriterState holds all service state for a single project writer.
# Buffer stores JSON-encoded event strings (List<String>) -- PostgreSQL parses
# the JSON server-side during INSERT (see Storage.Writer.insert_event).
struct WriterState do
  pool :: PoolHandle
  project_id :: String
  buffer :: List<String>
  buffer_len :: Int
  batch_size :: Int
  max_buffer :: Int
end

# --- Batch flush and retry logic ---
# LOCKED: 3 attempts with exponential backoff (100ms, 500ms), drop on final failure.
# Uses recursive loop since Mesh has no mutable variable assignment.

fn flush_loop(pool :: PoolHandle, project_id :: String, events, i :: Int, total :: Int) -> Int!String do
  if i < total do
    let event_json = List.get(events, i)
    let r = insert_event(pool, project_id, event_json)
    case r do
      Ok(_) -> flush_loop(pool, project_id, events, i + 1, total)
      Err(_) -> Err("flush failed")
    end
  else
    Ok(0)
  end
end

fn flush_batch(pool :: PoolHandle, project_id :: String, events) -> Int!String do
  let total = List.length(events)
  flush_loop(pool, project_id, events, 0, total)
end

fn flush_drop(project_id :: String, count_val :: Int) -> Int!String do
  println("[StorageWriter] Dropping batch of " <> String.from(count_val) <> " events for project " <> project_id <> " after 3 retries")
  Ok(0)
end

fn flush_retry3(pool :: PoolHandle, project_id :: String, events, event_count :: Int) -> Int!String do
  Timer.sleep(500)
  let r3 = flush_batch(pool, project_id, events)
  case r3 do
    Ok(n) -> Ok(n)
    Err(_) -> flush_drop(project_id, event_count)
  end
end

fn flush_retry2(pool :: PoolHandle, project_id :: String, events, event_count :: Int) -> Int!String do
  Timer.sleep(100)
  let r2 = flush_batch(pool, project_id, events)
  case r2 do
    Ok(n) -> Ok(n)
    Err(_) -> flush_retry3(pool, project_id, events, event_count)
  end
end

fn flush_with_retry(pool :: PoolHandle, project_id :: String, events) -> Int!String do
  let event_count = List.length(events)
  let r1 = flush_batch(pool, project_id, events)
  case r1 do
    Ok(n) -> Ok(n)
    Err(_) -> flush_retry2(pool, project_id, events, event_count)
  end
end

# --- Buffer management helpers ---
# Kept as standalone functions so cast handler bodies remain minimal
# (avoids complex expressions inside service dispatch codegen).

fn writer_store(state :: WriterState, event_json :: String) -> WriterState do
  let appended = List.append(state.buffer, event_json)
  let new_len = state.buffer_len + 1
  # Drop oldest if over capacity (LOCKED: drop-oldest backpressure)
  let buf = if new_len > state.max_buffer do List.drop(appended, new_len - state.max_buffer) else appended end
  let blen = if new_len > state.max_buffer do state.max_buffer else new_len end
  # Flush if batch size reached (LOCKED: size trigger)
  if blen >= state.batch_size do
    let _ = flush_with_retry(state.pool, state.project_id, buf)
    WriterState { pool: state.pool, project_id: state.project_id, buffer: List.new(), buffer_len: 0, batch_size: state.batch_size, max_buffer: state.max_buffer }
  else
    WriterState { pool: state.pool, project_id: state.project_id, buffer: buf, buffer_len: blen, batch_size: state.batch_size, max_buffer: state.max_buffer }
  end
end

fn writer_flush(state :: WriterState) -> WriterState do
  if state.buffer_len > 0 do
    let _ = flush_with_retry(state.pool, state.project_id, state.buffer)
    WriterState { pool: state.pool, project_id: state.project_id, buffer: List.new(), buffer_len: 0, batch_size: state.batch_size, max_buffer: state.max_buffer }
  else
    state
  end
end

# --- Storage Writer Service ---
# Per-project batch writer with bounded buffer and dual flush triggers.
# LOCKED: per-project writers for isolation, drop-oldest backpressure,
# size + timer flush triggers, retry with exponential backoff.
# insert_event imported from Storage.Writer. All other logic is local
# to avoid cross-module polymorphic type variable scoping issues.
# Buffer stores JSON-encoded event strings; PostgreSQL parses JSON server-side.

service StorageWriter do
  fn init(pool :: PoolHandle, project_id :: String) -> WriterState do
    WriterState {
      pool: pool,
      project_id: project_id,
      buffer: List.new(),
      buffer_len: 0,
      batch_size: 50,
      max_buffer: 500
    }
  end

  # Store a JSON-encoded event string in the buffer.
  # Drops oldest events if buffer exceeds capacity.
  # Triggers immediate flush if buffer reaches batch_size threshold.
  cast Store(event_json) do |state|
    writer_store(state, event_json)
  end

  # Flush all buffered events to PostgreSQL. Called by ticker actor on timer.
  cast Flush() do |state|
    writer_flush(state)
  end
end

# Ticker actor for periodic flush (LOCKED: timer flush trigger).
# Uses Timer.sleep + recursive call because Timer.send_after delivers raw bytes
# that cannot match service cast dispatch tags (type_tag-based dispatch).
# Spawned alongside each StorageWriter to provide the timer-based flush trigger.
actor flush_ticker(writer_pid, interval :: Int) do
  Timer.sleep(interval)
  StorageWriter.flush(writer_pid)
  flush_ticker(writer_pid, interval)
end

# --- Main Entry Point ---

fn start_services(pool :: PoolHandle) do
  # Run schema creation (idempotent -- all CREATE IF NOT EXISTS)
  let schema_result = create_schema(pool)
  case schema_result do
    Ok(_) -> println("[Mesher] Schema created/verified")
    Err(_) -> println("[Mesher] Schema error")
  end

  # Create initial partitions (7 days ahead)
  let partition_result = create_partitions_ahead(pool, 7)
  case partition_result do
    Ok(_) -> println("[Mesher] Partitions created (7 days ahead)")
    Err(_) -> println("[Mesher] Partition error")
  end

  # Start services
  let org_svc = OrgService.start(pool)
  println("[Mesher] OrgService started")

  let project_svc = ProjectService.start(pool)
  println("[Mesher] ProjectService started")

  let user_svc = UserService.start(pool)
  println("[Mesher] UserService started")

  println("[Mesher] Foundation ready")

  # Keep the main process alive (services run as actors).
  # In Phase 88, this will be replaced by HTTP.serve which blocks.
  Timer.sleep(999999999)
end

fn main() do
  println("[Mesher] Connecting to PostgreSQL...")
  let pool_result = Pool.open("postgres://mesh:mesh@localhost:5432/mesher", 2, 10, 5000)
  case pool_result do
    Ok(pool) -> start_services(pool)
    Err(_) -> println("[Mesher] Failed to connect to PostgreSQL")
  end
end
