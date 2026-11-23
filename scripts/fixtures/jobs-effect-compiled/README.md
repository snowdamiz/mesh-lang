# Run the compiled Jobs effect candidate

The candidate supports compiled GNU/Linux ARM64 and x86-64 programs with one queue of
1 to 64 runner slots, batch claims, and a Jobs pool capped at one connection.
LISTEN defaults to off. To enable it, set `"listen":true` on a non-drain runner.
With one slot and one application session, claims and lifecycle work share a
connection. With 2 to 64 slots, the batch claim thread and the lifecycle
dispatcher each have a dedicated connection. LISTEN uses a separate connection
in both cases. Local controls can pause, resume, stop, restart, or scale the
configured queue within its initial slot limit.
Use `Jobs.start_effects(pool, config, handler, app_url, operations)` to opt in.
Pass a named top-level `fn(String) -> String` as the third argument. The REPL
and other targets return an unsupported error.

Set `effect_sessions` in the runner config to an integer from one to the
smaller of 16 and the initial slot limit. Omit it to prewarm one session. The
runtime validates the login, each named routine, and an idle reset on every
lane before admitting work. An admission failure closes the sessions opened
so far. Local queue controls leave the prewarmed session count alone.

Use the Jobs database's numeric address in both URLs. The application URL
uses a separate login and verified TLS with SCRAM-SHA-256. Grant that login
`USAGE` on the application schema and `EXECUTE` on the declared routines.
Give each routine's owner permission to call the Jobs fence and to write the
application tables. Apply the fence migration as the Jobs object owner first.
[Database fence and grants](../../../docs/durable-jobs-oban-parity.md#fence-application-writes-with-the-current-jobs-attempt)

Pass a JSON array of operation declarations. Each entry declares a name, a
schema-qualified routine, and an array of extra argument types. The handler
calls `Jobs.effect(name, values)` with a JSON array of values for those types.
The registry accepts up to 16 entries and up to eight arguments per entry.
Supported types are `text`, `int`, and `bool`. Text is limited to 512 bytes
and integers are signed 64-bit values.

```json
[{"name":"first","routine":"compiled_demo.first","args":["text"]}]
```

Define a volatile, scalar `SECURITY DEFINER` function that returns
`boolean`. The runtime supplies the namespace, UUID job ID, bigint run,
integer attempt, and UUID lease token before the declared arguments. Call
`mesh_jobs_effects.fence_v1` before writing in the same transaction. Return
`true` for a new effect and `false` if the operation already committed for
this job and run. Use a durable key that includes the job, run, and operation.
The worker receives the claim context and a private endpoint. The runtime
keeps credentials and the lease token out of worker arguments, bootstrap
frames, and the inherited environment. Run only trusted application code
under the runner UID. Same-UID access to the parent through procfs or
ptrace depends on the host's policy. [Fixture application SQL](application.sql)

Keep routine definitions, schema ownership, and grants under trusted
administrative control while the runner is active. Admission checks selected
role and routine properties once at startup. Audit effective grants and role
memberships separately. Stop the runner before changing the admitted
routines or their privileges, then restart it to repeat admission.

The parent checks out one warm session for one call. It confirms the
autocommit response and runs `DISCARD ALL` before returning a receipt and
releasing the session. The session starts with a three-second statement
timeout and a 250-millisecond lock timeout. An uncertain response or reset
retires that session. The remaining lanes continue serving calls. Once the
final lane retires, the runner stops admitting claims. A confirmed claim
that has yet to pass launch admission goes through the ordinary unstarted
settlement. Active attempts retain their handler and lease limits. An explicit
runner or queue stop still imposes its earliest cutoff.

The pool mutex orders final retirement and logical launch admission. While
the runner holds its state lock, a successful pool capacity check admits the
launch. An already-admitted launch can spawn and register after final
retirement. The empty pool denies every checkout at that point. The runner
settles each attempt after reaping its child and observing relay cleanup.
Restart the runner to restore capacity. A later attempt can reconcile an
unknown outcome by calling the idempotent routine under its own confirmed
claim.

The private request frame carries the worker's absolute five-second
`CLOCK_MONOTONIC` deadline. The relay caps that deadline with its own I/O
and handler deadlines. When all healthy lanes are busy, the relay ends the
checkout wait 100 ms before the resulting cutoff. This leaves time to publish
`Unavailable` if the attempt still has authority. An idle lane can be checked
out immediately within the call deadline. A returning lane, final retirement,
pool closure, or authority change can wake a waiter. The relay also probes
for a disconnected worker every 10 ms.

Each checkout owns its session through autocommit confirmation, full reset
confirmation, and receipt publication. PostgreSQL I/O runs outside the pool
lock. Scheduling delays or socket errors can still prevent delivery of the
receipt. After sending a complete request, a worker that loses the receipt
keeps the outcome uncertain for the rest of the attempt.

Each nonblocking application socket write and receipt publication requires
admission under the attempt's authority mutex. The deadline check under
that mutex admits the operation. The earliest runner or queue grace deadline,
confirmed lease expiry, or handler deadline prevents new admissions. A send
admitted before the cutoff can reach the kernel afterward if the thread is
preempted. A zero-grace runner or queue stop waits for any send already admitted
under the mutex and closes admission before returning. PostgreSQL can still
commit work from an admitted send, subject to the routine's lease fence.

Before attaching a checked-out session, the relay checks the worker socket for
hangup, even if a complete request was buffered. The worker can exit after
the socket check, and a write can be admitted before the supervisor observes
the exit and revokes authority. The supervisor reaps the exact child before
settling the attempt. On relay exit, shutdown makes later writes fail even
while the supervisor holds another descriptor. Receipts already queued
remain readable. If the child sends a complete request and loses the receipt,
the runtime fails the attempt even if the handler reports success.

## Reconcile two commits

Use PostgreSQL 17.11 and the repository's native GNU/Linux Rust and LLVM
toolchain. Run the fixture from the checkout root.

```sh
env -u MESH_TEST_DATABASE_URL PG_BIN_DIR=/path/to/postgresql/bin \
  bash scripts/verify-postgres-jobs.sh --effect-compiled
```

The fixture builds the Mesh program and starts a disposable TLS cluster. The
database cap is five connections. In polling mode, the runner uses the
maintenance pool connection and separate claim and lifecycle connections.
Set `MESH_EFFECT_LISTEN=1` to run the fixture with LISTEN. The dispatcher then
shares one connection for claims, renewals, and settlement, and the listener
uses its own connection. The fixture waits for the subscription scan before
enqueueing. It enables publishing with `SET LOCAL` in the same producer
transaction and requires a notification scan after commit. The LISTEN arm
uses a 60-second fallback interval. The polling arm keeps its 25 ms interval
and exercises local pause and resume. Both modes include one application
session and one producer connection, which also reads the proof tables.

The program calls two declared routines with a handler hold after each
receipt. The fixture checks the exact backend PID, two transaction IDs,
the typed values, and one worker PID. The first routine leaves a session
advisory lock. The second routine checks that the lock is gone. While the
worker is held, the application session must be idle and the job must still
be executing. After release, the fixture checks durable completion and the
reaped child. It then terminates the effect backend during a staged second
transaction. The fixture checks that the old attempt stays retryable after
the handler ignores the missing receipt. A replacement attempt reconciles
the first effect and commits the second one.
[Compiled fixture](main.mpl)

Run the descriptor tests on the same host.

```sh
cargo test --locked -p mesh-rt --lib worker::effect_endpoint
cargo test --locked -p mesh-rt --lib jobs_effect_session
cargo test --locked -p mesh-rt --lib jobs::effects::tests
cargo test --locked -p mesh-rt --lib effect_pool
cargo test --locked -p mesh-rt --lib effect_config
```

Record the exact executable's `posix_spawn` and exec events with the Linux
launch probe in `scripts/fixtures/jobs-effect-transaction/README.md` at
`0ee8756a`. The prior `linux_fresh_worker_startup_phases` receipt indexes the
terminal `wait4` witness (`exact-child-rusage.json`). The
`linux_warm_short_causal_trace` receipt records the order of reap and
settlement submission. For the exact executable, require a successful
consuming wait that reports the exact child's terminal status. Record the
wait and settlement submission on the same monotonic clock and match both
to the runner, child, and durable attempt. [Prior wait and settlement receipts](../../../proof/jobs-parity/qualification-v1.json)

For a comparison, repeat the lost-receipt check with a server commit held
before its response reaches the parent. The staged disconnect above
interrupts a transaction before commit.

## Scope the 4/8/16-lane comparison

Build a short-workload program once with the compiled Jobs effect entry point
and supply the runner JSON at launch. Use the same ELF for the three runs.
Keep one runner and one queue with `"queues":{"default":16}`,
`"claim_batch":true`, and `"listen":false`. Vary only the integer
`"effect_sessions"` across `4`, `8`, and `16`. Keep the same compiled
handler and two declared, separately confirmed fenced writes per job.
Keep the queue limit, workload, poll cadence, and connection settings
fixed. Validate each run's durable write pairs, exact worker reaps, and
observed backend counts.

With one Jobs Pool connection, one dedicated claim connection, one dedicated
lifecycle connection, and one producer connection, the configured live
connection ceilings are 8, 12, and 20 respectively. Use the producer session
for observations during a trial. Keep the frozen LISTEN-off option and report
this as a feasibility comparison using the short-1r-16s workload. The
notification-enabled canonical receipt covers a different configuration.
[Prior parity evidence](../../../docs/durable-jobs-parity-evidence.md#combine-warm-effect-sessions-and-notification-pickup-on-linux)

## Compare LISTEN at 16 slots and eight application sessions

Use the same compiled workload in both arms. Set `"queues":{"default":16}`,
`"claim_batch":true`, `"effect_sessions":8`, and `"poll_ms":250`. With
`"listen":true`, expect 13 live connections. Count one Jobs pool connection
for maintenance, one batch claim connection, one lifecycle connection for
renewals and settlement, one listener, eight isolated application sessions,
and one producer. Use the producer for observations. With `"listen":false`,
expect 12 live connections with the same workload and connection settings.

Publishing is a separate opt-in. In the LISTEN arm, set
`SET LOCAL mesh_jobs.publish_wakeups = on` in each producer transaction before
enqueueing. Leave publishing off in the polling control. In the LISTEN arm,
wait for the runner's initial subscription scan to finish and submit a small
batch in one producer transaction. Check that `notification_scans` advances
and that the jobs settle with both durable effects and exact child reaps.
Record the backend count through the producer before stopping the runner.

## Measure worker THP during a bounded handler hold

The Linux diagnostic accepts only `MESH_DIAGNOSTIC_JOB_DISABLE_THP=1`
with exactly one argument after `argv[0]`. Use the ordinary selector
`--mesh-internal-job-attempt-v1` or the GNU/Linux ARM64 and x86-64 named-effect selector
`--mesh-internal-job-attempt-effect-v1`. The named-effect spawn clears its
environment and forwards only the literal `1` for this diagnostic, alongside
the existing scheduler settings. Other values preserve the inherited policy.

The policy runs at the start of `mesh_rt_init`, ahead of the Mesh arena and
scheduler. On Linux, a C entry point with no Rust argv uses a bounded
`/proc/self/cmdline` read. If the early check cannot read the command line,
the exact C worker selector still applies the policy before bootstrap input.
Pages faulted before the check can keep their original backing. A failed
`prctl` or readback exits with worker bootstrap status 174.

Run the deterministic argument and environment tests on Linux. The selector
test covers both markers on GNU/Linux ARM64 and x86-64 and checks that other Linux targets
reject the named-effect marker.

```sh
cargo test --locked -p mesh-rt --lib jobs::worker::thp_tests::
```

Adapt a copy of the compiled fixture and its driver to accept 1 or 16 slots
at launch. Make the scale-up rejection check request one more slot than the
initial limit. Give each job a separate marker directory. Include the job ID,
worker PID, and process start time in both `committed-1` and `committed-2`.
Read the PID and start time from `/proc/self/stat` in the handler. Keep the
two declared effects and the existing producer session for observations.
Complete each release before the earliest 15-second hold expires. Keep the
handler and lease deadlines in force.

1. Build one ELF for the entire witness. Record its SHA-256, device and inode,
   source revision, toolchain, Linux kernel and THP settings, and CPU and
   memory limits. Start each arm with `THP_enabled: 1` in the launcher. Keep
   one runner, one queue, one warm effect session, one Jobs pool connection,
   batch claims, and LISTEN disabled. Keep the seed, job payloads, poll cadence,
   credentials, and connection limits fixed. Verify the five-connection budget
   through the producer and record the warm application backend PID.
2. At each slot count, run two pairs of control and treatment arms in opposite
   orders. Unset the diagnostic for each fresh control runner and set it to
   `1` for each fresh treatment runner. Run the same ELF under both settings
   and enqueue exactly as many jobs as there are slots. Set each job's
   maximum attempts to one. At runner readiness, open `/proc/<runner-pid>/exe`
   and verify the loaded file's SHA-256, device, and inode against the recorded
   ELF. Match the runner PID and start time around the check. Sample the
   runner process tree and database session counts every 100 ms from the idle
   baseline through teardown.
3. Wait up to twelve seconds after enqueue for every first receipt marker.
   Require one committed stage-one row and an executing attempt for each
   job. Verify that the recorded application backend served those effects and
   is idle during the hold. Match each marker's job ID, PID, and start time to
   a live direct child of the runner. Check `/proc/<pid>/cmdline` for the exact
   named-effect selector as the sole argument after `argv[0]`. Open
   `/proc/<pid>/exe` and verify its SHA-256, device, and inode against the
   recorded ELF. Check the start time again after inspecting procfs. Require
   the two scheduler variables in the control environment and those variables
   plus the literal diagnostic `1` in treatment. Record the environment key
   names and the diagnostic value.
   Require `THP_enabled: 1` in the launcher, both runners, and control workers,
   and `THP_enabled: 0` in treatment workers.
4. Sample the full cohort held at `committed-1` for one second at a 100 ms
   cadence. Check process start times before and after each sample. Create
   `release-1` for every job and wait for all `committed-2` markers. Confirm
   that the same children are still executing the same attempts and that
   each job has two committed effects with distinct transaction IDs. Check
   that the same warm application backend served the second effects and is
   idle again. Sample the second held cohort at the same cadence for one
   second, then create `release-2` for every job. Record both release times.
5. Allow up to ten seconds after `release-2` for settlement. Require one
   completed attempt per job. Require both the spawn and exec trace and the
   consuming-wait and settlement evidence described above for each attempt.
   Before stopping the runner, confirm that none of the recorded PID and
   start-time pairs remains in procfs, including in zombie state.
   Check that the runner's connections
   drain after it stops and that the producer closes its session. Cap each arm
   at 60 seconds, including readiness and teardown. On failure, stop the
   runner, terminate and reap remaining fixture processes within that limit,
   and retain the failed arm.

Use the parity driver's [`process_sample`](../../jobs-parity.py) for canonical
RSS. It follows the runner's live descendants, excludes zombies and recycled
PIDs, and converts `ps` RSS from KiB to bytes. Record `root_rss_bytes`,
`descendant_rss_bytes`, and `rss_bytes` for the idle baseline and both holds.
Use the maximum single `rss_bytes` sample across the whole arm as peak tree
RSS. In both holds, collect per-PID `Rss`, `Pss`, `Private_Dirty`, and
`AnonHugePages` from `/proc/<pid>/smaps_rollup` for attribution. Report PSS
separately to account for shared pages. Record peak process, thread, and
database session counts, with the producer included in the database count.
Use the runner's `wait4` accounting for user and system CPU, including reaped
children. Record CPU per completed job, page faults, context switches, host
load, and memory, swap, and OOM counters. Report elapsed time, throughput,
and pickup p99 alongside the hold duration.

Compare paired worker RSS and `AnonHugePages` deltas at each slot count and
check the runner samples for drift. The held runs measure memory attribution.

## Verify 64-slot occupancy

Configure one runner with `"queues":{"default":64}` and `"claim_batch":true`.
Set the durable queue concurrency to 64. Use `"effect_sessions":8` or
`"effect_sessions":16` and keep the frozen 1r/64s connection cap at 68.
A polling runner uses 12 client connections with eight warm sessions and 20
with 16. LISTEN adds one connection. Count sessions through the producer,
which these totals include.

Enqueue 64 jobs and use the bounded holds above. Require 64 simultaneous
first-receipt markers with distinct live child PID and start-time pairs,
64 executing attempt-one rows, and `queues.default.in_flight == 64` with
`pending_claims == 0` in `Jobs.status`. The runner fills the queue with
successive claims of at most 16 jobs. Check two separate committed
transactions for each job and exact child reaps before settlement. For the
multi-session reset check, give the first routine's advisory lock a distinct
key per application backend and check that each used backend has released
its lock by the first-receipt hold.

The frozen `slow-1r-64s` proof remains pending. Run matched Oban groups with
throughput, pickup p99, CPU per job, peak RSS, process, thread, and connection
gates and durable reconciliation.
[Frozen profile settings](../../../scripts/jobs-parity-profiles.json)
