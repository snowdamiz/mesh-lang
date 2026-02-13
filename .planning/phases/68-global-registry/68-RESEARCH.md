# Phase 68: Global Registry - Research

**Researched:** 2026-02-12
**Domain:** Distributed process name registration, replicated name tables, cluster-wide cleanup on node disconnect
**Confidence:** HIGH

## Summary

Phase 68 delivers a cluster-wide process name registry: `Global.register(name, pid)` makes a name visible on all connected nodes, `Global.whereis(name)` resolves a name to a PID from any node, and registrations are automatically cleaned up when the owning node disconnects. This is the distributed counterpart to the existing local `ProcessRegistry` in `registry.rs`.

The existing infrastructure is remarkably complete. The local `ProcessRegistry` (in `crates/snow-rt/src/actor/registry.rs`) already provides `register(name, pid)`, `whereis(name)`, `unregister(name)`, and `cleanup_process(pid)` with a bi-directional `names <-> pid_names` map protected by `RwLock`. The distribution layer (in `crates/snow-rt/src/dist/node.rs`) provides wire message framing (`write_msg`/`read_dist_msg`), session management (`NodeState.sessions`), node disconnect handling (`cleanup_session` -> `handle_node_disconnect`), and existing wire tags up to `0x1A`. The codegen pipeline has a proven pattern for adding new "module" APIs: add to `STDLIB_MODULES` in `lower.rs`, add type signatures in the type checker (`infer.rs`), declare intrinsics in `intrinsics.rs`, and implement the extern "C" functions in the runtime.

The key architectural decision is the replication strategy. Following Erlang's `:global` model, Snow should use **fully replicated name tables** -- every node holds a complete copy of the global registry, lookups are always local (fast), and mutations are broadcast to all peers. This avoids a central coordinator, is simple to implement, and matches Snow's existing mesh topology (3-20 nodes). The alternative (a leader-based or consensus-based approach) adds complexity that is unnecessary at Snow's target scale.

**Primary recommendation:** Structure as three plans: (1) Global registry data structure and runtime APIs -- build a `GlobalRegistry` in `snow-rt` with register/whereis/unregister/cleanup, three new wire tags (`DIST_GLOBAL_REGISTER`, `DIST_GLOBAL_UNREGISTER`, `DIST_GLOBAL_SYNC`), reader-loop handlers, and process/node disconnect cleanup hooks; (2) Compiler integration -- add `Global` module to the type checker, `STDLIB_MODULES`, intrinsic declarations, and MIR lowering for `Global.register`, `Global.whereis`, `Global.unregister`; (3) Synchronization and edge cases -- initial sync on node connect (send full registry snapshot), conflict resolution on duplicate names, and integration tests.

## Standard Stack

### Core (already in Cargo.toml -- zero new dependencies)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| parking_lot | 0.12 | RwLock for global registry concurrent access | Already used in ProcessRegistry, NodeState |
| rustc-hash | 2 | FxHashMap for name->PID and PID->names maps | Already used throughout codebase |

