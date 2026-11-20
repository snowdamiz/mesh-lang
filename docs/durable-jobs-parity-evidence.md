# Run the PostgreSQL Jobs comparison

The [parity plan](durable-jobs-oban-parity.md) defines which evidence is required
for each phase. The comparison driver uses the Mesh executable built from a
Git commit and the pinned Oban 2.24.2 Basic fixture. Commit changes to the
driver and fixtures before running comparisons. The driver archives them
from the current Git head, independently of the selected Mesh runtime.

## Run correctness checks

Install Rust, LLVM 21, Python 3.11 or later, Elixir 1.18 or later, Erlang,
and the PostgreSQL server tools. Include the PostgreSQL `pg_stat_statements`
extension. Set `PG_BIN_DIR` if those tools are outside your shell's `PATH`.

```sh
bash scripts/verify-postgres-jobs.sh --parity-driver-check
python3 scripts/jobs-parity.py --smoke --mesh-ref 59f6a7b3
python3 scripts/jobs-parity.py --faults --mesh-ref 59f6a7b3
```

The correctness script runs the runtime tests and compiled process proof.
The compiled proof uses both poll-only and `listen: true` runners. A loopback
relay pauses a listener reconnect while leaving the claim and handler sessions
available. Run with `--compatibility` to cover the same paths over verified TLS
and SCRAM. To compare sparse pickup behavior with a pinned reference, build
the `mesh-rt` static library from `7c162056` and set
`MESH_JOBS_REFERENCE_RT_LIB_PATH` to its absolute path before running the
verifier. It builds the reference fixture from the same pinned revision and
checks the same committed job with polling on the reference and notifications
on the candidate. Set `MESH_JOBS_REFERENCE_MESHC_PATH` to the absolute path
of the pinned `meshc` to use its compiler as well. These reference runs
also compare manual enqueue inspection and persisted pause and resume for
two queues. The pinned runtime uses its per-queue controls. The candidate
uses the namespace-wide controls.

The smoke test runs one small matched pair. The fault test observes a claim
and a committed effect before terminating the runner, then checks recovery
from a separate database connection. Mesh recovers by lease expiry. Oban
uses its Core Lifeline recovery policy.

To capture J2 lane 10, run the verifier on a disposable database and save
`j2-lane-10.log`.

```bash
set -o pipefail
env -u MESH_TEST_DATABASE_URL bash scripts/verify-postgres-jobs.sh 2>&1 | tee j2-lane-10.log
```

Look for `J2 lane 10 passed` and a successful verifier exit. The proof holds
the first 16 busy claims in a PostgreSQL transaction until the sparse job
becomes due. The busy queue has one free slot after that transaction commits.
PostgreSQL must record the sparse admission before the next busy admission,
with three due busy jobs unclaimed throughout. The compiled runner must
start and settle every admitted job on attempt one. This lane covers one
poll-only batch runner with fixed queue limits. It passed on macOS arm64
with PostgreSQL 17.11, 18.7, and 19.0. [Compiled receipts](../proof/jobs-parity/qualification-v1.json)

The compiled process proof also includes J7 lane 8. On a schedulable host,
run the verifier with its disposable loopback database and capture the
`J7 lane 8 passed` line in `j7-lane-8.log`. The lane holds the runner's sole
application checkout open, interrupts the listener, and observes recovery and
cron checkout failures. It times `Jobs.stop` from API entry and bounds
`Jobs.join` from the release of the checkout. A replacement runner polls
while the listener is still unavailable, then recovers an expired lease and
catches up a persisted calendar occurrence. Independent PostgreSQL sessions
observe the lease, the job attempts, and the cron cursor.
On macOS arm64, the proof passed with PostgreSQL 17.11, 18.7, and 19.0.
The longest stop took 42.5 ms with a 40 ms grace period. The longest join
took 32.3 ms from checkout release. In each run, the expired job completed
on attempt two and the cron cursor advanced.
[J7 lane 8 receipt](../proof/jobs-parity/j7-lane-8.log)

The default fault run also cuts the settlement connection before the write
reaches PostgreSQL and after PostgreSQL confirms the commit. This probe runs
on the driver's disposable loopback database. With verified TLS enabled,
the driver runs the rollback and runner-death cases only.

For an exact PostgreSQL version and verified TLS, point `PG_BIN_DIR` to that
version's binaries and pass the version to both commands.

```sh
bash scripts/verify-postgres-jobs.sh --compatibility 18.7 --parity-driver-check
python3 scripts/jobs-parity.py --smoke --postgres-version 18.7 --mesh-ref 59f6a7b3
```

## Replay the fresh-application guides

Set `release` to a release directory containing `meshc`,
`libmesh_rt.a`, and `jobs-parity-toolchain.json`. With PostgreSQL server
tools, Python 3, and `jq` installed, run from the checkout root.

```sh
PG_BIN_DIR="$(pg_config --bindir)" bash scripts/verify-jobs-guide.sh "$release"
```

