# Requirements: Snow

**Defined:** 2026-02-12
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v5.0 Requirements

Requirements for Distributed Actors milestone. Each maps to roadmap phases.

### Node Infrastructure

- [ ] **NODE-01**: User can start a named node with `Node.start("name@host", cookie: "secret")`
- [ ] **NODE-02**: User can connect to a remote node with `Node.connect("name@host:port")`
- [ ] **NODE-03**: Node connections are TLS-encrypted by default using existing rustls infrastructure
- [ ] **NODE-04**: Nodes authenticate via HMAC-SHA256 cookie-based challenge/response during handshake
- [ ] **NODE-05**: Built-in node registry resolves node names to host:port without external daemon
- [ ] **NODE-06**: Connected nodes automatically form a mesh (connecting to B discovers and connects to C)
- [ ] **NODE-07**: User can query connected nodes with `Node.list()` and own identity with `Node.self()`
- [ ] **NODE-08**: Heartbeat detects dead connections (configurable tick interval, default 60s)

### Messaging

- [ ] **MSG-01**: PIDs encode node identity in upper 16 bits of existing u64 (backward compatible with local PIDs)
- [ ] **MSG-02**: `send(pid, msg)` transparently routes to remote node when PID is non-local
- [ ] **MSG-03**: Binary wire format (Snow Term Format) serializes all Snow types for inter-node transport
- [ ] **MSG-04**: Wire format includes version byte for future evolution
- [ ] **MSG-05**: Closures/function pointers are detected and rejected during serialization with clear error
- [ ] **MSG-06**: User can send to a named process on a remote node with `send({name, node}, msg)`
- [ ] **MSG-07**: Message ordering is preserved per sender-receiver pair (guaranteed by TCP)
- [ ] **MSG-08**: Local send path has zero performance regression (branch-free locality check)

### Fault Tolerance

- [ ] **FT-01**: User can monitor a remote process with `Process.monitor(remote_pid)` receiving `:down` on crash
- [ ] **FT-02**: User can monitor a node with `Node.monitor(node)` receiving `:nodedown`/`:nodeup` events
- [ ] **FT-03**: Connection loss fires `:noconnection` exit signals for all remote links and `:down` for monitors
- [ ] **FT-04**: Remote links propagate exit signals across nodes (bidirectional, like local links)
- [ ] **FT-05**: Creation counter distinguishes PIDs from restarted nodes (stale PIDs treated as dead)

### Remote Execution

- [ ] **EXEC-01**: User can spawn an actor on a remote node with `Node.spawn(node, function, args)`
- [ ] **EXEC-02**: User can spawn and link with `Node.spawn_link(node, function, args)`
- [ ] **EXEC-03**: Remote spawn uses function name registry (not pointers) for cross-binary compatibility

### Cluster

- [ ] **CLUST-01**: User can register a name globally with `Global.register(name, pid)` visible across all nodes
- [ ] **CLUST-02**: User can look up global names with `Global.whereis(name)` returning PID from any node
- [ ] **CLUST-03**: Global registrations are cleaned up automatically when owning node disconnects
- [ ] **CLUST-04**: WebSocket rooms broadcast messages across connected nodes transparently
- [ ] **CLUST-05**: Supervision trees can monitor and restart children on remote nodes

## Future Requirements

Deferred to future release. Tracked but not in current roadmap.

### Optimization

- **OPT-01**: Atom cache for wire format (reduce repeated string serialization)
- **OPT-02**: Message fragmentation for large messages (interleave small messages)
- **OPT-03**: Connection backoff and retry on transient failures

### Extended Features

- **EXT-01**: Process groups (pg equivalent) for pub/sub across cluster
- **EXT-02**: Distributed ETS-style shared term storage
- **EXT-03**: Hot code reloading for rolling cluster upgrades

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Erlang ETF compatibility | Snow's type system is different; custom STF is simpler and safer |
| EPMD daemon | Built-in registry follows single-binary philosophy |
| QUIC transport | TCP + TLS is sufficient; QUIC adds async complexity |
| Automatic partition healing | Applications should decide recovery strategy |
| Closure serialization | Function pointers are address-space-local; detect and reject |
| mDNS/multicast discovery | Explicit connection is sufficient for v5.0 |
| 1000+ node clusters | Target 3-20 nodes; O(N^2) mesh is fine at this scale |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| NODE-01 | Phase 64 | Pending |
| NODE-02 | Phase 64 | Pending |
| NODE-03 | Phase 64 | Pending |
| NODE-04 | Phase 64 | Pending |
| NODE-05 | Phase 64 | Pending |
| NODE-06 | Phase 65 | Pending |
| NODE-07 | Phase 65 | Pending |
| NODE-08 | Phase 64 | Pending |
| MSG-01 | Phase 63 | Pending |
| MSG-02 | Phase 65 | Pending |
| MSG-03 | Phase 63 | Pending |
| MSG-04 | Phase 63 | Pending |
| MSG-05 | Phase 63 | Pending |
| MSG-06 | Phase 65 | Pending |
| MSG-07 | Phase 65 | Pending |
| MSG-08 | Phase 63 | Pending |
| FT-01 | Phase 66 | Pending |
| FT-02 | Phase 66 | Pending |
| FT-03 | Phase 66 | Pending |
| FT-04 | Phase 66 | Pending |
| FT-05 | Phase 63 | Pending |
| EXEC-01 | Phase 67 | Pending |
| EXEC-02 | Phase 67 | Pending |
| EXEC-03 | Phase 67 | Pending |
| CLUST-01 | Phase 68 | Pending |
| CLUST-02 | Phase 68 | Pending |
| CLUST-03 | Phase 68 | Pending |
| CLUST-04 | Phase 69 | Pending |
| CLUST-05 | Phase 69 | Pending |

**Coverage:**
- v5.0 requirements: 29 total
- Mapped to phases: 29
- Unmapped: 0

---
*Requirements defined: 2026-02-12*
*Last updated: 2026-02-12 after roadmap creation*
