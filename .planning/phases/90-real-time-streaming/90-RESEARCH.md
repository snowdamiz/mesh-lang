# Phase 90: Real-Time Streaming - Research

**Researched:** 2026-02-14
**Domain:** WebSocket room-based pub/sub streaming with per-project subscriptions, client-side filtering, issue notifications, and backpressure -- all implemented in the Mesh language using existing runtime infrastructure
**Confidence:** HIGH

## Summary

Phase 90 transforms the existing WebSocket infrastructure from a simple event ingestion endpoint into a real-time streaming platform for dashboard clients. Currently, the WebSocket server on port 8081 accepts events from external clients (SDKs) and routes them into the ingestion pipeline. Phase 90 adds a second WebSocket use case: dashboard clients that subscribe to a project's event stream and receive push notifications for new events, issue state changes, and issue count updates.

The critical discovery from this research is that the Mesh language's WebSocket room primitives (`Ws.join`, `Ws.leave`, `Ws.broadcast`, `Ws.broadcast_except`) exist fully implemented in the Rust runtime (`rooms.rs`) and are wired through the codegen pipeline (intrinsics.rs + lower.rs), but they are **missing from the typechecker** (`infer.rs`). The `ws_mod` HashMap in `stdlib_modules()` only contains `send` and `serve` -- the four room functions were never added during Phase 62 or Phase 88. This means any Mesh code calling `Ws.join(conn, "room")` will fail type-checking with an "unknown function" error. Fixing this is the first prerequisite for Phase 90.

The second major design challenge is **how to hook the streaming into the event processing pipeline**. Currently, when an event arrives via HTTP/WS, it flows: HTTP handler -> EventProcessor.process_event (fingerprint + upsert issue) -> StorageWriter.store (buffer + batch persist). To stream events to dashboard clients, the EventProcessor (or the HTTP handler after receiving the result) must also broadcast the event to the appropriate project room. Similarly, issue state transitions (resolve, archive, unresolve in routes.mpl) must broadcast notifications to the project's issue room. The most natural integration point is after EventProcessor returns successfully -- the HTTP handler has the project_id and the processing result, and can broadcast before returning the HTTP response.

**Primary recommendation:** First add `Ws.join`, `Ws.leave`, `Ws.broadcast`, `Ws.broadcast_except` to the typechecker (`infer.rs`). Then build a subscription protocol where dashboard WS clients send a JSON message `{"subscribe":"project_id","filters":{...}}` to join a room named `project:<project_id>`, with optional filter state tracked per-connection in a service actor. After event processing succeeds, broadcast the event to the project room. After issue state transitions, broadcast issue updates. Implement backpressure via a per-connection bounded buffer service that drops oldest events when full.

## Standard Stack

### Core
| Component | Version/Detail | Purpose | Why Standard |
|-----------|---------------|---------|--------------|
| Mesh language | v8.0 (current) | All application code | Dogfooding -- entire streaming layer in Mesh |
| Ws.join(conn, room) | Runtime: rooms.rs | Subscribe dashboard client to project room | Existing room registry with automatic cleanup on disconnect |
| Ws.leave(conn, room) | Runtime: rooms.rs | Unsubscribe from project room | Existing |
| Ws.broadcast(room, msg) | Runtime: rooms.rs | Push events to all subscribers in a project room | Cluster-aware broadcast (also forwards to remote nodes) |
| Ws.send(conn, msg) | Runtime: server.rs | Direct message to individual connection | For filtered delivery and per-client backpressure |
| service blocks | Built-in | ConnectionState service for per-client filter tracking | GenServer pattern for stateful operations |
| Process.register / Process.whereis | Built-in | Named service lookup for filter state | Established pattern from PipelineRegistry |
| Map<String, String> | Built-in | Filter criteria storage, JSON message construction | Core data structure |

