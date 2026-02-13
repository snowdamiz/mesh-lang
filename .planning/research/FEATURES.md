# Feature Landscape

**Domain:** Distributed actor system for compiled programming language with BEAM-style concurrency (Snow v5.0)
**Researched:** 2026-02-12
**Confidence:** HIGH (BEAM distribution protocol is extensively documented by Ericsson; Akka/Orleans provide additional reference points; Snow's existing actor runtime thoroughly reviewed)

---

## Current State in Snow

Before defining features, here is what already exists and what this milestone builds on:

**Working infrastructure these features extend:**
- Actor runtime: M:N work-stealing scheduler with corosensei coroutines (64 KiB stacks), typed `Pid<M>`, `snow_actor_send/receive`, FIFO mailbox with deep-copy message passing, crash isolation via `catch_unwind`
- Process identity: `ProcessId` is a sequential `u64` from a global `AtomicU64` counter -- currently node-local only
- Process linking: bidirectional links with exit signal propagation, `trap_exit` for supervisors, `EXIT_SIGNAL_TAG` encoded as `[u64 pid, u8 reason_tag, ...]`
- Process registry: `ProcessRegistry` with `RwLock<FxHashMap>` for name-to-PID lookup, reverse index for cleanup on exit
- Supervision trees: `Supervisor.start` with OneForOne/OneForAll/RestForOne/SimpleOneForOne strategies, automatic restart with intensity limits
- Timer support: `Timer.sleep(ms)`, `Timer.send_after(pid, ms, msg)` for delayed messaging
- Receive with timeout: `snow_actor_receive(timeout_ms)` supports blocking, non-blocking, and timed receive
- TLS infrastructure: rustls 0.23 with `ServerConfig`, PEM certificate loading, `StreamOwned` for lazy handshake
- WebSocket rooms: `RoomRegistry` with dual-map (rooms + conn_rooms), join/leave/broadcast -- currently connection-handle based (usize), not PID-based
- JSON serde: `deriving(Json)` for encode/decode
- Message format: `MessageBuffer { data: Vec<u8>, type_tag: u64 }` with deep-copy on send
- HTTP/WS servers: actor-per-connection model with reader thread bridge for non-blocking I/O

**What this milestone adds:**
- Node naming, identity, and cookie-based authentication
- Node connection over TLS-encrypted TCP with automatic mesh formation
- Built-in node registry (epmd equivalent) for discovery
- Location-transparent PIDs that route `send` across nodes automatically
- Remote spawn of actors on other nodes
- Remote process monitoring delivering `:down` on crash or net split
- Node monitoring delivering `:nodedown`/`:nodeup` events
- Binary wire format (ETF-style) for efficient inter-node serialization
- Distributed global process registry across cluster
- Cross-node integration with existing WebSocket rooms and supervision trees

---

## Table Stakes

Features users expect from any distributed actor system claiming BEAM-style distribution. Missing = fundamentally broken or unusable for distributed programming.

| Feature | Why Expected | Complexity | Dependencies |
|---------|-------------|------------|--------------|
| Node naming and identity | Every distributed system needs addressable endpoints. BEAM uses `name@host` format. Without named nodes, no connection is possible. | Low | None -- new concept |
| Cookie-based authentication | Prevents unauthorized nodes from joining the cluster. BEAM uses challenge-response with shared cookie (never sent in cleartext). Minimum viable security. | Medium | Node naming |
| TLS-encrypted inter-node transport | Data in transit must be encrypted. BEAM recommends TLS for production. Snow already has rustls 0.23 infrastructure from HTTP/WS/PG. | Medium | Existing rustls infrastructure |
| Node connection establishment | TCP connection + handshake between named nodes. BEAM uses a multi-phase handshake: name exchange, status check, challenge-response authentication. | High | Node naming, cookie auth |
| Automatic mesh formation | When node A connects to B which knows C, A should also connect to C. BEAM does this by default (transitive connections). Without this, manual topology management becomes burdensome. | Medium | Node connection |
| Location-transparent PIDs | The core value proposition. `send(pid, msg)` must work identically whether `pid` is local or remote. The runtime routes automatically based on PID contents. BEAM encodes the originating node into every PID. | High | Node connection, wire format |
| Remote message delivery | Messages sent to remote PIDs must be serialized, transmitted over the connection, deserialized, and delivered to the remote process mailbox. Must maintain per-sender FIFO ordering. | High | Location-transparent PIDs, wire format |
| Binary wire format | Efficient serialization of Snow values for inter-node transfer. BEAM uses External Term Format (ETF). Needed for all cross-node communication. | High | None -- new concept |
| Node registry / discovery | An equivalent to EPMD: a service that maps node names to `host:port`. Without this, nodes cannot find each other. | Medium | Node naming |
| Remote process monitoring | `Process.monitor(pid)` must work on remote PIDs. When the monitored process dies OR the connection drops, the monitoring process receives `{:DOWN, ref, pid, reason}`. Reason is `:noconnection` on net split. | High | Location-transparent PIDs, node connection |
| Node monitoring | `Node.monitor(node)` delivers `:nodedown` when a connected node becomes unreachable and `:nodeup` when it reconnects. Essential for building robust distributed applications. | Medium | Node connection |
| Connection loss handling | When a node goes down or network partitions occur, all remote links must fire exit signals (reason `:noconnection`), all remote monitors must fire `:DOWN` with `:noconnection`, and the node must be removed from the connected set. | High | Remote monitoring, links |
| Remote spawn | `spawn(node, function, args)` creates an actor on a remote node and returns a local-usable PID. BEAM supports `spawn(Node, Fun)`, `spawn(Node, M, F, A)`, and `spawn_link` variants. | High | Location-transparent PIDs, wire format, node connection |

---

## Differentiators

Features that set Snow's distribution apart from a minimal implementation. Not expected in an MVP, but valuable for real-world use.

| Feature | Value Proposition | Complexity | Dependencies |
|---------|-------------------|------------|--------------|
| Distributed global process registry | Register a name that resolves to a PID across all nodes in the cluster. BEAM has `global` module (slow, consistent) and `pg` module (fast, eventually consistent groups). Enables service discovery without passing PIDs manually. | High | Node connection, local registry |
| Cross-node WebSocket room broadcast | WebSocket rooms currently track local connection handles. Extending rooms to broadcast across nodes enables multi-node chat, presence systems, and real-time applications. This is where Snow's unique combination of WS + distribution shines. | High | Node connection, wire format, existing rooms |
| Cross-node supervision integration | Supervisors that monitor and restart children on remote nodes. Enables building resilient distributed topologies where failure on one node triggers restart on another. | High | Remote monitoring, remote spawn |
| Atom/fragment caching in wire format | BEAM's ETF uses an atom cache in distribution headers to avoid re-sending frequently used atoms. Significantly reduces bandwidth for chatty protocols. | Medium | Wire format |
| Connection backoff and retry | Automatic reconnection attempts with exponential backoff when a node becomes unreachable. Reduces manual connection management in real applications. | Low | Node connection |
| `node(pid)` introspection | Return which node a PID belongs to. BEAM supports this natively because node identity is encoded in the PID. Useful for debugging and routing decisions. | Low | Location-transparent PIDs |
| `nodes()` listing | Return the list of currently connected nodes. Essential for cluster management, health checks, and adaptive routing. | Low | Node connection |
| `is_alive()` check | Return whether the current runtime is operating as a distributed node. Allows code to branch on distributed vs. local execution. | Low | Node naming |
| Remote `send` by registered name | `send({name, node}, msg)` sends to a named process on a specific node without knowing its PID. BEAM supports `{RegName, Node} ! Msg`. Ergonomic for cross-node service calls. | Medium | Node connection, local registry |
| Heartbeat / tick for connection liveness | Periodic keepalive messages on idle connections to detect silent failures (network cable unplug, VM freeze). BEAM uses `net_ticktime` (default 60s, configurable). Without this, dead connections are only detected on next send attempt. | Medium | Node connection |

---

## Anti-Features

Features to explicitly NOT build in this milestone. Including rationale and what to do instead.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| Consensus protocol (Raft/Paxos) | Consensus is a separate problem domain from actor distribution. BEAM itself does not include consensus -- it is built on top (e.g., Ra library for RabbitMQ). Adding consensus dramatically increases scope and complexity. | Provide the building blocks (node connections, messaging, monitoring) that a consensus library could be built on top of. |
| Distributed transactions | Two-phase commit across nodes is fragile, slow, and fundamentally at odds with the actor model's "let it crash" philosophy. BEAM does not provide distributed transactions (Mnesia's transactions are a separate application, not part of distribution). | Use the existing actor model: coordinate via message passing and idempotent operations. |
| Hot code reloading | While closely associated with BEAM, hot code reloading is orthogonal to distribution. It requires a completely different set of compiler and runtime changes (module versioning, purging, upgrading running processes). | Defer to a dedicated milestone. Distribution works fine with restart-based deployment. |
| Automatic partition healing | Automatically merging divergent cluster state after a network partition is an unsolved general problem (CAP theorem). BEAM's own `dist_auto_connect once` approach acknowledges that automatic reconnection causes instability. | Provide node monitoring events (`:nodedown`/`:nodeup`) so applications can implement domain-specific healing. Let the user decide how to resolve split-brain. |
| Shared-nothing verification | Statically proving no shared mutable state between actors across nodes. While theoretically valuable, Snow already enforces this at the local level via deep-copy message passing. Distribution inherently serializes. | The existing deep-copy + serialization approach already guarantees isolation. No additional verification needed. |
| Global ordering across all senders | Guaranteeing that messages from A and B to C arrive in global timestamp order. No actor system provides this (including BEAM). It requires distributed clocks or vector clocks and destroys performance. | Document the per-sender ordering guarantee clearly. Users who need global ordering can use sequence numbers in messages. |
| Adaptive load balancing / placement | Orleans-style grain placement strategies that automatically move actors to balance load. This is a higher-level concern that belongs in a framework layer, not the core distribution protocol. | Provide remote spawn so users can choose placement explicitly. Framework-level placement can be built on top. |
| Multi-language node interop | Erlang's distribution protocol is designed for Erlang-to-Erlang communication. Supporting heterogeneous nodes (e.g., Snow talking to Erlang) requires full ETF compatibility and BEAM protocol compliance. Too much scope. | Design the wire format to be extensible but focused on Snow-to-Snow communication. Document the format so third-party implementations are possible in the future. |
| Process migration | Moving a running process from one node to another. BEAM does not support this. It requires serializing the entire process state including stack, which is extremely complex for compiled native code. | Use the pattern of spawning a new process on the target node and transferring state via messages, then terminating the old process. |
| Distributed tracing / observability | OpenTelemetry-style distributed tracing across nodes. Valuable but orthogonal to the core distribution protocol. | Provide node/PID metadata in messages. Tracing can be layered on top as a separate library. |

