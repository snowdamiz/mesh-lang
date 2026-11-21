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
uses drain process loss on the same primary. The walkthrough and reference
snippets also passed with the exact `aa40fa5d` release compiler and runtime
after integrating main. [Integrated guide receipt](../proof/jobs-parity/j7-lane-9-main-integration.log)

The `39b472f2` replay passed with release artifacts from `5d45b53b` after
verifying the artifact hashes and confirming that the production sources
and Cargo inputs matched the checkout. [Bootstrap release guide receipt](../proof/jobs-parity/j7-lane-9-bootstrap-release.log)

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

### Run the comparison on dedicated Linux CPUs

Use one dedicated-CPU Ubuntu x86-64 instance for both engines and the
disposable PostgreSQL server. Keep builds and other workloads outside the
measurement window. Record the CPU model, logical CPU count, and starting
load. Record host CPU steal and I/O wait during the run so the receipt can
be interpreted alongside the per-trial load samples.

Install the pinned Rust, LLVM, Erlang, and Elixir versions in
[versions.env](../tools/performance/versions.env). Use the Ubuntu setup
and PostgreSQL 17.11 source pin in the
[authoritative workflow](../.github/workflows/authoritative-verification.yml).
Include `pg_stat_statements` and point `PG_BIN_DIR` to the pinned server
tools. Set `LLVM_SYS_211_PREFIX` to the LLVM installation and run from the
clean checkout as a non-root user.

```bash
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$PWD/scripts/ci-x86_64-linux-gnu-clang"
export MIX_HOME="$PWD/target/jobs-parity/build/oban/mix"
export HEX_HOME="$PWD/target/jobs-parity/build/oban/hex"
mix local.rebar --force
mesh_ref="$(git rev-parse HEAD)"
python3 -B scripts/jobs-parity.py --smoke --release --mesh-batch \
  --postgres-version 17.11 --mesh-ref "$mesh_ref"

python3 -B scripts/jobs-parity.py --load --mesh-batch \
  --postgres-version 17.11 --mesh-ref "$mesh_ref" \
  --toolchains "$PWD/target/jobs-parity/build"
```

The smoke builds the pinned release artifacts and verifies fixture access
to the database. The load command verifies those artifact hashes before
linking and runs all 35 profiles. Preserve the manifest, per-trial raw
files, summaries, and `result.json` from the printed evidence directory.
Check that collection passed and `budget_breaches` is empty before
reporting that the comparison met the throughput and resource limits.

### Inspect the workload and measurements

Sparse trials start one producer process before starting the runners. Each
enqueue opens and closes its database connections for that transaction.
The driver submits one job at a time after the previous job settles and the
configured gap elapses. The raw commit timestamps show the resulting spacing.
Sparse throughput measures this serial submit-and-settle flow.

The driver resets `pg_stat_statements` on its disposable cluster before each
trial and checks for counter eviction during measurement. PostgreSQL can
retain entries for dropped trial databases until a reset. The raw trial
records the deallocation counter alongside the SQL call counts.

The Oban fixture uses a private Erlang hosts entry for the disposable
cluster's `127.0.0.1` address. The driver applies it after fetching
dependencies. This avoids launching native DNS helpers for the numeric
loopback address and keeps the TLS hostname and certificate checks.

The grouped Oban comparison, two historical replays, and the dense and
sparse main reruns used the reset and eviction check. The full 35-profile
main comparison used harness `eb472832` before those checks existed.
Its SQL counts are positive. The saved evidence cannot establish whether
any statement entries were evicted in that comparison. [Main comparison](../proof/jobs-parity/j7-lane-1.log)

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

The rerun with transaction-scoped producer connections also passed all
five pairs. The worst healthy trial claim p99 was 4.4 ms. The slowest
disconnected pickup was 971.2 ms. [Rerun receipt](../proof/jobs-parity/qualification-v1.json)
Check `result.json` for budget breaches.

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

On PostgreSQL 17.11, 18.7, and 19.0, the compiled stderr proof kept a
saturated socket unread through durable settlement and `Jobs.join`. At
join the drop counter was zero. After the observer closed the socket,
`Jobs.status` reported four dropped events. [Logger proof](../proof/jobs-parity/qualification-v1.json)

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

The five-round quiet rerun peaked at 0.391 one-minute load per logical CPU
and passed. Dispatch p99 ranged from 0.927 to 0.938 times main. The largest
claim gap around a scripted reindex was 115 ms. Claims bracketed the
reindex statements. The compiled scoped-role proof separately observed
settlement during a rebuild. [Rerun receipt](../proof/jobs-parity/qualification-v1.json)

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
the newer binary during rollout and worker rollback. The owner replay probe
compares legacy function definitions and OIDs, then checks that replaying
the old migration preserves the installed catalog and legacy grants. It
checks the namespace pause on existing and newly created queues while an
old lease renews and settles. The same legacy runner observes a rejected
claim and completes both queued jobs after resume. Its `last_error` retains
the rejection after successful work.

The v2 runs used release runtimes `7c162056` and `a431fbbf` with harness
`6191a712`. They preserved all 14 legacy function definitions on macOS arm64
with PostgreSQL 17.11, 18.7, and 19.0 over verified TLS. After resume, the
current single-claim worker also completed a job on attempt one under the
claim-and-settle grants. The paired PostgreSQL 17.11 fault run used the same
release runtimes and loopback SCRAM. [V2 receipt](../proof/jobs-parity/rolling-upgrade-v2.json)

The historical v1 receipts compare `7c162056` and `23a95b87` on macOS arm64
with verified TLS. On those revisions, owner replay could replace the old
claim body and bypass the namespace pause. [PostgreSQL 17.11](../proof/jobs-parity/rolling-upgrade-v1.pg17.11.json) ·
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

## Matched Oban release comparison

On macOS arm64 and PostgreSQL 17.11 over verified TLS, release Mesh
`1b12243b` used batch claims and 250 ms polling against Oban Basic 2.24.2.
The five workload groups each ran seven configurations with per-engine
warmups and five alternating measured pairs. The audit matched the final
job IDs, business rows, attempt numbers, and effect keys to the input and
enqueue receipts after shutdown. It reconciled 43,080 measured jobs per
engine across 350 trials. [Comparison receipt](../proof/jobs-parity/j7-lane-10.log)

| Workload | Throughput within limit | Pickup p99 within limit | Within every limit |
| --- | ---: | ---: | ---: |
| Short, 1 ms and 1 KiB | 0 of 7 | 0 of 7 | 0 of 7 |
| Slow, 100 ms and 16 KiB | 0 of 7 | 3 of 7 | 0 of 7 |
| Sparse | 7 of 7 | 6 of 7 | 5 of 7 |
| No-op stress | 0 of 7 | 0 of 7 | 0 of 7 |
| Large-context stress, 512 KiB | 1 of 7 | 4 of 7 | 1 of 7 |

In the one-runner, 16-slot short profile, median throughput was 256.4
jobs per second on Mesh and 748.2 on Oban. Median trial pickup p99 was
478.2 ms on Mesh and 166.0 ms on Oban. In the corresponding sparse
profile, serial throughput was 1.99 and 2.03 jobs per second. Pickup p99
was 204.2 and 240.6 ms, respectively. Oban reached four processes
against the two-process cap in the one-slot profiles. The eight-runner,
64-slot sparse profile exceeded the relative pickup limit. [Sparse receipt](../proof/jobs-parity/j1-lane-6.log)

Starting one-minute host load ranged from 6.25 to 23.62 on 15 logical CPUs.
The highest sampled one-minute load was 34.73. The recorded timings apply
to these configurations and host conditions. Each group links its raw
trials, SQL counters, and resource samples. [Comparison receipt](../proof/jobs-parity/j7-lane-10.log)

The raw archive includes the original summaries, trial metrics, and
measured rows and resource samples from both 35-profile comparisons.
Its `files.json` lists each recorded source path and SHA-256. The
qualification index records the archive's SHA-256. [Raw archive](../proof/jobs-parity/matched-workload-samples-v1.tar.xz) · [Qualification index](../proof/jobs-parity/qualification-v1.json)

```sh
tar -xOf proof/jobs-parity/matched-workload-samples-v1.tar.xz \
  oban/short/short-1r-16s/trials.json
```

To replay a group, check out its `harness_sha` from the receipt and run
the same runtime with its workload name. The four dense and stress groups
used harness `3e0339f7`. The sparse group used `04cce85d`, which keeps both
producer processes warm and closes their connections after each enqueue.

```sh
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin \
  python3 scripts/jobs-parity.py --load --mesh-ref 1b12243b --mesh-batch \
  --workload sparse --postgres-version 17.11
```

### Follow-up on short jobs and the one-slot process cap

The one-slot rerun used harness `40811314` and a fixture-local Erlang
hosts entry. The original numeric address lookup started two
`inet_gethost` helpers. With the hosts entry, Oban peaked at two processes
in all five one-slot profiles. The paired trials reconciled 2,760 jobs per
engine over verified TLS. The short and 100 ms profiles met the throughput
and pickup limits with the experimental Mesh runtime `4f50a567` and still
exceeded the CPU-per-job limit. [Rerun receipts](../proof/jobs-parity/qualification-v1.json)

The instrumented `1b12243b` short-job fixture separated the handler's
`Pg.connect` call from its first write. Across five trials with 16 slots,
the median of trial medians was 4.95 ms from database claim to native
constructor, 0.52 ms from constructor to handler entry, 5.31 ms in
verified-TLS connection setup, and 0.52 ms from connect to attempt commit.
The sequential plaintext SCRAM run recorded 3.32 ms in connection setup.
These diagnostic runs had separate host-load samples. [Trace receipts](../proof/jobs-parity/qualification-v1.json)

The macOS `4f50a567` experiment signaled the supervisor when the kernel
reported child exit. At one slot, median short-job throughput was 97.6
jobs per second against 72.9 on `1b12243b`, and the median effect-to-settlement
interval was 0.62 ms against 2.33 ms. Both used batch claims. At 16 slots,
the experiment still missed the Oban throughput, pickup, and CPU limits.
The candidate stays experimental because its per-child watcher descriptor
can prevent a later launch under descriptor pressure. [Experiment receipts](../proof/jobs-parity/qualification-v1.json)

The shared watcher experiment `8c86c592` uses one kqueue per opt-in runner.
It passed five native macOS tests and the PostgreSQL 17.11 proof with 52
Jobs runtime tests and both compiled suites. Five release pairs per
profile used the same binary with the feature toggled. With one slot and
direct effect writes, median throughput rose from 45.3 to 75.0 jobs/s.
The effect-commit-to-settlement median fell from 11.79 to 0.87 ms. At 16
slots with the same warm effect relay in both arms, throughput was 399.2
and 402.5 jobs/s. Three of five pairs improved. The shared watcher remains
experimental. [Shared watcher receipt](../proof/jobs-parity/qualification-v1.json)

The macOS dispatch trace at `edf82c7a` reconciled 3,840 short jobs with
one runner, 16 slots, and PostgreSQL 17.11 over verified TLS. Five
rotating trials per arm compared an untagged executable, a tagged
executable with tracing off, and the same tagged executable with
tracing on. Each route used the same job inputs. The second route used
16 warm effect sessions in a fixture relay.

| Median traced interval | Direct effect writes | Warm effect sessions |
| --- | ---: | ---: |
| Claim confirmation to launch | 0.72 ms | 0.99 ms |
| `posix_spawnp` call | 0.22 ms | 0.16 ms |
| Spawn start to native constructor | 3.72 ms | 3.18 ms |
| Native constructor to handler entry | 0.55 ms | 0.47 ms |

