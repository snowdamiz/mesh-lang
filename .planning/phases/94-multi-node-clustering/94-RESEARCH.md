# Phase 94: Multi-Node Clustering - Research

**Researched:** 2026-02-15
**Domain:** Distributed systems integration -- using existing Mesh distributed actor primitives (Node.connect, Global.register, Node.spawn, Ws.broadcast) to make the Mesher monitoring platform run as a multi-node cluster
**Confidence:** HIGH

## Summary

Phase 94 is an **application-level integration phase**, not a runtime infrastructure phase. All the distributed primitives needed -- node discovery and mesh formation (Phase 64-65), remote send (Phase 65), remote links and monitors (Phase 66), remote spawn (Phase 67), global process registry (Phase 68), distributed WebSocket room broadcast (Phase 69), and remote supervision (Phase 69) -- already exist and are verified. The task is to wire these into the Mesher application code so that multiple Mesher nodes can form a cluster and distribute work.

The current Mesher architecture is a single-node monolith: `main.mpl` starts services (EventProcessor, StorageWriter, StreamManager, RateLimiter, PipelineRegistry), registers PipelineRegistry via `Process.register("mesher_registry", pid)` (node-local), and all HTTP/WS handlers look up services via `Process.whereis("mesher_registry")`. This design works for a single node but breaks in a cluster because `Process.register`/`Process.whereis` are node-local -- a handler on node B cannot find the PipelineRegistry registered on node A.

The transformation requires five changes: (1) Start the node with `Node.start` and connect to peers with `Node.connect` for mesh formation; (2) Register key services (PipelineRegistry, EventProcessor, StorageWriter) in the global registry via `Global.register` so any node can discover them; (3) Use `Ws.broadcast` (already cluster-aware since Phase 69) for cross-node dashboard streaming; (4) Route events across nodes by looking up remote EventProcessor/StorageWriter PIDs via `Global.whereis`; (5) Spawn remote processor actors via `Node.spawn` when local load is high.

**Primary recommendation:** Structure as 3 plans: (1) Node startup, mesh formation, and global service registration; (2) Cross-node event routing and distributed service discovery; (3) Load-based remote processor spawning with monitoring.

## Standard Stack

### Core (Zero new dependencies -- everything is in the existing runtime)
| Library/Module | Version | Purpose | Why Standard |
|----------------|---------|---------|--------------|
| Node.start | Phase 64 | Start named node with cookie auth | Built-in Mesh stdlib |
| Node.connect | Phase 64 | Connect to peer nodes, auto mesh formation | Built-in Mesh stdlib |
| Node.self | Phase 65 | Get own node identity | Built-in Mesh stdlib |
| Node.list | Phase 65 | List connected nodes | Built-in Mesh stdlib |
| Node.spawn | Phase 67 | Spawn actors on remote nodes | Built-in Mesh stdlib |
| Node.spawn_link | Phase 67 | Spawn + link for supervised remote actors | Built-in Mesh stdlib |
| Global.register | Phase 68 | Register service names cluster-wide | Built-in Mesh stdlib |
| Global.whereis | Phase 68 | Look up services on any node | Built-in Mesh stdlib |
| Ws.broadcast | Phase 69 | Already cluster-aware room broadcast | Built-in Mesh stdlib |
| Process.register | Phase 6 | Node-local name registration | Built-in Mesh stdlib |
| Process.whereis | Phase 6 | Node-local name lookup | Built-in Mesh stdlib |