### Supporting
| Component | Detail | Purpose | When to Use |
|-----------|--------|---------|-------------|
| String.split | Built-in | Parse WebSocket path for project_id extraction | e.g., /ws/projects/:project_id |
| String.contains | Needs verification | Check if event matches filter criteria | Filter matching |
| Json.parse | Built-in | Parse subscription messages from dashboard clients | Protocol message handling |
| Json.encode | Built-in | Serialize event/issue notifications | Outbound message construction |
| Timer.sleep + recursive actor | Built-in | Periodic buffer drain for slow clients | Backpressure mechanism |
| List.length | Built-in | Buffer size checking | Drop-oldest backpressure |
| List.drop | Built-in | Remove oldest events from buffer | Backpressure |

### Runtime Extensions Required
| Extension | Location | Purpose | Why Needed |
|-----------|----------|---------|------------|
| Ws.join typechecker entry | infer.rs `ws_mod` | Enable Ws.join(conn, room) to type-check | Missing from Phase 62/88 -- codegen exists but typechecker blocks compilation |
| Ws.leave typechecker entry | infer.rs `ws_mod` | Enable Ws.leave(conn, room) to type-check | Same gap as Ws.join |
| Ws.broadcast typechecker entry | infer.rs `ws_mod` | Enable Ws.broadcast(room, msg) to type-check | Same gap |
| Ws.broadcast_except typechecker entry | infer.rs `ws_mod` | Enable Ws.broadcast_except(room, msg, conn) to type-check | Same gap |

## Architecture Patterns

### Recommended Project Structure
```
mesher/
  ingestion/
    ws_handler.mpl              # MODIFY: add subscription protocol, room management
    routes.mpl                  # MODIFY: add broadcast after event processing and issue transitions
    pipeline.mpl                # MODIFY: register StreamManager service
    ...
  services/
    stream_manager.mpl          # NEW: per-connection filter state, backpressure buffers
    event_processor.mpl         # EXISTING (unchanged)
    writer.mpl                  # EXISTING (unchanged)
    ...
```

### Pattern 1: Typechecker Gap Closure (Ws.join/leave/broadcast/broadcast_except)
**What:** Add four function entries to the `ws_mod` HashMap in `stdlib_modules()` in `infer.rs`
**When to use:** Before any Mesh code can use Ws.join, Ws.leave, Ws.broadcast, Ws.broadcast_except
**Details:**
The codegen pipeline already maps these functions correctly:
- `lower.rs`: `ws_join -> mesh_ws_join`, `ws_leave -> mesh_ws_leave`, `ws_broadcast -> mesh_ws_broadcast`, `ws_broadcast_except -> mesh_ws_broadcast_except`
- `intrinsics.rs`: All four extern declarations exist with correct signatures
- `rooms.rs`: All four runtime functions are implemented and tested

Only the typechecker entries are missing. Based on the runtime signatures:
```rust
// mesh_ws_join(conn: *mut u8, room: *const MeshString) -> i64
// mesh_ws_leave(conn: *mut u8, room: *const MeshString) -> i64
// mesh_ws_broadcast(room: *const MeshString, msg: *const MeshString) -> i64
// mesh_ws_broadcast_except(room: *const MeshString, msg: *const MeshString, except: *mut u8) -> i64
```

The Mesh-level type signatures should be:
```
Ws.join: fn(Int, String) -> Int     # (conn_handle, room_name) -> 0 success
Ws.leave: fn(Int, String) -> Int    # (conn_handle, room_name) -> 0 success
Ws.broadcast: fn(String, String) -> Int  # (room_name, message) -> failure_count
Ws.broadcast_except: fn(String, String, Int) -> Int  # (room, msg, except_conn) -> failure_count
```

Note: `conn` is typed as `Int` at the Mesh level (same convention as `Ws.send`), which is actually a pointer cast to i64. This is consistent with `Ws.send: fn(Int, String) -> Int`.

