# Dense hot checkout with two or four scheduler workers

The original same-ELF checkout comparison advanced in four of six completed Linode groups. Independent raw replay reproduced wins in groups 1, 2, 4 and 6. The controller's final release and the 120 qualified arm digests are bound by the prerequisite receipt. [Checkout gate](evidence/checkout-gate.json)

Earlier fixed-four and fixed-eight experiments used V1 with two synchronous checkout probes per job. The Apple DEV scheduler sweep reported lower median durable throughput and higher runner CPU for both settings. The concentrated two-runner fixed-eight experiment improved durable throughput in one of five groups and used more CPU in all five. The earlier native concentrated series used two fixed workers per runner. No archived result located exercised fixed four on the sealed hot R2 D2 Linux executable. [Scheduler sweep](../2026-11-23-ordinary-scheduler-supply/README.md) · [Concentrated local experiment](../2026-11-23-ordinary-topology-scheduler-supply/README.md) · [Earlier native topology](../2026-11-23-ordinary-runner-topology/README.md)

## Frozen comparison

The pilot has two reverse-order blocks across six lanes, each with its original warmup and measured arm. The Mesh lanes are two fixed-two duplicates and one fixed-four arm. The comparison also includes stock equal-2×32 River, Oban and Apalis. The original source, 1,024-job input, deduplicated durable enqueue, handler, and settlement behavior come from the released dense matrix. Each job must commit each application write independently before durable settlement.

Both D2 variants use runner c7c50e726ca0 with ELF SHA-256 beginning 972514ef and the official bulk broker with manifest 7f228c19. The runner children alone vary the two scheduler settings. Their broker stays fixed at two. The common envelope is synchronous TLS and durability, hot checkout enabled, completion writer unset, application pool min/max 16 per runner, two 32-slot runners, 41 warmed Mesh client sessions, PostgreSQL 17.11 physical cap 96, default ext4 storage, and CPU affinity 0–1. An offline stage check compares the sealed paths, storage and digests with group 6's raw arms.

The original timed SQL and sampling cadence stay in place. Live process identity, thread counts and TLS ownership are checked at readiness. PostgreSQL CPU frames use procfs immediately before submit and after settlement is observed. A separate SQL readback after the last recorded settlement and original SQL counters identifies each warmed app session's last write. Those distinct backend PIDs are a lower bound on write participation. The retained arm samples provide sampled peak threads and RSS. Host CPU includes PostgreSQL, other work on the pinned CPUs and recorder-window overhang. The per-process PostgreSQL CPU sample covers matching Postgres identities in the two Mesh frames, reports tick resolution and boundary slack, and identifies processes that could not be paired. Peer PG CPU remains part of the host window. Runner CPU includes startup, idle, and shutdown from the existing adapters. SQL counters retain each adapter's original attribution.

## Decision

In **both** pilot groups, fixed four must beat the faster fixed-two duplicate on submit-to-settlement, commit-to-settlement and the interval from the 103rd to the 922nd committed second write. For each clock, the gain must strictly exceed the larger of the same-clock duplicate spread and 5% of the faster control. The last second-write commit relative to enqueue commit must also precede both controls. Every arm must reconcile. The replay reports these clocks, last-write clocks, complete peer orderings, counters, and paired CPU and RSS differences. Only a passing pilot unlocks the four further balanced rotations under the sole controller. The four extension rotations are frozen in the packet.

Run the offline checks from the repository root.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-dense-hot-worker-supply
python3 -B "$p/test_offline.py" -v
python3 -B "$p/controller.py" check
(cd "$p" && shasum -a 256 -c SHA256SUMS)
```

The existing ARM64 guest exercised the exact staged Python entry at UID/GID 65534 in root-owned 0711 directories. Both explicit file imports reached the deliberately failing one-CPU guard before loading any benchmark binary. The regular sibling import reproduced the old failure. The same guest checked the procfs read in an unprivileged process. [Stage receipt](evidence/linux-stage.json) · [First guest attempt](evidence/first-linux-guest-failure.txt)

The [controller packet](controller-packet.json) is for the sole Linode controller after the booked series. Native raw attempts and the replay decision go under `evidence/native/`. The packet requires fresh controller timing grants. No benchmark binary was built and no workstation workload was run.