### Supporting (Existing Mesher modules -- will be modified)
| Module | Purpose | What Changes |
|--------|---------|-------------|
| `ingestion/pipeline.mpl` | Service startup orchestration | Add Node.start, Node.connect, Global.register calls; add load monitoring actor |
| `main.mpl` | Entry point | Accept node name, cookie, and peer addresses as config |
| `ingestion/routes.mpl` | HTTP handlers | Replace `Process.whereis` with `Global.whereis` for cross-node service discovery |
| `ingestion/ws_handler.mpl` | WebSocket handlers | Replace `Process.whereis` with `Global.whereis` |
| `services/event_processor.mpl` | Event processing | No change needed -- already a service actor reachable by PID |
| `services/writer.mpl` | Storage writer | No change needed -- already a service actor reachable by PID |
| `services/stream_manager.mpl` | WS stream state | Keep node-local (connection handles are node-local pointers) |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Global.register for service discovery | Hardcoded node addresses | Global registry is dynamic, handles restarts automatically; hardcoded is fragile |
| Node.connect with manual peer list | Automatic DNS-based discovery | Node.connect is what the runtime provides; DNS discovery would require runtime changes out of scope |
| Shared PostgreSQL for coordination | Actor messaging | PostgreSQL is already shared (all nodes connect to same PG); but using it for coordination adds polling latency. Actor messaging is sub-millisecond. Use PG only for persistent data, actors for coordination. |
| Single writer node (all events routed to one node) | Per-node writers with shared PG | Per-node writers avoid a single bottleneck; PostgreSQL handles concurrent writes from multiple connections. Use per-node writers. |

## Architecture Patterns

### Recommended Project Structure
```
mesher/
├── main.mpl              # MODIFIED: Add node config (name, cookie, peers), call start_node
├── ingestion/
│   ├── pipeline.mpl      # MODIFIED: Add start_node, connect_peers, global registration,
│   │                     #           load monitor actor, remote processor spawning
│   ├── routes.mpl        # MODIFIED: Replace Process.whereis with Global.whereis for
│   │                     #           cross-node service discovery
│   └── ws_handler.mpl    # MODIFIED: Replace Process.whereis with Global.whereis
├── services/
│   ├── event_processor.mpl  # UNCHANGED: Already a service actor, PID-addressable
│   ├── writer.mpl           # UNCHANGED: Already a service actor, PID-addressable
│   ├── stream_manager.mpl   # UNCHANGED: Kept node-local (connection handles are pointers)
│   └── rate_limiter.mpl     # UNCHANGED: Kept node-local (rate limiting is per-ingestion-node)
├── api/ (all files)         # MODIFIED: Replace Process.whereis with Global.whereis
└── storage/ (all files)     # UNCHANGED: SQL layer is stateless, uses pool handle
```

### Pattern 1: Node Startup and Mesh Formation (CLUSTER-01)

**What:** Each Mesher node starts with a unique name and shared cookie, then connects to known peer addresses to form a mesh. Mesh auto-gossips peer lists, so connecting to one peer eventually connects to all.

**When to use:** At application startup in `main.mpl`, before starting services.

**Example:**
```mesh
# In main.mpl or pipeline.mpl
fn start_node(node_name :: String, cookie :: String, peer :: String) do
  let _ = Node.start(node_name, cookie)
  println("[Mesher] Node started: " <> node_name)

  # Connect to seed peer (mesh auto-gossip handles the rest)
  if peer != "" do
    let result = Node.connect(peer)
    if result == 0 do
      println("[Mesher] Connected to peer: " <> peer)
    else
      println("[Mesher] Failed to connect to peer: " <> peer)
    end
  else
    println("[Mesher] No peers configured (standalone mode)")
  end
end
```

**Key constraint:** Node names must be unique across the cluster (e.g., `"mesher1@host1:9100"`, `"mesher2@host2:9100"`). The cookie must be identical on all nodes. The distribution port (in the node name) is separate from the HTTP/WS ports.

### Pattern 2: Global Service Registration (CLUSTER-02)

**What:** After starting services, register them in the global registry so any node can discover them. Use a naming convention like `"mesher_registry@<node_name>"` to distinguish registrations per node.

**When to use:** In `start_pipeline` after `Process.register`.

**Design choice -- single shared PipelineRegistry vs per-node registries:**

Each node should run its own full set of services (EventProcessor, StorageWriter, RateLimiter, StreamManager) because:
1. Each node has its own HTTP/WS server accepting requests
2. StorageWriter needs a local PG pool handle (not serializable across nodes)
3. RateLimiter is per-ingestion-point by design
4. StreamManager tracks local WebSocket connection handles (raw pointers, node-local)

