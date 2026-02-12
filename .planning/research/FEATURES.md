# Feature Landscape

**Domain:** WebSocket support for compiled programming language with actor runtime (Snow)
**Researched:** 2026-02-12
**Confidence:** HIGH (WebSocket protocol is a mature, stable RFC 6455 standard from 2011; actor-per-connection patterns well-studied in Erlang/Elixir; Snow's existing HTTP server and actor runtime thoroughly reviewed)

---

## Current State in Snow

Before defining features, here is what already exists and what this milestone builds on:

**Working infrastructure these features extend:**
- HTTP server: hand-rolled HTTP/1.1 parser with actor-per-connection, `HttpStream` enum (Plain/Tls), `SnowRouter` with path params, method routing, middleware pipeline, TLS via rustls
- Actor runtime: M:N work-stealing scheduler with corosensei coroutines (64 KiB stacks), typed `Pid<M>`, `snow_actor_send/receive`, FIFO mailbox with deep-copy message passing, crash isolation via `catch_unwind`
- Supervision trees: `Supervisor.start` with OneForOne/OneForAll/RestForOne/SimpleOneForOne strategies, automatic restart with intensity limits
- Actor registry: `snow_actor_register/whereis` for named process lookup
- Timer support: `Timer.sleep(ms)`, `Timer.send_after(pid, ms, msg)` for delayed messaging
- `receive` with timeout: `snow_actor_receive(timeout_ms)` supports blocking, non-blocking, and timed receive
- TLS infrastructure: rustls 0.23 with `ServerConfig`, PEM certificate loading, `StreamOwned` for lazy handshake
- JSON serde: `deriving(Json)` for encode/decode
- `HttpStream` enum pattern: already demonstrates Plain/Tls stream polymorphism (reusable for WS)
- SHA-1 available via ring (already a transitive dependency of rustls)

**What this milestone adds:**
- WebSocket server (`Ws.serve` / `Ws.serve_tls`) as a separate server type
- HTTP upgrade handshake (101 Switching Protocols, Sec-WebSocket-Key/Accept)
- WebSocket frame parsing and writing (text, binary, ping, pong, close, continuation)
- Client-to-server frame unmasking
- Unified mailbox messaging (WS frames arrive as actor messages)
- Callback-style API (`on_connect`, `on_message`, `on_close`)
- Room/channel system with join/leave/broadcast
- Ping/pong heartbeat with configurable interval
- Connection lifecycle management with close handshake

---

## Table Stakes

Features users expect from any WebSocket implementation. Missing = fundamentally broken or unusable.

### 1. HTTP Upgrade Handshake (RFC 6455 Section 4)

Every WebSocket connection begins as an HTTP/1.1 GET request with specific headers. The server must validate the request, compute `Sec-WebSocket-Accept` from the client's `Sec-WebSocket-Key`, and respond with HTTP 101 Switching Protocols. This is the foundational protocol requirement -- without it, no WebSocket client can connect.

**Confidence: HIGH** -- RFC 6455 is definitive. The algorithm is: concatenate `Sec-WebSocket-Key` + magic string `"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"`, SHA-1 hash, base64-encode. Snow already has SHA-1 available via ring (rustls dependency) and the hand-rolled HTTP parser can be extended to detect upgrade requests.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Detect `Upgrade: websocket` + `Connection: Upgrade` headers | Without detection, HTTP server can't distinguish upgrade requests from normal requests | Low | Check headers in existing `parse_request` function. Case-insensitive comparison per RFC. |
| Validate `Sec-WebSocket-Version: 13` | RFC mandates version check. Return 426 Upgrade Required with correct version if mismatch. | Low | Single header check. Version 13 is the only standard version. |
| Compute `Sec-WebSocket-Accept` from `Sec-WebSocket-Key` | Client validates the accept value. Wrong value = client rejects connection. | Low | SHA-1 + base64. Both available via ring (SHA-1) and base64 crate or manual encoder. ~15 lines of Rust. |
| Respond with `HTTP/1.1 101 Switching Protocols` | This is the protocol switch response. Without it, connection stays HTTP. | Low | Write status line + Upgrade + Connection + Sec-WebSocket-Accept headers to the stream. Reuse existing `write` on `HttpStream`. |
| Reject malformed upgrade requests with 400 | Invalid or missing required headers must be rejected cleanly. | Low | Return 400 Bad Request instead of 101. |

**Depends on:** Existing HTTP parser, existing `HttpStream`, ring (SHA-1)

### 2. WebSocket Frame Parsing (RFC 6455 Section 5)

After the handshake, all communication happens via WebSocket frames. The server must parse incoming frames and write outgoing frames according to the binary framing protocol. This includes handling the FIN bit, opcode, masking, and variable-length payload encoding.

**Confidence: HIGH** -- RFC 6455 frame format is precisely specified. The wire format is straightforward binary parsing.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Parse frame header: FIN, opcode, mask bit, payload length | Every received frame must be parsed. The 2-14 byte header encodes all frame metadata. | Med | Variable-length header: 2 bytes minimum, +2 for 16-bit length, +8 for 64-bit length, +4 for mask key. Bit-level parsing required. |
| Handle 3 payload length encodings (7-bit, 16-bit, 64-bit) | Payload length <= 125 uses 7 bits. 126 = next 2 bytes are length. 127 = next 8 bytes are length. | Low | Three branches based on 7-bit length field. Network byte order (big-endian) for extended lengths. |
| Unmask client-to-server frames (XOR with 4-byte key) | RFC 6455 mandates: clients MUST mask all frames. Server MUST close connection if frame is unmasked. | Low | XOR each payload byte with `mask_key[i % 4]`. Simple loop. ~5 lines of Rust. |
| Write server-to-client frames (unmasked) | Server frames must NOT be masked. Client MUST close if it receives a masked server frame. | Low | Write header bytes + raw payload. No masking needed for server-to-client. |
| Reject unknown opcodes | RFC mandates: receiving an unknown opcode MUST fail the connection. | Low | Check opcode in 0x0-0xA range. Close with 1002 (Protocol Error) for unknowns. |
| Enforce max frame/message size | Without limits, a single client could exhaust server memory with enormous payloads. | Low | Configurable max (default 64 KiB or 1 MiB). Close with 1009 (Message Too Big) if exceeded. |

**Depends on:** Raw TCP stream access (already have via `HttpStream`)

### 3. Text and Binary Frame Types (RFC 6455 Section 5.6)

The two data frame types are text (opcode 0x1, must be valid UTF-8) and binary (opcode 0x2, arbitrary bytes). Every WebSocket application needs at least text frames. Binary frames are required for non-text protocols.

**Confidence: HIGH** -- These are the fundamental data-carrying frame types.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Text frames (opcode 0x1) with UTF-8 validation | Chat, JSON APIs, command protocols all use text. Invalid UTF-8 must trigger close with 1007 (Invalid Payload Data). | Low | Validate with `std::str::from_utf8()` on received text payloads. Snow strings are already UTF-8. |
| Binary frames (opcode 0x2) | File transfer, protocol buffers, custom binary protocols all need binary frames. | Low | Pass raw bytes to Snow handler. No validation needed beyond frame parsing. |
| Distinguish text vs binary in handler callbacks | User code needs to know what type of frame it received to process it correctly. | Low | Deliver as tagged union to Snow: `WsMessage::Text(String)` vs `WsMessage::Binary(Bytes)`. Maps to Snow ADT. |

**Depends on:** Frame parsing (feature 2)

### 4. Control Frames: Close Handshake (RFC 6455 Section 5.5.1, 7)

WebSocket has a two-phase close handshake. Either side can initiate close by sending a Close frame (opcode 0x8). The other side must respond with a Close frame. Only after both frames are exchanged does the TCP connection close. Without this, connections leak or clients see abnormal closure (code 1006).

**Confidence: HIGH** -- Every WebSocket library implements the close handshake. It is required for clean connection lifecycle.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Send Close frame with status code and optional reason | Server needs to initiate clean shutdown. Status code is 2-byte big-endian in payload. | Low | Write close frame: opcode 0x8, payload = [status_hi, status_lo, ...reason_utf8]. |
| Receive Close frame, respond with Close frame | RFC: endpoint MUST respond to Close with Close. After both, close TCP. | Med | State machine: track whether close has been sent/received. Must not send data frames after sending close. |
| Standard close codes: 1000 (Normal), 1001 (Going Away), 1002 (Protocol Error), 1003 (Unsupported Data), 1007 (Invalid Payload), 1008 (Policy Violation), 1009 (Message Too Big), 1011 (Internal Error) | Applications need standardized error signaling. Codes 1005, 1006, 1015 are reserved (never sent on wire). | Low | Enum of close codes. Only codes 1000-1003, 1007-1011 are valid to send. |
| Timeout on close handshake | If remote never responds to Close, TCP must be dropped after timeout (e.g., 5 seconds). | Low | Use existing `set_read_timeout` on TcpStream. Drop connection if Close response not received within timeout. |

**Depends on:** Frame parsing (feature 2)

### 5. Control Frames: Ping/Pong Heartbeat (RFC 6455 Section 5.5.2-3)

Ping (0x9) and Pong (0xA) frames are the protocol-level keepalive mechanism. The server sends Ping; the client must respond with Pong containing the same payload. This detects dead connections that TCP alone may not notice for minutes or hours. Without heartbeat, idle connections silently die behind NATs, firewalls, and load balancers.

**Confidence: HIGH** -- Every production WebSocket server implements ping/pong. Browsers automatically respond to Ping with Pong.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Server sends periodic Ping frames | Detects dead connections. Default interval: 30 seconds. | Med | Use `Timer.send_after` to schedule periodic ping messages to the connection actor. Actor sends Ping frame on timer tick. |
| Receive Pong, validate payload matches sent Ping | RFC: Pong payload MUST echo Ping payload. Mismatch is a protocol violation. | Low | Compare Pong payload to last sent Ping payload. |
| Close connection on missed Pongs | If N consecutive Pongs are missed, the connection is dead. Close with 1001 (Going Away). | Low | Counter in connection actor state. Reset on Pong receipt. Close when threshold exceeded (e.g., 2-3 missed). |
| Receive unsolicited Ping from client, respond with Pong | RFC: endpoint MUST send Pong in response to Ping (unless Close already received). | Low | On receiving Ping frame, immediately write Pong frame with same payload. Max Ping/Pong payload: 125 bytes. |
| Configurable ping interval and pong timeout | Different deployments need different intervals. Mobile needs shorter. Internal services can be longer. | Low | Configuration parameters: `ping_interval_ms` (default 30000), `pong_timeout_ms` (default 10000). |

**Depends on:** Frame parsing (feature 2), Timer.send_after (already exists)

### 6. Actor-per-Connection with Mailbox Integration

Snow's defining feature is the actor-per-connection model. For WebSocket, this means each connected client gets its own lightweight actor (64 KiB stack, crash-isolated). Incoming WebSocket messages are delivered to the actor's mailbox as regular messages, unifying the programming model. The user writes a single `receive` loop that handles both WebSocket frames and actor-to-actor messages.

**Confidence: HIGH** -- This directly mirrors how Phoenix Channels work on BEAM (each channel is a process with a mailbox). Snow's existing HTTP server already uses actor-per-connection.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Each WS connection spawns a dedicated actor | Crash isolation: one bad connection can't take down others. State management: actor state persists for connection lifetime. | Med | Mirror existing `connection_handler_entry` pattern. After handshake, enter a read loop that pushes frames into the actor's own processing. |
| WS messages delivered to actor mailbox | Unified receive: `receive do WsText(msg) -> ... end`. Same pattern as actor-to-actor messages. | Med | Read frame from socket, construct message with type tag, push to own mailbox or call handler directly. Key design decision: direct callback vs mailbox delivery. |
| Actor can send WS frames back to its client | `Ws.send(conn, "hello")` writes a text frame to the connection's socket. | Med | Actor holds reference to its `HttpStream`. `Ws.send` writes a frame to the stream. Must handle concurrent write access (only the owning actor writes). |
| Actor crash = clean WS close | If handler panics, connection closes with 1011 (Internal Error). | Low | Wrap handler in `catch_unwind` (already done for HTTP). On panic, send Close frame before dropping socket. |
| Actor termination on WS disconnect | When client disconnects, the actor should exit. Linked actors get notified. | Low | Socket read returns EOF/error -> actor exits. Exit signal propagates via existing link mechanism. |

**Depends on:** Existing actor runtime, existing `HttpStream`, existing `catch_unwind` pattern

### 7. Connection Lifecycle Callbacks

Users need hooks into the WebSocket connection lifecycle: when a client connects, when it sends a message, and when it disconnects. This is the primary user-facing API. Phoenix has `join/3`, `handle_in/3`, `terminate/2`. Socket.IO has `on('connection')`, `on('message')`, `on('disconnect')`.

**Confidence: HIGH** -- Every WebSocket framework provides these callbacks. This is the table-stakes API surface.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `on_connect` callback: invoked when handshake completes | Authenticate users, initialize state, join rooms. Returning error rejects the connection. | Med | Called after 101 is sent. Receives connection handle + request headers (for auth tokens, cookies). Can return initial state. |
| `on_message` callback: invoked on each text/binary frame | Core message handler. Receives the message payload and connection state. | Low | Dispatched per-frame after parsing. Provides message + current state, returns updated state. |
| `on_close` callback: invoked when connection ends | Cleanup: leave rooms, persist state, log disconnection. | Low | Called before actor exits, whether close was clean or abnormal. Receives close code + reason. |
| Pass request headers to `on_connect` | Authentication tokens, cookies, and subprotocol negotiation happen in the upgrade request. | Low | Existing `ParsedRequest` already captures headers. Pass them to `on_connect` callback. |

**Depends on:** Upgrade handshake (feature 1), actor lifecycle (feature 6)

### 8. Ws.serve / Ws.serve_tls Entry Points

Separate server entry points for WebSocket (not mixed with HTTP router). This follows the same pattern as the existing `Http.serve(router, port)` / `Http.serve_tls(router, port, cert, key)` but for WebSocket-specific handler configuration.

**Confidence: HIGH** -- This mirrors Snow's existing API pattern.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Ws.serve(handler, port)` starts a plaintext WS server | Basic entry point. Accepts TCP connections, performs upgrade, dispatches to handler actors. | Med | New accept loop similar to `snow_http_serve`. Calls upgrade handshake, then enters WS frame loop. |
| `Ws.serve_tls(handler, port, cert, key)` starts a WSS server | Production deployments require encryption (wss://). | Low | Mirror `snow_http_serve_tls` pattern: load certs, wrap in `HttpStream::Tls`, same handshake+frame logic. |
| Handler configuration struct | Bundle `on_connect`, `on_message`, `on_close` callbacks into a single configuration. | Low | Snow struct or builder pattern: `Ws.handler(on_connect, on_message, on_close)`. |

**Depends on:** Existing TLS infrastructure, handshake (feature 1), callbacks (feature 7)

---

## Differentiators

Features that set Snow's WebSocket apart from minimal implementations. Not expected from a basic WS library, but valued for real applications.

### 9. Rooms/Channels with Join/Leave/Broadcast

A room/channel system lets connections subscribe to named groups and receive broadcasted messages. This is what makes WebSocket useful for chat, notifications, live dashboards, multiplayer games, and collaborative editing. Without rooms, users must manually maintain subscriber lists -- tedious and error-prone.

Phoenix calls these "topics" (e.g., `"chat:lobby"`). Socket.IO calls them "rooms." The underlying mechanism is identical: a registry mapping room names to sets of connection PIDs.

**Confidence: HIGH** -- The pattern is well-established across Phoenix Channels, Socket.IO, and ActionCable. Snow's existing actor registry provides the foundation.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| `Ws.join(conn, room_name)` subscribes connection to a room | Connections can participate in named groups without manual PID tracking. | Med | Global room registry: `RwLock<HashMap<String, HashSet<ProcessId>>>`. Join adds PID to set. |
| `Ws.leave(conn, room_name)` unsubscribes connection | Explicit unsubscribe for switching rooms or scoping messages. | Low | Remove PID from room set. |
| `Ws.broadcast(room_name, message)` sends to all room members | The core value: one call reaches all subscribers. No manual iteration. | Med | Look up PIDs in room set, send WS frame to each. Must handle dead connections (PID no longer valid) gracefully. |
| `Ws.broadcast_except(room_name, message, except_conn)` sends to all except sender | Chat pattern: sender doesn't need to receive their own message. | Low | Same as broadcast but skip the `except_conn` PID. |
| Auto-leave on disconnect | When connection actor exits, automatically remove from all rooms. | Med | Hook into actor exit cleanup (like `cleanup_process` in registry). Room registry tracks PID->rooms reverse index for efficient cleanup. |
| Room authorization in `on_connect` or `join` | Not all connections should access all rooms. | Low | `Ws.join` can return `Result<(), String>`. Application logic in callback decides. |

**Depends on:** Actor registry pattern (exists), connection lifecycle (feature 6)

### 10. Message Fragmentation (RFC 6455 Section 5.4)

Large messages can be split across multiple frames using continuation frames (opcode 0x0). The first frame has the data type opcode (text/binary) with FIN=0. Subsequent frames have opcode 0x0 with FIN=0. The final frame has opcode 0x0 with FIN=1. Control frames (ping/pong/close) can be interleaved between fragments.

**Confidence: HIGH** -- RFC 6455 specifies this precisely. Most clients don't fragment small messages, but large file transfers or streaming data will use fragmentation.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Reassemble fragmented messages | Clients may split large messages. Server must reconstruct the full message before delivering to handler. | Med | Buffer continuation frames, concatenate on FIN=1. Track "currently fragmented" state. Only one message can be fragmented at a time. |
| Handle interleaved control frames during fragmentation | RFC allows ping/pong/close between fragments. Must not disrupt reassembly. | Med | Process control frames immediately even mid-fragment. Resume fragment reassembly after control frame handling. |
| Server-side fragmentation for large outgoing messages | Send large messages without buffering entire payload. | Low | Optional: chunk outgoing messages into continuation frames if payload exceeds threshold. Most implementations send complete frames. |

**Depends on:** Frame parsing (feature 2)

### 11. Subprotocol Negotiation (RFC 6455 Section 4.2.2)

The `Sec-WebSocket-Protocol` header allows client and server to agree on an application-level protocol (e.g., `graphql-ws`, `mqtt`, `stomp`). The client lists supported protocols; the server picks one. This enables protocol versioning and multi-protocol servers.

**Confidence: HIGH** -- Well-defined in RFC 6455. Important for GraphQL subscriptions, MQTT-over-WS, and protocol evolution.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Parse `Sec-WebSocket-Protocol` from upgrade request | Read client's preferred subprotocol list from headers. | Low | Split comma-separated header value. Already have header access in `ParsedRequest`. |
| Server selects subprotocol and includes in 101 response | Server picks the first mutually supported protocol. | Low | Match against configured supported protocols. Include selected protocol in response `Sec-WebSocket-Protocol` header. |
| Pass selected subprotocol to `on_connect` callback | Application logic may behave differently based on negotiated protocol. | Low | String field in connection info passed to callback. |
| Reject if no common subprotocol | If client requires a subprotocol the server doesn't support, reject with 400. | Low | Return HTTP 400 instead of 101 if no match and client specified protocols. |

**Depends on:** Upgrade handshake (feature 1)

### 12. Configurable Connection Limits and Backpressure

Production deployments need to limit concurrent connections and handle slow consumers. Without limits, a server can be overwhelmed by connection storms. Without backpressure, fast producers can exhaust memory on slow consumers.

**Confidence: MEDIUM** -- The pattern is well-understood, but Snow's actor scheduler already provides natural backpressure through mailbox depth and reduction-based preemption.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Max concurrent connections limit | Prevent resource exhaustion. Reject new connections with 503 when at capacity. | Low | Atomic counter in accept loop. Increment on accept, decrement on close. |
| Per-connection send buffer limit | If client can't keep up, buffer fills. Close connection rather than OOM. | Med | Track outgoing buffer size per actor. Close with 1008 (Policy Violation) if threshold exceeded. |
| Connection rate limiting | Prevent connection storms from overwhelming the server. | Low | Simple token-bucket or fixed-window counter in accept loop. |

**Depends on:** Accept loop (feature 8), actor lifecycle (feature 6)

---

## Anti-Features

Features to explicitly NOT build in this milestone. Adding these would increase complexity without proportional value, or they conflict with Snow's design philosophy.

### 1. Per-Message Deflate Compression (RFC 7692)

**Why Avoid:** Enormous implementation complexity for marginal benefit. Adds ~300KB memory overhead per connection. Requires negotiation of 4 parameters (`server_no_context_takeover`, `client_no_context_takeover`, `server_max_window_bits`, `client_max_window_bits`), context management across frames, integration with message fragmentation, and zlib/deflate dependency. Most WebSocket traffic is small JSON messages where compression overhead exceeds savings.

**What to Do Instead:** Defer to a future milestone. Users who need compression can compress at the application level (e.g., gzip the JSON payload before sending as a binary frame). The extension negotiation hook can be stubbed (ignore `Sec-WebSocket-Extensions` header) without breaking clients -- they simply proceed uncompressed.

### 2. WebSocket Extensions Framework (RFC 6455 Section 9)

**Why Avoid:** The only widely-used extension is permessage-deflate (anti-feature 1 above). Building a generic extensions framework for a single extension is over-engineering. Extensions modify frame-level behavior (RSV bits, payload transformation) which significantly complicates the frame parser.

**What to Do Instead:** Ignore `Sec-WebSocket-Extensions` header. Clients handle this gracefully -- they simply don't use extensions.

### 3. HTTP/WebSocket Dual-Protocol on Same Port

**Why Avoid:** Snow's design uses separate `Http.serve` and `Ws.serve` entry points. Mixing protocols on one port requires discriminating upgrade requests from normal HTTP at the routing level, adds complexity to the request pipeline, and blurs the architectural boundary. The HTTP middleware chain doesn't apply to WebSocket connections.

**What to Do Instead:** Keep servers separate. Users who need both run `Http.serve` on one port and `Ws.serve` on another. This is clean, debuggable, and follows the principle of separation of concerns. A future milestone could add a combined server if demand warrants it.

### 4. Client-Side WebSocket Library

**Why Avoid:** Snow is a server-side language. Building a WebSocket client means implementing masking (client MUST mask frames), reconnection logic, exponential backoff, and a fundamentally different connection lifecycle. The use case (Snow connecting to external WS services) is niche compared to serving WS connections.

**What to Do Instead:** Focus on the server. If Snow programs need to consume WebSocket APIs, they can use the existing HTTP client for REST alternatives, or a future milestone can add WS client support.

### 5. WebTransport / HTTP/3 Support

**Why Avoid:** WebTransport over HTTP/3 (RFC 9220) has no production browser implementations as of 2025. Chrome reached "Intent to Prototype" stage; Firefox has no announced implementation. The protocol requires QUIC (UDP-based), which is fundamentally different from Snow's TCP-based actor-per-connection model.

**What to Do Instead:** Build on the stable, universal WebSocket protocol (RFC 6455 over TCP). WebTransport can be evaluated in 2-3 years when browser support matures.

### 6. Automatic JSON Deserialization in WS Handlers

**Why Avoid:** Coupling JSON parsing to the WebSocket frame layer conflates protocol concerns. Text frames can carry any format (XML, YAML, custom protocols). Binary frames are inherently format-agnostic.

**What to Do Instead:** Users call `Json.parse(msg)` or `Struct.from_json(msg)` explicitly in their `on_message` callback. This is one extra line and keeps the WS layer protocol-agnostic. Snow's existing `deriving(Json)` makes this trivial.

---

## Feature Dependencies

```
Feature 1: Upgrade Handshake
    |
    +---> Feature 2: Frame Parsing
    |         |
    |         +---> Feature 3: Text/Binary Frames
    |         +---> Feature 4: Close Handshake
    |         +---> Feature 5: Ping/Pong Heartbeat
    |         +---> Feature 10: Message Fragmentation
    |
    +---> Feature 7: Lifecycle Callbacks
    |         |
    |         +---> Feature 6: Actor-per-Connection (parallel with 2)
    |
    +---> Feature 11: Subprotocol Negotiation

Feature 6: Actor-per-Connection
    |
    +---> Feature 9: Rooms/Channels (needs connection PIDs)
    +---> Feature 12: Connection Limits

Feature 8: Ws.serve Entry Points (depends on 1 + 2 + 6 + 7)
```

**Critical path:** Handshake -> Frame Parsing -> Actor Integration -> Callbacks -> Ws.serve

**Parallelizable:** Ping/Pong and Close can be developed alongside text/binary frames once frame parsing exists. Rooms can be developed once actor-per-connection works. Subprotocol negotiation is independent of frame parsing.

---

## MVP Recommendation

**Phase 1 (Protocol Core):** Build the foundation that everything else depends on.

Prioritize:
1. **Upgrade Handshake** (feature 1) -- without this, nothing works
2. **Frame Parsing/Writing** (feature 2) -- the wire protocol
3. **Text/Binary Frames** (feature 3) -- the data-carrying frames
4. **Close Handshake** (feature 4) -- clean connection lifecycle
5. **Actor-per-Connection** (feature 6) -- Snow's core model

These 5 features constitute a minimal but functional WebSocket server.

**Phase 2 (Production Features):** Make it usable for real applications.

Prioritize:
6. **Lifecycle Callbacks** (feature 7) -- the user-facing API
7. **Ws.serve / Ws.serve_tls** (feature 8) -- the entry points
8. **Ping/Pong Heartbeat** (feature 5) -- production keepalive
9. **Message Fragmentation** (feature 10) -- protocol compliance

**Phase 3 (Application Layer):** Higher-level features for real-world patterns.

Prioritize:
10. **Rooms/Channels** (feature 9) -- the multiplayer/chat/notification pattern
11. **Subprotocol Negotiation** (feature 11) -- protocol versioning
12. **Connection Limits** (feature 12) -- production safety

Defer:
- **Per-message deflate** (anti-feature 1): high complexity, low value for v1
- **Dual-protocol server** (anti-feature 3): clean separation is better for now
- **WS client** (anti-feature 4): server-only focus

---

## Sources

- [RFC 6455: The WebSocket Protocol](https://datatracker.ietf.org/doc/html/rfc6455) -- Definitive protocol specification
- [RFC 7692: Compression Extensions for WebSocket](https://datatracker.ietf.org/doc/html/rfc7692) -- Per-message deflate extension (deferred)
- [MDN: Writing WebSocket servers](https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API/Writing_WebSocket_servers) -- Implementation guide
- [MDN: Sec-WebSocket-Accept](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-WebSocket-Accept) -- Handshake header details
- [WebSocket.org: Close Codes Reference](https://websocket.org/reference/close-codes/) -- Complete close code table
- [websockets (Python) Keepalive docs](https://websockets.readthedocs.io/en/stable/topics/keepalive.html) -- Ping/pong best practices
- [Phoenix Channels documentation](https://hexdocs.pm/phoenix/channels.html) -- Rooms/topics pattern reference
- [Socket.IO Rooms documentation](https://socket.io/docs/v3/rooms/) -- Rooms pattern reference
- [WebSock (Elixir)](https://github.com/phoenixframework/websock) -- Actor-per-WebSocket specification
- [OpenMyMind: WebSocket Framing](https://www.openmymind.net/WebSocket-Framing-Masking-Fragmentation-and-More/) -- Frame format deep dive
- [VideoSDK: Ping Pong Frame WebSocket](https://www.videosdk.live/developer-hub/websocket/ping-pong-frame-websocket) -- Heartbeat implementation guide
- [OneUptime: WebSocket Heartbeat](https://oneuptime.com/blog/post/2026-01-24-websocket-heartbeat-ping-pong/view) -- Production heartbeat configuration
- [Ably: WebSocket Architecture Best Practices](https://ably.com/topic/websocket-architecture-best-practices) -- Scalability patterns
- Snow source: `crates/snow-rt/src/http/server.rs` -- Existing HTTP server with actor-per-connection pattern
- Snow source: `crates/snow-rt/src/actor/mod.rs` -- Actor runtime with send/receive/mailbox
- Snow source: `crates/snow-rt/src/actor/registry.rs` -- Named process registry (foundation for rooms)
