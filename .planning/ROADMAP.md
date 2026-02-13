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
- [x] **v4.0 WebSocket Support** - Phases 59-62 (shipped 2026-02-12)
- [ ] **v5.0 Distributed Actors** - Phases 63-69 (in progress)

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

<details>
<summary>v4.0 WebSocket Support (Phases 59-62) - SHIPPED 2026-02-12</summary>

See milestones/v4.0-ROADMAP.md for full phase details.
8 plans across 4 phases. ~84,400 lines of Rust (+~950). 38 commits.

</details>

### v5.0 Distributed Actors (In Progress)

**Milestone Goal:** BEAM-style distributed actor system -- Snow programs on different machines form a cluster with location-transparent PIDs, remote spawn, cross-node monitoring, and a binary wire format over TLS.

- [x] **Phase 63: PID Encoding & Wire Format** - Location-transparent PID representation and binary serialization for all Snow types (completed 2026-02-13)
- [x] **Phase 64: Node Connection & Authentication** - TLS-encrypted inter-node TCP with cookie-based auth and discovery (completed 2026-02-13)
- [ ] **Phase 65: Remote Send & Distribution Router** - Transparent message routing across nodes with mesh formation
- [ ] **Phase 66: Remote Links, Monitors & Failure Handling** - Cross-node fault tolerance with exit signal and partition propagation
- [ ] **Phase 67: Remote Spawn & LLVM Integration** - Spawn actors on remote nodes with full Snow-level API
- [ ] **Phase 68: Global Registry** - Cluster-wide process name registration and lookup
- [ ] **Phase 69: Cross-Node Integration** - Distributed WebSocket rooms and remote supervision trees

## Phase Details

### Phase 63: PID Encoding & Wire Format
**Goal**: PIDs carry node identity and all Snow values can be serialized to a binary format for inter-node transport
**Depends on**: Nothing (foundation phase)
**Requirements**: MSG-01, MSG-03, MSG-04, MSG-05, MSG-08, FT-05
**Success Criteria** (what must be TRUE):
  1. Existing Snow programs produce identical output with no PID encoding regressions (all 1,524 tests pass)
  2. A PID created on one node can be decoded on another node to identify the originating node and local process
  3. Every Snow value type (Int, Float, Bool, String, List, Map, Set, tuples, structs, sum types, Option, Result, PID) round-trips through STF encode/decode without data loss
  4. Attempting to serialize a closure or function pointer produces a clear runtime error instead of silent corruption
  5. Local send performance is unchanged -- the locality check adds no measurable overhead to the existing fast path
**Plans:** 3 plans
Plans:
- [x] 63-01-PLAN.md -- PID bit-packing and locality check in send
- [x] 63-02-PLAN.md -- STF module scaffold and scalar type encode/decode
- [x] 63-03-PLAN.md -- STF container/composite types and round-trip tests

### Phase 64: Node Connection & Authentication
**Goal**: Snow nodes can discover each other, establish TLS-encrypted connections, and authenticate via shared cookie
**Depends on**: Phase 63
**Requirements**: NODE-01, NODE-02, NODE-03, NODE-04, NODE-05, NODE-08
**Success Criteria** (what must be TRUE):
  1. User can start a named node with `Node.start("name@host", cookie: "secret")` and the process becomes addressable
  2. User can connect to a remote node with `Node.connect("name@host:port")` and the connection succeeds with mutual authentication
  3. A connection attempt with a wrong cookie is rejected with a clear error (not silent failure or crash)
  4. Inter-node traffic is TLS-encrypted using the existing rustls infrastructure (not plaintext)
  5. A dead node connection is detected via heartbeat within the configured timeout interval
**Plans:** 3 plans
Plans:
- [x] 64-01-PLAN.md -- NodeState, TLS config, ephemeral cert, TCP listener, Node.start
- [x] 64-02-PLAN.md -- HMAC-SHA256 handshake protocol, Node.connect, NodeSession
- [x] 64-03-PLAN.md -- Heartbeat, reader thread, connection lifecycle, tests