Register each node's PipelineRegistry globally with a node-specific name:
```mesh
fn register_global_services(registry_pid, node_name :: String) do
  # Register this node's registry globally for cross-node discovery
  let _ = Global.register("mesher_registry@" <> node_name, registry_pid)

  # Also register a "well-known" name for the first node that starts
  # (used as a fallback; Global.register rejects duplicates silently)
  let _ = Global.register("mesher_registry", registry_pid)

  println("[Mesher] Services registered globally")
end
```

**Why per-node registries:** A handler on node A that receives an event for a project whose writer is on node B can look up `Global.whereis("mesher_registry@nodeB")` to get nodeB's PipelineRegistry PID, then call `PipelineRegistry.get_writer(remote_pid)` which is a cross-node service call (transparent thanks to Phase 65's remote send).

### Pattern 3: Cross-Node Event Routing (CLUSTER-03)

**What:** When an event is ingested on node A, it is processed locally by node A's EventProcessor. The Ws.broadcast for dashboard streaming already reaches all nodes (Phase 69). Events are stored via the local StorageWriter (all nodes connect to the same PostgreSQL). No explicit cross-node event routing is needed for the basic case.

For load distribution, if node A is overloaded, it can forward events to node B's EventProcessor:

```mesh
# In a load-aware event router:
fn route_event_distributed(project_id :: String, writer_pid, event_json :: String) do
  let local_reg = Process.whereis("mesher_registry")
  let local_processor = PipelineRegistry.get_processor(local_reg)

  # Check local load (processed_count or queue depth)
  # If high, find a remote processor
  let result = EventProcessor.process_event(local_processor, project_id, writer_pid, event_json)
  result
end
```

**Key insight:** Since all nodes share the same PostgreSQL, it does not matter which node processes and stores an event. The critical requirement is that Ws.broadcast reaches all nodes -- which it already does since Phase 69.

### Pattern 4: Distributed WebSocket Broadcast (CLUSTER-04)

**What:** `Ws.broadcast(room, msg)` already sends to local room members AND forwards to all connected cluster nodes (Phase 69 `broadcast_room_to_cluster`). No application-level changes needed.

**When to use:** Already used in `routes.mpl` (`broadcast_event`, `broadcast_issue_update`, `broadcast_issue_count`). These calls automatically become cluster-aware once Node.start/Node.connect are called.

**Verification:** The existing `Ws.broadcast` calls in `routes.mpl` and `pipeline.mpl` will automatically broadcast across nodes. The StreamManager remains node-local (it tracks local WS connection handles), but Ws.broadcast handles the cross-node delivery at the runtime level.

### Pattern 5: Remote Processor Spawning Under Load (CLUSTER-05)

**What:** A load monitor actor on each node tracks event processing rate. When the rate exceeds a threshold, it spawns additional EventProcessor actors on remote nodes using `Node.spawn`.

**When to use:** When local EventProcessor queue/processing time exceeds a configurable threshold.

```mesh
actor load_monitor(pool :: PoolHandle, threshold :: Int) do
  Timer.sleep(5000)  # Check every 5 seconds

  # Get local processor count (from pipeline registry)
  let reg_pid = Process.whereis("mesher_registry")
  let processor_pid = PipelineRegistry.get_processor(reg_pid)

  # Check connected nodes
  let nodes = Node.list()
  let node_count = List.length(nodes)

  if node_count > 0 do
    # Spawn a remote processor if load is high
    # Node.spawn(target_node, function, args...)
    let target = List.head(nodes)
    let remote_pid = Node.spawn(target, event_processor_worker, pool)
    println("[Mesher] Spawned remote processor on " <> target)
  else
    0
  end

  load_monitor(pool, threshold)
end
```

**Constraint:** `Node.spawn` requires the function to be registered in the FUNCTION_REGISTRY (Phase 67). All actors and functions defined in the Mesher binary are automatically registered at startup. The remote node must be running the same Mesher binary (same function names).

### Anti-Patterns to Avoid