The proof checks both artifact hashes and compares the pinned production
compiler and runtime sources with the current checkout. It runs the
operations walkthrough from Markdown on a disposable cluster, then compiles
and runs the concurrency guide's transactional enqueue and worker examples.
The receipt covers Mesh
0.1.8 on macOS arm64 and PostgreSQL 17.11 over loopback SCRAM. Lease recovery
uses drain process loss on the same primary. [J7 lane 9 receipt](../proof/jobs-parity/j7-lane-9.log)

The same fresh-application walkthrough passed with release runtime
`1b12243b`. [Latest guide receipt](../proof/jobs-parity/qualification-v1.json)

## Record a baseline

Run on an otherwise idle machine. Keep the database and both engines on the
same host, and use the same `PG_BIN_DIR` throughout a comparison. Use
`--load` without profile flags to run the manifest in full. The following
command runs the dense short-job profile with four runners and 16 total slots.

```sh
python3 scripts/jobs-parity.py --load --mesh-ref 59f6a7b3 --workload short --runners 4 --slots 16
```

Use `db606548` for the pre-reliability reference and `59f6a7b3` for the
post-reliability reference. Use the Git SHA of the candidate when measuring a
change. The driver alternates the engines and records five pairs after warm-up.
Commit the workload settings before using the baseline to evaluate changes.
Both fixtures receive the same seeded inputs and write an attempt and an
idempotent effect for each completed job.

Sparse trials warm one producer connection before starting the runners.
The driver submits one job at a time after the previous job settles and
the configured gap elapses. The raw commit timestamps show the resulting
spacing. Sparse throughput measures this serial submit-and-settle flow.

The driver resets `pg_stat_statements` on its disposable cluster before each
trial and checks for counter eviction during measurement. PostgreSQL can
retain entries for dropped trial databases until a reset. The raw trial
records the deallocation counter alongside the SQL call counts.

The archived comparison `j1-load-xqlrv50b` reconciled 42,360 jobs per
engine across 32 profiles. It stopped when one Oban large-context trial
reported zero enqueue SQL calls despite completing all 64 jobs. A local
pressure probe reproduced this counter loss after retaining entries for a
dropped database. The corrected large-context rerun reconciled five pairs
with zero counter evictions. [Archived and corrected receipts](../proof/jobs-parity/qualification-v1.json)

For repeated comparisons, `--toolchains target/jobs-parity/build` reuses
the compiler and runtime pairs recorded by a prior archive build. It
checks the source SHA, build profile, Rust toolchain, and file hashes
before linking each fixture.

For a Mesh revision that includes batch claims, pass `--mesh-batch` to measure
the opt-in runner. Omit the flag for the single-claim profile. Both modes
record claim, runner, and producer SQL calls per completed job. Use
`--against-ref` to run five pairs against another Mesh commit with the same
inputs and connection budget. To measure batch dispatch on the same Mesh
commit, pass that commit to both `--mesh-ref` and `--against-ref`, then add
`--mesh-batch`. To exercise batch commit-response loss, use
`--faults --mesh-batch`.

Use `--faults --against-ref 7c162056` to compare transactional rollback and
runner-death recovery against main. Each runtime gets a fresh database.
The driver checks each enqueue ID and effect key against the final durable
rows, then writes the matching results and both receipts to `j1-lane-1.log`.

Measure committed wakeups with `--mesh-listen`. It enables the listener on
the candidate and transactional publishing on its producer. The Mesh
reference remains polling-only. `--poll-ms` and `--baseline-poll-ms` set
the runner intervals without changing the workload's enqueue cadence.

```sh
python3 scripts/jobs-parity.py --load --workload sparse --runners 1 --slots 16 \
  --mesh-listen --poll-ms 1000 --against-ref 7c162056 --baseline-poll-ms 250
```

Repeat with `--baseline-poll-ms 1000` to compare with polling at the same
cadence. The receipt records claim timing and notification-triggered scans.
Use the compiled compatibility proof to exercise listener interruption and
the reconnect catch-up scan.

At `53fb9189`, the five-pair sparse comparison with a 1-second listener
fallback and main's 250 ms polling cadence recorded a worst trial
commit-to-claim p99 of 30.9 ms. Median idle queries per minute were 709
with listening and 1,781 on main. With both poll intervals set to one
second, the listener's worst trial commit-to-claim p99 was 4.2 ms. The
one-second polling control exceeded the configured 750 ms pickup budget.

The `1b12243b` release runtime repeated both five-pair comparisons on
macOS arm64 and PostgreSQL 17.11 over verified TLS. The listener used
670 idle queries per minute against 1,778 on main at 250 ms polling.
The worst healthy trial commit-to-claim p99 was 5.5 ms in that comparison
and 21.8 ms at a matched 1-second interval. The polling reference again
exceeded the 750 ms pickup budget at 1 second. [Polling comparison receipts](../proof/jobs-parity/qualification-v1.json)

### Measure fallback with the listener disconnected

