# Time between confirmed settlements and the next dense grant

On the public eight-runner deployment, allowing an outstanding request to grow after a confirmed settlement produced 74–112 full eight-job refills per rotation. The same-image off controls produced zero to four. For the full refills in the enabled arm, the median interval from the previous jobs’ last durable terminal commit on that runner to the next database grant stamp was 19–26 ms. This interval includes terminal acknowledgement, scheduling, queue-lock acquisition, and part of the next checked SELECT. The archives cannot divide the time among them.

| Rotation | Full grants / productive grants | Full refills after first grant | Median last terminal → full grant | Partial grants with an older job still unsettled at first entry¹ | Runners between batches during middle 512 writes², B / public reference |
|---|---:|---:|---:|---:|---:|
| 1 | 120 / 134 | 112 | 21.3 ms | 5 / 13 | 2.40 / 2.46 |
| 2 | 82 / 159 | 74 | 25.8 ms | 44 / 76 | 1.34 / 2.30 |
| 3 | 113 / 136 | 105 | 19.1 ms | 11 / 22 | 2.14 / 2.50 |
| 4 | 110 / 137 | 102 | 23.9 ms | 9 / 26 | 1.98 / 2.35 |
| 5 | 112 / 137 | 104 | 20.9 ms | 6 / 25 | 2.12 / 1.94 |

¹ The denominator counts partial grants for which the current group plus jobs granted strictly later across all runners contained at least eight jobs. Work granted later may have been locked at the earlier instant. The numerator counts groups where even the first handler entry preceded a previous same-runner job’s terminal commit. Moving the cutoff five milliseconds later gives 5, 38, 9, 8 and 6. In these cases that seat could not have been credited from a confirmed prior settlement at grant time.

² Sum the intervals from the last terminal commit of all earlier same-runner jobs to the next productive grant, then divide by the observed middle-write duration. Clip each interval to effects 256–768 and subtract one millisecond from its right endpoint for the grant clock’s resolution. Startup and the interval after each runner’s final grant are excluded. A runner between batches can be executing claim or lifecycle code. The slowest B arm, rotation 2, had the fewest runners between batches on this measure. Reducing this count alone would be a poor advancement criterion. [Original phase measurements](LATE-CAPACITY-PEER-PHASES.md)

**Correctness constraint.** Asking PostgreSQL for even one extra provisional job and then rolling back changed what a concurrent recovery worker could promote with `SKIP LOCKED`. A trusted retry committed the same two jobs yet left the next job scheduled. Commit `de8eeeae7` preserves the PostgreSQL 17.11 witness. A candidate can use a genuine terminal receipt already processed by the runner to increase its reservation before taking a job lock. The frequency of such receipts at the instant a partial request enters the claim lane remains unknown.

From this directory, run `python3 -B grant_capacity_frontier.py | cmp - grant-capacity-frontier.json`. Replay verifies the release seal, controller scores and archive hashes for 20 measured Mesh arms. Each arm reconciles 1,024 jobs and 2,048 distinct application XIDs. The number of runner-and-stamp groups equals checked calls minus excess marker rows in every arm. That equality rules out two productive grants on one runner sharing a stamp in these captures. [Per-arm measurements](grant-capacity-frontier.json) · [Released peer comparison](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md)