In full 16-job batches, the median wait from confirmation to the last
launch was 5.54 ms for direct writes and 4.19 ms for warm sessions.
Across all batches, the largest median interval was from spawn to
constructor. This interval includes loading and scheduling the new
process. Tracing changed median first-claim-to-last-settlement time from
164.0 to 165.7 ms with direct writes and from 99.0 to 107.3 ms with warm
sessions. These diagnostic trials leave the Oban comparison open.
[Dispatch trace receipt](../proof/jobs-parity/qualification-v1.json)

An independent executable link added macOS `-dead_strip` to the same
tagged fixture. It reduced the executable from 26.1 to 7.4 MiB. Across
five pairs at 16 slots, the median traced time from spawn to constructor
fell from 5.33 to 4.25 ms, and every pair used less application CPU per
job. The median pair used 12.6% less CPU per job and processed 10.2%
more jobs per second. At one slot, the median paired throughput gain
was 2.2%. Host load reached 19.6 on 15 logical CPUs, and the probe leaves
linker compatibility and the Oban budgets open.
[Link experiment receipt](../proof/jobs-parity/qualification-v1.json)

The separate `4f7d702f` experiment schedules one supervisor recheck after
an early reply and uses the existing watchdog for later reaps. Across five
batch pairs, one-slot throughput was 73.4 jobs per second against 42.2 on
`1b12243b`. The effect-to-settlement median was 1.73 ms against 11.36 ms.
The 16-slot throughput medians were 429.8 and 404.1 jobs per second.
Eleven runner unit cases and the release smoke passed. Full compatibility
and Oban qualification for this experiment remain open. [Experiment receipts](../proof/jobs-parity/qualification-v1.json)

The integrated retest compared `b62ec83a` with two arms running the same
`c64eee8a` binary. All 3,840 measured short jobs reconciled on
PostgreSQL 17.11 over verified TLS. At 16 slots, the identical baseline
arms measured 448 and 346 jobs/s, and the candidate measured 525 jobs/s.
The candidate beat the first baseline in three of five throughput pairs.
At one slot, all arm medians fell between 38 and 41 jobs/s. End-to-end
gain remains inconclusive under the measured host load. The candidate
stays experimental. [Paired retest receipt](../proof/jobs-parity/qualification-v1.json)

The follow-up compared `4f7d702f` with Oban Basic 2.24.2 on four frozen
dense profiles with one runner and 16 slots. The release runs used
PostgreSQL 17.11 with verified TLS and SCRAM on macOS arm64. Five
alternating pairs per profile reconciled 2,640 jobs per engine. The host's
one-minute load started at 14.0 and reached 21.2 on 15 logical CPUs.

| Profile | Mesh/Oban throughput | Mesh/Oban pickup p99 | Limits exceeded |
| --- | ---: | ---: | --- |
| Short | 0.470× | 2.119× | Throughput, pickup |
| 100 ms | 0.925× | 1.078× | CPU per job, peak RSS |
| No-op | 0.808× | 1.204× | Throughput, CPU per job |
| 512 KiB | 0.911× | 1.064× | None |

The short profiles took medians of 245.5 ms on Mesh and 135.1 ms on Oban
from first claim to final settlement. The largest per-job median difference
after claim was from handler entry to its first committed write. That
interval was 9.53 ms on Mesh and 0.69 ms on Oban and includes connection
setup and the write. The effect-to-settlement medians were 3.54 ms and
1.20 ms. [Frozen-profile receipts](../proof/jobs-parity/qualification-v1.json)

A separate five-pair short-job diagnostic held the Mesh release binary,
batch settings, and polling interval fixed. Transactional publishing and
listening reduced median time to first claim from 150.6 ms to 2.1 ms. The
first-claim-to-final-settlement ranges overlapped at 158–459 ms with
polling and 171–386 ms with notifications. [Notification diagnostic](../proof/jobs-parity/qualification-v1.json)

### PostgreSQL socket experiment

Release candidate `7e4666da` enabled `TCP_NODELAY` before TLS and SCRAM
authentication. On PostgreSQL 17.11, five paired short-job trials with
batch claims on both Mesh arms shortened the median interval from handler
entry to its first committed write from 6.37 ms to 5.92 ms. All five
paired differences favored the candidate. Throughput ranges overlapped
at 386–614 jobs/s on `1b12243b` and 437–611 jobs/s on the candidate. The
no-op profile's throughput medians were 621 and 618 jobs/s respectively.

In separate frozen comparisons against Oban Basic 2.24.2, the socket
candidate missed the short profile's throughput, pickup, and CPU limits.
The no-op profile exceeded the CPU limit. Each comparison reconciled
1,920 jobs per engine. These macOS arm64 runs used verified TLS and SCRAM
on a busy host. The socket change remains experimental.
[Socket experiment receipts](../proof/jobs-parity/qualification-v1.json)

### Unix worker watchdog experiment

Candidate `cc00553e` combined the Unix attempt's deadline timer and
parent-control reader in one thread. On macOS arm64, the debug worker
exited at its own handler deadline and when its parent socket closed. A
fragmented renewal extended the lease after the complete frame arrived.
A separate compiled PostgreSQL 17.11 run confirmed that a handler blocked
on stdin timed out and wrote its entry marker once.

Five single-claim pairs per profile compared the candidate with
`1b12243b`. Across the short and no-op profiles at one and 16 slots,
throughput ranged from 0.907× to 1.022× the reference and CPU per job
ranged from 0.971× to 1.021×. One-minute host load reached 31.5 on 15
logical CPUs during the one-slot comparison.

The batched comparison used the same inputs and Oban Basic 2.24.2.
PostgreSQL 17.11 used SCRAM and verified TLS. Each comparison reconciled
3,840 jobs per arm across the four profiles.

| Profile | Mesh/Oban throughput | Mesh/Oban pickup p99 | Mesh/Oban CPU per job |
| --- | ---: | ---: | ---: |
| Short, 1 slot | 0.499× | 2.004× | 1.101× |
| No-op, 1 slot | 0.449× | 2.221× | 1.761× |
| Short, 16 slots | 0.803× | 1.190× | 1.324× |
| No-op, 16 slots | 0.729× | 1.309× | 2.716× |

All four profiles missed the throughput limit, and three exceeded the
CPU limit. At 16 slots, the median of trial medians from short-job
handler entry to its first committed write was 7.27 ms on Mesh and
0.37 ms on Oban. That interval includes connection setup on the Mesh
fixture. The watchdog change remains experimental.
[Watchdog receipts](../proof/jobs-parity/qualification-v1.json)

### Measure the effect-write session cost

The compiled fixture used the same two durable SQL writes with a new
session for every pair of writes and with one session reused for the
trial. Five alternating pairs each wrote and reconciled 128 attempt and
effect rows per arm on macOS arm64. PostgreSQL 17.11 used SCRAM and
verified TLS, and the persisted rows recorded 128 server backends in
each fresh-session trial and one in each reuse trial.

With release runtime `1b12243b`, the median of trial medians to the first
committed write was 4.82 ms with fresh sessions and 0.11 ms with reuse.
Opening a fresh session took 4.10 ms. This serial diagnostic runs the
fixture SQL directly. The full Jobs comparison also pays for child
startup, dispatch, and acknowledged settlement.
[Session-cost receipt](../proof/jobs-parity/qualification-v1.json)

The follow-up ran the full short-job workload with one batch runner and
16 slots. Both arms used the same compiled Mesh worker and the same
fixture-only Unix socket relay to libpq. One arm opened a verified
PostgreSQL session per attempt and the other reused 16 sessions. Two
direct Mesh trials bracketed the five alternating pairs. All 1,536
measured jobs completed with one attempt and one effect each.

| Effect route | Entry to first commit | Jobs/s from first claim to final settlement | CPU per job |
| --- | ---: | ---: | ---: |
| Direct Mesh | 7.49 ms | 698 | 6.50 ms |
| Relay, fresh session | 15.00 ms | 496 | 13.50 ms |
| Relay, reused session | 0.37 ms | 1,026 | 6.98 ms |

The reused arm improved first-claim-to-settlement throughput in every
pair. CPU includes the helper and the runner's attempt processes.
The fresh relay incurred more overhead than the direct Mesh path.
These results establish the effect of reuse within the experimental
relay. The direct trials provide a separate reference.
[Relayed session receipt](../proof/jobs-parity/qualification-v1.json)

The native relay repeated the five pairs with Mesh's pinned PostgreSQL
driver in both arms. Reuse increased throughput from first claim to
final settlement from 462 to 772 jobs/s. CPU per job fell from
6.29 to 5.56 ms, including the relay and session setup. The two direct
trials measured 488 jobs/s after first claim and 6.36 ms CPU per job.
The native relay cut the gap from handler entry to the first committed
write from 9.61 to 0.44 ms.

In five additional pairs against Oban Basic 2.24.2, the experimental
reuse path delivered 419 jobs/s against 486, or 86.1% of Oban against
the 90% throughput floor. Pickup p99 was 298 ms against 260 ms, and CPU
per job was 5.41 ms against 5.39 ms. Both met their limits. The sampled
resource limits passed, with the reuse path reaching 20 connections
against a cap of 20. Host load reached 28.7 on 15 logical CPUs.

The medians of trial medians from claim to handler entry were 7.60 ms
for Mesh and 2.09 ms for Oban. The interval from effect commit to
settlement was 3.47 ms against 0.58 ms. These timings apply to the
experimental short-job profile on macOS arm64 and PostgreSQL 17.11.
[Native session receipt](../proof/jobs-parity/native-effect-sessions-v1.json)

### Historical references

On the same macOS arm64 host and PostgreSQL 17.11 configuration, the driver
replayed pre-reliability `db606548` and post-reliability `59f6a7b3` with
release builds and single-row claims. Each revision ran five pairs against
Oban for the one-runner, 16-slot short and sparse profiles and reconciled
760 measured jobs per engine. Both short profiles missed the CPU,
throughput, and pickup limits. Both sparse profiles met their limits.
The receipts preserve the exact inputs, toolchains, and host-load samples
for each run. [Historical receipts](../proof/jobs-parity/qualification-v1.json)

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

### Batch comparison against pinned main

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

The targeted rerun of sparse four-runner, 64-slot pickup used the revised
producer fixture for both runtimes. It reconciled all 120 jobs per runtime
and measured 178.0 ms p99 on the candidate and 184.1 ms on main. Both results
are recorded because the producer and host conditions differed. [Rerun receipt](../proof/jobs-parity/qualification-v1.json)

The full rerun compared release runtime `1b12243b` with `7c162056` on
macOS arm64 and PostgreSQL 17.11 over verified TLS. The dense reference
used 0.227 claim calls per job with batching and 1.016 on main, a 77.7%
reduction. All seven sparse profiles met the 1.10× pickup limit, with a
largest ratio of 1.031×. The sparse rerun reconciled 840 jobs per arm and
used a warm producer in each arm with a fresh database connection per
enqueue transaction. Both reruns used five pairs per profile and verified
zero `pg_stat_statements` evictions. The earlier matrix miss remains part
of the record. [Rerun receipt](../proof/jobs-parity/qualification-v1.json)

