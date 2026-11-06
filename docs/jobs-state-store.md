# Internal jobs state store

`compiler/mesh-rt/src/jobs/` provides the first durable jobs implementation unit.
It owns typed state transitions and an indexed redb database for one application
deployment. Its interface is crate-private. It has no public Mesh API, worker
executor, Raft adapter, external database, or broker.

`model.rs` defines validated values, commands, receipts, and the pure job
transition function. `store.rs` owns database access. The only production
mutation method after initialization is `JobStore::apply(index, &command)`.

## Application contract

`JobStore::open(path, deployment, limits)` opens or initializes the database.
The caller supplies an existing, protected runtime directory. Unix files use
mode `0600`. The store rejects a final-component symlink, opens the actual file
handle without following one on Unix, and synchronizes its parent directory
after initialization. Directory protection and authentication belong to the
runtime adapter. The file lock is local protection, not proof of remote-node
exclusion.

Metadata persists schema version 2, deployment identity, admission limits,
a latest-applied checkpoint, retained-job and queue counts, receipt count,
logical time, and UTC schedule time. Opening with a different deployment,
schema, or limits fails. Opening inventories existing tables before any table
creation. Only a database with no tables can initialize. Missing metadata,
even with otherwise empty tables, or a missing authoritative index fails
without changing the schema. Version 1 is rejected before decoding version 2
fields or creating tables. There is no migration or snapshot installation API.

The future Raft adapter calls `apply` only for committed jobs commands in
increasing log-index order. Index zero and a changed command at the latest
applied index are log corruption and return `LogConflict`. Gaps are allowed
for entries owned by the adapter, such as membership entries. The applied
position covers jobs commands only, not Raft membership or term metadata.

The checkpoint stores only the latest index, command hash, and receipt. Repeating
that index requires an exact hash and returns its receipt. An older index returns
`AlreadyApplied { through }` without applying or verifying that command's body.
This is a coverage result, **not an enqueue receipt or dispatch authority**.
The adapter must verify committed log identity and resume after the checkpoint.
It resolves old client results through `receipt`, not through old log indexes.
Bounded checkpoint storage cannot also verify arbitrary historical log bodies.

The command hash covers the deterministic serde JSON encoding. The schema
version fixes this representation. Changing variant names or encoding requires
a storage migration. Unknown command versions or undecodable persisted data
require recovery, not interpretation as a new request.

`apply` returns `Result<Receipt, Error>`. Policy and typed-command validation
rejections are receipts and advance the checkpoint. They never halt application
because receipt capacity is full. Storage and encoding failures, incompatible
storage, and log corruption halt application. An I/O error can have an uncertain
commit outcome. The adapter must stop application and dispatch, recover or reopen,
and replay the same committed index. It must not generate a new identity or turn
an error into success. Successful commits use `Durability::Immediate`.

## Client streams and retirement

`ClientStream` is a bounded slot number and a nonzero generation. `Register`
opens generation 1 of an unused slot. An open generation can be registered again
to resolve a lost registration response. It does not reset sequence progress.
A slot can open exactly the next generation only after its current generation
is closed. A closed generation and every older generation are rejected forever.
The retained slot row is a generation watermark, so reuse needs no growing list
of retired client IDs. Generations never wrap.

The future adapter allocates slots, durably binds each generation to its owner,
and authenticates registration, requests, retirement, and closure. These numeric
identities grant no authority. This store does not implement an ownership registry.

`Command::enqueue(stream, sequence, jobs)` creates the replayable request.
Sequences start at 1 and must become admitted in contiguous order. Each stream
retains at most `receipts_per_stream` outstanding receipts. A request at the next
sequence with room in the window binds its complete body hash and original
receipt atomically with its job changes. Domain failures, including job capacity,
unknown queues, invalid batches, and duplicates, also bind a negative receipt.
A changed body for a bound sequence returns `RequestConflict`. Repeating the
original body returns the original receipt and index, including negative results.
Neither case adds another receipt row.

Requests outside the admitted window return `NotAdmitted(reason)` and still
advance the committed checkpoint. `WindowFull`, `SequenceGap`, and `UnknownStream`
do not bind a sequence or body and do not promise a permanent negative outcome.
For example, sequence 2 can be admitted after sequence 1 arrives, or after the
client retires enough receipts. A committed non-admission is distinct from a
finalized negative receipt. A client must retain and retry its original body
while its outcome is uncertain. Concurrent proposals for one sequence are ordered
by the committed log. The first admitted body wins. Later changed bodies conflict.
The client serializes sequence admission or retries gaps after their predecessors.

