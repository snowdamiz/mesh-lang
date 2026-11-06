# Built-in server capabilities

This design implements the contract in `server-runtime-expansion.md`. It is a
design contract, not evidence that a feature already exists. The source baseline
is `ab92750ea970cd6db71d5a0c884a5030905b0d4a`.

## Application contract

An application declares named, versioned workers and passes serializable typed
arguments to `Jobs`. Mesh owns the storage directory, scheduler, workers, retry
policy, recovery, and operator commands. Local development needs no service,
connection string, database credentials, or broker installation.

The intended interface has one prepared job value and one submission path.
`Jobs.new(worker, arguments)` creates a typed request. Options set its queue,
scheduled time, retry policy, and uniqueness key. `Jobs.enqueue(request)` and
`Jobs.enqueue_all(requests)` return durable identities and operation receipts.
An atomic batch may span named queues within the application. Inspection,
cancellation, retry, discard, queue pause/resume, and recurring schedules use
the same state owner.

The initial worker declaration uses compiler support and the existing derived
serialization machinery. Jobs must not wait for user macros, a new general
codec language, hot replacement, or a wholesale standard-library rewrite.
Use a `@job` declaration with an explicit stable worker name and version.
Persistence stores versioned names and schema-checked bytes, never closures,
function pointers, PIDs, borrowed resources, or GC addresses. The precise public
syntax is finalized with compiled positive and negative examples in the API
unit. Existing transient `Job` and collection `Queue` APIs remain supported.

## Durable authority

Use one dedicated Jobs Raft group per application deployment. Store its log,
metadata, and indexed job records in runtime-owned redb storage. Standalone mode
is the same machinery with one voter. Cluster mode uses authenticated Mesh
sessions and explicitly established membership. Loss of quorum cannot bootstrap
a fresh one-member group. Capacity control retains its own command domain and
transport budget. Extract the existing durable Raft log mechanics for both
concrete groups and replace the singleton RPC slot with a group registry.

One state-machine application owns job rows, due and lease indexes, queue
counters, uniqueness, request receipts, and schedule cursors. A successful
submission follows durable consensus commit and durable state application.
Storage failure prevents success and dispatch. Replaying an applied command
returns its original result. No worker, operator, or transport path may update
these records independently.

Records are updated individually. Do not reuse the control store's serialized
whole-state blob for retained job history. Reuse concrete storage/transport
mechanics where needed without making the control consensus a Jobs dependency.
All local state, including schema/deployment identity, remains under the
runtime's protected data directory. Never fall back silently to memory.

Private domain types include `JobId`, `WorkerKey`, `EncodedArgs`, `QueueId`,
`RequestId`, `Revision`, `RunGeneration`, `LeaseToken`, `WorkerSession`, and
`JobState`. State distinguishes pending, blocked, leased, and terminal work.
A lease identifies its job, run, generation, and holder incarnation. Terminal
outcomes distinguish completion, cancellation, discard, and exhausted retries.
Validated constructors enforce size and arithmetic bounds at admission.

The first implementation unit owns `compiler/mesh-rt/src/jobs/` and its runtime
module declaration. It establishes the domain transitions and indexed durable
store behind a narrow crate-private API. It is not a completed public queue
until the Raft, worker, compiler, operator, and process-proof units are integrated.

## Execution and recovery

Workers pull bounded claims according to their local capacity and supported
worker/schema versions. Claiming commits before execution starts. The runtime
runs each invocation on an actor and tracks the token to prevent duplicate
starts of one dispatch in the same process. Renewal, completion, expiration,
and cancellation validate the current token and session.

Queue concurrency counts current authoritative leases. Cancellation retains an
active slot until termination is acknowledged or its lease expires. A stale
completion cannot release a successor's slot. Pausing or reducing a limit stops
new claims and permits existing work to drain.

