# Project Research Summary

**Project:** Snow compiler -- WebSocket server (ws:// and wss://), RFC 6455 frame codec, actor-per-connection, rooms/channels, heartbeat
**Domain:** WebSocket support for compiled language with actor runtime
**Researched:** 2026-02-12
**Confidence:** HIGH

## Executive Summary

WebSocket support for Snow requires extending the existing HTTP server infrastructure with RFC 6455 frame codec, long-lived bidirectional connections, and pub/sub rooms. The recommended approach leverages Snow's proven patterns: actor-per-connection model (reusing HTTP server architecture), hand-rolled frame parser (consistent with HTTP/PostgreSQL wire protocol implementations), and reader-thread-to-mailbox bridge for unified event handling. The implementation is additive -- a new `http/ws.rs` module with compiler registration, requiring only ONE new dependency (`sha1 0.10` for the upgrade handshake).

The critical architectural challenge is reconciling WebSocket's long-lived connections with Snow's M:N cooperative scheduler. Unlike HTTP's request-response-close model, WebSocket actors must simultaneously handle socket I/O and actor messages indefinitely without blocking worker threads. The solution: reader thread per connection that delivers frames to actor mailbox + short read timeouts (50-100ms) with yield cycles. This enables ping/pong heartbeat, dead connection detection, and room broadcasts without scheduler starvation.

Key risks are protocol compliance (masking direction, close handshake, fragmentation) and resource management (connection leaks, room cleanup, broadcast contention). All are mitigated through established patterns: RFC test vectors for handshake, Autobahn Testsuite for frames, actor linking for cleanup, and RwLock registry for rooms. The architecture positions WebSocket as a peer to HTTP (separate servers on separate ports), avoiding invasive changes to existing infrastructure.

## Key Findings

### Recommended Stack

Snow's WebSocket implementation reuses 90% of existing infrastructure and adds only one new direct dependency. The hand-rolled approach (frame codec + upgrade handshake) is consistent with the codebase philosophy and adds ~600-800 lines to `http/ws.rs`.

**Core technologies:**
- **sha1 0.10**: SHA-1 digest for WebSocket upgrade handshake (`Sec-WebSocket-Accept` computation) -- RustCrypto project, same family as existing `sha2`, pure Rust, minimal API surface
- **Hand-rolled RFC 6455 frame codec**: Parse and serialize WebSocket frames (~200 lines) -- consistent with hand-rolled HTTP/1.1 parser and PostgreSQL wire protocol
- **Existing dependencies reused**: `base64 0.22` (upgrade handshake), `rustls 0.23` (wss:// via WsStream::Tls), `parking_lot 0.12` (room registry + stream mutex), `rustc-hash` (room subscriber sets)

**Alternatives rejected:**
- tungstenite: Pulls 6+ crates including `http`, `httparse`, `thiserror`. Snow already has HTTP parsing and SHA-1 available. Frame parsing is simpler than what Snow has already implemented.
- ring for SHA-1: Works but large API surface. `sha1 0.10` is purpose-built, small, and from the same RustCrypto family.
- Room actor (message-passing): Centralized bottleneck for broadcast. Lock-based registry with direct `snow_actor_send()` fan-out is faster and matches existing process registry pattern.

### Expected Features

Research identified 12 features across three tiers: table stakes (8), differentiators (4), and anti-features (6 to avoid).

**Must have (table stakes):**
- HTTP upgrade handshake (RFC 6455 Section 4): `Sec-WebSocket-Accept` computation, 101 Switching Protocols
- WebSocket frame parsing: 2-14 byte header, 3 payload length encodings, client-to-server unmasking
- Text/binary frame types: opcode 0x1 (UTF-8 validated), opcode 0x2 (raw bytes)
- Close handshake (opcode 0x8): Two-phase close with status codes, clean TCP shutdown
- Ping/pong heartbeat (opcodes 0x9/0xA): Detects dead connections, auto-Pong response, configurable interval
- Actor-per-connection with mailbox integration: WS frames arrive as actor messages, unified `receive` loop
- Connection lifecycle callbacks: `on_connect`, `on_message`, `on_close` for user-facing API
- `Ws.serve` / `Ws.serve_tls` entry points: Separate server (not mixed with HTTP router)

**Should have (competitive):**
- Rooms/channels with join/leave/broadcast: Named groups for chat, notifications, multiplayer -- core WebSocket value proposition
- Message fragmentation (RFC 6455 Section 5.4): Reassemble multi-frame messages, handle interleaved control frames
- Subprotocol negotiation (`Sec-WebSocket-Protocol`): Support GraphQL subscriptions, MQTT-over-WS, protocol versioning
- Configurable connection limits and backpressure: Max concurrent connections, send buffer limits, rate limiting

**Defer (anti-features for v1):**
- Per-message deflate compression (RFC 7692): 300KB/connection overhead, 4-parameter negotiation, minimal benefit for small JSON messages
- HTTP/WebSocket dual-protocol on same port: Clean separation is better; users run `Http.serve` on one port, `Ws.serve` on another
- WebSocket client library: Server-only focus; client use cases are niche
- WebTransport/HTTP/3: No browser implementations as of 2025

### Architecture Approach

The WebSocket implementation is entirely additive -- a new `http/ws.rs` module (~600-800 lines) plus compiler registration. No structural changes to HTTP server, actor runtime, or scheduler. The architecture follows proven patterns from the codebase.

**Major components:**
1. **WsStream enum**: Reuses `HttpStream` pattern (`Plain(TcpStream)` / `Tls(StreamOwned)`) for transparent TLS dispatch. Separate type avoids coupling to HTTP server internals.
2. **Reader thread bridge**: Each connection spawns an OS thread that blocks on `read_frame()` and delivers frames to the actor mailbox via `snow_actor_send()`. The connection actor uses standard `snow_actor_receive()` for both WS frames AND actor messages in a unified mailbox. Rationale: WebSocket readers block indefinitely (hours), not suitable for M:N coroutines. Same pattern as `snow_timer_send_after` OS thread.
3. **WsConn handle (u64)**: GC-safe connection state (`Mutex<WsStream>` for writer, PID, shutdown flag, path) exposed as opaque handle. Same pattern as `PgConn`, `SqliteConn`, `PgPool`.
4. **Room registry**: Global `RwLock<HashMap<String, HashSet<ProcessId>>>` for named groups. Join adds PID to set, broadcast iterates set and calls `snow_actor_send()` directly. Cleanup via actor linking and terminate callbacks.
5. **Inline ping/pong timer**: Driven by short read timeout cycles (50-100ms) rather than `snow_timer_send_after` mailbox messages. The actor tracks `last_ping_sent`/`last_pong_received` and checks timers on each read timeout iteration. Avoids the "ping can't fire while read is blocking" pitfall.

### Critical Pitfalls

**1. HTTP Upgrade Handshake Fails Because Current Parser Consumes and Discards the Connection**
The current `connection_handler_entry` follows request-response-done lifecycle: parse, handle, respond, exit. No mechanism for handlers to say "keep this connection alive for WebSocket." Prevention: Redesign to detect `Upgrade: websocket` headers at runtime level, validate handshake, send 101 with correct headers (not through `write_response`), then transition actor into WebSocket frame loop. Phase: HTTP upgrade handshake (foundational).

**2. WebSocket Actor Blocks Worker Thread Indefinitely During Frame Read Loop**
Blocking `TcpStream::read()` for next WebSocket frame can wait hours, starving all actors pinned to that worker thread (coroutines are `!Send` and thread-pinned). With 4 worker threads, 4 idle connections can halt the entire runtime. Prevention: Set short read timeout (50-100ms), yield to scheduler on timeout, create cooperative blocking pattern: brief block, check results, yield, repeat. Phase: Core frame loop (must be solved with parser).

**3. Sec-WebSocket-Accept Computed from Hex Digest Instead of Raw Binary**
The most common WebSocket handshake bug across dozens of libraries: base64-encoding the hex string representation of SHA-1 instead of raw 20 bytes. Produces 56-char result instead of correct 28-char result. Every connection fails silently during handshake. Prevention: Use `sha1::Sha1::digest()` raw bytes directly, never convert to hex. Unit test with RFC example values: key `dGhlIHNhbXBsZSBub25jZQ==` must produce accept `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`. Phase: HTTP upgrade handshake.

**4. Frame Masking Direction Reversed (Server Masks, Client Unmasks)**
RFC mandates asymmetric masking: clients MUST mask, servers MUST NOT mask. Browser clients close with code 1002 if they receive masked server frames. 50% failure rates when mask key has specific bit patterns (ESP-IDF bug). Prevention: In frame writer, NEVER set mask bit for server-to-client. In frame reader, ALWAYS check mask bit on client-to-server. Separate code paths for reading/writing. Test with real browsers. Phase: Frame parser/writer.

**5. WebSocket Connection Actor Leaks When Client Disconnects Without Close Frame**
Network failures, browser tab closes, NAT timeouts leave ghost actors blocking on `read()` indefinitely. Without ping/pong heartbeat, actors accumulate consuming PIDs, memory, room memberships. Prevention: Implement server-side ping/pong heartbeat (30s ping, 10s pong timeout). Use short read timeout approach so ping timer is not blocked by read loop. Clean up room membership via terminate callbacks. Phase: Ping/pong heartbeat (implement immediately after frame loop).

**6. Fragmented Messages Not Reassembled, Causing Data Corruption**
RFC allows messages split across multiple frames (FIN=0 + continuation opcode 0x0). Control frames (Ping/Pong/Close) can interleave. Large messages (>64KB) arrive corrupted without proper reassembly. Prevention: State machine (Idle / ReceivingFragments), handle control frames independently, buffer partial TCP reads, enforce max message size (16MB), validate continuation opcodes. Test with Autobahn Testsuite. Phase: Frame parser (must be in initial implementation).

**7. Mailbox Message Type Collision Between WS Frames and Actor Messages**
WebSocket messages delivered to actor mailbox need type tags to distinguish from actor-to-actor messages. The current first-8-bytes derivation can collide with user content or EXIT_SIGNAL_TAG. Prevention: Reserved type tag ranges: `WS_TEXT_TAG = 0xFFFF_FFFF_0001`, `WS_BINARY_TAG = 0xFFFF_FFFF_0002`, etc. Upper 32 bits (0xFFFF_FFFF) for runtime system messages, lower 32 bits for user-defined types. Phase: Mailbox integration (design before writing delivery code).

## Implications for Roadmap

Based on research, suggested phase structure follows dependency chains and enables incremental testing:

### Phase 1: Protocol Core (Frame Codec + Upgrade Handshake)
**Rationale:** Foundation -- everything depends on frame I/O and upgrade working correctly. WebSocket is fundamentally a wire protocol transformation on top of HTTP/1.1. Get the protocol implementation solid before adding application features.

**Delivers:**
- `WsStream` enum (Plain/Tls)
- `read_frame()` / `write_frame()` -- RFC 6455 frame parser (2-14 byte header, 3 length encodings, masking, opcodes)
- HTTP upgrade validation and 101 response writer
- SHA-1 + base64 for `Sec-WebSocket-Accept`
- Text/binary frame types with UTF-8 validation
- Close handshake (opcode 0x8, status codes, two-phase close)

**Addresses features:**
- Feature 1: HTTP Upgrade Handshake
- Feature 2: WebSocket Frame Parsing
- Feature 3: Text and Binary Frame Types
- Feature 4: Close Handshake

**Avoids pitfalls:**
- Pitfall 3: Sec-WebSocket-Accept hex vs binary (RFC test vector)
- Pitfall 4: Frame masking direction (separate read/write paths)
- Pitfall 11: Control frames >125 bytes or fragmented
- Pitfall 12: Text frames not UTF-8 validated
- Pitfall 13: Stack overflow from large buffers (use heap allocation)
- Pitfall 15: Extended payload length encoding (boundary tests)

**Test strategy:** Unit tests for frame round-trip, RFC handshake example values, Autobahn Testsuite for frame compliance.

**Estimated complexity:** Medium. Frame format is precisely specified but has edge cases (3 length tiers, masking). Upgrade handshake reuses existing parser.

---

### Phase 2: Actor Integration (Connection Lifecycle + Reader Thread Bridge)
**Rationale:** Establishes Snow's unique actor-per-connection model. The reader thread bridge solves the fundamental problem: WebSocket's long-lived blocking I/O on M:N cooperative scheduler. Must be designed together with frame loop (Pitfall 2).

**Delivers:**
- `snow_ws_serve()` accept loop (copy `snow_http_serve` pattern)
- `ws_connection_handler_entry()` with upgrade + reader thread spawn
- Reader thread: blocking `read_frame()` loop sending to actor mailbox
- `WsConn` handle creation and lifecycle
- `snow_ws_send()` / `snow_ws_send_binary()` / `snow_ws_close()`
- Short read timeout (50-100ms) + yield cycle for cooperative blocking
- Reserved type tags for WS mailbox messages (0xFFFF_FFFF_000X range)

**Addresses features:**
- Feature 6: Actor-per-Connection with Mailbox Integration
- Feature 7: Connection Lifecycle Callbacks
- Feature 8: Ws.serve Entry Points

**Avoids pitfalls:**
- Pitfall 1: Connection lifecycle (redesigned handler entry)
- Pitfall 2: Blocking worker thread (short timeout + yield)
- Pitfall 7: Mailbox type tag collision (reserved ranges)
- Pitfall 8: Close handshake state machine

**Uses stack:**
- Existing actor runtime: `snow_actor_spawn`, `snow_actor_send`, `snow_actor_receive`
- `std::thread::spawn` for reader thread (same pattern as `snow_timer_send_after`)
- `WsConn` as u64 handle (GC-safe pattern from `PgConn`, `SqliteConn`)

**Test strategy:** E2E test with simple echo WebSocket server, verify reader thread delivers frames to mailbox, test crash isolation (`catch_unwind`).

**Estimated complexity:** High. Reader thread bridge is novel architecture. Event loop design (read timeout + yield + timer checks) requires careful tuning.

---

### Phase 3: Production Hardening (TLS + Heartbeat + Fragmentation)
**Rationale:** Makes it production-ready. TLS is straightforward (reuse existing rustls infrastructure). Heartbeat is critical for detecting dead connections. Fragmentation is required for RFC compliance and large message support.

**Delivers:**
- `snow_ws_serve_tls()` -- reuse `build_server_config` from HTTP server
- `WsStream::Tls` variant with `Mutex<WsStream>` for reader/writer coordination
- Inline ping/pong timer (driven by read timeout cycles, not mailbox messages)
- Auto-Pong response in reader thread (immediate response before mailbox delivery)
- Server-initiated Ping (30s interval, 10s pong timeout)
- Connection cleanup on missed Pongs
- Message fragmentation reassembly (state machine: Idle / ReceivingFragments)
- Interleaved control frame handling
- Max message size limit (16MB default, close with 1009 if exceeded)

**Addresses features:**
- Feature 5: Ping/Pong Heartbeat
- Feature 10: Message Fragmentation
- TLS support (part of Feature 8: Ws.serve_tls)

**Avoids pitfalls:**
- Pitfall 5: Connection leaks (heartbeat detects dead connections)
- Pitfall 6: Fragmented messages (reassembly state machine)
- Pitfall 10: Ping timer starvation (inline timer checks, not mailbox)
- Pitfall 16: TLS handshake burst delay (existing architecture handles this)

**Test strategy:** E2E with self-signed cert, test heartbeat timeout with killed client, Autobahn Testsuite for fragmentation cases.

**Estimated complexity:** Medium. TLS reuses proven patterns. Heartbeat driven by timeout cycle (designed in Phase 2). Fragmentation requires state machine but well-specified.

---

### Phase 4: Application Layer (Rooms + Compiler Integration)
**Rationale:** Higher-level features for real-world patterns. Rooms enable the multiplayer/chat/notification use cases that make WebSocket valuable. Compiler integration unlocks Snow-language-level usage.

**Delivers:**
- Global room registry (`RwLock<HashMap<String, HashSet<ProcessId>>>`)
- `snow_ws_join()` / `snow_ws_leave()` / `snow_ws_broadcast()`
- Auto-cleanup on actor exit (terminate callback + actor linking)
- Reverse index (PID -> rooms) for efficient cleanup
- Reference-counted messages for broadcast (Arc<Vec<u8>> instead of N copies)
- Register all functions in `intrinsics.rs`, `builtins.rs`, `lower.rs`
- Subprotocol negotiation (`Sec-WebSocket-Protocol` header)
- Connection limits (max concurrent, per-connection send buffer)

**Addresses features:**
- Feature 9: Rooms/Channels with Join/Leave/Broadcast
- Feature 11: Subprotocol Negotiation
- Feature 12: Configurable Connection Limits

**Avoids pitfalls:**
- Pitfall 9: Broadcast contention (direct send fan-out, reference-counted messages)
- Pitfall 14: Room membership leaks (terminate callback + linking)
- Pitfall 17: Write backlog (write timeouts, skip slow clients)

**Test strategy:** Multi-connection broadcast test, verify cleanup after disconnect, load test with 1000 connections.

**Estimated complexity:** Medium. Room registry follows existing process registry pattern. Compiler integration is mechanical (15 functions).

---

### Phase Ordering Rationale

**Dependency chains:**
- Phase 1 (protocol core) is foundational -- everything depends on frame I/O working
- Phase 2 (actor integration) requires Phase 1 frames but implements the unique Snow model
- Phase 3 (production hardening) builds on Phase 2 event loop (heartbeat driven by timeout cycle)
- Phase 4 (application layer) requires working connections (Phase 2-3) to test rooms

**Risk mitigation:**
- Most critical pitfalls (1-7) addressed in Phases 1-2 before adding features
- Fragmentation (Pitfall 6) in Phase 3 because it requires state machine in frame parser
- Room leaks (Pitfall 14) addressed in Phase 4 with room implementation

**Incremental value:**
- Phase 1: Can test upgrade handshake + basic frame I/O with external tools
- Phase 2: Fully functional WebSocket server (echo, simple apps)
- Phase 3: Production-ready (TLS, heartbeat, large messages)
- Phase 4: Real-world features (chat, multiplayer, notifications)

**Architectural coherence:**
- Phase 2 establishes the event loop structure (timeout + yield + inline timers)
- Phase 3 heartbeat naturally fits into Phase 2 structure (no refactor)
- Phase 4 builds on stable connection model from Phase 2-3

### Research Flags

**Phases likely needing deeper research during planning:**
- **Phase 2 (Actor Integration):** Reader thread bridge is novel architecture. May need research on optimal read timeout value, yield frequency, and mailbox type tag allocation scheme. Prototype recommended.
- **Phase 3 (Fragmentation):** RFC 6455 Section 5.4 is precise but complex (interleaved control frames, partial TCP reads). Study Autobahn Testsuite expectations and reference implementations (tungstenite frame parser patterns).

**Phases with standard patterns (skip research-phase):**
- **Phase 1 (Protocol Core):** RFC 6455 is definitive. Frame format is precisely specified. Upgrade handshake algorithm is deterministic.
- **Phase 3 (TLS):** Reuses existing `rustls 0.23` infrastructure from HTTP server. Known pattern.
- **Phase 4 (Rooms):** Follows existing process registry pattern (`actor/registry.rs`). Broadcast is well-studied (Phoenix Channels, Socket.IO).

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Codebase analysis verified all dependencies exist or are minimal additions. RFC 6455 is authoritative for protocol requirements. Hand-rolled approach is proven (HTTP, PostgreSQL wire protocol). |
| Features | HIGH | WebSocket protocol is mature (RFC from 2011). Table stakes features cross-verified with MDN, production libraries (tungstenite, websockets), and Phoenix Channels. Autobahn Testsuite provides definitive compliance testing. |
| Architecture | HIGH | Based on direct inspection of all relevant snow-rt modules (server.rs, scheduler.rs, actor/mod.rs, process.rs, stack.rs). Reader thread bridge follows existing pattern (`snow_timer_send_after`). GC-safe handle pattern proven in 3 places (pg.rs, sqlite.rs, pool.rs). |
| Pitfalls | HIGH | 18 pitfalls sourced from real-world bugs in production systems (actix-web, Bun, aiohttp, GNS3, ESP-IDF, WebSocketPP, Ktor). Each pitfall has documented prevention strategy and detection method. Critical path pitfalls (1-7) aligned with phase structure. |

**Overall confidence:** HIGH

Research is backed by:
- Authoritative RFC 6455 specification
- Direct codebase analysis of Snow runtime (all modules inspected)
- Real-world bug analysis from 10+ production WebSocket implementations
- Cross-verification with 3+ established libraries (tungstenite, websockets, Phoenix)
- Autobahn Testsuite for protocol compliance testing

### Gaps to Address

**Architecture gaps:**
1. **Optimal read timeout value**: Research suggests 50-100ms but actual value depends on scheduler load and target latency. Should be configurable or auto-tuned. Addressed during Phase 2 implementation with benchmarking.
2. **Broadcast scalability at 10K+ connections**: Reader-thread-per-connection model works up to ~5K-10K connections. Beyond that, need epoll/io_uring reader pool. Document connection limit for v1, plan epoll upgrade for v2 if demand warrants.
3. **TLS stream splitting for wss://**: `StreamOwned` cannot be cloned for separate read/write. Research proposes `Mutex<WsStream>` shared between reader thread and actor. Needs prototype to verify performance. Fallback: plain TCP uses `try_clone()`, TLS uses Mutex.

**Feature gaps:**
1. **Subprotocol negotiation priority**: RFC 6455 specifies server picks from client's list, but doesn't mandate "first match" vs "priority order." Check GraphQL subscriptions (`graphql-ws`) and MQTT-over-WebSocket for conventions. Address during Phase 4 implementation.
2. **Connection rate limiting algorithm**: Research mentions token-bucket or fixed-window but doesn't specify which. Use simple fixed-window (count per second) for v1, upgrade to token-bucket if needed. Phase 4 decision.

**Pitfall validation:**
1. **Autobahn Testsuite coverage**: Verify that Autobahn covers all identified pitfalls (especially fragmentation, masking, close handshake). Run full suite during Phase 1 implementation to confirm test harness is adequate.
2. **Reader thread vs coroutine decision**: Research strongly recommends OS thread for long-lived blocking I/O. But should prototype "non-blocking read with short timeout in coroutine" to empirically verify it causes scheduler starvation. Phase 2 prototyping task.

None of these gaps block starting Phase 1. All are addressable during implementation with validation testing.

## Sources

### Primary (HIGH confidence)
- [RFC 6455: The WebSocket Protocol](https://datatracker.ietf.org/doc/html/rfc6455) -- Authoritative protocol specification for handshake, frame format, close codes, fragmentation
- Snow codebase: `crates/snow-rt/src/http/server.rs` -- HttpStream enum, parse_request, connection_handler_entry, actor-per-connection pattern
- Snow codebase: `crates/snow-rt/src/actor/mod.rs` -- snow_actor_spawn/send/receive, timer_send_after OS thread pattern, type_tag derivation
- Snow codebase: `crates/snow-rt/src/actor/scheduler.rs` -- M:N scheduler, worker loop, !Send coroutines, thread-pinned execution
- Snow codebase: `crates/snow-rt/src/db/pg.rs` -- PgStream enum (Plain/Tls pattern), GC-safe u64 handle pattern
- Snow codebase: `crates/snow-rt/Cargo.toml` + `Cargo.lock` -- Current dependency versions verified

### Secondary (MEDIUM confidence)
- [tungstenite-rs GitHub](https://github.com/snapview/tungstenite-rs) -- Reference frame codec patterns
- [sha1 crate on crates.io](https://crates.io/crates/sha1) -- RustCrypto SHA-1 API
- [Phoenix Channels documentation](https://hexdocs.pm/phoenix/channels.html) -- Rooms/topics pattern for BEAM actors
- [Socket.IO Rooms documentation](https://socket.io/docs/v3/rooms/) -- Rooms pattern reference
- [WebSock (Elixir)](https://github.com/phoenixframework/websock) -- Actor-per-WebSocket specification
- [MDN: Writing WebSocket servers](https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API/Writing_WebSocket_servers) -- Implementation guide
- [Autobahn WebSocket Testsuite](https://github.com/crossbario/autobahn-testsuite) -- Comprehensive protocol compliance tests

### Real-World Bug Analysis (MEDIUM confidence)
- [actix-web Issue #2441](https://github.com/actix/actix-web/issues/2441) -- WebSocket actor paralyzed by lack of IO timeout (Pitfall 2, 5, 10)
- [Bun Issue #3742](https://github.com/oven-sh/bun/issues/3742) -- Continuation frame handling failure (Pitfall 6)
- [aiohttp PR #1962](https://github.com/aio-libs/aiohttp/pull/1962) -- Fragmented frame handling bugs (Pitfall 6)
- [ESP-IDF Issue #18227](https://github.com/espressif/esp-idf/issues/18227) -- PONG frame 50% failure rate from mask key bits (Pitfall 4)
- [GNS3 Issue #2320](https://github.com/GNS3/gns3-server/issues/2320) -- Server fails to initiate TCP close (Pitfall 8)
- [Ktor Issue #423](https://github.com/ktorio/ktor/issues/423) -- Client masking disabled by default (Pitfall 4)
- [WebSocketPP Issue #591](https://github.com/zaphoyd/websocketpp/issues/591) -- Control frames sent fragmented (Pitfall 11)
- [faye-websocket-node Issue #48](https://github.com/faye/faye-websocket-node/issues/48) -- Malformed frames from TCP fragmentation (Pitfall 6)

### Tertiary (LOW confidence, context only)
- [OpenMyMind: WebSocket Framing](https://www.openmymind.net/WebSocket-Framing-Masking-Fragmentation-and-More/) -- Frame format walkthrough
- [websockets library keepalive docs](https://websockets.readthedocs.io/en/stable/topics/keepalive.html) -- Ping/pong timing defaults (30s recommended)
- [Ably: WebSocket Architecture Best Practices](https://ably.com/topic/websocket-architecture-best-practices) -- Scalability patterns
- [websockets library broadcast docs](https://websockets.readthedocs.io/en/stable/topics/broadcast.html) -- Backpressure in broadcast

---
*Research completed: 2026-02-12*
*Ready for roadmap: yes*