### Phase 65: Remote Send & Distribution Router
**Goal**: `send(pid, msg)` works transparently for remote PIDs and connected nodes form a mesh
**Depends on**: Phase 64
**Requirements**: MSG-02, MSG-06, MSG-07, NODE-06, NODE-07
**Success Criteria** (what must be TRUE):
  1. User can send a message to a PID on a remote node using the same `send(pid, msg)` syntax as local sends, and the remote actor receives it
  2. User can send a message to a named process on a remote node with `send({name, node}, msg)` and it arrives
  3. Messages between a given sender-receiver pair arrive in the order they were sent
  4. Connecting node A to node B causes automatic mesh formation with node C (if B is already connected to C)
  5. User can call `Node.list()` to see all connected nodes and `Node.self()` to get own node identity
**Plans**: TBD

### Phase 66: Remote Links, Monitors & Failure Handling
**Goal**: Distributed fault tolerance -- supervisors and monitors detect remote crashes and network partitions
**Depends on**: Phase 65
**Requirements**: FT-01, FT-02, FT-03, FT-04
**Success Criteria** (what must be TRUE):
  1. User can monitor a remote process with `Process.monitor(remote_pid)` and receives a `:down` message when that process crashes
  2. User can monitor a node with `Node.monitor(node)` and receives `:nodedown` when the node disconnects and `:nodeup` when it reconnects
  3. When a node connection is lost, all remote links fire `:noconnection` exit signals and all remote monitors fire `:down` messages
  4. Remote links propagate exit signals bidirectionally -- a crash on node A terminates linked processes on node B and vice versa
**Plans**: TBD

### Phase 67: Remote Spawn & LLVM Integration
**Goal**: Users can spawn actors on remote nodes from Snow code with full language-level API
**Depends on**: Phase 66
**Requirements**: EXEC-01, EXEC-02, EXEC-03
**Success Criteria** (what must be TRUE):
  1. User can spawn an actor on a remote node with `Node.spawn(node, function, args)` and receive a usable PID back
  2. User can spawn-and-link with `Node.spawn_link(node, function, args)` so that the remote actor's crash propagates back
  3. Remote spawn uses function names (not pointers) so that differently-compiled binaries can spawn each other's functions
**Plans**: TBD

### Phase 68: Global Registry
**Goal**: Processes can be registered by name across the entire cluster and looked up from any node
**Depends on**: Phase 65
**Requirements**: CLUST-01, CLUST-02, CLUST-03
**Success Criteria** (what must be TRUE):
  1. User can register a process globally with `Global.register(name, pid)` and the name is visible from all connected nodes
  2. User can look up a globally registered name with `Global.whereis(name)` from any node and get back the correct PID
  3. When a node disconnects, all global registrations owned by processes on that node are automatically cleaned up
**Plans**: TBD

### Phase 69: Cross-Node Integration
**Goal**: Existing WebSocket rooms and supervision trees work transparently across node boundaries
**Depends on**: Phase 66, Phase 68
**Requirements**: CLUST-04, CLUST-05
**Success Criteria** (what must be TRUE):
  1. A WebSocket room broadcast on one node delivers the message to room members connected to other nodes
  2. A supervision tree can monitor and restart child actors running on remote nodes, treating remote crashes the same as local ones
**Plans**: TBD

## Progress

**Execution Order:** 63 -> 64 -> 65 -> 66 -> 67 -> 68 -> 69

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
| 59-62 | v4.0 | 8/8 | Complete | 2026-02-12 |
| 63 | v5.0 | 3/3 | Complete | 2026-02-13 |
| 64 | v5.0 | 3/3 | Complete | 2026-02-13 |
| 65 | v5.0 | 0/TBD | Not started | - |
| 66 | v5.0 | 0/TBD | Not started | - |
| 67 | v5.0 | 0/TBD | Not started | - |
| 68 | v5.0 | 0/TBD | Not started | - |
| 69 | v5.0 | 0/TBD | Not started | - |

**Total: 64 phases shipped across 14 milestones. 176 plans completed. v5.0 in progress (2/7 phases complete).**
