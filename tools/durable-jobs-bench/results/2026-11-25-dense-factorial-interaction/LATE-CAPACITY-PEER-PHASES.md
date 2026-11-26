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

## Concurrent jobs between their two durable writes

River led Mesh B in **78 of 80** consecutive 32-effect intervals between effects 256 and 768. Even giving Mesh River's time in its three worst intervals per rotation leaves a middle-write deficit in every rotation. The last column gives the sum of the signed differences in the other thirteen intervals. The intervals are subdivisions of five paired runs and share the scheduling and host conditions of their run.

| Rotation | Intervals where River was faster, of 16 | Mean runners with a job between commits, Mesh / River | Residual Mesh deficit after replacing its three worst intervals |
|---|---:|---:|---:|
| 1 | 15 | 2.13 / 3.29 | 118.9 ms |
| 2 | 16 | 3.40 / 3.32 | 490.8 ms |
| 3 | 15 | 2.24 / 3.15 | 145.3 ms |
| 4 | 16 | 2.19 / 3.45 | 179.5 ms |
| 5 | 16 | 2.08 / 3.17 | 171.9 ms |

For each job, the replay counts the time from the PostgreSQL commit of its first application write to the commit of its second. It clips that interval to each engine's middle 512 effects and groups jobs by their original runner PID. On Mesh, a runner had at most two such jobs at once in B and both same-image OFF arms. Among the runner-time with at least one job between commits, B had two jobs for 15.5–22.5%. River spent 48.0–54.5% of its corresponding runner-time with **three or more**. Oban also spent 43.1–48.8% with three or more, yet its middle writes were slower than Mesh's in four rotations. This occupancy includes client work, scheduling and response waits. More jobs between commits can mean longer waits as well as more concurrent service.

Rotation 2 is especially informative for a worker-supply change. Mesh already had slightly more runners carrying jobs between the two writes than River and still lost 947.0 ms in the middle. A third worker could overlap another handler on an occupied runner only when another handler has work to do. The ledger records PostgreSQL commits and handler entry. The pending work at each scheduler worker and the time it can execute on the two CPUs require separate evidence.

The [replay](owner_cadence.py) uses the 25 original measured arm hashes, including both Mesh OFF controls. It recovers River's runner PIDs from its product attempt rows by job key and transaction ID. The [receipt](owner-cadence.json) retains all 16 intervals, the wall-time distribution by active runner count, and runner-time by job count for both entry-to-second-commit and first-to-second-commit.
## How much of the middle gap can occur with few runners between writes?

On exact public DENSE B, each measured middle interval starts at the 256th effect commit and ends at the 768th. In four orders at least one handler had entered and had yet to commit its effect throughout that interval. The fifth order had 3.26 ms with zero such handlers. Global lack of entered work occupies little of the measured deficit. A particular runner can still lack work while others carry jobs.

For a more generous bound, count every microsecond when **at most one of the eight runner PIDs** had any job between its first and second PostgreSQL commit timestamps. Subtract *all* that time from B's interval, even though some of those jobs did productive work. Keep the rest of the schedule and the peer clocks fixed. Mesh still trails River in every order.

| Order | Mesh middle (ms) | Mesh minus River (ms) | Time with 0–1 PIDs between commits (ms) | Residual vs River (ms) | Mean entered jobs awaiting their first commit |
|---:|---:|---:|---:|---:|---:|
| 1 | 507.1 | +191.1 | 146.5 | +44.6 | 9.55 |
| 2 | 1,238.2 | +947.0 | 81.2 | +865.8 | 15.05 |
| 3 | 525.5 | +227.4 | 155.3 | +72.1 | 10.93 |
| 4 | 537.8 | +257.9 | 150.3 | +107.6 | 10.08 |
| 5 | 534.3 | +238.5 | 175.7 | +62.8 | 10.60 |

Order 2 also retains a **506.3 ms** deficit to Oban after the subtraction. Mesh was already faster through the middle than Oban in each of the other four orders. For a worker-supply change to recover the whole River gap, it must advance work during intervals with concurrent writes on multiple runners, or bring later grants and writes forward. A component that detects both workers blocked needs to show that the same PID also has a published, still-runnable attempt at that time. The ledgers delimit these windows but cannot identify socket waits or scheduler publication.

The [five-order receipt](owner-cadence.json) has SHA-256 `3941995e892bba9f8fafb20b95d92467d56a8962aad4ca874d046b2c2d0dbaa4`. The time with 0–1 PIDs sums bins 0 and 1 of `between_commits.wall_us_by_owners`. For the final column, weight each `owner_us_by_jobs` bin by its job count, subtract the between-commits sum from the entered sum, and divide by `span_us`. The calculation clips both boundaries to the same window and counts each overlapping job once per unit of wall time. This includes handler code, service handoff, scheduling and time to the first database commit.