Point `PG_BIN_DIR` to the PostgreSQL version you pass below. Use the driver
from the commit that adds `--listener-outage` and compile the runtime from the
pinned comparison revision.

```sh
python3 scripts/jobs-parity.py --listener-outage \
    --mesh-ref 9f4ae4f19e9b081d6b70752f4688a16fab3c27c7 --postgres-version 17.11
```

The driver alternates five pairs of healthy and disconnected trials after an
unmeasured warmup. Both arms use the frozen sparse workload and seed, one
runner with 16 slots, batch claims, and a 1000 ms runner poll interval. They
publish wakeups and use the same commit, claim, and handler entry timestamps
as the sparse comparison.

The driver checks the disposable database's ownership nonce and waits for
PostgreSQL and the runner to confirm the TLS listener and its initial scan.
It then holds new coordinator connections at the loopback relay and
terminates the exact idle listener in the disconnected arm. It waits for a
reconnect to reach the gate before enqueuing. The original pool, claim, and
lease settlement sessions keep their verified TLS connections. Handlers and
producers connect directly to the same SCRAM-authenticated cluster.

Check `raw.json` for the PostgreSQL session identities and live wake counters
before and after the workload. The disconnected arm must retain the original
runner sessions, complete every job on its first attempt, and report fallback
scans without notifications or a new subscription. Check `summary.json` for
both arms' claim p99 and idle SQL calls per minute. The healthy claim p99
uses the existing 100 ms limit. The disconnected arm's maximum
commit-to-handler-entry time must be at most 2000 ms across all measured jobs.

Runtime `1b12243b` passed five pairs on macOS arm64 with PostgreSQL 17.11
over verified TLS. The worst healthy trial commit-to-claim p99 was
13.3 ms. The slowest disconnected pickup was 871.9 ms. Each trial
reconciled 24 jobs, and each disconnected runner kept its original
database sessions apart from the interrupted listener. [Listener outage receipt](../proof/jobs-parity/qualification-v1.json)
Check `result.json` for budget breaches.

Live timing from this probe is pending. The historical healthy comparison
above and the compiled reconnect proof cover their own configurations.

## Measure runner telemetry

On an idle host, pair the committed candidate with pinned main. Run all three
modes with the same PostgreSQL binaries and profile.

```sh
jobs_candidate=$(git rev-parse HEAD)
for mode in disabled logged backpressured; do
    python3 scripts/jobs-parity.py --load --mesh-ref "$jobs_candidate" \
        --against-ref 7c162056 --mesh-events "$mode" --workload short --runners 1 --slots 16
done
```

The logged mode writes JSON lines asynchronously. The backpressured mode
leaves the 128-event reader unread. In each measured backpressured trial,
check that the reader's drop count is positive and its sampled peak reaches
its bound. Check the throughput and pickup p99 verdicts in `summary.json`
against the existing profile limits.

`raw.json` records `runner_event_buffer_bytes` in runner order.
`metrics.json` and `trials.json` record the largest per-runner value for
each sink in `event_buffer_bytes`. `summary.json` takes the maximum for
each sink across measured trials. A missing report from an older or
disabled runtime becomes `null` in the receipt. A disabled sink in a
report also appears as `null`.

The byte unit charges each occupied channel slot for an event and a full
attempt identity, including the identity's two strings. Each queued copy
pays the full charge even when the identity is absent or shared.
`upper_bound_bytes` multiplies that charge by the sink's slot capacity.
`sampled_peak_bytes` applies the same charge to the largest slot count
observed after a successful enqueue or when an enqueue found a full
channel. A consumer can drain slots between samples, so the sampled peak
can miss a transient maximum. Both fields cover queued records and
exclude unused slots, channel and allocation overhead, serialized output,
and records already removed by the reader or logger.

The five-pair dense comparison on runtime `1b12243b` met the configured
limits in all three modes against main `7c162056` on PostgreSQL 17.11.
The unread reader reached its 29,184-byte bound and dropped 130 events
in each trial. The logger's sampled peak was 228 bytes with the same
29,184-byte bound. Each trial reconciled 128 jobs. The run IDs are
`j1-load-krk1cjmw` for disabled, `j1-load-fmcd4oma` for logged, and
`j1-load-mm7ws85q` for backpressured.

Use `--faults --mesh-events logged` on a current runtime to verify that
an acknowledgement lost after commit produces uncertainty without a
commit confirmation from the runner.

The five-pair short-job tests compared the combined `53fb9189` runtime
with main `7c162056`. The disabled run missed both 5% limits, at 273.9
versus 338.3 jobs per second and pickup p99 of 440.1 versus 354.2 ms.
The logged and backpressured runs both met their throughput limit, and
the unread buffer dropped 130 events in every measured trial. Host load
varied across the runs. These comparisons include the combined changes
to dispatch and maintenance.

A second disabled run with the same binaries recorded 326.5 jobs per second
against 311.7 on main, with pickup p99 of 365.9 ms against 385.9 ms. Both
metrics met their limits on the rerun. Its startup one-minute load was 17.5
on 15 logical CPUs. Raw receipts are in run `j1-load-7tn1vav3`.

