# Architecture Patterns

**Domain:** WebSocket support for Snow programming language -- server-side ws:// and wss://, actor-per-connection, unified mailbox messaging, rooms/channels, heartbeat
**Researched:** 2026-02-12
**Confidence:** HIGH (based on direct codebase inspection of all snow-rt modules, RFC 6455 protocol specification, and established patterns from HTTP server + actor runtime)

## Current Architecture Summary

The Snow runtime (`snow-rt`) is a static library linked into compiled Snow binaries. It exposes `extern "C"` functions that LLVM-generated code calls directly. New features integrate at up to four layers:

1. **typeck builtins** (`snow-typeck/src/builtins.rs`) -- Snow-level type signatures
2. **MIR lowering** (`snow-codegen/src/mir/lower.rs`) -- name mapping + known function registration
3. **LLVM intrinsics** (`snow-codegen/src/codegen/intrinsics.rs`) -- LLVM function declarations
4. **Runtime implementation** (`snow-rt/src/`) -- actual Rust code

The existing pattern for adding new runtime functionality is well-established: SQLite, PostgreSQL, HTTP server, HTTP client, JSON, connection pooling, and transactions all follow the same four-layer integration path.

### Existing Architecture Relevant to WebSocket

**HTTP Server (`http/server.rs`):**
- Accept loop on `TcpListener::incoming()` dispatches each connection to a lightweight actor
- `HttpStream` enum wraps `TcpStream` (Plain) or `StreamOwned<ServerConnection, TcpStream>` (TLS)
- `HttpStream` implements `Read + Write` for transparent plain/TLS dispatch
- Hand-rolled HTTP/1.1 parser: reads request line, headers, body from `BufReader<&mut HttpStream>`
- `connection_handler_entry` is the actor entry function: parse request, process via router, write response, actor exits
- `ConnectionArgs` struct transfers router address + boxed stream via raw pointer to the actor
- Each connection actor runs with `catch_unwind` for crash isolation
- TLS handshake is lazy (happens on first read inside the actor, not in accept loop)
- `Connection: close` -- no HTTP keep-alive, one request per actor

**Actor Runtime (`actor/mod.rs`, `actor/scheduler.rs`):**
- `snow_actor_spawn(fn_ptr, args, args_size, priority)` -> PID
- `snow_actor_send(target_pid, msg_ptr, msg_size)` -- deep-copies message into target mailbox
- `snow_actor_receive(timeout_ms)` -- blocks actor (yields to scheduler), returns message pointer
- Messages in mailbox have layout: `[u64 type_tag][u64 data_len][u8... data]`
- `snow_actor_register(name_ptr, name_len)` / `snow_actor_whereis(name_ptr, name_len)` for named processes
- `snow_actor_link(target_pid)` for bidirectional links, exit signal propagation
- `snow_timer_send_after(pid, ms, msg_ptr, msg_size)` for delayed messages (used for heartbeat)
- Coroutines are `!Send` -- thread-pinned, resumed by the same worker thread

**GC-Safe Handle Pattern (`db/pg.rs`, `db/pool.rs`):**
- External resources stored as `Box::into_raw` cast to `u64` handle
- GC never traces integer values, so handles survive garbage collection
- Callers pass handles (u64) through Snow code; runtime functions cast back to `*mut T`

**Stream Abstraction Pattern (used twice already):**
- `PgStream { Plain(TcpStream), Tls(StreamOwned<ClientConnection, TcpStream>) }` in `db/pg.rs`
- `HttpStream { Plain(TcpStream), Tls(StreamOwned<ServerConnection, TcpStream>) }` in `http/server.rs`
- Both implement `Read + Write`, enabling transparent TLS dispatch

---

## Recommended Architecture

### High-Level Integration Map

```
FEATURE              RUNTIME (snow-rt)             COMPILER              SNOW API
=======              =============                 ========              ========

WS Accept Loop       http/ws.rs (NEW)              intrinsics.rs         Ws.serve(port, handler)
                     WsListener, accept loop       builtins.rs           Ws.serve_tls(port, handler, cert, key)
                     Spawns actor per connection    lower.rs

WS Connection        http/ws.rs (NEW)              intrinsics.rs         Ws.send(conn, msg)
                     WsConn handle (u64)           builtins.rs           Ws.send_binary(conn, data)
                     Frame read/write, masking      lower.rs             Ws.close(conn)

WS Mailbox Bridge    http/ws.rs (NEW)              intrinsics.rs         receive { ... }
                     Reader thread -> send()       builtins.rs           (WS frames arrive as actor messages)
                     Unified with actor receive     lower.rs

WS Rooms             http/ws.rs (NEW)              intrinsics.rs         Ws.join(conn, room)
                     Global room registry          builtins.rs           Ws.leave(conn, room)
                     Broadcast via send_all         lower.rs             Ws.broadcast(room, msg)

WS Heartbeat         http/ws.rs (NEW)              intrinsics.rs         (automatic, configurable)
                     Timer.send_after loop         builtins.rs           Ws.set_ping_interval(conn, ms)
                     Auto-Pong on Ping frames       lower.rs
```