- **Routing all events through a single "coordinator" node:** Creates a bottleneck. Each node should process its own ingested events. Use cross-node routing only for load balancing.
- **Using Global.register for StreamManager:** StreamManager tracks local WS connection handles (raw pointers). These are meaningless on remote nodes. Keep StreamManager node-local.
- **Replacing ALL Process.whereis with Global.whereis:** Some lookups (like `stream_manager`) MUST remain node-local. Only replace lookups that need to find services on other nodes (like `mesher_registry` for cross-node event routing).
- **Synchronous cross-node calls in the hot path:** Service calls across nodes have network latency. Prefer local processing with async cross-node forwarding for load balancing.
- **Starting Node.start before PostgreSQL connection:** The PG pool must be available for schema creation and service initialization. Start the node (Node.start + Node.connect) after PG is ready but before HTTP/WS servers start accepting traffic.
- **Forgetting that Global.register rejects duplicates:** If two nodes both try `Global.register("mesher_registry", pid)`, only the first succeeds. Use node-specific names or accept first-writer-wins semantics.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Node discovery | Custom TCP discovery protocol | `Node.connect` + mesh auto-gossip (Phase 65) | Peer list exchange already built into the runtime |
| Cross-node service lookup | Custom service registry | `Global.register` / `Global.whereis` (Phase 68) | Fully replicated, auto-cleanup on node disconnect |
| Cross-node WebSocket broadcast | Custom forwarding logic | `Ws.broadcast` (already cluster-aware, Phase 69) | Forwards to all connected nodes automatically |
| Remote actor spawning | Custom work distribution | `Node.spawn` / `Node.spawn_link` (Phase 67) | Full spawn protocol with function name registry |
| Remote supervision | Custom health monitoring | Supervisor with `target_node` in child spec (Phase 69) | Restart-on-same-node, DIST_EXIT propagation |
| Dead node detection | Custom heartbeat | Runtime heartbeat (Phase 64, 60s/15s) + `Node.monitor` (Phase 66) | Automatic NODEDOWN events |
| Split-brain resolution | Custom consensus protocol | Accept eventual consistency + manual recovery | At 2-5 node scale, split-brain is rare; Mesh's global registry uses first-writer-wins |

**Key insight:** This phase is 100% application-level integration. Every distributed primitive is already implemented and tested in the Mesh runtime (Phases 63-69). The work is modifying Mesher's .mpl files to call these APIs.

## Common Pitfalls

### Pitfall 1: PoolHandle Not Serializable Across Nodes
**What goes wrong:** Attempting to pass a PoolHandle to a remote EventProcessor via `Node.spawn(node, processor_fn, pool)`. PoolHandle is a raw pointer to the connection pool -- meaningless on another machine.
**Why it happens:** The STF wire format serializes PoolHandle as an integer (raw pointer value). The remote node receives a garbage pointer.
**How to avoid:** Each node MUST create its own PG connection pool in `main.mpl`. Remote processors must use the pool handle from their own node. If you need to spawn a processor on a remote node, that node must already have a pool handle registered (via its own PipelineRegistry).
**Warning signs:** Segfault on remote node when trying to execute a query with a forwarded PoolHandle.

### Pitfall 2: StreamManager is Node-Local
**What goes wrong:** Registering StreamManager globally and having remote nodes try to manage local WS connections via the global PID.
**Why it happens:** WS connection handles are raw pointers (`WsConnection*` cast to `usize`). They point to memory on the local node and are meaningless remotely.
**How to avoid:** Keep `Process.register("stream_manager", pid)` as node-local. Each node manages its own streaming connections. The cross-node broadcast is handled by `Ws.broadcast` at the runtime level, not the application level.
**Warning signs:** Segfault or garbage data when calling `StreamManager.register_client` with a remote connection handle.

### Pitfall 3: Race Between Node.connect and Global.register
**What goes wrong:** Node A starts and registers services globally. Node B starts, connects to A, and immediately tries `Global.whereis("mesher_registry@nodeA")` -- returns 0 because the global sync hasn't completed yet.
**Why it happens:** `Node.connect` returns after the handshake, but global registry sync is asynchronous (Phase 68).
**How to avoid:** Add a brief delay after `Node.connect` before relying on `Global.whereis` for remote services. Or use a retry loop: `let pid = Global.whereis(name); if pid == 0 do Timer.sleep(100); retry() end`.
**Warning signs:** Intermittent "service not found" errors during startup on the second node.

