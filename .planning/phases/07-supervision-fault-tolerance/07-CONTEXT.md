# Phase 7: Supervision & Fault Tolerance - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

OTP-style supervision trees with restart strategies and let-it-crash semantics. Supervisors monitor child actors, automatically restart them on failure according to configurable strategies, and escalate when restart limits are exceeded. Supervision trees are nestable from day one. GenServer and other high-level abstractions are Phase 9.

</domain>

<decisions>
## Implementation Decisions

### Supervisor syntax & API
- Claude's discretion on whether supervisors use a dedicated `supervisor` block, function-based API, or actor-with-trait approach — pick what fits Snow's existing actor syntax best
- Supervision trees nestable from the start — a supervisor can supervise other supervisors
- Children start sequentially in declared order (Erlang style) — if one fails to start, remaining children are not started and supervisor fails
- Claude's discretion on supervisor naming (auto-registered vs explicit) — should be consistent with Phase 6 actor naming via ProcessRegistry

### Restart strategies & limits
- All four OTP strategies: one_for_one, one_for_all, rest_for_one, simple_one_for_one
- Restart limits follow Erlang model: max_restarts within max_seconds (e.g., 3 restarts in 5 seconds)
- Supervisor terminates when restart limit exceeded — propagates up to parent supervisor (standard OTP escalation)
- Per-supervisor configurable limits with sensible defaults (Erlang defaults: 3 restarts in 5 seconds)

### Crash propagation & trap_exit
- Claude's discretion on trap_exit mechanism — supervisors must trap exits automatically, regular actors can opt-in
- Structured ExitReason sum type: Normal, Shutdown, Custom(String) — typed and pattern-matchable at compile time
- Fresh Pid on restart (Erlang style) — old Pid references become stale, named registration handles lookup
- Erlang exit-to-restart semantics: permanent children restart on any exit (normal or abnormal), transient only on abnormal, temporary never

### Child specification
- Claude's discretion on child spec representation — should leverage Snow's type system for compile-time validation
- All three restart types: permanent (always restart), transient (restart on abnormal), temporary (never restart)
- Configurable shutdown per-child: timeout in ms or brutal_kill, with sensible default (e.g., 5000ms)
- Full compile-time validation of child specs — supervisor knows child message types, mismatched start functions or invalid specs caught at compile time

### Claude's Discretion
- Supervisor definition syntax (dedicated block vs function-based vs trait-based)
- Supervisor naming policy (auto vs explicit registration)
- trap_exit mechanism design
- Child spec struct/representation design
- Shutdown signal implementation details
- simple_one_for_one dynamic child management internals

</decisions>

<specifics>
## Specific Ideas

- Follow Erlang/OTP conventions closely — Snow supervision should feel familiar to anyone who knows OTP
- ExitReason as a proper sum type leverages Snow's pattern matching and type system strengths
- Phase 6 already has process linking, exit signal propagation (EXIT_SIGNAL_TAG = u64::MAX), and ProcessRegistry — build on these directly

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 07-supervision-fault-tolerance*
*Context gathered: 2026-02-06*