### Component Boundaries

| Component | Responsibility | Communicates With | New/Modified |
|-----------|---------------|-------------------|--------------|
| `http/ws.rs` | WebSocket server, frame I/O, connection lifecycle, rooms, heartbeat | `http/server.rs` (shares `HttpStream` type), actor scheduler, `string.rs`, `gc.rs` | **NEW** |
| `http/mod.rs` | Re-export new WS functions | `http/ws.rs` | **MODIFY** (add `pub mod ws;` + re-exports) |
| `codegen/intrinsics.rs` | LLVM declarations for `snow_ws_*` functions | LLVM IR module | **MODIFY** (add ~15 function declarations) |
| `typeck/builtins.rs` | Type signatures for `Ws.*` module functions | typeck | **MODIFY** (add Ws module + function signatures) |
| `mir/lower.rs` | Map `Ws.*` calls to runtime function names | codegen pipeline | **MODIFY** (add known function entries) |

**Key observation: NO existing files need structural changes.** The HTTP server, actor runtime, and scheduler are untouched. WebSocket support is entirely additive -- a new module (`http/ws.rs`) plus compiler registration of new builtin functions. This follows the exact pattern used for SQLite, PostgreSQL, and connection pooling.

---

## Detailed Component Design

### 1. WsStream -- Stream Abstraction

Reuse the proven `HttpStream` pattern for WebSocket connections.

```
enum WsStream {
    Plain(TcpStream),
    Tls(StreamOwned<ServerConnection, TcpStream>),
}

impl Read for WsStream { ... }
impl Write for WsStream { ... }
```

**Rationale:** The `HttpStream` and `PgStream` patterns are proven in this codebase. `WsStream` is identical in structure. It could even reuse `HttpStream` directly, but a separate type avoids coupling WebSocket internals to HTTP server internals. The types are small enough (~10 lines each) that duplication is preferable to coupling.

### 2. WebSocket Upgrade Handshake

The upgrade handshake validates an HTTP/1.1 GET request and returns HTTP 101.

**Data flow:**
```
Client                    Snow Runtime (inside WS connection actor)
  |                              |
  | -- GET / HTTP/1.1 ---------> |  parse_request() (reuse existing HTTP parser)
  | Upgrade: websocket           |  validate_ws_upgrade():
  | Connection: Upgrade          |    - check Upgrade: websocket
  | Sec-WebSocket-Key: ...       |    - check Connection: Upgrade
  | Sec-WebSocket-Version: 13    |    - extract Sec-WebSocket-Key
  |                              |    - compute SHA-1(key + GUID), base64 encode
  | <--- 101 Switching --------- |  write_101_response():
  | Upgrade: websocket           |    - write HTTP/1.1 101 Switching Protocols
  | Connection: Upgrade          |    - write Sec-WebSocket-Accept header
  | Sec-WebSocket-Accept: ...    |
  |                              |
  | <==== WebSocket Frames ====> |  enter frame_loop()
```

**Implementation detail:** The existing `parse_request()` function in `http/server.rs` already parses HTTP/1.1 requests (method, path, headers). The WS handshake reuses this parser to read the initial upgrade request. The path from the upgrade request is available to the Snow handler as context.

**Sec-WebSocket-Accept computation (RFC 6455 Section 4.2.2):**
```
accept = base64_encode(SHA-1(Sec-WebSocket-Key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))
```

**New dependency:** `sha1` crate (pure Rust, ~200 lines). `base64` is already a direct dependency of `snow-rt` (used by PostgreSQL SCRAM-SHA-256 auth).

### 3. WebSocket Frame Codec (RFC 6455 Section 5)

**Frame wire format:**
```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-------+-+-------------+-------------------------------+
|F|R|R|R| opcode|M| Payload len |    Extended payload length    |
|I|S|S|S|  (4)  |A|     (7)     |             (16/64)           |
|N|V|V|V|       |S|             |   (if payload len == 126/127) |
| |1|2|3|       |K|             |                               |
+-+-+-+-+-------+-+-------------+-------------------------------+
|                     Masking-key (if MASK set)                  |
+-------------------------------+-------------------------------+
|                         Payload Data                          |
+---------------------------------------------------------------+
```

**Opcodes:**
- `0x1` -- Text frame
- `0x2` -- Binary frame
- `0x8` -- Close
- `0x9` -- Ping
- `0xA` -- Pong
- `0x0` -- Continuation (fragmented messages)