The owner replay follow-up used release runtime `a431fbbf` with one runner
and 16 slots on PostgreSQL 17.11 over verified TLS. In five short pairs,
claim calls fell by 65.1%, from 1.008 to 0.352 per job. Five sparse pairs
measured pickup p99 of 252.4 ms versus 172.1 ms on pinned main, above the
1.10× limit. In a separate pre-fence control with the same harness file
hashes, `1b12243b` measured 241.5 ms against 245.9 ms on main. Most of the
tail wait in the follow-up preceded the claim. The two runs had different
main pickup medians, and the cause of the miss remains open.
[Follow-up receipts](../proof/jobs-parity/qualification-v1.json)

The sparse-only retest used the integrated release `aa40fa5d` and the same
pinned main. On PostgreSQL 17.11 over verified TLS, five alternating
one-runner, 16-slot pairs reconciled 120 jobs per arm. Pickup p99 was
261.5 ms versus 258.5 ms, within the 1.10× limit for this profile.
The integrated `c64eee8a` rerun covered all seven sparse profiles and
reconciled 840 jobs per arm. Six profiles met the limit. The one-runner,
one-slot profile measured 237.6 ms versus 195.2 ms, or 1.217× pinned main.
The jobs defining those medians differed by 42.4 ms in pickup time,
including 39.8 ms between enqueue commit and the recorded claim.
The 16-slot profile met the limit in both integrated runs. Host load
varied, and peak one-minute load reached 41.0 on 15 logical CPUs.
The earlier miss and all seven new results remain in the receipt.
[Sparse rerun receipts](../proof/jobs-parity/qualification-v1.json)

The `9bd80627` release rerun on macOS arm64 used PostgreSQL 17.11 with
verified TLS, pinned main `7c162056`, and the frozen 250 ms polling
interval. All seven sparse profiles met the pickup limit. Five pairs per
profile reconciled 840 jobs per arm with no statement-counter eviction.
The largest ratio was 1.089×, and the one-slot profile that previously
missed measured 267.6 ms against 262.0 ms. The dense short reference at
one runner and 16 slots used 0.266 claim calls per job against 1.016 on
main, a 73.8% reduction across five pairs. These comparisons cover the
J2 reference workload and all sparse profiles at that runtime. The
earlier broader dense comparison results remain above.
[Qualification receipt](../proof/jobs-parity/qualification-v1.json) · [Raw trials](../proof/jobs-parity/j2-batch-qualification-9bd80627.tar.xz)

An identical-binary control used `7c162056` for both arms of the
one-runner, one-slot sparse profile. Five alternating pairs gave median
pickup p99s of 263.4 ms and 259.5 ms, with trial ranges of 261.6–269.1 ms
and 208.1–265.3 ms. These results show how individual trials can vary even
when both arms use the same executable and settings. The sparse
candidate retest remains open. [Control receipt](../proof/jobs-parity/qualification-v1.json)

The batch retry at `3f651c2c`, measured at `b81c2819`, uses a bounded retry when an empty
claim can still see eligible work and open capacity. It keeps the
PostgreSQL admission lock, lease checks, and confirmed commit boundary.
The scoped SQL probe passed on PostgreSQL 17.11, 18.7, and 19.0. The
compiled PostgreSQL 17.11 run passed 51 runtime tests and all three suites.
The pinned release also passed the 12 lifecycle faults and the macOS
launch-denial and interrupted-startup proof.

The release comparison used five groups per profile, with a candidate and
two arms running the same `c64eee8a` baseline binary. It reconciled 11,520
jobs on macOS arm64 with PostgreSQL 17.11 over verified TLS. Peak
one-minute host load was 20.0 on 15 logical CPUs.

| Profile | Baseline jobs/s | Repeat jobs/s | Candidate jobs/s |
| --- | ---: | ---: | ---: |
| Short, four runners, 16 slots | 291 | 309 | 534 |
| 100 ms, eight runners, 64 slots | 256 | 238 | 425 |
| Short, one runner, 16 slots | 518 | 529 | 501 |

The candidate exceeded both controls in all five pairs on each
multi-runner profile. The median estimated slot utilization rose from
36–38% to 76% for the short profile and from 52–53% to 89% for the
100 ms profile. Claim calls per job rose from 0.711 to 0.883 and from
0.496 to 0.686, respectively, against the first control. The one-runner
control recorded zero contention hints.

Separate frozen comparisons against Oban Basic 2.24.2 reached 86.3% and
88.1% of its throughput on the two multi-runner profiles. Both met pickup
and resource limits, with throughput below the historical 90% floor.
The current performance gate requires throughput above Oban in every
frozen profile. The short
one-runner dense reference used 0.227 claim calls per job versus 1.023
on pinned main. [Batch retry receipts](../proof/jobs-parity/qualification-v1.json)

### Batch completion experiment

Release `d4746af9` groups already-reaped completions on the lifecycle
session. On macOS arm64 and PostgreSQL 17.11 over verified TLS, 60 measured
trials reconciled 32,640 jobs. Each profile alternated the candidate and
two arms using the same `b81c2819` binary. Peak one-minute load was 25.0
on 15 logical CPUs.

| Profile | Baseline jobs/s | Repeat jobs/s | Candidate jobs/s | Baseline → candidate runner SQL/job |
| --- | ---: | ---: | ---: | ---: |
| Short, one runner, 64 slots | 786 | 781 | 806 | 3.229 → 2.703 |
| No-op, one runner, 64 slots | 731 | 680 | 669 | 3.217 → 2.611 |
| 100 ms, one runner, 64 slots | 315 | 272 | 291 | 3.471 → 2.947 |
| Short, one runner, 16 slots | 355 | 518 | 507 | 3.984 → 3.656 |

The 64-slot short profile's median effect-to-settlement time fell from
7.94 ms to 6.56 ms, with three improvements in five pairs. Throughput
and settlement gains varied by pair and profile, so completion batching
remains an experiment. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Warm effect sessions with multiple runners

The fixture-only relay comparison used `b81c2819` and Oban Basic 2.24.2
on PostgreSQL 17.11 over verified TLS. Five counterbalanced groups per
profile reconciled 12,800 jobs. Each group ran direct writes, an
identical direct repeat, warm relay sessions, and Oban. The warm path
opened one session per slot. Relay CPU and connections count toward the
Mesh totals. Peak one-minute host load reached 36.0 on 15 logical CPUs.

| Profile | Direct jobs/s | Direct repeat jobs/s | Warm jobs/s | Oban jobs/s |
| --- | ---: | ---: | ---: | ---: |
| Short, four runners, 16 slots | 251 | 233 | 360 | 457 |
| 100 ms, eight runners, 64 slots | 305 | 189 | 378 | 416 |

Median handler entry to first commit fell from 13.39 ms to 0.75 ms on
short jobs and from 27.22 ms to 0.85 ms on 100 ms jobs. Warm sessions
beat both direct controls for throughput in four short groups and all
five 100 ms groups. Both profiles met the measured pickup and resource
limits, with throughput below Oban. The relay is a diagnostic fixture.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Deferred scheduler startup in attempts

On PostgreSQL 17.11, the compiled proof for `6745d08d` observed four
threads in an eager attempt and three with deferral enabled. An opted-in
child created its scheduler on the first `Job.async` and completed
`Job.await` in the same process. The runtime test also passed concurrent
first spawns through one scheduler start.

Five groups per 64-slot profile used the same release binary in all three
arms, with deferral enabled only on the candidate. All 15,360 jobs
reconciled. Median CPU fell from 6.50 to 6.28 ms per short job and from
7.42 to 7.02 ms per 100 ms job. The candidate used less CPU than both
controls in four of five groups on each profile. Throughput varied by group,
and sampled peak RSS on the 100 ms profile was between the two controls.
The deferral stays experimental. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

The `603265aa` retest fixed the reply-pipe exit wake on in all three arms
and toggled scheduler deferral only for the candidate. Five groups on the
one-runner, one-slot short profile reconciled 1,920 jobs over verified TLS
on macOS arm64 and PostgreSQL 17.11. Median claim-to-handler-entry time
was 4.71 ms with deferral and 4.98 and 5.09 ms in the two controls. Median
CPU per job was 5.90 ms versus 6.26 and 6.37 ms. Deferral beat both controls
in two groups on each of those metrics and in zero groups on throughput.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Smaller worker image screen

Two macOS arm64 fixture images used the same compiled handler, native
constructor, watchdog, and verified-TLS PostgreSQL writes. The smaller
image omitted the fixture's coordinator entry and unused registrations.
Each of five alternating pairs reconciled 64 worker starts, attempt
writes, and effects per image. The component measures child CPU without
a Jobs runner and samples all 64 children before releasing their effect
writes. It uses a separate blocked-write run for RSS.

| Link | Stock → smaller image | Stock → smaller child CPU/job | Stock → smaller RSS at 64 live children |
| --- | ---: | ---: | ---: |
| Standard | 26.05 → 26.03 MiB | 6.64 → 6.68 ms | 618 → 608 MiB |
| macOS dead stripping | 7.44 → 4.31 MiB | 6.19 → 6.02 ms | 515 → 487 MiB |

The smallest child group alone used substantially more resident memory
than the historical 189 MiB ceiling for the one-runner, 64-slot slow
profile. This local component result supports keeping the current worker
image while investigating the remaining per-attempt costs.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

A local macOS arm64 check held each of the four linked images at its
empty bootstrap pipe after worker entry. One child at a time used
6.2–6.8 MiB of RSS before receiving a job context or opening PostgreSQL.
Closing the empty pipe ended each child through the expected bootstrap
error path.

With the same dead-stripped objects and runtime, the linker's unused
dylib stripping removed Security and retained CoreFoundation. Startup
RSS changed by at most 48 KiB in the three-pair medians. Omitting both
explicit framework arguments left only libSystem as a direct dylib
dependency and lowered the stock image's median from 6,592 to 3,008 KiB
and the smaller image's from 6,384 to 2,784 KiB. The undefined symbol sets
matched. These are bootstrap component measurements on the exact linked
fixtures. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Keep batch claim ordering outside PostgreSQL's context sort

The query in `9bd80627` returns each row's batch position. Mesh validates
the positions and restores the claim and diagnostic order before it
commits the transaction. PostgreSQL no longer sorts the context payloads.
The claim function, role grants, and commit confirmation remain the same.

On PostgreSQL 17.11, 18.7, and 19.0, the repository's worker grants allowed both
queries to return 16 contexts with 512 KiB payloads. With 4 MiB `work_mem`,
the original plan spilled an 8,224 KiB sort. The new plan removed that
sort. The claim function still buffers its result. Temporary block reads
and writes changed from 2,053 and 2,054 to 1,025 and 1,025 on each version.

The macOS release comparison used five groups per profile with two
identical `b81c2819` controls and reconciled 3,120 jobs. On both large-job
profiles, the candidate had lower median claim-to-handler-entry time than
both controls in all five groups. End-to-end throughput beat both controls
in three of five groups on each large-job profile. The short-job control
varied across groups. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

The exact candidate passed 53 Jobs runtime tests and three compiled
suites on each of PostgreSQL 17.11, 18.7, and 19.0 with verified TLS,
including large contexts and scoped workers on fresh and upgraded
schemas. The PostgreSQL 17.11 release build passed all 12 recovery and
commit-loss cases with loopback SCRAM.
[Correctness receipts](../proof/jobs-parity/qualification-v1.json)

The separate five-pair comparison with Oban Basic 2.24.2 on PostgreSQL
17.11 still missed the strict throughput target. Mesh reached 97.5 jobs/s
versus 137.2 at 16 slots, and 152.0 versus 179.0 at 64 slots, with one
runner in both profiles. Pickup p99 also missed its limit at 16 slots.
These are local macOS comparisons. [Receipts](../proof/jobs-parity/qualification-v1.json)