### Pitfall 4: HTTP.serve and Ws.serve Are Blocking
**What goes wrong:** If `Node.start` / `Node.connect` are called after `HTTP.serve`, they never execute because `HTTP.serve` blocks the main function.
**Why it happens:** `HTTP.serve` enters a blocking accept loop (current Mesher architecture).
**How to avoid:** Call `Node.start` and `Node.connect` BEFORE `HTTP.serve` (and `Ws.serve`, which is non-blocking). The order should be: PG connect -> schema -> services -> node start -> peer connect -> WS serve -> HTTP serve.
**Warning signs:** Node never joins the cluster; `Node.list()` always returns empty.

### Pitfall 5: Single Default StorageWriter for All Projects
**What goes wrong:** The current Mesher uses a single `StorageWriter.start(pool, "default")` for all events. In a cluster, each node runs its own StorageWriter. This means events for the same project may be written by different nodes' writers. This is actually fine (PostgreSQL handles concurrent writes), but the "default" project_id in the writer state is misleading.
**Why it happens:** The StorageWriter was designed as a single-project buffer, but the EventProcessor passes the actual project_id when calling `insert_event`.
**How to avoid:** No change needed. The StorageWriter's `project_id` field in `WriterState` is used as a label; the actual project_id comes from the event's `extract_event_fields` pipeline. Multiple writers writing to the same PG are safe.
**Warning signs:** None -- this works correctly as-is.

### Pitfall 6: Split-Brain When Network Partitions Occur
**What goes wrong:** If the network between two groups of nodes is severed, each group continues operating independently. Global registry entries diverge. When the partition heals, there may be conflicting registrations.
**Why it happens:** Mesh's global registry is eventually consistent with no leader. Split-brain is inherent to AP systems.
**How to avoid:** At Mesher's target scale (2-5 nodes), split-brain is rare. Since all nodes share the same PostgreSQL, data is always consistent in PG. The only inconsistency is in the in-memory global registry and WS broadcast routing. When the partition heals, node reconnection triggers global registry sync (Phase 68), which converges. For Mesher's use case (monitoring tool, not financial transactions), brief inconsistency in WS streaming is acceptable.
**Warning signs:** Dashboard on one group of nodes stops receiving events from the other group during partition. Automatically resolves when partition heals.

## Code Examples

### Node Startup Integration in main.mpl

```mesh
# Modified main.mpl structure:
fn main() do
  println("[Mesher] Connecting to PostgreSQL...")
  let pool_result = Pool.open("postgres://mesh:mesh@localhost:5432/mesher", 2, 10, 5000)
  case pool_result do
    Ok(pool) ->
      # Start distributed node (before services, after PG)
      start_node("mesher1@localhost:9100", "mesher_secret_cookie", "")
      start_services(pool)
    Err(_) -> println("[Mesher] Failed to connect to PostgreSQL")
  end
end
```

### Global Service Registration in pipeline.mpl

```mesh
# In start_pipeline, after existing Process.register calls:
pub fn start_pipeline(pool :: PoolHandle) do
  # ... existing service startup ...

  let registry_pid = PipelineRegistry.start(pool, rate_limiter_pid, processor_pid, writer_pid)
  let _ = Process.register("mesher_registry", registry_pid)

  # NEW: Register globally for cross-node discovery
  let node_name = Node.self()
  if node_name != "" do
    let _ = Global.register("mesher_registry@" <> node_name, registry_pid)
    let _ = Global.register("mesher_registry", registry_pid)  # first-writer-wins default
    println("[Mesher] Services registered globally as mesher_registry@" <> node_name)
  else
    println("[Mesher] Running in standalone mode (no distribution)")
  end

  # ... existing health checker, spike checker, alert evaluator spawns ...
  registry_pid
end
```

### Cross-Node Service Discovery in Route Handlers

```mesh
# Current pattern (node-local only):
let reg_pid = Process.whereis("mesher_registry")

# New pattern (cluster-aware with local fallback):
fn get_registry() do
  # Try local first (fastest path)
  let local = Process.whereis("mesher_registry")
  if local != 0 do
    local
  else
    # Fall back to global (finds registry on any node)
    Global.whereis("mesher_registry")
  end
end
```