`Retire { stream, through }` is the owner's acknowledgement that it no longer
needs any result through that sequence, including negative results. It atomically
removes those receipt rows and advances `retired_through`. Retirement cannot pass
the last admitted sequence. Repeating an acknowledgement or sending an older
one is harmless. Every sequence at or below the watermark is rejected forever,
even with another body or job ID. Retirement uses no receipt slot and can run
when the window is full. Each retirement removes at most one bounded window.

`Close` requires all admitted receipts to be retired. It fences all remaining
in-flight proposals from that generation, including previously non-admitted
requests. Repeated closure of that generation succeeds until the slot moves to
a newer generation. A client may explicitly abandon an uncertain result by
acknowledging it, but the adapter must never infer acknowledgement from timeout,
disconnect, or age. Sequence exhaustion requires closing and opening a fresh
generation. Generation exhaustion permanently retires that slot. Neither blocks
accepted-work lifecycle.

`receipt(&request)` uses one read transaction and returns `Recorded`, `Conflict`,
`Retired`, or `NotAdmitted`. `Retired` never means that work can be inserted again.
`NotAdmitted` describes this read, not the cancellation of an in-flight proposal.
The future adapter provides a consensus read barrier before using it to resolve
uncertainty. It must not blindly mint a new identity when a response is lost.
`stream(slot)` exposes the durable generation and acknowledgement progress.

This design replaces the architecture document's provisional lifetime receipt
bound before adapter integration. A separate lifecycle reserve would eventually
exhaust. Expiring arbitrary identities would permit duplicate work. Registered
generations and acknowledged contiguous windows bound both results and retirement
history while preserving permanent rejection of old identities.

## Internal command contract

All non-enqueue job operations use `Command::internal(operation)`. Enqueue cannot
use this path. Registration, retirement, and closure are separate commands.
Internal commands use no client receipt window. Completion, cancellation, renewal,
expiration, recovery, and their deterministic rejections therefore advance even
when every client window and retained-job slot is full.

Internal retries at the latest committed index return the checkpoint receipt.
At a new index they are new intents, not transport receipt replay. Claims,
cancellation, retry, and discard recheck revisions. Completion and renewal recheck
the current lease token and deadline. A repeated renewal can extend a still-valid
lease at the current tick. Repeated clock commands can sweep another page.
Configuration and pause commands set desired state. The adapter must not resubmit
an old configuration intent over a newer one to recover a lost response.
It retries the committed index or reads current state and issues a fresh intent.
Long-term exact-result replay for internal operations is not offered.

The adapter authenticates worker ownership and operator permissions before
proposal. A replayed claim receipt describes historical authority. An executor
must check the current lease and suppress duplicate starts. No identifier or
receipt independently authorizes dispatch.

## Durable rows and atomicity

The database has these tables.

| Table | Key and value |
| --- | --- |
| `jobs_meta_v1` | One versioned metadata record and latest checkpoint |
| `jobs_rows_v1` | Job ID to one job, its arguments, and bounded history |
| `jobs_queues_v1` | Queue ID to policy, pause state, active leases, and retained count |
| `jobs_requests_v2` | Stream generation and sequence to hash and original receipt |
| `jobs_streams_v2` | Bounded slot to generation, phase, and sequence watermarks |
| `jobs_unique_v1` | Queue/uniqueness key to job ID |
| `jobs_due_v1` | Queue/UTC schedule timestamp/job ID to job ID |
| `jobs_leases_v1` | Expiration tick/job ID to job ID |
| `jobs_by_queue_v1` | Queue/job ID to job ID |

An operation first computes a bounded change set inside the write transaction.
Only a successful plan writes job rows, queue rows, and affected indexes.
Those writes, any replay receipt, stream watermarks, and the checkpoint commit
in one transaction. No transaction serializes all retained jobs into one value.
No transaction invokes a worker, reads a clock, or calls application code.

`Enqueue` accepts a nonempty batch across configured queues. An unknown queue,
duplicate job ID, duplicate uniqueness key, or capacity rejection discards the
entire planned batch. No member inserts a job, due index, queue index, uniqueness
claim, or changes a queue/job counter. Duplicate members within one batch also
reject the whole batch.

An admitted rejected batch retains its negative request receipt. It reserves no
job IDs or uniqueness keys. A corrected batch uses the next sequence. Reusing
the original sequence replays the rejection or rejects a changed body. A batch
that never entered its stream window instead receives `NotAdmitted`, as above.

Terminal rows remain as tombstones with their arguments. Uniqueness is scoped
to a queue and lasts for the retained lifetime, including terminal jobs. There
is no automatic job reuse or pruning. Receipt retirement is explicit.
Manual retry reuses the job identity and starts a new run.

## Job states and operations

