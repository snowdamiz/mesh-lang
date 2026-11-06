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

Metadata persists schema version 1, deployment identity, admission limits,
applied command position, retained-job and queue counts, logical time, and UTC
schedule time. Opening with a different deployment, schema, or limits fails.
A missing authoritative table fails instead of silently rebuilding an empty
index. There is no migration or snapshot installation API in this unit.

The future Raft adapter calls `apply` only for committed jobs commands, in
increasing log-index order. Index zero is invalid. Gaps are allowed for entries
owned by the adapter, such as membership entries. The returned applied position
covers jobs commands only. It is not a replacement for Raft membership or term
metadata. Reapplying a recorded index requires the exact command hash. An
unrecorded old index or a changed command at an old index returns `LogConflict`.

A command contains `ClientId`, `RequestId`, and a typed `Operation`. Its SHA-256
hash covers the deterministic serde JSON representation of all three fields.
The schema version fixes this representation. Renaming enum variants or changing
their encoding requires a storage migration. The adapter must authenticate the
client identity and must not infer authorization from the identifier alone.

`apply` returns `Result<Receipt, Error>`. A receipt contains the original command
index and `Result<Effect, Rejection>`. A domain rejection is durably recorded and
advances the applied position. A storage, encoding, invalid-envelope, log
conflict, or admission-full error does not report acceptance.

Repeating the same client/request identity and body returns the original
receipt, including its original index and rejection. A different body returns
`RequestConflict` without replacing the original receipt. Repeated commands at
new log positions record their positions but never repeat the state change.
`receipt(&command)` resolves a previously uncertain result and checks the hash.

A storage error can have an uncertain commit outcome. The adapter must stop
application and dispatch, reopen or recover the database, and replay the same
index and identity. It must not turn the error into success or generate a new
request identity. Successful application uses `Durability::Immediate`; no
in-memory fallback exists.

## Durable rows and atomicity

The database has these tables.

| Table | Key and value |
| --- | --- |
| `jobs_meta_v1` | One bounded metadata record |
| `jobs_rows_v1` | Job ID to one job, its arguments, and bounded history |
| `jobs_queues_v1` | Queue ID to policy, pause state, active leases, and retained count |
| `jobs_requests_v1` | Client/request to command hash and original receipt |
| `jobs_applied_v1` | Committed index to hash and receipt |
| `jobs_unique_v1` | Queue/uniqueness key to job ID |
| `jobs_due_v1` | Queue/UTC schedule timestamp/job ID to job ID |
| `jobs_leases_v1` | Expiration tick/job ID to job ID |
| `jobs_by_queue_v1` | Queue/job ID to job ID |

An operation first computes a bounded change set inside the write transaction.
Only a successful plan writes job rows, queue rows, and affected indexes.
Those writes, the replay receipt, the applied-index row, and metadata commit in
one transaction. No transaction serializes all retained jobs into one value.
No transaction invokes a worker, reads a clock, or calls application code.

`Enqueue` accepts a nonempty batch across configured queues. An unknown queue,
duplicate job ID, duplicate uniqueness key, or capacity rejection discards the
entire planned batch. No member inserts a job, due index, queue index, uniqueness
claim, or changes a queue/job counter. Duplicate members within one batch also
reject the whole batch.

Rejected batches retain a negative request receipt and an applied-index record.
This is the interpretation of replay protection in v1. These records consume a
command slot; they do not reserve job IDs or uniqueness keys. A corrected batch
uses a new request identity. Reusing the rejected identity returns its original
rejection. Thus “no dedupe claims on rejection” applies to job admission and
uniqueness, not to the durable record of the rejected command.

Terminal rows remain as tombstones with their arguments. Uniqueness is scoped
to a queue and lasts for the retained lifetime, including terminal jobs. There
is no automatic reuse, pruning, or receipt expiry. Manual retry reuses the job
identity and starts a new run.

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
Additional pages can use the same tick with fresh request identities. Leases
whose deadline passed but have not reached the sweep remain counted, and their
renewals and completions fail. Capacity becomes available when their expiration
commits. Cancellation intent becomes terminal cancellation on expiration.

A lease is authoritative state, not proof that an isolated process stopped
executing. The future executor remains at least once and needs external-effect
idempotency. The store does not enforce an absolute physical execution cap.

