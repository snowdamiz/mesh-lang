# WebSocket callbacks for event streaming.
# Ws.serve provides on_connect, on_message, on_close callbacks.
# Each WS connection runs in its own actor (crash-isolated by runtime).
#
# Dual-purpose WS server on port 8081:
#   /ingest           -> SDK event ingestion (existing behavior)
#   /stream/projects/:id -> dashboard streaming subscription (Phase 90)

from Services.EventProcessor import EventProcessor
from Services.StreamManager import StreamManager
from Ingestion.Pipeline import PipelineRegistry
from Api.Helpers import get_registry

# Helper: check authorization header as fallback
fn check_authorization_header(conn, headers) do
  let has_auth = Map.has_key(headers, "authorization")
  if has_auth do
    conn
  else
    0
  end
end

# Helper: send a text frame to a WS connection (discards send result).
fn ws_write(conn, msg :: String) do
  let _result = Ws.send(conn, msg)
  nil
end

# Helper: send accepted response over WS
fn ws_send_accepted(conn) do
  ws_write(conn, "{\"status\":\"accepted\"}")
end

# Helper: send error response over WS
fn ws_send_error(conn, reason :: String) do
  let msg = "{\"error\":\"" <> reason <> "\"}"
  ws_write(conn, msg)
end

# Helper: handle a /stream/projects/:id connection -- join room and register in StreamManager
fn handle_stream_connect(conn, path :: String) do
  let parts = String.split(path, "/")
  let project_id = List.get(parts, 3)
  let room = "project:" <> project_id
  let _ = Ws.join(conn, room)
  let stream_mgr_pid = Process.whereis("stream_manager")
  StreamManager.register_client(stream_mgr_pid, conn, project_id, "", "")
  conn
end

# Helper: handle an /ingest (or other) connection -- auth check
fn handle_ingest_connect(conn, headers) do
  let has_key = Map.has_key(headers, "x-sentry-auth")
  if has_key do
    conn
  else
    check_authorization_header(conn, headers)
  end
end

# Helper: check if path starts with /stream/projects/
fn is_stream_path(path :: String) -> Bool do
  let parts = String.split(path, "/")
  let len = List.length(parts)
  if len > 3 do
    let seg1 = List.get(parts, 1)
    let seg2 = List.get(parts, 2)
    let s1_ok = seg1 == "stream"
    if s1_ok do
      seg2 == "projects"
    else
      false
    end
  else
    false
  end
end

# WebSocket on_connect callback.
# Receives (conn, path, headers) where headers is a Map<String, String>.
# Routes /stream/projects/:id to room subscription, /ingest to event ingestion auth.
pub fn ws_on_connect(conn, path, headers) do
  let is_stream = is_stream_path(path)
  if is_stream do
    handle_stream_connect(conn, path)
  else
    handle_ingest_connect(conn, headers)
  end
end

# Helper: apply filter update from parsed rows
# Defined before handle_subscribe_update (Mesh requires define-before-use).
fn apply_filter_update(conn, rows) do
  if List.length(rows) > 0 do
    let row = List.head(rows)
    let level = Map.get(row, "level")
    let env = Map.get(row, "env")
    let stream_mgr_pid = Process.whereis("stream_manager")
    let project_id = StreamManager.get_project_id(stream_mgr_pid, conn)
    StreamManager.register_client(stream_mgr_pid, conn, project_id, level, env)
    ws_write(conn, "{\"type\":\"filters_updated\"}")
  else
    ws_write(conn, "{\"type\":\"error\",\"message\":\"filter parse failed\"}")
  end
end

# Helper: update subscription filters from a JSON subscribe message.
# Uses PostgreSQL jsonb extraction (same pattern as handle_assign_issue in routes.mpl).
fn handle_subscribe_update(conn, message :: String) do
  let reg_pid = get_registry()
  let pool = PipelineRegistry.get_pool(reg_pid)
  let query_result = Pool.query(pool, "SELECT COALESCE($1::jsonb->'filters'->>'level', '') AS level, COALESCE($1::jsonb->'filters'->>'environment', '') AS env", [message])
  case query_result do
    Ok(rows) -> apply_filter_update(conn, rows)
    Err(_) -> ws_write(conn, "{\"type\":\"error\",\"message\":\"invalid subscribe message\"}")
  end
end

# Helper: handle message from an ingestion client (existing behavior)
fn handle_ingest_message(conn, message :: String) do
  let reg_pid = get_registry()
  let processor_pid = PipelineRegistry.get_processor(reg_pid)
  let writer_pid = PipelineRegistry.get_writer(reg_pid)
  let result = EventProcessor.process_event(processor_pid, "ws-project", writer_pid, message)
  case result do
    Ok(_) -> ws_send_accepted(conn)
    Err(reason) -> ws_send_error(conn, reason)
  end
end

# WebSocket on_message callback.
# Receives (conn, message) where message is the raw text frame content.
# Routes streaming clients to subscription updates, ingestion clients to EventProcessor.
pub fn ws_on_message(conn, message) do
  let stream_mgr_pid = Process.whereis("stream_manager")
  let is_stream = StreamManager.is_stream_client(stream_mgr_pid, conn)
  if is_stream do
    handle_subscribe_update(conn, message)
  else
    handle_ingest_message(conn, message)
  end
end

# WebSocket on_close callback.
# Cleans up StreamManager state; Ws.join auto-cleanup handles room removal.
pub fn ws_on_close(conn, code, reason) do
  let stream_mgr_pid = Process.whereis("stream_manager")
  StreamManager.remove_client(stream_mgr_pid, conn)
  println("[WS] Connection closed: " <> String.from(code))
end