**Key rules:**
- Server-to-client frames are NEVER masked
- Client-to-server frames are ALWAYS masked (32-bit key, XOR each byte)
- Payload length: 7-bit (0-125), 16-bit (126 prefix), 64-bit (127 prefix)

**Implementation:**

```rust
struct WsFrame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

fn read_frame(stream: &mut WsStream) -> Result<WsFrame, String> {
    // Read 2-byte header
    // Decode payload length (7/16/64 bit)
    // Read masking key (4 bytes, always present from client)
    // Read payload bytes
    // XOR-unmask payload
    // Return frame
}

fn write_frame(stream: &mut WsStream, opcode: u8, payload: &[u8]) -> Result<(), String> {
    // Set FIN bit (no fragmentation for server-sent frames)
    // MASK bit = 0 (server never masks)
    // Encode payload length (7/16/64 bit)
    // Write header + payload
    // Flush
}
```

**Message reassembly:** For fragmented messages (FIN=0 + continuation frames), buffer fragments until FIN=1. Reassembled message has the opcode of the first fragment.

### 4. Actor-per-Connection with Reader Thread Bridge

This is the most architecturally significant design decision. WebSocket connections are long-lived and bidirectional, unlike HTTP's request-response-close model.

**The Problem:** Each WebSocket connection needs to simultaneously:
1. Read incoming frames from the client (blocking I/O on `WsStream`)
2. Receive messages from other Snow actors (via mailbox)
3. Send outgoing frames to the client

The actor runtime uses `snow_actor_receive()` which yields the coroutine. But if the actor is yielded waiting for a mailbox message, it cannot also be blocking on `stream.read()` for incoming WebSocket frames. The actor can only block on one thing at a time.

**The Solution: Reader Thread Bridge**

```
                     +------------------+
                     |  Snow Actor      |
                     |  (WS Handler)    |
                     |                  |
                     |  receive {       |      <--- unified mailbox
                     |    :text(msg) -> |
                     |    :binary(d) -> |
                     |    :ping ->      |
                     |    :close ->     |
                     |    other_msg ->  |      <--- regular actor msgs
                     |  }               |
                     +--------+---------+
                              |
                     mailbox  |  snow_actor_send()
                              |
              +---------------+----------------+
              |                                |
    +---------+--------+           +-----------+---------+
    |  Reader Thread   |           |  Other Snow Actors  |
    |  (per connection)|           |  (app logic, rooms) |
    |                  |           |                     |
    |  loop {          |           |  send(ws_pid, msg)  |
    |    read_frame()  |           +---------------------+
    |    send(pid, frame)
    |  }               |
    +------------------+
```

**How it works:**

1. The accept loop spawns a Snow actor for each connection (same as HTTP)
2. Inside the connection actor, the runtime spawns a background OS thread (not an actor) that owns the read half of the `WsStream`
3. The reader thread runs `read_frame()` in a blocking loop
4. Each received frame is converted to an actor message and sent to the connection actor's mailbox via `snow_actor_send()`
5. The connection actor uses `snow_actor_receive()` (the standard actor receive) to get both WS frames AND regular actor messages in a single unified mailbox
6. The connection actor owns the write half of the `WsStream` for sending frames

**Why a reader thread, not a reader actor:**
- Actors use coroutines with 64 KiB stacks on the M:N scheduler
- Blocking I/O in an actor is acceptable for short operations (HTTP request, DB query)
- But a WebSocket reader blocks indefinitely (connection lifetime could be hours/days)
- A permanently-blocked actor wastes a scheduler slot and can cause starvation
- A background OS thread is the correct primitive for "block forever on I/O" -- same pattern as `snow_timer_send_after` which spawns an OS thread

**Stream splitting:** The `WsStream` cannot be shared between the reader thread and the actor (both need `&mut` for read/write). Solution: after the upgrade handshake completes, extract the inner `TcpStream` and use `TcpStream::try_clone()` to create separate read/write halves. The reader thread gets one clone, the actor gets the other. For TLS streams, `StreamOwned` does not support cloning -- instead, use the file descriptor directly: `TcpStream::try_clone()` works at the OS level (duplicates the fd), and TLS can be layered on each half independently. Alternatively (and simpler): keep TLS at the outer level and clone the underlying `TcpStream` before TLS wrapping, giving each side its own `StreamOwned`.

**Simpler alternative for TLS:** Since the TLS handshake completes during the HTTP upgrade phase (which happens synchronously before splitting), the simplest approach is:

```rust
// After upgrade handshake completes on the full WsStream:
match ws_stream {
    WsStream::Plain(tcp) => {
        let read_tcp = tcp.try_clone()?;
        // reader_thread gets read_tcp
        // actor keeps tcp for writing
    }
    WsStream::Tls(tls_stream) => {
        // TLS is trickier -- StreamOwned<ServerConnection, TcpStream> cannot be split
        // Option A: Use a Mutex<WsStream> shared between reader thread and actor
        // Option B: Downgrade to fd-level and re-wrap
        // Option C: Single-threaded approach with non-blocking read + timeout
    }
}
```

**Recommended approach for TLS WebSocket:** Use a `Mutex<WsStream>` shared between the reader thread and the connection actor. The reader thread locks to read a frame, then unlocks. The actor locks to write a frame, then unlocks. Since reads and writes don't overlap (reader thread blocks on read, actor only writes when it has a message to send), contention is minimal. This is simpler than splitting the TLS stream and follows a pattern the codebase already uses (e.g., `parking_lot::Mutex` on process state).

**Alternative simpler approach:** For the initial implementation, avoid TLS stream splitting entirely. The reader thread can use `set_read_timeout` on the underlying TcpStream to periodically check a shutdown flag. For plain TCP, `try_clone()` works cleanly. For TLS, the Mutex approach works. Both patterns are well-tested in the Rust ecosystem.

### 5. WsConn Handle -- GC-Safe Connection State

```rust
struct WsConn {
    /// Write half of the stream (Plain or TLS).
    writer: Mutex<WsStream>,
    /// PID of the connection actor (for sending messages from rooms/broadcast).
    actor_pid: u64,
    /// Shutdown flag checked by the reader thread.
    shutdown: AtomicBool,
    /// Path from the upgrade request (available to handler).
    path: String,
}
```

Exposed to Snow code as an opaque `u64` handle (same pattern as `PgConn`, `SqliteConn`, `PgPool`).

### 6. Room/Channel Registry

Rooms are a global registry mapping room names to sets of connection PIDs.

```rust
static WS_ROOMS: OnceLock<RwLock<HashMap<String, HashSet<u64>>>> = OnceLock::new();

fn ws_join(conn_handle: u64, room_name: &str) {
    // Add conn's actor PID to the room's PID set
}

fn ws_leave(conn_handle: u64, room_name: &str) {
    // Remove conn's actor PID from the room's PID set
}

fn ws_broadcast(room_name: &str, msg: &[u8]) {
    // For each PID in the room, send the message via snow_actor_send()
    // The connection actor receives it, then calls write_frame() to push to client
}
```

**Why a global registry, not an actor:** Rooms are a lookup table, not a process. Using a `RwLock<HashMap>` is simpler and faster than routing room join/leave/broadcast through a centralized actor. The actor-per-connection model means each connection actor handles its own I/O -- the room registry just provides the fan-out list.

**Cleanup on disconnect:** When a connection actor exits (client disconnects or error), its terminate callback removes the actor PID from all rooms. This uses the existing `snow_actor_set_terminate` infrastructure.

### 7. Heartbeat / Ping-Pong

**Auto-Pong:** When the frame reader receives a Ping frame (opcode 0x9), it immediately writes back a Pong frame (opcode 0xA) with the same payload. This happens in the reader thread BEFORE forwarding to the actor mailbox. Rationale: Pong responses must be timely (RFC 6455 Section 5.5.2). Routing through the actor mailbox adds latency.

**Server-initiated Ping:** Configurable ping interval (default: 30 seconds). Uses `snow_timer_send_after` to send a `:ping_tick` message to the connection actor periodically. The actor writes a Ping frame to the client. If no Pong is received within a timeout, the connection is closed.

```
Timer thread                Connection Actor
    |                             |
    | -- :ping_tick (every 30s) -> |
    |                             | write_frame(Ping)
    |                             |
    |        Reader Thread        |
    |             |               |
    |             | -- Pong -----> | (auto-forwarded as :pong message)
    |             |               | reset pong_received flag
    |                             |
    | -- :ping_tick -----------> | if !pong_received: close connection
```

### 8. Message Type Tags for WS Frames in Mailbox

WebSocket frames are delivered to the connection actor's mailbox using specific type tags so they can be distinguished from regular actor messages.

```rust
// Type tags for WS frame messages in actor mailbox
const WS_TEXT_TAG: u64    = 0x5753_5445_5854_0001; // "WSTEXT" + 01
const WS_BINARY_TAG: u64  = 0x5753_4249_4E00_0002; // "WSBIN" + 02
const WS_CLOSE_TAG: u64   = 0x5753_434C_4F53_0003; // "WSCLOS" + 03
const WS_PING_TAG: u64    = 0x5753_5049_4E47_0004; // "WSPING" + 04
const WS_PONG_TAG: u64    = 0x5753_504F_4E47_0005; // "WSPONG" + 05
const WS_PING_TICK_TAG: u64 = 0x5753_5449_434B_0006; // "WSTICK" + 06
```

