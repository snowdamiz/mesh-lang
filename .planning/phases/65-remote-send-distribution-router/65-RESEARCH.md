# Phase 65: Remote Send & Distribution Router - Research

**Researched:** 2026-02-12
**Domain:** Distributed message routing, mesh formation, and remote process name resolution
**Confidence:** HIGH

## Summary

Phase 65 bridges the gap between "nodes can connect" (Phase 64) and "actors can communicate across nodes." The phase has five concrete deliverables: (1) replace the `dist_send_stub` with actual remote message routing via NodeSession's TLS stream, using the STF wire format from Phase 63; (2) implement `send({name, node}, msg)` for sending to named processes on remote nodes; (3) preserve per-sender-receiver message ordering (inherently guaranteed by TCP's stream semantics, but must be preserved through the internal routing path); (4) automatic mesh formation when connecting node A to node B that is already connected to node C; and (5) `Node.list()` and `Node.self()` query APIs.

The critical architectural insight is that the existing infrastructure is remarkably complete. Phase 63 delivered the STF encoder/decoder (`wire.rs`) with full Snow type coverage. Phase 64 delivered `NodeState` with `sessions: RwLock<FxHashMap<String, Arc<NodeSession>>>` and `node_id_map: RwLock<FxHashMap<u16, String>>`, the `reader_loop_session` with a `_ => { /* Phase 65 will handle actual message routing */ }` branch, and `write_msg`/`read_msg` length-prefixed wire helpers. The send path already has the locality check (`target_pid >> 48 == 0`) that routes to `dist_send_stub` for non-local PIDs. This phase is primarily a **wiring exercise** -- connecting the STF encoder to the send path, decoding incoming messages in the reader thread, and adding mesh/query functionality.

**Primary recommendation:** Structure as three plans: (1) Distribution message protocol and `dist_send` implementation (replaces `dist_send_stub`, defines wire message tags, sends via `NodeSession.stream`, and handles incoming messages in `reader_loop_session`); (2) Named remote send (`send({name, node}, msg)`) with `REG_SEND` wire message, plus `Node.list()` and `Node.self()` extern "C" APIs; (3) Automatic mesh formation via peer list exchange during handshake, plus integration tests covering multi-node messaging scenarios.

## Standard Stack

### Core (already in Cargo.toml -- zero new dependencies)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| rustls | 0.23.36 | TLS streams for inter-node communication | Already used in Phase 64 node connections |
| parking_lot | 0.12 | RwLock for sessions/node_id_map concurrent access | Already used throughout codebase |
| rustc-hash | 2 | FxHashMap for fast lookups | Already used in node state and scheduler |
| rand | 0.9 | Random data for tests | Already used in handshake challenges |

### Supporting (from existing crate modules)
| Module | Purpose | When to Use |
|--------|---------|-------------|
| `dist::wire` (STF) | `stf_encode_value` / `stf_decode_value` | Encoding/decoding message payloads for inter-node transport |
| `dist::node` | `NodeState`, `NodeSession`, `write_msg`, `read_msg` | Sending/receiving length-prefixed messages on TLS streams |
| `actor::process` | `ProcessId`, `ProcessId::from_remote` | Constructing/deconstructing remote PIDs |
| `actor::registry` | `global_registry().whereis()` | Resolving named process sends on receiving node |
| `actor::mod` | `local_send()`, `copy_msg_to_actor_heap()` | Delivering decoded remote messages to local actor mailboxes |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Inline STF encode in send path | Separate serialization layer | Inline is simpler; STF is already self-contained |
| Channel-based message queue per session | Direct stream write with mutex | Mutex is already the pattern (Phase 64); channel adds complexity without benefit at this scale |
| Gossip protocol for mesh | Peer list exchange on connect | Peer list is simpler and deterministic; gossip is for large clusters (>20 nodes, out of scope) |