**Note:** In practice, every node runs its own PipelineRegistry, so `Process.whereis("mesher_registry")` will always succeed locally. The global lookup is needed only for finding registries on OTHER nodes for load distribution.

### Load Monitor Actor for Remote Spawning

```mesh
# Track processing rate and spawn remote processors when overloaded.
actor load_monitor(pool :: PoolHandle, check_interval :: Int, spawn_threshold :: Int) do
  Timer.sleep(check_interval)

  let nodes = Node.list()
  let node_count = List.length(nodes)

  # Only attempt load balancing if there are peer nodes
  if node_count > 0 do
    # Simple round-robin: pick a peer node
    let target = List.head(nodes)
    println("[Mesher] Load monitor: " <> String.from(node_count) <> " peers available")
    # Future: spawn remote processor if local queue depth exceeds threshold
  else
    0
  end

  load_monitor(pool, check_interval, spawn_threshold)
end
```

## State of the Art

| Old Approach (Current) | New Approach (Phase 94) | Impact |
|------------------------|------------------------|--------|
| Single-node Mesher | Multi-node cluster with mesh formation | Horizontal scalability |
| `Process.register`/`Process.whereis` (node-local) | `Global.register`/`Global.whereis` (cluster-wide) | Cross-node service discovery |
| `Ws.broadcast` (already cluster-aware but unused) | Actively leveraged in multi-node deployment | Dashboard streaming reaches all nodes |
| No load balancing | Load monitor + remote processor spawning | Distribute processing across nodes |
| No fault tolerance across nodes | Node.monitor + automatic reconnection | Detect and recover from node failures |

**No deprecated/outdated APIs:** All distributed actor APIs (Node.*, Global.*, Ws.broadcast) were implemented in Phases 63-69 and verified. The runtime crate name is `mesh-rt` (renamed from `snow-rt` in quick task 1).

## Open Questions

1. **How should node configuration be provided?**
   - What we know: Mesh doesn't have command-line argument parsing or environment variable reading (beyond what Env.get provides). Node name, cookie, and peer addresses need to be configurable per deployment.
   - What's unclear: Should these be hardcoded with different values per binary? Read from environment variables via `Env.get`? Read from a config file?
   - Recommendation: Use `Env.get` for `MESHER_NODE_NAME`, `MESHER_COOKIE`, `MESHER_PEERS`. Default to standalone mode if not set. This is the simplest approach that works for both development and production.

2. **Should load-based remote spawning be a simple actor or use supervision?**
   - What we know: `Node.spawn_link` provides crash propagation. Supervisor `target_node` in child_spec provides supervised remote actors (Phase 69).
   - What's unclear: How complex should the load-balancing logic be? Simple round-robin? Based on actual metrics?
   - Recommendation: Start with a simple load monitor actor that uses `Node.list()` to find peers and `Node.spawn` to place processors. Do NOT build a sophisticated load balancer -- this is a dogfooding exercise, not a production scheduler. The load monitor should track a simple metric (e.g., events processed in last interval) and spawn remote helpers when it exceeds a threshold.

3. **Should each node run its own HTTP and WebSocket servers?**
   - What we know: Yes -- each node needs to accept ingestion traffic. A load balancer in front distributes HTTP traffic across Mesher nodes. Each node runs HTTP on port 8080 and WS on port 8081.
   - What's unclear: Should the ports be configurable per node?
   - Recommendation: Use `Env.get` for `MESHER_HTTP_PORT` and `MESHER_WS_PORT` with defaults of 8080 and 8081. This allows running multiple nodes on the same machine for testing.