### Supporting (from existing crate modules)
| Module | Purpose | When to Use |
|--------|---------|-------------|
| `dist::node` | `NodeState`, `NodeSession`, `write_msg`, `read_dist_msg`, session iteration, `cleanup_session` | Broadcasting register/unregister to all peers, sending sync on connect |
| `actor::process` | `ProcessId` with `node_id()`, `local_id()`, `is_local()` | Identifying which node owns a registered process |
| `actor::registry` | Existing `ProcessRegistry` pattern (bi-directional map, cleanup_process) | Architectural reference for GlobalRegistry |
| `actor::scheduler` | `handle_process_exit` cleanup hook | Adding global registry cleanup on local process exit |
| `codegen::intrinsics` | `declare_intrinsics`, `get_intrinsic` | Declaring snow_global_register etc. |
| `mir::lower` | `STDLIB_MODULES`, `lower_field_access` | MIR lowering for Global.* calls |
| `typeck::infer` | `build_module_types` | Type signatures for Global module |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Fully replicated name table | Leader-based centralized registry | Replicated is simpler, no leader election needed, fast local reads; centralized is consistent but adds SPOF and latency. At 3-20 nodes, replication wins. |
| Synchronous broadcast (Erlang-style) | Asynchronous broadcast with eventual consistency | Synchronous ensures name is visible everywhere when register returns, but blocks the caller. Asynchronous is faster for the caller but has a visibility window. Recommend async with silent-drop semantics (matching Snow's existing distribution patterns). |
| Dedicated gen_server process (Erlang model) | Direct Rust data structure with RwLock | Erlang's `:global` runs as a gen_server because Erlang has no shared memory. Snow has shared-memory concurrency (RwLock), so a dedicated process is unnecessary overhead. Direct data structure is simpler and faster. |
| Name conflict resolution with callbacks | Last-writer-wins or reject-duplicate | Reject-duplicate (return error if name taken) is simpler and matches the local registry semantics. Conflict resolution callbacks add complexity for a rare edge case. |

**Installation:** No new dependencies. All work is within existing `snow-rt/src/dist/`, `snow-rt/src/actor/`, `snow-codegen/src/codegen/`, `snow-codegen/src/mir/`, and `snow-typeck/src/`.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-rt/src/
├── dist/
│   ├── mod.rs           # MODIFIED: re-export global registry module
│   ├── node.rs          # MODIFIED: DIST_GLOBAL_* wire tags, reader-loop handlers,
│   │                    #           sync on connect, cleanup on disconnect
│   └── global.rs        # NEW: GlobalRegistry data structure with replicated name table
├── actor/
│   ├── mod.rs           # MODIFIED: snow_global_register, snow_global_whereis,
│   │                    #           snow_global_unregister extern "C" functions
│   └── scheduler.rs     # MODIFIED: add global registry cleanup in handle_process_exit
└── lib.rs               # MODIFIED: re-export new extern "C" functions

crates/snow-codegen/src/
├── codegen/
│   ├── intrinsics.rs    # MODIFIED: declare snow_global_register, snow_global_whereis,
│   │                    #           snow_global_unregister
│   └── expr.rs          # MODIFIED: codegen for Global.register/whereis/unregister
│                        #           (string arg unpacking like Node.start)
├── mir/
│   └── lower.rs         # MODIFIED: add "Global" to STDLIB_MODULES

crates/snow-typeck/src/
└── infer.rs             # MODIFIED: add Global module type signatures
```

### Pattern 1: GlobalRegistry Data Structure (Replicated Name Table)

**What:** A `GlobalRegistry` struct mirroring the local `ProcessRegistry` pattern but with additional metadata for distributed operation. Each entry tracks the owning node name alongside the PID, enabling per-node cleanup on disconnect. Every node holds a complete replica.

**When to use:** All global name operations.

**Rationale:** Erlang's `:global` stores names in replica tables on every node with no central storage point. Lookups are always local. This provides O(1) `whereis` performance and avoids network round-trips for reads.

```rust
// Source: New file crates/snow-rt/src/dist/global.rs

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::sync::OnceLock;
use crate::actor::process::ProcessId;

/// Global process name registry, replicated across all cluster nodes.
///
/// Unlike the local ProcessRegistry, this tracks the owning node name
/// for each registration to enable cleanup when a node disconnects.
pub struct GlobalRegistry {
    /// name -> (PID, owning_node_name) mapping
    names: RwLock<FxHashMap<String, (ProcessId, String)>>,
    /// PID -> names reverse index for efficient cleanup on process exit
    pid_names: RwLock<FxHashMap<ProcessId, Vec<String>>>,
    /// node_name -> names index for efficient cleanup on node disconnect
    node_names: RwLock<FxHashMap<String, Vec<String>>>,
}

#[derive(Debug)]
pub enum GlobalRegisterError {
    NameAlreadyRegistered { name: String, existing_pid: ProcessId },
    NodeNotStarted,
}

impl GlobalRegistry {
    pub fn new() -> Self { /* ... */ }

    /// Register a name locally (called on the node that initiates the registration).
    /// Also called on remote nodes when DIST_GLOBAL_REGISTER arrives.
    pub fn register(&self, name: String, pid: ProcessId, node_name: String)
        -> Result<(), GlobalRegisterError> { /* ... */ }

    /// Look up a globally registered name. Always local (no network).
    pub fn whereis(&self, name: &str) -> Option<ProcessId> { /* ... */ }

    /// Unregister a name.
    pub fn unregister(&self, name: &str) -> bool { /* ... */ }

    /// Remove all registrations owned by a specific node.
    /// Called when a node disconnects.
    pub fn cleanup_node(&self, node_name: &str) -> Vec<String> { /* ... */ }

    /// Remove all registrations for a specific PID.
    /// Called when a local process exits.
    pub fn cleanup_process(&self, pid: ProcessId) -> Vec<String> { /* ... */ }

    /// Get all current registrations for syncing to a newly connected node.
    pub fn snapshot(&self) -> Vec<(String, ProcessId, String)> { /* ... */ }

    /// Bulk-insert registrations from a remote node's sync message.
    pub fn merge_snapshot(&self, entries: Vec<(String, ProcessId, String)>) { /* ... */ }
}

static GLOBAL_REGISTRY: OnceLock<GlobalRegistry> = OnceLock::new();

pub fn global_name_registry() -> &'static GlobalRegistry {
    GLOBAL_REGISTRY.get_or_init(GlobalRegistry::new)
}
```

**Key difference from local registry:** The `node_names` reverse index. When node B disconnects, we need to remove all global names owned by processes on node B. Without this index, we would need to scan the entire `names` table. With it, cleanup is O(k) where k is the number of names owned by the disconnected node.

### Pattern 2: Wire Protocol for Global Registration

**What:** Three new distribution message tags for replicating global registry operations across the cluster. These sit in the existing 0x1B-0x1F range (0x10-0x1A are taken).

**When to use:** Every global register/unregister operation and new node connection.

**Rationale:** Follows the same single-tag wire format pattern as all existing DIST_* messages. Broadcast to all connected sessions (not just one).

```rust
// New distribution message tags in dist/node.rs
pub(crate) const DIST_GLOBAL_REGISTER: u8 = 0x1B;
pub(crate) const DIST_GLOBAL_UNREGISTER: u8 = 0x1C;
pub(crate) const DIST_GLOBAL_SYNC: u8 = 0x1D;