The settlement-wakeup runtime `83a76b7d` also met both limits with
telemetry disabled. In five pairs, it recorded 391.8 jobs per second and
310.1 ms pickup p99. Main recorded 328.7 jobs per second and 363.0 ms pickup
p99. Raw receipts are in run `j1-load-f8ghhalg`.

Each run prints its evidence directory under `target/jobs-parity/<sha>/`.
`manifest.json` records the compiled SHA, fixture hashes, tool versions,
database parameters, and host. `settings.json` records the frozen workload
and budgets. Each profile stores its raw attempt and occupancy samples,
trial metrics, and median and range. Exit code 1 indicates a queue
reconciliation mismatch. Exit code 2 indicates a fixture or environment
failure. The run records budget breaches separately in `result.json`.
`summary.json` includes the Mesh-to-Oban throughput and pickup comparisons.
Batch trials also record the largest confirmed context payload held before
launch on any runner. A historical runtime can omit that metric. The
summary then marks the batch context-size budget unverified.

Reconciliation runs after runner shutdown for load and fault cases. It
compares the final job, business-write, attempt, and effect identities
across the disposable database against the producer's enqueue receipts.
A direct occupancy observation above the configured slots
fails the trial. The timestamp reconstruction stays available separately.

Pickup uses the interval between the enqueue transaction's commit timestamp
and the first handler entry. Claim and settlement are tracked separately.
The raw attempt records include the commits of the handler's first attempt
record and its effect. These timestamps split the interval between handler
entry and settlement without adding work to the handler.
The connection and operating system resource samples describe observed
peaks at the configured sampling interval. Each sample also records the
host's one-minute load average. Read the trial peak alongside the CPU
count and startup load in the manifest. Runner CPU includes the process
startup, idle window, and shutdown. The timed settlement window comes from
PostgreSQL's commit timestamps. The load generator and the observer are
excluded from runner process measurements.

`--smoke` and `--faults` produce correctness evidence. Use the five-pair load
receipts and the plan's separate live gates to assess performance and
functional parity.

## Run the J6 role and operations proof

Use a local checkout with the pinned main revision `7c162056` available.
The proof starts a disposable cluster and creates separate databases for a
fresh install and an upgrade.

```sh
env -u MESH_TEST_DATABASE_URL -u MESH_JOBS_TEST_PG_URL \
  bash scripts/verify-postgres-jobs.sh --j6-roles
```

The role proof revokes default `PUBLIC EXECUTE` before migration. It
checks a held legacy attempt across the upgrade, then runs the compiled
producer, worker, operator, inspector, and migrator with scoped logins
in both databases. The PostgreSQL proof also covers local queue control
with single and batch claims, cron owner loss across commit, retained
receipts after delete and prune, and a concurrent index swap while a
compiled worker continues settling.

Use this receipt as functional evidence for the host and PostgreSQL
version printed by the script. Keep the separately measured maintenance
latency, the paired comparison against main, and the remaining platform
lanes in the parity plan.

## Compare maintenance under load

Run `scripts/jobs-j6-maintenance.py` with pinned main and candidate
compiler/runtime pairs. It compares main with native reindex, the
candidate with native reindex, and the candidate with the scoped reindex
script. It uses equal page limits for pruning, recovery, and due staging.
Use release artifacts with `--build-profile release --qualify` to apply
the dispatch and claim-gap budgets to five or more paired rounds.

The initial PostgreSQL 17.11 debug smoke reconciled 72 jobs and effects per
arm and kept request receipts after pruning. The script arm's dispatch p99
was 1.659 times main and its maximum observed claim gap was 164 ms against
a 500 ms budget. One busy-host round provided functional evidence for the
pages and receipt replay. The compiled operations proof separately exercised
settlement while a concurrent reindex held its administrative lock.

The five PostgreSQL 17.11 release rounds with main `7c162056` and candidate
`302cdce8` recorded script-arm dispatch p99 between 0.958 and 1.026 times
main. Every round met the measured latency and claim-gap budgets. The
probe classified the run as diagnostic because the peak one-minute load
was 1.76 per logical CPU.

The `53fb9189` release rerun met those measured budgets in all five
rounds too, with script-arm dispatch p99 from 0.972 to 1.021 times main.
Host load reached 5.92 per logical CPU during the rerun, so its receipt
also records a diagnostic verdict.

The `1b12243b` release rerun completed five rounds. Script-arm dispatch
p99 ranged from 0.920 to 0.953 times main. The largest observed claim gap
was 120 ms against a 500 ms budget. All rounds met the measured limits.
The host peaked at 0.812 one-minute load per logical CPU against the 0.75
quiet-host limit, so the receipt records a diagnostic verdict. [Maintenance receipt](../proof/jobs-parity/qualification-v1.json)

## Exercise primary promotion

