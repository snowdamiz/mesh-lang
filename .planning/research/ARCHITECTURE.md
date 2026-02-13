# Architecture Patterns: Distributed Actors for Snow

**Domain:** BEAM-style distributed actor system for Snow programming language
**Researched:** 2026-02-12
**Overall confidence:** HIGH (based on direct codebase analysis + BEAM protocol documentation)

## Executive Summary

Snow's existing actor runtime in `snow-rt` provides single-node M:N scheduling with ProcessId (sequential u64), per-actor mailboxes, process registry, links/monitors, and exit signal propagation. The distribution layer must make `send`, `link`, `spawn`, and `monitor` location-transparent without restructuring the single-node fast path.

The architecture inserts a **distribution layer** between the public `snow_actor_*` API functions and the local scheduler, following the same pattern BEAM uses: a PID encodes its origin node, and the runtime checks locality on every operation. Local operations proceed at current speed. Remote operations are routed through a per-node TCP session actor that serializes messages onto the wire.

## Recommended Architecture

### Layer Diagram

```
Snow-compiled program
    |
    v
snow_actor_send(pid, msg_ptr, msg_size)    <-- existing extern "C" ABI
    |
    v
+-------------------------------------------+
|          Distribution Router               |  NEW (in snow-rt)
|  if is_local(pid) -> local_send()          |
|  if is_remote(pid) -> remote_send(pid,msg) |
+-------------------------------------------+
    |                           |
    v                           v
+------------------+   +-------------------+
| Local Scheduler  |   | NodeSession actor  |  NEW (in snow-rt)
| (existing)       |   | per remote node    |
| Mailbox.push()   |   | serializes msg     |
| wake if Waiting  |   | writes to TCP/TLS  |
+------------------+   +-------------------+
                                |
                                v
                        +-------------------+
                        | Wire Protocol     |  NEW (in snow-rt)
                        | Binary encoding   |
                        | of Snow values    |
                        +-------------------+
                                |
                                v
                          TCP / TLS stream
```

### Component Boundaries

| Component | Responsibility | Location | Communicates With |
|-----------|---------------|----------|-------------------|
| **DistributionRouter** | Check PID locality, route to local or remote path | `snow-rt/src/dist/router.rs` | `snow_actor_*` API, NodeSession, Scheduler |
| **NodeId** | Unique node identity (name + creation) | `snow-rt/src/dist/node.rs` | Everything in dist module |
| **DistributedPid** | Extended PID encoding with node information | `snow-rt/src/actor/process.rs` (modified) | Router, wire format, all PID consumers |
| **NodeSession** | Per-connection actor managing one remote node link | `snow-rt/src/dist/session.rs` | Router, Wire codec, TCP stream |
| **NodeRegistry** | Maps node names to NodeSession actors | `snow-rt/src/dist/registry.rs` | Router, NodeSession, EPMD-equivalent |
| **WireCodec** | Binary serialization of messages, PIDs, signals | `snow-rt/src/dist/wire.rs` | NodeSession, MessageBuffer |
| **Handshake** | Challenge/response authentication between nodes | `snow-rt/src/dist/handshake.rs` | NodeSession, TLS stream |
| **NodeDiscovery** | Node name resolution (EPMD-equivalent) | `snow-rt/src/dist/discovery.rs` | NodeSession startup, CLI |

## PID Representation: The Critical Design Decision

### Current State

```rust
// snow-rt/src/actor/process.rs
pub struct ProcessId(pub u64);

impl ProcessId {
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        ProcessId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}
```

The current PID is a plain sequential `u64` from a global atomic counter. The LLVM intrinsics pass PIDs as `i64`. The Snow type system exposes PIDs as `Pid<M>` which at the LLVM level is also `i64`. Every call to `snow_actor_send`, `snow_actor_link`, `snow_actor_self`, `snow_actor_spawn` uses `u64` PIDs. The `ProcessTable` is `FxHashMap<ProcessId, Arc<Mutex<Process>>>`.

### Design: Encode Node ID Into the u64

**Recommended approach:** Partition the u64 PID space to embed a node identifier.

```
Bit layout (64 bits total):
[  16-bit node_id  |  48-bit local_id  ]
 bits 63..48           bits 47..0
```

