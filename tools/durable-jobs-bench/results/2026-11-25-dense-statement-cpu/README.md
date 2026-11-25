# CPU attribution for the frozen individual-terminal dense comparison

This diagnostic uses the released logger-off hot Mesh ELF from `1e70f18d` and the stock River v0.51.2 2×32 ELF. It keeps the 1,024-job input, handler, isolated PostgreSQL 17.11 clusters, TLS, durability and CPUs 0–1 from the six released rotations. The existing process-roster diagnostic lacked stable conclusively reconciled role CPU. The earlier live-plan module captured plan shape and elapsed time including waits. The stock archives contain an elapsed CPU ceiling for three executors. They provide no direct per-statement CPU for either engine. The new hooks use PostgreSQL's existing query IDs and leave its stock statement collector in place. [Stock rotations](../2026-11-24-dense-individual-hot-pool/logger-off-retry/NATIVE-RESULTS.md) · [Role diagnostic](../2026-11-23-dense-low-overhead-role-cpu/README.md) · [Live plans](../2026-11-25-dense-live-grant-plans/README.md)

## What the archives already rule out

- Wait overlap alone saves zero CPU. Every rotation requires **337–512 CPU-ms** less busy work within the submit window to fit a 20% throughput lead at the same River elapsed and two-core budget.
- The two application INSERT executors total **70.4–78.9 ms** elapsed. Removing even that much CPU leaves **260–433 CPU-ms** elsewhere in every rotation. This ceiling covers the two executors only.
- Settlement executor work alone cannot close the gap in every rotation, even if all its elapsed time were CPU. In rotations 2 and 6 the ceiling for settlement plus both application INSERT executors is still too low.
- The same runner-only target would require **62–93%** of both Mesh runners' combined lifetime CPU. Product candidates that change protocol, commits, scheduling or contemporaneous work in other processes need broader accounting. [Historical source and replay](../2026-11-25-dense-host-cpu-budget/README.md)

## Measurement and decision

The extension brackets PostgreSQL executor start, run, finish and end and ProcessUtility with `CLOCK_PROCESS_CPUTIME_ID`. It charges the outer invocation for nested SPI, including caught errors, and restores nesting on error. The planner guard suppresses planner-triggered SPI charges. The counter key is effective role OID, database OID and signed PG query ID within an authenticated backend. Hook slice counts distinguish resumed executions and utility invocations. Transaction utilities can include transaction work that occurs inside the utility call. Protocol completion and any deferred commit or flush outside hooks go to the session residual along with planning, TLS, auth and backend cleanup. Concurrent parallel worker work stays in reaped-child residual and blocks statement-only decisions. [PostgreSQL statement keys](https://www.postgresql.org/docs/17/pgstatstatements.html)

Each authenticated backend gets a fixed-size portion of anonymous shared memory once. Statement updates touch only that portion and do no SQL, file or log output. Both modes record lifetime backend CPU to the exit callback. ON adds statement hook charges. A stopped postmaster publishes one atomic report with session slots, CPU totals and its reaped-child CPU. The reducer rejects an unexplained lost backend, overflow, failed clock, duplicate key, or impossible CPU sum. Fresh postmasters reset the extension. Stock `pg_stat_statements_reset()` can remove the text dictionary entry while retaining the module's CPU charge. Those keys, failures with no pgss row, zero IDs and unmatched keys retain explicit CPU totals. SQL text is used privately to classify the frozen statement family. The export retains the shape hash and pgss identity. Structurally equivalent or colliding query IDs inherit pgss's identity uncertainty.

The exit census covers short-lived and warmed sessions. Mesh readiness TLS backend PIDs must appear under the same login and database. Mesh application and participant role names separate those login authorities. Broker and harness work can share producer authority. River's login serves multiple purposes, so its role-level total stays shared and only recognized statement families distinguish work. The owner and unknown SQL remain visible. River completion keys use the pinned driver’s batch state-update shape. [River v0.51.2 driver SQL](https://github.com/riverqueue/river/blob/v0.51.2/riverdriver/riverpgxv5/internal/dbsqlc/river_job.sql) The cluster total samples postmaster and reaped-child CPU. Its residual includes background processes, parallel work, and CPU after backend exit callbacks. The host residual contains everything outside sampled PostgreSQL work plus tick uncertainty. Session sums cover the full arm, including database setup, resets, inspection and shutdown.

Both groups run a fresh warmup and measured arm for every case. Group 1 uses Mesh OFF A, River OFF A, Mesh ON, River ON, Mesh OFF B, River OFF B. Group 2 reverses the entire order. The observer module and workload ELF hashes are identical between modes. Each engine's two adjacent-by-mode controls must spread at most 10% about their midpoint. Absolute ON shift must be at most 5% of the OFF midpoint or half the duplicate spread, whichever is larger. Both groups must pass for submit, drain, total adapter duration and total pinned host busy CPU. The last two include process setup, off-clock dictionary collection, shutdown and report writing/export into the adapter directory. The host tick bracket is reported separately from measured clocks. Controller release timestamps and logs retain staging, transfer, exported raw archive and replay overhead. No confidence interval or production speed claim follows from two rotations.

Use surviving rows to compare Mesh and River charges per statement family and locate the largest repeatable CPU-consuming operation. The whole-arm scored hook sum is an optimistic ceiling on direct savings inside the submit interval. The reducer adds unresolved query CPU for the same login scope before ruling out a statement-only change. It shows the share of a measured hook bucket that would need to disappear to close the largest historical gap. Residual work needs separate attribution before selecting a change that targets it. A prospective change must preserve the lifecycle and application constraints and earn a separate uninstrumented matched comparison.

## Replay and sole-controller intake

Use the checkout root. The request runs at the tail of the existing sole-controller queue. Both grants and the receipt bind the sealed request. SETUP compiles the extension using the host's pinned PG17.11 headers and checks real backend and postmaster exits in both modes, nested SPI, caught and top-level errors, abort, reset and client idle. TIMING needs a successful Linux SETUP release.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-statement-cpu
python3 -B "$d/controller.py" check
python3 -B "$d/test_measure.py"

# Sole controller after earlier bookings. Register byte-exact request.json
# as target/linode-jobs-stepchange/requests/2026-11-25-dense-statement-cpu.json.
seal=$(shasum -a 256 "$d/request.json" | cut -d' ' -f1)
python3 -B "$d/controller.py" intake setup "$seal"
python3 -B "$d/controller.py" intake timing "$seal"

# After release
python3 -B "$d/controller.py" replay
```