---

## Feature Details: Expected Behaviors

### Node Naming and Identity

**BEAM behavior (the model):**
- Two naming modes: short names (`-sname name`, format `name@hostname`) and long names (`-name name`, format `name@fqdn`). Short and long name nodes cannot communicate.
- The node name is an atom, globally visible via `node()`.
- An unnamed node cannot participate in distribution.

**Snow behavior (recommendation):**
- Single naming mode: `name@host:port` format. Unlike BEAM, Snow compiles to native binaries without a VM, so there is no separate EPMD port -- the node name directly encodes the connection address.
- Node name is a string, returned by `Node.self()`.
- A Snow program not started with a node name runs in local-only mode (existing behavior).

### Cookie Authentication

**BEAM behavior:**
- Shared secret stored in `~/.erlang.cookie` (file permissions 0400).
- Challenge-response handshake: nodes exchange 32-bit random challenges and prove knowledge of the cookie by computing `MD5(cookie ++ challenge_as_text)`. The cookie is never transmitted.
- Per-node cookie overrides via `erlang:set_cookie(Node, Cookie)`.

**Snow behavior (recommendation):**
- Cookie passed via command-line flag or environment variable (no file auto-creation -- explicit over implicit).
- Challenge-response using HMAC-SHA256 instead of MD5 (SHA-256 already available via the `sha2` crate used for PostgreSQL auth). Stronger security at negligible performance cost.
- Single cookie per cluster (no per-node overrides in MVP).