- **node_id = 0** means "this node" (local). All existing PIDs are local since they start from 0 and increment, so the high 16 bits are already zero for any PID < 2^48 (~281 trillion). This means **existing code does not need to change** -- current PIDs naturally encode as local.
- **node_id > 0** means the PID belongs to a remote node. The 16-bit field supports 65,535 remote nodes.
- **48-bit local_id** supports ~281 trillion processes per node, far more than any realistic workload.

This preserves the `u64` ABI -- all LLVM intrinsic signatures remain identical. The `ProcessId(u64)` struct keeps its `#[derive(Clone, Copy, PartialEq, Eq, Hash)]`. No codegen changes required.

```rust
// New methods on ProcessId (backward compatible):
impl ProcessId {
    pub fn node_id(self) -> u16 {
        (self.0 >> 48) as u16
    }

    pub fn local_id(self) -> u64 {
        self.0 & 0x0000_FFFF_FFFF_FFFF
    }

    pub fn is_local(self) -> bool {
        self.node_id() == 0
    }

    pub fn from_remote(node_id: u16, local_id: u64) -> Self {
        ProcessId((node_id as u64) << 48 | (local_id & 0x0000_FFFF_FFFF_FFFF))
    }

    /// Next local PID (backward compatible -- high 16 bits stay 0)
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        ProcessId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}
```

### Impact Analysis

| Existing code path | Impact | Change needed |
|--------------------|--------|---------------|
| `ProcessId::next()` | NONE | Counter stays < 2^48, high bits are 0 |
| `snow_actor_spawn` returns `u64` | NONE | Local spawns return node_id=0 PIDs |
| `snow_actor_send(target_pid, ...)` | MINOR | Add locality check at entry point |
| `snow_actor_self()` returns `u64` | NONE | Returns local PID as before |
| `snow_actor_link(target_pid)` | MINOR | Add locality check, remote link protocol |
| `snow_actor_receive(timeout)` | NONE | Mailbox is local, remote messages arrive via NodeSession push |
| `ProcessTable` HashMap lookup | NONE | Remote PIDs are not in local table; router handles before lookup |
| `snow_actor_register(name)` | NONE | Names are local to the node |
| `snow_actor_whereis(name)` | POTENTIAL | Could extend to `{name, node}` tuples later |
| Exit signal propagation (`link.rs`) | MINOR | Must handle remote linked PIDs |
| Supervisor (`supervisor.rs`) | NONE | Supervisors are node-local (same as BEAM) |
| WebSocket server (`ws/server.rs`) | NONE | WebSocket connections are node-local |
| Reserved type tags (`u64::MAX-N`) | CAUTION | Must reserve tags for dist signals |
| `Display for ProcessId` (`<0.N>`) | MINOR | Change to `<node_id.N>` format |
| LLVM intrinsics (`intrinsics.rs`) | NONE | PID is still i64, no signature changes |

### Alternative Considered: 128-bit PID

A 128-bit PID would provide separate 64-bit fields for node and local process. However, this would require changing every LLVM intrinsic signature, every codegen call site, the Snow type system representation of `Pid<M>`, and every internal API. The churn would be enormous for negligible practical benefit (65K nodes and 281T processes is sufficient). **Rejected.**

### Alternative Considered: Indirection Table

Map opaque local u64 handles to `(node_id, remote_pid)` pairs via a side table. This avoids changing PID representation but adds a hash lookup on every send for every PID type (local and remote). **Rejected** -- locality check on the high 16 bits is a single shift+compare, much cheaper than a hash lookup.

## Distribution Router: Where Remote Send Integrates

The key integration point is `snow_actor_send`. Currently:

```rust
pub extern "C" fn snow_actor_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) {
    let sched = global_scheduler();
    let pid = ProcessId(target_pid);
    // ... deep copy, push to mailbox, wake if Waiting
}
```

The modified version adds a locality check:

```rust
pub extern "C" fn snow_actor_send(target_pid: u64, msg_ptr: *const u8, msg_size: u64) {
    let pid = ProcessId(target_pid);

    if pid.is_local() {
        // Fast path: existing local send (unchanged)
        local_send(pid, msg_ptr, msg_size);
    } else {
        // Distribution path: route through NodeSession
        dist::router::remote_send(pid, msg_ptr, msg_size);
    }
}
```