The connection actor's receive loop pattern-matches on these tags to distinguish WebSocket events from application-level messages.

**Snow-level API:**
```snow
fn handle_connection(conn) do
  loop do
    receive do
      {:ws_text, message} ->
        # Handle text message from client
        Ws.send(conn, "echo: " <> message)

      {:ws_binary, data} ->
        # Handle binary message from client

      {:ws_close, _} ->
        # Client sent Close frame
        break

      {:broadcast, room_msg} ->
        # Message from another actor (e.g., room broadcast)
        Ws.send(conn, room_msg)
    end
  end
end
```

### 9. Accept Loop Architecture

```rust
#[no_mangle]
pub extern "C" fn snow_ws_serve(handler_fn: *mut u8, port: i64) {
    crate::actor::snow_rt_init_actor(0);

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).expect("bind failed");

    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream { Ok(s) => s, Err(_) => continue };
        tcp_stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        let ws_stream = WsStream::Plain(tcp_stream);

        // Pack handler_fn + stream into ConnectionArgs
        let args = WsConnectionArgs {
            handler_fn: handler_fn as usize,
            stream_ptr: Box::into_raw(Box::new(ws_stream)) as usize,
        };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;

        let sched = actor::global_scheduler();
        sched.spawn(
            ws_connection_handler_entry as *const u8,
            args_ptr,
            std::mem::size_of::<WsConnectionArgs>() as u64,
            1, // Normal priority
        );
    }
}
```

This is structurally identical to `snow_http_serve` in `http/server.rs`. The TLS variant (`snow_ws_serve_tls`) follows the `snow_http_serve_tls` pattern exactly: build `ServerConfig`, `StreamOwned::new()` for lazy handshake, wrap in `WsStream::Tls`.

### 10. Connection Actor Entry Function

```rust
extern "C" fn ws_connection_handler_entry(args: *const u8) {
    let args = unsafe { Box::from_raw(args as *mut WsConnectionArgs) };
    let handler_fn = args.handler_fn as *mut u8;
    let mut stream = unsafe { *Box::from_raw(args.stream_ptr as *mut WsStream) };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Phase 1: HTTP upgrade handshake
        let upgrade_req = parse_request(&mut stream);  // reuse HTTP parser
        let (path, headers) = match upgrade_req {
            Ok(parsed) => validate_ws_upgrade(parsed),
            Err(e) => { eprintln!("[ws] parse error: {}", e); return; }
        };
        write_101_response(&mut stream, &headers);

        // Phase 2: Create WsConn handle
        let my_pid = snow_actor_self();
        let conn = WsConn {
            writer: Mutex::new(stream),
            actor_pid: my_pid,
            shutdown: AtomicBool::new(false),
            path,
        };
        let conn_handle = Box::into_raw(Box::new(conn)) as u64;

        // Phase 3: Spawn reader thread
        // (reader thread reads frames, sends to actor mailbox)
        spawn_reader_thread(conn_handle);

        // Phase 4: Call Snow handler with conn handle
        // Handler signature: fn(conn: u64) -> void
        call_ws_handler(handler_fn, conn_handle);

        // Phase 5: Cleanup on handler return
        shutdown_connection(conn_handle);
    }));

    if let Err(panic_info) = result {
        eprintln!("[ws] handler panicked: {:?}", panic_info);
    }
}
```

---

## Data Flow Diagrams

### Connection Lifecycle

```
1. TCP Accept
   TcpListener.accept() -> TcpStream

2. Actor Spawn
   scheduler.spawn(ws_connection_handler_entry, args)

3. HTTP Upgrade (inside actor)
   parse_request() -> validate_ws_upgrade() -> write_101_response()

4. Reader Thread Spawn (inside actor)
   std::thread::spawn(reader_loop)
   Reader blocks on stream.read(), sends frames to actor mailbox

5. Handler Invocation (inside actor)
   call_ws_handler(handler_fn, conn_handle)
   Handler runs receive loop, processes WS frames + actor messages

6. Disconnect
   Client sends Close frame -> reader delivers :ws_close to mailbox
   OR handler returns -> actor sets shutdown flag
   OR reader error -> reader sends :ws_close to mailbox
   -> cleanup: leave all rooms, close stream, free WsConn
```

### Message Flow: Client sends text, server echoes

```
Client           Reader Thread        Actor Mailbox        Connection Actor
  |                   |                    |                      |
  | -- Text Frame --> |                    |                      |
  |                   | read_frame()       |                      |
  |                   | unmask payload     |                      |
  |                   | snow_actor_send()  |                      |
  |                   | ----------------> [WS_TEXT_TAG, payload]   |
  |                   |                    |                      |
  |                   |                    | snow_actor_receive()  |
  |                   |                    | <-------------------- |
  |                   |                    |  match WS_TEXT_TAG    |
  |                   |                    |                      |
  | <-- Text Frame ---|--------------------|----- write_frame() --|
  |                   |                    |                      |
```