Build the compiler and runtime from the revision under test. Set `PG_BIN_DIR`
to a PostgreSQL installation with `pg_basebackup` and `pg_stat_statements`.
Choose a new evidence directory under the checkout for each run.

```sh
cargo build --locked -p meshc -p mesh-rt
python3 scripts/jobs-promotion.py --postgres-version 17.11 \
  --listen --evidence-dir target/jobs-parity/promotion-17.11
```

The proof starts a primary and a physical streaming standby. It loses a
settlement reply after commit, waits for the standby to replay through the
primary's flush LSN, stops and fences the old primary, then promotes the
standby. It checks receipts, job IDs, effect keys, and stale-token rejection
while the replacement attempt holds its lease. Replication and direct
database connections use verified TLS and SCRAM. The one-shot fault proxy
uses plaintext loopback with SCRAM so it can identify the settlement reply.
`--listen` also verifies that the replacement runner subscribes and scans
for committed work on the promoted primary.

The replay barrier in this proof preserves the observed commits before
promotion. For a deployment with asynchronous replication, the failover
operator must account for commits newer than the standby's replay position.

## Exercise rolling upgrade and worker rollback

Use `scripts/verify-jobs-upgrade.py --postgres-version 17.11` to compile the
pinned main and candidate fixtures and run both under scoped database roles.
Repeat with PostgreSQL 18.7 and 19.0. The proof keeps a legacy lease active
through migration, checks the pending deadlines and attempt budget, and
returns to a legacy worker on the upgraded schema. Keep schema migration on
the newer binary during rollout and worker rollback. Running the old
migration as schema owner restores the legacy claim body and bypasses the
namespace pause for legacy claims until the new migrator reapplies it.

The checked-in v1 receipts compare `7c162056` and `23a95b87` on macOS arm64
with verified TLS. [PostgreSQL 17.11](../proof/jobs-parity/rolling-upgrade-v1.pg17.11.json) ·
[PostgreSQL 18.7](../proof/jobs-parity/rolling-upgrade-v1.pg18.7.json) ·
[PostgreSQL 19.0](../proof/jobs-parity/rolling-upgrade-v1.pg19.0.json)

## Recorded baseline

For the five-pair run `j1-load-19blcy6t`, the benchmark used harness `6141ab94`,
Mesh `59f6a7b3` in its default single-claim mode, Oban Basic 2.24.2, and
PostgreSQL 17.11 over loopback SCRAM. It ran on macOS arm64 with one runner
and 16 total slots. Each short-job trial reconciled 128 jobs and each sparse
trial reconciled 24. These are medians across the five measured trials.

| Profile | Metric | Mesh | Oban Basic |
| --- | --- | ---: | ---: |
| Short, 1 KiB | Completed jobs per second | 201.2 | 565.7 |
| Short, 1 KiB | Pickup p99, ms | 608.3 | 223.4 |
| Short, 1 KiB | CPU per completed job, ms | 7.02 | 5.39 |
| Sparse, 1 KiB | Pickup p99, ms | 241.3 | 252.7 |

The host's one-minute load average was 42.2 on 15 logical CPUs at startup.
The short profile missed the proposed throughput, pickup, and CPU thresholds.
That harness undercounted macOS threads. This run records two diagnostic
post-reliability profiles.
Release qualification still requires the matched candidate matrix on a
host with capacity for the workload and its observers.

## Release comparisons for batch dispatch

Harness and runtime `53fb9189` completed five pairs per profile on
PostgreSQL 17.11 with verified TLS, one runner, and 16 slots. Against main
`7c162056`, the short profile used 0.102 claim calls per completed job with
batching and 1.008 on main. Sparse pickup p99 was 252.5 ms with batching and
269.7 ms on main. The largest queued context was 21,696 bytes in these
profiles, below the 32 MiB limit. Raw receipts are in run `j1-load-l5_ot88g`.

The dense short-job comparison against Oban Basic 2.24.2 reconciled 128
jobs per trial. Mesh completed 235.9 jobs per second against Oban's 405.1,
with pickup p99 of 493.0 ms against 311.0 ms. The startup one-minute load
was 25.1 on 15 logical CPUs. This profile missed both proposed Core parity
thresholds. Raw receipts are in run `j1-load-pqsep8or`.
The same run's sparse profile reconciled 24 jobs per trial and met its
measured limits, with pickup p99 of 258.8 ms on Mesh and 252.1 ms on Oban.

With four runners and 16 slots, another PostgreSQL 17.11 run reconciled all
five pairs for each of the following profiles. Each missed both Core
throughput and pickup limits. Startup one-minute load was 18.9 on 15
logical CPUs. Raw receipts are in run `j1-load-5hx0vvd4`.

| Profile | Jobs per trial | Mesh jobs/s | Oban jobs/s | Mesh pickup p99, ms | Oban pickup p99, ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 100 ms, 16 KiB | 128 | 91.2 | 124.1 | 1,277.0 | 926.4 |
| No-op, 1 KiB | 256 | 266.3 | 959.7 | 913.9 | 266.1 |
| Large-context stress, 512 KiB | 16 | 56.5 | 132.5 | 261.7 | 117.3 |