// DIST_GLOBAL_REGISTER wire format:
// [tag][u16 name_len LE][name bytes][u64 pid LE][u16 node_name_len LE][node_name bytes]

// DIST_GLOBAL_UNREGISTER wire format:
// [tag][u16 name_len LE][name bytes]

// DIST_GLOBAL_SYNC wire format (bulk snapshot):
// [tag][u32 count LE]
// For each entry:
//   [u16 name_len LE][name bytes][u64 pid LE][u16 node_name_len LE][node_name bytes]
```

### Pattern 3: Broadcast on Register/Unregister

**What:** When `Global.register(name, pid)` is called, the runtime: (1) registers locally in the GlobalRegistry, (2) broadcasts a `DIST_GLOBAL_REGISTER` message to ALL connected sessions. When `Global.unregister(name)` is called, the same pattern with `DIST_GLOBAL_UNREGISTER`.

**When to use:** Every global registration/unregistration.

**Rationale:** Matches the existing pattern of iterating `state.sessions.read()` and writing to each session's stream. The broadcast is fire-and-forget (silent drop on write failure) matching Snow's existing distribution semantics.

```rust
/// Broadcast a global registry change to all connected nodes.
fn broadcast_global_register(name: &str, pid: ProcessId, node_name: &str) {
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    let name_bytes = name.as_bytes();
    let node_bytes = node_name.as_bytes();
    let mut payload = Vec::with_capacity(1 + 2 + name_bytes.len() + 8 + 2 + node_bytes.len());
    payload.push(DIST_GLOBAL_REGISTER);
    payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(name_bytes);
    payload.extend_from_slice(&pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&(node_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(node_bytes);

    let sessions = state.sessions.read();
    for session in sessions.values() {
        let mut stream = session.stream.lock().unwrap();
        let _ = write_msg(&mut *stream, &payload);
    }
}
```

### Pattern 4: Sync on Node Connect

**What:** When a new node connects (after handshake + session registration), send a `DIST_GLOBAL_SYNC` snapshot of the local node's global registry entries. The newly connected node merges these into its own replica. Both sides do this, so after mutual exchange both have the union of all global names.

**When to use:** Immediately after `register_session` + `spawn_session_threads`, alongside the existing `send_peer_list` call.

**Rationale:** Without initial sync, a node that connects to an existing cluster would not know about any globally registered names. The sync must be bidirectional: the new node sends its names to the existing cluster, and the existing node sends its names to the new node. Since mesh formation means the new node will eventually connect to all nodes, each node only needs to send its own view (which already includes names from all nodes it knows about).

```rust
/// Send our global registry snapshot to a newly connected node.
fn send_global_sync(session: &Arc<NodeSession>) {
    let registry = global_name_registry();
    let snapshot = registry.snapshot();

    if snapshot.is_empty() {
        return;
    }

    let mut payload = Vec::new();
    payload.push(DIST_GLOBAL_SYNC);
    payload.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());

    for (name, pid, node_name) in &snapshot {
        let name_bytes = name.as_bytes();
        payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        payload.extend_from_slice(name_bytes);
        payload.extend_from_slice(&pid.as_u64().to_le_bytes());
        let node_bytes = node_name.as_bytes();
        payload.extend_from_slice(&(node_bytes.len() as u16).to_le_bytes());
        payload.extend_from_slice(node_bytes);
    }

    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
}
```

### Pattern 5: Cleanup on Node Disconnect

**What:** When a node disconnects (detected by reader loop timeout or heartbeat failure), `cleanup_session` calls `handle_node_disconnect`. Phase 68 adds a global registry cleanup step: remove all global names owned by the disconnected node, then broadcast `DIST_GLOBAL_UNREGISTER` for each removed name to remaining nodes.

**When to use:** In `handle_node_disconnect` (after existing link/monitor cleanup).

**Rationale:** Erlang's `:global` subscribes to nodedown events and unregisters all names for the disconnected node. Snow already has the `handle_node_disconnect` function that handles links, monitors, and nodedown events. Adding global registry cleanup here is a natural extension.

```rust
// In handle_node_disconnect, after existing link/monitor cleanup:

// Phase 68: Clean up global registrations for the disconnected node.
let removed_names = global_name_registry().cleanup_node(node_name);
for name in &removed_names {
    broadcast_global_unregister(name);
}
```

### Pattern 6: Cleanup on Local Process Exit

**What:** When a local process exits, `handle_process_exit` in `scheduler.rs` already calls `registry::global_registry().cleanup_process(pid)` for the local registry. Phase 68 adds a similar call for the global registry, plus broadcasts unregister messages for any globally-held names.

**When to use:** In `handle_process_exit` (alongside existing local registry cleanup at line 671).

**Rationale:** Erlang's `:global` monitors registered PIDs and unregisters names when processes terminate. This ensures names don't become stale (pointing to dead processes).

```rust
// In handle_process_exit, after local registry cleanup:

// Phase 68: Clean up global registrations for the exiting process.
let removed_global_names = crate::dist::global::global_name_registry().cleanup_process(pid);
if !removed_global_names.is_empty() {
    for name in &removed_global_names {
        crate::dist::global::broadcast_global_unregister(name);
    }
}
```

### Pattern 7: Compiler Integration (Global Module)

**What:** Add "Global" as a new stdlib module with three functions: `Global.register(name, pid)`, `Global.whereis(name)`, and `Global.unregister(name)`. This follows the exact same pattern used for `Node`, `Process`, `Timer`, etc.

**When to use:** Snow programs that need cluster-wide name registration.

**Four integration points:**

1. **`infer.rs` (type checker):** Add Global module with type signatures:
```rust
let mut global_mod = HashMap::new();
// Global.register: fn(String, Int) -> Int  (name, pid -> 0 on success, 1 on error)
global_mod.insert("register".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::string(), Ty::int()],
    Ty::int(),
)));
// Global.whereis: fn(String) -> Int  (name -> pid, or 0 if not found)
global_mod.insert("whereis".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::string()],
    Ty::int(),
)));
// Global.unregister: fn(String) -> Int  (name -> 0 on success)
global_mod.insert("unregister".to_string(), Scheme::mono(Ty::fun(
    vec![Ty::string()],
    Ty::int(),
)));
modules.insert("Global".to_string(), global_mod);
```

2. **`lower.rs` (MIR lowering):** Add `"Global"` to `STDLIB_MODULES`. The existing `lower_field_access` mechanism will map `Global.register` -> `global_register`, then `map_builtin_name` maps to `snow_global_register`.

3. **`intrinsics.rs` (LLVM declarations):** Declare the three new functions:
```rust
// snow_global_register(name_ptr: ptr, name_len: i64, pid: i64) -> i64
module.add_function("snow_global_register",
    i64_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false),
    Some(Linkage::External));