### Pattern 2: Subscription Protocol via WebSocket Messages
**What:** Dashboard clients send JSON messages to subscribe/unsubscribe to project streams with optional filters
**When to use:** STREAM-01 (project subscription) and STREAM-02 (filtering)
**Example:**
```mesh
# Client -> Server subscription message:
# {"type":"subscribe","project_id":"abc-123","filters":{"level":"error","environment":"production"}}
#
# Client -> Server unsubscribe message:
# {"type":"unsubscribe","project_id":"abc-123"}
#
# Server -> Client event notification:
# {"type":"event","data":{...event fields...}}
#
# Server -> Client issue notification:
# {"type":"issue","action":"new","data":{...issue fields...}}
#
# Server -> Client issue count update:
# {"type":"issue_count","project_id":"abc-123","count":42}

fn handle_subscribe(conn, project_id :: String, filters) do
  # Join the project's event room
  let room = "project:" <> project_id
  let _ = Ws.join(conn, room)

  # Register filters with StreamManager (per-connection state)
  let mgr_pid = Process.whereis("stream_manager")
  StreamManager.set_filters(mgr_pid, conn, project_id, filters)

  ws_write(conn, "{\"type\":\"subscribed\",\"project_id\":\"" <> project_id <> "\"}")
end
```

### Pattern 3: Event Broadcasting After Processing
**What:** After EventProcessor returns Ok(issue_id), broadcast the event to the project's room
**When to use:** STREAM-01 and STREAM-03 (new event and issue notifications)
**Integration point:** In `routes.mpl` after `EventProcessor.process_event` returns successfully
**Example:**
```mesh
# In handle_event (routes.mpl), after successful processing:
fn route_to_processor_with_stream(processor_pid, project_id :: String, writer_pid, body :: String) do
  let result = EventProcessor.process_event(processor_pid, project_id, writer_pid, body)
  case result do
    Ok(issue_id) ->
      # Broadcast event to project room for real-time streaming
      let room = "project:" <> project_id
      let notification = "{\"type\":\"event\",\"issue_id\":\"" <> issue_id <> "\",\"data\":" <> body <> "}"
      let _ = Ws.broadcast(room, notification)
      accepted_response()
    Err(reason) -> bad_request_response(reason)
  end
end
```

### Pattern 4: Issue State Change Broadcasting
**What:** After issue state transitions (resolve, archive, etc.), broadcast to the project room
**When to use:** STREAM-03 and STREAM-04 (issue notifications and count updates)
**Example:**
```mesh
# In handle_resolve_issue, after successful state transition:
pub fn handle_resolve_issue(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let issue_id = Request.param(request, "id")
  let result = resolve_issue(pool, issue_id)
  case result do
    Ok(n) ->
      # Look up issue's project_id for room targeting
      # Broadcast issue state change notification
      broadcast_issue_update(pool, issue_id, "resolved")
      HTTP.response(200, "{\"status\":\"ok\",\"affected\":" <> String.from(n) <> "}")
    Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
  end
end

fn broadcast_issue_update(pool :: PoolHandle, issue_id :: String, action :: String) do
  # Query the issue to get project_id for room targeting
  let rows_result = Pool.query(pool, "SELECT project_id::text FROM issues WHERE id = $1::uuid", [issue_id])
  case rows_result do
    Ok(rows) ->
      if List.length(rows) > 0 do
        let project_id = Map.get(List.head(rows), "project_id")
        let room = "project:" <> project_id
        let msg = "{\"type\":\"issue\",\"action\":\"" <> action <> "\",\"issue_id\":\"" <> issue_id <> "\"}"
        let _ = Ws.broadcast(room, msg)
        0
      else 0 end
    Err(_) -> 0
  end
end
```

### Pattern 5: Per-Connection Backpressure via StreamManager Service
**What:** A service actor tracking per-connection state (filters, buffer) with drop-oldest backpressure
**When to use:** STREAM-05 (backpressure for slow clients)
**Example:**
```mesh
struct ConnectionState do
  project_id :: String
  filters :: Map<String, String>    # level -> "error", environment -> "production"
  buffer :: List<String>            # pending messages for slow client
  buffer_len :: Int
  max_buffer :: Int                 # drop oldest when exceeded
end

struct StreamState do
  connections :: Map<Int, ConnectionState>  # conn_handle -> state
end

service StreamManager do
  fn init() -> StreamState do
    StreamState { connections: Map.new() }
  end

  # Register a new subscription with filters
  cast SetFilters(conn :: Int, project_id :: String, filters :: Map<String, String>) do |state|
    let cs = ConnectionState {
      project_id: project_id,
      filters: filters,
      buffer: List.new(),
      buffer_len: 0,
      max_buffer: 100
    }
    let new_conns = Map.put(state.connections, conn, cs)
    StreamState { connections: new_conns }
  end

  # Check if a message passes a connection's filters
  call MatchesFilter(conn :: Int, level :: String, environment :: String) :: Bool do |state|
    # ... check filters map, return (state, matches)
  end

  # Remove connection on disconnect
  cast RemoveConnection(conn :: Int) do |state|
    let new_conns = Map.remove(state.connections, conn)
    StreamState { connections: new_conns }
  end
end
```

