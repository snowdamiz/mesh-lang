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
its `mesh-rt` static library and set `MESH_JOBS_REFERENCE_RT_LIB_PATH` to the
absolute path of that library before running the verifier. It compiles the
same fixture twice, with local queue control disabled for the reference, and
checks the same committed input with polling on the reference and notifications
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

Each run prints its evidence directory under `target/jobs-parity/<sha>/`.
`manifest.json` records the compiled SHA, fixture hashes, tool versions,
database parameters, and host. `settings.json` records the frozen workload
and budgets. Each profile stores its raw attempt and occupancy samples,
trial metrics, and median and range. Exit code 1 indicates a queue
reconciliation mismatch. Exit code 2 indicates a fixture or environment
failure. The run records budget breaches separately in `result.json`.

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

Use a local checkout with the pinned pre-J6 revision `74db988d` available.
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