// snow_global_whereis(name_ptr: ptr, name_len: i64) -> i64
module.add_function("snow_global_whereis",
    i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
    Some(Linkage::External));

// snow_global_unregister(name_ptr: ptr, name_len: i64) -> i64
module.add_function("snow_global_unregister",
    i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
    Some(Linkage::External));
```

4. **`expr.rs` (codegen):** Add special handling for `snow_global_register` (needs string ptr/len unpacking, like `codegen_node_start` does for `Node.start`). `snow_global_whereis` and `snow_global_unregister` need the same string unpacking via `codegen_node_string_call` pattern.

### Anti-Patterns to Avoid
- **Central coordinator / leader election:** At Snow's target scale (3-20 nodes), fully replicated tables with broadcast are simpler and correct. A central coordinator adds SPOF, complexity, and is unnecessary.
- **Synchronous broadcast (blocking caller until all nodes ACK):** This would require a request-reply protocol with timeout handling for every register call. Asynchronous broadcast with eventual consistency is simpler and matches Snow's existing fire-and-forget distribution semantics.
- **Using the local ProcessRegistry for global names:** The local and global registries serve different purposes. A local name is node-scoped; a global name is cluster-scoped. Mixing them would cause confusion when the same name is registered locally and globally.
- **Allowing closures or non-serializable PIDs as global registry values:** Global registration values must be PIDs (u64 integers) that can be sent over the wire. Do not allow arbitrary values.
- **Querying remote nodes for whereis:** The whole point of replication is that `whereis` is always local. Never send a network message for a lookup.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Wire message framing | Custom framing | Existing `write_msg` / `read_dist_msg` length-prefixed protocol | Already used by all DIST_* messages |
| Session iteration for broadcast | Custom node list | `state.sessions.read().values()` | Already how peer_list and other broadcasts work |
| Node disconnect detection | Custom heartbeat | Existing `handle_node_disconnect` in `node.rs` | Already handles links, monitors, nodedown |
| Process exit detection | Custom monitoring | Existing `handle_process_exit` in `scheduler.rs` | Already cleans up local registry, links, monitors |
| Module API compiler integration | Custom AST/IR nodes | Existing STDLIB_MODULES + `lower_field_access` pattern | How all modules (Node, Timer, List, etc.) work |
| String argument unpacking in codegen | Manual LLVM IR | Existing `codegen_node_string_call` pattern | Handles ptr+len extraction from Snow strings |
| Concurrent hash map | Custom lock-free structure | `parking_lot::RwLock<FxHashMap>` | Already proven pattern in ProcessRegistry and NodeState |

**Key insight:** The global registry is architecturally a combination of two existing patterns: (1) the local `ProcessRegistry` (data structure, bi-directional maps, cleanup_process) and (2) the distribution broadcast pattern (iterate sessions, write DIST_* message to each). Both are well-established in the codebase. Phase 68 is primarily a **composition exercise**.

## Common Pitfalls

### Pitfall 1: Race Between Register and Node Connect
**What goes wrong:** Node A registers a global name. Node C connects to A. A sends its sync snapshot. Meanwhile, A also broadcasts the register to existing nodes (B). Node C gets the name from sync. Node B gets it from broadcast. But if C connected to B before A's broadcast arrived at B, B's sync to C might not include the name yet.
**Why it happens:** Network messages arrive in different orders on different paths.
**How to avoid:** This is inherent to eventually-consistent replication. Accept that there is a brief window where not all nodes agree. The sync mechanism (DIST_GLOBAL_SYNC sent on every new connection) ensures convergence. If a duplicate name arrives via both sync and broadcast, `merge_snapshot` should skip names that are already registered (idempotent insert). This matches Snow's existing "silently drop" semantics.
**Warning signs:** Test failures where `whereis` returns `None` immediately after another node's `register` call. Add a small delay in tests.

### Pitfall 2: Name Conflict on Concurrent Registration
**What goes wrong:** Node A and Node B simultaneously register the same name with different PIDs. Both succeed locally and both broadcast. Both receive the other's broadcast and reject it (name already taken). Result: A thinks the name points to PID-A, B thinks it points to PID-B.
**Why it happens:** No global lock or consensus protocol.
**How to avoid:** For Snow's target use case (service registration at startup), simultaneous registration of the same name is a programming error, not a runtime scenario. The simplest correct behavior: on receiving a `DIST_GLOBAL_REGISTER` for a name that is already registered locally, **reject the incoming registration** (keep the existing one). The broadcasting node will have its version; the receiving node keeps its version. This is an inconsistency, but it is detectable (the sender's register returns success but the name resolves differently on other nodes). For Phase 68's scope, document this limitation. A future phase could add conflict resolution (Erlang's `random_exit_name` callback).
**Warning signs:** `whereis` returns different PIDs on different nodes. In practice, this won't occur if programs register names at startup before other nodes connect (the common pattern).

### Pitfall 3: Deadlock During Broadcast
**What goes wrong:** `broadcast_global_register` acquires `state.sessions.read()` and then `session.stream.lock()` for each session. If another thread is holding `session.stream.lock()` and trying to acquire `sessions.write()`, deadlock occurs.
**Why it happens:** Nested lock acquisition in different orders.
**How to avoid:** Follow the existing pattern in the codebase: read `sessions` to collect session references, drop the sessions lock, THEN iterate and write to each session's stream individually. This is how `send_peer_list` works. Never hold the sessions lock while acquiring a stream lock.
**Warning signs:** Hangs during multi-node tests when register is called concurrently with new node connections.

### Pitfall 4: Global Registry Not Initialized Before Node Start
**What goes wrong:** A program calls `Global.register(name, pid)` before `Node.start()`. The global registry works (it's a local data structure), but there are no sessions to broadcast to. When nodes later connect and sync, the registration may or may not be visible depending on timing.
**How to avoid:** This is acceptable behavior. The global registry is always available (initialized via `OnceLock`). If no nodes are connected, the name is registered locally and will be synced when nodes connect (because the sync snapshot includes all entries). No special handling needed.
**Warning signs:** None -- this works correctly by design.

### Pitfall 5: Stale PID After Node Restart
**What goes wrong:** Node A registers `"db_service"` globally with PID-X. Node A crashes and restarts. The new incarnation of A gets a new creation counter. Other nodes still have PID-X (with old creation) in their global registry. Messages sent to PID-X are silently dropped because the creation counter doesn't match the new node's creation.
**Why it happens:** The `cleanup_node` call in `handle_node_disconnect` removes A's registrations. But if A reconnects before all nodes have processed the disconnect, there's a window where the old entry exists.
**How to avoid:** The `handle_node_disconnect` -> `cleanup_node` path already runs before any reconnection can be processed (disconnect is handled synchronously in the reader loop before the session is available for new connections). When A reconnects, the sync exchange provides A's new registrations (if any). The old stale entries are already cleaned up. No additional handling needed.
**Warning signs:** Very brief windows where `whereis` returns a stale PID. Acceptable for Snow's target scale.

### Pitfall 6: Missing "Global" in STDLIB_MODULES
**What goes wrong:** `Global.register(...)` in Snow source fails to compile with "unknown function" error.
**Why it happens:** The `STDLIB_MODULES` list in `lower.rs` does not include `"Global"`.
**How to avoid:** Add `"Global"` to the STDLIB_MODULES constant. Verify the `map_builtin_name` function maps `"global_register"` -> `"snow_global_register"` etc.
**Warning signs:** Compilation errors on any `Global.*` call.

### Pitfall 7: Broadcast Storm on Sync
**What goes wrong:** When a new node connects to a large cluster, all existing nodes send their full snapshots. The new node receives N snapshots each containing M names, processing N*M entries.
**Why it happens:** Each connected node sends its complete view of the global registry.
**How to avoid:** At Snow's target scale (3-20 nodes, likely <100 global names), this is not a practical problem. Each sync message is small (name strings + PIDs). For future scaling, the sync could be optimized to only send entries that the new node might not have (based on a version vector). For Phase 68, accept the O(N*M) cost during connect -- it's a one-time event per connection.
**Warning signs:** Slow node join with very large numbers of global registrations. Not a concern at target scale.

## Code Examples

### GlobalRegistry Core Implementation
```rust
// Source: Architecture design based on existing ProcessRegistry (registry.rs)
// File: crates/snow-rt/src/dist/global.rs

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::sync::OnceLock;
use crate::actor::process::ProcessId;

