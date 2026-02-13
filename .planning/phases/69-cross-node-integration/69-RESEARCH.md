# Phase 69: Cross-Node Integration - Research

**Researched:** 2026-02-13
**Domain:** Distributed WebSocket rooms and distributed supervision trees
**Confidence:** HIGH

## Summary

Phase 69 is the final phase of the v5.0 Distributed Actors milestone. It makes two existing subsystems -- WebSocket rooms (Phase 62) and supervision trees (Phases 53-55) -- work transparently across node boundaries using the distribution infrastructure built in Phases 63-68. The two requirements (CLUST-04 and CLUST-05) are largely independent of each other and can be implemented in parallel.

**WebSocket rooms (CLUST-04):** The current `RoomRegistry` in `ws/rooms.rs` is purely node-local. `snow_ws_broadcast` iterates local connection handles and writes frames directly to TCP streams. Cross-node broadcast requires a new wire message (`DIST_ROOM_BROADCAST`) that propagates the room name and message text to all connected nodes, where each node then performs local broadcast to its own room members. The room registry itself remains node-local (each node tracks only its own connections), but broadcast operations become cluster-wide by forwarding to peers.

**Supervision trees (CLUST-05):** The current supervisor in `actor/supervisor.rs` uses `scheduler.spawn()` to start children, which only creates local processes. For remote supervision, the supervisor must be able to: (1) spawn children on remote nodes via the existing `snow_node_spawn` infrastructure from Phase 67, and (2) monitor remote children via the existing remote monitor infrastructure from Phase 66. When a remote child crashes, the `DIST_MONITOR_EXIT` or `DIST_EXIT` message arrives at the supervisor's node, is delivered to its mailbox (since the supervisor has `trap_exit = true`), and the supervisor's existing `handle_child_exit` logic handles the restart. The key insight is that remote monitoring and remote spawn already work -- the supervisor just needs to use them instead of local-only primitives.

**Primary recommendation:** Structure as three plans: (1) Distributed WebSocket room broadcast -- new DIST_ROOM_BROADCAST wire message, cluster-wide broadcast functions, and per-node local delivery; (2) Remote supervision -- extend supervisor child spec to support remote node targeting, use `snow_node_spawn` for remote children, use remote monitors/links for crash detection, handle remote child restart; (3) Integration tests verifying both features work end-to-end across node boundaries.

## Standard Stack

### Core (already in Cargo.toml -- zero new dependencies)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| parking_lot | 0.12 | RwLock for room registry, Mutex for process/session locks | Already used throughout codebase |
| rustc-hash | 2 | FxHashMap for all registries | Already used everywhere |
| rustls | current | TLS streams for node sessions | Already used for distribution |

### Supporting (from existing crate modules)
| Module | Purpose | When to Use |
|--------|---------|-------------|
| `dist::node` | `NodeState`, `NodeSession`, `write_msg`, wire message tags | Sending DIST_ROOM_BROADCAST and using remote spawn |
| `dist::global` | Broadcast pattern (collect sessions, drop lock, iterate) | Template for room broadcast to all nodes |
| `ws::rooms` | `RoomRegistry`, `global_room_registry()`, `snow_ws_broadcast` | Extending with cluster-aware broadcast |
| `ws::server` | `WsConnection`, `write_frame` | Local delivery of broadcast messages |
| `actor::supervisor` | `SupervisorState`, `start_single_child`, `handle_child_exit` | Extending for remote children |
| `actor::link` | `EXIT_SIGNAL_TAG`, `DOWN_SIGNAL_TAG`, exit signal encoding | Remote failure detection for supervisors |
| `dist::node::snow_node_spawn` | Remote process spawning | Spawning children on remote nodes |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| New DIST_ROOM_BROADCAST wire message | Reuse DIST_SEND to room members by PID | DIST_SEND requires knowing each remote member's PID; DIST_ROOM_BROADCAST is simpler (just room name + text) and avoids room membership tracking across nodes |
| Per-room cross-node membership tracking | Broadcast-to-all-nodes approach | Per-room tracking adds complexity (join/leave sync); broadcast-to-all-nodes is simpler and relies on each node knowing its own members. Cost is O(nodes) messages per broadcast vs O(nodes_with_members), but cluster sizes are small enough that this is negligible |
| New supervisor type for remote children | Extend existing ChildSpec with optional target_node field | Extension is simpler, minimally invasive, and lets existing supervisor logic work unchanged for local children |