### Pattern 6: Filtered Broadcast (Iterate + Filter + Send)
**What:** Instead of raw Ws.broadcast (which sends to ALL room members), iterate members and send only to those whose filters match
**When to use:** STREAM-02 (filtered streaming)
**Trade-off:** `Ws.broadcast` is simple and fast but cannot filter per-connection. For filtered delivery, the EventProcessor/handler must call StreamManager to check each connection's filters and use `Ws.send` directly.
**Recommended approach:** Use Ws.broadcast for unfiltered notifications (issue updates, counts), and per-connection Ws.send for filtered event streaming.
**Example:**
```mesh
# For event streaming with filters:
# 1. EventProcessor returns Ok(issue_id) with event data
# 2. Handler asks StreamManager for matching connections in the project
# 3. Ws.send to each matching connection individually

# For issue notifications (no filtering needed):
# 1. Broadcast to room "project:<id>" with Ws.broadcast
# 2. All subscribers receive it
```

### Anti-Patterns to Avoid
- **Broadcasting raw event JSON to all subscribers:** Events should be filtered per-connection (STREAM-02). Ws.broadcast sends to everyone in the room. For filtered delivery, use Ws.send to individual connections after checking filters.
- **Blocking the EventProcessor with broadcast logic:** The EventProcessor service is a bottleneck (one actor, synchronous call). Do NOT add broadcast logic inside the service. Instead, broadcast AFTER the call returns, in the HTTP handler.
- **Storing filter state in the WS handler closure:** Mesh closures capture environment at creation time; they cannot accumulate per-connection state. Use a service actor (StreamManager) for mutable per-connection state.
- **Using Ws.broadcast for backpressure-aware delivery:** Ws.broadcast writes directly to each connection's TCP stream. There is no buffering or backpressure. For slow clients, you need a per-connection buffer (in StreamManager) that accumulates messages and drains them with Ws.send on a ticker.
- **Putting project_id in the WS URL path without parsing:** The WS on_connect callback receives `(conn, path, headers)` where `path` is a String like `/ws/projects/abc123`. There is no Request.param equivalent for WS paths -- you must parse the path manually using String.split.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Room pub/sub | Custom connection tracking | Ws.join/Ws.broadcast (rooms.rs) | Already implemented, cluster-aware, auto-cleanup on disconnect |
| Connection cleanup | Manual tracking of connections | Ws.join auto-cleanup via RoomRegistry.cleanup_connection | Runtime calls this automatically when WS actor exits |
| JSON construction | Complex string concatenation for nested objects | Simple flat JSON with string concatenation | Mesh has no Json.set/Json.build; keep notification messages flat |
| Per-connection state | Global mutable variables | StreamManager service actor | Mesh has no mutable variables; service actors thread state |
| Periodic buffer drain | Timer.send_after | Timer.sleep + recursive actor (flush_ticker pattern) | Timer.send_after incompatible with service dispatch tags |

**Key insight:** The room infrastructure from Phase 62 provides automatic connection tracking and cleanup. When a dashboard client disconnects, `cleanup_connection` removes it from all rooms automatically. The main work for Phase 90 is: (1) fixing the typechecker gap, (2) designing the subscription protocol, (3) hooking broadcasts into the event/issue pipelines, and (4) implementing per-connection filtering and backpressure.

## Common Pitfalls