### Release bootstrap buffers after copying the context

In `5d45b53b`, the runner drops its Rust context buffer after flushing
the worker bootstrap. The child drops its Rust copy after copying the
context onto the Mesh heap, before calling the handler.

On PostgreSQL 17.11, the exact checkout passed 53 Jobs runtime tests and
all three compiled Jobs suites, including large contexts and scoped workers.
The release build preserved the job ID and effect key after runner death,
completed on attempt two, and rejected settlement with the old token.
It passed all 12 recovery and commit-loss cases with loopback SCRAM.

On macOS arm64 and PostgreSQL 17.11 with verified TLS, five groups compared
the release candidate with two identical `9bd80627` controls. Each measured
trial held 64 workers with 512 KiB payloads at the same effect write. The
observer verified 64 TLS sessions blocked on the table lock, 64 recorded
attempts, and zero committed effects before releasing the lock. All 960
jobs then reconciled with one attempt and one effect each.

Runner RSS fell from a median 49.0 MiB in both controls to 29.8 MiB. It was
lower than both controls in all five groups. Summed runner and child RSS
fell from 760.9 and 760.8 MiB to 737.6 MiB, lower than both controls in four
groups. The barrier measures residency at equal concurrency. In the separate
unblocked large-job run, median first-claim-to-final-settlement time fell
from 314 and 324 ms to 260 ms, while measured throughput fell from 160 and
168 to 133 jobs/s. Local measurements leave the throughput gate open.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Reply-pipe exit wake experiment

The opt-in `ccf62ff1` experiment sends a second supervisor wake when the
worker's reply pipe reaches EOF. The child leaves the pipe open through
process exit. Reaping still gates acknowledgement, and the 10 ms timed scan
covers missed or early hints.

Five groups per profile used one release binary, two flag-off controls,
and PostgreSQL 17.11 over verified TLS on macOS arm64. All 3,840 measured
jobs reconciled. With one runner and one slot, median effect-commit-to-
settlement fell from 9.21 and 8.39 ms to 1.17 ms. Throughput increased from
38.1 and 39.8 to 56.1 jobs/s. Both metrics improved against both controls
in all five groups. At 16 slots, throughput beat both controls in two
groups and median settlement latency rose to 16.57 ms from 10.17 and
11.53 ms. The host's one-minute load peaked at 47.95 on 15 logical CPUs.

The combined opt-in `da0a51bc` candidate also drops the bootstrap buffers.
In five one-slot short-job pairs against Oban Basic 2.24.2, Mesh reached
59.2 versus 91.3 jobs/s. Pickup p99 was 2,125 versus 1,389 ms, and CPU per
job was 6.22 versus 3.99 ms. All three limits were missed. Mesh took a
median 4.72 ms from claim to handler entry and 4.44 ms from entry to the
first durable write, versus 0.69 and 0.21 ms for Oban. The combined release
passed the 12 recovery and commit-loss cases. Linux and dense refill
resource behavior remain unverified. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

A time-budget calculation from the five saved pairs subtracts each job's
entry-to-first-write interval while holding all other recorded time fixed.
The resulting median would be 87.9 jobs/s against Oban's measured 91.3.
Substituting the corresponding Oban intervals for both claim-to-entry and
entry-to-first-write would yield about 141 jobs/s under the same assumption.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Combine the exit wake with effect-session reuse

Five counterbalanced groups ran the frozen one-runner, one-slot short
profile on macOS arm64 and PostgreSQL 17.11 with verified TLS. All Mesh
arms used the same `da0a51bc` release fixture with the reply-pipe wake
enabled. The relay used the PostgreSQL driver from that same release.
Across 2,560 jobs, the trial reconciled every attempt and effect after
shutdown. Both writes confirmed 128 distinct backends per fresh-session
trial and one per reused-session trial.

| Arm | Jobs/s | Entry to first write | Pickup p99 | CPU/job |
| --- | ---: | ---: | ---: | ---: |
| Mesh direct | 60.6 | 5.53 ms | 2,085 ms | 7.13 ms |
| Mesh relay, fresh sessions | 60.0 | 5.01 ms | 2,110 ms | 6.54 ms |
| Mesh relay, reused session | 95.1 | 0.28 ms | 1,334 ms | 5.64 ms |
| Oban Basic 2.24.2 | 83.6 | 0.35 ms | 1,520 ms | 5.34 ms |

The reused arm beat Oban on throughput in all five groups. Its CPU total
includes the helper and prewarming. Both relay arms require the runner,
the relay, and one isolated attempt process to overlap. This exceeds the
two-process limit even when the sampler sees only two. The comparison
supports testing reuse inside the runner process with the same isolated
attempt and durable writes. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

The same four arms ran five groups on the frozen one-runner, 64-slot
100 ms profile. All 10,240 jobs reconciled. The warm relay opened 64
verified sessions and used each one. The median first-write interval
fell from 10.55 ms with fresh relay sessions to 0.31 ms with reuse.

| Arm | Jobs/s | CPU/job | Sampled peak RSS |
| --- | ---: | ---: | ---: |
| Mesh direct | 414.9 | 7.43 ms | 444.0 MiB |
| Mesh relay, fresh sessions | 364.9 | 7.11 ms | 341.3 MiB |
| Mesh relay, reused sessions | 410.5 | 6.14 ms | 399.6 MiB |
| Oban Basic 2.24.2 | 490.7 | 1.95 ms | 118.1 MiB |

The warm relay missed throughput in all five groups and exceeded the
relative CPU and RSS limits. Two warm trials sampled 64 live isolated
workers and the two coordinator processes, with tree RSS of 503.9 and
504.1 MiB. The trial peaks span different active worker counts, so their
medians describe each workload's observed peak. The 64-worker samples
show that the RSS limit still requires a smaller worker footprint on this
host. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Attribute memory with all 64 workers at the same write

The same fixture sources and toolchain were linked three ways and ran two
counterbalanced cohorts per effect route. Every capture independently
observed 64 attempt children, 64 committed attempt identities, and 64
verified-TLS effect writes waiting on the observer's table lock. The
producer committed before runner startup
to leave room for the observer under the existing database connection cap.
The harness released the gate and reconciled all 768 jobs.

| Fixture link | Direct tree RSS | Reused tree RSS | Reused RSS per worker |
| --- | ---: | ---: | ---: |
| Standard | 625.0 MiB | 503.1 MiB | 7.43 MiB |
| Dead strip, both frameworks | 527.1 MiB | 464.0 MiB | 6.85 MiB |
| Dead strip, omit both frameworks | 297.5 MiB | 235.8 MiB | 3.34 MiB |

All values are medians of the first held snapshot in each cohort. The
reuse totals include the relay and the runner. The linker trace and
Mach-O dependency list confirm the omitted-framework build uses only
libSystem as a direct dylib dependency. The 235.8 MiB held total remains
above the earlier 177.1 MiB comparison limit. The `vmmap` files preserve
separate per-process physical-footprint, stack, and heap estimates for
the two sampled jobs in each cohort.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

Five unheld groups then ran the frozen 64-slot slow profile using the
framework-omitting fixture. All 10,240 jobs reconciled. The reused arm
measured 425.4 jobs/s, 3.90 ms CPU per job, and 178.4 MiB sampled peak RSS.
Oban measured 498.2 jobs/s, 1.67 ms CPU per job, and 118.3 MiB sampled
peak RSS. The reused arm met the pickup limit and missed throughput,
CPU, and RSS. Its RSS was about 1 MiB above that run's 1.5× limit.
The held measurement and the unheld benchmark have separate receipts.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Attribute CPU on the frozen 64-slot slow profile

The same framework-omitting fixture recorded its own CPU and reaped
child CPU after joining the runner and closing the pool. The sum agreed
with the runner's external `wait4` CPU within 0.48 ms per trial. The
separate helper's CPU includes session prewarming. Five counterbalanced
groups reconciled 10,240 measured jobs on macOS arm64 with PostgreSQL
17.11. The host's one-minute load reached 15.20 on 15 logical CPUs.

| Arm | Jobs/s | Runner CPU/job | Child CPU/job | Helper CPU/job | Total CPU/job |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mesh direct | 431.7 | 0.71 ms | 4.33 ms | | 5.04 ms |
| Mesh with fresh relay sessions | 410.4 | 0.74 ms | 3.30 ms | 0.87 ms | 4.87 ms |
| Mesh with reused relay sessions | 409.4 | 0.65 ms | 3.01 ms | 0.22 ms | 3.89 ms |
| Oban Basic 2.24.2 | 440.2 | | | | 1.65 ms |