The short-profile timing run `j1-load-6h3ntgfp` used the same release
runtime and one runner with 16 slots. Median time from handler entry to
the first attempt record's commit was 9.8 ms on Mesh and 0.3 ms on Oban.
That interval includes connection setup for Mesh and a pool checkout
for Oban. Median time from effect commit to settlement was 15.8 ms on
Mesh and 0.5 ms on Oban. Each value is the median across five trial medians.

Runtime `83a76b7d` wakes the dispatcher when the supervisor reaps an attempt
and has a reply ready to settle. In five single-claim pairs against
`53fb9189`, the median effect-to-settlement interval fell from 13.8 ms to
9.7 ms. All five pairs improved. Median throughput rose from 326.3 to
388.2 jobs per second, with trial ranges of 254.2–345.3 and 354.9–395.3
respectively. Each trial reconciled 128 jobs. Raw receipts are in run
`j1-load-p3kxhctg`.

In five separate batch pairs against Oban, `83a76b7d` recorded 363.5 jobs
per second and 333.4 ms pickup p99. Oban recorded 438.7 jobs per second
and 287.4 ms pickup p99. The batch profile met the pickup limit and missed
the throughput and CPU limits. Raw receipts are in run `j1-load-if2jom40`.

Runtime `e8fa00c0` uses ring for the SCRAM password derivation. In five
single-claim pairs against `83a76b7d`, median CPU per completed job fell
from 6.41 to 5.74 ms. Median handler entry to the first committed write
fell from 5.96 to 5.19 ms. The throughput ranges overlap. Startup
one-minute load was 14.6 on 15 logical CPUs. Raw receipts are in run
`j1-load-6dnh2p3i`.

In the separate batch comparison, Mesh used 1.15 times Oban's CPU per
job, within the 1.25 limit. Median throughput was 316.5 jobs per second
on Mesh and 442.2 on Oban. Pickup p99 was 387.7 ms on Mesh and 286.5 ms
on Oban. Both Core performance limits still failed in this run. Startup
one-minute load was 8.5 on 15 logical CPUs. Raw receipts are in run
`j1-load-fycjmfc1`.

The worker-reply wakeup candidate `125772f8` shortened median time from
effect commit to settlement from 9.78 ms to 2.35 ms in five single-claim
pairs against `e8fa00c0`. All five pairs improved on that interval. Median
throughput was 411.5 jobs per second, with a trial range of 311.6–521.5.
The baseline median was 378.5, with a range of 304.7–507.4. Raw receipts
are in run `j1-load-3q6w3p22`.

In the batch comparison against Oban Basic 2.24.2, Mesh met the two
dense timing limits at 411.5 jobs per second and 293.8 ms pickup p99.
Oban recorded 425.5 jobs per second and 298.2 ms pickup p99. Mesh used
1.30 times Oban's CPU per job, above the 1.25 limit. Raw receipts are in
run `j1-load-3vl21x7c`. The candidate's worker test confirmed that the
supervisor cannot consume a reply until it reaps the child.

Runtime `1b12243b` restricts early scans to pending replies and keeps the
full watchdog deadline independent. The single-claim comparison against
`125772f8` had wide and overlapping trial ranges for throughput and CPU.
The separate batched comparison passed the CPU limit and missed both
dense timing limits. It started at a one-minute host load of 32.4 on 15
logical CPUs. Raw receipts are in runs `j1-load-0ghrr0su` and
`j1-load-dnc2kr57`.

## Compare bounded drain

`scripts/jobs-drain-perf.py` accepts the pinned main and candidate compiler
and runtime artifacts. It interleaves legacy due-only drain and the new
bounded drain with one slot. Each trial checks the same completed jobs,
their effects, and the untouched future rows. Pass `--direct` to measure the
same compiled handler on both toolchains. Main calls it with a prebuilt
context. The candidate uses `Test.perform_job` to build and validate the
context and decode the outcome on every call. Pass `--build-profile release`
when comparing two release toolchains.

The local PostgreSQL 17.11 debug run with main `7c162056` and candidate
`03990801` reconciled 24 completions and four future rows in each of seven
pairs. The median paired duration was 1.294 times main, above the 1.10
budget. Host load was about 30–34 on 15 logical CPUs. The candidate's inline
helper took 69.1 µs per call in that debug run.

With release artifacts `7c162056` and `302cdce8`, all seven PostgreSQL 17.11
pairs met the 1.10 budget. The median paired ratio was 1.002, with median
drain durations of 29.0 ms per job on main and 28.4 ms on the candidate.
The host's one-minute load was about 20–22 on 15 logical CPUs during this
run.

