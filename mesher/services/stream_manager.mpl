# StreamManager service -- per-connection subscription state for WebSocket streaming clients.
# Tracks which connections are streaming clients, their project association,
# and filter preferences (level, environment) for targeted event delivery.
# Buffer fields (buffer, buffer_len, max_buffer) defined here for backpressure;
# actual BufferMessage/DrainBuffers handlers added in Plan 03.

struct ConnectionState do
  project_id :: String
  level_filter :: String      # "" means no filter (accept all)
  env_filter :: String        # "" means no filter (accept all)
  buffer :: List<String>      # pending messages for slow client
  buffer_len :: Int
  max_buffer :: Int           # drop oldest when exceeded (default 100)
end

struct StreamState do
  connections :: Map<Int, ConnectionState>  # conn_handle -> state
end

# --- Helper functions (outside service block per established pattern) ---

fn register_client(state :: StreamState, conn :: Int, project_id :: String, level_filter :: String, env_filter :: String) -> StreamState do
  let cs = ConnectionState {
    project_id: project_id,
    level_filter: level_filter,
    env_filter: env_filter,
    buffer: List.new(),
    buffer_len: 0,
    max_buffer: 100
  }
  let new_conns = Map.put(state.connections, conn, cs)
  StreamState { connections: new_conns }
end

fn remove_client(state :: StreamState, conn :: Int) -> StreamState do
  let new_conns = Map.delete(state.connections, conn)
  StreamState { connections: new_conns }
end

fn is_stream_client(state :: StreamState, conn :: Int) -> Bool do
  Map.has_key(state.connections, conn)
end

fn get_project_id(state :: StreamState, conn :: Int) -> String do
  let has = Map.has_key(state.connections, conn)
  if has do
    let cs = Map.get(state.connections, conn)
    cs.project_id
  else
    ""
  end
end

# AND helper for filter matching -- avoids && codegen issue inside nested if blocks.
fn both_match(a :: Bool, b :: Bool) -> Bool do
  if a do b else false end
end

fn matches_filter(state :: StreamState, conn :: Int, level :: String, environment :: String) -> Bool do
  let has = Map.has_key(state.connections, conn)
  if has do
    let cs = Map.get(state.connections, conn)
    let level_ok = if cs.level_filter == "" do true else cs.level_filter == level end
    let env_ok = if cs.env_filter == "" do true else cs.env_filter == environment end
    both_match(level_ok, env_ok)
  else
    false
  end
end

# --- StreamManager Service ---
# Per-connection subscription state for WebSocket streaming clients.
# RegisterClient/RemoveClient are casts (fire-and-forget state updates).
# IsStreamClient/GetProjectId/MatchesFilter are calls (synchronous queries).

service StreamManager do
  fn init() -> StreamState do
    StreamState { connections: Map.new() }
  end

  # Register a streaming client connection with project and filter preferences
  cast RegisterClient(conn :: Int, project_id :: String, level_filter :: String, env_filter :: String) do |state|
    register_client(state, conn, project_id, level_filter, env_filter)
  end

  # Remove a connection (called on disconnect)
  cast RemoveClient(conn :: Int) do |state|
    remove_client(state, conn)
  end

  # Check if a connection is a streaming client (vs ingestion client)
  call IsStreamClient(conn :: Int) :: Bool do |state|
    (state, is_stream_client(state, conn))
  end

  # Get the project_id for a streaming client
  call GetProjectId(conn :: Int) :: String do |state|
    (state, get_project_id(state, conn))
  end

  # Check if an event matches a connection's filters
  call MatchesFilter(conn :: Int, level :: String, environment :: String) :: Bool do |state|
    (state, matches_filter(state, conn, level, environment))
  end
end
