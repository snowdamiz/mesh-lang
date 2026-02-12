# Domain Pitfalls: WebSocket Support in Actor-Based Runtime

**Domain:** Adding WebSocket support (RFC 6455) to an actor-based compiled language with cooperative scheduling, per-actor GC, hand-rolled HTTP/1.1 parser, and TLS via rustls 0.23
**Researched:** 2026-02-12
**Confidence:** HIGH (based on direct Snow codebase analysis of server.rs, scheduler.rs, mailbox.rs, actor/mod.rs, process.rs, stack.rs; RFC 6455 specification; real-world WebSocket implementation bugs from ws, tungstenite, Bun, aiohttp, Jetty, actix-web; actor-WebSocket integration patterns from Akka, Play Framework, actix)

**Scope:** This document covers pitfalls specific to adding WebSocket support to the existing Snow runtime. Each pitfall is analyzed against Snow's current architecture: corosensei coroutines (!Send, thread-pinned) on M:N work-stealing scheduler with reduction-based cooperative preemption, per-actor mark-sweep GC with conservative stack scanning, hand-rolled HTTP/1.1 parser (server.rs parse_request/write_response), HttpStream enum (Plain/Tls) wrapping TcpStream or StreamOwned<ServerConnection, TcpStream>, actor-per-connection model with 64 KiB stacks, Connection: close header (no keep-alive), and FIFO mailbox with MessageBuffer (type_tag + data).

**Relationship to prior research:** The v3.0 PITFALLS.md (2026-02-12) covered connection pooling, TLS handshake blocking, and database transaction pitfalls. This document covers WebSocket-specific pitfalls that arise when extending the HTTP layer to support long-lived bidirectional connections. TLS handshake blocking (v3.0 Pitfall 2) applies to WSS connections but is not re-covered; references are noted where relevant.

---

## Critical Pitfalls

Mistakes that cause protocol violations, connection leaks, scheduler deadlocks, or require architectural rewrites.

---

### Pitfall 1: HTTP Upgrade Handshake Fails Because Current Parser Consumes and Discards the Connection

**What goes wrong:**
The current `connection_handler_entry` (server.rs lines 374-398) follows a strict request-response-done lifecycle: parse one request, call one handler, write one response, then the actor exits and the stream is dropped. The function signature and control flow assume a single exchange. There is no mechanism for the handler to say "this connection should be upgraded to WebSocket -- keep it alive and give me the stream." After `process_request` returns, the handler has no access to the underlying `HttpStream`. The stream is a local variable inside `connection_handler_entry` and is dropped when the actor function returns.

For WebSocket upgrade, the server must: (1) parse the HTTP upgrade request, (2) validate the Upgrade, Connection, Sec-WebSocket-Key, and Sec-WebSocket-Version headers, (3) compute and send the 101 Switching Protocols response with `Sec-WebSocket-Accept`, and then (4) keep the TCP connection open for bidirectional WebSocket framing indefinitely. Steps 1-3 can reuse parts of the existing parser, but step 4 requires the connection actor to enter a long-lived read/write loop instead of exiting.

**Why it happens:**
- `connection_handler_entry` is designed for HTTP request-response, not long-lived connections (server.rs line 335: `Connection: close`)
- The `HttpStream` is owned by the actor entry function, not by the Snow-level handler. The Snow handler only receives a `SnowHttpRequest` struct -- it never touches the raw stream
- The `write_response` function hardcodes `Connection: close` and `Content-Type: application/json`, which are wrong for a 101 Switching Protocols response
- There is no "upgrade" return path from `process_request` -- it always returns `(u16, Vec<u8>)`, which is a status code and body

**Consequences:**
- Cannot implement WebSocket upgrade without modifying the connection handler architecture
- If attempted by hacking `process_request` to return a special status code (101), the stream is still dropped after the response is written
- If the stream is somehow kept alive, there is no mechanism for the Snow-level code to read/write WebSocket frames on it

**Prevention:**
1. **Redesign `connection_handler_entry` to support upgrade.** After `parse_request`, check for `Upgrade: websocket` and `Connection: Upgrade` headers. If present, do NOT call the normal `process_request` path. Instead, validate the handshake, send the 101 response, and transition the actor into a WebSocket frame read/write loop.
2. **The upgrade detection must happen at the Rust runtime level, not in Snow user code.** The Snow handler cannot access raw streams. The runtime must detect upgrade requests, call the appropriate Snow WebSocket handler (registered via a new route type like `router.ws("/chat", handler)`), and manage the frame loop.
3. **Send the 101 response with correct headers, not through `write_response`.** The 101 response requires specific headers (`Upgrade: websocket`, `Connection: Upgrade`, `Sec-WebSocket-Accept: <computed>`) and NO body. Write a dedicated `write_upgrade_response` function.
4. **After the 101 response, the actor transitions into a WebSocket loop** that reads frames, delivers payloads as mailbox messages, and writes outgoing frames when the Snow handler sends them.

**Detection:**
- WebSocket clients receive `Connection: close` and the TCP connection terminates after the 101 response
- Client-side errors like "WebSocket connection closed before open"
- The upgrade request is processed as a regular HTTP request and returns 404 or the handler receives it without access to the stream

**Phase:** HTTP upgrade handshake. This is the very first thing that must work. Everything else depends on it.

---

### Pitfall 2: WebSocket Actor Blocks Worker Thread Indefinitely During Frame Read Loop

**What goes wrong:**
After the WebSocket upgrade, the connection actor enters a loop reading frames from the socket. The current Snow runtime uses blocking `std::io::Read` on `TcpStream` (server.rs line 21: `use std::io::{BufRead, BufReader, Read, Write}`). In the HTTP case, this is acceptable because each read is a single short-lived request (bounded by the 30-second read timeout, server.rs line 440). But a WebSocket connection can be idle for minutes or hours between messages. A blocking `read()` call that waits for the next WebSocket frame will block the entire OS worker thread, preventing all other actors pinned to that thread from making progress.

With Snow's M:N scheduler, coroutines are `!Send` and thread-pinned (scheduler.rs line 9). If a WebSocket actor on worker thread 3 blocks in `TcpStream::read()` waiting for a frame, all other actors suspended on thread 3 (including other WebSocket connections, HTTP handlers, database actors) are starved. The scheduler's Phase 1 (lines 400-443) processes suspended coroutines sequentially -- it cannot skip a blocked coroutine because the block happens inside the coroutine's `resume()`, which does not return until the blocking I/O completes.