The `1b12243b` release rerun reconciled 24 effects and four untouched
future rows in every trial. All seven pairs met the 1.10 duration budget.
The median paired ratio was 0.760, with median durations of 30.8 ms per
job on main and 23.9 ms on the candidate. The separate probes measured
22 ns per direct call with a prebuilt context and 5.91 µs per call through
`Test.perform_job`, including context construction and outcome decoding.
The matched default short-job workload recorded 1.055 enqueue SQL calls
per job in every trial and a median of 1.016 claim calls per job on both
runtimes. [Drain and production SQL receipts](../proof/jobs-parity/qualification-v1.json)

## Compiled lane receipts

The macOS arm64 verifier passed the batch admission and lease isolation
cases, the listener lanes, the selected telemetry and operations lanes,
and the executable replacement and unstarted-claim proof. It ran with
PostgreSQL 17.11, 18.7, and 19.0 over verified TLS and SCRAM. The listener
framing case uses a scripted TCP peer that sends partial, coalesced, and
malformed frames.

The `macos_compiled_lanes` entry records the exact checkouts, full logs,
and named excerpts under `target/jobs-parity/<checkout>/lane-evidence/`.
Each excerpt identifies the assertion source and the lines from the full
log. The `macos_paired_workload_lanes` entry records the seed, input digest,
and reconciliation for the selected pairing lanes. [Qualification receipt](../proof/jobs-parity/qualification-v1.json)

J2 lane 1 uses all seven short-job profiles from the PostgreSQL 17.11
main/head comparison. The audit matched 20,480 final job IDs to the
enqueue receipts and verified one attempt and one effect per job. The
one-runner, 16-slot reference profile used 55.0% fewer claim calls per job.
Five profiles met the 30% reduction threshold in the driver. The
one-slot profile had no reduction, and eight runners with 16 slots used
18.4% fewer calls. The lane receipt links all seven raw summaries.

J7 lane 1 covers all 35 frozen profiles, including 100 ms jobs, sparse
pickup, no-op stress, and 512 KiB payloads. There were five alternating
pairs after warm-up per profile. PostgreSQL 17.11 ran with verified TLS
and SCRAM on macOS arm64. Each runtime reconciled 43,080 jobs. The two
short-profile claim-call misses persisted in the full matrix. Sparse pickup
p99 with four runners and 64 slots was 241.2 ms on the candidate and
205.5 ms on main, above the 1.10 ratio limit. The run started at a one-minute
host load of 13.2 and reached 32.6 on 15 logical CPUs. [Matrix receipt](../proof/jobs-parity/j7-lane-1.log)

The follow-up verifier used the same runtime sources and passed 45 Jobs
runtime tests and both compiled suites on PostgreSQL 17.11, 18.7, and 19.0.
Both suites complete shared schema setup before running namespace traffic.
The verifier covered batch pause and scaling, oversized contexts, queue
rotation, stale acknowledgement
events, and lease renewal while the reader buffer was full. The batch
context test reached 25,166,688 queued bytes and completed the four
admissible jobs. The oversized row retained attempt zero.

The release fault harness recovered the same job ID and deduplicated the
effect after runner death. The two Mesh revisions produced matching
results in the pinned-main recovery comparison over verified TLS. The
settlement and batch-claim cuts used SCRAM on plaintext loopback. Losing
the reply before batch commit produced one native worker start at attempt
one. Losing it after commit, including after `CommandComplete` and before
`ReadyForQuery`, produced one start at attempt two. An uncertain settlement
emitted `ack_uncertain`. Both engines left one durable effect in each
settlement case. The `macos_follow_up_lanes` entry links the named receipts.
[Qualification receipt](../proof/jobs-parity/qualification-v1.json)

### Verified PostgreSQL workflows

The following cases passed on macOS arm64 with PostgreSQL 17.11, 18.7, and
19.0 using runtime `1b12243b`. The proof includes both compiled Jobs
suites and the PostgreSQL-backed runtime tests.

| Workflow | Tested behavior |
| --- | --- |
| Transactional enqueue | A business write and its job commit together. Rollback leaves neither, and receipt replay returns the same job identity. |
| Uniqueness and replacement | A permitted duplicate updates the stored args while the active attempt keeps its claim token and initial context. The retry receives the new args. |
| Scheduled and recurring work | Scheduled and retryable jobs reach their due timers. A replacement cron owner preserves a committed occurrence and recovers an interrupted transaction. |
| Retry, snooze, and exhaustion | Compiled retries preserve job and effect identities, honor the due time, and discard at exhaustion. The runtime store proof preserves the attempt count after a snooze. |
| Cancellation and recovery | A committed cancellation wins over an acknowledgement. After runner death, the replacement holds a new token and PostgreSQL rejects the stale token. |
| Queue controls and admission | Four compiled runners respect the global cap. Local pause leaves work for a peer, scale respects the lower limit, and stopping one queue leaves another able to work. |
| Tests and bounded drain | Compilation rejects test helpers in a production build. The test runner verifies borrowed-transaction visibility and namespace isolation. Compiled drain cases cover cutoff, recursion, error policy, and restart. |
| Telemetry and maintenance | The runner reports database-confirmed dispositions and redacts job data. Scoped role proofs cover fresh and upgraded databases. A concurrent index rebuild allows another job to settle and refuses a second rebuild owner. |

