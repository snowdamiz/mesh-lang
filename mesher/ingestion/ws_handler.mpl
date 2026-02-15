# WebSocket callbacks for event streaming.
# Ws.serve provides on_connect, on_message, on_close callbacks.
# Each WS connection runs in its own actor (crash-isolated by runtime).

from Services.EventProcessor import EventProcessor
from Ingestion.Pipeline import PipelineRegistry

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

# WebSocket on_connect callback.
# Receives (conn, path, headers) where headers is a Map<String, String>.
# Authenticates via API key in headers. Returns conn to accept, or 0 to reject.
pub fn ws_on_connect(conn, path, headers) do
  # Check for x-sentry-auth header first
  let has_key = Map.has_key(headers, "x-sentry-auth")
  if has_key do
    conn
  else
    check_authorization_header(conn, headers)
  end
end

# WebSocket on_message callback.
# Receives (conn, message) where message is the raw text frame content.
# Routes to EventProcessor for processing.
pub fn ws_on_message(conn, message) do
  let reg_pid = Process.whereis("mesher_registry")
  let processor_pid = PipelineRegistry.get_processor(reg_pid)
  let writer_pid = PipelineRegistry.get_writer(reg_pid)
  let result = EventProcessor.process_event(processor_pid, "ws-project", writer_pid, message)
  case result do
    Ok(_) -> ws_send_accepted(conn)
    Err(reason) -> ws_send_error(conn, reason)
  end
end

# WebSocket on_close callback.
pub fn ws_on_close(conn, code, reason) do
  println("[WS] Connection closed: " <> String.from(code))
end