### Node Connection and Mesh Formation

**BEAM behavior:**
- First reference to a remote node triggers automatic connection via EPMD lookup.
- Connections are TCP, bidirectional, and persistent.
- Default: transitive connections (full mesh). Can disable with `-connect_all false`.
- Handshake: name exchange -> status -> challenge -> challenge-reply -> challenge-ack.
- 16-bit packet headers during handshake, 4-byte headers after.

**Snow behavior (recommendation):**
- Explicit connection via `Node.connect("name@host:port")` or automatic on first `send` to a remote PID.
- TLS-encrypted TCP by default (BEAM only recommends TLS; Snow should require it since there is no legacy compatibility concern).
- Transitive connections enabled by default. When A connects to B, B tells A about all other nodes B knows, and A connects to them.
- Handshake: name exchange -> cookie challenge-response -> capability flags -> connected.

### Location-Transparent PIDs

**BEAM behavior:**
- PIDs contain node identity: local PIDs are `<0.X.Y>`, remote PIDs are `<N.X.Y>` where N > 0 identifies the node.
- `node(Pid)` returns the node a PID belongs to.
- `send(Pid, Msg)` routes transparently: if `node(Pid) == node()`, deliver locally; otherwise, serialize and send over the distribution connection.
- PIDs serialize/deserialize correctly over the wire. A PID created on node A can be sent to node B, which can then send it to node C, and C can message the process on A.