### Pitfall 1: Ws.join/broadcast Not in Typechecker
**What goes wrong:** Any Mesh code calling `Ws.join(conn, "room")` fails at compile time with "unknown function in module Ws"
**Why it happens:** Phase 62 added runtime + codegen wiring but never added entries to `ws_mod` in `infer.rs`. Phase 88 added `Ws.send` and `Ws.serve` to the typechecker but not the room functions.
**How to avoid:** Add `join`, `leave`, `broadcast`, `broadcast_except` entries to `ws_mod` in `stdlib_modules()` in `infer.rs` BEFORE writing any Mesh streaming code.
**Warning signs:** Compilation errors mentioning unknown Ws function.

### Pitfall 2: Broadcasting Inside EventProcessor Service
**What goes wrong:** Adding Ws.broadcast calls inside the EventProcessor service's `route_event` function blocks the single EventProcessor actor on network I/O (writing to potentially many WebSocket connections), creating a bottleneck.
**Why it happens:** It seems logical to broadcast where the event is processed.
**How to avoid:** Broadcast AFTER EventProcessor.process_event returns, in the HTTP handler or in a separate spawned actor. The handler has the project_id and the result. Broadcasting outside the service keeps the service fast.
**Warning signs:** Event processing latency increases proportionally to connected clients.

### Pitfall 3: No Query String Parsing for WS Subscriptions
**What goes wrong:** Trying to use `Request.query(conn, "project_id")` in the WS on_connect callback -- but WS connections don't have Request objects.
**Why it happens:** HTTP requests have query_params; WS upgrade requests expose only path and headers.
**How to avoid:** Use the path for project scoping (e.g., `ws://host:8081/stream/projects/<id>`) and parse it with String.split, OR use the first WS message as a subscription command.
**Warning signs:** Runtime crash or type error trying to access query params on WS connection.

### Pitfall 4: Ws.broadcast Sends to ALL Room Members (No Filtering)
**What goes wrong:** Using Ws.broadcast to push events to a project room delivers events to ALL subscribers, regardless of their filter preferences (level, environment).
**Why it happens:** Ws.broadcast is a simple "send to everyone in room" -- no per-recipient filtering.
**How to avoid:** For filtered event delivery, use StreamManager to check each connection's filters, then Ws.send to matching connections individually. Use Ws.broadcast only for unfiltered messages (issue state changes, count updates).
**Warning signs:** Clients receive events that should be filtered out by their subscription criteria.

### Pitfall 5: Unbounded Message Buffers for Slow Clients
**What goes wrong:** Fast event producers flood slow dashboard clients. Ws.send blocks briefly on the write lock, but if events arrive faster than the client can consume, the actor's message loop falls behind.
**Why it happens:** Ws.send writes directly to the TCP stream. There is no application-level buffering. But the OS TCP send buffer has a finite size, and if the client is slow to read, the write will eventually block.
**How to avoid:** Implement application-level backpressure: buffer messages in StreamManager, cap at max_buffer, drop oldest when full. A ticker actor periodically drains the buffer via Ws.send. If Ws.send returns -1 (error), remove the connection.
**Warning signs:** Memory growth on the server; dashboard clients receive very delayed events.