## Local correctness evidence

The Jobs compatibility script exercises the compiled producer and worker,
the PostgreSQL driver, leases, cancellation, timeout, recovery, and
shutdown. The full-role variant adds scoped migrator, producer, worker,
operator, and inspector logins, plus cron takeover and reindex.

| Local proof | PostgreSQL | Scope |
| --- | --- | --- |
| Linux arm64 container, pinned main `7c162056` | 17.11, 18.7, 19.0 | Baseline transaction, timeout, and process recovery over verified TLS |
| Linux arm64 container, candidate `0c40c3ad` | 17.11, 18.7, 19.0 | 43 Jobs runtime cases, two compiled suites, scoped roles, and concurrent reindex over verified TLS |
| Linux arm64 container, candidate `ebe14b91` | 17.11 | Repeat of the full-role proof with the batch dispatcher wakeup change |
| Linux arm64 container, runtime `1b12243b` | 17.11, 18.7, 19.0 | 36 PostgreSQL driver cases, 45 Jobs runtime cases, both compiled suites, and fresh and upgraded scoped roles over verified TLS with SCRAM. [Receipt](../proof/jobs-parity/j1-postgres-matrix.log) |
| macOS arm64, candidate `53fb9189` | 17.11, 18.7, 19.0 | 43 Jobs runtime cases, two compiled suites, scoped roles, reindex, and pinned polling reference over verified TLS |
| macOS arm64, release candidate `53fb9189` against main `7c162056` | 17.11, 18.7, 19.0 | Compiled rolling upgrade and rollback with scoped roles and default function execution revoked |
| macOS arm64, promotion harness `ebe14b91` and release runtime `302cdce8` | 17.11, 18.7, 19.0 | Lost settlement reply, replay barrier, old-primary fencing, listener subscription catch-up, and one effect per job |
| macOS arm64, candidate `83a76b7d` | 17.11, 18.7, 19.0 | Repeat of the full-role proof with the settlement wakeup. All 43 Jobs runtime cases and both compiled suites passed on each version. |
| macOS arm64, candidate `e8fa00c0` | 17.11, 18.7, 19.0 | Verified TLS and SCRAM after the derivation change, including all 36 PostgreSQL driver cases, 43 Jobs runtime cases, both compiled suites, and fresh and upgraded scoped roles |
| macOS arm64, runtime `1b12243b` | 17.11, 18.7, 19.0 | 45 Jobs runtime cases, both compiled suites, fresh and upgraded scoped roles, and combined pool, listener, and maintenance interruption over verified TLS |
| macOS arm64, release runtime `1b12243b` with main `7c162056` for the upgrade | 17.11, 18.7, 19.0 | Rolling upgrade, legacy settlement and rollback under scoped grants, and primary promotion with an uncertain acknowledgement. [Receipt](../proof/jobs-parity/j7-rolling-and-promotion.log) |

The Linux process proof captured the worker PID and kernel start time for
each held attempt before injecting parent death or waiting for the hard
timeout. It checked that both original processes exited and reconciled a
replacement attempt for each job. [Linux process receipt](../proof/jobs-parity/j7-lane-2.log)

The listener promotion runs resumed eligible work within 1.31 seconds of writability
against a 2-second budget. Replication and direct sessions used verified
TLS and SCRAM. The loopback response-loss relay used SCRAM without TLS.
The paired smoke and faults used release runtime and harness `53fb9189`.
Both engines reconciled producer IDs, business writes, attempts, and effects
after shutdown. The Mesh batch ran exactly one handler after each of the
three COMMIT cuts, including loss of `ReadyForQuery` after `CommandComplete`.
The one-runner, one-slot smoke observed four Oban processes against a cap
of two. That configuration exceeds the frozen process budget.

The `83a76b7d` release rerun also passed the nine SCRAM fault cases,
including settlement response loss and the three batch claim response
cuts. All nine passed again on `e8fa00c0`. Raw receipts are in runs
`j1-fault-1akgie45` and `j1-fault-k1yd8gnc`.

The local workspace gate passed at `e5a5f3c8`, including formatter and
linter checks, compiler and runtime tests, the workspace and fuzz builds,
the REPL, and the compiled `meshc` suites.

The follow-up gate on source `64666eb7` passed 1,432 compiler tests,
1,550 runtime and library tests, and 690 compiled CLI and REPL tests. The
workspace built, and the locked fuzz targets passed `cargo check`.
Formatter and linter checks passed for examples and packages. [Workspace receipts](../proof/jobs-parity/qualification-v1.json)

The authoritative workflow targets Linux x64 and macOS arm64. These local
receipts cover arm64 hosts and a Linux arm64 container. Windows has no
compiled Jobs qualification in this matrix. Core parity remains open until
the full candidate comparison and the workload-specific limits pass.
The [qualification receipt](../proof/jobs-parity/qualification-v1.json)
records the source revisions, measured results, and remaining gates.