**Snow behavior (recommendation):**
- Extend `ProcessId` from `u64` to include node identity. Format: encode `(node_id: u32, local_id: u32)` into the existing `u64` representation. Local PIDs have `node_id == 0`. Allows backward compatibility since existing local PIDs use sequential small numbers.
- `snow_actor_send` checks `node_id`: if 0, deliver locally (current path); if non-zero, look up the distribution connection for that node and serialize/transmit.
- `Node.of(pid)` returns the node name for a PID.
- PIDs serialize as part of the wire format, preserving node identity across the network.

### Message Ordering Guarantees

**BEAM behavior:**
- Per-sender-to-per-receiver ordering is guaranteed: if A sends S1 then S2 to B, S1 arrives before S2. This holds across nodes.
- No global ordering: if A sends to B and C sends to B, the relative order of A's and C's messages at B is undefined.
- This guarantee extends to all signals (messages, link signals, monitor signals, exit signals) from the same sender to the same receiver.

**Snow behavior (recommendation):**
- Maintain the same per-sender ordering guarantee. This is naturally achieved by using a single TCP connection per node pair (messages are ordered within a TCP stream).
- Document explicitly: no global ordering across different senders. Users who need total ordering should use sequence numbers or a coordination service.

### Remote Process Monitoring

**BEAM behavior:**
- `erlang:monitor(process, Pid)` works on remote PIDs. Returns a unique reference.
- When the monitored process exits, the monitor owner receives `{'DOWN', Ref, process, Pid, Reason}`.
- When the distribution connection drops, the monitor owner receives `{'DOWN', Ref, process, Pid, noconnection}`. This is indistinguishable from the process dying -- the monitoring process cannot know if the remote process is actually dead or just unreachable.
- Monitors are unidirectional (unlike links). Multiple independent monitors can observe the same process.
- `demonitor(Ref)` cancels a monitor.

**Snow behavior (recommendation):**
- `Process.monitor(pid)` returns a monitor reference (u64). Works for both local and remote PIDs.
- On remote process death: deliver `{:down, ref, pid, reason}` to monitor owner's mailbox.
- On connection loss: deliver `{:down, ref, pid, :noconnection}` to monitor owner's mailbox.
- On `demonitor(ref)`: cancel the monitor. If the `:DOWN` message is already in the mailbox, it remains (matching BEAM -- use `demonitor(ref, [:flush])` to also flush).

