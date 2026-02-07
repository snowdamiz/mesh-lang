# Phase 9: Concurrency Standard Library - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

High-level concurrency abstractions built on the actor primitives from Phases 6-7. Delivers two core behaviors: Service (GenServer equivalent) for stateful server processes with sync call/async cast, and Job (Task equivalent) for async computation with awaitable results. Both fully type-checked with inference. Does NOT add new runtime primitives — builds on existing spawn/send/receive/link/supervisor infrastructure.

</domain>

<decisions>
## Implementation Decisions

### Service (GenServer) API shape
- Claude's discretion on definition style (module callbacks vs actor block extension vs other approach fitting Snow's patterns)
- Functional state management: handlers receive state, return new state (no mutable state)
- Both synchronous call (caller blocks for reply) and asynchronous cast (fire-and-forget) supported
- Generated typed helper functions: defining a Service auto-generates typed functions (e.g., Counter.increment(pid)) that wrap call/cast — callers don't use generic Service.call() directly

### Naming & conventions
- Snow-native names, NOT OTP names:
  - **Service** (not GenServer) for stateful server processes
  - **Job** (not Task) for async computation
- Callback names: **init**, **call**, **cast** (short, clean — not handle_call/handle_cast)
- Claude's discretion on module structure (flat top-level vs namespace grouping)

### Type system integration
- Claude's discretion on whether call and cast use separate message types or a single union type
- **Exhaustiveness enforced**: compiler error if a call/cast message variant has no matching handler arm
- **Per-variant reply types**: each call variant defines its own return type (e.g., GetCount returns Int, GetName returns String) — caller gets back the exact type
- Job.await returns **Result<T, Error>** — Ok(value) on success, Err on crash

### Job (Task) semantics
- Claude's discretion on timeout behavior (required vs optional vs default)
- Claude's discretion on default supervision/linking behavior
- **Job.map included**: Job.map(list, fn) spawns parallel jobs per element and collects results
- Claude's discretion on crash-during-await behavior (Err result vs propagation — should align with the Result<T, Error> return type)

### Claude's Discretion
- Service definition mechanism (module callbacks vs actor block extension)
- Call vs cast message type separation strategy
- Job timeout behavior
- Job supervision/linking defaults
- Crash-during-await semantics
- Module namespace structure (flat vs nested)

</decisions>

<specifics>
## Specific Ideas

- Generated helper functions for Service: defining a Counter service with an Increment call should auto-generate Counter.increment(pid) that callers use directly — not generic Service.call(pid, Increment)
- Callback naming is deliberately short: init/call/cast — mirrors Snow's preference for concise syntax
- Service + Job naming chosen to feel native to Snow rather than borrowed from Erlang/OTP

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 09-concurrency-standard-library*
*Context gathered: 2026-02-06*
