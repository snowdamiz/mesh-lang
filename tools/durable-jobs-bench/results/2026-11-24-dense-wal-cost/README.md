# Dense PostgreSQL WAL cost

The isolated four-writer replay on the same Linode and PostgreSQL 17.11 confirmed `fdatasync`. The two stock controls each issued 2,048 distinct durable application commits across four TLS backends. Their median PostgreSQL WAL sync counts were 1,256 and 1,275.5 across six rotations. Median cumulative sync times were 343.390 and 336.508 ms, with ranges of 332.934–393.194 and 326.335–1,602.335 ms. Median WAL write times were 13.214 and 13.596 ms. The per-arm median ratios were 0.272 and 0.264 ms per reported sync. The replay reconciled all 36,864 application rows across the 18 component arms. [Exact recovered replay](evidence/direct-writer-replay.json)

As a sensitivity estimate, a 25% or 50% reduction in this component's median sync charge would remove about 84–86 or 168–172 cumulative milliseconds, respectively, if the cost per sync held. Overlap and other work determine how much of that could reach wall time. The isolated fixture cannot transfer that charge to the full queue. Its recovered replay uses the controller's sealed retry input and exactly matches the archived summary. The first direct replay against the current source correctly rejected the sampler hash.

The archived hot R2 drain took 498–576 ms longer than equal River between the 103rd and 922nd committed second writes. Six paired whole-device flush count intervals are disjoint. Whole-device flushes include other PostgreSQL writes and filesystem activity, and several commits can share one flush. The device readings also omit backend time around the sync, including any wait to take PostgreSQL’s WAL write lock. [Six paired dense blocks](../2026-11-23-iteration-comparison/application-intervals.md#dense-jobs-after-hot-checkout)

## Native diagnostic

Two opposite-order blocks each run the canonical warmup and measured phases of the 1,024-job dense input. The four lanes are Mesh hot R2 D2 at fixed two workers, equal River, Oban as a synchronous durable control, and equal Apalis. The diagnostic reuses the installed Linux executables and stock reconciliation. The native scorer preserves the same job identities, two independent application writes per job, settlement and enqueue fences, 32 warmed Mesh application TLS sockets, 64 total runner slots, PostgreSQL's 96 connection cap, and pinned CPUs 0 and 1. The runner and peer binaries are unchanged. Each diagnostic inventory is created under a new stage. Official peer inventory files are read-only inputs.

The one server setting change is `track_wal_io_timing=on` for every arm. The verifier requires PostgreSQL 17.11, TLS, `synchronous_commit=on`, `fsync=on`, `full_page_writes=on`, stock `fdatasync`, and zero commit delay. Enabling the timing option reads clock values on PostgreSQL's WAL I/O path. The diagnostic is for attribution. The archived stock comparison remains the performance reference.

- The baseline WAL query uses the same TLS session as the original statement-statistics reset, before the producer and workers launch. This includes River's binary-owned setup in the later measurement.
- The live WAL and statement queries use one TLS session after the frozen adapter has finished and its clients have disconnected. No additional SQL runs during the benchmark or while worker and producer sessions are active. The original harness keeps its observation cadence. Live snapshots include PostgreSQL's WAL record and byte totals, write and sync counts and times, relation I/O, and checkpointer and background writer statistics. Statement WAL generation by role and operation is available from the same post-run session.
- PostgreSQL 17 can defer publishing pending WAL counters, and connection exit flushes pending client counters. The diagnostic performs the original fast shutdown, verifies the persisted statistics file, and reads the counters after a clean restart on the same cluster and CPU mask. If that fails, the original cleanup still runs. The replay checks cluster identity, reset epoch, LSN and counter monotonicity, client closure, and wall and monotonic timestamps. [PostgreSQL 17.11 statistics reporting](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/utils/activity/pgstat.c) · [PostgreSQL monitoring statistics](https://www.postgresql.org/docs/17/monitoring-stats.html)

The final minus baseline counters are labeled **whole workload**. They give a conservative upper bound on reported PostgreSQL WAL work charged during the drain. Their scope includes worker startup, unpublished pre-submit activity, queue and application WAL, admin or maintenance work, and shutdown/readback. The live-to-restored difference mixes delayed publication with work after the live sample. LSN movement, checkpointer counters and capture timestamps expose that overhang. The replay keeps the in-drain device brackets separate and flags outer brackets that could include the shutdown checkpoint. It reports WAL generation by statement where available. A shared WAL sync cannot be assigned to a particular role or commit using these views.

If cumulative Mesh WAL write plus sync time for the whole workload falls below the paired middle-write deficit, even crediting all of that time to the critical path cannot fill the observed gap through direct I/O-time savings alone. Otherwise the attribution remains open. Wall-clock effects from changing lock waits, scheduling or concurrency can differ from the direct I/O charge.

## Queue overlap and decision

The booked fixed-four pilot tests actual hot R2 worker supply against duplicate fixed-two controls. Cooperative permits test cold checkout admission and different application concurrency. The role CPU runs price process work, and the producer-boundary run uses nested statistics for short R2 enqueue. The controller confirmed none collects complete PostgreSQL 17 WAL counters on the dense queue. Those bookings retain priority. [Fixed workers](../2026-11-24-dense-hot-worker-supply/README.md) · [Role CPU](../2026-11-23-dense-low-overhead-role-cpu/README.md)

No causal treatment is included. The isolated replay supports collecting data. Any group-commit experiment needs credible timed-drain headroom and a concurrency comparison that leaves the question open.

## Reproduce

Run from the checkout root.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-dense-wal-cost
python3 -B "$p/controller.py" check
python3 -B "$p/prove.py" /path/to/pgsql-17.11/bin
(cd "$p" && shasum -a 256 -c SHA256SUMS)
```

The local proof uses real PostgreSQL and two fixture writes. It exercises delayed publication and clean restart plus rejection of overlap, a changed reset epoch, a live competing client, and a decreasing counter. It produces no workload timing. The sole controller owns the separate native grant and dispatch. Replay and archive every attempt under `evidence/native/`.
