# Research Summary: Snow v5.0 Distributed Actors

**Project:** Snow compiler -- BEAM-style distributed actor system (multi-node clustering, location-transparent PIDs, remote spawn/send/monitor, binary wire format, node registry, TLS-encrypted inter-node transport)
**Domain:** Distributed actor system for compiled language with existing single-node actor runtime
**Researched:** 2026-02-12
**Overall confidence:** HIGH

## Executive Summary

Adding BEAM-style distribution to Snow's existing actor runtime is architecturally feasible with minimal disruption to the single-node fast path. The key insight is that Snow's ProcessId is already a u64 passed through LLVM intrinsics as i64 -- by encoding a 16-bit node identifier in the upper bits of this u64, PIDs become location-transparent without changing any extern "C" function signatures, LLVM IR, or existing codegen. All existing local PIDs naturally have node_id=0 (the counter starts at 0 and increments), so the encoding change is fully backward compatible with the 1,524 existing tests.

The distribution layer inserts a locality check at the top of `snow_actor_send`: a single bit-shift (`pid >> 48 == 0`) routes local sends through the existing fast path unchanged, while remote sends are dispatched to a per-node NodeSession actor that serializes messages over TLS-encrypted TCP. This follows the same reader-thread-bridge pattern proven in WebSocket (`ws/server.rs`) -- one OS thread per peer connection for blocking I/O, with messages delivered to the NodeSession's mailbox via reserved type tags in the `u64::MAX - N` range.