### Pitfall 6: WS Path Parsing Without String.split
**What goes wrong:** Attempting to extract project_id from a WS path like `/stream/projects/abc123` without proper parsing.
**Why it happens:** WS on_connect provides the raw path string. No built-in path parameter extraction for WS (unlike HTTP's Request.param).
**How to avoid:** Use `String.split(path, "/")` to tokenize the path, then `List.get(parts, N)` to extract the project_id segment. The path will be something like `/stream/projects/<project_id>` where the project_id is at index 3 (after splitting on `/`: ["", "stream", "projects", "<id>"]).
**Warning signs:** Empty or incorrect project_id; room joins fail silently.

### Pitfall 7: Dual-Purpose WebSocket Server Confusion
**What goes wrong:** The existing WS server on port 8081 handles event ingestion (SDKs sending events). Phase 90 adds dashboard streaming (clients receiving events). If both use the same WS endpoint without distinguishing, an SDK connection might accidentally receive stream data.
**Why it happens:** The current ws_on_message handler assumes all WS messages are events to ingest.
**How to avoid:** Two approaches: (A) Use a second WS server on a different port (e.g., 8082 for dashboard streaming), or (B) Distinguish by path -- `/ingest` for SDK clients, `/stream/projects/:id` for dashboard clients. Approach (B) is more practical since Ws.serve already passes the path to on_connect, and on_message can use it to route to different logic.
**Warning signs:** SDK events getting echoed back as stream notifications; dashboard clients having their messages processed as events.

## Code Examples

### Adding Ws.join/leave/broadcast to Typechecker (infer.rs)
```rust
// In stdlib_modules(), after existing ws_mod entries for "send" and "serve":

// Ws.join: fn(Int, String) -> Int
ws_mod.insert("join".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::int(), Ty::string()],
    Ty::int(),
)));
// Ws.leave: fn(Int, String) -> Int
ws_mod.insert("leave".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::int(), Ty::string()],
    Ty::int(),
)));
// Ws.broadcast: fn(String, String) -> Int
ws_mod.insert("broadcast".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::string(), Ty::string()],
    Ty::int(),
)));
// Ws.broadcast_except: fn(String, String, Int) -> Int
ws_mod.insert("broadcast_except".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::string(), Ty::string(), Ty::int()],
    Ty::int(),
)));
```

### Subscription Protocol Handler (ws_handler.mpl)
```mesh
# Handle subscription message from dashboard client.
# Expected JSON: {"type":"subscribe","project_id":"...","filters":{"level":"error"}}
fn handle_subscribe_msg(conn, json_val) do
  let msg_type = Map.get(json_val, "type")
  if msg_type == "subscribe" do
    let project_id = Map.get(json_val, "project_id")
    let room = "project:" <> project_id
    let _ = Ws.join(conn, room)
    ws_write(conn, "{\"type\":\"subscribed\",\"project_id\":\"" <> project_id <> "\"}")
  else
    if msg_type == "unsubscribe" do
      let project_id = Map.get(json_val, "project_id")
      let room = "project:" <> project_id
      let _ = Ws.leave(conn, room)
      ws_write(conn, "{\"type\":\"unsubscribed\"}")
    else
      ws_send_error(conn, "unknown message type")
    end
  end
end
```

### Event Broadcast from HTTP Handler (routes.mpl)
```mesh
# Modified route_to_processor that also broadcasts to streaming clients.
fn route_to_processor(processor_pid, project_id :: String, writer_pid, body :: String) do
  let result = EventProcessor.process_event(processor_pid, project_id, writer_pid, body)
  case result do
    Ok(issue_id) ->
      # Broadcast event notification to project room
      let room = "project:" <> project_id
      let _ = Ws.broadcast(room, "{\"type\":\"event\",\"issue_id\":\"" <> issue_id <> "\",\"data\":" <> body <> "}")
      accepted_response()
    Err(reason) -> bad_request_response(reason)
  end
end
```

### Issue Count Update Query and Broadcast
```mesh
# Query current issue counts and broadcast to project room.
fn broadcast_issue_count(pool :: PoolHandle, project_id :: String) do
  let rows_result = Pool.query(pool, "SELECT count(*)::text AS cnt FROM issues WHERE project_id = $1::uuid AND status = 'unresolved'", [project_id])
  case rows_result do
    Ok(rows) ->
      if List.length(rows) > 0 do
        let count = Map.get(List.head(rows), "cnt")
        let room = "project:" <> project_id
        let _ = Ws.broadcast(room, "{\"type\":\"issue_count\",\"project_id\":\"" <> project_id <> "\",\"count\":" <> count <> "}")
        0
      else 0 end
    Err(_) -> 0
  end
end
```

### Drop-Oldest Backpressure in StreamManager
```mesh
# Buffer a message for a slow client with drop-oldest backpressure.
fn buffer_message(cs :: ConnectionState, msg :: String) -> ConnectionState do
  let appended = List.append(cs.buffer, msg)
  let new_len = cs.buffer_len + 1
  # Drop oldest if over capacity (same pattern as StorageWriter)
  let buf = if new_len > cs.max_buffer do List.drop(appended, new_len - cs.max_buffer) else appended end
  let blen = if new_len > cs.max_buffer do cs.max_buffer else new_len end
  ConnectionState { project_id: cs.project_id, filters: cs.filters, buffer: buf, buffer_len: blen, max_buffer: cs.max_buffer }
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| WS server for event ingestion only | WS server for ingestion + streaming | Phase 90 | Dashboard clients get real-time updates |
| No room functions in typechecker | Ws.join/leave/broadcast/broadcast_except usable from Mesh | Phase 90 | Completes the Phase 62 API surface at the language level |
| HTTP polling for issue updates | WebSocket push notifications | Phase 90 | Instant dashboard updates instead of polling interval |
| No client filtering | Per-connection filter state in StreamManager | Phase 90 | Clients receive only matching events |

**Current state (pre-Phase 90):**
- WebSocket server runs on port 8081 with on_connect, on_message, on_close callbacks
- on_message routes all messages to EventProcessor (ingestion only)
- Room runtime functions exist (rooms.rs) but cannot be called from Mesh code
- No streaming, no filtering, no backpressure
- Issue state transitions return HTTP responses but do not notify connected clients

## Open Questions

1. **Separate WS server vs shared with path routing**
   - What we know: The existing WS server on port 8081 handles ingestion. Phase 90 adds streaming.
   - What's unclear: Should streaming use a second Ws.serve on port 8082, or share port 8081 with path-based routing?
   - Recommendation: Share port 8081 with path-based routing. The on_connect callback already receives the path. Use `/ingest` for SDK clients (existing behavior) and `/stream/projects/:id` for dashboard clients. This avoids a second accept loop and simplifies deployment. The on_message handler checks which path was used (can store in connection state via StreamManager) to route to the correct logic.

2. **Filtered broadcast: Ws.broadcast vs per-connection Ws.send**
   - What we know: Ws.broadcast sends to ALL room members. STREAM-02 requires per-client filtering.
   - What's unclear: Is the overhead of per-connection Ws.send (iterating + checking filters + sending individually) acceptable?
   - Recommendation: Hybrid approach. Use Ws.broadcast for unfiltered messages (issue state changes, issue count updates -- STREAM-03, STREAM-04). For event streaming (STREAM-01, STREAM-02), use StreamManager to track filters per-connection and Ws.send to matching connections. This keeps issue notifications simple and fast while supporting filtered event delivery. If no filters are set, Ws.broadcast can be used for events too.

3. **Backpressure granularity: application-level vs OS TCP buffer**
   - What we know: Ws.send writes directly to the TCP stream. OS TCP buffers provide some inherent backpressure. STREAM-05 requires dropping old events for slow clients.
   - What's unclear: Is application-level buffering actually needed, or does OS TCP buffer backpressure suffice? If Ws.send blocks on a full TCP buffer, the actor message loop stalls and subsequent events pile up.
   - Recommendation: Application-level buffering via StreamManager. Even though OS TCP buffers provide some backpressure, relying on them means the write call blocks, stalling the actor that is trying to send. A per-connection buffer in StreamManager with drop-oldest semantics ensures the event handler never blocks. A ticker actor drains buffers periodically. This is the same pattern used by StorageWriter for database writes.

4. **How to store connection path/type for routing**
   - What we know: on_connect receives `(conn, path, headers)`. on_message receives `(conn, message)`. The connection handle (`conn`) is an integer. There is no built-in way to associate metadata with a connection handle.
   - What's unclear: How does on_message know whether this connection is an ingestion client or a streaming client?
   - Recommendation: Register connection metadata in StreamManager during on_connect. When on_connect receives a `/stream/...` path, call `StreamManager.register_stream_client(conn, project_id)`. When on_message fires, check StreamManager to determine if this is a stream client or an ingestion client. If not registered as a stream client, treat the message as event ingestion (existing behavior).

5. **Issue count update timing**
   - What we know: STREAM-04 requires real-time issue count updates.
   - What's unclear: Should counts be broadcast after every event (potentially noisy) or periodically?
   - Recommendation: Broadcast issue count updates after each event processing batch (when StorageWriter flushes) rather than per-event. This reduces noise. Alternatively, broadcast on issue state transitions (resolve/archive/etc.) and when a new issue is created (first occurrence of a fingerprint). The upsert_issue query's RETURNING clause already tells us whether the issue was new or existing.

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/ws/rooms.rs` -- RoomRegistry, global_room_registry, mesh_ws_join, mesh_ws_leave, mesh_ws_broadcast, mesh_ws_broadcast_except, local_room_broadcast, cleanup_connection
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/ws/server.rs` -- WsConnection, ws_connection_entry, call_on_connect (receives path + headers), actor_message_loop, mesh_ws_send, mesh_ws_serve
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/ws/handshake.rs` -- perform_upgrade returns (path, headers), path includes URL path from GET request
- `/Users/sn0w/Documents/dev/snow/crates/mesh-typeck/src/infer.rs` -- ws_mod HashMap (lines 864-881): only `send` and `serve` present; `join`, `leave`, `broadcast`, `broadcast_except` MISSING
- `/Users/sn0w/Documents/dev/snow/crates/mesh-codegen/src/mir/lower.rs` -- ws_join/ws_leave/ws_broadcast/ws_broadcast_except mapped to mesh_ws_* functions (lines 9867-9870), known_functions entries with MirType::Ptr (lines 734-741)
- `/Users/sn0w/Documents/dev/snow/crates/mesh-codegen/src/codegen/intrinsics.rs` -- All four room function LLVM declarations exist (lines 443-453)
- `/Users/sn0w/Documents/dev/snow/mesher/ingestion/ws_handler.mpl` -- Current WS handler: on_connect checks auth headers, on_message routes to EventProcessor
- `/Users/sn0w/Documents/dev/snow/mesher/ingestion/routes.mpl` -- HTTP route handlers for event ingestion and issue management
- `/Users/sn0w/Documents/dev/snow/mesher/ingestion/pipeline.mpl` -- PipelineRegistry service, health_checker, spike_checker actors
- `/Users/sn0w/Documents/dev/snow/mesher/services/event_processor.mpl` -- EventProcessor.process_event: fingerprint + upsert + store
- `/Users/sn0w/Documents/dev/snow/mesher/services/writer.mpl` -- StorageWriter with drop-oldest backpressure pattern
- `/Users/sn0w/Documents/dev/snow/.planning/phases/62-rooms-channels/62-02-SUMMARY.md` -- Confirms codegen wiring done in Phase 62 but typechecker not updated
- `/Users/sn0w/Documents/dev/snow/.planning/phases/88-ingestion-pipeline/88-04-PLAN.md` -- Ws.serve/Ws.send added to typechecker in Phase 88

### Secondary (MEDIUM confidence)
- `/Users/sn0w/Documents/dev/snow/.planning/phases/88-ingestion-pipeline/88-RESEARCH.md` -- Event ingestion architecture, pipeline patterns, middleware constraints
- `/Users/sn0w/Documents/dev/snow/.planning/phases/89-error-grouping-issue-lifecycle/89-RESEARCH.md` -- EventProcessor enrichment pipeline, issue upsert pattern

### Tertiary (LOW confidence)
- None -- all findings verified against actual codebase

## Metadata

**Confidence breakdown:**
- Typechecker gap: HIGH -- verified by reading infer.rs ws_mod (lines 864-881) and confirming only `send`/`serve` entries exist; cross-referenced with codegen which HAS the entries
- Architecture: HIGH -- patterns derived from existing working Phase 88/89 code and Phase 62 runtime implementation
- Subscription protocol: HIGH -- standard WebSocket pub/sub pattern adapted for Mesh constraints
- Backpressure: HIGH -- same drop-oldest pattern as StorageWriter, proven in Phase 87
- Filtering: MEDIUM -- hybrid Ws.broadcast/Ws.send approach is sound but implementation details depend on how efficiently StreamManager can check filters per-connection

**Research date:** 2026-02-14
**Valid until:** 2026-03-14 (stable -- Mesh runtime changes are controlled by this project)
