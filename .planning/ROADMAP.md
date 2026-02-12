# Roadmap: Snow

## Milestones

- [x] **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- [x] **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- [x] **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- [x] **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- [x] **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- [x] **v1.5 Compiler Correctness** - Phases 26-29 (shipped 2026-02-09)
- [x] **v1.6 Method Dot-Syntax** - Phases 30-32 (shipped 2026-02-09)
- [x] **v1.7 Loops & Iteration** - Phases 33-36 (shipped 2026-02-09)
- [x] **v1.8 Module System** - Phases 37-42 (shipped 2026-02-09)
- [x] **v1.9 Stdlib & Ergonomics** - Phases 43-48 (shipped 2026-02-10)
- [x] **v2.0 Database & Serialization** - Phases 49-54 (shipped 2026-02-12)
- [x] **v3.0 Production Backend** - Phases 55-58 (shipped 2026-02-12)
- [ ] **v4.0 WebSocket Support** - Phases 59-62 (in progress)

## Phases

<details>
<summary>v1.0 MVP (Phases 1-10) - SHIPPED 2026-02-07</summary>

See milestones/v1.0-ROADMAP.md for full phase details.
55 plans across 10 phases. 52,611 lines of Rust. 213 commits.

</details>

<details>
<summary>v1.1 Language Polish (Phases 11-15) - SHIPPED 2026-02-08</summary>

See milestones/v1.1-ROADMAP.md for full phase details.
10 plans across 5 phases. 56,539 lines of Rust (+3,928). 45 commits.

</details>

<details>
<summary>v1.2 Runtime & Type Fixes (Phases 16-17) - SHIPPED 2026-02-08</summary>

See milestones/v1.2-ROADMAP.md for full phase details.
6 plans across 2 phases. 57,657 lines of Rust (+1,118). 22 commits.

</details>

<details>
<summary>v1.3 Traits & Protocols (Phases 18-22) - SHIPPED 2026-02-08</summary>

See milestones/v1.3-ROADMAP.md for full phase details.
18 plans across 5 phases. 63,189 lines of Rust (+5,532). 65 commits.

</details>

<details>
<summary>v1.4 Compiler Polish (Phases 23-25) - SHIPPED 2026-02-08</summary>

See milestones/v1.4-ROADMAP.md for full phase details.
5 plans across 3 phases. 64,548 lines of Rust (+1,359). 13 commits.

</details>

<details>
<summary>v1.5 Compiler Correctness (Phases 26-29) - SHIPPED 2026-02-09</summary>

See milestones/v1.5-ROADMAP.md for full phase details.
6 plans across 4 phases. 66,521 lines of Rust (+1,973). 29 commits.

</details>

<details>
<summary>v1.6 Method Dot-Syntax (Phases 30-32) - SHIPPED 2026-02-09</summary>

See milestones/v1.6-ROADMAP.md for full phase details.
6 plans across 3 phases. 67,546 lines of Rust (+1,025). 24 commits.

</details>

<details>
<summary>v1.7 Loops & Iteration (Phases 33-36) - SHIPPED 2026-02-09</summary>

See milestones/v1.7-ROADMAP.md for full phase details.
8 plans across 4 phases. 70,501 lines of Rust (+2,955). 34 commits.

</details>

<details>
<summary>v1.8 Module System (Phases 37-42) - SHIPPED 2026-02-09</summary>

See milestones/v1.8-ROADMAP.md for full phase details.
12 plans across 6 phases. 73,384 lines of Rust (+2,883). 52 commits.

</details>

<details>
<summary>v1.9 Stdlib & Ergonomics (Phases 43-48) - SHIPPED 2026-02-10</summary>

See milestones/v1.9-ROADMAP.md for full phase details.
13 plans across 6 phases. 76,100 lines of Rust (+2,716). 56 commits.

</details>

<details>
<summary>v2.0 Database & Serialization (Phases 49-54) - SHIPPED 2026-02-12</summary>

See milestones/v2.0-ROADMAP.md for full phase details.
13 plans across 6 phases. 81,006 lines of Rust (+4,906). 52 commits.

</details>

<details>
<summary>v3.0 Production Backend (Phases 55-58) - SHIPPED 2026-02-12</summary>

See milestones/v3.0-ROADMAP.md for full phase details.
8 plans across 4 phases. 83,451 lines of Rust (+2,445). 33 commits.

</details>

### v4.0 WebSocket Support (In Progress)