**Installation:** No new dependencies. All work is within existing `snow-rt/src/dist/` and `snow-rt/src/actor/`.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-rt/src/
├── dist/
│   ├── mod.rs           # MODIFIED: re-export new message types
│   ├── wire.rs          # EXISTING: STF encoder/decoder (Phase 63, unchanged)
│   └── node.rs          # MODIFIED: dist_send, reader routing, mesh, Node.list/self
├── actor/
│   ├── mod.rs           # MODIFIED: replace dist_send_stub, add extern "C" APIs
│   └── registry.rs      # EXISTING: used for named remote sends (unchanged)
└── lib.rs               # MODIFIED: re-export new extern "C" functions
```

### Pattern 1: Distribution Message Wire Format
**What:** Define message tags for the distribution protocol layer on top of the existing length-prefixed `write_msg`/`read_msg` framing. Each distribution message starts with a 1-byte tag identifying the operation, followed by operation-specific fields, followed by the STF-encoded payload.
**When to use:** All inter-node message passing.
**Rationale:** Mirrors Erlang's SEND/REG_SEND/PEER_LIST control messages. The existing `HEARTBEAT_PING` (0xF0) and `HEARTBEAT_PONG` (0xF1) already use this single-tag pattern in the reader loop.

```rust
// Distribution message tags (occupy the 0x10-0x1F range, heartbeat is 0xF0-0xF1)
const DIST_SEND: u8 = 0x10;       // Send to PID: [tag][u64 target_pid][STF payload]
const DIST_REG_SEND: u8 = 0x11;   // Send to name: [tag][u16 name_len][name][STF payload]
const DIST_PEER_LIST: u8 = 0x12;  // Mesh formation: [tag][u16 count][name1...nameN]
```

### Pattern 2: Remote Send Path (replacing dist_send_stub)
**What:** The `dist_send_stub` in `actor/mod.rs` is replaced with a real `dist_send` function that: (1) extracts `node_id` from the PID's upper 16 bits, (2) looks up the node name in `NODE_STATE.node_id_map`, (3) looks up the `NodeSession` in `NODE_STATE.sessions`, (4) constructs a `DIST_SEND` wire message with the STF-encoded payload, and (5) writes it to the session's TLS stream.
**When to use:** Automatically invoked when `snow_actor_send` receives a PID with `node_id != 0`.
**Rationale:** Maintains the existing locality check (`target_pid >> 48 == 0`) that is already branch-free. The remote path is `#[cold]` since most sends are local.

```rust
// Replaces dist_send_stub in actor/mod.rs
#[cold]
fn dist_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) {
    let node_id = (target_pid >> 48) as u16;
    let state = match crate::dist::node::node_state() {
        Some(s) => s,
        None => return, // Node not started; silently drop
    };

    // Look up which node this PID belongs to
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return, // Unknown node; silently drop
        }
    };

    // Look up the session for that node
    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => Arc::clone(s),
            None => return, // Not connected; silently drop
        }
    };

    // Build wire message: [DIST_SEND][u64 target_pid][raw msg bytes]
    let mut payload = Vec::with_capacity(1 + 8 + msg_size as usize);
    payload.push(DIST_SEND);
    payload.extend_from_slice(&target_pid.to_le_bytes());
    if !msg_ptr.is_null() && msg_size > 0 {
        let slice = unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) };
        payload.extend_from_slice(slice);
    }

    // Write to TLS stream (mutex protects concurrent writes)
    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
}
```

### Pattern 3: Reader Thread Message Delivery
**What:** The `reader_loop_session` currently has a `_ => {}` catch-all for non-heartbeat messages. Phase 65 adds handling for `DIST_SEND` and `DIST_REG_SEND`: decode the target PID or name, then call `local_send()` (or the registry + local_send path) to deliver to the local actor's mailbox.
**When to use:** When the reader thread receives a non-heartbeat message.
**Rationale:** Message delivery happens on the reader thread (dedicated OS thread per connection), which is safe because `local_send` only pushes to a mailbox and wakes the target -- no blocking operations.

```rust
// In reader_loop_session, replace the _ => {} branch
DIST_SEND => {
    if msg.len() >= 9 { // 1 tag + 8 pid
        let target_pid = u64::from_le_bytes(msg[1..9].try_into().unwrap());
        let msg_data = &msg[9..]; // raw message bytes (same format as local send)
        // Deliver to local actor mailbox
        crate::actor::local_send(target_pid, msg_data.as_ptr(), msg_data.len() as u64);
    }
}
DIST_REG_SEND => {
    if msg.len() >= 3 { // 1 tag + 2 name_len minimum
        let name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        if msg.len() >= 3 + name_len {
            if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                if let Some(pid) = crate::actor::registry::global_registry().whereis(name) {
                    let msg_data = &msg[3 + name_len..];
                    crate::actor::local_send(pid.as_u64(), msg_data.as_ptr(), msg_data.len() as u64);
                }
            }
        }
    }
}
```

