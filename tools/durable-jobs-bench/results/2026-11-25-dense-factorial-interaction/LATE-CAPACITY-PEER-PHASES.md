# Where River gained time in the five 8×8 rotations

Mesh delivered the first 64 durable effects sooner after enqueue commit in every rotation. River then delivered effects 65–768 sooner by 100–1,190 ms and the last 256 sooner by 159–209 ms. Mesh recovered 185–241 ms after the last effect, bringing its median paired drain deficit down to 56 ms. River also committed enqueue 99–161 ms sooner and won all five submit comparisons. The original peer gates returned inconclusive for River, Oban and Apalis. [Peer verdicts and controls](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md)

Each number below is Mesh elapsed minus River elapsed in milliseconds for the same block. The five phase differences sum to the submit difference before rounding. Each workload committed two application writes per job. Effect refers to the second write.

| Rotation | Submit to enqueue commit | Commit to effect 64 | Effects 64–768 | Effects 768–1,024 | Last effect to settlement | Full submit |
|---|---:|---:|---:|---:|---:|---:|
| 1 | +99 | −45 | +100 | +175 | −235 | +94 |
| 2 | +101 | −62 | +1,190 | +209 | −241 | +1,197 |
| 3 | +134 | −122 | +203 | +159 | −221 | +153 |
| 4 | +161 | −92 | +212 | +177 | −241 | +217 |
| 5 | +132 | −130 | +253 | +188 | −185 | +256 |

Matching River’s enqueue duration alone, with each observed drain held fixed, would put Mesh ahead on submit only in rotation 1. Even an instantaneous Mesh enqueue would leave deficits of about 1,017 ms and 51 ms in rotations 2 and 5. These counterfactuals hold the downstream schedule fixed. The second rotation had 22 Mesh contention hints, compared with zero in each other rotation. Its excess elapsed time was concentrated between effects 64 and 768. The cause remains unresolved.

## The slow second rotation

The PostgreSQL server log places all 22 queue-lock timeouts inside the drain, 20 in the preliminary queue-row lock and two in the bounded claim. The pinned collector reads CPU counters before the frame timestamp and device counters after it. The original nine-interval calculation treated both reads as simultaneous. Requiring the neighboring frame timestamps to fall inside the drain leaves seven intervals for B. Their frame timestamps span 1.667 s and contain 713 durable effects and 21 of the timeouts. The two pinned CPUs recorded 63 of 322 ticks as I/O wait. [Original frame-based receipt](late-capacity-io.json) · [Corrected read brackets](late-capacity-io-bounded.json)

Device 8:0 accumulated 1,234 busy ms across those seven adjacent counter deltas. The disk reads span 1.428–1.909 s, giving a busy fraction of about 65–86%. The corresponding conservative ranges are 13–40% for A1 and 14–29% for A2. Device activity was higher for B even with uncertainty about the read times. The counters include every process on that device. The captures leave the cause of the stalls unresolved.

## Work waiting before its first write

During effects 256–768, Mesh had a time-average of at least 7.47 jobs that had entered a handler and had yet to receive their first PostgreSQL transaction ID in every block. River’s upper bound stayed at or below 2.20. The observed Mesh interval was 507–1,238 ms, against River’s 280–316 ms. XID assignment follows a globally ordered sequence within each fresh cluster, which lets the ledger bracket it between observed events. Overlap measures jobs waiting concurrently. It does not identify the share spent in pool lookup, runnable queues, SQL processing, or lock waits. The earlier two-runner direct-pool experiment also showed that removing a wait after entry can move it before entry. [Method and prior controls](BEFORE-FIRST-XID.md)