At-least-once execution is explicit. A process may perform an external effect
and die before recording completion. A disconnected or blocked worker may also
outlive its lease. Lease fencing cannot undo external effects or establish an
absolute physical execution cap. Worker context supplies a stable effect key
for an execution run; manual retry starts an explicit new run.

All nondeterministic choices are carried in committed commands. Followers do
not read clocks or invoke user code during application. Use monotonic elapsed
time for lease decisions, with persisted logical progress and a leadership
barrier. Absolute schedules retain UTC instants and documented clock assumptions.
Recurring schedule definition, revision, occurrence identity, and cursor are
durable. Firing inserts the job and advances the cursor atomically.

Transport retries reuse a client-minted durable identity and command digest. A timeout can
mean the command committed, so the API can resolve an uncertain receipt. Receipt
retirement must reject expired identities instead of treating them as new work.
Retention cannot discard active arguments or erase replay protection silently.
Snapshots include every authoritative index, receipt, and schedule cursor.
The first version retains replay records and refuses admission at its configured
bound rather than introducing a new client-session retirement protocol. A later
retention unit must preserve rejection of retired identities. Reclaim commands
recheck the current lease expiry or renewal revision during application; checking
only a token would incorrectly revoke an intervening successful renewal.

## Application database transactions

Jobs requires no application database. An unrelated application transaction does
not share the queue's commit. Direct enqueue inside a callback therefore commits
independently. Atomic queue batches do not change this boundary.

For applications that need business writes and a durable work obligation in one
transaction, add an optional runtime-managed outbox to the application's existing
database. The transaction writes the intent on its borrowed connection; Mesh
publishes it using a stable operation identity and marks delivery after durable
queue acceptance. This adapter does not become the queue's storage backend.

Do not use a timeout followed by an absence check as a substitute. The original
transaction might still commit after the confirmation discarded its job.

## Language and runtime units

The explicit `receive matching` form preserves unmatched arrival order,
mailbox bounds, GC roots, one deadline, and sender wakeups. Matching uses the same
generated pattern decision tree as ordinary cases. Unmatched messages move into
an actor-owned saved queue, are copied once, remain GC roots, and count against
mailbox limits. Guards run outside the mailbox lock, once per tested message.
Reject reentrant receive during a guard. Plain receive and ordinary case retain
their existing exhaustiveness checks.

Map subset patterns, pinned bindings, and bitstring construction/destructuring
must work through lexer, parser, inference, exhaustiveness, MIR, code generation,
formatter, and editor grammars. Bitstrings carry their bit length; byte buffers
alone cannot represent an unaligned remainder. Width arithmetic and reads are
checked before allocation or access.

Direct mutual tail calls use an SCC transformation after monomorphization, with
original entry symbols retained and resource cleanup preserved. This is a bounded
first unit, not a claim of general higher-order tail-call elimination. The
transformation retains reduction checks and clears obsolete GC-visible slots.

Actor stack exhaustion must reach ordinary actor finalization before a guard-page
fault. Prototype compiler entry checks only together with a verified machine-frame
cap, stack probes, rejected or heap-allocated dynamic frames, and bounded runtime
calls. Every call path, including the REPL, needs coverage. A guessed reserve alone
cannot prove containment. Restore the thread limit on coroutine yield. If that
contract cannot be proved on a target, investigate checked pre-prologue entries.
Do not unwind from signal handlers or across native ABI 1 calls. Native callbacks
need an explicit bounded-stack or isolated-execution contract.

## Tooling and application units

References, rename, and definition share a semantic resolution index. It records
declarations and uses with scope and original source spans. Open buffers overlay
closed project files. Rename validates collisions, versions, and UTF-16 edits;
equal-spelled unrelated symbols and strings remain unchanged.

Tests have explicit case metadata, original spans, tags, and fixture scope.
Selection by name, location, and tags occurs before execution. Bounded subprocess
execution isolates asynchronous cases from today's global runtime test state.
Doctests use the same compiler and report diagnostics at their documentation
origins. Coverage instruments Mesh regions/branches, identifies its artifact,
includes unexecuted code, and refuses stale profiles.