pub struct GlobalRegistry {
    names: RwLock<FxHashMap<String, (ProcessId, String)>>,
    pid_names: RwLock<FxHashMap<ProcessId, Vec<String>>>,
    node_names: RwLock<FxHashMap<String, Vec<String>>>,
}

impl GlobalRegistry {
    pub fn new() -> Self {
        GlobalRegistry {
            names: RwLock::new(FxHashMap::default()),
            pid_names: RwLock::new(FxHashMap::default()),
            node_names: RwLock::new(FxHashMap::default()),
        }
    }

    pub fn register(&self, name: String, pid: ProcessId, node_name: String) -> Result<(), String> {
        let mut names = self.names.write();
        if let Some((existing_pid, _)) = names.get(&name) {
            return Err(format!("name '{}' already globally registered to {}", name, existing_pid));
        }
        names.insert(name.clone(), (pid, node_name.clone()));
        self.pid_names.write().entry(pid).or_default().push(name.clone());
        self.node_names.write().entry(node_name).or_default().push(name);
        Ok(())
    }

    pub fn whereis(&self, name: &str) -> Option<ProcessId> {
        self.names.read().get(name).map(|(pid, _)| *pid)
    }

    pub fn unregister(&self, name: &str) -> bool {
        let mut names = self.names.write();
        if let Some((pid, node_name)) = names.remove(name) {
            let mut pid_names = self.pid_names.write();
            if let Some(list) = pid_names.get_mut(&pid) {
                list.retain(|n| n != name);
                if list.is_empty() { pid_names.remove(&pid); }
            }
            let mut node_names = self.node_names.write();
            if let Some(list) = node_names.get_mut(&node_name) {
                list.retain(|n| n != name);
                if list.is_empty() { node_names.remove(&node_name); }
            }
            true
        } else {
            false
        }
    }