The wire format is a custom Snow Term Format (STF) rather than Erlang ETF compatibility. Snow's type system (HM-inferred, static types, no atoms/funs/ports) is fundamentally different from Erlang's dynamic terms. A Snow-native format maps directly to Snow's types, avoids ETF's security baggage (RCE via fun deserialization), and requires approximately 300-400 lines of encode/decode. Authentication uses HMAC-SHA256 challenge/response (stronger than Erlang's MD5, using existing `sha2` + `hmac` crates). The entire implementation requires zero new crate dependencies -- all needed cryptographic primitives, TLS infrastructure, and synchronization primitives are already in Cargo.toml from PostgreSQL, HTTP, and WebSocket milestones.

The most critical risks are: (1) message serialization correctness -- Snow's local messages are raw byte copies containing heap pointers that are meaningless across address spaces, requiring a proper serialization layer for remote sends; (2) PID reuse after node restart -- without a creation counter in the PID encoding, restarted nodes can confuse old cached PIDs with new processes; and (3) remote link/monitor propagation -- the current `propagate_exit` only queries the local process table, silently dropping exit signals for remote PIDs. All three have well-understood solutions modeled on BEAM's architecture.

## Key Findings

**Stack:** Zero new dependencies. Reuses rustls 0.23 (TLS), sha2+hmac (HMAC-SHA256 auth), parking_lot (locks), rustc-hash (FxHashMap), crossbeam-channel (message routing). Hand-rolled wire format and discovery, consistent with the project's hand-rolled HTTP parser, PostgreSQL wire protocol, and WebSocket frame codec.

**Architecture:** Distribution router in `snow_actor_send` with `pid >> 48` locality check. One NodeSession actor per peer node following the WebSocket reader-thread-bridge pattern. New `snow-rt/src/dist/` module with 8 files. Only 3 existing files need modification (actor/mod.rs, actor/process.rs, actor/link.rs) with ~50 total lines changed.

**Critical pitfall:** Message serialization cannot simply copy raw bytes across nodes -- heap pointers, closure environments, and collection handles are address-space-local. A proper Snow value serialization format must be built, and closures must be detected and rejected at serialization time.

## Implications for Roadmap

Based on research, suggested phase structure:

### Phase 1: PID Encoding and Wire Format Foundation
**Rationale:** Everything depends on the PID bit-packing scheme and the serialization format. These are independent of networking and can be tested in isolation. Getting the PID encoding right first means all 1,524 existing tests validate backward compatibility before any distribution code is written.

**Delivers:**
- ProcessId bit-packing: 16-bit node_id + 48-bit local_id in existing u64
- `node_id()`, `local_id()`, `is_local()`, `from_remote()` methods on ProcessId
- Snow Term Format (STF) binary encoder/decoder for all Snow types
- Wire format versioning (version byte from day one)
- Closure detection and rejection during serialization
- Unit tests: PID encoding round-trips, STF encode/decode for every Snow type, existing test suite passes with zero regressions

**Addresses features:** Binary wire format, location-transparent PID encoding
**Avoids pitfalls:** Pitfall 1 (ABI break), Pitfall 2 (message serialization), Pitfall 6 (PID reuse via creation counter), Pitfall 9 (wire format versioning), Pitfall 12 (closure serialization)

**Estimated complexity:** High. The STF encoder/decoder must handle all Snow types including recursive structures (List of Maps of Strings). The PID encoding must be validated against all existing tests.

---

### Phase 2: Node Connection and Authentication
**Rationale:** With PIDs and wire format ready, establish the networking layer. This phase focuses purely on node-to-node connectivity: TCP/TLS streams, challenge/response handshake, and connection lifecycle. No actor integration yet -- just transport.

**Delivers:**
- DistStream enum (Plain/Tls) following PgStream/HttpStream/WsStream pattern
- Cookie-based HMAC-SHA256 challenge/response handshake
- Node naming (`name@host:port` format) and identity (creation counter)
- Built-in node registry (EPMD equivalent) for name resolution
- TLS encryption using existing rustls 0.23 infrastructure
- Framed message reading/writing (4-byte length prefix)
- Heartbeat tick on dedicated OS thread (60s interval, 15s timeout)

**Addresses features:** Node naming/identity, cookie authentication, TLS transport, node registry, heartbeat
**Avoids pitfalls:** Pitfall 8 (O(N^2) mesh -- start with explicit connections), Pitfall 11 (no auth -- HMAC-SHA256 from day one), Pitfall 13 (heartbeat on scheduler -- dedicated thread)

**Estimated complexity:** High. Handshake protocol has multiple failure modes. TLS integration reuses proven patterns but needs both client and server roles (unlike previous milestones which were one or the other).

---

### Phase 3: Remote Send and Distribution Router
**Rationale:** The core value proposition: `send(pid, msg)` works transparently for remote PIDs. This phase wires the PID locality check into `snow_actor_send` and creates the NodeSession actor that manages per-peer message flow.

**Delivers:**
- Distribution router: locality check in `snow_actor_send` (branch-free for local path)
- NodeSession actor (one per peer, reader-thread-bridge pattern)
- NodeRegistry dual-map (by_name + by_id)
- Reserved type tags for distribution signals (u64::MAX-10 through u64::MAX-19)
- Remote message delivery with per-sender FIFO ordering (guaranteed by TCP)
- Remote send by registered name (`send({name, node}, msg)`)
- `Node.connect()`, `Node.self()`, `nodes()` API functions

**Addresses features:** Location-transparent PIDs, remote message delivery, remote send by name, node listing
**Avoids pitfalls:** Pitfall 5 (local perf regression -- branch-free check), Pitfall 14 (process table pollution -- remote PIDs never in local table)

**Estimated complexity:** High. NodeSession actor is the main workhorse. Reader thread bridge follows WebSocket pattern but handles more message types (SEND, LINK, EXIT, MONITOR, SPAWN). Must benchmark local send path to verify zero regression.

---

### Phase 4: Remote Links, Monitors, and Failure Handling
**Rationale:** Distribution without fault tolerance is dangerous. Remote links and monitors complete the safety story: supervisors can detect remote child crashes, and network partitions trigger `:noconnection` exit signals.

**Delivers:**
- Remote link protocol (LINK/UNLINK wire messages)
- Remote process monitoring (MONITOR/DEMONITOR/MONITOR_EXIT)
- Node monitoring (`:nodedown`/`:nodeup` events)
- Connection loss handling: fire exit signals for all remote links, `:DOWN` for all monitors, `:nodedown` for all node monitors
- Creation counter validation (reject stale PIDs from restarted nodes)
- Extended `propagate_exit` to route exit signals over the network for remote PIDs

**Addresses features:** Remote process monitoring, node monitoring, connection loss handling
**Avoids pitfalls:** Pitfall 7 (silent link breakage), Pitfall 6 (PID reuse -- creation counter validation)

**Estimated complexity:** High. Failure handling has many edge cases: race between monitor setup and process death, simultaneous node crashes, link cycles across nodes. Must test with simulated network partitions.

---

### Phase 5: Remote Spawn and LLVM Integration
**Rationale:** Remote spawn requires a function name registry (new concept for Snow -- currently functions are identified by pointer, not name). This is the final core feature. LLVM intrinsic registration wires everything into the Snow language.

**Delivers:**
- Function name registry: maps Snow module+function names to entry point pointers
- `Node.spawn(node, function, args)` and `Node.spawn_link(node, function, args)`
- SPAWN_REQUEST/SPAWN_REPLY wire protocol
- All new LLVM intrinsics registered in intrinsics.rs, builtins.rs, lower.rs
- `Node.start()`, `Node.connect()`, `Node.self()`, `Node.spawn()`, `Node.monitor()` Snow-level API
- E2E tests: multi-node spawn, remote send, monitor, link, partition handling

**Addresses features:** Remote spawn, LLVM integration
**Avoids pitfalls:** Pitfall 12 (closure serialization -- named functions only for remote spawn)

**Estimated complexity:** High. Function name registry is a new concept. Remote spawn must handle: function not found on remote node, serialization failure for args, remote node disconnects during spawn.

---

### Phase 6: Distributed Global Registry (Differentiator)
**Rationale:** Cluster-wide name resolution enables service discovery without passing PIDs. This is the highest-level feature, building on all previous phases.

**Delivers:**
- Cluster-wide name registry (one name -> one PID across all nodes)
- Coordinator-based registration (broadcast to all nodes, first-writer-wins)
- Automatic cleanup on `:nodedown` (remove registrations from disconnected node)
- `Global.register(name, pid)`, `Global.whereis(name)`, `Global.unregister(name)`

**Addresses features:** Distributed global process registry
**Avoids pitfalls:** Pitfall 3 (split-brain -- first-writer-wins with explicit conflict resolution, no automatic partition healing)

**Estimated complexity:** High. Distributed coordination is inherently complex. Defer process groups (pg equivalent) to v5.1.

---

### Phase 7: Cross-Node Integration (Differentiator)
**Rationale:** Integration with existing WebSocket rooms and supervision trees. These are the features that make Snow's distribution unique -- combining WS + actors + distribution in a single language.

**Delivers:**
- Cross-node WebSocket room broadcast (extend RoomRegistry to route across nodes)
- Cross-node supervision integration (supervisors monitoring remote children)

**Addresses features:** Cross-node WS rooms, cross-node supervision
**Avoids pitfalls:** Pitfall 10 (supervisor restart timing -- async restart, separate `:noconnection` handling)

**Estimated complexity:** Medium. Builds on solid foundation from phases 1-5. WS room extension follows existing broadcast pattern. Supervision integration requires careful handling of network latency in restart rate limiting.

---

### Phase Ordering Rationale

**Dependency chain:**
- Phase 1 (PID + wire format) is the foundation -- nothing else can start without it
- Phase 2 (connections) requires wire format for handshake framing
- Phase 3 (remote send) requires both PID encoding and connections
- Phase 4 (links/monitors) requires remote send infrastructure
- Phase 5 (remote spawn) requires all previous phases plus function name registry
- Phases 6-7 are highest-level features requiring everything below to be solid

**Risk mitigation:**
- Critical pitfalls (1, 2, 5, 6) are addressed in Phase 1 before any networking code
- Security (Pitfall 11) is addressed in Phase 2, not deferred
- Fault tolerance (Pitfall 7) is addressed in Phase 4, before remote spawn adds more failure modes
- Split-brain (Pitfall 3) is explicitly deferred to Phase 6 with a simple coordinator approach

**Incremental value:**
- Phase 1: Validated PID encoding + tested wire format (no runtime behavior change)
- Phase 2: Nodes can connect and authenticate (foundation for all cross-node features)
- Phase 3: `send(remote_pid, msg)` works -- the core distribution feature
- Phase 4: Distribution is safe -- links, monitors, and partition handling work
- Phase 5: Full distribution API accessible from Snow code
- Phases 6-7: Advanced features for production distributed applications

**Research flags for phases:**
- **Phase 1 (PID + Wire Format):** Needs validation research during planning -- must verify that STF encoder handles all Snow types correctly, including nested ADT variants and generic type instances. The interaction between type_tag derivation and STF encoding needs careful design.
- **Phase 3 (Remote Send):** May need research on NodeSession actor design -- mailbox ordering, flow control, backpressure for slow remote nodes. Prototype the reader-thread-bridge for distribution to verify it handles the higher message diversity (SEND, LINK, EXIT, MONITOR, SPAWN) compared to WebSocket (TEXT, BINARY, CLOSE, PING).
- **Phase 4 (Failure Handling):** Needs research on race conditions -- what happens when a monitor request is in flight while the target process exits? BEAM has well-defined semantics for these races; Snow must match them.
- **Phase 6 (Global Registry):** Likely needs deeper research on distributed coordination. The first-writer-wins approach is simple but may not handle all edge cases. Study Erlang's `global` module failure modes before implementation.

**Phases with standard patterns (likely skip research-phase):**
- **Phase 2 (Connections):** TLS + TCP + handshake follows established patterns from PG, HTTP, WS. Cookie auth is well-specified.
- **Phase 5 (Remote Spawn + LLVM):** Mechanical wiring of established patterns. Function registry is new but straightforward.
- **Phase 7 (Cross-Node Integration):** Extends existing patterns (rooms, supervision) with distribution routing.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Zero new dependencies. All crates verified in existing Cargo.toml/Cargo.lock. Hand-rolled approach follows established project pattern (PG wire protocol, HTTP parser, WS frame codec). |
| Features | HIGH | BEAM distribution protocol is exhaustively documented by Ericsson. Feature set cross-referenced with Erlang/OTP, Akka, Orleans, Proto.Actor. Table stakes vs differentiators clearly separated with dependency graph. |
| Architecture | HIGH | Based on direct analysis of all relevant snow-rt modules (actor/mod.rs, process.rs, scheduler.rs, link.rs, registry.rs, heap.rs, ws/server.rs, ws/rooms.rs). PID encoding impact analyzed for every existing code path. Distribution patterns follow proven reader-thread-bridge from WebSocket. |
| Pitfalls | HIGH | 16 pitfalls documented with severity (7 critical, 5 moderate, 4 minor). Each verified against Snow's specific architecture (not generic distributed systems advice). Sources include Erlang distribution protocol docs, academic papers on distributed actor GC, real-world post-mortems from Akka/Proto.Actor/Swift distributed actors. |

## Gaps to Address

### Resolved During Research
- **PID representation strategy:** Resolved as 16-bit node_id + 48-bit local_id bit-packing in existing u64. Backward compatible.
- **Wire format approach:** Resolved as custom STF (not ETF). Maps directly to Snow's type system.
- **Authentication:** Resolved as HMAC-SHA256 challenge/response. Stronger than BEAM's MD5, using existing crates.
- **Node discovery:** Resolved as built-in (no separate EPMD daemon). Follows single-binary philosophy.

### Open Questions for Phase Planning
1. **Creation counter bit allocation:** The PITFALLS.md suggests 16-bit node_id + 8-bit creation + 40-bit local_id, while ARCHITECTURE.md uses 16-bit node_id + 48-bit local_id without explicit creation bits. Need to finalize: embed creation in PID (reduces local_id space) or track creation separately in NodeRegistry. BEAM tracks creation separately from PID bits in newer versions.
2. **Function name registry for remote spawn:** Snow currently identifies functions by pointer, not name. Need to design a mapping from Snow module+function names to entry point addresses that works across different compiled binaries (which may have different LLVM optimization layouts). This is a new concept requiring Phase 5 research.
3. **STF encoding for generic types:** Snow's monomorphization means `List<Int>` and `List<String>` are different types at runtime. Does the wire format need to encode the concrete type, or can the receiver infer it from context? Needs Phase 1 design work.
4. **Backpressure for remote send:** If a remote node is slow to consume messages, the local NodeSession's outgoing buffer grows unboundedly. Need a flow control strategy (bounded buffer with backpressure, drop policy, or TCP backpressure via write blocking). Address during Phase 3.
5. **Testing strategy for distributed scenarios:** Need a test harness that can start multiple Snow runtimes on localhost with different ports. Deterministic simulation testing (DST) is ideal but complex; may start with integration tests using real TCP on localhost. Address during Phase 1 test infrastructure setup.

None of these gaps block starting Phase 1. All are addressable during phase-specific planning.

## Sources

### Primary (HIGH confidence)
- [Erlang Distribution Protocol (erts v16.2)](https://www.erlang.org/doc/apps/erts/erl_dist_protocol.html) -- Handshake protocol, message framing, capability flags
- [Erlang External Term Format (erts v16.2)](https://www.erlang.org/doc/apps/erts/erl_ext_dist.html) -- Type tags and binary encoding reference
- [Distributed Erlang System Documentation v28.3.1](https://www.erlang.org/doc/system/distributed.html) -- Node naming, connection, PIDs, remote spawn
- [Erlang Distribution over TLS (ssl v11.5.1)](https://www.erlang.org/doc/apps/ssl/ssl_distribution.html) -- TLS integration for distribution
- [Erlang Processes Reference Manual v28.3.1](https://www.erlang.org/doc/system/ref_man_processes.html) -- Monitors, links, signal ordering guarantees
- [Erlang Blog: Message Passing](https://www.erlang.org/blog/message-passing/) -- Ordering guarantees, remote delivery, noconnection semantics
- [EEF Security WG: Serialization](https://erlef.github.io/security-wg/secure_coding_and_deployment_hardening/serialisation.html) -- ETF security concerns
- [EEF Security WG: Distribution](https://erlef.github.io/security-wg/secure_coding_and_deployment_hardening/distribution.html) -- Cookie auth limitations, TLS recommendation
- Snow codebase: all actor runtime modules, WebSocket modules, codegen intrinsics (direct file analysis)

### Secondary (MEDIUM confidence)
- [Programming Distributed Erlang Applications: Pitfalls and Recipes (Svensson, Fredlund)](https://dl.acm.org/doi/10.1145/1292520.1292527) -- PID reuse, naming, semantic differences
- [Akka Location Transparency](https://doc.akka.io/libraries/akka-core/current/general/remoting.html) -- Transparent routing, actor paths
- [Orleans Overview (Microsoft Learn)](https://learn.microsoft.com/en-us/dotnet/orleans/overview) -- Virtual actor model reference
- [ractor_cluster](https://docs.rs/ractor_cluster/latest/ractor_cluster/) -- Erlang-style cluster protocol in Rust
- [Proto.Actor Location Transparency](https://proto.actor/docs/location-transparency/) -- PID-based routing patterns
- [Distribunomicon (Learn You Some Erlang)](https://learnyousomeerlang.com/distribunomicon) -- Practical distribution pitfalls

### Tertiary (LOW confidence)
- [Deterministic Simulation Testing](https://www.risingwave.com/blog/deterministic-simulation-a-new-era-of-distributed-system-testing/) -- DST overview for testing strategy
- [FoundationDB's Simulation Testing](https://alex-ii.github.io/notes/2018/04/29/distributed_systems_with_deterministic_simulation.html) -- DST reference

---
*Research completed: 2026-02-12*
*Ready for roadmap: yes*