**Installation:** No new dependencies needed.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-rt/src/
  dist/
    node.rs          # MODIFIED: add DIST_ROOM_BROADCAST wire tag (0x1E),
                     #           reader loop handler, room broadcast sender
  ws/
    rooms.rs         # MODIFIED: add cluster_broadcast functions that forward
                     #           to all connected nodes, plus local delivery
  actor/
    supervisor.rs    # MODIFIED: add remote child support (spawn on node,
                     #           remote monitor for crash detection)
    child_spec.rs    # MODIFIED: add optional target_node field to ChildSpec
    mod.rs           # MODIFIED: update snow_supervisor_start to handle
                     #           target_node in child specs, codegen support
```

### Pattern 1: Distributed Room Broadcast (Broadcast-to-All-Nodes)

**What:** When `snow_ws_broadcast(room, msg)` is called on any node, the message is first delivered locally to room members on the calling node, then forwarded via `DIST_ROOM_BROADCAST` to all connected nodes. Each remote node receives the wire message and performs local delivery to its own room members.

**When to use:** Every room broadcast call.

**Key design principle:** Room membership remains purely node-local. Each node only tracks its own WebSocket connections. The broadcast message is forwarded to ALL connected nodes (not just nodes that have members in the room), because tracking per-room membership across nodes adds complexity that is not worth it at typical cluster sizes (2-20 nodes).

**Wire format:**
```
DIST_ROOM_BROADCAST (0x1E):
[tag 0x1E][u16 room_name_len][room_name bytes][u32 msg_len][msg bytes]
```

**Flow:**
```
Node A: snow_ws_broadcast("lobby", "hello")
  1. Local delivery to Node A's lobby members (existing code)
  2. Build DIST_ROOM_BROADCAST payload
  3. Send to all connected node sessions (Phase 68 broadcast pattern)

Node B receives DIST_ROOM_BROADCAST:
  1. Reader loop decodes room_name and msg
  2. Calls local room broadcast (existing code path)
  3. Does NOT forward to other nodes (prevents infinite loop)
