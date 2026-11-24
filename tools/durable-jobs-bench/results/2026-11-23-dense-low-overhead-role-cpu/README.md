# Dense PostgreSQL role CPU measurement

**Hold the controller SETUP.** A C sampler brought median clock frame width below 0.6 ms on the local Linux fixture. Both sets of duplicate observer-off arms failed the existing drain spread limit. The Mesh-shaped drain also failed CPU reconciliation, and the peer-shaped drain still had a frame over 25 ms. Qualified role CPU remains unmeasured. [Replay results](evidence/summary.json) · [Raw first run](evidence/local-01/serial.log)

## What the local run priced

The ARM64 Virtualization.framework guest ran PostgreSQL 17.11 with certificate-verified TLS, SCRAM, synchronous commit, fsync, and a 96-connection cap. CPUs 0 and 1 ran the workload and CPU 2 ran the observer. Both ran as UID 65534. Every arm committed 1,024 rows at each of four stages. The fixture uses bulk SQL transactions and persistent psql clients. Actual Mesh V1, stock peer executables, and canonical runner lifetime wait4 are outside this fixture. PostgreSQL settings and workload placement were identical within each triple. The Mac's one-minute load rose from 46 at boot to 81 by exit on 15 logical CPUs. [Guest inputs](evidence/local-01/inputs.sha256) · [Host receipt](evidence/local-01/host.json)

| Fixture | TLS backends | Frames | Median / max frame | Recorder CPU over sampled interval | Drain off / on / off | Off spread |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mesh-shaped | 65 | 71 | 0.191 / 0.526 ms | 17.95 ms | 2,356 / 1,113 / 685 ms | 109.9% |
| Peer-shaped | 81 | 191 | 0.573 / 26.353 ms | 296.41 ms | 849 / 4,529 / 1,008 ms | 17.1% |
| Mesh-shaped with forced churn | 65 at opening, 66 at closing | 70 | 0.217 / 0.636 ms | 19.00 ms | 1,121 ms on | — |

The earlier two-census Linux smoke reported medians of 4.1–5.3 ms for the Mesh shape and 5.8 ms for the Oban shape. The new fixture establishes a smaller hot loop. Workload and host differences make the timing comparison descriptive. Snapshot costs are separate. The nominal Mesh-shaped opening and closing scans took 17 and 14 ms, and the peer-shaped scans took 38 and 34 ms. They ran outside the durable windows. [Earlier Linux receipts](../2026-11-23-ordinary-native-cpu-recovery/evidence/linux/README.md)

The original gates keep the 25 ms frame limit, 100 ms cadence and edge limits, 1 ms wall-clock bracket tolerance, and paired drain spread limit of 10%. The probe shift must fit within 5% or half the control spread, whichever is larger. The clean Mesh-shaped probe passed the frame width check. Its 2,074.7 ms of captured process CPU wholly inside drain exceeded the 1,920.0 ms enclosing host busy upper bound. The peer-shaped probe hit the frame width and clock bracket gates. The local elapsed shifts therefore give no estimate of whole-workload observer overhead. [Original gates](../2026-11-23-ordinary-native-cpu/replay/reduce.py)

## Identity and coverage

The opening snapshot pins each known process with a Linux pidfd before its PostgreSQL login, backend start, and TLS evidence is read. It checks kernel start ticks, parent, and affinity during registration. The C loop uses an individual monotonic bracket around each process CPU clock, one batched pidfd poll, and a bracketed read of host CPU ticks. An exited pidfd rejects the associated clock value even if Linux later reuses the numeric PID. The loop performs zero SQL queries, zero process-tree walks, and zero per-frame pidfd opens. The sampler writes its buffered frames after the timed window. [Native collector](replay/clock.c) · [Registration](replay/observe.py)

The forced run moved from 23 Jobs backends at readiness to 31 at the opening capture. It then killed one, opened a replacement, opened another late connection, and opened and closed a short-lived connection between snapshots. The closing snapshot captured the replacement and late identities. The original pidfd reported 54 failed clock reads after exit. The short-lived connection escaped both snapshots and left an unexplained increment in PostgreSQL's session counter. The raw fixture records its PID for verification. The replay left its CPU null and classified an unrelated TLS login as unknown. Replay tests also exercised same-number PID reuse and denied clock reads using copies of the real trace. [Lifecycle results](evidence/summary.json) · [Checks](evidence/check.log)

