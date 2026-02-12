# Requirements: Snow v4.0 WebSocket Support

**Defined:** 2026-02-12
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v4.0 Requirements

Requirements for WebSocket support milestone. Each maps to roadmap phases.

### Protocol Core

- [ ] **PROTO-01**: WebSocket server performs HTTP upgrade handshake (101 Switching Protocols) with correct Sec-WebSocket-Accept computation per RFC 6455
- [ ] **PROTO-02**: WebSocket server parses incoming frames (2-14 byte header, 3 payload length encodings, FIN bit, opcodes)
- [ ] **PROTO-03**: WebSocket server unmasks client-to-server frames using 4-byte XOR key per RFC 6455
- [ ] **PROTO-04**: WebSocket server writes unmasked server-to-client frames
- [ ] **PROTO-05**: WebSocket server handles text frames (opcode 0x1) with UTF-8 validation
- [ ] **PROTO-06**: WebSocket server handles binary frames (opcode 0x2) with raw byte delivery
- [ ] **PROTO-07**: WebSocket server implements close handshake (opcode 0x8) with status codes and two-phase close
- [ ] **PROTO-08**: WebSocket server rejects malformed upgrade requests with HTTP 400
- [ ] **PROTO-09**: WebSocket server rejects unknown opcodes with close code 1002

### Actor Integration

- [ ] **ACTOR-01**: Each WebSocket connection spawns a dedicated actor with crash isolation via catch_unwind
- [ ] **ACTOR-02**: WebSocket frames are delivered to actor mailbox as typed messages with reserved type tags (no collision with actor-to-actor messages)
- [ ] **ACTOR-03**: Actor can send text frames back to its client via Ws.send(conn, message)
- [ ] **ACTOR-04**: Actor can send binary frames back to its client via Ws.send_binary(conn, data)
- [ ] **ACTOR-05**: Actor crash sends close frame (1011 Internal Error) before dropping connection
- [ ] **ACTOR-06**: Client disconnect causes actor to exit with exit signal propagation to linked actors
- [ ] **ACTOR-07**: Reader thread bridge delivers frames to actor mailbox without blocking M:N scheduler (short read timeout + yield cycle)

### Server Entry Points

- [ ] **SERVE-01**: Ws.serve(handler, port) starts a plaintext WebSocket server accepting TCP connections
- [ ] **SERVE-02**: Ws.serve_tls(handler, port, cert_path, key_path) starts a TLS WebSocket server using existing rustls infrastructure
- [ ] **SERVE-03**: Handler bundles on_connect, on_message, on_close callbacks into a single configuration

### Lifecycle Callbacks

- [ ] **LIFE-01**: on_connect callback invoked after handshake completes, receives connection handle and request headers
- [ ] **LIFE-02**: on_connect can reject connections by returning error
- [ ] **LIFE-03**: on_message callback invoked on each text or binary frame with message payload
- [ ] **LIFE-04**: on_close callback invoked when connection ends (clean or abnormal), receives close code and reason

### Heartbeat

- [ ] **BEAT-01**: Server sends periodic Ping frames at configurable interval (default 30s)
- [ ] **BEAT-02**: Server automatically responds to client Ping with Pong (echoing payload)
- [ ] **BEAT-03**: Server validates Pong payload matches sent Ping payload
- [ ] **BEAT-04**: Server closes connection after configurable missed Pong threshold (default 10s timeout)
- [ ] **BEAT-05**: Ping/pong timer operates inline with read timeout cycle (not blocked by frame read loop)

### Fragmentation

- [ ] **FRAG-01**: Server reassembles fragmented messages (continuation frames with opcode 0x0) into complete messages
- [ ] **FRAG-02**: Server handles interleaved control frames (ping/pong/close) during fragment reassembly without data corruption
- [ ] **FRAG-03**: Server enforces max message size limit (default 16MB) with close code 1009 on exceeded

### Rooms & Channels

