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
on the candidate.
The smoke test runs one small matched pair. The fault test observes a claim
and a committed effect before terminating the runner, then checks recovery
from a separate database connection. Mesh recovers by lease expiry. Oban
uses its Core Lifeline recovery policy.

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

For a Mesh revision that includes batch claims, pass `--mesh-batch` to measure
the opt-in runner. Omit the flag for the single-claim profile. Both modes
record claim, runner, and producer SQL calls per completed job. Use
`--against-ref` to run five pairs against another Mesh commit with the same
inputs and connection budget. To measure batch dispatch on the same Mesh
commit, pass that commit to both `--mesh-ref` and `--against-ref`, then add
`--mesh-batch`. To exercise batch commit-response loss, use
`--faults --mesh-batch`.

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

For telemetry comparisons, pair the candidate with pinned main and measure
the same profile with `--mesh-events disabled`, `--mesh-events logged`,
and `--mesh-events backpressured`. The logged variant writes JSON events
to the runner's log. The backpressured variant leaves the 128-event reader
buffer unread and records its drop count in the trial receipt.
Use `--faults --mesh-events logged` on a current runtime to verify that
an acknowledgement lost after commit produces uncertainty without a
commit confirmation from the runner.

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

Reconciliation runs after runner shutdown. It checks the final job,
business-write, attempt, and effect identities across the disposable
database. A direct occupancy observation above the configured slots
fails the trial. The timestamp reconstruction stays available separately.

Pickup uses the interval between the enqueue transaction's commit timestamp
and the first handler entry. Claim and settlement are tracked separately.
The connection and operating system resource samples describe observed
peaks at the configured sampling interval. Runner CPU includes the process
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

## Exercise primary promotion

Build the compiler and runtime from the revision under test. Set `PG_BIN_DIR`
to a PostgreSQL installation with `pg_basebackup` and `pg_stat_statements`.
Choose a new evidence directory under the checkout for each run.

```sh
cargo build --locked -p meshc -p mesh-rt
python3 scripts/jobs-promotion.py --postgres-version 17.11 \
  --evidence-dir target/jobs-parity/promotion-17.11
```

The proof starts a primary and a physical streaming standby. It loses a
settlement reply after commit, waits for the standby to replay through the
primary's flush LSN, stops and fences the old primary, then promotes the
standby. It checks receipts, job IDs, effect keys, and stale-token rejection
while the replacement attempt holds its lease. Replication and direct
database connections use verified TLS and SCRAM. The one-shot fault proxy
uses plaintext loopback with SCRAM so it can identify the settlement reply.

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
This run records the post-reliability baseline for two profiles under load.
Release qualification still requires the matched candidate matrix on a
host with capacity for the workload and its observers.

## Compare bounded drain

`scripts/jobs-drain-perf.py` accepts the pinned main and candidate compiler
and runtime artifacts. It interleaves legacy due-only drain and the new
bounded drain with one slot. Each trial checks the same completed jobs,
their effects, and the untouched future rows. Pass `--direct` to time the
compiled inline helper in a separate test.

The local PostgreSQL 17.11 debug run with main `7c162056` and candidate
`03990801` reconciled 24 completions and four future rows in each of seven
pairs. The median paired duration was 1.294 times main, above the 1.10
budget. Host load was about 30–34 on 15 logical CPUs. The inline helper took
69.1 µs per call in that run. Release-build timing and the pinned main
direct-handler baseline remain open.