The session delta is a coverage check. Pending statistics from an already-open connection can also cause a mismatch, and unusual exits can limit what the counter proves. Closing snapshots identify surviving late connections and can detect retirement or identity reuse. Full coverage of between-snapshot sessions would require a loss-checked lifetime record. The sampled totals cover stable captured PIDs. The producer database login shares producer and harness-observer work. PostgreSQL background CPU, unknown sessions, and absent application-role measurements remain null. Host residual includes any work outside the attributed PIDs. [PostgreSQL session statistics](https://www.postgresql.org/docs/17/monitoring-stats.html) · [PostgreSQL 17.11 connection accounting](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/utils/activity/pgstat_database.c)

## What the native archives establish

The original Native-02 replay reproduced all 18 durable arms and the published recovery JSON byte for byte. It qualifies the pinned host drain comparison and preserves the original role failures. There are no Mesh role rows. Oban's observer shifted drain by 19.31% and 24.04% in the two orders. The earlier PC sampler measured an older final-noop fixture and left profiler overhead inconclusive. Those role estimates cannot fill the ordinary V1 gaps. [Recovery report](../2026-11-23-ordinary-native-cpu-recovery/README.md) · [Reproduced native replay](evidence/original-native-replayed.json) · [Prior PC sampler report](../../../../target/dense-cpu-sampling/REPORT.md)

The five-block stock replay recovered 1,024 terminal jobs per measured arm for actual V1 8×8 and 2×32, Oban, River, and Apalis. The median V1 drain rates were 452.45 and 751.82 jobs/s. Runner lifetime CPU was 1.615 and 0.637 ms/job. The 2×32 topology uses 41 connections compared with 65 for 8×8. The stock measurements price the topology control and leave the PostgreSQL CPU split open. The available ordinary Linux V1 release pin is x86-64. The local ARM64 VZ route supports the staged PostgreSQL fixture. The archived Darwin V1 executable cannot run inside it. [Stock scorecard](../2026-11-23-dense-matched-topology/evidence/stock/scorecard.md) · [Reproduced stock replay](evidence/stock-replayed.json)

## Cost of the next check

One controlled Linux boot with a sustained TLS workload can distinguish the clock disagreement before another full series. Add independent per-process user and system tick reads to four known edge frames and compare their deltas with the process clocks and pinned host ticks. Pre-open the stat files for 75 or 91 verified processes. That costs 300 or 364 extra reads for the calibration, with no per-frame SQL. At 100 Hz, two CPU fields allow up to 20 ms of edge uncertainty per PID, or 1.50 and 1.82 CPU-s across the two rosters. A replay the length of the observed drain could hide its 155 CPU-ms disagreement inside that allowance. A calibration with at least 25 busy CPU-s per roster gives an 8% disagreement room to exceed the tick uncertainty. On two busy CPUs that takes at least 12.5 seconds of workload per roster. The discriminant is whether process ticks agree with host busy ticks while process clocks overshoot them. If both process measurements disagree with host accounting, the CPU split remains unqualified.

An actual controller diagnostic would require that accounting check, complete lifecycle coverage under the controller's privileges, and both control orders on the pinned V1 and stock peer workload. Existing boundaries make a first V1-only set six arms across two OFF/ON/OFF orders. Price the optional 2×32 pairing at six more arms and keep its 41-session roster. The historical 18-arm multi-lane allocation was 12 minutes. No reservation was requested for this experiment.

## Replay

Run the portable replay from the checkout root. The archived workload folders and VZ base image are existing checkout dependencies.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-dense-low-overhead-role-cpu
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
python3 -B "$packet/replay/check.py" "$packet/evidence/local-01/serial.log" -v
python3 -B "$packet/replay/replay.py" "$packet/evidence/local-01/serial.log" |
    cmp "$packet/evidence/summary.json" -
```

To repeat the Linux fixture, build the collector for ARM64 Linux with Zig 0.15.2 and run the local VZ overlay against a fresh output directory. Point the Zig caches and temporary directory inside your checkout.

```sh
zig cc -target aarch64-linux-musl -static -O2 -Wall -Wextra -Werror \
    "$packet/replay/clock.c" -o target/dense-low-overhead-role-cpu/clock
python3 -B "$packet/replay/local.py" target/dense-low-overhead-role-cpu/local-repeat \
    target/dense-low-overhead-role-cpu/clock
```