### Node Monitoring

**BEAM behavior:**
- `monitor_node(Node, true)` subscribes to node status changes.
- `{nodedown, Node}` message when connection to Node is lost.
- Repeated calls create multiple monitors (each gets its own `{nodedown, ...}` message).
- `monitor_node(Node, false)` removes one monitor (LIFO).

**Snow behavior (recommendation):**
- `Node.monitor(node_name)` subscribes to node status changes.
- Delivers `{:nodedown, node_name}` on connection loss.
- Delivers `{:nodeup, node_name}` on reconnection (BEAM does not do this by default -- Snow can add it as a convenience since there is no legacy behavior to maintain).
- `Node.demonitor(node_name)` unsubscribes.

### Remote Spawn

**BEAM behavior:**
- `spawn(Node, Fun)` creates a process on Node running Fun. Returns a PID usable locally.
- `spawn_link(Node, Fun)` creates + links atomically.
- `spawn(Node, Module, Function, Args)` is the MFA form.
- `spawn_opt(Node, Fun, Opts)` with options like `link`, `monitor`.
- The spawned process runs on the remote node's scheduler. The returned PID is a remote PID that routes messages correctly.

**Snow behavior (recommendation):**
- `Node.spawn(node_name, function, args)` sends a spawn request over the distribution connection. The remote node creates the actor and replies with the PID.
- `Node.spawn_link(node_name, function, args)` atomically spawns + links.
- The function must be a named function (not a closure -- closures capture local heap references that cannot be serialized). This matches practical BEAM usage where remote spawn typically uses MFA.
- Returns a `Pid<M>` that is location-transparent and works with existing `send`.

### Binary Wire Format

**BEAM behavior:**
- External Term Format (ETF): version byte 131, followed by type-tagged terms.
- Key types: atoms (tag 118/119), integers (tag 97/98), floats (tag 70), tuples (tag 104/105), lists (tag 108), binaries (tag 109), maps (tag 116), PIDs (tag 88), references (tag 90), ports (tag 89).
- Distribution adds a header with atom cache for frequently-used atoms and fragmentation support for large messages.
- Control messages (LINK, SEND, EXIT, MONITOR_P, SPAWN_REQUEST, etc.) are tuples with numeric operation codes.

**Snow behavior (recommendation):**
- Custom binary format inspired by ETF but tailored to Snow's type system.
- Version byte for forward compatibility.
- Type tags for: Int (i64), Float (f64), Bool, String (length-prefixed UTF-8), Atom/Symbol (length-prefixed), Tuple (arity + elements), List (length + elements), Map (length + key-value pairs), Binary (length + bytes), Pid (node_id + local_id), Ref (node_id + id), Struct (name + fields), SumType (tag + variant + fields), Unit.
- No atom cache in MVP (optimization for later).
- Message framing: 4-byte big-endian length prefix per message (matching BEAM post-handshake framing).
- Control messages as tagged tuples: `{SEND, dest_pid, msg}`, `{REG_SEND, name, msg}`, `{LINK, from, to}`, `{EXIT, from, to, reason}`, `{MONITOR, from, to, ref}`, `{DEMONITOR, from, to, ref}`, `{MONITOR_EXIT, from, to, ref, reason}`, `{SPAWN_REQ, id, from, mfa, opts}`, `{SPAWN_REPLY, id, to, pid_or_error}`.

### Network Partition Handling

**BEAM behavior:**
- When a connection drops (TCP timeout, keepalive failure, explicit disconnect):
  - All remote links fire exit signals with reason `noconnection`.
  - All remote monitors fire `{:DOWN, ref, process, pid, noconnection}`.
  - All node monitors fire `{nodedown, Node}`.
  - The node is removed from `nodes()`.
- BEAM does NOT automatically heal partitions. The `dist_auto_connect once` mode prevents automatic reconnection (reconnection after split causes instability with global registries, distributed applications, etc.).
- Resolution is left to the application layer. Mnesia fires `{inconsistent_database, running_partitioned_network, Node}` events for application-level handling.