## Bounds and inspection

Identifiers contain 1 to 96 ASCII letters, digits, dots, underscores, or hyphens.
Worker versions are nonzero `u32` values. A schema ID is an opaque validated
identifier; the worker/compiler adapter supplies and verifies its actual schema
meaning. Encoded arguments are opaque bytes limited to 64 KiB. A batch has at
most 128 members and a serialized command has at most 1 MiB. Constructors and
serde decoding validate bounded value types; command validation also checks
batch, timestamp, and encoded-envelope bounds before opening a transaction.

Concurrency is 1 to 1024. Lease duration is 1 to 1,000,000 ticks. Retry policy
permits 1 to 100 attempts with positive base and maximum delays no greater than
1,000,000 ticks. Arithmetic is checked. Times cannot exceed `i64::MAX`.

Default retention limits are 100,000 jobs, 1,000,000 command positions, and
1,024 queues. Hard configuration maxima are 1,000,000 jobs, 10,000,000 command
positions, and 4,096 queues. `status()` reports remaining capacity and time in
one read transaction. Domain rejections and transport repeats at new committed
positions consume command capacity. Replaying the same recorded position does
not.

The command bound is a hard lifetime bound. At exhaustion, new application
positions fail with `AdmissionFull`, including lifecycle commands and ticks.
Reads and existing-index replay still work. The proposer must account for
in-flight reservations and stop admission before committing beyond this bound.
The integration must budget lifecycle capacity. There is no online limit change
or safe receipt-retirement protocol yet; this finite-lifetime constraint must
not be hidden by a public API.

`inspect(queue, after_job_id, limit)` reads a queue-index page. `due(queue,
after_cursor, limit)` scans at most `limit` due-index entries and excludes jobs
still in logical backoff. It includes blocked work for later compatibility
checks. An empty page can still have a continuation cursor. Cursors are opaque,
queue-specific, and not snapshot promises across mutations. A scheduler must
restart scanning periodically because new due records can appear before its
cursor. Claims always revalidate.

Inspection and due pages permit 1 to 128 entries. `history` returns at most
that page bound from the newest 32 changes retained per job. Older history is
truncated; jobs, arguments, uniqueness, and replay receipts are not.

## Verification and remaining integration

The initial focused test failed to compile because the store types did not
exist. After implementation it verified that an accepted job and its exact
arguments, identity, state, and replay receipt survive reopening.

The jobs tests exercise atomic cross-queue batches, duplicate members, identity
conflicts, lease replacement, both renewal/expiration orders, cancellation races,
queue draining, worker compatibility, retention bounds, and bounded history.
Barrier-controlled threads hold stale observations across committed claims and
renewals. Exhaustive three-step sequences cover 343 command combinations and
check stored counters against authoritative lease rows after each step.

The process proof starts the real test binary, commits an enqueue and claim,
flushes an acknowledgement over a pipe, and blocks. The parent kills that
process only after receiving the acknowledgement. Reopening verifies arguments,
lease authority, counters, replay, and expiration. This proves the store
boundary only, not a public queue or cluster failover.

Fault tests wrap redb's real file backend and inject read, write, and sync errors.
They require errors to reach the caller. After reopening they accept either
atomic commit outcome for an uncertain write and replay to one admitted job.
Corrupt rows, incompatible metadata, and missing indexes also refuse success.

The local verification commands are:

```sh
export PATH=/Users/andreyyurlov/.rustup/toolchains/stable-aarch64-apple-darwin/bin:/opt/homebrew/opt/llvm@21/bin:$PATH
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21
export CARGO_TARGET_DIR="$PWD/.tmp/jobs-state-store/target"
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
cargo test --locked -p mesh-rt jobs
cargo fmt --all -- --check
git diff --check
```

Tests use this checkout's private Cargo target. Local logs are under
`.tmp/jobs-state-store/evidence/` and are not release evidence.

Raft integration, authenticated RPC, logical-time leadership handling, worker
execution, compiler serialization, public/operator APIs, recurring schedules,
snapshot creation/install, and safe retention are separate units. No recurring
scheduler or snapshot stub is exposed. The parent owns independent review and
workspace/proof gates on the integrated commit.
