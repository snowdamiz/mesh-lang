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
