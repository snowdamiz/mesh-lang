# Slow V1 runner concentration

The 2×32 deployment saved a paired median **261 ms from submit** and **178 ms after enqueue commit** against the faster 1×64 duplicate. Identical controls differed by a median 4,932 ms from submit and 3,234 ms after commit. One of six submit comparisons cleared both 100 ms and the duplicate spread. Zero drain comparisons met both conditions. The result qualifies a native diagnostic. A throughput gain across orders remains unproven. [Local scores and full ranges](evidence/local-summary.json)

## Deployment and work proof

All 18 timed arms used the retained true-bulk Darwin V1 producer and runner. Each arm processed the same 512 canonical 16 KiB payloads at 64 slots, with a 100 ms application hold and eight warm application sessions. Each arm started a fresh PostgreSQL 17.11 cluster with the physical cap of 68, verified TLS, fsync, synchronous commit, full-page writes, and commit timestamps. The host had 15 logical CPUs and no pinning. Launch load ranged from 26.7 to 181.5. The six rotations used all permutations of two fresh 1×64 controls and the 2×32 deployment. [Frozen input and build pins](../2026-11-23-slow-storage-transfer/frozen/local-pins.json) · [Raw evidence](evidence/local-raw.tar.gz)

| Layout | Runners | Slots per runner | Scheduler workers total | Queue sessions | Warm app sessions | App backends that wrote | Broker sessions | Peak physical clients |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1×64 A and B | 1 | 64 | 2 | 4 | 8 | 2 | 1 | 13 |
| 2×32 | 2 | 32 | 4 | 8 | 8 | 4 | 1 | 17 |

Every timed arm completed and reconciled all 512 jobs, with one producer XID, 1,024 application XIDs, and 512 terminal XIDs, all independent. The same warm physical backends persisted through the terminal observation. All arms reached 64 concurrent committed hold intervals. The two-runner arms reached 32 on each runner and finished 245 to 267 jobs per runner. The observer recorded zero capacity violations, retries, recovery, stale acknowledgments, or lost jobs. The statement ledger shows eight bulk calls granting 504 inserts and eight scalar calls inserting the remainder in every timed arm. The untimed role-scoped PostgreSQL wire log verifies the exact arguments and their 512 positions, in eight consecutive groups of 63 followed by eight scalar positions. The first parser rejected a multiline SQL log entry. The final replay reads the continuation and validates the original log. [Work and position replay](evidence/local-summary.json) · [Six rejection checks](evidence/offline.json)

## Elapsed time and committed holds

Each difference below is the faster control minus 2×32 for that metric. Positive values favor 2×32. Times are in milliseconds.

| Order | Submit saved | Submit control spread | Drain saved | Drain control spread | Vacancy saved before last claim |
| --- | ---: | ---: | ---: | ---: | ---: |
| A, 2×32, B | +356 | 2 | −60 | 466 | +37 |
| B, 2×32, A | +93 | 6,322 | −227 | 6,455 | −58 |
| 2×32, A, B | +391 | 1,561 | +185 | 1,059 | +114 |
| B, A, 2×32 | +3,992 | 5,225 | +2,931 | 5,536 | +498 |
| A, B, 2×32 | +59 | 5,895 | +171 | 4,074 | +149 |
| 2×32, B, A | +166 | 4,639 | +236 | 2,393 | +87 |

The 2×32 arm won the drain with the candidate first or last. Both middle-position runs lost. The unchanged producer phase also ran sooner in the candidate. Median submit-to-commit was 621 ms for 2×32 and 1,088 or 1,054 ms for the duplicates. The run cannot attribute this producer difference to refill. Group 4 remains in the record along with the other noisy controls.