    pub fn cleanup_node(&self, node_name: &str) -> Vec<String> {
        let names_to_remove = {
            let mut node_names = self.node_names.write();
            node_names.remove(node_name).unwrap_or_default()
        };
        if !names_to_remove.is_empty() {
            let mut names = self.names.write();
            let mut pid_names = self.pid_names.write();
            for name in &names_to_remove {
                if let Some((pid, _)) = names.remove(name) {
                    if let Some(list) = pid_names.get_mut(&pid) {
                        list.retain(|n| n != name);
                        if list.is_empty() { pid_names.remove(&pid); }
                    }
                }
            }
        }
        names_to_remove
    }

    pub fn cleanup_process(&self, pid: ProcessId) -> Vec<String> {
        let names_to_remove = {
            let mut pid_names = self.pid_names.write();
            pid_names.remove(&pid).unwrap_or_default()
        };
        if !names_to_remove.is_empty() {
            let mut names = self.names.write();
            let mut node_names = self.node_names.write();
            for name in &names_to_remove {
                if let Some((_, node_name)) = names.remove(name) {
                    if let Some(list) = node_names.get_mut(&node_name) {
                        list.retain(|n| n != name);
                        if list.is_empty() { node_names.remove(&node_name); }
                    }
                }
            }
        }
        names_to_remove
    }

    pub fn snapshot(&self) -> Vec<(String, ProcessId, String)> {
        self.names.read().iter()
            .map(|(name, (pid, node))| (name.clone(), *pid, node.clone()))
            .collect()
    }

    pub fn merge_snapshot(&self, entries: Vec<(String, ProcessId, String)>) {
        let mut names = self.names.write();
        let mut pid_names = self.pid_names.write();
        let mut node_names = self.node_names.write();
        for (name, pid, node_name) in entries {
            // Skip if already registered (idempotent merge)
            if names.contains_key(&name) {
                continue;
            }
            names.insert(name.clone(), (pid, node_name.clone()));
            pid_names.entry(pid).or_default().push(name.clone());
            node_names.entry(node_name).or_default().push(name);
        }
    }
}

static GLOBAL_NAME_REGISTRY: OnceLock<GlobalRegistry> = OnceLock::new();

pub fn global_name_registry() -> &'static GlobalRegistry {
    GLOBAL_NAME_REGISTRY.get_or_init(GlobalRegistry::new)
}
```

### Runtime Extern "C" Functions
```rust
// Source: New extern "C" APIs in actor/mod.rs

/// Register a process globally across the cluster.
///
/// - `name_ptr`, `name_len`: UTF-8 name string
/// - `pid`: raw u64 PID value
///
/// Returns 0 on success, 1 on error (name already taken, node not started).
#[no_mangle]
pub extern "C" fn snow_global_register(
    name_ptr: *const u8,
    name_len: u64,
    pid: u64,
) -> u64 {
    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => return 1,
        }
    };

    let pid = ProcessId(pid);

    // Determine our node name for the owning_node field.
    let node_name = match crate::dist::node::node_state() {
        Some(s) => s.name.clone(),
        None => "nonode@nohost".to_string(),
    };

    let registry = crate::dist::global::global_name_registry();
    match registry.register(name.clone(), pid, node_name.clone()) {
        Ok(()) => {
            // Broadcast to all connected nodes.
            crate::dist::global::broadcast_global_register(&name, pid, &node_name);
            0
        }
        Err(_) => 1,
    }
}

/// Look up a globally registered name.
///
/// Returns the PID if found, 0 if not registered.
#[no_mangle]
pub extern "C" fn snow_global_whereis(
    name_ptr: *const u8,
    name_len: u64,
) -> u64 {
    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    match crate::dist::global::global_name_registry().whereis(name) {
        Some(pid) => pid.as_u64(),
        None => 0,
    }
}