REPL bindings store evaluated typed values in rooted runtime slots. Later input
loads those values instead of recompiling their initializers. Captured values,
affine ownership, GC roots, and executable-image lifetime remain valid across
evaluations and reset.

Typed queries decode preserved database values into declared row types and report
NULL, conversion, and schema errors. Named dependent transaction operations and
savepoints use one borrowed connection. Pool checkout inside a transaction must
not masquerade as participation in that transaction.

Live UI includes typed escaped rendering, real HTTP/WebSocket integration, an
embedded browser client, bounded updates, authorized events, and explicit reconnect
behavior. A browser proof must exercise rendered state and events. Existing rooms
and raw WebSockets alone do not satisfy this unit.

Macros operate on hygienic syntax with source origins and explicit expansion
budgets. A bounded pure interpreter avoids unrestricted host-code execution and
cross-compilation dependence. All compiler entry points share expansion semantics.

Hot replacement requires versioned callable indirection, compatible layouts, and
retained code ownership for active frames, closures, callbacks, and heap metadata.
Initially never unload an image and cap retained versions, so no atomic per-call
pin is required. Unloading remains disabled until safe retirement is proven. Incompatible versions
are rejected or use supervised process replacement. Process handoff alone does
not satisfy the requested live module replacement contract.

## Candidate synthesis

Candidate A is the Codex design in worktree `t3-3d81f573`. Candidate B is the
Claude design committed as `ecb4b140` in worktree `t3-fd27da19`. Both were read
in full and screened against the architect skill's design red flags.

The selected base is A because one application-wide log preserves atomic
batches and one writer, and its receipt, effect, and code-lifetime boundaries
are more explicit. Graft B's pull workers, existing serialization/compiler
precedents, and bounded SCC tail-call unit. Do not graft its cross-partition
batch restriction, timeout-based transaction confirmation, exact physical
concurrency claim, fixed stack reserve as proof, or process-only interpretation
of hot replacement. Avoid front-loading unrelated global refactors.

The independent cross-judge agreed on A. Scores, on a five-point scale, were
A 4/4/3/4/3 and B 3/3/4/3/4 for durability, developer simplicity, source fit,
scope/proofs, and complexity/staging respectively. Parent review agrees with that
tradeoff. Additional accepted grafts are B's group-keyed transport and reusable
log, explicit matching receive with a rooted saved queue, and a bounded stack-check
feasibility unit. Keep durable schedules from A; a wall-clock jump uses the chosen
misfire policy rather than automatically requiring an operator. Keep replay
protection after record retention; a primary-key insert alone cannot protect a
pruned job. Do not treat a local volume lock as proof that a same-named node on a
different volume has stopped.

The drafts are retained under `.tmp/server-runtime/design/`. Candidate A's SHA-256
is `b042f5abe7b1159c719e21393223f1b1e941c0ccc8c4d5fafbf3c37b93fd828c`;
candidate B's is `743c8d95c9dd5bbc4557de37467d812ec5dd1da07fd19044cc04a5abd6925709`.
The synthesis passes the source/ownership and document checks. Runtime acceptance
tests remain pending implementation; this is not a runtime verification result.

## Verification

Every unit keeps the acceptance rows in `server-runtime-expansion.md` honest.
Queue proofs include real process death after acknowledgement, atomic rejection,
uncertain-response replay, competing claims, stale completion, cancellation races,
leader loss, minority refusal, snapshot recovery, and storage-error refusal.
Deterministic time drives transition tests; process tests exercise actual runtime
and compiled programs. Store helper tests alone do not qualify the public queue.

Integrate commits sequentially after checking concurrent Mesh changes. Run focused
checks for each unit, then the authoritative workspace and applicable proof gates
on the integrated head. No release, merge, or deployment is implied.
