# Phase 6: Actor Runtime - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Lightweight actor processes with typed message passing, a work-stealing scheduler, and per-actor isolation, integrated into compiled Snow programs. Actors can be spawned, send/receive messages, link to each other, and are preemptively scheduled across CPU cores. Supervision trees and restart strategies are Phase 7. High-level abstractions (GenServer, Task) are Phase 9.

</domain>

<decisions>
## Implementation Decisions

### Actor spawning & lifecycle
- Dedicated `actor` keyword block syntax — actors are a first-class language construct, not just spawned closures
- Recursive functional state (Erlang-style) — state passed as argument, updated by calling self with new state. No mutable state bindings.
- Terminate callback supported — actors can define cleanup logic that runs before full termination (resource cleanup, final messages)

### Message passing semantics
- Strict FIFO mailbox — messages processed in arrival order, no selective receive
- Typed messages checked at compile time via Pid typing

### Scheduling & preemption
- Reduction counting (BEAM-style) — each actor gets N reductions before being preempted. Ensures fairness even with tight loops.
- M:N threading — one OS thread per CPU core, actors multiplexed across cores with work-stealing for load balancing
- Basic priority levels — high/normal/low. High-priority actors scheduled first. Useful for system-level actors.
- Crash behavior: actor killed, exit reason propagated to linked processes. No global impact.

### Typed actor identity (Pid)
- Named process registration — `register(pid, :name)` for global lookup by atom/string
- `self()` returns the actor's own Pid — essential for reply-to patterns in messages
- Untyped `Pid` allowed as escape hatch — Pid without type parameter permitted but requires runtime type check on send. Enables heterogeneous collections of actors.

### Claude's Discretion
- Normal exit behavior (whether return value is accessible to parent/linked process, or silent cleanup)
- Send semantics (fire-and-forget only vs both send and call primitives at the base level)
- Receive timeout support (after clause vs separate timer mechanism)
- Unmatched message handling (consistent with strict FIFO — likely crash or drop-with-warning)
- Pid typing strategy (Pid<M> single message type vs Pid<Protocol> — based on what Snow's current type system supports)

</decisions>

<specifics>
## Specific Ideas

- Actor block as first-class construct suggests syntax like `actor MyCounter do ... end` with dedicated receive/state semantics
- BEAM-style reduction counting implies the compiler must instrument code with reduction decrement + yield check at call sites and loop back-edges
- Work-stealing scheduler across cores is the full BEAM model — this is the highest engineering risk in the project
- Link notification on crash means Phase 6 must implement basic process linking (not just Phase 7 supervision)

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 06-actor-runtime*
*Context gathered: 2026-02-06*