/// Unregister a globally registered name.
///
/// Returns 0 on success, 1 if the name was not registered.
#[no_mangle]
pub extern "C" fn snow_global_unregister(
    name_ptr: *const u8,
    name_len: u64,
) -> u64 {
    let name = unsafe {
        let slice = std::slice::from_raw_parts(name_ptr, name_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => return 1,
        }
    };

    let registry = crate::dist::global::global_name_registry();
    if registry.unregister(&name) {
        crate::dist::global::broadcast_global_unregister(&name);
        0
    } else {
        1
    }
}
```

### Reader Loop Handlers
```rust
// Source: Extension of reader_loop_session in dist/node.rs

DIST_GLOBAL_REGISTER => {
    // Wire format: [tag][u16 name_len][name][u64 pid][u16 node_name_len][node_name]
    if msg.len() >= 3 {
        let name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        if msg.len() >= 3 + name_len + 8 + 2 {
            if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                let pid_raw = u64::from_le_bytes(
                    msg[3 + name_len..3 + name_len + 8].try_into().unwrap());
                let node_name_len = u16::from_le_bytes(
                    msg[3 + name_len + 8..3 + name_len + 10].try_into().unwrap()) as usize;
                if msg.len() >= 3 + name_len + 10 + node_name_len {
                    if let Ok(node_name) = std::str::from_utf8(
                        &msg[3 + name_len + 10..3 + name_len + 10 + node_name_len]) {
                        let _ = crate::dist::global::global_name_registry()
                            .register(name.to_string(), ProcessId(pid_raw), node_name.to_string());
                        // Silently drop if name already taken (conflict)
                    }
                }
            }
        }
    }
}

DIST_GLOBAL_UNREGISTER => {
    // Wire format: [tag][u16 name_len][name]
    if msg.len() >= 3 {
        let name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        if msg.len() >= 3 + name_len {
            if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                crate::dist::global::global_name_registry().unregister(name);
            }
        }
    }
}

