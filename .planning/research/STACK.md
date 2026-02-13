# Technology Stack: Snow Distributed Actors

**Project:** Snow compiler -- BEAM-style distributed actor system (multi-node clustering, location-transparent PIDs, remote spawn/send/monitor, binary wire format, node registry)
**Researched:** 2026-02-12
**Confidence:** HIGH (codebase analysis of snow-rt actor/scheduler/registry/TLS, Erlang distribution protocol specification, verified crate availability via Cargo.lock)

## Recommended Stack

### Design Decision: Custom Wire Format, Not ETF

Snow should implement a **Snow-native binary term format (STF)** inspired by ETF but tailored to Snow's type system, rather than attempting ETF compatibility. Rationale:

1. **Snow is not Erlang.** Snow has HM-inferred static types, not Erlang's dynamic terms. ETF encodes atoms, references, funs, ports -- Snow has none of these. Implementing full ETF is wasted effort.
2. **ETF carries security baggage.** ETF allows deserializing anonymous functions, which is a known RCE vector (per [EEF Security WG](https://erlef.github.io/security-wg/secure_coding_and_deployment_hardening/serialisation.html)). A custom format avoids this by design.
3. **Type-tag dispatch already exists.** Snow messages already use `[u64 type_tag, u64 data_len, u8... data]` layout in actor heaps. STF extends this with node/PID metadata, not replaces it.
4. **Simpler, faster, smaller.** No need for varint atom encoding, external fun refs, or BERT compatibility. Big-endian fixed-width fields match what the runtime already does.

The wire format should use **big-endian** byte order (matching Erlang's convention and network byte order) with fixed-width integer fields. This aligns with the PostgreSQL wire protocol already implemented in `pg.rs`.

### Design Decision: Built-in Node Registry, Not EPMD

Snow should implement a **built-in node registry** (the node process itself handles registration/discovery) rather than requiring a separate EPMD daemon. Rationale:

1. **Single-binary philosophy.** Snow compiles to standalone native binaries. Requiring a separate daemon process contradicts this.
2. **Simpler deployment.** One binary, one process. No `epmd` to manage, no port 4369 to secure.
3. **EPMD is simple.** The protocol is ~5 message types with trivial wire format. Embedding it adds minimal complexity.
4. **Erlang itself supports this.** OTP 21+ supports `-epmd_module` for custom discovery. Snow can bake this in from the start.

Each Snow node listens on a configurable port for both distribution connections and registry queries. Nodes discover each other via explicit `Node.connect("name@host:port")` -- no multicast/mDNS needed for v1.

### NEW Dependencies

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| *None* | -- | -- | All new functionality is built with existing deps + std library. See rationale below. |

**Zero new crate dependencies.** This is deliberate and follows the project's established pattern:

- The PostgreSQL wire protocol was built with `std::net::TcpStream` + existing crypto crates (sha2, hmac, md5, base64, rand).
- The HTTP server was built with `std::net::TcpListener` + hand-rolled parser.
- The WebSocket server was built with `sha1` (same RustCrypto family) + hand-rolled frame codec.
- Distribution follows the same pattern: `std::net::TcpStream` + `rustls` for TLS + hand-rolled wire protocol.

The only crate that was considered and rejected is `dashmap` (see Alternatives below).

### Existing Dependencies Reused (NO CHANGES to Cargo.toml)

| Technology | Version (Cargo.lock) | New Use | Already Used For |
|------------|---------------------|---------|-----------------|
| rustls | 0.23.36 | TLS for inter-node connections (`DistStream::Tls` variant) | PostgreSQL TLS (`PgStream`), HTTPS (`HttpStream`), WSS (`WsStream`) |
| webpki-roots | 0.26 | Root CA certs for verifying peer node certificates | PostgreSQL TLS, HTTPS |
| rustls-pki-types | 1 | PEM cert/key loading for node TLS identity | HTTPS `Http.serve_tls()`, WSS `Ws.serve_tls()` |
| ring | 0.17.14 | (transitive via rustls) Available for MD5 challenge digest if needed | Transitive dep of rustls |
| sha2 | 0.10.9 | HMAC-SHA256 for cookie-based challenge/response authentication | PostgreSQL SCRAM-SHA-256 |
| hmac | 0.12.1 | HMAC construction for challenge/response | PostgreSQL SCRAM-SHA-256 |
| rand | 0.9.2 | Generate 32-bit random challenges for handshake | PostgreSQL SCRAM nonce |
| parking_lot | 0.12.5 | `RwLock` for node connection table, `Mutex` for per-connection state | Actor process table, process registry, room registry |
| rustc-hash | workspace (2) | `FxHashMap` for node table, remote PID routing table | Process registry, scheduler, room registry |
| crossbeam-channel | 0.5.15 | Channel for distribution controller to enqueue remote messages | Actor scheduler high-priority channel |
| crossbeam-deque | 0.8.6 | (unchanged) Work-stealing scheduler | Actor scheduler |
| corosensei | 0.3.2 | (unchanged) Coroutines for actor execution | M:N actor scheduler |
| base64 | 0.22 | (potentially) Encoding cookie in config files | PostgreSQL SCRAM |
| serde_json | 1 | (potentially) Node config file parsing | Existing JSON support |

### Core Framework: Hand-Rolled Distribution Protocol

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Hand-rolled STF (Snow Term Format) | N/A | Binary serialization of messages for inter-node transport | Consistent with project approach: hand-rolled PostgreSQL wire protocol, HTTP/1.1 parser, WebSocket frame codec. Snow's message format is simpler than ETF (no atoms, funs, ports). ~300-400 lines of encode/decode. |
| Hand-rolled distribution handshake | N/A | Node authentication via HMAC-SHA256 challenge/response with shared cookie | Mirrors Erlang's approach but uses HMAC-SHA256 instead of MD5 (stronger, already available via `sha2` + `hmac`). ~200 lines. |
| Hand-rolled node registry | N/A | Built-in EPMD-equivalent: node name registration, port lookup | Simpler than EPMD (no separate daemon). Each node can answer "what port is node X on this host?" ~150 lines. |

### Infrastructure

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `std::net::TcpListener` | std | Distribution listener for incoming node connections | Same pattern as HTTP server and WebSocket server. Blocking accept is appropriate for Snow's blocking I/O model. |
| `std::net::TcpStream` | std | Distribution connections to peer nodes | Same as PostgreSQL client, HTTP server connections. |
| `std::thread::spawn` | std | Background thread per node connection for receiving messages | Same pattern as WebSocket reader threads. One OS thread per peer node connection (typically 2-20 nodes, not thousands). |
| `std::sync::atomic::AtomicBool` | std | Shutdown flags for connection threads | Same pattern as WebSocket reader shutdown. |
| `std::sync::atomic::AtomicU64` | std | Node creation counter (incarnation number) | Same pattern as `ProcessId::next()`. |

## Integration Points with Existing Runtime

### ProcessId Extension

Current `ProcessId` is a simple `u64` counter:
```
pub struct ProcessId(pub u64);
// Generated via: static COUNTER: AtomicU64
```

For distribution, PIDs must encode node identity. Recommended approach:

```
// Bit layout for distributed ProcessId (still u64):
// [16 bits: node_id][48 bits: local_process_number]
//
// node_id 0 = local node (backward compatible)
// node_id 1-65535 = remote nodes
//
// 48 bits = 281 trillion local PIDs per node (plenty)
```

This is a **zero-cost change** for existing local code: `node_id = 0` means all existing PIDs keep working. The `ProcessId(pub u64)` struct is unchanged; interpretation changes.

### snow_actor_send Extension

Current `snow_actor_send` looks up the target in the local process table. For distribution:

```
// Pseudocode for distributed send:
fn snow_actor_send(target_pid: u64, msg_ptr, msg_size) {
    let node_id = target_pid >> 48;
    if node_id == 0 {
        // Local send (existing path, unchanged)
        local_send(target_pid, msg_ptr, msg_size);
    } else {
        // Remote send: serialize message, route to node connection
        let conn = node_connections.get(node_id);
        conn.send_distributed(target_pid, msg_ptr, msg_size);
    }
}
```

### DistStream (New, Following Existing Pattern)

```rust
// Mirrors PgStream, HttpStream, WsStream pattern exactly:
enum DistStream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),  // client side
    TlsServer(StreamOwned<ServerConnection, TcpStream>),  // server side
}
```

### Connection Lifecycle

Following the established Snow pattern (OS thread per connection for blocking I/O):

1. **Listener thread** (`std::thread::spawn`): accepts incoming TCP connections on the distribution port
2. **Handshake** on the accepted connection (challenge/response with cookie)
3. **TLS upgrade** if configured (using existing `rustls` ClientConfig/ServerConfig)
4. **Receiver thread** per connection: reads framed messages, deserializes, routes to local actor mailboxes via `snow_actor_send`
5. **Send path**: when `snow_actor_send` targets a remote PID, the message is serialized and written to the connection's `DistStream` (under `Mutex`, same as WebSocket write path)

## Wire Format: Snow Term Format (STF)

### Message Frame

```
[4 bytes: total_length (big-endian u32)]
[1 byte:  message_type]
[variable: payload]
```

Message types:
- `0x01` SEND -- send message to remote PID
- `0x02` LINK -- create link between local and remote PID
- `0x03` UNLINK -- remove link
- `0x04` EXIT -- propagate exit signal
- `0x05` MONITOR -- set up monitor
- `0x06` DEMONITOR -- remove monitor
- `0x07` MONITOR_DOWN -- monitor triggered
- `0x08` SPAWN_REQUEST -- request remote spawn
- `0x09` SPAWN_REPLY -- response with new PID
- `0x10` REG_SEND -- send to named process on remote node
- `0x11` HEARTBEAT -- keepalive tick

### Value Encoding (for message payloads)

```
Tag byte followed by type-specific data:

0x01  u8:     [1 byte value]
0x02  i64:    [8 bytes big-endian]
0x03  f64:    [8 bytes IEEE 754 big-endian]
0x04  bool:   [1 byte: 0=false, 1=true]
0x05  string: [4 bytes length][UTF-8 bytes]
0x06  binary: [4 bytes length][raw bytes]
0x07  tuple:  [2 bytes arity][elements...]
0x08  list:   [4 bytes length][elements...]
0x09  map:    [4 bytes count][key,value pairs...]
0x0A  pid:    [8 bytes: full u64 ProcessId]
0x0B  nil:    (unit/none)
0x0C  atom:   [2 bytes length][UTF-8 bytes] (for tagged unions/enum variants)
0x0D  result: [1 byte: 0=ok, 1=err][value]
```

This maps directly to Snow's type system. No ETF baggage (no funs, ports, refs, external funs, compressed terms).

### Handshake Protocol

Simplified from Erlang's 7-step handshake to 4 steps:

```
1. A -> B:  HELLO  [name_len:2][name:UTF-8][flags:8][creation:4]
2. B -> A:  CHALLENGE [name_len:2][name:UTF-8][flags:8][creation:4][challenge:4]
3. A -> B:  CHALLENGE_REPLY [challenge:4][digest:32]
                            (digest = HMAC-SHA256(cookie, challenge_B))
4. B -> A:  CHALLENGE_ACK [digest:32]
                          (digest = HMAC-SHA256(cookie, challenge_A))
```

Uses HMAC-SHA256 instead of Erlang's MD5. Both `sha2` and `hmac` are already in Cargo.toml. The cookie never crosses the wire.

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Wire format | Hand-rolled STF | Erlang ETF | ETF encodes Erlang-specific types (atoms, funs, ports, refs) that Snow does not have. Full ETF compatibility is wasted effort and carries security baggage (RCE via fun deserialization). |
| Wire format | Hand-rolled STF | Protocol Buffers (prost) | Adds `prost` + `prost-build` + protoc dependency. Overkill for an internal binary protocol. Snow controls both ends of the wire. Protobuf's schema evolution features are unnecessary. |
| Wire format | Hand-rolled STF | bincode 3.0 / postcard 1.1 | Adds external dependency for what is ~300 lines of code. These crates optimize for Rust struct serde, but Snow needs to serialize Snow-typed values (from LLVM-compiled code), not Rust structs. The runtime operates on raw `*const u8` buffers, not serde-compatible types. |
| Wire format | Hand-rolled STF | MessagePack (rmp-serde) | Adds dependency. MessagePack is a general-purpose format with JSON-like semantics. Snow's type system is richer (tuples, tagged unions, typed PIDs). Custom format maps directly. |
| Node discovery | Built-in registry | External EPMD daemon | Requires deploying a separate process. Violates single-binary philosophy. |
| Node discovery | Built-in registry | mDNS (mdns-sd crate) | Adds dependency. mDNS is for LAN discovery. Production distributed systems use explicit configuration (known hosts/ports), not multicast discovery. LAN discovery can be added later as an optional feature. |
| Node discovery | Built-in registry | Gossip protocol (memberlist) | Massive dependency. Overkill for initial version. Snow nodes will use explicit `Node.connect()`. Gossip-based auto-discovery is a future enhancement. |
| Node connections | `parking_lot::Mutex<DistStream>` | DashMap for connection table | Adds a new dependency. `parking_lot::RwLock<FxHashMap>` is the existing pattern used in the process table, process registry, and room registry. Connection table has the same access pattern (many reads, few writes). Stay consistent. |
| Challenge auth | HMAC-SHA256 | MD5 (Erlang's approach) | MD5 is cryptographically broken. HMAC-SHA256 is stronger and both `sha2` and `hmac` are already direct dependencies. Zero additional cost for better security. |
| Challenge auth | HMAC-SHA256 | Full TLS client certs only | TLS client certs are harder to set up and manage. Cookie-based auth is simpler for development/testing. TLS is still used for transport encryption. The cookie provides cluster membership control (which nodes can join). |
| Transport | TCP + optional TLS | QUIC (quinn crate) | Adds `quinn` + `rustls` (different integration) + UDP complexity. QUIC's multiplexing benefit is marginal for node-to-node connections (typically 1 connection per peer). TCP is simpler, proven for this use case. BEAM uses TCP. The Snow runtime uses blocking I/O, and quinn requires async. |
| Serialization lib | No external crate | serde + bincode | The runtime works with raw `*const u8` message buffers passed via `extern "C"` ABI from LLVM-compiled code. These are not Rust structs with `#[derive(Serialize)]`. Serde does not apply here. |

## Architecture: What Goes Where

```
crates/snow-rt/src/
  actor/
    mod.rs          -- extend snow_actor_send() with remote routing
    process.rs      -- ProcessId bit-layout documentation (no struct change)
    registry.rs     -- (unchanged, local registry)
  dist/             -- NEW MODULE
    mod.rs          -- public API: snow_dist_* extern "C" functions
    node.rs         -- NodeId, NodeInfo, node connection management
    handshake.rs    -- challenge/response protocol
    transport.rs    -- DistStream enum, framed read/write
    stf.rs          -- Snow Term Format encode/decode
    registry.rs     -- node name registry (EPMD equivalent)
    monitor.rs      -- remote process monitoring
    remote_spawn.rs -- remote spawn request/reply
```

This follows the existing pattern: `db/` has `pg.rs`, `pool.rs`, `row.rs`; `http/` has `server.rs`, `router.rs`; `ws/` has `frame.rs`, `handshake.rs`, `server.rs`.

## Installation

```toml
# In crates/snow-rt/Cargo.toml:
# NO CHANGES REQUIRED.
#
# All dependencies needed for distribution are already present:
# - rustls 0.23 (TLS transport)
# - sha2 0.10 + hmac 0.12 (HMAC-SHA256 challenge auth)
# - rand 0.9 (challenge generation)
# - parking_lot 0.12 (connection table locks)
# - rustc-hash (FxHashMap for routing tables)
# - crossbeam-channel 0.5 (message routing channels)
```

## Version Pinning Summary

| Crate | Pin | Status | Role in Distribution |
|-------|-----|--------|---------------------|
| rustls | `"0.23"` | Existing | TLS transport for inter-node connections |
| webpki-roots | `"0.26"` | Existing | CA certs for peer node verification |
| rustls-pki-types | `"1"` | Existing | PEM cert/key loading for node identity |
| sha2 | `"0.10"` | Existing | HMAC-SHA256 challenge digest |
| hmac | `"0.12"` | Existing | HMAC construction for challenge/response |
| rand | `"0.9"` | Existing | 32-bit random challenge generation |
| parking_lot | `"0.12"` | Existing | RwLock/Mutex for connection state |
| rustc-hash | workspace | Existing | FxHashMap for node/routing tables |
| crossbeam-channel | `"0.5"` | Existing | Distribution message routing |
| **NEW deps** | -- | **None** | -- |

## Key Risks and Mitigations

| Risk | Severity | Mitigation |
|------|----------|------------|
| Blocking I/O for inter-node connections | Medium | One OS thread per peer node is acceptable for typical cluster sizes (2-20 nodes). This is the same model used for PostgreSQL, HTTP, and WebSocket connections. If Snow needs 1000+ peer connections, revisit with async I/O. |
| ProcessId bit-packing breaks existing code | Low | `node_id = 0` preserves all existing behavior. Only new distributed code reads the node_id bits. Existing `ProcessId::next()` continues to produce `0x0000_XXXXXXXXXXXX` values. |
| STF format evolution | Low | Version byte in handshake flags. Both nodes negotiate compatible format version. |
| Cookie security | Medium | Cookie is never sent on the wire (HMAC challenge/response). TLS encrypts the transport. For production, recommend TLS-required mode. |

## Sources

- [Erlang Distribution Protocol Specification (erts v16.2)](https://www.erlang.org/doc/apps/erts/erl_dist_protocol.html) -- Handshake protocol, message framing, capability flags
- [Erlang External Term Format (erts v16.2)](https://www.erlang.org/doc/apps/erts/erl_ext_dist.html) -- ETF type tags and encoding (reference, not target)
- [Erlang Distribution over TLS (ssl v11.5.1)](https://www.erlang.org/doc/apps/ssl/ssl_distribution.html) -- TLS integration for distribution
- [Alternative Node Discovery (erts v16.2)](https://www.erlang.org/doc/apps/erts/alt_disco.html) -- Custom EPMD replacement patterns
- [EEF Security WG: Serialization](https://erlef.github.io/security-wg/secure_coding_and_deployment_hardening/serialisation.html) -- ETF security concerns (RCE via fun deserialization)
- [EEF Security WG: Distribution](https://erlef.github.io/security-wg/secure_coding_and_deployment_hardening/distribution.html) -- Cookie auth limitations, TLS recommendation
- [rustls 0.23 on crates.io](https://crates.io/crates/rustls) -- Current version 0.23.36
- [ractor_cluster](https://github.com/slawlor/ractor) -- Reference for Rust actor distribution patterns (not used as dependency)
- [Coerce-rs](https://github.com/LeonHartley/Coerce-rs) -- Reference for location-transparent ActorRef (not used as dependency)
- Snow codebase: `crates/snow-rt/Cargo.toml` -- current dependency list
- Snow codebase: `crates/snow-rt/src/actor/mod.rs` -- ProcessId, snow_actor_send, scheduler integration points
- Snow codebase: `crates/snow-rt/src/actor/process.rs` -- ProcessId(pub u64) struct, Message layout
- Snow codebase: `crates/snow-rt/src/actor/registry.rs` -- ProcessRegistry pattern (RwLock<FxHashMap>)
- Snow codebase: `crates/snow-rt/src/actor/scheduler.rs` -- ProcessTable type, work-stealing architecture
- Snow codebase: `crates/snow-rt/src/db/pg.rs` -- PgStream enum (TLS abstraction pattern), hand-rolled wire protocol precedent
- Snow codebase: `crates/snow-rt/src/http/server.rs` -- HttpStream enum, TLS server pattern
- Snow codebase: `crates/snow-rt/src/ws/` -- WsStream, hand-rolled frame codec, reader thread pattern

---
*Stack research for: Snow Language Distributed Actor System*
*Researched: 2026-02-12*