### Pattern 4: Automatic Mesh Formation
**What:** After a connection is authenticated and registered, the newly connected node sends a `DIST_PEER_LIST` message containing the names of all its other connected peers. The receiving node iterates through the list and initiates connections to any nodes it is not already connected to.
**When to use:** Immediately after `register_session` + `spawn_session_threads` completes (in both `accept_loop` and `snow_node_connect`).
**Rationale:** This is how Erlang achieves "connections are by default transitive" -- when node A connects to node B, B tells A about C, and A connects to C. The peer list approach is simpler than gossip for the target cluster size (3-20 nodes).

```rust
// After spawn_session_threads in both accept_loop and snow_node_connect:
send_peer_list(&session); // Tell new peer about our other connections

// In reader_loop_session, handle DIST_PEER_LIST:
DIST_PEER_LIST => {
    if msg.len() >= 3 {
        let count = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        let mut pos = 3;
        for _ in 0..count {
            if pos + 2 > msg.len() { break; }
            let name_len = u16::from_le_bytes(msg[pos..pos+2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + name_len > msg.len() { break; }
            if let Ok(peer_name) = std::str::from_utf8(&msg[pos..pos+name_len]) {
                // Connect to this peer if not already connected and not ourselves
                maybe_connect_to_peer(peer_name);
            }
            pos += name_len;
        }
    }
}
```