DIST_GLOBAL_SYNC => {
    // Wire format: [tag][u32 count][(u16 name_len, name, u64 pid, u16 node_len, node)*]
    if msg.len() >= 5 {
        let count = u32::from_le_bytes(msg[1..5].try_into().unwrap()) as usize;
        let mut pos = 5;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if pos + 2 > msg.len() { break; }
            let name_len = u16::from_le_bytes(msg[pos..pos+2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + name_len + 10 > msg.len() { break; }
            let name = std::str::from_utf8(&msg[pos..pos+name_len]).unwrap_or("");
            pos += name_len;
            let pid_raw = u64::from_le_bytes(msg[pos..pos+8].try_into().unwrap());
            pos += 8;
            let node_len = u16::from_le_bytes(msg[pos..pos+2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + node_len > msg.len() { break; }
            let node_name = std::str::from_utf8(&msg[pos..pos+node_len]).unwrap_or("");
            pos += node_len;
            entries.push((name.to_string(), ProcessId(pid_raw), node_name.to_string()));
        }
        crate::dist::global::global_name_registry().merge_snapshot(entries);
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Local-only process registry | Cluster-wide global registry | Phase 68 | Service discovery works across nodes |
| No `Global.*` API in Snow | `Global.register`, `Global.whereis`, `Global.unregister` | Phase 68 | Users can register processes for cluster-wide lookup |
| Node disconnect only cleans local registry + links + monitors | Also cleans global names owned by disconnected node | Phase 68 | No stale global names after node loss |

**Deprecated/outdated:**
- The local `ProcessRegistry` (in `actor/registry.rs`) remains unchanged and active. It serves a different purpose (node-local named processes for `send({name, node}, msg)` syntax). The global registry is a separate, cluster-scoped system.

## Open Questions

1. **Should `Global.register` take a PID argument or use the current process?**
   - What we know: The success criteria says `Global.register(name, pid)` (explicit PID argument). The existing local `snow_actor_register` uses the current process implicitly (gets PID from `stack::get_current_pid()`).
   - What's unclear: Should the user be able to register any PID globally, or only their own? Erlang allows registering any PID.
   - Recommendation: Support explicit PID argument as specified in the success criteria: `Global.register(name, pid)`. This is more flexible (a supervisor can register its children). The type signature is `fn(String, Int) -> Int`.

2. **PID Encoding in Wire Messages**
   - What we know: PIDs are u64 with `[16-bit node_id | 8-bit creation | 40-bit local_id]`. A local PID has `node_id=0`. When sent to a remote node, the PID must be meaningful on the receiving side.
   - What's unclear: Should the wire message send the PID as-is, or should it be translated? If node A registers PID-X (node_id=0, local), and broadcasts to node B, node B sees node_id=0 which means "local to B" -- wrong!
   - Recommendation: The registering node must construct the PID with its own node_id before broadcasting. When `snow_global_register` is called on node A, the PID's `node_id` is checked. If it's 0 (local), reconstruct it with the appropriate node_id that other nodes would use for this node. This can be looked up from the session's `node_id` field. Alternatively, include the owning node name in the wire message (which we already do) and let the receiving node use `ProcessId::from_remote(session.node_id, session.creation, pid.local_id())` to reconstruct a valid remote PID. The second approach is cleaner because each node assigns its own node_ids to remote nodes.

3. **Lock Ordering for Three-Way Index Updates**
   - What we know: The GlobalRegistry has three maps (`names`, `pid_names`, `node_names`) that must be updated atomically for consistency.
   - What's unclear: What lock ordering prevents deadlocks?
   - Recommendation: Always acquire locks in the order: `names` -> `pid_names` -> `node_names`. Since all operations follow this order, deadlocks are impossible. Alternatively, use a single lock for all three maps (simpler but more contention). At Snow's target scale, a single `RwLock<GlobalRegistryInner>` wrapping all three maps is the simplest correct approach.

4. **Should `Global.unregister` require ownership?**
   - What we know: Erlang's `global:unregister_name` can be called by any process.
   - What's unclear: Should any process be able to unregister any name, or only the process that registered it (or the node that owns it)?
   - Recommendation: Allow any process to unregister any name (matching Erlang). Ownership enforcement adds complexity without practical benefit (in practice, only the owner or a supervisor unregisters names).

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** (direct file reads):
  - `crates/snow-rt/src/actor/registry.rs` -- Local ProcessRegistry with register/whereis/unregister/cleanup_process, bi-directional name<->pid maps, RwLock<FxHashMap> pattern, OnceLock singleton
  - `crates/snow-rt/src/dist/node.rs` -- NodeState with sessions/node_id_map, DIST_* wire tags (0x10-0x1A), reader_loop_session dispatch, write_msg/read_dist_msg, cleanup_session -> handle_node_disconnect, register_session -> handle_node_connect, send_peer_list broadcast pattern
  - `crates/snow-rt/src/actor/scheduler.rs` -- handle_process_exit (line 607) with local registry cleanup (line 671), link/monitor propagation, terminate callback
  - `crates/snow-rt/src/actor/mod.rs` -- snow_actor_register (line 755), snow_actor_whereis (line 787), snow_actor_send_named (line 364), local_send
  - `crates/snow-rt/src/actor/process.rs` -- ProcessId with node_id()/creation()/local_id()/is_local() accessors, from_remote constructor
  - `crates/snow-rt/src/lib.rs` -- Re-export structure for extern "C" functions
  - `crates/snow-codegen/src/mir/lower.rs` -- STDLIB_MODULES list (line 9274), lower_field_access (line 5789), map_builtin_name
  - `crates/snow-codegen/src/codegen/intrinsics.rs` -- declare_intrinsics pattern, all existing intrinsic declarations
  - `crates/snow-codegen/src/codegen/expr.rs` -- codegen_node_start/codegen_node_string_call patterns for string argument handling
  - `crates/snow-typeck/src/infer.rs` -- build_module_types (line 762+), Node/Process module type definitions
  - `.planning/REQUIREMENTS.md` -- CLUST-01, CLUST-02, CLUST-03 definitions
  - `.planning/ROADMAP.md` -- Phase 68 success criteria and dependencies

### Secondary (MEDIUM confidence)
- [Erlang global module documentation](https://www.erlang.org/doc/apps/kernel/global.html) -- API reference: register_name/2, whereis_name/1, unregister_name/1 semantics; replicated name tables stored locally on every node; automatic cleanup on node disconnect via nodeup/nodedown monitoring; synchronous registration semantics; conflict resolution functions (random_exit_name, etc.)
- [Erlang global module source](https://github.com/erlang/otp/blob/master/lib/kernel/src/global.erl) -- Implementation reference showing gen_server-based architecture
- [Tony's Blog: Erlang/OTP's global module](https://leastfixedpoint.com/tonyg/kcbbs/lshift_archive/erlangotps-global-module-20090213.html) -- Design criticism and analysis of :global's consistency model
- [Syn: Scalable Process Registry](https://github.com/ostinelli/syn) -- Alternative registry design choosing availability over consistency with eventual consistency via CRDTs

### Tertiary (LOW confidence)
- [Erlang EEP-0032](https://www.erlang.org/eeps/eep-0032) -- Proposal for module-local process variables as alternative to global registry; discusses data race concerns with global shared mutable state

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- zero new dependencies; all existing infrastructure reused
- Architecture (data structure): HIGH -- directly mirrors existing ProcessRegistry pattern with node-name index addition
- Architecture (wire protocol): HIGH -- follows exact same DIST_* pattern used by all 11 existing wire message types
- Architecture (sync on connect): HIGH -- follows existing send_peer_list pattern (called at same hook point)
- Architecture (cleanup): HIGH -- hooks into existing handle_node_disconnect and handle_process_exit, both well-understood
- Architecture (compiler integration): HIGH -- follows exact same Global/Node/Process/Timer module integration pattern
- Pitfalls: HIGH -- derived from analyzing actual code paths and distributed systems race conditions; Erlang's known issues as reference
- Open questions: MEDIUM -- PID encoding in wire messages requires careful handling; lock ordering is a design choice with clear recommendation

**Research date:** 2026-02-12
**Valid until:** 2026-03-14 (stable domain; all dependencies frozen)