### Message Flow: Room broadcast

```
Actor A (sender)     Room Registry        Actor B (ws conn)     Client B
  |                      |                      |                  |
  | Ws.broadcast("chat", msg)                   |                  |
  | ----- ws_broadcast() |                      |                  |
  |                      | lookup "chat" PIDs   |                  |
  |                      | for each PID:        |                  |
  |                      | snow_actor_send() -->|                  |
  |                      |                      | receive match    |
  |                      |                      | write_frame() -->|
  |                      |                      |                  |
```

---

## Patterns to Follow

### Pattern 1: GC-Safe Handle (proven in pg.rs, sqlite.rs, pool.rs)

**What:** External resources stored as `Box::into_raw() as u64` handles
**When:** Any Rust struct that must persist across Snow function calls
**Example:** `WsConn` handle passed to `Ws.send(conn, msg)`

### Pattern 2: Accept Loop + Actor Spawn (proven in http/server.rs)

**What:** TCP listener accept loop spawns a new actor per connection
**When:** Server-side network services
**Why reuse:** Same `sched.spawn()` call, same `ConnectionArgs` pattern, same `catch_unwind` wrapping

### Pattern 3: Stream Enum for Plain/TLS (proven in pg.rs, http/server.rs)

**What:** Enum dispatching between `TcpStream` and `StreamOwned` for transparent TLS
**When:** Any network connection that may be plaintext or encrypted
**Why reuse:** Identical pattern, avoids trait objects, compile-time dispatch

### Pattern 4: Reader Thread for Long-Lived I/O (proven in timer.rs)

**What:** Background OS thread performing blocking I/O, delivers results via `snow_actor_send()`
**When:** I/O that blocks indefinitely and must not consume a scheduler coroutine slot
**Example:** `snow_timer_send_after` spawns an OS thread that sleeps then sends; WS reader thread blocks on `read_frame()` then sends

### Pattern 5: Global Registry with RwLock (proven in actor/registry.rs)

**What:** `OnceLock<RwLock<HashMap<String, ...>>>` for named lookups
**When:** Service discovery / group membership
**Example:** Process registry maps names to PIDs; Room registry maps room names to PID sets

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: HTTP Upgrade Through Existing HTTP Server

**What:** Routing WebSocket upgrades through the existing `snow_http_serve` router
**Why bad:** The HTTP server uses `Connection: close` -- each actor exits after one response. WebSocket connections are long-lived. The HTTP pipeline (router -> middleware -> handler -> response) returns a `SnowHttpResponse`, not a persistent connection. Retrofitting long-lived connections into the request/response model would require invasive changes to `server.rs`.
**Instead:** Separate `Ws.serve(port, handler)` with its own accept loop. Keeps HTTP and WS architecturally independent. If users need both on the same port, a future "protocol detection" layer can be added later.

### Anti-Pattern 2: WebSocket Reader as Actor (instead of OS thread)

**What:** Using `snow_actor_spawn` for the frame reader
**Why bad:** The reader blocks on `stream.read()` indefinitely. Blocking I/O in an actor coroutine is tolerable for short operations (HTTP request ~30s timeout, DB query ~seconds), but a WebSocket connection can last hours or days. A permanently-blocked actor wastes a scheduler slot, prevents the worker thread from running other actors, and can cause thread starvation with many connections. The scheduler has a fixed number of worker threads (default = CPU cores).
**Instead:** Background OS thread (cheap, blocks on kernel I/O without consuming scheduler resources).

### Anti-Pattern 3: Centralizing Room Broadcast Through a Room Actor

**What:** A single "room manager" actor that processes all join/leave/broadcast messages
**Why bad:** Creates a bottleneck. A chat server with 10K connections broadcasting 100 msg/s means the room actor must process 1M send operations per second. The actor is single-threaded (one coroutine). This limits broadcast throughput to one actor's capacity.
**Instead:** Lock-free room registry (`RwLock<HashMap>`) with direct `snow_actor_send()` fan-out. Broadcast iterates the PID set and sends directly -- no routing through a central actor. The `RwLock` is held only briefly for lookups, not for the actual sends.

### Anti-Pattern 4: Sharing WsStream Without Synchronization

**What:** Passing `WsStream` raw pointer to both reader thread and actor
**Why bad:** Data race on `Read + Write` operations. `TcpStream` is not thread-safe for concurrent read+write (on the same fd) without synchronization. For TLS streams, `StreamOwned` is definitely not safe for concurrent access.
**Instead:** For plain TCP, use `TcpStream::try_clone()` to get separate read/write fds. For TLS, use `Mutex<WsStream>` with the reader thread and actor taking turns.