The `is_local()` check is a single bitwise operation (`pid >> 48 == 0`), adding negligible overhead to the local path. The critical property: **existing single-node programs see zero performance impact**.

### Remote Send Flow

```
1. snow_actor_send(remote_pid, msg_ptr, msg_size)
2. pid.is_local() == false
3. dist::router::remote_send(pid, msg_ptr, msg_size)
4. Router looks up NodeSession for pid.node_id()
5. NodeSession actor serializes: [SEND | target_local_id | type_tag | msg_bytes]
6. NodeSession writes framed message to TCP/TLS stream
7. Remote node's NodeSession reads frame
8. Remote node looks up local ProcessId(msg.target_local_id)
9. Remote node does local_send(target, deserialized_msg)
```

### Remote Link Flow

Links between nodes require a protocol exchange (not just mailbox push):

```
1. snow_actor_link(remote_pid)
2. Router sends LINK signal to remote NodeSession
3. Remote node creates bidirectional link entry:
   - Remote process.links adds DistributedPid(our_node, our_pid)
   - Local process.links adds remote_pid
4. When either process exits, exit signal traverses the wire
```

This follows the BEAM model exactly: link.rs `propagate_exit` already iterates `HashSet<ProcessId>`. For remote PIDs in that set, the exit signal is serialized and sent via the NodeSession rather than looking up a local process.

### Remote Spawn Flow

```
1. snow_dist_spawn(node_name, fn_name, args) -> remote_pid
2. Router resolves node_name to NodeSession
3. NodeSession sends SPAWN_REQUEST [fn_name | serialized_args]
4. Remote node looks up fn_name in its function registry
5. Remote node does local spawn, returns PID
6. NodeSession sends SPAWN_REPLY [node_id | local_pid]
7. Caller receives DistributedPid(remote_node_id, remote_local_pid)
```

Note: Remote spawn requires the remote node to have a registry of spawnable functions (by name). This is a new concept for Snow -- currently functions are identified by pointer, not name. The function registry maps Snow module+function names to entry point pointers.

## Node Connection Management

### Architecture: One NodeSession Actor Per Peer

Following the established **reader thread bridge pattern** from WebSocket (`ws/server.rs`), each remote node connection uses:

1. **NodeSession actor**: Runs on the M:N scheduler as a regular Snow actor with `trap_exit = true`
2. **Reader thread**: Dedicated OS thread reads framed messages from the TCP/TLS stream, pushes to NodeSession mailbox via reserved type tags
3. **Writer access**: NodeSession actor writes outgoing messages directly to the stream (same `Arc<Mutex<Stream>>` pattern as `WsConnection`)

```
+------------------------------------------+
| NodeSession Actor (on M:N scheduler)      |
|                                            |
|  Mailbox receives:                         |
|    - DIST_SEND_TAG: forward to remote      |  <-- from Router
|    - DIST_RECV_TAG: incoming from remote   |  <-- from Reader thread
|    - DIST_LINK_TAG: link protocol          |
|    - DIST_MONITOR_TAG: monitor protocol    |
|    - DIST_SPAWN_TAG: remote spawn request  |
|    - EXIT_SIGNAL_TAG: exit signals         |
+------------------------------------------+
        |                         ^
        v (writes)                | (reads)
+------------------------------------------+
|     Arc<Mutex<TLS/TCP Stream>>            |
+------------------------------------------+
        |                         ^
        v                         |
+------------------------------------------+
|     Reader Thread (OS thread)             |
|     read_frame() -> push to mailbox       |
+------------------------------------------+
```

### Reserved Type Tags for Distribution

Following the existing pattern (WebSocket uses `u64::MAX-1` through `u64::MAX-4`, exit signals use `u64::MAX`):