**Snow behavior (recommendation):**
- Same behavior on connection drop: fire all link exit signals, monitor DOWN messages, and nodedown events with reason `:noconnection`.
- NO automatic partition healing. Provide events so applications can decide.
- `Node.connect` can be called explicitly to attempt reconnection after partition.
- Document clearly: `:noconnection` means "we lost the connection" not "the remote process is dead." The remote process may still be running. Applications must handle this ambiguity.

### Distributed Global Process Registry

**BEAM behavior:**
- `global` module: strongly consistent, slow (locks across all nodes for each registration). Single name -> single PID cluster-wide.
- `pg` module (OTP 23+): eventually consistent process groups. Multiple PIDs per group. Scope-based for scalability. Based on CloudI Process Groups (cpg).
- Third-party: `syn` (fast, handles net splits), `gproc` (extended features but gen_leader concerns).

**Snow behavior (recommendation):**
- Start with a simple cluster-wide name registry (like `global` but simpler). One name -> one PID across all nodes.
- Use a coordinator-based approach: one node (the one where registration is requested) broadcasts the registration to all connected nodes. Conflict resolution: first-writer-wins with node name as tiebreaker.
- On `:nodedown`, registrations owned by the disconnected node are removed from local registry copies.
- Do NOT implement process groups (pg equivalent) in MVP. The simple name registry covers the primary use case. Groups can be added as a differentiator later.

---

## Feature Dependencies

```
Node naming
  |
  +--> Cookie authentication
  |      |
  |      +--> Node connection (handshake)
  |             |
  |             +--> Automatic mesh formation
  |             |
  |             +--> Heartbeat / tick
  |             |
  |             +--> Node monitoring (:nodedown/:nodeup)
  |             |
  |             +--> Binary wire format
  |                    |
  |                    +--> Location-transparent PIDs
  |                    |      |
  |                    |      +--> Remote message delivery
  |                    |      |
  |                    |      +--> Remote process monitoring
  |                    |      |
  |                    |      +--> Remote spawn
  |                    |      |      |
  |                    |      |      +--> Cross-node supervision
  |                    |      |
  |                    |      +--> node(pid) introspection
  |                    |
  |                    +--> Distributed global registry
  |                    |
  |                    +--> Cross-node WS room broadcast
  |
  +--> Node registry (epmd equivalent)

Node registry (epmd equivalent) [independent of connection, needed for discovery]
```

### Dependency Ordering for Phases

1. **Binary wire format** must come first or in parallel with node connection, because the handshake itself needs serialization (node names, capabilities, challenges).
2. **Node naming + registry** are prerequisites for everything.
3. **Node connection** (TLS TCP + handshake + auth) unlocks all cross-node features.
4. **Location-transparent PIDs** are the pivotal feature -- without them, nothing else works transparently.
5. **Remote send/monitoring/spawn** can be built incrementally once PIDs and connections work.
6. **Global registry and cross-node WS rooms** are the highest-level features, built on everything below.

---

## MVP Recommendation

**Prioritize (must ship):**
1. Binary wire format -- the foundation for all serialization
2. Node naming, registry, and connection (cookie auth + TLS)
3. Location-transparent PIDs with automatic routing
4. Remote message delivery (the core "distributed send" feature)
5. Remote process monitoring (`:DOWN` on crash or net split)
6. Node monitoring (`:nodedown`/`:nodeup`)
7. Connection loss handling (firing all link/monitor signals)

**Include if possible:**
8. Remote spawn (`Node.spawn`)
9. Heartbeat/tick for connection liveness
10. `nodes()` listing and `node(pid)` introspection

**Defer:**
- Distributed global registry: complex coordination problem, can be added in v5.1
- Cross-node WS room broadcast: requires registry + additional protocol design, defer to v5.1
- Cross-node supervision: high complexity, requires remote spawn + monitoring to be solid first
- Atom cache for wire format: optimization, not correctness
- Connection backoff and retry: convenience, not essential for MVP

---

## Complexity Assessment by Feature Area