The replay verifies 70 unique archive hashes and the five reconciled score files. For each of the 35 measured arms it checks 1,024 completed jobs, 2,048 distinct application XIDs, and that the intervals sum to both original durable clocks. The five-block comparison and both outliers are also available in the [viewer](../2026-11-23-iteration-comparison/chart.html#public-dense-8x8-late-capacity).

From this directory, run the offline checks against the Git-tracked evidence.

```sh
python3 -B test_xid_bounds.py
python3 -B late_capacity_peer_phases.py ../2026-11-25-public-dense-late-capacity/peer-comparison/evidence | cmp - late-capacity-peer-phases.json
python3 -B late_capacity_io.py ../2026-11-25-public-dense-late-capacity/peer-comparison/evidence | cmp - late-capacity-io-bounded.json
```

## What the first handler entry proves about the rest of its grant

During the middle 512 effects, 90.4–97.7% of the **provable waiting job-time after grant confirmation** occurred on a runner where at least two other jobs had already entered their handlers and had yet to commit their second writes. A longer scheduler park targets workers that have become empty. These counts cannot measure their exposure. They show why an entered-handler count or a queue-row timestamp would make a poor substitute for the native scheduler observation.

| Rotation | Mean jobs provably waiting to enter | Share of that wait with ≥2 entered on same runner | Mean runners with entered work, out of 8 | Wall ms with possible backlog on a runner with zero entered |
|---|---:|---:|---:|---:|
| 1 | 1.64 | 97.7% | 3.23 | 434.4 |
| 2 | 1.77 | 90.4% | 5.27 | 733.3 |
| 3 | 1.69 | 94.6% | 3.65 | 409.3 |
| 4 | 1.40 | 93.3% | 3.39 | 441.5 |
| 5 | 1.42 | 93.7% | 3.48 | 423.3 |

Each checked COMMIT precedes the first recorded handler entry from that grant. Every later member still awaiting entry at that point is known to have a confirmed grant. On the other boundary, the millisecond on the database claim is the earliest possible confirmation. Between those edges the status is unknown. In all five B arms the first-entry lower bound gives zero time with a confirmed, unentered member and zero entered handlers on its owner. The generous claim-stamp upper bounds in the last column allow hundreds of milliseconds. The gap concerns the time *before* the first observed entry and cannot distinguish slow COMMIT, slow adoption, or slow initial scheduling. The lower bound can only witness later members, and a first member helps keep its owner's entered count positive until its own second write commits.

An entered handler can be sleeping or awaiting a connection or database result. A confirmed row can still be awaiting actor submission. The table therefore refers to jobs and durable writes, and supports no inference of scheduler thread utilization or runnable queue depth. The two same-ELF OFF controls had 0.31–0.57 provably waiting jobs on average compared with B's 1.40–1.77. This comparison also changes the widths of the actual grants. The slow second rotation had the most runners with entered work even as its durable writes advanced the slowest.

The [15-arm replay](confirmed_entry_supply.py) binds each raw to the controller and reconciled scores. For each PID, the grant buckets reconcile to the number of positive calls, and the ledger gives one entry and two independent application transactions per job. The [interval counts](confirmed-entry-supply.json) report both runner-time and wall-time unions, including the case with at most one entered handler.

The queue lock also supplies an independent witness before a batch's first handler entry. In the bounded SQL, the claim stamp follows acquisition of the queue row lock and the transaction holds that lock through COMMIT. A different productive grant with a strictly later stamp on that queue proves the earlier rows became visible in PostgreSQL. A later grant by the same runner additionally proves the earlier claim lane received its COMMIT. The replay permits the full 999 µs of stamp rounding and makes no ordering inference from equal stamps. This comparison assumes the shared realtime clock advanced normally. [Bounded claim SQL](../../../../compiler/mesh-rt/src/jobs/grouped_schema.sql)

Using the database visibility witness, the middle windows contained at least **9.398, 16.392, 3.429, 8.723 and 6.538 ms** respectively where a runner had a granted job still awaiting entry and zero entered handlers awaiting effects. The same-runner client witness adds zero such time in these B windows. The confirmed-claim and database-visible bounds answer different questions. These milliseconds establish a post-commit wait at the server, while its division between client receipt, dispatch and scheduling remains open.