```rust
pub const EXIT_SIGNAL_TAG:     u64 = u64::MAX;      // existing
pub const WS_TEXT_TAG:         u64 = u64::MAX - 1;   // existing
pub const WS_BINARY_TAG:      u64 = u64::MAX - 2;   // existing
pub const WS_DISCONNECT_TAG:  u64 = u64::MAX - 3;   // existing
pub const WS_CONNECT_TAG:     u64 = u64::MAX - 4;   // existing

// Distribution tags:
pub const DIST_SEND_TAG:       u64 = u64::MAX - 10;  // outgoing send request
pub const DIST_RECV_TAG:       u64 = u64::MAX - 11;  // incoming message from remote
pub const DIST_LINK_TAG:       u64 = u64::MAX - 12;  // link protocol signal
pub const DIST_UNLINK_TAG:     u64 = u64::MAX - 13;  // unlink protocol signal
pub const DIST_MONITOR_TAG:    u64 = u64::MAX - 14;  // monitor signal
pub const DIST_DEMONITOR_TAG:  u64 = u64::MAX - 15;  // demonitor signal
pub const DIST_SPAWN_REQ_TAG:  u64 = u64::MAX - 16;  // spawn request
pub const DIST_SPAWN_REPLY_TAG:u64 = u64::MAX - 17;  // spawn reply
pub const DIST_EXIT_TAG:       u64 = u64::MAX - 18;  // remote exit signal
pub const DIST_NODE_DOWN_TAG:  u64 = u64::MAX - 19;  // node disconnect notification
```

### NodeRegistry: Dual-Map Pattern

Following the existing `RoomRegistry` pattern from `ws/rooms.rs`:

```rust
pub struct NodeRegistry {
    /// node_name -> NodeSession info
    by_name: RwLock<FxHashMap<String, NodeSessionInfo>>,
    /// node_id (u16) -> NodeSession info
    by_id: RwLock<FxHashMap<u16, NodeSessionInfo>>,
}

pub struct NodeSessionInfo {
    pub node_id: u16,
    pub node_name: String,
    pub session_pid: ProcessId,
    pub creation: u32,
}
```

The `by_id` map is the hot path (every remote send does `pid.node_id()` lookup), so it must be fast. The `by_name` map is used for initial connection setup and `Node.connect(name)`.

### Connection Lifecycle

```
1. Node A calls Node.connect("node_b@host")
2. Discovery resolves "node_b@host" to IP:port
3. TCP connection established
4. TLS handshake (using existing rustls CryptoProvider)
5. Distribution handshake:
   a. send_name: node_a sends its name, capabilities, creation
   b. recv_status: node_b accepts or rejects
   c. challenge: node_b sends random challenge
   d. challenge_reply: node_a proves knowledge of shared cookie
   e. challenge_ack: node_b proves knowledge of shared cookie
6. NodeSession actors spawned on both sides
7. node_id assigned and registered in NodeRegistry
8. Nodes exchange alive process lists (optional, for pg-style groups)
```

### Node Failure Detection

Each NodeSession uses heartbeat (similar to WebSocket heartbeat in `ws/server.rs`):

- NodeSession sends periodic TICK messages (every 15s)
- Reader thread detects TCP read timeout (no data for 60s)
- On detected failure: push `DIST_NODE_DOWN_TAG` to all processes monitoring that node
- All remote PIDs from the failed node become "dead" -- sends to them silently fail
- Links to remote processes on the failed node trigger exit signals locally

## Binary Wire Format

### Design Philosophy

Snow does NOT need to be compatible with Erlang's External Term Format (ETF). The wire format should be:

1. **Simple**: Snow has fewer types than Erlang
2. **Efficient**: Minimal overhead for common cases (integers, strings, tuples)
3. **Self-describing**: Type tags on the wire for safe deserialization
4. **Versionable**: A version byte for future evolution

### Framing Layer

Each message on the wire is framed as:

```
[4-byte length (big-endian)] [message bytes]
```

This matches the Erlang distribution protocol framing and is a well-understood pattern. The 4-byte length supports messages up to 4 GiB.

### Message Envelope