Each column is the median across the five trials. CPU totals exclude the
producer and PostgreSQL. In the reuse arm, the children accounted for a
median 77.6% of total CPU, including 1.85 ms/job of system CPU. The CPU
limit for the full Mesh tree was 2.06 ms/job. Reuse met the pickup and
sampled RSS limits in this run and missed throughput and CPU. The CPU
breakdown measures process ownership. It leaves the system CPU split
among executable loading, thread startup, and I/O unmeasured.
[Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

### Measure the worker launch prefix

Two local diagnostics compared a small libSystem executable with four
cutoffs in one framework-omitting Mesh fixture linked against
`da0a51bc`. Both used five counterbalanced groups, 512 children per arm,
and waves of 64. Every child returned its expected exit status. The last
cutoff exercised the real empty-bootstrap pipe error.

| Child CPU/job | Fork and exec | `posix_spawn` |
| --- | ---: | ---: |
| Small executable, exit at main | 1.60 ms | 1.29 ms |
| Mesh executable, exit at main | 2.25 ms | 1.75 ms |
| Mesh worker, exit on bootstrap EOF | 2.07 ms | 1.94 ms |
| Paired Mesh-minus-small difference at main | 0.44 ms | 0.44 ms |

The paired `posix_spawn` difference ranged from 0.36 to 0.48 ms/job.
Every group fell below the 0.9 ms/job screening threshold for a
linker-ordering experiment. The paired system CPU difference was
0.05 ms/job. The 1.94 ms/job prefix is close to the 2.06 ms/job budget
observed for the entire runner tree in the separate 64-slot workload.
The cutoff occurs before a handler runs or either child watchdog starts.
It measures child CPU on this macOS host. The paired stages use an
inherited diagnostic descriptor and do not exercise PostgreSQL or
measure completed jobs/s.
[Launch screen receipt](../proof/jobs-parity/qualification-v1.json)

### Fork a fresh child before worker initialization

A single-threaded native launcher ran five alternating pairs of 512
children on macOS arm64. Both arms used the same framework-free fixture
image and 64-wide waves. The launcher invoked the fixture's native
startup code inside each child before entering the original generated
Mesh main. The linker audit found two pre-main AWS-LC initializers for
CPU capabilities and cleanup registration. The launcher checked one
thread before creating children. Every reaped PID matched its native
startup marker and the expected bootstrap EOF exit.

| CPU/job | Fresh exec | Forked child |
| --- | ---: | ---: |
| Children | 2.06 ms | 0.49 ms |
| Launcher and reap | 0.19 ms | 0.34 ms |
| Combined | 2.25 ms | 0.83 ms |

The median paired saving was 1.42 ms/job, with a range of 1.34 to 1.48.
The forked arm fell below the 1.19 ms/job screening ceiling in all five
pairs. That ceiling came from the CPU budget left after runner and relay
costs in the earlier 64-slot slow comparison. This prototype exits before
reading a valid handler context. Handler and watchdog work, PostgreSQL
effects, and the extra process and IPC for a template in a real runner
remain unmeasured. The exact fixture run leaves arbitrary application
initializers and cross-platform fork safety open.
[Fork screen receipt](../proof/jobs-parity/qualification-v1.json)

The macOS handoff probe passed eight repetitions of five cases using an
anonymous Unix socket and a runner-owned control pipe. A live template
published the exact child's status after reaping it. Closing the control
pipe before the grant suppressed the application-start marker. Closing
it after the grant stopped the test child. After template death, the
runner observed child exit through kqueue and received `ECHILD` from
`waitpid`. A runner with that topology cannot derive a reap receipt
from a child exit notification. The probe used a synthetic child marker.
Removing the pre-launch grant check or letting the child keep the
template's management socket caused the corresponding case to fail.
[Handoff probe receipt](../proof/jobs-parity/qualification-v1.json)

### Reap Linux attempts after template loss

A local ARM64 Linux 6.12.115 guest on the Mac ran the ownership fixture
with `clone3` required. Six boots produced identical results. A dedicated
runner thread reaped waiting, stopped, and already exited children after
the template died before sending their pidfds. It also reaped a granted
child through its exact pidfd after template death. Closing the gate before
grant prevented the test child from entering its work.

A separate probe put an unrelated child owned by another thread in the
template's process group. The runner's group-scoped `waitid` with
`__WNOTHREAD` left that child's exit status for its owning thread. Removing
the thread filter exposed the unrelated child to the cleanup scan.
Removing `CLONE_PARENT` broke the pre-transfer reap case.
[Linux ownership receipt](../proof/jobs-parity/qualification-v1.json)

The follow-up ran a compiled Jobs worker in the same Linux guest. Six
boots each passed three cases. Template death before pidfd transfer
left no constructor, handler, or reply output. After transfer and grant,
`execve` preserved the attempt PID through the native constructor and
the compiled handler returned its exact framed reply. Closing the
control pipe during a separate running handler produced native exit
173 with no reply. Both granted cases consumed the exit status through
the runner's exact pidfd. Three further boots added short handler and
lease deadlines with the control writer held open. The same worker
entered the handler, then exited 171 and 172 respectively without a
reply. The runner consumed both statuses through the exact pidfds.
The proof used the normal worker startup ABI and a cached Linux debug
runtime. Database claims, settlement, and throughput remain open for
this launcher.
[Compiled Linux ownership receipt](../proof/jobs-parity/qualification-v1.json)

### Run the stock release dispatcher in the Linux guest

The normal Jobs dispatcher at `a39447f7` ran in the same ARM64 guest
with a cross-built Rust 1.99.0 release runtime and PostgreSQL 17.11.
The guest used the Ubuntu Base 26.04.1 loader and libraries for the
cached PostgreSQL build. The fixture used batch claims and an isolated
database with verified loopback TLS and SCRAM.

Three fresh boots each committed two enqueue receipts and one business
row. Both jobs completed on attempt one. An independent SQL connection
matched both job IDs, the effect keys, the two distinct worker PIDs, and
the final lease fields. The native startup markers matched those PIDs.
The runner reported two completions and shut down drained. This smoke
exercised the stock launch and settlement path and recorded no throughput
comparison. [Release Linux smoke receipt](../proof/jobs-parity/qualification-v1.json)

### Measure the Linux supervisor wait after a worker reply

The one-slot comparator ran a single compiled release worker as a direct
child. Both arms used the same `posix_spawn`, opened a pidfd, and
required the exact successful child reap and reply. Arm A checked exit
on reply and on a 10 ms sweep. Arm B also checked exit when the pidfd
became readable. Each attempt verified the native constructor PID and
process group, the handler context, and the reply frame. An unrelated
child remained waitable by its owner.

Two guest runs each measured five alternating pairs of 128 attempts per
arm after warm-up, reversing the initial order. The median paired
reduction in mean slot-ready time was 8.91 ms and 6.77 ms. Arm B was
faster in all ten pairs. Timed sweeps supplied 79–80% of exact reaps in
Arm A and 0.6–4.2% in Arm B. A separate five-pair comparison with the
same policy in both arms had paired differences of −1.12 to +0.41 ms.
The Mac's one-minute load ranged from 21.99 to 29.70 on 15 logical CPUs.

Child `wait4` and active comparator CPU averaged 2.93 → 2.12 ms per
attempt in the first run and 1.13 → 0.65 ms in the second. Both arms
opened the pidfd, and the comparator omitted the runner's channels,
dispatcher and PostgreSQL. The result isolates the modeled wait after
reply. A real-runner comparison must measure acquisition and descriptor
pressure as well as enqueue-to-settlement throughput.
[Linux exit-wake receipt](../proof/jobs-parity/qualification-v1.json)

The real-runner follow-up used one release binary at `03a7a024` with
`MESH_DIAGNOSTIC_JOB_PIDFD_WAKE` set to 0 or 1. The enabled output
reader sends the ordinary reply wake and can send a second wake when
the exact child's pidfd becomes readable. A separate held-worker smoke
observed two pidfds for two live children. Both jobs later settled.

On `short-1r-1s`, five alternating pairs and two identical-policy
pairs reconciled 1,792 measured jobs over PostgreSQL 17.11 verified
TLS and SCRAM. The median of trial medians from effect commit to
settlement fell from 4.63 to 1.74 ms. The enabled arm improved this
interval in all five pairs, with paired savings of 1.28–4.80 ms.
Enqueue-to-settlement throughput medians were 16.0 and 17.1 jobs/s.
The median paired difference was +0.36 jobs/s, within the −1.64 to
+2.26 jobs/s range in the identical-policy control. CPU and throughput
remain inconclusive under this load. The handler still took 46.6–46.7 ms
from entry to its first committed write.

The two-vCPU guest ran on a Mac whose one-minute load ranged from 18.21
to 24.45 on 15 logical CPUs. The optimized worker disables process
dumpability, so the unprivileged timing observer left FD peaks unreported.
The smoke used a separate guest-root observer for the pidfd witness.
Descriptor pressure, failure qualification and comparison to Oban
remain open. [Full-runner Linux receipt](../proof/jobs-parity/qualification-v1.json)

### Remove Linux PostgreSQL startup buffering

The Linux `short-1r-1s` diagnostic used one release binary at `6f4ee72c`
with the pidfd wake disabled in both arms. It changed only whether
`TCP_NODELAY` was set on Mesh's PostgreSQL sockets before TLS startup.
The parity driver ran five alternating pairs and two identical-policy
pairs, with warmups excluded. All 1,792 measured jobs reconciled over
PostgreSQL 17.11 with verified TLS and SCRAM.

Median throughput rose from 15.4 to 72.0 jobs/s. The change improved all
five pairs by 27.5–59.4 jobs/s. The disabled-policy control differed by
−0.64 to +0.45 jobs/s. Median handler entry to the first committed write
fell from 50.0 to 4.9 ms, with paired savings of 41.3–47.6 ms. The host's
one-minute load ranged from 14.90 to 23.92 on 15 logical CPUs. The trial
measures the socket option's effect on the real Jobs path. It covers
one profile on a two-vCPU ARM64 guest and compares two Mesh configurations.
[Linux socket receipt](../proof/jobs-parity/qualification-v1.json)

The stock Linux connector now sets `TCP_NODELAY` before registering the
socket and starting TLS. The release runtime at `b67206f4` passed two
fresh PostgreSQL 17.11 guest smokes with the normal Jobs worker launch.
Each run independently reconciled the two enqueue receipts and worker
effects and observed a drained runner. The focused Linux driver suite
passed 37 tests, including the socket-option assertion and the existing
SCRAM handshake tests. [Stock Linux receipt](../proof/jobs-parity/qualification-v1.json)

The same Linux guest setup compiled Oban Basic 2.24.2 offline with
Erlang/OTP 29.1.1, Elixir 1.20.4, and the fixture's eight locked Hex
packages. With 1 GiB of guest memory and two vCPUs, the repository
driver reconciled four jobs over PostgreSQL 17.11 verified TLS and
SCRAM. The runner peaked at two processes, 12 threads, and three
connections, within the one-slot limits. This smoke establishes that
both release subjects can run in the same local guest.
[Offline Oban smoke receipt](../proof/jobs-parity/qualification-v1.json)

In the paired `short-1r-1s` run in that guest, the stock release Mesh
dispatcher and Oban Basic each completed 640 measured jobs. The driver
excluded a warm-up for each engine, alternated five pairs, and reconciled
the input, transaction, attempt, effect, and terminal rows. Both engines
used PostgreSQL 17.11 with verified TLS and SCRAM.

Mesh won four of five throughput pairs. Median throughput was 76.55
jobs/s for Mesh and 75.94 for Oban. The trial ranges were 59.68–97.48
and 45.94–84.53 jobs/s. Median pickup p99 was 1,649 versus 1,671 ms.
Runner-tree CPU per job was 3.68 versus 6.75 ms and median peak RSS was
34.0 versus 115.4 MiB. The summary recorded zero budget breaches for
this profile. The median throughput margin was 0.8%, and the Mac's
one-minute load was 9.91 before and 14.04 after the run. This local
result covers one profile on a two-vCPU guest.
[Paired Linux receipt](../proof/jobs-parity/qualification-v1.json)

The `short-1r-16s` comparison reconciled another 640 measured jobs per
engine. Mesh and Oban recorded median throughput of 234.44 and 728.95
jobs/s, with Oban ahead in all five pairs. Median pickup p99 was 525.49
and 169.46 ms. Throughput and pickup missed the relative limits.
CPU per job, peak RSS and the absolute resource budgets passed.

Median time from enqueue commit to the first recorded claim was 170
versus 79 ms. The first claim to final settlement took 378 versus
91 ms. The per-attempt medians for claim to handler entry, handler
entry to first committed write, and effect commit to settlement were
11.15, 11.14, and 10.54 ms for Mesh, versus 1.50, 0.68, and 0.82 ms
for Oban. These intervals locate the remaining delay across dispatch,
the handler's database write path, and settlement.
[16-slot Linux receipt](../proof/jobs-parity/qualification-v1.json)

### Separate first pickup from backlog drain on Linux

The two-vCPU Linux guest ran the frozen `short-1r-16s` workload with
PostgreSQL 17.11 over verified TLS and SCRAM. Each of three runs used
five alternating pairs after warmup and reconciled 640 jobs per arm.
The Linux socket setting was enabled in every arm.

The pidfd follow-up at `6f4ee72c` measured 226.95 jobs/s with the wake
disabled and 225.85 enabled. The enabled arm won two pairs. The
identical-policy control varied from −37.72 to +38.63 jobs/s, and the
pidfd median paired change was −7.60 jobs/s. Effect commit to settlement
fell from 11.40 to 10.01 ms on the arm medians. This result leaves the
throughput effect inconclusive.

On the stock `b67206f4` binary, enabling `LISTEN` and transactional
enqueue notifications reduced median time to the first claim from
177.79 to 4.21 ms. The paired reduction ranged from 100.45 to 235.10 ms.
Throughput medians were 123.24 and 184.74 jobs/s, with the enabled arm
faster in each pair. First-claim-to-final-settlement time improved in
two pairs. [Linux wake diagnostics](../proof/jobs-parity/qualification-v1.json)

In a separate matched run with notifications enabled, Mesh reached
228.11 jobs/s against Oban's 546.54 and missed the relative pickup limit.
Mesh took a median 2.87 ms to make its first claim and 558.26 ms to drain
the backlog. Oban took 125.13 and 138.19 ms respectively. The Mac's
one-minute load was 32.59 before and 22.10 after the run. These receipts
support the opt-in wake path for first pickup and leave backlog drain
as the dominant gap on this profile.
[Notification comparison](../proof/jobs-parity/qualification-v1.json)

### Overlap confirmed batch launches with settlement on Linux

Release prototype `70752964` used the existing batch claim thread to
start confirmed attempts while the dispatcher settled other attempts.
The two-vCPU guest ran the frozen `short-1r-16s` workload with verified
TLS and SCRAM, 250 ms polling, and the listener disabled in both arms.
Five alternating off/on pairs and five flag-off control pairs each
reconciled 640 jobs per arm after warmup.

The median paired throughput gain was 4.9%, with gains in all five
pairs. The arm medians were 124.37 and 125.09 jobs/s. The flag-off
controls varied by −51.45 to +88.06 jobs/s across pairs, compared with
+0.72 to +18.01 jobs/s for off/on. First claim to final settlement
improved in four pairs, with a median paired reduction of 13.9%.
Claim calls per job fell in all five pairs, from a median of 0.180 to
0.117. The throughput signal was within the control variation. The
prototype remains outside the product branch.
[Launch overlap receipt](../proof/jobs-parity/qualification-v1.json)

The 64-slot short-job follow-up reconciled 2,560 jobs per arm in each
comparison. Overlap reduced claim-to-entry and effect-to-settlement
time in all five pairs. Median paired savings were 20.52 ms and 18.14 ms
respectively. Throughput rose 21.8% within the median pair. The
flag-off control rose 37.5% within its median pair during the same noisy
window. The stage timings support overlap, and the control limits the
end-to-end conclusion. The exact candidate passed 14 runner unit tests
on ARM64 Linux, including queue restart fencing and adoption after a
claim-thread panic. Four database-backed runner tests were ignored.
[64-slot and runner test receipts](../proof/jobs-parity/qualification-v1.json)

### Reuse effect-write sessions on Linux

The frozen `short-1r-16s` workload ran on the same two-vCPU guest with
verified PostgreSQL 17.11 TLS and SCRAM. Five counterbalanced groups used
one worker image built against stock `b67206f4`. Each arm reconciled
640 measured jobs and both durable writes per job after warmup.

| Effect route | Median jobs/s | Entry to first write commit | CPU/job |
| --- | ---: | ---: | ---: |
| Direct from the isolated child | 79.1 | 27.71 ms | 8.21 ms |
| Same-driver relay with fresh sessions | 178.7 | 14.87 ms | 3.57 ms |
| Same-driver relay with 16 prewarmed sessions | 337.7 | 0.78 ms | 2.20 ms |

The warm relay shortened first-write time in all five fresh-versus-warm
pairs and improved throughput in four. Its median throughput gain
within pairs was 218.2 jobs/s. The session receipts confirm 128 distinct
backends per fresh trial and 16 prewarmed backends per warm trial. The
warm route reached the sampled 20-client budget. CPU includes the relay
and its prewarm. Host one-minute load moved from 16.16 to 39.99 on 15
logical CPUs. This experiment measures a private fixture route and its
session reuse. Product lease fencing remains open.
[Linux session receipt](../proof/jobs-parity/qualification-v1.json)

The same-window comparison with Oban Basic 2.24.2 reconciled 1,920
measured jobs across five groups. The two identical warm Mesh arms
reached median throughput of 440.9 and 464.6 jobs/s against Oban's
534.7. Both missed the strict throughput condition and met the other
recorded profile limits. Median CPU was 1.37 and 1.34 ms/job for the
warm arms and 3.82 for Oban. Both warm arms reached the 20-client cap.

Median time from enqueue commit to first claim was 171.1 and 167.8 ms
for Mesh and 142.7 ms for Oban. From first claim to final settlement,
the medians were 119.2, 106.7, and 105.0 ms. The listener and producer
publication were disabled in both Mesh arms. The two warm arms differed
by −103.3 to +52.1 jobs/s across paired groups.
[Warm Mesh and Oban receipt](../proof/jobs-parity/qualification-v1.json)

### Combine warm effect sessions and notification pickup on Linux

An ARM64 Linux guest ran five counterbalanced groups of the frozen
`short-1r-16s` workload with PostgreSQL 17.11 over verified TLS and
SCRAM. Both Mesh arms used the same compiled worker and 15 prewarmed
effect sessions for 16 slots. The enabled arm added one listener and
published hints from the enqueue transaction. Each arm reconciled 640
measured jobs and both durable writes per job after warmup.

| Arm | Median jobs/s | Range | Pickup p99 | CPU/job | Peak client count |
| --- | ---: | ---: | ---: | ---: | ---: |
| Mesh with listener off | 321.0 | 291.5–359.3 | 388.1 ms | 2.20 ms | 19 |
| Mesh with listener on | 543.7 | 518.4–633.4 | 227.6 ms | 2.47 ms | 20 |
| Oban Basic 2.24.2 | 430.2 | 258.3–740.9 | 291.5 ms | 8.14 ms | 20 |

The enabled arm beat the disabled arm in all five groups and Oban in
four. It exceeded Oban's median throughput by 26.4% and met every
measured profile limit. Median time from durable enqueue to first claim
fell from 222.9 to 2.7 ms with the listener. Each enabled trial verified
the registered listener, a notification-triggered scan, and both writes
for each job on a prewarmed backend. Host one-minute load ranged from
19.0 to 34.9 on 15 logical CPUs. The pinned Oban fixture used its
in-VM notifier and a 250 ms staging poll, so the separate enqueue
process could not wake its runner across VMs. From first claim to final
settlement, medians were 232.7 ms for listener-on Mesh and 150.7 ms for
Oban. The effect relay is private fixture code. The receipt covers one
frozen profile with that route.
[Warm sessions and notification receipt](../proof/jobs-parity/qualification-v1.json)

### Match PostgreSQL notifications in the 16-slot control

A separate five-group Linux diagnostic compared the same warm Mesh
image with both pinned Oban Isolated and Oban's PostgreSQL notifier.
The latter used 17 pooled runner connections and a dedicated listener.
The producer's notification was observed inside its enqueue transaction.
Each arm reconciled 640 measured jobs under the 20-client budget.

Warm Mesh reached a median 1,109.0 jobs/s (537.4–1,226.0). Oban Isolated
reached 445.1 (432.3–847.5), and Oban with PostgreSQL notifications
reached 1,120.6 (911.2–1,386.9). Mesh won two of five pairs against
the PostgreSQL notifier and met the other measured limits. Its
throughput median was 1.0% lower. The observed ranges overlap.

Median commit-to-first-claim time was 1.6 ms for Mesh, 165.8 ms for Oban
Isolated, and 7.2 ms for Oban with the PostgreSQL notifier. The alternate
notifier changes the comparison configuration. The frozen comparator
and release qualification rule remain in force.
[Matched notification control receipt](../proof/jobs-parity/qualification-v1.json)

### Test launch overlap with warm sessions and notifications

Five counterbalanced groups compared serial and overlapping launches
from the same `70752964` release fixture. Both Mesh arms used 15 warm
effect sessions and PostgreSQL notifications. Oban Basic 2.24.2 used
its PostgreSQL notifier. All three arms reconciled 640 measured jobs
each under the 20-client workload budget on the two-vCPU Linux guest.

Serial Mesh reached a median 694.6 jobs/s (270.8–727.1), and overlap
reached 633.1 (344.9–790.0). Overlap improved throughput in one of five
pairs. Oban reached 862.1 (849.6–966.1). Both Mesh arms missed the
throughput and pickup conditions in this comparison. The time from
enqueue commit to first claim was 1.6 ms for serial launches and 2.8 ms
with overlap. Median time from first claim to final settlement was
182.7, 200.4, and 134.9 ms for serial Mesh, overlap Mesh, and Oban. The
overlap prototype stays outside the product branch.
[Warm launch overlap receipt](../proof/jobs-parity/qualification-v1.json)

### Trace the warm 16-slot Linux worker lifecycle

Five alternating pairs used the same `1abc0b2c` release fixture and
serial dispatcher with tracing off and on. Each arm completed and
reconciled 640 measured jobs using 15 warm effect sessions, PostgreSQL
notifications, and a 250 ms fallback. The local ARM64 guest had two
vCPUs and 1 GiB of RAM. Host one-minute load ranged from 6.11 to 9.23.

| Stage | Median of five traced trial medians |
| --- | ---: |
| Confirmed claim to launch | 1.93 ms |
| Spawn call | 0.29 ms |
| Spawn return to handler entry | 0.35 ms |
| Reply frame written to observed reap | 0.73 ms |
| Observed reap to settlement submission | 1.14 ms |
| PostgreSQL settlement call through confirmation | 0.14 ms |

The first batch member launched within 0.03 ms in each trial's median.
The 13 jobs at ordinal 16 gave trial medians from 5.73 to 10.60 ms.
This run places more of the observed wait in dispatch and lifecycle
scheduling than in the PostgreSQL acknowledgement call. The trace
itself slowed four of five pairs. Median throughput was 1,283 jobs/s
with tracing off and 1,163 with it on, with overlapping ranges of
1,009–1,408 and 750–1,371 jobs/s. The reap timestamp records the
supervisor observing the child's wait status. Both arms used Mesh.
[Warm lifecycle trace receipt](../proof/jobs-parity/qualification-v1.json)

### Measure stock Linux Jobs with 64 slow-job slots

The frozen `slow-1r-64s` workload ran five alternating pairs against
Oban Basic 2.24.2. Stock Mesh `b67206f4` used batch claims, direct effect
connections, and a 250 ms poll. PostgreSQL 17.11 used verified TLS and
SCRAM in the two-vCPU ARM64 guest. Each engine reconciled 2,560 measured
jobs with one attempt and one durable effect per job.

The first boot with 1 GiB of RAM ended before the benchmark emitted a
receipt. The identical guest archive completed with 2 GiB of RAM for both
engines and the same application limits. Mesh reached a median 245.6
jobs/s, ranging from 201.8 to 271.7. Oban reached 389.1 jobs/s, ranging
from 228.0 to 448.2. Mesh won one pair. It missed throughput and pickup
but met the relative CPU limit at 2.30 versus 1.99 ms/job.

The median sampled peak RSS was 557.4 MiB for Mesh and 153.8 MiB for
Oban, above the 230.7 MiB relative limit. The median of each Mesh trial's
peak-RSS samples included 49 processes and 518.5 MiB of descendant RSS.
These figures identify the attempt processes as the main source of
measured memory. They leave the contribution of per-attempt PostgreSQL
sessions open. [64-slot slow-job receipt](../proof/jobs-parity/qualification-v1.json)

### Isolate connection reuse on the 64-slot Linux slow profile

Five counterbalanced groups ran the same slow-job inputs on the two-vCPU,
2 GiB ARM64 guest. The direct and warm Mesh arms used one release image
and the same wrapper. The warm arm reused 64 verified PostgreSQL
sessions. All three arms reconciled 2,560 measured jobs each. The host's
one-minute load ranged from 26.5 to 42.4 on 15 logical CPUs.

| Arm | Median jobs/s | Range | First write commit after entry | Peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Mesh direct | 137.5 | 94.1–189.0 | 21.44 ms | 404.5 MiB |
| Mesh warm | 280.6 | 219.6–420.9 | 1.11 ms | 601.4 MiB |
| Oban Basic 2.24.2 | 345.3 | 268.3–471.3 | 3.12 ms | 147.8 MiB |

The warm arm improved throughput in all five Mesh pairs and beat Oban
in one. It missed throughput, pickup, and RSS against Oban. The warm
arm's median sampled process peak was 56 versus 36 with direct writes.
This comparison shows that reuse removes first-write delay and leaves
a substantial memory budget to close as more attempt children stay
active. The RSS metric sums process residency, including pages mapped
by several processes. [Warm 64-slot receipt](../proof/jobs-parity/qualification-v1.json)

A separate one-group boot read `/proc/PID/status` for live attempt PIDs
that matched durable writes. Across nonempty samples, median anonymous
RSS per covered child was 6.53 MiB for direct writes and 6.49 MiB for
warm sessions. Shared-memory RSS was 4.23 and 3.69 MiB. The census
covered 420 of 512 direct children and 252 of 512 warm children at least
once. This diagnostic used a separate boot because the added reads can
affect timing. The samples identify anonymous memory as a sizable part
of the remaining worker footprint.
[Linux residency receipt](../proof/jobs-parity/qualification-v1.json)

### Measure transparent huge pages in Linux attempt children

A separate diagnostic captured five `/proc/self/smaps` snapshots for
each of 64 direct and 64 warm effect workers using the pinned release
runtime. Every PID matched its durable attempt and effect. Median warm
anonymous RSS grew from 0.11 MiB at the native constructor to 2.40 MiB
at handler entry and 6.41 MiB after the 100 ms hold.

A follow-up used the same fixture image and 64 prewarmed sessions in
both arms. Each worker verified its process-local huge-page setting in
the constructor. After the hold, median anonymous RSS was 6.41 MiB
with the guest's default, including 6.00 MiB of anonymous huge pages.
Disabling huge pages reduced anonymous RSS to 0.42 MiB and total RSS
from 10.17 to 4.18 MiB per worker. The two arms reconciled all 128
jobs. The snapshots were taken in separate diagnostic runs. They
establish the source of most anonymous residency on this guest.
[Phase and huge-page receipts](../proof/jobs-parity/qualification-v1.json)

### Measure worker-local huge-page policy under the slow workload

Five counterbalanced groups compared the same Mesh fixture with its
worker-local huge-page setting toggled, alongside frozen Oban Basic
2.24.2. All 7,680 measured jobs reconciled. Both Mesh arms used 64 warm
effect sessions, 250 ms polling, and no listener. The ARM64 guest had
two vCPUs and 2 GiB of RAM. PostgreSQL 17.11 used verified TLS and SCRAM.
The timed runs omitted the per-worker memory probes.

| Median metric | Mesh, guest default | Mesh, worker THP disabled | Oban |
| --- | ---: | ---: | ---: |
| Sampled peak tree RSS | 628.9 MiB | 324.3 MiB | 147.5 MiB |
| Durable jobs/s | 369.4 | 394.6 | 423.6 |
| Pickup p99 | 1,268 ms | 1,191 ms | 1,101 ms |
| Runner-tree CPU/job | 1.79 ms | 1.60 ms | 1.82 ms |

Worker-local THP disablement lowered peak RSS in all five groups and
still exceeded the 221.3 MiB limit. The sampler sums RSS by process and
counts shared mapped pages in each process that reports them. Throughput
ranges overlapped at 222–505, 283–496, and 371–498 jobs/s, respectively.
The disabled arm won three of five groups against default Mesh and two
against Oban. Pickup, CPU, and absolute resource limits passed. The
throughput and RSS gates for this profile remain open.
[Timed huge-page receipts](../proof/jobs-parity/qualification-v1.json)

The separate `242702e1` runtime prototype passed eight startup cases in
the local ARM64 guest using a release fixture linked against that
runtime. Only the exact internal worker invocation with the opt-in set
to `1` reported `THP_enabled = 0` after scheduler startup. Unset and
nonmatching values, ordinary application launches, and worker-like
argument lists reported `THP_enabled = 1`. A second run launched two
compiled jobs through the runner on PostgreSQL 17.11 with verified TLS
and SCRAM. The runner reported `THP_enabled = 1`, and both children
reported `0` while their handlers were active. The observed PIDs matched
the native start markers and both durable attempts. Both jobs completed
at attempt one with one effect and one receipt each. The timed result
above used a fixture constructor to apply the policy.
[Runtime opt-in receipt](../proof/jobs-parity/qualification-v1.json)

### Run the compiled handler through the fork template

Five alternating pairs ran the 100 ms handler with 512 attempts and 64
slots per arm on macOS arm64 and PostgreSQL 17.11. Both arms used the
same compiled worker image, watchdogs, and native relay with 64 warmed
sessions over verified TLS and SCRAM. All 5,120 attempts matched the
native startup PID, exact reap, framed completion, effect key, and the
two committed writes. The two writes for each attempt used the same
backend. One-minute host load ranged from 14.25 to 17.24.

| CPU/job | Fresh exec | Forked child |
| --- | ---: | ---: |
| Children | 3.37 ms | 1.60 ms |
| Launcher and reap | 0.49 ms | 0.76 ms |
| Warm effect relay, including prewarm | 0.26 ms | 0.26 ms |
| Total | 4.12 ms | 2.63 ms |

The median paired saving was 1.47 ms/job, with a range of 1.33 to 1.88.
Fork used less CPU in all five pairs. Its median stayed at 2.52 ms/job
after excluding relay prewarming. The run already exceeds the 2.06 ms/job
budget from the earlier 64-slot Jobs runner comparison. It uses a
standalone launcher and relay, with no Jobs dispatcher or durable
settlement. The paired results use one pinned harness revision.
[Real handler receipt](../proof/jobs-parity/qualification-v1.json)

### Defer scheduler threads in the forked handler

The experimental `cfd85706` compiler passes the worker entry's C
arguments to the runtime. The compiled thread witness observed four
threads with eager startup and three with deferral, at the first handler
effect. The witness reconciled 32 attempts across fresh exec and fork.
The compiled Jobs proof on PostgreSQL 17.11 also awaited a captured
string after the first actor spawn under GC stress. The deferred attempt
went from three to four threads in the original PID. The exact-argument
and concurrent-first-spawn unit tests passed.

Five alternating pairs held the fork launcher, compiled image,
100 ms workload, and warmed effect relay fixed. All 5,120 attempts
reconciled with normal watchdogs and both durable writes. The thread
witness was disabled during timing. One-minute host load ranged from
10.49 to 11.61.

| CPU/job | Eager | Deferred |
| --- | ---: | ---: |
| Children | 1.26 ms | 0.82 ms |
| Launcher and reap | 0.54 ms | 0.56 ms |
| Warm relay including prewarm | 0.21 ms | 0.21 ms |
| Total | 2.02 ms | 1.60 ms |

Deferral reduced CPU in all five pairs. The median paired saving was
0.45 ms/job, with a range of 0.12 to 0.61. The deferred total leaves
0.46 ms/job against the earlier full-runner budget for the dispatcher
and settlement omitted by this fixture. The experimental compiler
also needs compatibility fixes for runtime-only reference builds and
the REPL's symbol table. [Scheduler diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

The follow-up at `52406a01` keeps the original startup ABI in normal
builds and selects the new entry only at fixture build time. Its release
compiler linked the original `7c162056` runtime-only fixture. The earlier
compiler failed that link on the missing experimental symbol. The
PostgreSQL 17.11 verifier passed with the old archive pinned, with 54
Jobs runtime cases and three compiled suites. The reference directly
exercised sparse polling, persisted pause, and manual enqueue.

The opt-in REPL test evaluated an expression with a stored Jobs main.
The codegen test exercised both startup entries and checked that the
handler fingerprint stays stable. The corrected compiler's fork and
exec witnesses again observed four eager threads and three deferred
threads across 32 reconciled attempts.
The compiled first-actor test passed under GC stress on the corrected
compiler as well. It built the fixture with the experimental entry and
checked the resulting thread count and `Job.await` in the original PID.
[Compatibility receipt](../proof/jobs-parity/qualification-v1.json)

### Measure scheduler deferral through the Jobs dispatcher

The `52406a01` experiment ran the frozen `slow-1r-64s` profile through
real enqueue, batch claims, fresh worker exec, exact reaps, and durable
settlement. Both Mesh arms used the same compiled image and warmed effect
relay. Five counterbalanced groups included Oban Basic 2.24.2. All
7,680 measured jobs reconciled on macOS arm64 and PostgreSQL 17.11.
Sampled one-minute host load reached 9.73.

| Arm | Jobs/s | CPU/job | Pickup p99 | Sampled peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Mesh eager | 472.0 | 3.53 ms | 975 ms | 236.2 MiB |
| Mesh deferred | 408.6 | 3.43 ms | 1,135 ms | 161.8 MiB |
| Oban | 494.5 | 1.45 ms | 931 ms | 118.5 MiB |

Deferral reduced CPU in four of five pairs, with a median paired saving
of 0.10 ms/job. Its throughput was lower in every pair. Both Mesh arms
missed the throughput and CPU limits. Deferral met the sampled RSS and
pickup limits. The median RSS peak contained 47 processes with deferral
and 66 with eager startup, leaving memory at equal live counts
unmeasured. The forked-handler CPU result has a separate receipt and
leaves dispatch and settlement costs open.
[Stock-exec comparison receipt](../proof/jobs-parity/qualification-v1.json)

### Reuse effect sessions inside the runner

The local `02afc9e2` diagnostic moved the helper into a thread in the Jobs
runner. It used the `da0a51bc` compiler and a private PostgreSQL overlay
in a disposable runtime build. The same fixture binary ran direct,
fresh-session, and reused-session Mesh arms with the reply-pipe wake
enabled. Five counterbalanced one-slot groups over verified TLS on macOS
arm64 and PostgreSQL 17.11 reconciled all 2,560 jobs.

| Arm | Jobs/s | Entry to first write | Pickup p99 | CPU/job |
| --- | ---: | ---: | ---: | ---: |
| Mesh direct | 53.0 | 5.64 ms | 2,390 ms | 7.11 ms |
| Mesh in-runner, fresh sessions | 47.2 | 5.95 ms | 2,671 ms | 7.57 ms |
| Mesh in-runner, reused session | 90.1 | 0.31 ms | 1,410 ms | 5.84 ms |
| Oban Basic 2.24.2 | 81.3 | 0.40 ms | 1,559 ms | 6.84 ms |

The reused arm beat Oban in three of five pairs and its median met every
measured limit for this profile. The fixture independently recorded one
distinct attempt child per job and one verified effect backend per reused
trial. The runner and attempt fit the two-process budget. The CPU includes
the helper thread and prewarming. The diagnostic covers steady-state
writes through the private one-slot route. A general application path
still needs proof for interruption, lease loss, and session reuse across
attempts. [Diagnostic receipt](../proof/jobs-parity/qualification-v1.json)

The `4818f8a0` transport experiment passed four focused tests on macOS
arm64 using owned TCP peers with plain and rustls streams. Revocation
before the first write produced zero observed bytes, including when
rustls already held the request. Stop after partial or full submission
yielded an unknown result and no fallback write. The tests also rejected
stale and expired permits. The experiment starts on an established
connection. Runner wakeup, PostgreSQL result reconciliation, and child
reply handling remain integration work.
[Transport test receipt](../proof/jobs-parity/qualification-v1.json)

The disposable effect transaction fixture passed over verified TLS on
PostgreSQL 17.11, 18.7, and 19.0. It locks the job row and validates
the attempt identity, expiry, and cancellation before a fixed application
write. An independent session verified one committed effect and its
receipt across rollback, replay, replacement, expiry, cancellation, and
lock contention cases. Removing `NOWAIT` caused the contention case to
reach its statement timeout on PostgreSQL 17.11. The proof covers
admission under a real job-row lock. Integrating the transport and
transaction checks into a reusable runner session remains open.

A follow-up on the same three PostgreSQL versions expired the writer's
lease after its effect committed and recovered the original job. The
stale token could no longer resolve the operation. The replacement lease
retrieved the original receipt with the same operation ID. An independent
session verified that the effect, receipt, and insertion counter stayed
unchanged.
[Transaction proof receipt](../proof/jobs-parity/qualification-v1.json)

The isolated `463b314e` proof ran both boundaries together. The
disposable PostgreSQL 17.11 cluster used verified TLS and SCRAM on macOS
arm64. All three native tests passed. The real-server test checks the
first-write fence, interrupts a submitted operation, holds a committed
receipt across revocation, and recovers it under a replacement claim.
It also verifies session reset before reuse.
[Integrated effect proof receipt](../proof/jobs-parity/qualification-v1.json)

The isolated exact-child test passed at `254df3ce` with PostgreSQL
17.11 over verified TLS and SCRAM. After running the normal Jobs migration,
the fixture took a confirmed claim and registered a real child through
the runner's launch path. The child sent a fixed operation ID over a
private channel. The parent retained the lease credentials and used a
login with permission to execute the fixture function. The test checked
revocation before send, a stale generation, a grant presented by a
different child, confirmed renewal, EOF, and a committed result held
across revocation. It verified receipt recovery, exact reap, and durable
settlement by the successor. The test owned the lifecycle calls and used
a native fixture child. Compiled handler and dispatcher integration
remain open.
[Exact-child effect receipt](../proof/jobs-parity/qualification-v1.json)

### Follow-up correctness proofs

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

After integrating compiler proof support from `27cc2c7b`, the
`b7550dce` checkout passed four proof-value tests and the callable-theorem
rejection cases. The macOS arm64 runs on PostgreSQL 17.11, 18.7, and 19.0
over verified TLS each passed 53 Jobs runtime tests and all three compiled
Jobs suites, including fresh and upgraded scoped worker roles. An
independent source review checked the resolved Jobs direct-call and
theorem-application guards.
[Integration receipt](../proof/jobs-parity/qualification-v1.json)

The Jobs compatibility script exercises the compiled producer and worker,
the PostgreSQL driver, leases, cancellation, timeout, recovery, and
shutdown. The full-role variant adds scoped migrator, producer, worker,
operator, and inspector logins, plus cron takeover and reindex.

The paired recovery baseline passed on hosted Linux x86_64 and local
macOS arm64 with release Mesh `7c162056` and Oban Basic 2.24.2.
PostgreSQL 17.11 used verified TLS and SCRAM. Both engines preserved the
committed enqueue, rolled back the other enqueue, and recovered the
original job after runner death. The replacement settled on attempt
two with one durable effect and shut down cleanly. Mesh also rejected
the stale lease token. The Linux run used the pinned pre-optimization
runtime after optimization work had begun.
[Paired recovery receipts](../proof/jobs-parity/j1-paired-linux-recovery-v1.json)

The hosted job also passed 45 Jobs runtime tests and two compiled Jobs
tests, including native timeout and parent-death recovery. Both
downloaded artifacts matched their published SHA-256 digests. The
tested merge checkout `1c5269a5` and PR head `e08d3f34` had identical
harness and runtime sources.
[Hosted PostgreSQL 17.11 job](https://github.com/snowdamiz/mesh-lang/actions/runs/44791300418/job/136501104028)

### PostgreSQL and platform matrix

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
| macOS arm64, integrated `bdb1bd42` | 17.11, 18.7, 19.0 | 51 Jobs runtime cases and all three compiled suites on each version, including scoped roles and contention retry over verified TLS. [Receipt](../proof/jobs-parity/qualification-v1.json) |

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

The hosted workflow passed PostgreSQL 17.11, 18.7, and 19.0 on Linux x64
and macOS arm64 for the `5868b649` tree. GitHub tested merge checkout
`23a49319`, whose tree matched the PR head. Each of the six matrix jobs
passed 36 PostgreSQL driver tests, 45 Jobs runtime tests, and both compiled
Jobs proofs, including scoped logins and the combined outage.
[Hosted PostgreSQL jobs and logs](https://github.com/snowdamiz/mesh-lang/actions/runs/44771848216)

The scoped-drain follow-up at `308bb6cf` passed the same three PostgreSQL
versions on macOS arm64. On both fresh and upgraded databases, the worker
login reported idle with work remaining while the row was locked and
returned an error when claim permission was withdrawn. With the grant
restored, it completed the original job on attempt one with one effect.
Bulk deletion and pruning each skipped a locked terminal row, honored the
page limit, and preserved its receipt after a later pass removed it.
[Scoped drain and cleanup proof](../proof/jobs-parity/qualification-v1.json)

The matched comparison recorded workload-specific performance and resource
misses. Broad Core parity remains open.
[Qualification receipt](../proof/jobs-parity/qualification-v1.json)

### Producer commit and final claim faults

The producer case cuts a connection after the fixture writes one business row
and enqueues two jobs in the same transaction. When PostgreSQL never receives
`COMMIT`, a separate connection sees zero rows and replay creates the batch.
When PostgreSQL commits and the reply is lost, replay returns both original
receipts with their job IDs, effect keys, states, and insertion fields intact.
The observer verifies the business row and stored job arguments against the
input. Each recovered job completes on attempt one with one effect.

The final batch claim case gives one job a one-attempt budget. The proxy
verifies PostgreSQL's `COMMIT` tag and withholds the response. A separate
connection observes the original job executing on attempt one with a live
fence. Recovery discards it after the 10-second lease expires. The runner
records zero native worker starts, handler entries, and effects. Both cases
passed on macOS arm64 with PostgreSQL 17.11, 18.7, and 19.0, using release
runtime `1b12243b` and harness `bc76c175`. The driver owns the disposable
PostgreSQL instance and uses SCRAM on loopback.
[Commit-loss receipt](../proof/jobs-parity/commit-loss-v1.json)

Run the paired fault suite with already built toolchains. Set `PG_BIN_DIR` to the
desired PostgreSQL installation.

```bash
python3 scripts/jobs-parity.py --faults --release --mesh-batch --poll-ms 100 \
  --mesh-ref 1b12243b --against-ref 7c162056 --toolchains target/jobs-parity/build
```

### Reproduce producer NOTIFY failures

Set `PG_BIN_DIR` to the PostgreSQL 17.11 bin directory and run the proof from
the committed harness. The driver builds the compiled producer in release mode
and starts its own loopback PostgreSQL cluster with SCRAM and verified TLS.

```bash
python3 scripts/jobs-parity.py --notify-producer-failures --postgres-version 17.11 \
  --mesh-ref a431fbbf
```

To reuse the pinned release pair, add `--toolchains target/jobs-parity/build`.
The manifest records the runtime revision independently of the harness
revision and fixture hashes.

The producer writes a business row and calls `Jobs.enqueue_in` on the same
connection. With publishing on, PostgreSQL rejects `PREPARE TRANSACTION`.
With publishing off, a separate session sees the prepared transaction and
empty business, job, receipt, attempt, and effect tables. The business row,
job, and receipt become visible together after `COMMIT PREPARED`.

For the full-queue case, the driver verifies the subscribed session in
`pg_stat_activity` and holds its transaction through both producer calls.
It commits 60 distinct 7,900-byte notifications. It then attempts to commit
2,500 distinct 32-byte notifications in one transaction and checks for the
queue-full error. With the queue still full, publishing on fails at
`COMMIT` and leaves all five tables empty. Publishing off commits the
business row, job, and receipt under the same pressure.

The compiled cases passed on macOS arm64 with PostgreSQL 17.11, 18.7,
and 19.0. [Producer failure receipt](../proof/jobs-parity/notification-producer-failures-v1.json)

Read `manifest.json`, `mesh-toolchain.json`, and `result.json` in the printed
evidence directory. This producer proof runs without workers or performance
measurements.

### Compiled drain and queue-local failures

The bounded drain proof holds a claim in PostgreSQL while removing execute
permission from the fixture. The caller receives a launch error. The original
job returns to attempt zero without effects, then completes after execute
permission is restored. A separate case disconnects the settlement backend.
The caller reports an unconfirmed settlement error, and a later drain
recovers the same job on attempt two with the original effect key.

The queue-local proof runs with single and batch claims. The runner terminates
a held handler and discards its job after the explicit grace has elapsed. The
runner continues serving another queue. A claim that returns during local stop
is refunded without a handler start. Process replacement resets local settings
from `Jobs.start`, preserves the database pause and concurrency limit, and
observes the limit after resume.
Both proofs passed on macOS arm64 with PostgreSQL 17.11, 18.7, and 19.0
using verified TLS and SCRAM.
[Compiled failure receipts](../proof/jobs-parity/qualification-v1.json)

The PostgreSQL 17.11 follow-up commits a real single claim and injects an
error at the caller boundary during queue-local stop. The runner clears its
local reservation, the original lease and attempt remain durable, and a
sibling queue settles. After expiry, a replacement claims attempt two with
the original effect key, and settlement with the obsolete token returns
`stale`. The compiled drain suite also verifies that an occupied global
slot leaves a queued job untouched, and a one-job drain recovers one
eligible expired lease while leaving excess and wrong-version rows intact.
The compiled inspection checks cover the 20-row default and insertion order.
[Follow-up receipt](../proof/jobs-parity/qualification-v1.json)

The compiled waiting assertions each completed one read with no visible
match before a separate transaction committed. `Test.assert_enqueued`
then succeeded and `Test.refute_enqueued` reported the matching job on
PostgreSQL 17.11 with verified TLS. [Waiting assertion receipt](../proof/jobs-parity/qualification-v1.json)

The compiled public API proof accepts inspection limits of 1 and 100,
an assertion wait of 60,000 ms with a committed match, and a drain budget of
1,000. It rejects inspection limits of 0 and 101, waits of −1 and 60,001 ms
through both assertion helpers, and drain budgets of 0 and 1,001. Each
rejected drain leaves the original job and its effect ledger untouched.
The verifier passed 47 Jobs runtime tests and all three compiled suites on
macOS arm64 with PostgreSQL 17.11 over verified TLS.
[Input bounds receipt](../proof/jobs-parity/qualification-v1.json)

After integrating main, the exact `aa40fa5d` checkout passed 47 Jobs
runtime tests and all three compiled suites on each of PostgreSQL 17.11,
18.7, and 19.0 on macOS arm64. The runs use verified TLS and cover
scoped roles on fresh and upgraded databases.
[Integrated matrix receipt](../proof/jobs-parity/qualification-v1.json)

The maintenance-join test pauses local admission and waits for any pending
claim to clear before stopping with the cron lock held. This lets the test
check that a drained runner still waits for maintenance. The corrected
PostgreSQL 19.0 run passed all 45 Jobs runtime tests and both compiled
proofs on macOS arm64. [Maintenance-join receipt](../proof/jobs-parity/qualification-v1.json)