`JobState` distinguishes `Pending`, `Blocked`, `Leased`, and `Terminal`.
Terminal outcomes are `Completed`, `Cancelled`, `Discarded`, and `Exhausted`.
Every successful job change sets `Revision` to its committed index and records
one history entry. Job arguments, worker key, queue, uniqueness key, and original
UTC schedule timestamp remain unchanged.

`ConfigureQueue` creates or changes a concurrency limit and lease duration.
It preserves pause state, active leases, and retained count. `SetPaused` pauses
or resumes an existing queue. Reducing concurrency below the active count leaves
current leases intact and prevents new claims until enough leases drain.

`Claim` selects one job with an expected revision, holder incarnation, and an
optional supported `WorkerKey`. Due-list results are observations; the committed
claim rechecks revision, time, pause state, and concurrency. The worker key must
match the saved name, version, and schema. A missing worker moves the job to
`Blocked(MissingWorker)`. A matching name with a different version or schema
moves it to `Blocked(IncompatibleWorker)`. Neither consumes an attempt or slot.
A later claim with the correct worker can claim a blocked job.

A successful claim increments attempts and the queue's active-lease count.
Its token binds job ID, run generation, committed claim index, and holder
incarnation. The claim index is its fencing value. The receipt must contain a
`Changed` effect with `Leased` state before an executor considers it dispatchable.
A replayed claim receipt describes the original claim, not fresh authority.
The worker adapter must suppress duplicate starts and check current authority.
It binds the holder to the authenticated worker session. Tokens are valid only
within their deployment and do not independently authorize callers.

`Renew` and `Complete` require the current token and an unexpired lease.
Renewal extends the expiration using the current queue policy and never shortens
it. Renewal during cancellation fails. Success completes the job. Failure
schedules exponential backoff, capped by the retry policy, or exhausts the job.
If the next retry deadline exceeds `MAX_TIME`, the job becomes terminally
`Exhausted` and releases its slot. The same rule applies during expiration, so
one unrepresentable retry cannot abort a clock update or strand other expired
jobs. Claims and renewals with unrepresentable lease deadlines still reject.
No completion payload or worker result codec is defined in this unit.

`Cancel` requires the current revision. Pending or blocked work becomes
cancelled immediately. Leased work records cancellation intent and retains its
slot. Success or failure acknowledged after that intent becomes `Cancelled`.
The slot is released only by a valid completion or committed expiration.
A completion that precedes cancellation makes the old cancellation revision
stale. An expired or superseded token cannot release a successor's slot.

`Retry` requires the current revision and a terminal job. It increments the run
generation, resets attempts, and makes the job eligible at the current logical
tick, subject to its original UTC schedule. The stable external effect key for
an execution run is deployment/job/run. Automatic attempts retain that run;
manual retry changes it. `Discard` requires the current revision and refuses
leased jobs. It retains the row and uniqueness claim.

## Time and expiration

`Tick { tick, utc_ms, expire_limit }` carries time through the committed log.
The store never reads host time. Logical ticks cannot decrease. UTC time is
persisted as the maximum committed value, so a backward wall-clock adjustment
cannot make previously due work become not due. A forward UTC jump makes all
eligible schedules due. This unit does not define recurring misfire policy.

Lease duration and retry backoff use logical ticks. Absolute schedules use UTC
milliseconds. The adapter defines the physical duration of one tick and must
persist that deployment configuration outside this store. Across leadership
changes it must establish a committed leadership barrier and safely advance
logical elapsed time before permitting claims. A restarted local monotonic
clock must never be copied directly into `tick`. Downtime does not advance time
unless the adapter commits progress.

A tick examines at most `expire_limit` entries from the lease-expiration index.
It reads the current lease in the same transaction and checks its latest
expiration. An earlier observation cannot revoke an intervening renewal.
Additional pages can use the same tick at new committed indexes. Leases
whose deadline passed but have not reached the sweep remain counted, and their
renewals and completions fail. Capacity becomes available when their expiration
commits. Cancellation intent becomes terminal cancellation on expiration.

A lease is authoritative state, not proof that an isolated process stopped
executing. The future executor remains at least once and needs external-effect
idempotency. The store does not enforce an absolute physical execution cap.

## Bounds and inspection

Text identifiers contain 1 to 96 ASCII letters, digits, dots, underscores, or hyphens.
Client slots are nonzero `u32` values. Generations and request sequences are
nonzero `u64` values. A retirement watermark may be zero.
Worker versions are nonzero `u32` values. A schema ID is an opaque validated
identifier; the worker/compiler adapter supplies and verifies its actual schema
meaning. Encoded arguments are opaque bytes limited to 64 KiB. A batch has at
most 128 members and a serialized command has at most 1 MiB. Constructors and
serde decoding validate bounded value types; command validation also checks
batch, timestamp, and encoded-envelope bounds at construction. If an invalid
typed command nevertheless reaches the committed log, application records its
rejection and advances. The adapter must bound the wire envelope before decoding.