```
[1-byte op_type] [payload...]

Op types:
  0x01 = SEND          [8-byte target_local_pid] [8-byte type_tag] [N-byte serialized_msg]
  0x02 = LINK          [8-byte from_pid] [8-byte to_pid]
  0x03 = UNLINK        [8-byte from_pid] [8-byte to_pid] [8-byte unlink_id]
  0x04 = EXIT          [8-byte from_pid] [8-byte to_pid] [exit_reason_bytes]
  0x05 = MONITOR       [8-byte from_pid] [8-byte to_pid] [8-byte ref]
  0x06 = DEMONITOR     [8-byte from_pid] [8-byte to_pid] [8-byte ref]
  0x07 = MONITOR_EXIT  [8-byte from_pid] [8-byte to_pid] [8-byte ref] [reason_bytes]
  0x08 = SPAWN_REQ     [8-byte req_id] [name_len:u16] [name_bytes] [args_bytes]
  0x09 = SPAWN_REPLY   [8-byte req_id] [8-byte pid_or_error]
  0x0A = REG_SEND      [name_len:u16] [name_bytes] [8-byte type_tag] [msg_bytes]
  0x0B = TICK           (empty -- heartbeat keepalive)
  0x0C = NODE_INFO     [capabilities...] (exchanged during handshake)
```

### Value Serialization Format

For the message body within SEND envelopes, Snow values are serialized as:

```
Type tags (1 byte):
  0x01 = Int(i64)       -> [8 bytes LE]
  0x02 = Float(f64)     -> [8 bytes LE]
  0x03 = Bool           -> [1 byte: 0 or 1]
  0x04 = String         -> [4-byte length] [UTF-8 bytes]
  0x05 = Atom           -> [2-byte length] [UTF-8 bytes]
  0x06 = Tuple          -> [1-byte arity] [element...]
  0x07 = List           -> [4-byte length] [element...]
  0x08 = Map            -> [4-byte length] [key, value...]
  0x09 = Pid            -> [2-byte node_id] [8-byte local_id]
  0x0A = Ref            -> [2-byte node_id] [8-byte ref_id]
  0x0B = Binary         -> [4-byte length] [bytes]
  0x0C = Nil            -> (no payload)
  0x0D = ADT variant    -> [4-byte tag_hash] [1-byte arity] [fields...]
  0x0E = Opaque handle  -> NOT SERIALIZABLE (error)
```

### Integration With Existing Type System

The `MessageBuffer` in `heap.rs` already carries `data: Vec<u8>` and `type_tag: u64`. For local sends, messages are copied as raw bytes between actor heaps. For remote sends, the same `Vec<u8>` is wrapped in the wire SEND envelope.

**Key insight**: Snow messages are already serialized as byte arrays with type tags in the existing MessageBuffer. The wire format wraps these byte arrays with routing information. The deserialization on the remote side reconstructs a MessageBuffer and pushes it into the local mailbox -- the receiving actor sees no difference from a local message.

```rust
// Remote send: serialize for wire
fn remote_send(pid: ProcessId, msg_ptr: *const u8, msg_size: u64) {
    let data = unsafe { std::slice::from_raw_parts(msg_ptr, msg_size as usize) };
    let type_tag = derive_type_tag(data);

    let node_id = pid.node_id();
    let session = node_registry().get_by_id(node_id);

    // Build wire frame: [op=SEND][target_local_pid][type_tag][msg_bytes]
    let mut frame = Vec::with_capacity(1 + 8 + 8 + data.len());
    frame.push(0x01); // SEND
    frame.extend_from_slice(&pid.local_id().to_le_bytes());
    frame.extend_from_slice(&type_tag.to_le_bytes());
    frame.extend_from_slice(data);

    // Push to NodeSession's outgoing queue
    session.enqueue_outgoing(frame);
}

// Remote receive: deserialize from wire
fn handle_incoming_send(frame: &[u8]) {
    let target_local_id = u64::from_le_bytes(frame[1..9].try_into().unwrap());
    let type_tag = u64::from_le_bytes(frame[9..17].try_into().unwrap());
    let msg_data = frame[17..].to_vec();

    let buffer = MessageBuffer::new(msg_data, type_tag);
    let msg = Message { buffer };

    // Standard local delivery
    let pid = ProcessId(target_local_id); // node_id=0 since it's local now
    if let Some(proc_arc) = global_scheduler().get_process(pid) {
        let mut proc = proc_arc.lock();
        proc.mailbox.push(msg);
        if matches!(proc.state, ProcessState::Waiting) {
            proc.state = ProcessState::Ready;
            drop(proc);
            global_scheduler().wake_process(pid);
        }
    }
}
```