**Milestone Goal:** Add WebSocket support with actor-per-connection model, unified actor messaging, rooms/channels, ping/pong heartbeat, binary+text frames, and TLS (wss://).

- [x] **Phase 59: Protocol Core** - RFC 6455 frame codec, HTTP upgrade handshake, text/binary/close frames (completed 2026-02-12)
- [x] **Phase 60: Actor Integration** - Actor-per-connection, reader thread bridge, mailbox delivery, callbacks, Ws.serve (completed 2026-02-12)
- [ ] **Phase 61: Production Hardening** - TLS (wss://), ping/pong heartbeat, message fragmentation
- [ ] **Phase 62: Rooms & Channels** - Named rooms with join/leave/broadcast and automatic cleanup

## Phase Details

### Phase 59: Protocol Core
**Goal**: WebSocket wire protocol speaks RFC 6455 -- frames can be parsed, written, masked/unmasked, and connections can be upgraded from HTTP
**Depends on**: Nothing (first phase of v4.0; builds on existing HTTP parser in snow-rt)
**Requirements**: PROTO-01, PROTO-02, PROTO-03, PROTO-04, PROTO-05, PROTO-06, PROTO-07, PROTO-08, PROTO-09
**Success Criteria** (what must be TRUE):
  1. A WebSocket client can complete an HTTP upgrade handshake and receive a valid 101 Switching Protocols response with correct Sec-WebSocket-Accept header
  2. The server can read client frames (text and binary) of all three payload length encodings (7-bit, 16-bit, 64-bit), correctly unmasking the payload
  3. The server can write unmasked text and binary frames back to the client
  4. A close handshake completes cleanly in both directions (server-initiated and client-initiated) with proper status codes
  5. Malformed upgrade requests receive HTTP 400, and unknown opcodes trigger close with code 1002
**Plans:** 2 plans
Plans:
- [x] 59-01-PLAN.md -- Frame codec (WsOpcode, WsFrame, read/write/mask)
- [x] 59-02-PLAN.md -- Handshake, close handshake, text UTF-8 validation

### Phase 60: Actor Integration
**Goal**: Each WebSocket connection runs as an isolated actor with WS frames arriving in the standard mailbox, callback-based user API, and a dedicated server entry point
**Depends on**: Phase 59 (frame codec and upgrade handshake)
**Requirements**: ACTOR-01, ACTOR-02, ACTOR-03, ACTOR-04, ACTOR-05, ACTOR-06, ACTOR-07, SERVE-01, SERVE-03, LIFE-01, LIFE-02, LIFE-03, LIFE-04
**Success Criteria** (what must be TRUE):
  1. A Snow program can call Ws.serve(handler, port) to start a WebSocket server that accepts connections, each handled by a dedicated actor with crash isolation
  2. WebSocket text and binary frames are delivered to the actor mailbox and can be received via the standard `receive` expression alongside actor-to-actor messages without type tag collision
  3. The actor can send text and binary frames back to the client via Ws.send and Ws.send_binary, and the reader thread does not block the M:N scheduler
  4. User-defined on_connect, on_message, and on_close callbacks fire at the correct lifecycle points, with on_connect able to reject connections
  5. Actor crash sends close frame 1011 to the client, and client disconnect causes actor exit with signal propagation to linked actors
**Plans:** 2 plans
Plans:
- [x] 60-01-PLAN.md -- Runtime server infrastructure (accept loop, reader thread bridge, lifecycle callbacks, Ws.send)
- [x] 60-02-PLAN.md -- Codegen wiring (intrinsics, STDLIB_MODULES, map_builtin_name)

### Phase 61: Production Hardening
**Goal**: WebSocket connections are production-ready with TLS encryption, dead connection detection via heartbeat, and large message support via fragmentation
**Depends on**: Phase 60 (working actor-per-connection WebSocket server)
**Requirements**: SERVE-02, BEAT-01, BEAT-02, BEAT-03, BEAT-04, BEAT-05, FRAG-01, FRAG-02, FRAG-03
**Success Criteria** (what must be TRUE):
  1. A Snow program can call Ws.serve_tls(handler, port, cert_path, key_path) to start a wss:// server using the existing rustls infrastructure
  2. The server sends periodic Ping frames and automatically responds to client Pings with Pong, closing connections that miss the Pong timeout threshold
  3. Fragmented messages (continuation frames) are reassembled into complete messages, with interleaved control frames handled correctly and messages exceeding 16MB rejected with close code 1009
**Plans**: TBD

### Phase 62: Rooms & Channels
**Goal**: Connections can join named rooms for pub/sub broadcast messaging with automatic cleanup on disconnect
**Depends on**: Phase 60 (working actor-per-connection model for room membership)
**Requirements**: ROOM-01, ROOM-02, ROOM-03, ROOM-04, ROOM-05, ROOM-06
**Success Criteria** (what must be TRUE):
  1. A connection actor can call Ws.join(conn, room) and Ws.leave(conn, room) to subscribe and unsubscribe from named rooms
  2. Ws.broadcast(room, message) delivers a text frame to all connections in the room, and Ws.broadcast_except(room, message, conn) delivers to all except the specified connection
  3. When a connection disconnects, it is automatically removed from all rooms it had joined
  4. Multiple connection actors can concurrently join, leave, and broadcast to the same room without data corruption
**Plans**: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 59 -> 60 -> 61 -> 62

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1-10 | v1.0 | 55/55 | Complete | 2026-02-07 |
| 11-15 | v1.1 | 10/10 | Complete | 2026-02-08 |
| 16-17 | v1.2 | 6/6 | Complete | 2026-02-08 |
| 18-22 | v1.3 | 18/18 | Complete | 2026-02-08 |
| 23-25 | v1.4 | 5/5 | Complete | 2026-02-08 |
| 26-29 | v1.5 | 6/6 | Complete | 2026-02-09 |
| 30-32 | v1.6 | 6/6 | Complete | 2026-02-09 |
| 33-36 | v1.7 | 8/8 | Complete | 2026-02-09 |
| 37-42 | v1.8 | 12/12 | Complete | 2026-02-09 |
| 43-48 | v1.9 | 13/13 | Complete | 2026-02-10 |
| 49-54 | v2.0 | 13/13 | Complete | 2026-02-12 |
| 55-58 | v3.0 | 8/8 | Complete | 2026-02-12 |
| 59. Protocol Core | v4.0 | 2/2 | Complete | 2026-02-12 |
| 60. Actor Integration | v4.0 | 2/2 | Complete | 2026-02-12 |
| 61. Production Hardening | v4.0 | 0/TBD | Not started | - |
| 62. Rooms & Channels | v4.0 | 0/TBD | Not started | - |

**Total: 60 phases shipped across 12 milestones. 166 plans completed. 2 phases remaining for v4.0.**