Concurrency is 1 to 1024. Lease duration is 1 to 1,000,000 ticks. Retry policy
permits 1 to 100 attempts with positive base and maximum delays no greater than
1,000,000 ticks. Arithmetic is checked. Times cannot exceed `i64::MAX`.

Default limits are 100,000 retained jobs, 1,024 stream slots, 128 outstanding
receipts per stream, and 1,024 queues. Hard maxima are 1,000,000 jobs, 4,096 slots,
1,024 receipts per stream, and 4,096 queues. Receipt rows never exceed the product
of the stream and window limits. Stream rows never exceed the slot limit.
Applied-index history occupies one checkpoint regardless of command count.
`status()` reports the receipt count, remaining job and queue capacity, applied
position, and time in one read transaction.

Retained jobs and queues remain bounded with no pruning in this unit. Their
admission limits cannot prevent draining accepted work. Retirement and closure
remain possible at full receipt capacity. No lifecycle reserve or per-command
lifetime counter exists.

`inspect(queue, after_job_id, limit)` reads a queue-index page. `due(queue,
after_cursor, limit)` scans at most `limit` due-index entries and excludes jobs
still in logical backoff. It includes blocked work for later compatibility
checks. An empty page can still have a continuation cursor. Cursors are opaque,
queue-specific, and not snapshot promises across mutations. A scheduler must
restart scanning periodically because new due records can appear before its
cursor. Claims always revalidate.

Inspection and due pages permit 1 to 128 entries. `history` returns at most
that page bound from the newest 32 changes retained per job. Older history is
truncated. Jobs, arguments, and uniqueness remain retained. Request receipts
remain until explicit acknowledgement.

## Verification and remaining integration

The original unit failed four tiny-bound regressions. An enqueue and claim
succeeded, repeats and a rejected batch filled command capacity, and completion,
cancellation, renewal, and expiration each returned `AdmissionFull`. The corrected
tests fill a two-receipt window and verify that all four lifecycle operations
advance. Original metadata-row and metadata-table deletion tests also failed,
as did a final-tick sweep of two expired jobs across queues.

The jobs tests retain atomic cross-queue batches, duplicate members, lease
replacement, both renewal and expiration orders, cancellation races, queue
draining, worker compatibility, retention bounds, and bounded history.
Barrier-controlled threads hold stale observations across committed claims and
renewals. Exhaustive three-step sequences cover 343 job-command combinations.
A further 512 replay traces check retirement, generation rejection, single
admission per identity, and receipt bounds. These are bounded executable checks,
not a proof over unbounded executions.

Replay tests cover out-of-order concurrent proposals, changed bodies, lost
responses, negative receipts, full windows, acknowledgement, slot reuse,
restart, and old-log coverage. A sustained test applies more than 1,600 commands
across 64 generations and reads actual table counts. Receipt rows stay at two
or fewer, the slot table stays at one row, and metadata stays at one checkpoint.

The process proof starts the real test binary, commits an enqueue and claim,
flushes an acknowledgement over a pipe, and blocks. The parent kills that
process only after receiving the acknowledgement. Reopening verifies arguments,
lease authority, counters, replay, and expiration. This proves the store
boundary only, not a public queue or cluster failover.

Fault tests wrap redb's real file backend and inject read, write, and sync errors.
They require errors to reach the caller. After reopening they accept either
atomic commit outcome for an uncertain write and replay to one admitted job.
Retirement fault tests require receipt deletion and watermark progress to recover
atomically. Corrupt rows, incompatible metadata, and missing indexes refuse
success. Missing metadata in an otherwise unused store cannot reinitialize it
under another deployment or limit set, and failed open does not change its schema.

The local verification commands are:

```sh
export PATH=/Users/andreyyurlov/.rustup/toolchains/stable-aarch64-apple-darwin/bin:/opt/homebrew/opt/llvm@21/bin:$PATH
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21
export CARGO_TARGET_DIR="$PWD/.tmp/jobs-progress/target"
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
cargo test --locked -p mesh-rt --lib jobs
rustfmt --edition 2021 --check compiler/mesh-rt/src/jobs/*.rs
git diff --check
```

Tests use this checkout's private Cargo target. Local logs are under
`.tmp/jobs-progress/evidence/` and are not release evidence.

Raft integration, authenticated RPC, logical-time leadership handling, worker
execution, compiler serialization, public/operator APIs, recurring schedules,
snapshot creation and installation, job pruning, and the external client
retirement protocol are separate units. No recurring scheduler or snapshot stub
is exposed. The parent owns independent review and workspace/proof gates on the
integrated commit.