| Feature Area | Complexity | Rationale |
|-------------|------------|-----------|
| Wire format | HIGH | Must handle all Snow types correctly, be extensible, handle nested structures. Every bug here corrupts messages silently. |
| Node connection | HIGH | TLS handshake + custom protocol handshake + cookie auth + mesh formation. Multiple failure modes. Must be robust against malformed input. |
| Location-transparent PIDs | HIGH | Changes the fundamental PID representation. Must be backward compatible with all existing local actor code. Touches type system, codegen, and runtime. |
| Remote send | MEDIUM | Once PIDs and connections work, routing is straightforward. Main complexity is in the wire format (already counted). |
| Remote monitoring | HIGH | Must correctly handle all failure modes: process exit, node disconnect, race conditions between monitor setup and process death. |
| Node monitoring | MEDIUM | Simpler than process monitoring. One event per connection state change. |
| Remote spawn | HIGH | Requires sending function references over the wire. Snow compiles to native code -- function pointers are memory addresses. Must use function names/symbols for remote identification. |
| Node registry | MEDIUM | Simple TCP service. Can be a separate process or built into the node. |
| Global process registry | HIGH | Distributed coordination problem. Consistency vs. availability tradeoff. Net split handling. |
| Cross-node WS rooms | MEDIUM | Existing room infrastructure + distribution connection. Main challenge is keeping room membership synchronized. |

---

## Sources

### Primary (HIGH confidence)
- [Distributed Erlang System Documentation v28.3.1](https://www.erlang.org/doc/system/distributed.html) -- node naming, connection, PIDs, remote spawn
- [Distribution Protocol (erts v16.2)](https://www.erlang.org/doc/apps/erts/erl_dist_protocol.html) -- handshake, EPMD, cookie auth, control messages, protocol versioning
- [External Term Format (erts v16.2)](https://www.erlang.org/doc/apps/erts/erl_ext_dist.html) -- ETF type tags, encoding format, wire format specification
- [Processes Reference Manual v28.3.1](https://www.erlang.org/doc/system/ref_man_processes.html) -- monitors, links, signal ordering guarantees
- [Erlang Blog: Message Passing](https://www.erlang.org/blog/message-passing/) -- ordering guarantees, remote delivery, noconnection semantics
- Snow codebase analysis -- actor runtime, process.rs, link.rs, registry.rs, scheduler.rs, rooms.rs (direct file reads)

### Secondary (MEDIUM confidence)
- [Akka Location Transparency](https://doc.akka.io/libraries/akka-core/current/general/remoting.html) -- Akka's approach to transparent routing, actor paths, configuration-driven deployment
- [Akka.NET Actor References and Addressing](https://getakka.net/articles/concepts/addressing.html) -- path-based actor identity model
- [Orleans Overview (Microsoft Learn)](https://learn.microsoft.com/en-us/dotnet/orleans/overview) -- virtual actor model, grain placement, silo architecture, activation lifecycle
- [Orleans Grain Placement](https://learn.microsoft.com/en-us/dotnet/orleans/grains/grain-placement) -- placement strategies, resource-optimized vs. random
- [Learn You Some Erlang: Distribunomicon](https://learnyousomeerlang.com/distribunomicon) -- practical distributed Erlang patterns, CAP theorem discussion
- [Syn: Scalable Global Process Registry](https://github.com/ostinelli/syn) -- distributed registry design, net split resolution
- [Erlang pg module (OTP 24)](https://www.erlang.org/docs/24/man/pg.html) -- distributed process groups, scope-based design
- [EEF Security WG: Distribution Security](https://erlef.github.io/security-wg/secure_coding_and_deployment_hardening/distribution.html) -- TLS for distribution, EPMD security recommendations

### Tertiary (LOW confidence)
- [An Evaluation of Erlang Global Process Registries](https://www.ostinelli.net/an-evaluation-of-erlang-global-process-registries-meet-syn/) -- comparative analysis of global, gproc, syn
- [Programming Distributed Erlang Applications: Pitfalls and Recipes](https://www.researchgate.net/publication/221211336_Programming_distributed_Erlang_applications_pitfalls_and_recipes) -- academic paper on distributed Erlang pitfalls