| Median phase in ms | A | B | 2×32 |
| --- | ---: | ---: | ---: |
| Submit to terminal | 2,549.1 | 4,723.7 | 1,916.6 |
| Enqueue commit to terminal | 1,509.3 | 3,166.3 | 1,257.9 |
| Enqueue commit to last claim | 1,352.7 | 2,793.5 | 1,110.2 |
| Unoccupied through last claim, normalized | 259.5–267.4 | 580.0–587.9 | 167.4–175.4 |
| First hold starts at attempt commit | 37.7 | 57.2 | 14.1 |
| Last hold starts at attempt commit | 1,386.3 | 2,823.3 | 1,115.5 |
| First hold ends at effect commit | 149.0 | 181.0 | 129.8 |
| Last hold ends at effect commit | 1,502.8 | 3,096.8 | 1,241.1 |
| Claim to handler entry, normalized | 114.4 | 426.6 | 49.4 |
| Handler entry to first write, normalized | 42.3 | 254.4 | 28.9 |
| Between writes, normalized | 939.9 | 1,302.6 | 875.8 |
| Second write to terminal, normalized | 129.5 | 240.8 | 75.9 |

The hold and claim offsets start at enqueue commit. The normalized rows sum each slot's intervals and divide by 64. The vacancy bounds use the original V1 method. The lower bound uses the millisecond claim stamp, and the upper bound shifts the stamp by 1 ms at most, capped at handler entry. The faster-control comparison puts the median savings at 100.4 ms of vacancy before the last claim and 43.1 ms of normalized residence after the second write. The last effect commit moved by a paired median 209.1 ms, with a range of 224 ms later to 2,748 ms earlier. [Original native V1 phase definitions](../2026-11-23-iteration-comparison/slow-slots.md) · [Per-job ledgers](evidence/local-raw.tar.gz)

## Extra refill work and native transfer

The 2×32 arm used 0.598 runner CPU ms per job (0.576 to 0.681). Control A used 0.485 (0.397 to 0.653), and control B used 0.517 (0.465 to 0.631). The median increase against the lower-CPU duplicate was 0.153 ms per job, with an increase in every rotation. Runner CPU includes startup, idle, and drain. The candidate made 80 claim calls at the median against 59.5 and 65.5, and recorded 5 to 26 contention hints against zero. All three arms still settled one job per SQL call. PostgreSQL's median claim statement execution time was 193 ms for 2×32, 196 ms for A, and 166 ms for B. The surrounding due probes took 17, 13, and 13 ms respectively. This SQL diagnostic brackets the official counters and includes one additional idle claim in two arms. [SQL phase evidence](evidence/local-sql.json)

The four-runner arm was omitted. Splitting into two already added admission work in every rotation, while the drain result depended on order. The conditional structural refill test for a clear CPU and wall loss did not trigger. The experiment found a CPU cost and a possible drain benefit.

The native pilot compared 2×32 with duplicate 1×64 V1 controls and stock 1×64 River, Oban, and Apalis. All arms used 64 slots and a PostgreSQL cap of 68 on CPUs 0 and 1. The original launcher failed before starting Mesh because Python could not enumerate the private staging directory. The failed attempts remain archived. The retry loaded the pinned helper by path through the same directory permissions. Both opposite-order blocks reconciled. [Original attempts](evidence/native-import-failed/) · [Retry request](request-import-retry.json) · [Native replay](evidence/native-import-retry-replay.json)

| Deployment | Block 1 submit / drain jobs/s | Block 2 submit / drain jobs/s |
| --- | ---: | ---: |
| Mesh true-bulk V1 2×32 | 288.83 / 418.77 | 281.60 / 399.98 |
| Mesh true-bulk V1 1×64 A | 260.28 / 360.07 | 263.04 / 358.96 |
| Mesh true-bulk V1 1×64 B | 264.06 / 364.48 | 233.02 / 306.40 |
| Oban 1×64 | 246.47 / 328.84 | 243.37 / 327.19 |
| River 1×64 | 257.59 / 335.92 | 253.44 / 331.34 |
| Apalis 1×64 | 384.32 / 508.25 | 342.67 / 442.44 |

Mesh 2×32 saved 166 and 128 ms from submit, and 182 and 146 ms from commit, against the faster Mesh duplicate. The savings exceeded the duplicate spread in the first block on both clocks. Connection peaks were 17 for Mesh 2×32 and 13 for each 1×64 duplicate. The two-block pilot used the alternate Mesh deployment and stock peer topologies. [Raw arms and hashes](evidence/native-import-retry/SHA256SUMS)

[Replay the packet](REPLAY.md)