---

## New vs Modified Components (Explicit)

### NEW Files

| File | Lines (est.) | Purpose |
|------|-------------|---------|
| `crates/snow-rt/src/http/ws.rs` | ~600-800 | WebSocket server: accept loop, frame codec, connection actor, reader thread, rooms, heartbeat |

### MODIFIED Files

| File | Change | Size of Change |
|------|--------|---------------|
| `crates/snow-rt/src/http/mod.rs` | Add `pub mod ws;` and re-export WS functions | ~10 lines |
| `crates/snow-rt/Cargo.toml` | Add `sha1 = "0.10"` dependency | 1 line |
| `crates/snow-codegen/src/codegen/intrinsics.rs` | Declare ~12-15 `snow_ws_*` LLVM functions | ~60 lines |
| `crates/snow-typeck/src/builtins.rs` | Add `Ws` module type signatures | ~40 lines |
| `crates/snow-codegen/src/mir/lower.rs` | Map `Ws.*` calls to `snow_ws_*` runtime names | ~30 lines |

### UNCHANGED Files

All existing files remain untouched structurally:
- `http/server.rs` -- HTTP server continues working independently
- `http/router.rs` -- HTTP routing is HTTP-only
- `actor/mod.rs` -- Actor runtime used as-is
- `actor/scheduler.rs` -- Scheduler used as-is
- `actor/mailbox.rs` -- Mailbox used as-is (WS frames are just actor messages)
- `db/*` -- Database modules unrelated

---

## New Runtime Functions (extern "C" ABI)

| Function | Signature | Purpose |
|----------|-----------|---------|
| `snow_ws_serve` | `(handler_fn: ptr, port: i64) -> void` | Start WS server on port, call handler per connection |
| `snow_ws_serve_tls` | `(handler_fn: ptr, port: i64, cert: ptr, key: ptr) -> void` | Start WSS server with TLS |
| `snow_ws_send` | `(conn: u64, msg: ptr) -> void` | Send text frame to client |
| `snow_ws_send_binary` | `(conn: u64, data: ptr, len: u64) -> void` | Send binary frame to client |
| `snow_ws_close` | `(conn: u64) -> void` | Send Close frame and shut down connection |
| `snow_ws_join` | `(conn: u64, room: ptr) -> void` | Join a named room |
| `snow_ws_leave` | `(conn: u64, room: ptr) -> void` | Leave a named room |
| `snow_ws_broadcast` | `(room: ptr, msg: ptr) -> void` | Send text to all connections in room |
| `snow_ws_broadcast_binary` | `(room: ptr, data: ptr, len: u64) -> void` | Send binary to all connections in room |
| `snow_ws_set_ping_interval` | `(conn: u64, ms: i64) -> void` | Configure ping interval (0 = disable) |
| `snow_ws_path` | `(conn: u64) -> ptr` | Get the upgrade request path |
| `snow_ws_conn_pid` | `(conn: u64) -> u64` | Get the actor PID for this connection |

---

## Suggested Build Order

Build order follows dependency chains and enables incremental testing.

### Phase 1: Frame Codec + Upgrade Handshake
- `WsStream` enum (copy `HttpStream` pattern)
- `read_frame()` / `write_frame()` -- frame parsing and serialization
- `validate_ws_upgrade()` / `write_101_response()` -- HTTP upgrade
- SHA-1 + base64 for `Sec-WebSocket-Accept`
- **Test:** Unit tests for frame codec (round-trip encode/decode), handshake validation
- **Dependencies:** `sha1` crate, existing `base64` crate
- **Rationale:** Foundation. Everything else depends on frame I/O working correctly.

### Phase 2: Accept Loop + Connection Actor + Reader Thread
- `snow_ws_serve()` accept loop (copy `snow_http_serve` pattern)
- `ws_connection_handler_entry()` with upgrade + reader thread spawn
- Reader thread: blocking `read_frame()` loop -> `snow_actor_send()` to actor mailbox
- `WsConn` handle creation and lifecycle
- `snow_ws_send()` / `snow_ws_close()`
- **Test:** E2E test with a simple echo WebSocket server
- **Dependencies:** Phase 1 (frame codec), actor runtime (spawn, send, receive)
- **Rationale:** Establishes the core connection model. Rooms and heartbeat are additive.

### Phase 3: TLS Support (wss://)
- `snow_ws_serve_tls()` -- reuse `build_server_config` from `http/server.rs`
- `WsStream::Tls` variant with `Mutex<WsStream>` for reader/writer coordination
- **Test:** E2E test with self-signed cert
- **Dependencies:** Phase 2, existing rustls infrastructure
- **Rationale:** TLS is important for production but can ship after basic ws:// works.