```

**Implementation pattern (from global.rs broadcast):**
```rust
// Follows the exact same collect-then-iterate pattern from global.rs
pub(crate) fn broadcast_room_to_cluster(room: &str, msg: &str) {
    let state = match crate::dist::node::node_state() {
        Some(s) => s,
        None => return,
    };

    let room_bytes = room.as_bytes();
    let msg_bytes = msg.as_bytes();
    let mut payload = Vec::with_capacity(1 + 2 + room_bytes.len() + 4 + msg_bytes.len());
    payload.push(DIST_ROOM_BROADCAST);
    payload.extend_from_slice(&(room_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(room_bytes);
    payload.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(msg_bytes);

    // Collect session Arcs under read lock, drop lock, then iterate
    let sessions: Vec<Arc<NodeSession>> = {
        let map = state.sessions.read();
        map.values().map(|s| Arc::clone(s)).collect()
    };

    for session in &sessions {
        let mut stream = session.stream.lock().unwrap();
        let _ = write_msg(&mut *stream, &payload);
    }
}
```

### Pattern 2: Remote Supervision via Existing Primitives

**What:** Extend the supervisor's child spec to include an optional `target_node: Option<String>` field. When a child has a target_node set, `start_single_child` uses `snow_node_spawn` instead of `scheduler.spawn()` to create the child on the remote node. The supervisor uses a remote monitor (or link, since supervisors have `trap_exit = true`) to detect remote child crashes.

**When to use:** When a child spec specifies a remote node target.

**Key insight:** The supervisor already works by:
1. Spawning children and linking to them (local link)
2. Trapping exit signals (`trap_exit = true`)
3. Receiving exit signals in mailbox
4. Looking up child by PID, calling `handle_child_exit`

For remote children, steps 2-4 are identical. Only step 1 changes: instead of local spawn + local link, we use remote spawn + remote link (via `snow_node_spawn` with `link_flag = 1`). The DIST_EXIT message from Phase 66 delivers the exit signal just like a local exit signal -- the supervisor's existing mailbox processing handles it.

**ChildSpec extension:**
```rust
pub struct ChildSpec {
    pub id: String,
    pub start_fn: *const u8,
    pub start_args_ptr: *const u8,
    pub start_args_size: u64,
    pub restart_type: RestartType,
    pub shutdown: ShutdownType,
    pub child_type: ChildType,
    // NEW: Optional target node for remote spawning
    pub target_node: Option<String>,
    // NEW: Function name for remote spawning (required when target_node is set)
    pub start_fn_name: Option<String>,
}
```

**Remote child spawn:**
```rust
fn start_single_child_remote(
    child: &mut ChildState,
    sup_pid: ProcessId,
    target_node: &str,
    fn_name: &str,
) -> Result<ProcessId, String> {
    // Use snow_node_spawn with link_flag=1 for bidirectional link
    let remote_pid = snow_node_spawn(
        target_node, fn_name,
        child.spec.start_args_ptr, child.spec.start_args_size,
        1, // link_flag -- establishes bidirectional link
    );
    if remote_pid == 0 {
        return Err("remote spawn failed".to_string());
    }
    child.pid = Some(ProcessId(remote_pid));
    child.running = true;
    Ok(ProcessId(remote_pid))
}
```

**Remote child crash detection:**
```
Remote Node: child crashes
  -> handle_process_exit propagates DIST_EXIT to supervisor's node
  -> Supervisor's node reader loop receives DIST_EXIT
  -> Since supervisor has trap_exit=true, exit signal delivered to mailbox
  -> Supervisor's receive loop picks it up
  -> handle_child_exit finds child by PID, applies restart strategy
  -> If restart needed, calls start_single_child_remote again
```

### Pattern 3: Remote Child Termination

**What:** Terminating a remote child during supervisor shutdown or restart strategy.

**Challenge:** The current `terminate_single_child` directly modifies the process state via `scheduler.get_process(child_pid)` -- this only works for local processes.

**Solution:** For remote children, send `Process.exit(child_pid, :shutdown)` via `snow_actor_exit(child_pid, 4)` which routes through DIST_EXIT to the remote node. Then wait for the DIST_EXIT response (child's actual exit) or timeout. This mirrors OTP's approach where supervisor sends exit signal to remote child and waits for the link exit message.

```rust
fn terminate_single_child(child: &mut ChildState, scheduler: &Scheduler, sup_pid: ProcessId) {
    let child_pid = match child.pid {
        Some(pid) => pid,
        None => { child.running = false; return; }
    };

    if child_pid.is_local() {
        // Existing local termination code (unchanged)
        // ... BrutalKill / Timeout logic ...
    } else {
        // Remote termination: send exit signal via distribution
        crate::dist::node::send_dist_exit(sup_pid, child_pid, &ExitReason::Shutdown);
        // Wait for acknowledgment (link exit signal back)
        // or timeout, then mark as not running
        let deadline = Instant::now() + Duration::from_millis(5000);
        loop {
            if !child.running { break; }
            if Instant::now() >= deadline { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    child.running = false;
    child.pid = None;
}
```

### Anti-Patterns to Avoid

- **Room membership replication across nodes:** Do NOT try to keep a cluster-wide room membership registry. It adds enormous complexity (join/leave sync, split-brain handling) for minimal benefit. Each node tracks only its own local connections. Broadcast forwards to all nodes unconditionally.

- **Re-implementing remote spawn for supervisors:** Do NOT build a new remote spawn mechanism. The existing `snow_node_spawn` from Phase 67 with `link_flag=1` does exactly what supervisors need -- spawn + bidirectional link in one operation.

- **Forwarding DIST_ROOM_BROADCAST between nodes:** Each node that receives DIST_ROOM_BROADCAST delivers only locally. It must NOT re-forward to other nodes, which would cause infinite broadcast storms. Only the originating node sends the wire message.

- **Blocking the supervisor coroutine for remote operations:** Remote spawn is already async (send request, wait for reply in mailbox via `snow_node_spawn`). The supervisor should not add additional blocking mechanisms.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Remote process spawning | Custom spawn-on-remote protocol | `snow_node_spawn` (Phase 67) | Already handles fn registry lookup, arg forwarding, reply delivery, and optional link |
| Remote crash detection | Custom heartbeat/monitoring for children | Remote links via `link_flag=1` + `trap_exit` (Phase 66) | Supervisor already traps exits; remote links deliver DIST_EXIT exactly like local exits |
| Node-to-node message broadcast | Custom pub/sub system | Collect-sessions-then-iterate pattern from `global.rs` | Proven pattern used by global registry, peer list exchange, and global sync |
| Remote process termination | Custom termination protocol | `send_dist_exit(sup_pid, child_pid, Shutdown)` (Phase 66) | Exit signal routing already works for remote PIDs |
| Cluster-wide room membership | Distributed set/CRDT | Local-only room registry + broadcast-to-all-nodes | Massively simpler; room membership is high-churn and doesn't need consistency guarantees |

**Key insight:** Phase 69 is fundamentally an integration phase. Every building block already exists. The work is wiring existing primitives together, not building new infrastructure.

## Common Pitfalls

### Pitfall 1: Infinite Broadcast Loop
**What goes wrong:** Node A receives DIST_ROOM_BROADCAST and re-forwards it to all connected nodes (including back to Node A or to other nodes that already received it), causing infinite message amplification.
**Why it happens:** Natural instinct to "relay" broadcast messages in a mesh.
**How to avoid:** DIST_ROOM_BROADCAST is only sent by the originating node. The receiver performs LOCAL delivery only, never re-forwarding. An "originator" field is not needed because the wire message is only ever sent point-to-point from originator to each peer.
**Warning signs:** Messages being received multiple times; CPU spike from message storm.

### Pitfall 2: Remote Supervisor PID Confusion
**What goes wrong:** Supervisor spawns child on remote node, gets back a local_id, but forgets to reconstruct the full remote PID (with node_id + creation). Later, `find_child_index` fails because the stored PID doesn't match the one in the exit signal.
**Why it happens:** `snow_node_spawn` returns a full remote-qualified PID via the mailbox reply (Phase 67 already handles this). But if the supervisor stores just the local_id, it won't match.
**How to avoid:** Store the full `ProcessId` (including node_id, creation, local_id) returned by `snow_node_spawn`. The PID comparison in `find_child_index` uses `ProcessId::eq` which compares the full u64.
**Warning signs:** "Unknown child" in exit handler logs; children not being restarted.

### Pitfall 3: Lock Ordering Between Room Registry and Session Locks
**What goes wrong:** Deadlock when broadcasting: room registry lock held while acquiring session stream lock, while another thread holds session lock and tries to acquire room registry lock.
**Why it happens:** The existing `snow_ws_broadcast` acquires room registry read lock to get members, then iterates members and acquires connection write_stream locks. The new cluster broadcast acquires session stream locks. If these nest incorrectly, deadlock.
**How to avoid:** Follow the established pattern: snapshot the data under the first lock, drop it, THEN iterate and acquire the second lock. The current `snow_ws_broadcast` already does this correctly (calls `registry.members(room)` which snapshots and releases). The cluster broadcast must also follow the collect-then-write pattern.
**Warning signs:** Application hangs under concurrent broadcast load.

### Pitfall 4: Remote Child Restart Targets Wrong Node
**What goes wrong:** Supervisor restarts a crashed remote child but spawns it locally instead of on the remote node.
**Why it happens:** `start_single_child` defaults to local spawn. If the restart path doesn't check `target_node`, the child silently becomes local.
**How to avoid:** The `ChildSpec.target_node` field persists across restarts. `start_single_child` must check this field and route to `start_single_child_remote` when set. This is data-driven, not control-flow dependent.
**Warning signs:** After restart, child PID has node_id=0 when it should have a remote node_id.

### Pitfall 5: Node Disconnect During Remote Child Restart
**What goes wrong:** Supervisor tries to restart a child on a node that just disconnected. `snow_node_spawn` returns 0 (failure), but the supervisor doesn't handle this gracefully.
**Why it happens:** Race condition between node disconnect and supervisor restart logic.
**How to avoid:** The existing error path in `start_single_child` already returns `Err(msg)` on spawn failure. The supervisor's `apply_strategy` propagates this error and may trigger restart limit exceeded, which is the correct OTP behavior -- if the target node is down, repeated restart failures will hit the limit and the supervisor terminates.
**Warning signs:** Supervisor repeatedly failing to restart, hitting restart limit.

### Pitfall 6: broadcast_except in Cluster Context
**What goes wrong:** `snow_ws_broadcast_except(room, msg, except_conn)` forwards to other nodes, but the `except_conn` pointer is meaningless on remote nodes. Remote nodes broadcast to ALL their local members, including connections that should have been excluded.
**Why it happens:** The `except_conn` is a raw pointer to a local WsConnection -- it has no meaning on other nodes.
**How to avoid:** `broadcast_except` should still forward to all nodes (the excluded connection is only on the local node by definition -- it's the sender's connection). Remote nodes deliver to ALL their local members, which is correct because the excluded connection is not on those nodes.
**Warning signs:** None -- this is actually correct behavior. The excluded connection is always local to the node calling broadcast_except.

## Code Examples

### Example 1: DIST_ROOM_BROADCAST Wire Tag and Reader Handler
```rust
// In node.rs -- add wire tag constant
pub(crate) const DIST_ROOM_BROADCAST: u8 = 0x1E;

// In reader_loop_session match arm
DIST_ROOM_BROADCAST => {
    // Wire format: [tag][u16 room_name_len][room_name][u32 msg_len][msg]
    if msg.len() >= 7 { // min: tag(1) + room_len(2) + room(>=1) + msg_len(4)
        let room_name_len = u16::from_le_bytes(
            msg[1..3].try_into().unwrap()
        ) as usize;
        if msg.len() >= 3 + room_name_len + 4 {
            if let Ok(room_name) = std::str::from_utf8(
                &msg[3..3 + room_name_len]
            ) {
                let msg_len = u32::from_le_bytes(
                    msg[3 + room_name_len..7 + room_name_len]
                        .try_into().unwrap()
                ) as usize;
                if msg.len() >= 7 + room_name_len + msg_len {
                    if let Ok(text) = std::str::from_utf8(
                        &msg[7 + room_name_len..7 + room_name_len + msg_len]
                    ) {
                        // Deliver locally to room members on THIS node
                        local_room_broadcast(room_name, text);
                    }
                }
            }
        }
    }
}
```

### Example 2: Cluster-Aware Room Broadcast
```rust
// In ws/rooms.rs -- new function wrapping existing broadcast + cluster forward
pub fn cluster_broadcast(room: &str, msg: &str) -> i64 {
    let payload = msg.as_bytes();

    // Step 1: Local delivery (existing code path)
    let members = global_room_registry().members(room);
    let mut failures = 0i64;
    for conn_usize in members {
        let conn = unsafe { &*(conn_usize as *const WsConnection) };
        if conn.shutdown.load(Ordering::SeqCst) { continue; }
        let mut stream = conn.write_stream.lock();
        if write_frame(&mut *stream, WsOpcode::Text, payload, true).is_err() {
            failures += 1;
        }
    }

    // Step 2: Forward to all connected nodes
    broadcast_room_to_cluster(room, msg);

    failures
}
```

### Example 3: ChildSpec with Remote Node Target
```rust
// Extended ChildSpec
pub struct ChildSpec {
    pub id: String,
    pub start_fn: *const u8,
    pub start_args_ptr: *const u8,
    pub start_args_size: u64,
    pub restart_type: RestartType,
    pub shutdown: ShutdownType,
    pub child_type: ChildType,
    pub target_node: Option<String>,    // NEW
    pub start_fn_name: Option<String>,  // NEW (required for remote spawn)
}
```

### Example 4: Remote-Aware start_single_child
```rust
pub fn start_single_child(
    child: &mut ChildState,
    scheduler: &Scheduler,
    sup_pid: ProcessId,
) -> Result<ProcessId, String> {
    // Route to remote spawn if target_node is set
    if let Some(ref node) = child.spec.target_node {
        let fn_name = child.spec.start_fn_name.as_deref()
            .ok_or("remote child requires start_fn_name")?;
        return start_single_child_remote(child, sup_pid, node, fn_name);
    }

    // Existing local spawn code (unchanged)
    let child_pid = scheduler.spawn(
        child.spec.start_fn, child.spec.start_args_ptr,
        child.spec.start_args_size, 1,
    );
    // ... link, update state ...
    Ok(child_pid)
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Local-only room broadcast | Cluster-wide room broadcast via DIST_ROOM_BROADCAST | Phase 69 (this phase) | Room messages reach all nodes transparently |
| Local-only child spawning in supervisors | Remote child spawning via snow_node_spawn | Phase 69 (this phase) | Supervisors can manage children across the cluster |
| Node-scoped fault isolation | Cross-node fault propagation and recovery | Phase 66 + 69 | Remote crashes trigger supervisor restart strategies |

**No deprecated features** -- this phase only adds capabilities on top of existing infrastructure.

## Open Questions

1. **Supervisor config wire format for target_node**
   - What we know: The supervisor config is serialized from compiled Snow programs via `parse_supervisor_config` in `actor/mod.rs`. Adding `target_node` requires extending this wire format.
   - What's unclear: The exact Snow language syntax for specifying `node: "name@host"` in a child spec. This depends on the Snow compiler's supervisor syntax.
   - Recommendation: Extend the binary config format with an optional node name field (prefixed by a presence byte: 0=local, 1=remote + u16 name_len + name bytes). The compiler side can be updated separately.

2. **broadcast_except cluster semantics**
   - What we know: `snow_ws_broadcast_except` excludes one connection. On the originating node, this works as expected. On remote nodes, all members receive the message (the excluded connection is local to the originator).
   - What's unclear: Whether users expect `broadcast_except` to exclude a specific user across all nodes (e.g., "don't send to user X regardless of which node they're on").
   - Recommendation: For Phase 69, keep simple semantics: `except_conn` only applies on the originating node. This matches the current API contract. If user-level exclusion is needed, it's a future enhancement using global registry names.

3. **Remote supervision testing without real multi-node setup**
   - What we know: Existing Phase 63-68 tests use unit tests with mocked streams or wire format roundtrip tests. E2E distribution tests would need two actual node processes.
   - What's unclear: Whether the existing test infrastructure supports multi-process e2e tests.
   - Recommendation: Focus unit tests on the new logic paths (remote child spec handling, DIST_ROOM_BROADCAST encode/decode). Integration tests can use a single test binary that starts two logical nodes on different ports (similar to what Phase 64 testing would require). The supervisor unit tests in `supervisor.rs` already mock the scheduler -- they can be extended with remote PID scenarios.

## Sources

### Primary (HIGH confidence)
- `crates/snow-rt/src/ws/rooms.rs` -- Current room registry implementation, broadcast functions, connection tracking
- `crates/snow-rt/src/actor/supervisor.rs` -- Current supervisor state management, child lifecycle, restart strategies
- `crates/snow-rt/src/dist/node.rs` -- Distribution infrastructure: wire protocol, session management, reader loop, disconnect handling, remote spawn
- `crates/snow-rt/src/dist/global.rs` -- Broadcast pattern template (collect sessions, drop lock, iterate)
- `crates/snow-rt/src/actor/child_spec.rs` -- ChildSpec and ChildState structures
- `crates/snow-rt/src/actor/mod.rs` -- snow_supervisor_start, snow_actor_send, local_send, dist_send
- `crates/snow-rt/src/actor/scheduler.rs` -- handle_process_exit, exit signal propagation to remote links/monitors
- `crates/snow-rt/src/actor/link.rs` -- Exit signal encoding/decoding, link/unlink, monitor references

### Secondary (MEDIUM confidence)
- `.planning/phases/62-rooms-channels/62-RESEARCH.md` -- Design rationale for room registry
- `.planning/phases/66-remote-links-monitors-failure-handling/66-RESEARCH.md` -- Remote failure handling architecture
- `.planning/phases/68-global-registry/68-VERIFICATION.md` -- Verified global registry integration

### Tertiary (LOW confidence)
- None -- all findings are based on direct codebase investigation

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components already exist in codebase, zero new dependencies
- Architecture: HIGH -- patterns directly copied from existing proven implementations (global.rs broadcast, snow_node_spawn remote spawning, existing supervisor restart logic)
- Pitfalls: HIGH -- identified from direct code reading of lock ordering, PID handling, and broadcast semantics
- Wire format: HIGH -- follows the exact same conventions as DIST_GLOBAL_REGISTER, DIST_SPAWN, etc.

**Research date:** 2026-02-13
**Valid until:** 2026-03-13 (stable -- internal codebase, no external dependencies to change)