**Why it happens:**
- `TcpStream::read()` is a blocking syscall -- it does not yield to the coroutine scheduler
- The existing HTTP model (server.rs line 19 comment: "Blocking I/O is accepted...since each actor runs on a scheduler worker thread") works because HTTP requests are short. WebSocket connections are long
- Snow's reduction check (`snow_reduction_check`, actor/mod.rs line 160) only yields at loop back-edges and function calls in Snow-compiled code, not during Rust runtime blocking I/O
- There is no mechanism to make the coroutine yield while waiting for socket data

**Consequences:**
- Worker thread starvation: if 4 WebSocket connections are idle on a 4-thread scheduler, ALL scheduler threads can be blocked, halting the entire runtime
- Ping/pong heartbeats on other connections cannot fire because their actors are starved
- New HTTP connections queue in the accept loop but are never dispatched because all workers are blocked
- The system appears hung under moderate WebSocket load even with minimal message traffic

**Prevention:**
1. **Set a read timeout on the TcpStream for WebSocket connections.** Use a short timeout (e.g., 50-100ms) so that `read()` returns `WouldBlock` or `TimedOut` periodically, allowing the coroutine to yield and let other actors run. This is a "cooperative blocking" pattern: block briefly, check results, yield, repeat.
2. **On each timeout/WouldBlock, yield to the scheduler** via `yield_current()`. The actor transitions from Running to Ready, gets re-enqueued, and is resumed later. Other actors on the same thread get to run.
3. **Structure the WebSocket read loop as: set short timeout -> attempt read -> if WouldBlock, yield -> re-check -> resume.** This mirrors how Erlang/BEAM handles socket I/O: the runtime polls sockets and only wakes the process when data is available.
4. **Do NOT remove the read timeout entirely.** An infinite read timeout on a WebSocket connection means a disconnected client (without TCP FIN) will hold the actor and thread forever. The 30-second timeout from the HTTP path is too long for WebSocket cooperative scheduling -- use a much shorter one.

**Detection:**
- All worker threads show as blocked in `read()` syscall (visible via `strace` or `lldb`)
- HTTP requests stop being processed even though the server is running
- Only observable under concurrent WebSocket connections (single connection works fine because it only blocks one thread)
- Ping/pong heartbeats time out on connections that should be alive

**Phase:** Core WebSocket frame loop. Must be solved in the same phase as the frame parser. A blocking frame loop cannot be retrofitted.

---

### Pitfall 3: Sec-WebSocket-Accept Computed from Hex Digest Instead of Raw Binary

**What goes wrong:**
The WebSocket handshake requires the server to compute `Sec-WebSocket-Accept` as: `base64(SHA-1(Sec-WebSocket-Key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))`. The most common implementation bug, documented across dozens of libraries (Node.js ws, PowerBASIC WebSocket, Espruino, many Stack Overflow answers), is base64-encoding the hex string representation of the SHA-1 digest instead of the raw 20-byte binary digest.

The hex digest is a 40-character ASCII string representing 20 bytes as hexadecimal. Base64-encoding this 40-character string produces a ~56-character result. The correct approach base64-encodes the raw 20 bytes, producing a ~28-character result. For the RFC's example key `dGhlIHNhbXBsZSBub25jZQ==`, the correct `Sec-WebSocket-Accept` is `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=` (28 chars). The hex-then-base64 bug produces `YjM3YTRmMmNjMDYyNGYxNjkwZjY0NjA2Y2YzODU5NDViMmJlYzRlYQ==` (56 chars).

**Why it happens:**
- Many cryptographic APIs default to hex output (e.g., `digest.hexdigest()` in Python, `.digest('hex')` in Node)
- Rust's `sha1` crate returns raw bytes by default (`sha1::Sha1::digest()` returns `[u8; 20]`), which is correct -- but if someone uses a helper that converts to hex string first, the bug manifests
- The GUID string `258EAFA5-E914-47DA-95CA-C5AB0DC85B11` is long and easy to mistype (one wrong character produces a silently wrong hash)
- No client will tell you WHY the handshake failed; they just close the connection

**Consequences:**
- Every WebSocket connection attempt fails during handshake
- The client disconnects immediately after receiving the 101 response with the wrong `Sec-WebSocket-Accept`
- Extremely hard to debug without packet capture: the server thinks the handshake succeeded, but the client rejects it
- Different clients may fail in different ways (some close silently, some report "handshake failed")

**Prevention:**
1. **In Rust, use `sha1::Sha1::digest()` directly, which returns raw bytes.** Then `base64::engine::general_purpose::STANDARD.encode(&digest)`. Never convert to hex string at any point.
2. **Store the GUID as a constant** and verify it matches the RFC exactly: `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`.
3. **Write a unit test using the RFC's example values.** RFC 6455 Section 4.2.2 provides: key `dGhlIHNhbXBsZSBub25jZQ==` must produce accept `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`. This is the single most valuable test for the entire WebSocket implementation.
4. **Test against a real browser.** Open Chrome DevTools Network tab, connect to the server, and verify the handshake succeeds. If it fails, compare the `Sec-WebSocket-Accept` value in the response with the expected one.

**Detection:**
- All WebSocket connections fail immediately after the 101 response
- Browser console shows "WebSocket connection failed" with no further details
- Wireshark shows the server sending 101 but the client sending a TCP RST
- Comparing the Accept header value length: correct is ~28 chars, hex-bug is ~56 chars

**Phase:** HTTP upgrade handshake. Test this with the RFC example values before testing anything else.

---

### Pitfall 4: Frame Masking Direction Reversed (Server Masks, Client Unmasks)

**What goes wrong:**
RFC 6455 Section 5.3 mandates asymmetric masking: clients MUST mask all frames sent to the server; servers MUST NOT mask frames sent to clients. A client that receives a masked frame from the server MUST close the connection (close code 1002, Protocol Error). Conversely, a server that receives an unmasked frame from a client MUST close the connection.