## Handshake and Authentication

### Cookie-Based Auth (BEAM-compatible model)

```
Node A (initiator)                    Node B (acceptor)
    |                                     |
    |--- TCP connect --->                 |
    |--- send_name(name_a, flags) ------->|
    |<-- recv_status("ok") --------------|
    |<-- send_challenge(name_b, chall_b)-|
    |--- send_challenge_reply(          ->|
    |      chall_a, md5(cookie+chall_b))  |
    |<-- send_challenge_ack(            --|
    |      md5(cookie+chall_a))           |
    |                                     |
    |===== CONNECTED (TLS encrypted) =====|
```

Snow already has rustls 0.23 with CryptoProvider. The TLS handshake happens BEFORE the distribution handshake, so all cookie exchange is encrypted. This is better than BEAM's default (which sends challenge/response in cleartext unless TLS is explicitly configured).

### Node Creation Counter

Following the BEAM model, each node incarnation gets a unique 32-bit `creation` value. This allows distinguishing PIDs from a crashed-and-restarted node versus the current incarnation. PIDs with a stale creation are treated as dead.

```rust
pub struct NodeIdentity {
    pub name: String,           // e.g., "snow@192.168.1.10"
    pub creation: u32,          // incremented on each node restart
    pub node_id: u16,           // assigned during connection setup
    pub cookie: String,         // shared secret for authentication
    pub listen_port: u16,       // port for incoming connections
}
```

## Node Discovery (EPMD Equivalent)

### Design: Embedded Discovery (No Separate Daemon)

BEAM uses a separate EPMD daemon (port 4369) for node name resolution. For Snow, **embed discovery into the runtime** to avoid requiring a separate process:

1. **Static configuration**: Pass node addresses via config file or environment
2. **DNS-based**: Resolve node names via DNS SRV records
3. **mDNS**: Optional zero-config discovery for development (like Elixir's libcluster)

For the first implementation, use **static configuration** (simplest, most predictable):

```elixir
# Snow config
Node.start("snow@192.168.1.10", cookie: "secret")
Node.connect("snow@192.168.1.20:4370")
```

The runtime listens on a configurable port (default 4370) for incoming distribution connections. Outgoing connections specify host:port directly.

## New Modules and Files

### New: `snow-rt/src/dist/` Module

```
snow-rt/src/dist/
    mod.rs              # Module root, public API
    node.rs             # NodeIdentity, NodeId, creation management
    router.rs           # Distribution router (locality check + remote dispatch)
    session.rs          # NodeSession actor (per-peer connection management)
    registry.rs         # NodeRegistry (node_name -> session mapping)
    wire.rs             # Binary wire format codec (serialize/deserialize)
    handshake.rs        # Challenge/response authentication protocol
    discovery.rs        # Node name resolution
    tick.rs             # Heartbeat / failure detection
```

### Modified: Existing Files

| File | Change | Scope |
|------|--------|-------|
| `snow-rt/src/actor/process.rs` | Add `node_id()`, `local_id()`, `is_local()`, `from_remote()` to ProcessId | Small, backward-compatible |
| `snow-rt/src/actor/mod.rs` | Add locality check in `snow_actor_send`, `snow_actor_link` | ~10 lines each |
| `snow-rt/src/actor/link.rs` | Handle remote PIDs in `propagate_exit` | ~20 lines |
| `snow-rt/src/actor/registry.rs` | No changes needed (names are node-local) | None |
| `snow-rt/src/lib.rs` | Add `pub mod dist;` and re-export dist API functions | Small |
| `snow-codegen/src/codegen/intrinsics.rs` | Declare new dist intrinsics (`snow_node_connect`, `snow_dist_spawn`, etc.) | ~30 lines |
| `snow-rt/src/actor/scheduler.rs` | No changes (scheduler is node-local) | None |

### New LLVM Intrinsics

```rust
// New extern "C" functions for Snow codegen:
snow_node_start(name: ptr, name_len: u64, port: i64, cookie: ptr, cookie_len: u64)
snow_node_connect(name: ptr, name_len: u64) -> i64  // returns node_id or 0
snow_node_disconnect(name: ptr, name_len: u64)
snow_node_self() -> ptr  // returns node name as SnowString
snow_node_list() -> ptr  // returns list of connected node names
snow_dist_spawn(node: ptr, node_len: u64, fn_name: ptr, fn_len: u64,
                args: ptr, args_size: u64) -> u64  // returns remote PID
snow_dist_send_named(node: ptr, node_len: u64, name: ptr, name_len: u64,
                     msg: ptr, msg_size: u64)  // send to {name, node}
snow_node_monitor(node: ptr, node_len: u64) -> u64  // monitor node connectivity
```

Note: `snow_actor_send` does NOT need a new intrinsic -- it already takes a `u64` PID. Remote PIDs just have a non-zero node_id in the high 16 bits. Location transparency is achieved through the existing send interface.

## Data Flow: Complete Remote Message Journey

```
Actor A on Node 1                        Actor B on Node 2
    |                                        |
1.  send(pid_b, message)                     |
2.  snow_actor_send(0x0002_0000_0000_0005,   |
                    msg_ptr, msg_size)        |
3.  pid.is_local() == false (node_id=2)      |
4.  router.remote_send(pid, msg)             |
5.  NodeRegistry.get_by_id(2) -> session     |
6.  Build wire frame:                        |
    [len:4][op:SEND][local_pid:8]            |
    [type_tag:8][msg_data:N]                 |
7.  session.write(frame) ----TCP/TLS-------->|
                                         8.  Reader thread: read_frame()
                                         9.  Parse: op=SEND, target=5, tag, data
                                         10. MessageBuffer::new(data, tag)
                                         11. local_send(ProcessId(5), msg)
                                         12. proc.mailbox.push(msg)
                                         13. wake if Waiting
                                             |
                                         14. Actor B receives message
                                             (identical to local delivery)
```

## Patterns to Follow

### Pattern 1: Reader Thread Bridge (from WebSocket)

**What:** Dedicated OS thread per connection for I/O, delivers messages to actor mailbox via reserved type tags.

**When:** Always for distribution connections. TCP reads are blocking; the M:N scheduler coroutines must not block on I/O.

**Why reuse this pattern:** It is already proven in the WebSocket implementation (`ws/server.rs`). The NodeSession actor follows the exact same structure as the WebSocket connection actor: reader thread reads frames, pushes to mailbox, actor processes messages.

### Pattern 2: Opaque Handle (from WsConnection)

**What:** Rust-heap allocated struct (not GC heap) referenced by raw pointer cast to u64.

**When:** For NodeSession connection handles exposed to Snow code.

**Why:** Same reason as WsConnection -- the connection state (TCP stream, TLS state) cannot live on the GC heap. The u64 handle is GC-safe.

### Pattern 3: RoomRegistry Dual-Map (from ws/rooms.rs)

**What:** Two synchronized maps for forward and reverse lookup with consistent lock ordering.

**When:** NodeRegistry needs both name-to-session and id-to-session lookups.

### Pattern 4: Reserved Type Tags (from WebSocket + exit signals)

**What:** Sentinel values in the `u64::MAX - N` range for special mailbox messages.

**When:** All distribution protocol messages delivered to NodeSession actors.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Global Lock on Every Send

**What:** Taking a global lock to check if a PID is remote.

**Why bad:** Would serialize all sends, destroying M:N scheduler throughput.

**Instead:** The `pid >> 48 == 0` locality check is a single bitwise operation with zero contention. Only remote sends need the NodeRegistry lock, and that is a read lock (RwLock) that can be concurrent.

### Anti-Pattern 2: Serializing Local Messages

**What:** Running local messages through the wire serializer for "uniformity."

**Why bad:** Adds serialization overhead to 100% of sends when only cross-node sends need it.

**Instead:** Local sends remain as raw byte copy between actor heaps (the existing MessageBuffer path). Only remote sends serialize to the wire format.

### Anti-Pattern 3: Synchronous Remote Operations

**What:** Blocking the calling actor while waiting for remote send acknowledgment.

**Why bad:** Defeats the purpose of async actors. Network latency would block the caller.

**Instead:** Remote sends are fire-and-forget (like BEAM). The message is enqueued in the NodeSession's outgoing buffer and sent asynchronously. If the node is down, the send silently fails (or triggers a nodedown monitor if the caller is monitoring).

### Anti-Pattern 4: Separate Distribution Thread Pool

**What:** Creating a separate thread pool for distribution I/O.

**Why bad:** Adds complexity and coordination overhead. The existing M:N scheduler already handles actor scheduling well.

**Instead:** NodeSession actors run on the existing M:N scheduler. Only the reader threads (one per peer connection) are separate OS threads, following the proven WebSocket pattern.

### Anti-Pattern 5: Embedding Full Node Name in Every PID

**What:** Storing the node name string inside the PID.

**Why bad:** PIDs are copied constantly. String allocation and comparison would destroy performance.

**Instead:** 16-bit node_id in the PID, with a side table mapping node_id to node name. The hot path only touches the u16.

## Scalability Considerations

| Concern | 1 Node | 10 Nodes | 100 Nodes |
|---------|--------|----------|-----------|
| PID encoding | No overhead | 16-bit check per send | Same |
| Connections | 0 | 9 TCP+TLS per node (full mesh) | 99 per node (consider partial mesh) |
| Reader threads | 0 | 9 OS threads | 99 OS threads (may need pooling) |
| NodeRegistry lookup | N/A | RwLock, ~10 entries | RwLock, ~100 entries |
| Wire format overhead | N/A | ~17 bytes per message | Same |
| Heartbeat traffic | None | 9 ticks/15s = 0.6 msg/s | 99 ticks/15s = 6.6 msg/s |

At 100+ nodes, the full-mesh approach (every node connects to every other) creates O(N^2) connections. For very large clusters, a partial-mesh or routing-node topology would be needed. This is a future concern -- BEAM systems typically run 5-50 node clusters.

## Suggested Build Order

Based on dependency analysis of the existing codebase:

1. **PID encoding** (`process.rs` changes) -- Foundation, everything depends on it
2. **Wire format codec** (`dist/wire.rs`) -- Independent, can be tested in isolation
3. **Distribution router** (`dist/router.rs` + `mod.rs` changes) -- Bridges PID to session
4. **Node identity and registry** (`dist/node.rs`, `dist/registry.rs`) -- NodeSession needs it
5. **Handshake protocol** (`dist/handshake.rs`) -- Needed before NodeSession can work
6. **NodeSession actor** (`dist/session.rs`) -- The main workhorse, depends on 1-5
7. **Node discovery** (`dist/discovery.rs`) -- Last mile: finding peers
8. **Remote spawn** -- Requires function name registry (new concept)
9. **Remote monitoring** -- Extension of existing `link.rs`
10. **LLVM intrinsics** (`intrinsics.rs`) -- Wire up Snow-level API

## Sources

- [Erlang Distribution Protocol -- erts v16.2](https://www.erlang.org/doc/apps/erts/erl_dist_protocol.html) -- Official protocol specification (HIGH confidence)
- [Erlang External Term Format -- erts v16.2](https://www.erlang.org/doc/apps/erts/erl_ext_dist.html) -- PID encoding format (HIGH confidence)
- [Distributed Erlang -- Erlang System Documentation v28.3.1](https://www.erlang.org/doc/system/distributed.html) -- Distribution architecture overview (HIGH confidence)
- [Erlang Distribution over TLS](https://www.erlang.org/doc/apps/ssl/ssl_distribution.html) -- TLS integration for distribution (HIGH confidence)
- [EEF Security WG -- Distribution Protocol and EPMD](https://security.erlef.org/secure_coding_and_deployment_hardening/distribution.html) -- Security considerations (HIGH confidence)
- [Proto.Actor -- Location Transparency](https://proto.actor/docs/location-transparency/) -- PID-based location transparency patterns (MEDIUM confidence)
- [Ractor -- Rust actor framework](https://github.com/slawlor/ractor) -- Rust distributed actor implementation reference (MEDIUM confidence)
- [ractor_cluster](https://docs.rs/ractor_cluster/latest/ractor_cluster/) -- Erlang-style cluster protocol in Rust (MEDIUM confidence)
- Direct analysis of Snow codebase: `snow-rt/src/actor/`, `snow-rt/src/ws/`, `snow-codegen/src/codegen/intrinsics.rs` (HIGH confidence)