### Pattern 5: Message Size Limit Upgrade
**What:** The current `read_msg` enforces `MAX_HANDSHAKE_MSG = 4096` bytes. This is appropriate for handshake messages but too small for actor messages. Phase 65 needs to either (a) use a separate `read_dist_msg` function with a larger limit (e.g., 16 MB, matching STF's `MAX_STRING_LEN`), or (b) increase the limit and add per-tag validation.
**When to use:** After handshake completes, the reader loop needs to accept larger messages.
**Rationale:** Actor messages can be significantly larger than 4KB (collections, structs with string fields). The limit should match STF's built-in safety limits.

```rust
// Option A (recommended): Separate function for post-handshake reads
const MAX_DIST_MSG: u32 = 16 * 1024 * 1024; // 16 MB, matches STF MAX_STRING_LEN

fn read_dist_msg(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_DIST_MSG {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("dist message too large: {} bytes (max {})", len, MAX_DIST_MSG)));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}
```

### Anti-Patterns to Avoid
- **STF-encoding in the send path for PID-addressed sends:** The current `snow_actor_send` receives raw message bytes (`msg_ptr`, `msg_size`). Do NOT attempt to STF-encode/decode in the send/receive hot path. The raw bytes are already the message payload (the actor serialization format is already handled by codegen). The remote send just needs to forward these raw bytes.
- **Blocking on stream write from the actor coroutine:** The `dist_send` function writes to the TLS stream, which acquires a mutex. Keep this operation as fast as possible. If the stream is slow, the sending actor briefly blocks, which is acceptable (same as Erlang's behavior -- distribution is synchronous on the sending side).
- **Connecting to ourselves during mesh formation:** When processing a peer list, always check that the peer name is not our own name and that we don't already have a session.
- **Using async/tokio for distribution:** Snow uses blocking I/O everywhere. Adding async for distribution would fight the M:N scheduler architecture.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Message serialization | Custom binary format | Forward raw message bytes from snow_actor_send | Bytes are already serialized by codegen/STF; avoid double serialization |
| Named process lookup | Custom remote registry | Local `ProcessRegistry::whereis()` on receiving node | The receiving node owns the registry; sender just passes the name |
| TLS stream I/O | Custom framing | Existing `write_msg`/`read_msg` length-prefixed protocol | Already proven by handshake and heartbeat |
| Per-sender ordering | Sequence numbers | TCP stream ordering guarantee | TCP guarantees in-order delivery on a single connection |
| Node identity lookups | Linear scan of sessions | `node_id_map: FxHashMap<u16, String>` reverse lookup | Already built in Phase 64 specifically for this purpose |

**Key insight:** The hardest parts (STF encoding, TLS connections, PID bit-packing, process registry) are already built. Phase 65 is connecting existing infrastructure, not building new primitives.

## Common Pitfalls

### Pitfall 1: read_msg 4KB Limit Rejects Actor Messages
**What goes wrong:** The current `read_msg` enforces `MAX_HANDSHAKE_MSG = 4096` bytes. Any actor message larger than 4KB is rejected with "handshake message too large."
**Why it happens:** The limit was appropriate for handshake-only Phase 64 but the reader loop now needs to handle arbitrary-size actor messages.
**How to avoid:** Create a `read_dist_msg` function with a larger limit (16 MB) and use it in the reader loop after handshake. Keep `read_msg` with its 4KB limit for handshake-only code paths.
**Warning signs:** "handshake message too large" errors when sending messages with collections or large strings.

### Pitfall 2: Mutex Contention on Stream During Concurrent Sends
**What goes wrong:** Multiple actors on the same node sending to processes on the same remote node all contend on `session.stream.lock()`. Under high load, this serializes sends.
**Why it happens:** The TLS stream is a single resource shared by all senders to that remote node.
**How to avoid:** Accept this as inherent to the design (Erlang has the same behavior -- one TCP connection per node pair). The mutex hold time is short (write a length-prefixed message to a buffered stream). For future optimization (OPT-02), message fragmentation could interleave large messages.
**Warning signs:** High contention metrics; can be observed in tests with many concurrent senders. Acceptable for the target cluster size (3-20 nodes).

### Pitfall 3: Dead Node During Send (Stream Write Failure)
**What goes wrong:** A node disconnects between the time `dist_send` looks up the session and the time it writes to the stream. The `write_msg` call fails with an I/O error.
**Why it happens:** Connection lifecycle is asynchronous. The heartbeat thread hasn't detected the dead connection yet.
**How to avoid:** Catch the `write_msg` error in `dist_send` and silently drop the message. Do NOT panic or propagate the error to the sending actor. Phase 66 will add proper `:nodedown` notifications. For now, message loss on disconnect is acceptable and matches Erlang's "at most once" delivery semantics.
**Warning signs:** Panics or error logs during node disconnect scenarios.

### Pitfall 4: Infinite Mesh Loop
**What goes wrong:** Node A connects to B, B sends peer list [C] to A, A connects to C, C sends peer list [B] to A, A tries to connect to B again.
**Why it happens:** Peer list exchange triggers connection attempts without checking existing connections.
**How to avoid:** Before connecting to a peer from a peer list, check: (1) the peer name is not our own name, (2) we don't already have a session to this peer, (3) we are not already in the process of connecting to this peer. Use a simple "already connected" check against `sessions.read()`.
**Warning signs:** Connection storms, duplicate session errors in logs, or high CPU from rapid connect/reject cycles.

### Pitfall 5: local_send Visibility from dist Module
**What goes wrong:** `local_send` in `actor/mod.rs` is a private function (`fn local_send`). The dist module's reader thread needs to call it to deliver messages.
**Why it happens:** Phase 63 extracted `local_send` as a private helper; it was never needed from outside the module.
**How to avoid:** Make `local_send` `pub(crate)` so `dist::node::reader_loop_session` can call it. This is a minor visibility change, not an architectural change.
**Warning signs:** Compile error "function is private."

### Pitfall 6: Named Send ({name, node}) Requires New Extern "C" API
**What goes wrong:** The current `snow_actor_send` takes a `target_pid: u64`. There is no API for sending to a `{name, node}` tuple.
**Why it happens:** Named remote sends are a new concept not present in the local-only actor system.
**How to avoid:** Create a new `snow_actor_send_named` extern "C" function that takes `(name_ptr, name_len, node_ptr, node_len, msg_ptr, msg_size)`. The codegen layer needs to emit calls to this function when the send target is a `{name, node}` tuple rather than a PID.
**Warning signs:** No way for Snow programs to use `send({name, node}, msg)` syntax without this API.

### Pitfall 7: Mesh Formation Spawns Threads That May Deadlock
**What goes wrong:** `send_peer_list` is called from the accept loop or the `snow_node_connect` flow. If `maybe_connect_to_peer` calls `snow_node_connect` synchronously, it could deadlock if it tries to acquire locks held by the current path.
**Why it happens:** Nested connection attempts while processing the first connection.
**How to avoid:** Spawn mesh connection attempts on a separate thread (fire-and-forget). Do NOT connect synchronously from within the reader loop or accept loop. Collect the peer names, then spawn a thread to connect to them.
**Warning signs:** Deadlock during multi-node connection tests.

## Code Examples

### Remote Send (Replacing dist_send_stub)
```rust
// Source: Architecture design based on existing codebase patterns
// File: crates/snow-rt/src/actor/mod.rs (modifying dist_send_stub)

use crate::dist::node::{node_state, write_msg, DIST_SEND};
use std::sync::Arc;

/// Send a message to a remote actor via its NodeSession's TLS stream.
///
/// Extracts the node_id from the PID, looks up the session, constructs a
/// DIST_SEND wire message, and writes it to the TLS stream.
///
/// Silently drops on any failure (node not started, unknown node_id,
/// disconnected session, write error). Phase 66 will add proper error
/// handling with :nodedown signals.
#[cold]
fn dist_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) {
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    let node_id = (target_pid >> 48) as u16;
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return,
        }
    };

    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => Arc::clone(s),
            None => return,
        }
    };

    // Build wire message: [DIST_SEND][u64 target_pid][message bytes]
    let mut payload = Vec::with_capacity(1 + 8 + msg_size as usize);
    payload.push(DIST_SEND);
    payload.extend_from_slice(&target_pid.to_le_bytes());
    if !msg_ptr.is_null() && msg_size > 0 {
        let slice = unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) };
        payload.extend_from_slice(slice);
    }

    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
}
```

### Reader Loop Message Dispatch
```rust
// Source: Extension of existing reader_loop_session in dist/node.rs
// In the match msg[0] { ... } block:

DIST_SEND => {
    // Wire format: [tag=0x10][u64 target_pid][raw message bytes]
    if msg.len() >= 9 {
        let target_pid = u64::from_le_bytes(msg[1..9].try_into().unwrap());
        let msg_data = &msg[9..];
        // Deliver to local actor using existing local_send
        crate::actor::local_send(target_pid, msg_data.as_ptr(), msg_data.len() as u64);
    }
}
DIST_REG_SEND => {
    // Wire format: [tag=0x11][u16 name_len][name bytes][raw message bytes]
    if msg.len() >= 3 {
        let name_len = u16::from_le_bytes(msg[1..3].try_into().unwrap()) as usize;
        if msg.len() >= 3 + name_len {
            if let Ok(name) = std::str::from_utf8(&msg[3..3 + name_len]) {
                if let Some(pid) = crate::actor::registry::global_registry().whereis(name) {
                    let msg_data = &msg[3 + name_len..];
                    crate::actor::local_send(
                        pid.as_u64(),
                        msg_data.as_ptr(),
                        msg_data.len() as u64,
                    );
                }
                // If name not registered, silently drop (matches Erlang behavior)
            }
        }
    }
}
DIST_PEER_LIST => {
    // Wire format: [tag=0x12][u16 count][u16 name_len, name bytes, ...]
    handle_peer_list(&msg[1..]);
}
```

### Named Remote Send API
```rust
// Source: New extern "C" API in actor/mod.rs
/// Send a message to a named process on a remote node.
///
/// Called from compiled Snow code for `send({name, node}, msg)` syntax.
///
/// - `name_ptr`, `name_len`: UTF-8 process name
/// - `node_ptr`, `node_len`: UTF-8 node name (e.g. "worker@host:9000")
/// - `msg_ptr`, `msg_size`: raw message bytes
#[no_mangle]
pub extern "C" fn snow_actor_send_named(
    name_ptr: *const u8, name_len: u64,
    node_ptr: *const u8, node_len: u64,
    msg_ptr: *const u8, msg_size: u64,
) {
    let name = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let node = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(node_ptr, node_len as usize))
    };

    let (name, node) = match (name, node) {
        (Ok(n), Ok(nd)) => (n, nd),
        _ => return,
    };

    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    // If the target node is ourselves, do a local registry lookup + send
    if node == state.name {
        if let Some(pid) = global_registry().whereis(name) {
            local_send(pid.as_u64(), msg_ptr, msg_size);
        }
        return;
    }

    // Look up the remote session
    let session = {
        let sessions = state.sessions.read();
        match sessions.get(node) {
            Some(s) => Arc::clone(s),
            None => return,
        }
    };

    // Build DIST_REG_SEND message: [tag][u16 name_len][name][msg bytes]
    let name_bytes = name.as_bytes();
    let mut payload = Vec::with_capacity(1 + 2 + name_bytes.len() + msg_size as usize);
    payload.push(DIST_REG_SEND);
    payload.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(name_bytes);
    if !msg_ptr.is_null() && msg_size > 0 {
        let slice = unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) };
        payload.extend_from_slice(slice);
    }

    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
}
```

### Node.list() and Node.self()
```rust
// Source: New extern "C" APIs in dist/node.rs

/// Return the current node's name as a SnowString pointer.
/// Returns null if node is not started.
#[no_mangle]
pub extern "C" fn snow_node_self() -> *const u8 {
    match node_state() {
        Some(state) => {
            crate::string::snow_string_new(
                state.name.as_ptr(),
                state.name.len() as u64,
            )
        }
        None => std::ptr::null(),
    }
}

/// Return a list of connected node names as a Snow list of strings.
/// Returns an empty list if node is not started or no connections.
#[no_mangle]
pub extern "C" fn snow_node_list() -> *const u8 {
    let state = match node_state() {
        Some(s) => s,
        None => {
            // Return empty list
            return alloc_empty_list();
        }
    };

    let sessions = state.sessions.read();
    let names: Vec<String> = sessions.keys().cloned().collect();
    drop(sessions);

    // Build a Snow list of SnowString pointers
    build_string_list(&names)
}
```

### Mesh Formation (Peer List Exchange)
```rust
// Source: Architecture design based on Erlang's transitive connection behavior

/// Send our current peer list to a newly connected node.
fn send_peer_list(session: &Arc<NodeSession>) {
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    let sessions = state.sessions.read();
    let peers: Vec<&String> = sessions.keys()
        .filter(|name| *name != &session.remote_name)
        .collect();

    if peers.is_empty() {
        return;
    }

    let mut payload = Vec::new();
    payload.push(DIST_PEER_LIST);
    payload.extend_from_slice(&(peers.len() as u16).to_le_bytes());
    for peer_name in &peers {
        let bytes = peer_name.as_bytes();
        payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        payload.extend_from_slice(bytes);
    }

    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
}

/// Handle an incoming peer list -- connect to unknown peers.
fn handle_peer_list(data: &[u8]) {
    if data.len() < 2 { return; }
    let count = u16::from_le_bytes(data[0..2].try_into().unwrap()) as usize;
    let mut pos = 2;
    let mut to_connect = Vec::new();

    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    for _ in 0..count {
        if pos + 2 > data.len() { break; }
        let name_len = u16::from_le_bytes(data[pos..pos+2].try_into().unwrap()) as usize;
        pos += 2;
        if pos + name_len > data.len() { break; }
        if let Ok(peer_name) = std::str::from_utf8(&data[pos..pos+name_len]) {
            // Skip self and already-connected nodes
            if peer_name != state.name {
                let sessions = state.sessions.read();
                if !sessions.contains_key(peer_name) {
                    to_connect.push(peer_name.to_string());
                }
            }
        }
        pos += name_len;
    }

    // Spawn connection attempts on a separate thread to avoid deadlock
    if !to_connect.is_empty() {
        std::thread::spawn(move || {
            for peer in to_connect {
                let bytes = peer.as_bytes();
                snow_node_connect(bytes.as_ptr(), bytes.len() as u64);
            }
        });
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `dist_send_stub` silently drops | `dist_send` routes via TLS stream | Phase 65 | Remote actors now reachable |
| No named remote send | `snow_actor_send_named` with REG_SEND | Phase 65 | `send({name, node}, msg)` works |
| Manual mesh: connect A->B, connect A->C | Automatic peer list exchange | Phase 65 | Connecting A->B auto-discovers C |
| `read_msg` 4KB limit | `read_dist_msg` 16MB limit for post-handshake | Phase 65 | Large actor messages supported |

**Deprecated/outdated:**
- `dist_send_stub`: replaced by real `dist_send` (this phase)
- The `_ => {}` catch-all in `reader_loop_session`: replaced by DIST_SEND/REG_SEND/PEER_LIST handlers

## Open Questions

1. **Raw Bytes vs STF Encoding for Remote Send**
   - What we know: `snow_actor_send` currently receives `(target_pid, msg_ptr, msg_size)` where `msg_ptr` points to raw message bytes that the codegen layer already serialized. For local sends, these bytes go directly into the mailbox's `MessageBuffer`.
   - What's unclear: Should the remote send path forward these raw bytes as-is (transparent relay), or should it STF-encode them (adding structure)? The current codegen already builds message buffers with `[tag][field_values...]` layout.
   - Recommendation: Forward raw bytes as-is. The message bytes from `snow_actor_send` are already the complete message payload. The remote side delivers them to the target actor's mailbox verbatim, exactly as if `local_send` had been called. This avoids double serialization and maintains the property that remote send is transparent. STF encoding would only be needed if the message contains pointers that need to be resolved (which they do for strings and collections), but the current actor message format already handles this via the codegen layer.
   - **CRITICAL CAVEAT:** If message payloads contain raw pointers to heap-allocated objects (strings, lists, maps), those pointers are NOT valid on the remote node. This would require STF encoding for the message body. Need to verify whether codegen serializes messages as self-contained byte buffers or as pointer-containing structures. If the latter, STF encode/decode is mandatory.

2. **Codegen Changes for Named Send**
   - What we know: `snow_actor_send_named` is a new extern "C" function. The codegen layer needs to emit calls to it when the send target is `{name, node}`.
   - What's unclear: Does the Snow language parser already support `{name, node}` as a send target? Does the type checker handle it?
   - Recommendation: If the language doesn't yet parse `send({name, node}, msg)`, defer the codegen/parser changes to a follow-up or include as a stretch goal. The runtime API can be ready before the language surface supports it.

3. **Node.list() Return Type**
   - What we know: `Node.list()` should return a list of node names. The Snow list type uses a `{len, cap, data}` heap layout.
   - What's unclear: Should it return `List<String>` (Snow strings) or `List<Atom>` (if Snow has atoms)? Does Snow have atoms?
   - Recommendation: Return a Snow list of Snow strings. Snow uses strings where Erlang uses atoms. Build the list using `snow_gc_alloc_actor` for the list header + `snow_string_new` for each name.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** (direct file reads):
  - `crates/snow-rt/src/dist/node.rs` -- NodeState, NodeSession, reader_loop_session, write_msg/read_msg, MAX_HANDSHAKE_MSG, node_state(), sessions, node_id_map
  - `crates/snow-rt/src/dist/wire.rs` -- STF encoder/decoder, stf_encode_value/stf_decode_value, all type tags
  - `crates/snow-rt/src/actor/mod.rs` -- snow_actor_send with locality check, dist_send_stub, local_send, copy_msg_to_actor_heap
  - `crates/snow-rt/src/actor/process.rs` -- ProcessId with node_id/creation/local_id bit-packing, from_remote constructor
  - `crates/snow-rt/src/actor/registry.rs` -- ProcessRegistry::whereis, global_registry
  - `crates/snow-rt/src/lib.rs` -- Current extern "C" re-exports
  - `.planning/phases/64-node-connection-authentication/64-RESEARCH.md` -- Phase 64 research and patterns
  - `.planning/phases/64-node-connection-authentication/64-VERIFICATION.md` -- Phase 64 completion confirmation
  - `.planning/REQUIREMENTS.md` -- MSG-02, MSG-06, MSG-07, NODE-06, NODE-07 definitions
  - `.planning/ROADMAP.md` -- Phase 65 success criteria

### Secondary (MEDIUM confidence)
- [Erlang Distribution Protocol](https://www.erlang.org/doc/apps/erts/erl_dist_protocol.html) -- SEND/REG_SEND message format, wire protocol design reference
- [Erlang Distributed Reference Manual](https://www.erlang.org/doc/reference_manual/distributed.html) -- "Connections are by default transitive" (mesh formation)

### Tertiary (LOW confidence)
- [Scaling Erlang Distribution](https://dl.acm.org/doi/10.1145/3331542.3342572) -- Research on mesh scaling limits; confirms O(N^2) mesh is practical for small clusters

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- zero new dependencies; all existing infrastructure reused
- Architecture (remote send): HIGH -- direct extension of existing dist_send_stub + reader_loop patterns
- Architecture (named send): HIGH -- straightforward registry lookup on receiving node
- Architecture (mesh formation): HIGH -- peer list exchange is a well-understood pattern (Erlang does it)
- Pitfalls: HIGH -- derived from analyzing actual code paths and distributed systems principles
- Open question (raw bytes vs STF): MEDIUM -- needs codegen investigation to determine if messages contain raw pointers

**Research date:** 2026-02-12
**Valid until:** 2026-03-14 (stable domain; all dependencies frozen)