This asymmetry is the source of bugs in production WebSocket implementations. The actix-web project had a critical bug (Issue #2441) where server-side masking caused connection failures. The Alchemy-Websockets project (Issue #21) had the server masking frames because its client and server code shared implementation. The Ktor framework (Issue #423) had the client NOT masking by default, violating the spec. The ESP-IDF project (Issue #18227, February 2026) had a 50% failure rate on PONG frames due to incorrect mask handling depending on random mask key bits.

**Why it happens:**
- When implementing both server-side frame writing and frame reading, it is natural to share code. If masking is a symmetric operation in the shared code, the server will accidentally mask outgoing frames
- The mask is a 4-byte XOR key applied byte-by-byte. The implementation is trivial but the DIRECTION matters: only decode incoming client frames (apply mask), never encode outgoing server frames (no mask)
- When testing server-to-server or with custom clients, the masking direction may not be checked, and bugs go unnoticed until a browser client connects

**Consequences:**
- Browser WebSocket clients immediately close the connection with code 1002 when they receive a masked frame from the server
- If the server fails to unmask incoming client frames, all received text appears as garbage (XOR'd with the mask key)
- 50% failure rates when mask key bytes have specific bit patterns (as seen in ESP-IDF)
- Connection failures that appear random and are extremely difficult to reproduce

**Prevention:**
1. **In the frame writer, NEVER set the mask bit (bit 7 of the second byte) for server-to-client frames.** The mask bit must be 0 and no masking-key field should be present.
2. **In the frame reader, ALWAYS check the mask bit on incoming client frames.** If bit 7 of the second byte is 0 (unmasked), close the connection with code 1002.
3. **Apply unmasking to incoming frames:** `for i in 0..payload.len() { payload[i] ^= mask_key[i % 4]; }`
4. **Write separate code paths for reading and writing frames.** Do not share a generic "frame codec" that applies masking in both directions.
5. **Test with a real browser.** Chrome and Firefox strictly enforce the no-server-masking rule and will close the connection immediately.

**Detection:**
- Browser console shows close code 1002 "Protocol Error"
- Text messages received by the server appear as garbage (binary data)
- Connection works with custom test clients that do not validate masking but fails with browsers
- Intermittent failures that depend on the random mask key value

**Phase:** Frame parser/writer. Must be tested with browser clients, not just custom test tools.

---

### Pitfall 5: WebSocket Connection Actor Leaks When Client Disconnects Without Close Frame

**What goes wrong:**
When a WebSocket client disappears without sending a Close frame (network failure, browser tab closed, mobile app backgrounded, NAT timeout), the server-side actor never receives the Close frame it expects. If the actor's read loop only exits on receiving a Close frame or clean EOF, the actor will block indefinitely on `read()` waiting for data that will never arrive. The TcpStream will eventually time out (if a read timeout is set), but the actor stays alive in the scheduler's suspended list consuming a PID, process table entry, mailbox, actor heap memory, and room/channel membership.

This is the WebSocket equivalent of "half-open connections" in TCP. Without active detection (ping/pong heartbeat), these ghost actors accumulate. In Snow's architecture, each ghost actor holds a `Process` in the process table (scheduler.rs line 100: `process_table: ProcessTable`), which includes an `ActorHeap`, a `Mailbox`, links, and state. With thousands of ghost actors, the process table grows unboundedly and memory leaks.

The actix-web project documented this exact bug (Issue #2441): "WebSocket actor can be paralyzed due to lack of IO timeout, leading to a connection leak." The actor becomes unable to run IntervalFuncs (like ping/pong) which could shut it down if ping/pong takes too long, because the network buffer fills up and all sends block.

**Why it happens:**
- TCP does not notify the server when a client simply disappears (no FIN packet sent)
- OS-level TCP keepalives default to 2 hours on most systems, far too long for WebSocket use
- Without ping/pong heartbeat, there is no application-level mechanism to detect dead connections
- The actor's read loop blocks on `TcpStream::read()`, preventing the ping/pong timer from firing (this is the key insight from the actix-web bug)
- Even with read timeouts, the actor may treat a timeout as "try again" rather than "connection is dead"

**Consequences:**
- Memory leak: each ghost actor holds ~64KB of stack space plus heap allocations
- Process table grows unboundedly: PID lookup degrades as the FxHashMap grows
- Room/channel membership contains PIDs of dead actors, causing broadcast to fail or waste resources
- Eventually the runtime runs out of memory or the process table becomes too large for the RwLock to perform well

**Prevention:**
1. **Implement server-side ping/pong heartbeat.** Send a Ping frame every 20-30 seconds. If no Pong is received within 10 seconds, consider the connection dead and terminate the actor.
2. **The ping timer must NOT be blocked by the read loop.** Use the short read timeout approach (Pitfall 2): read with 50-100ms timeout, yield, check ping timer, repeat. Do not rely on being able to "interrupt" a blocking read.
3. **On read timeout or I/O error, check if a ping response is overdue.** If the last Pong was received more than (ping_interval + pong_timeout) ago, close the connection.
4. **On actor exit, clean up room/channel membership, named registrations, and links.** Use the existing `terminate_callback` mechanism (actor/mod.rs line 562: `snow_actor_set_terminate`) to remove the actor from rooms and clean up resources.
5. **Count active WebSocket connections and log when count grows unexpectedly.** A monitoring/diagnostic runtime function (`snow_ws_connection_count()`) helps detect leaks early.

**Detection:**
- Memory usage grows linearly over time without corresponding increase in active connections
- `process_table.read().len()` grows monotonically (no cleanup)
- Room broadcasts become slower over time (iterating over dead PIDs)
- After hours of operation, the system runs out of file descriptors or memory

**Phase:** Ping/pong heartbeat. Should be implemented immediately after the frame loop works, not deferred.

---

### Pitfall 6: Fragmented Messages Not Reassembled, Causing Data Corruption

**What goes wrong:**
RFC 6455 Section 5.4 defines message fragmentation: a message can be split across multiple frames. The first fragment has the opcode (text/binary) with FIN=0. Subsequent continuation frames have opcode 0x0 with FIN=0. The final continuation frame has opcode 0x0 with FIN=1. The server must reassemble all fragments into a single message before delivering to the application. Additionally, control frames (Ping, Pong, Close) can be interleaved between fragments of a data message.

This is where many WebSocket implementations fail. Bun's WebSocket client (Issue #3742) threw "Invalid binary message format" errors because it did not handle continuation frames. The aiohttp library (PR #1962) had "several bugs mainly caused by wrong handling of fragmented frames." The faye-websocket-node library (Issue #48) parsed partial frames split across TCP packets as complete frames, producing garbage opcodes. Chrome's DevTools Protocol does not support fragmentation at all, silently dropping messages larger than 1MB.

**Why it happens:**
- Simple test clients often send single-frame messages, so fragmentation bugs are not caught in basic testing
- The frame parser must maintain state between frames: "am I in the middle of a fragmented message?"
- Control frames (Ping/Pong/Close) can arrive BETWEEN data fragments, requiring the parser to handle interleaved control frames without disrupting fragment reassembly
- TCP itself can split a WebSocket frame across multiple `read()` calls, requiring the parser to handle partial frame headers
- Memory management for reassembly: fragments must be accumulated in a buffer that grows up to the message size limit

**Consequences:**
- Large messages (images, JSON documents, file uploads) arrive as corrupted partial data
- Interleaved Ping frames during large message transfer cause the parser to lose track of the fragment state, treating the Ping payload as a data fragment
- Partial TCP reads cause the parser to interpret garbage as frame headers (wrong opcodes, wrong lengths), leading to cascading parse errors
- Messages may be silently truncated or duplicated depending on the nature of the bug

**Prevention:**
1. **Implement a proper frame parser state machine** with states: Idle, ReceivingFragments(opcode, accumulated_data). When FIN=0, accumulate. When FIN=1, deliver the complete message.
2. **Handle control frames independently of fragment state.** When in ReceivingFragments state and a control frame arrives (opcode 0x8, 0x9, 0xA), process it immediately (respond to Ping with Pong, handle Close) without disturbing the accumulated data fragments.
3. **Handle partial TCP reads.** A single `read()` call may not return a complete frame. Buffer incoming bytes and only parse frames when enough data is available. Use a ring buffer or growable Vec for this.
4. **Set a maximum message size limit** (e.g., 16 MB). If accumulated fragments exceed this, close the connection with code 1009 (Message Too Big). Without this, a malicious client can exhaust server memory by sending infinite fragments without FIN=1.
5. **Validate that continuation frames have opcode 0x0** and that a new data frame (opcode 0x1 or 0x2) does not arrive while a fragmented message is being assembled (this is a protocol error).
6. **Test with the Autobahn WebSocket Testsuite** (open source). It has comprehensive test cases for fragmentation, interleaved control frames, and edge cases.

**Detection:**
- Large messages (>64KB, which often triggers fragmentation in many client libraries) arrive corrupted or truncated
- Ping/Pong stops working when large messages are being transferred
- Parse errors or connection resets during large message transfers
- The Autobahn Testsuite reveals dozens of failing cases in the fragmentation section

**Phase:** Frame parser. Must be part of the initial frame parser implementation, not added later. Retrofitting fragment support is extremely difficult.

---

### Pitfall 7: Mailbox Message Type Collision Between WS Frames and Actor Messages

**What goes wrong:**
Snow's actor mailbox uses `MessageBuffer` with a `type_tag: u64` (heap.rs) derived from the first 8 bytes of the message data (actor/mod.rs lines 274-279). When WebSocket messages are delivered to the actor's mailbox alongside regular actor-to-actor messages, the actor needs to distinguish "this is a WebSocket text frame" from "this is a message from another actor" from "this is an exit signal" (which uses `EXIT_SIGNAL_TAG`). If WebSocket frame messages use ad-hoc type tags that collide with regular message tags or the exit signal tag, the actor will misinterpret messages.

The current type_tag derivation is naive: it reads the first 8 bytes of the message as a u64 (actor/mod.rs line 274-279). For a WebSocket text message containing "hello\0\0\0", the type_tag would be the u64 encoding of "hello\0\0\0". This could collide with a regular actor message that happens to start with the same bytes.

**Why it happens:**
- The type_tag system was designed for actor-to-actor messaging where the compiler controls the message format
- WebSocket messages have arbitrary user content, so their first 8 bytes are unpredictable
- Exit signals use `EXIT_SIGNAL_TAG` (link.rs), and if a WebSocket message happens to produce the same tag, it will be misinterpreted as an exit signal
- There is no reserved tag space for "system messages" vs "user messages" vs "I/O messages"

**Consequences:**
- A WebSocket text message is mistakenly matched as an exit signal, causing the actor to terminate
- A regular actor message is mistakenly treated as a WebSocket frame, causing the handler to process garbage as a chat message
- Pattern matching on message type in Snow code (`receive` with match) cannot reliably distinguish message sources
- Extremely hard to debug: depends on the exact bytes of the message content

**Prevention:**
1. **Define dedicated type tags for WebSocket messages in a reserved range.** For example, `WS_TEXT_TAG = 0xFFFF_FFFF_0001`, `WS_BINARY_TAG = 0xFFFF_FFFF_0002`, `WS_CLOSE_TAG = 0xFFFF_FFFF_0003`, `WS_PING_TAG = 0xFFFF_FFFF_0004`, `WS_PONG_TAG = 0xFFFF_FFFF_0005`. Reserve the upper 32 bits (0xFFFF_FFFF) for runtime system messages.
2. **When injecting a WebSocket frame into the actor mailbox, use the dedicated tag**, not the first-8-bytes derivation. The message buffer should contain the frame payload (without frame headers), and the type_tag should identify it as a WS frame.
3. **Ensure the Snow compiler generates type tags in a non-colliding range** (e.g., lower 32 bits only for user-defined message types).
4. **Document the tag allocation scheme** so that future runtime features (database notifications, timer events, file I/O events) also get reserved tags.

**Detection:**
- Actors randomly exit when receiving WebSocket messages that happen to match EXIT_SIGNAL_TAG
- WebSocket messages are handled by the wrong match clause in Snow pattern matching
- Works most of the time but fails with specific message content

**Phase:** Mailbox integration. Must be designed before writing any mailbox delivery code. Retrofitting a tag scheme is painful because it changes the ABI.

---

## Moderate Pitfalls

Mistakes that cause incorrect behavior, poor performance, or security vulnerabilities but do not require full rewrites.

---

### Pitfall 8: Close Handshake State Machine Not Implemented, Causing Resource Leaks

**What goes wrong:**
RFC 6455 Section 7 defines a close handshake: either side can send a Close frame (opcode 0x8), and the other side must respond with a Close frame. After the close exchange, the TCP connection is shut down (the server should initiate TCP close). Many implementations get this wrong. The GNS3 server (Issue #2320) failed to initiate TCP close after the WebSocket close exchange. .NET Core prior to 3.0 had a state machine bug where the client transitioned to the wrong state after receiving the server's Close response. The ASP.NET Core Module killed TCP instead of sending a Close frame.

If Snow's WebSocket actor simply calls `TcpStream::shutdown()` when it wants to close, without sending a Close frame first, compliant clients will report "WebSocket connection closed without completing the close handshake." If the actor receives a Close frame but does not respond with its own Close frame before shutting down TCP, the client will also report an error.

**Why it happens:**
- Closing a WebSocket connection requires a 4-state machine (OPEN -> CLOSING -> CLOSED, plus the server-should-close-TCP-first rule)
- It is tempting to just drop the TcpStream (which sends TCP FIN), skipping the WebSocket close frame entirely
- When the actor panics (caught by `catch_unwind` in server.rs line 383), the cleanup path may skip the close frame
- The close frame can optionally contain a status code (2 bytes) and a reason string (UTF-8). Invalid status codes (like 1005, which is reserved) cause client-side errors

**Prevention:**
1. **Implement a minimal close state machine:** Track whether a Close frame has been sent and/or received. When the application wants to close: send Close frame, set state to CLOSING, continue reading until a Close frame is received (or timeout), then shut down TCP.
2. **When receiving a Close frame from the client:** respond with a Close frame (echoing the status code), then shut down TCP. The server should be the one to call `TcpStream::shutdown()`.
3. **In the `catch_unwind` error path and `terminate_callback`:** attempt to send a Close frame with status 1011 (Internal Error) before dropping the stream. Use a short timeout (1-2 seconds) for this -- do not let a failed close frame delay actor cleanup.
4. **Only use valid close status codes.** 1000 (Normal), 1001 (Going Away), 1008 (Policy Violation), 1011 (Internal Error) are the most common. Never send 1005 (No Status) or 1006 (Abnormal) on the wire -- they are reserved for local use.

**Detection:**
- Client-side console shows "WebSocket connection closed without completing the close handshake"
- Connections in TIME_WAIT state on the client instead of the server (RFC says server should close TCP first)
- Wireshark shows TCP FIN without a preceding Close frame

**Phase:** Frame parser/writer, should be implemented alongside the frame loop.

---

### Pitfall 9: Room/Channel Broadcast Creates O(N) Mailbox Contention Under the Same Mutex

**What goes wrong:**
When a message is broadcast to a room with N members, the naive approach iterates over all member PIDs and calls `snow_actor_send` for each one. Each `snow_actor_send` call (actor/mod.rs lines 261-297) acquires the process table's `RwLock` for reading, then acquires the target process's `Mutex` to push a message into the mailbox, then checks if the process is Waiting to potentially wake it. For a room with 1000 members, this means 1000 sequential RwLock read acquisitions and 1000 Mutex lock/unlock cycles. All of this happens synchronously on the sending actor's thread, blocking it for the entire duration.

In Snow's architecture, the process table is a `Arc<RwLock<FxHashMap<ProcessId, Arc<Mutex<Process>>>>>` (scheduler.rs line 67). The `RwLock` allows concurrent reads, so multiple room broadcasts can overlap. But each individual `Mutex<Process>` is per-actor, so if two broadcasts target the same actor, they serialize on that actor's mutex. More critically, the broadcasting actor is blocked for the entire iteration, unable to process its own incoming messages or yield cooperatively.

**Why it happens:**
- The natural implementation of "send to all members" is a loop calling `snow_actor_send`
- There is no batch-send or multicast primitive in the actor runtime
- Room membership is likely stored as a `Vec<ProcessId>` or `HashSet<ProcessId>`, requiring O(N) iteration
- The deep-copy semantics of `snow_actor_send` (actor/mod.rs line 269: `slice.to_vec()`) mean the message is copied N times

**Consequences:**
- Broadcast latency scales linearly with room size: 1000-member room = 1000 lock/unlock cycles
- The broadcasting actor's thread is blocked during the entire broadcast, starving co-located actors
- For large rooms with frequent messages (e.g., chat rooms), broadcast becomes the bottleneck
- Memory usage spikes: N copies of the same message in N different actor heaps

**Prevention:**
1. **Implement broadcast as a dedicated runtime function** (`snow_ws_broadcast`) that iterates over members and pushes messages without going through the full `snow_actor_send` path. Skip the process table RwLock re-acquisition for each member (acquire once, iterate).
2. **Use a reference-counted message for broadcast** instead of deep-copying N times. Wrap the message payload in an `Arc<Vec<u8>>` and store a reference in each mailbox. This reduces broadcast memory from O(N*M) to O(N+M) where M is the message size.
3. **Consider a dedicated room actor** that owns the member list and performs broadcasts. This isolates the broadcast cost to one actor and prevents it from blocking the sending actor's thread. The sender sends one message to the room actor, which handles distribution.
4. **For very large rooms (1000+ members), batch the broadcast across yield points.** Send to 50-100 members, yield, resume, send to the next batch. This prevents a single broadcast from monopolizing a worker thread.

**Detection:**
- Message latency increases linearly with room size
- Broadcasting actor stops receiving messages during broadcast (blocked)
- CPU profiling shows excessive time in `snow_actor_send` during broadcasts
- Memory spikes when messages are sent to large rooms

**Phase:** Rooms/channels implementation. Design the room actor pattern from the start; retrofitting reference-counted messages is a significant refactor.

---

### Pitfall 10: Ping/Pong Timer Cannot Fire Because Read Loop Blocks the Actor

**What goes wrong:**
Ping/pong heartbeat requires two concurrent activities within the same actor: (1) reading frames from the socket, and (2) sending Ping frames at regular intervals. In a traditional threaded model, these would be two threads or two async tasks. In Snow's coroutine model, an actor is single-threaded -- it executes sequentially. If the actor is blocked in `TcpStream::read()` waiting for a data frame, it cannot simultaneously check "has 30 seconds elapsed since the last ping?"

This is exactly the bug documented in the actix-web project (Issue #2441): "If an HTTP connection is severed at the client side without notification, and the WebSocket Actor sends a certain amount of messages, it becomes unable to run IntervalFuncs which could shut it down." The ping timer is starved because the actor is stuck in I/O.

**Why it happens:**
- Snow actors have no internal concurrency -- they are single-threaded coroutines
- There is no "select" or "poll" primitive that waits on multiple sources (socket + timer) simultaneously
- `TcpStream::read()` is blocking and does not cooperate with timer checks
- The `snow_timer_send_after` function (actor/mod.rs line 463) spawns an OS thread to deliver a timer message to the mailbox. But if the actor is blocked in `read()`, it never checks the mailbox to see the timer message

**Consequences:**
- Dead connections are never detected because ping never fires while read is blocking
- The timeout-based detection described in Pitfall 5 is the only mechanism that works, but it is coarse-grained (depends on the read timeout value)
- If read timeout is set to 30 seconds and ping interval is 30 seconds, the ping is always late (it can only fire after the read timeout triggers a yield)
- Combining read timeout + ping interval requires careful tuning: the read timeout must be significantly shorter than the ping interval

**Prevention:**
1. **Use the short read timeout approach (50-100ms) from Pitfall 2.** After each timeout, check: (a) is it time to send a Ping? (b) has the Pong deadline expired? (c) is there a message in the mailbox? This unified event loop handles both socket I/O and timers.
2. **Track `last_ping_sent: Instant` and `last_pong_received: Instant` in the actor's state.** On each read timeout cycle, check if `now - last_ping_sent > ping_interval` and send a Ping. Check if `now - last_pong_received > ping_interval + pong_timeout` and close the connection.
3. **Do NOT use `snow_timer_send_after` for ping scheduling.** It works for regular actors but fails for WebSocket actors because the actor cannot check its mailbox while blocked in read. Instead, drive the timer inline in the read loop.
4. **Design the WebSocket event loop as:**
   ```
   loop {
       // 1. Try to read a frame (short timeout)
       match read_frame_with_timeout(stream, 100ms) {
           Ok(frame) => handle_frame(frame),
           Err(Timeout) => { /* no data, continue */ }
           Err(other) => { break; /* connection error */ }
       }
       // 2. Check ping/pong timers
       if should_send_ping() { send_ping(stream); }
       if pong_overdue() { break; /* dead connection */ }
       // 3. Check mailbox for outgoing messages
       while let Some(msg) = mailbox.try_pop() {
           write_frame(stream, msg);
       }
       // 4. Yield to scheduler
       yield_current();
   }
   ```

**Detection:**
- Dead connections are only detected after the read timeout (e.g., 30 seconds) rather than the ping timeout (e.g., 10 seconds)
- Ping frames are never sent (observable via Wireshark)
- Connection cleanup takes much longer than expected

**Phase:** Ping/pong heartbeat. The event loop structure must be designed together with the frame loop (Pitfall 2).

---

### Pitfall 11: Control Frames Fragmented or Exceeding 125 Bytes

**What goes wrong:**
RFC 6455 Section 5.5 states: "All control frames MUST have a payload length of 125 bytes or less and MUST NOT be fragmented." Control frames are Close (0x8), Ping (0x9), and Pong (0xA). The WebSocketPP library (Issue #591) violated this by sending Close frame headers and payloads in separate TCP writes, which network stacks could split into separate packets, making it appear fragmented to the receiver. If the server sends a Ping with more than 125 bytes of payload, compliant clients must close the connection.

**Why it happens:**
- When writing a frame, it is natural to write the header first and the payload second with separate `write()` calls. TCP may send these as separate packets
- If the Ping payload includes diagnostic data (timestamps, connection IDs), it could exceed 125 bytes
- Close frames with a long reason string can exceed 125 bytes

**Prevention:**
1. **Write control frames atomically** -- assemble the complete frame (header + payload) in a buffer and write it in a single `write_all()` call.
2. **Validate payload length before sending control frames.** Close frames: 2 bytes (status code) + reason string <= 123 bytes. Ping/Pong: payload <= 125 bytes.
3. **Never set FIN=0 on a control frame.** Control frames are always complete in a single frame.

**Detection:**
- Clients disconnect with code 1002 after receiving an oversized or fragmented control frame
- Intermittent failures depending on TCP segmentation behavior

**Phase:** Frame writer. Simple to prevent if checked during implementation.

---

### Pitfall 12: Text Frames Not Validated as UTF-8

**What goes wrong:**
RFC 6455 Section 5.6 specifies that text frames (opcode 0x1) must contain valid UTF-8 text. The server must validate that incoming text frames are valid UTF-8 (after unmasking). If the payload is not valid UTF-8, the server must close the connection with code 1007 (Invalid Frame Payload Data). Similarly, when the server sends text frames, the payload must be valid UTF-8.

**Why it happens:**
- Binary frames (opcode 0x2) have no encoding requirement, so it is tempting to treat text and binary the same
- Rust strings are always valid UTF-8, so if the server converts the payload to a Rust `String`, invalid UTF-8 will cause a panic or error at the conversion point rather than a proper close with code 1007
- Snow strings (SnowString) are byte buffers with a length field -- they may or may not enforce UTF-8

**Prevention:**
1. **After unmasking a text frame, validate UTF-8** using `std::str::from_utf8()`. If it fails, send a Close frame with code 1007 and terminate.
2. **For fragmented text messages, validate UTF-8 on the reassembled complete message**, not on individual fragments (a multi-byte UTF-8 character can be split across fragments).
3. **When sending text frames from Snow code, ensure the payload is valid UTF-8.** If Snow strings can contain arbitrary bytes, provide both `ws_send_text` (validates UTF-8) and `ws_send_binary` (no validation) APIs.

**Detection:**
- Autobahn Testsuite will catch this immediately
- Clients sending non-UTF-8 in text frames cause panics or garbage instead of clean disconnects

**Phase:** Frame parser. Part of frame validation.

---

### Pitfall 13: 64 KiB Actor Stack Overflow During Large Message Reassembly

**What goes wrong:**
Snow actors run on 64 KiB corosensei stacks (stack.rs line 17: `DEFAULT_STACK_SIZE`). If the WebSocket frame reassembly buffer is allocated on the stack (e.g., `let mut buf = [0u8; 65536]` for reading frames), it immediately consumes the entire stack space, causing a stack overflow. Even smaller buffers (4KB-8KB for frame headers + payload read buffers) leave very little stack space for the rest of the actor's call chain.

The `parse_request` function for HTTP (server.rs line 247) uses `BufReader` and `String` which allocate on the heap, so this is not a problem for HTTP. But WebSocket frame parsing may involve fixed-size read buffers on the stack, and fragment reassembly requires accumulating arbitrarily large data.

**Why it happens:**
- 64 KiB is plenty for normal function call chains but is consumed quickly by large stack buffers
- Frame read buffers, fragment accumulation buffers, and masking buffers can all be on the stack
- Stack overflow in a coroutine manifests as a segfault, not a Rust panic, because corosensei stacks are manually allocated
- The actor heap (ActorHeap) is available for heap allocation, but the frame parser runs in Rust runtime code, not in Snow code, so it does not naturally use the actor heap

**Prevention:**
1. **Allocate all frame buffers on the Rust heap** (Vec, Box) rather than on the stack. Read into a small stack buffer (e.g., 4KB) and copy to a heap-allocated Vec.
2. **Fragment reassembly must use a heap-allocated Vec** that grows as fragments arrive.
3. **Keep the frame parser's stack footprint minimal.** The frame header is at most 14 bytes (2 byte header + 8 byte extended length + 4 byte mask key), so only a small stack buffer is needed for header parsing.
4. **Consider increasing the stack size for WebSocket actors** if needed, but prefer heap allocation to avoid coupling frame parser size to stack size.

**Detection:**
- Segfaults (not panics) when handling WebSocket connections
- Crashes that only occur with large messages or during fragment reassembly
- The crash happens inside the corosensei coroutine, making backtraces difficult to interpret

**Phase:** Frame parser implementation. Use heap allocation from the start.

---

### Pitfall 14: Room Membership Not Cleaned Up When Actor Exits

**What goes wrong:**
When a WebSocket connection actor exits (cleanly or due to crash), it must be removed from all rooms/channels it has joined. If room membership is stored as a `HashSet<ProcessId>` in a room manager actor, and the connection actor exits without explicitly leaving, the room retains the dead PID. Subsequent broadcasts to the room will attempt to send messages to the dead PID. The `snow_actor_send` function will silently fail (the process table lookup returns None, actor/mod.rs line 285), but the iteration cost is wasted.

Over time, rooms accumulate thousands of dead PIDs, making broadcast O(total_ever_joined) instead of O(currently_active). This is a memory and performance leak.

**Why it happens:**
- The actor may crash (panic caught by `catch_unwind`) and skip cleanup
- Network disconnection triggers an I/O error, not a clean "leave all rooms" sequence
- The `terminate_callback` mechanism (actor/mod.rs line 562) provides a cleanup hook, but it must be explicitly set and must know which rooms the actor has joined
- Room membership may be distributed: the room manager must be notified, but if the connection actor crashes, it cannot send a "leave" message

**Prevention:**
1. **Use actor linking.** Link the connection actor to the room manager actor. When the connection actor exits, the room manager receives an exit signal (delivered as a mailbox message with EXIT_SIGNAL_TAG) and can remove the dead PID from all rooms.
2. **Alternatively, set a `terminate_callback`** on the connection actor that sends "leave" messages to all joined rooms. The callback runs even on crash (scheduler.rs line 621: "invoke terminate_callback if set, wrapped in catch_unwind").
3. **Periodically prune dead PIDs from room membership.** Check each PID against the process table and remove Exited ones. This is a safety net, not the primary mechanism.
4. **Design rooms so that the room manager OWNS membership** and the connection actor only holds a reference to the room name/ID, not a membership slot. The room manager's exit signal handler does the cleanup.

**Detection:**
- Room broadcast slows down over time as dead PIDs accumulate
- Memory usage for room data structures grows without bound
- `snow_actor_send` to dead PIDs fails silently (no error, no delivery)

**Phase:** Rooms/channels implementation. Design the cleanup mechanism with room creation.

---

## Minor Pitfalls

Mistakes that cause suboptimal behavior or minor spec violations but are easy to fix.

---

### Pitfall 15: Extended Payload Length Encoding Off-By-One

**What goes wrong:**
WebSocket frame payload length encoding has three tiers: 0-125 bytes use the 7-bit length field directly. 126-65535 bytes use 126 as the 7-bit value followed by a 16-bit big-endian length. 65536+ bytes use 127 as the 7-bit value followed by a 64-bit big-endian length. Common bugs include using 126 for payloads > 65535 (truncating the length to 16 bits) or encoding the extended length as little-endian instead of big-endian.

**Prevention:**
1. **Use network byte order (big-endian)** for the 16-bit and 64-bit extended length fields. In Rust: `len.to_be_bytes()`.
2. **Test with payloads of exactly 125, 126, 65535, and 65536 bytes** to exercise all three length tiers.
3. **Test with large payloads (>65536 bytes)** to verify the 64-bit encoding.

**Phase:** Frame parser/writer. Simple to get right with boundary tests.

---

### Pitfall 16: Blocking TLS Handshake for WSS Connections Delays All Pending Connections

**What goes wrong:**
For WSS connections, the TLS handshake happens inside the connection actor (server.rs line 472: "The TLS handshake is lazy: StreamOwned::new() does NO I/O"). This is correct and already solved for HTTPS. But WebSocket connections may arrive in bursts (e.g., page load with multiple WebSocket connections). If the scheduler only has 4 worker threads and 10 WSS connections arrive simultaneously, 6 connections wait in the queue while 4 undergo TLS handshake (50-200ms each). The queued connections see a delay of 100-400ms before their handshake even begins.

This is the v3.0 Pitfall 2 (TLS handshake blocking) applied to WebSocket connections, but worse because WebSocket connections are long-lived. For HTTP, the TLS handshake is amortized over one request. For WebSocket, the handshake is amortized over potentially hours of communication, so the initial burst cost is more acceptable.

**Prevention:**
- The existing architecture (lazy TLS handshake inside actor) is correct for WebSocket. No architectural change needed.
- If burst connection latency is a concern, consider increasing worker thread count or moving TLS handshake to a dedicated thread pool. But this is an optimization, not a correctness issue.

**Phase:** TLS integration. The existing HTTPS architecture already handles this correctly.

---

### Pitfall 17: Outgoing Message Write Blocks When Network Buffer Is Full

**What goes wrong:**
When the server writes a WebSocket frame to a client with a slow or congested connection, `TcpStream::write()` blocks until the OS send buffer has space. If the send buffer is full (client is not reading), the write blocks the entire worker thread. For broadcasts to large rooms, one slow client can block the entire broadcast, delaying messages to all other clients.

This is related to the websockets library's documentation on broadcast: "A naive broadcast approach will block until the slowest client times out."

**Prevention:**
1. **Set a write timeout on the TcpStream** (e.g., 5 seconds). If a write times out, consider the client dead and close the connection.
2. **For broadcasts, skip clients that have a pending write backlog.** Track whether each connection has undelivered outgoing messages and skip it during broadcast if so.
3. **Consider per-connection outgoing buffers** with a maximum size. If the buffer exceeds the limit, close the connection as unresponsive.

**Phase:** Frame writer / broadcast implementation.

---

### Pitfall 18: Binary and Text Frame Opcodes Mixed Up in Response

**What goes wrong:**
If a client sends a text frame (opcode 0x1) and the server responds with opcode 0x2 (binary), the client may handle the response differently (e.g., delivering it as an ArrayBuffer instead of a string in JavaScript). Conversely, responding to a binary frame with a text opcode may cause UTF-8 validation failures on the client.

**Prevention:**
1. **Track the opcode of the received message and use the same opcode for responses** where applicable, or let the Snow application explicitly choose text vs binary.
2. **Provide separate `ws_send_text` and `ws_send_binary` APIs** in the Snow runtime. Do not auto-detect.

**Phase:** Frame writer API design.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| HTTP Upgrade Handshake | Pitfall 1 (connection lifecycle), Pitfall 3 (Sec-WebSocket-Accept), Pitfall 4 (masking direction) | Redesign connection handler; unit test with RFC example values; test with browser |
| Frame Parser/Writer | Pitfall 6 (fragmentation), Pitfall 11 (control frames), Pitfall 12 (UTF-8), Pitfall 13 (stack overflow), Pitfall 15 (length encoding) | State machine design; heap allocation; boundary tests; Autobahn Testsuite |
| Actor Integration / Event Loop | Pitfall 2 (blocking read), Pitfall 7 (type tag collision), Pitfall 10 (timer starvation) | Short read timeout + yield loop; reserved type tag ranges; inline timer checks |
| Ping/Pong Heartbeat | Pitfall 5 (connection leak), Pitfall 10 (timer cannot fire) | Heartbeat driven by read timeout cycle, not by mailbox timer |
| Rooms/Channels | Pitfall 9 (broadcast contention), Pitfall 14 (membership cleanup) | Room actor pattern; actor linking for cleanup; reference-counted messages |
| Close Handshake | Pitfall 8 (state machine), Pitfall 4 (masking direction on Close frames) | Implement close state machine; validate close status codes |
| TLS (WSS) | Pitfall 16 (handshake burst delay) | Existing HTTPS architecture is adequate; optimize only if measured |
| Outgoing Messages | Pitfall 17 (write backlog), Pitfall 18 (opcode mismatch) | Write timeouts; separate text/binary APIs |

---

## Sources

### RFC and Protocol References
- [RFC 6455: The WebSocket Protocol](https://datatracker.ietf.org/doc/html/rfc6455) -- PRIMARY SOURCE for all protocol behavior
- [Sec-WebSocket-Accept header - MDN](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-WebSocket-Accept)
- [WebSocket Close Codes Reference](https://websocket.org/reference/close-codes/)

### Real-World Implementation Bugs
- [actix-web Issue #2441: WebSocket actor paralyzed by lack of IO timeout](https://github.com/actix/actix-web/issues/2441) -- connection leak via blocked actor
- [Bun Issue #3742: Continuation frame handling failure](https://github.com/oven-sh/bun/issues/3742) -- fragmentation bug
- [aiohttp PR #1962: Fix fragmented frame handling](https://github.com/aio-libs/aiohttp/pull/1962) -- fragmentation and control frame interleaving
- [GNS3 Issue #2320: Server fails to initiate TCP close](https://github.com/GNS3/gns3-server/issues/2320) -- close handshake violation
- [Ktor Issue #423: Client masking disabled by default](https://github.com/ktorio/ktor/issues/423) -- masking direction
- [ESP-IDF Issue #18227: PONG frame 50% failure rate](https://github.com/espressif/esp-idf/issues/18227) -- mask key bit handling
- [WebSocketPP Issue #591: Control frames sent fragmented](https://github.com/zaphoyd/websocketpp/issues/591) -- control frame fragmentation
- [Jetty Issue #2491: FragmentExtension producing invalid frame streams](https://github.com/jetty/jetty.project/issues/2491)
- [faye-websocket-node Issue #48: Malformed frames from TCP fragmentation](https://github.com/faye/faye-websocket-node/issues/48)
- [.NET Runtime Issue #100771: Ping/Pong frame handling abstracted away](https://github.com/dotnet/runtime/issues/100771)
- [reactor-netty Issue #1891: Invalid close status code 1005](https://github.com/reactor/reactor-netty/issues/1891)

### Architecture and Design
- [WebSocket Framing: Masking, Fragmentation and More](https://www.openmymind.net/WebSocket-Framing-Masking-Fragmentation-and-More/) -- excellent frame format walkthrough
- [websockets library keepalive documentation](https://websockets.readthedocs.io/en/stable/topics/keepalive.html) -- ping/pong timing defaults
- [How to Close a WebSocket (Correctly)](https://mcguirev10.com/2019/08/17/how-to-close-websocket-correctly.html) -- close handshake state machine
- [WebSocket architecture best practices - Ably](https://ably.com/topic/websocket-architecture-best-practices) -- room/channel patterns
- [websockets library broadcast documentation](https://websockets.readthedocs.io/en/stable/topics/broadcast.html) -- backpressure in broadcast
- [Tokio Discussion #6175: Fairness and starvation](https://github.com/tokio-rs/tokio/discussions/6175) -- cooperative scheduling starvation
- [Why Actors Are Perfect for WebSockets](https://redandgreen.co.uk/why-actors-are-perfect-for-websockets/rust-programming/) -- actor-WebSocket patterns
- [How to Fix Invalid Frame Header WebSocket Errors](https://oneuptime.com/blog/post/2026-01-24-websocket-invalid-frame-header/view) -- frame parsing errors
- [How to Configure WebSocket Heartbeat/Ping-Pong](https://oneuptime.com/blog/post/2026-01-24-websocket-heartbeat-ping-pong/view) -- timeout tuning

### Snow Codebase References
- `crates/snow-rt/src/http/server.rs` -- HttpStream, parse_request, write_response, connection_handler_entry
- `crates/snow-rt/src/actor/scheduler.rs` -- M:N scheduler, worker_loop, thread-pinned coroutines
- `crates/snow-rt/src/actor/mod.rs` -- snow_actor_send, snow_actor_receive, mailbox delivery, type_tag derivation
- `crates/snow-rt/src/actor/mailbox.rs` -- FIFO mailbox with Mutex<VecDeque<Message>>
- `crates/snow-rt/src/actor/process.rs` -- Process, ProcessState, MessageBuffer, EXIT_SIGNAL_TAG
- `crates/snow-rt/src/actor/stack.rs` -- corosensei coroutines, 64 KiB stacks, yield_current