4. **What about the retention_cleaner and alert_evaluator -- should only one node run them?**
   - What we know: Currently every node would run its own retention_cleaner (daily) and alert_evaluator (30s). Running multiple cleaners is wasteful but safe (idempotent SQL). Running multiple alert evaluators could fire duplicate alerts.
   - What's unclear: Should these be singleton actors elected via the global registry?
   - Recommendation: For Phase 94, accept that each node runs its own background actors. retention_cleaner is idempotent (DELETE WHERE already handles duplicates). For alert_evaluator, the fire_alert function uses cooldown logic (should_fire_by_cooldown) which prevents duplicate firing even if multiple nodes evaluate the same rule. Document that a future phase could use global registry election for singleton actors.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** (direct file reads of existing Mesher application):
  - `mesher/main.mpl` -- Current entry point, service startup, HTTP/WS server binding
  - `mesher/ingestion/pipeline.mpl` -- PipelineRegistry, service startup orchestration, background actors
  - `mesher/ingestion/routes.mpl` -- HTTP handlers with Process.whereis pattern
  - `mesher/ingestion/ws_handler.mpl` -- WebSocket handlers with Process.whereis pattern
  - `mesher/services/event_processor.mpl` -- EventProcessor service (PID-addressable)
  - `mesher/services/writer.mpl` -- StorageWriter service (per-node, PG pool)
  - `mesher/services/stream_manager.mpl` -- StreamManager (node-local, WS connection handles)
  - `mesher/services/rate_limiter.mpl` -- RateLimiter (node-local)
  - `mesher/services/retention.mpl` -- Retention cleaner (idempotent, safe for multi-node)

- **Runtime infrastructure verification** (Phases 63-69 VERIFICATION.md reports):
  - Phase 63: PID bit-packing, STF wire format -- 14/14 truths verified
  - Phase 64: Node.start, Node.connect, TLS, cookie auth, heartbeat -- 5/5 criteria verified
  - Phase 65: Remote send, named send, mesh formation, Node.list/self -- 5/5 truths verified
  - Phase 66: Remote links, monitors, node disconnect failure handling -- 4/4 criteria verified
  - Phase 67: Node.spawn, Node.spawn_link, FUNCTION_REGISTRY, codegen pipeline -- 3/3 truths verified
  - Phase 68: Global.register, Global.whereis, sync-on-connect, cleanup-on-disconnect -- 5/5 truths verified
  - Phase 69: Distributed WS room broadcast, remote supervision -- 9/9 truths verified

- **Runtime source code** (direct reads):
  - `crates/mesh-rt/src/dist/node.rs` -- mesh_node_start, mesh_node_connect, mesh_node_spawn, mesh_node_self, mesh_node_list
  - `crates/mesh-rt/src/dist/global.rs` -- GlobalRegistry with fully-replicated name table, sync-on-connect, cleanup-on-disconnect
  - `crates/mesh-rt/src/ws/rooms.rs` -- broadcast_room_to_cluster (already wired in Ws.broadcast since Phase 69)
  - `crates/mesh-rt/src/actor/supervisor.rs` -- Remote child spawning via target_node in ChildSpec
  - `crates/mesh-typeck/src/infer.rs` -- Node, Global, Process module type signatures

### Secondary (MEDIUM confidence)
- `.planning/STATE.md` -- Prior decision: "Phase 94 may need research-phase for split-brain handling"
- `.planning/REQUIREMENTS.md` -- CLUSTER-01 through CLUSTER-05 definitions
- `.planning/ROADMAP.md` -- Phase 94 success criteria and dependency on Phase 90

### Tertiary (LOW confidence)
- Split-brain handling research: Based on general distributed systems knowledge. At Mesher's 2-5 node target scale with shared PostgreSQL, split-brain is a bounded problem. The global registry converges on reconnection. Needs validation in actual multi-node testing.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- zero new dependencies; all APIs already exist and are verified in Phases 63-69
- Architecture: HIGH -- direct analysis of existing Mesher code reveals clear integration points; transformation is well-defined
- Pitfalls: HIGH -- identified from actual code analysis (PoolHandle serialization, StreamManager locality, startup ordering, race conditions)
- Split-brain handling: MEDIUM -- based on general distributed systems knowledge; Mesh's global registry has known eventual-consistency semantics documented in Phase 68 research
- Load-based spawning: MEDIUM -- the mechanism (Node.spawn) is verified, but the load metric and threshold policy are design choices

**Research date:** 2026-02-15
**Valid until:** 2026-03-17 (stable domain; all runtime APIs frozen since Phase 69)