- [ ] **ROOM-01**: Ws.join(conn, room) subscribes a connection to a named room
- [ ] **ROOM-02**: Ws.leave(conn, room) unsubscribes a connection from a room
- [ ] **ROOM-03**: Ws.broadcast(room, message) sends a text frame to all connections in a room
- [ ] **ROOM-04**: Ws.broadcast_except(room, message, conn) sends to all room members except specified connection
- [ ] **ROOM-05**: Connections are automatically removed from all rooms on disconnect (actor exit cleanup)
- [ ] **ROOM-06**: Room registry supports concurrent access from multiple connection actors

## Future Requirements

Deferred to a later milestone. Tracked but not in current roadmap.

### Protocol Extensions

- **EXT-01**: Per-message deflate compression (RFC 7692) for bandwidth-sensitive deployments
- **EXT-02**: WebSocket extensions framework for custom RSV bit usage

### Advanced Features

- **ADV-01**: Subprotocol negotiation (Sec-WebSocket-Protocol) for GraphQL subscriptions, MQTT-over-WS
- **ADV-02**: Configurable max concurrent connection limits with 503 rejection
- **ADV-03**: Per-connection send buffer limits with backpressure
- **ADV-04**: Connection rate limiting (token-bucket or fixed-window)
- **ADV-05**: WebSocket client library for Snow-to-external-service connections

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Per-message deflate compression | ~300KB/connection overhead, 4-parameter negotiation, minimal benefit for small messages |
| HTTP/WebSocket dual-protocol on same port | Clean separation (Ws.serve vs Http.serve on different ports) is simpler and more debuggable |
| WebSocket client library | Server-only focus; client use cases are niche for Snow's target |
| WebTransport / HTTP/3 | No production browser implementations; requires QUIC (UDP), fundamentally different model |
| Automatic JSON deserialization in WS handlers | Conflates protocol and serialization; users call Json.parse() explicitly |
| WebSocket extensions framework | Only widely-used extension is permessage-deflate (deferred); generic framework is over-engineering |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| PROTO-01 | Phase 59 | Pending |
| PROTO-02 | Phase 59 | Pending |
| PROTO-03 | Phase 59 | Pending |
| PROTO-04 | Phase 59 | Pending |
| PROTO-05 | Phase 59 | Pending |
| PROTO-06 | Phase 59 | Pending |
| PROTO-07 | Phase 59 | Pending |
| PROTO-08 | Phase 59 | Pending |
| PROTO-09 | Phase 59 | Pending |
| ACTOR-01 | Phase 60 | Pending |
| ACTOR-02 | Phase 60 | Pending |
| ACTOR-03 | Phase 60 | Pending |
| ACTOR-04 | Phase 60 | Pending |
| ACTOR-05 | Phase 60 | Pending |
| ACTOR-06 | Phase 60 | Pending |
| ACTOR-07 | Phase 60 | Pending |
| SERVE-01 | Phase 60 | Pending |
| SERVE-02 | Phase 61 | Pending |
| SERVE-03 | Phase 60 | Pending |
| LIFE-01 | Phase 60 | Pending |
| LIFE-02 | Phase 60 | Pending |
| LIFE-03 | Phase 60 | Pending |
| LIFE-04 | Phase 60 | Pending |
| BEAT-01 | Phase 61 | Pending |
| BEAT-02 | Phase 61 | Pending |
| BEAT-03 | Phase 61 | Pending |
| BEAT-04 | Phase 61 | Pending |
| BEAT-05 | Phase 61 | Pending |
| FRAG-01 | Phase 61 | Pending |
| FRAG-02 | Phase 61 | Pending |
| FRAG-03 | Phase 61 | Pending |
| ROOM-01 | Phase 62 | Pending |
| ROOM-02 | Phase 62 | Pending |
| ROOM-03 | Phase 62 | Pending |
| ROOM-04 | Phase 62 | Pending |
| ROOM-05 | Phase 62 | Pending |
| ROOM-06 | Phase 62 | Pending |

**Coverage:**
- v4.0 requirements: 37 total
- Mapped to phases: 37
- Unmapped: 0

---
*Requirements defined: 2026-02-12*
*Last updated: 2026-02-12 after roadmap creation*
