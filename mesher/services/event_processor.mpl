# EventProcessor service -- routes validated events to StorageWriter.
# Uses synchronous call handler so HTTP handlers get processing results back.
# Validation is done by the caller (HTTP handler) using Ingestion.Validation
# before calling ProcessEvent. This avoids cross-module from_json limitations.

from Services.Writer import StorageWriter

struct ProcessorState do
  pool :: PoolHandle
  processed_count :: Int
end

# Route an event to the StorageWriter and build updated state.
fn route_event(state :: ProcessorState, project_id :: String, writer_pid, event_json :: String) -> (ProcessorState, String!String) do
  StorageWriter.store(writer_pid, event_json)
  let new_state = ProcessorState { pool: state.pool, processed_count: state.processed_count + 1 }
  (new_state, Ok(project_id))
end

service EventProcessor do
  fn init(pool :: PoolHandle) -> ProcessorState do
    ProcessorState { pool: pool, processed_count: 0 }
  end

  # Synchronous event processing: routes pre-validated event to writer.
  # The caller is responsible for JSON parsing and field validation
  # (using Ingestion.Validation) before calling ProcessEvent.
  # Returns Ok(project_id) on success.
  call ProcessEvent(project_id :: String, writer_pid, event_json :: String) do |state|
    route_event(state, project_id, writer_pid, event_json)
  end
end