### Phase 4: Rooms + Broadcast
- Global room registry (`WS_ROOMS`)
- `snow_ws_join()` / `snow_ws_leave()` / `snow_ws_broadcast()`
- Auto-cleanup on connection actor exit (terminate callback)
- **Test:** Multi-connection broadcast test
- **Dependencies:** Phase 2 (working connections)
- **Rationale:** Rooms are the primary differentiator. They depend on basic connections working.

### Phase 5: Heartbeat + Compiler Integration
- Auto-Pong in reader thread
- Server-initiated Ping via `snow_timer_send_after`
- Pong timeout detection
- `snow_ws_set_ping_interval()`
- Register all functions in `intrinsics.rs`, `builtins.rs`, `lower.rs`
- **Test:** Connection timeout test, full E2E with Snow source
- **Dependencies:** Phase 2 (connections), Phase 4 (rooms complete)
- **Rationale:** Heartbeat is important for production resilience. Compiler integration unlocks Snow-language-level usage.

---

## Scalability Considerations

| Concern | At 100 connections | At 10K connections | At 100K connections |
|---------|-------------------|-------------------|--------------------|
| Memory per connection | ~64 KiB actor stack + ~4 KiB WsConn + OS thread stack (~8 KiB) = ~76 KiB | ~760 MB total | ~7.6 GB -- possible with tuning (smaller stacks) |
| Reader threads | 100 OS threads | 10K OS threads -- **bottleneck** | Need epoll/io_uring reader pool |
| Room broadcast (100-member room) | 100 `snow_actor_send()` calls -- instant | Same -- scales linearly | Same -- O(N) per broadcast, N = room size |
| Scheduler pressure | 100 actors, minimal -- well within scheduler capacity | 10K actors, moderate -- scheduler handles this (BEAM runs millions) | Scheduler fine, reader threads are the bottleneck |

**10K+ connections scaling note:** The reader-thread-per-connection model works well up to ~5K-10K connections. Beyond that, an epoll/io_uring-based reader pool (small fixed number of reader threads multiplexing many connections) would be needed. This is out of scope for the initial implementation. The actor-per-connection model itself scales well -- it is the OS thread per connection for the reader that limits scale.

**Mitigation for initial release:** Document the connection limit. For most WebSocket use cases (chat, real-time dashboards, multiplayer games), 5K-10K concurrent connections is sufficient. If higher scale is needed later, the reader thread can be replaced with an epoll-based reader pool without changing the actor-per-connection model or the Snow API.

---

## Sources

- [RFC 6455 -- The WebSocket Protocol](https://datatracker.ietf.org/doc/html/rfc6455) -- HIGH confidence (authoritative specification)
- [WebSocket Protocol Wikipedia](https://en.wikipedia.org/wiki/WebSocket) -- MEDIUM confidence (overview, verified against RFC)
- [Tungstenite -- Synchronous WebSocket for Rust](https://github.com/snapview/tungstenite-rs) -- MEDIUM confidence (reference implementation for frame codec patterns)
- Snow codebase direct inspection (all files listed above) -- HIGH confidence

### Snow Codebase (direct inspection)
- `crates/snow-rt/src/http/server.rs` -- HTTP server accept loop, `HttpStream`, actor-per-connection pattern
- `crates/snow-rt/src/http/router.rs` -- HTTP router (NOT used by WS, but shows the module pattern)
- `crates/snow-rt/src/http/mod.rs` -- HTTP module re-exports
- `crates/snow-rt/src/actor/mod.rs` -- Actor spawn, send, receive, timer_send_after, register/whereis
- `crates/snow-rt/src/actor/scheduler.rs` -- M:N scheduler, worker loop, `!Send` coroutines
- `crates/snow-rt/src/actor/process.rs` -- Process Control Block, mailbox, links
- `crates/snow-rt/src/actor/mailbox.rs` -- FIFO mailbox with `parking_lot::Mutex`
- `crates/snow-rt/src/actor/registry.rs` -- Named process registry (`OnceLock<ProcessRegistry>`)
- `crates/snow-rt/src/actor/service.rs` -- Service call/reply pattern
- `crates/snow-rt/src/actor/stack.rs` -- Corosensei coroutines, thread-local state
- `crates/snow-rt/src/db/pg.rs` -- `PgStream` enum, GC-safe u64 handles, TLS upgrade
- `crates/snow-rt/src/db/pool.rs` -- Connection pool with Mutex+Condvar
- `crates/snow-rt/src/gc.rs` -- Global arena + per-actor heap allocation
- `crates/snow-rt/Cargo.toml` -- Current dependencies (base64 already present, sha1 needed)
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- LLVM function declarations pattern

---
*Architecture research for: Snow Language WebSocket Support*
*Researched: 2026-02-12*
