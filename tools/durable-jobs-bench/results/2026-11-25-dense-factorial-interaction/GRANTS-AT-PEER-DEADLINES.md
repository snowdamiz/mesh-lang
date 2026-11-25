# Dense 8×8 grants at River’s durable milestones

When River completed its 768th application effect, Mesh had granted fewer than 768 jobs in every matched rotation. Both clocks below start at each engine’s own durable enqueue commit. Even instantaneous handler execution for every job already stamped would have left Mesh short of that milestone by 39, 389, 65, 84 and 103 jobs on the recorded grant schedules. Faster handlers can free seats and advance subsequent grants in a real run.

| Rotation | Mesh jobs granted at River effect 768 | Mesh effects at River effect 768 | Mesh jobs awaiting a grant at River final settlement, from enqueue commit | Mesh jobs awaiting a grant at River final settlement, from first submit |
|---|---:|---:|---:|---:|
| 1 | 729 | 713 | 0 | 65 |
| 2 | 379 | 358 | 413 | 453 |
| 3 | 703 | 688 | 0 | 127 |
| 4 | 684 | 660 | 31 | 159–163 |
| 5 | 665 | 647 | 74–82 | 176 |

In rotations 1 and 3, all Mesh jobs had grant stamps by River’s final settlement on the drain clock. On the full submit clock, the last Mesh grant stamp came at least 69, 1,168, 118, 188 and 200 ms after River’s final settlement across the five rotations. Earlier enqueue and sustained replacement work both matter to this comparison. The result leaves the division between queue contention, time to confirm free seats, and downstream handler costs open.

All 22 PostgreSQL lock timeouts occurred in rotation 2. For each error, the preceding 100 ms contained at least 9 and as many as 35 newly stamped jobs on 2–5 runners, as well as 11–44 durable effects. These windows can overlap. They demonstrate continuing queue progress during the affected part of the run. They do not time a particular lock holder. [Lock sites and physical host samples](LATE-CAPACITY-PEER-PHASES.md)

The replay verifies the four measured engines’ 20 raw archives and the controller release. It matches every application payload by job key, both independent application transactions per job, the enqueue ID maps and the final clocks. A grant stamp represents a millisecond interval. The count uses both ends of that interval, with handler entry supplying a tighter upper bound when available. All peer comparisons use elapsed times relative to the specified origin. Counts at first submit and enqueue commit answer different questions. The original paired peer verdicts remain inconclusive.

Run from this directory to reproduce all counts, including the Oban and Apalis milestones.

```sh
python3 -B peer_grant_progress.py | cmp - peer-grant-progress.json
```

## Equal start at the 256th durable effect

Align each engine at its own 256th committed second write. This removes the producer and first 256 effects from the comparison. River completed the next 512 effects in 280–316 ms. Giving Mesh the same time in each rotation yields the counts below. Each Mesh job keeps a seat through its terminal receipt. With 64 seats, at least 704 of the first 768 effect-complete jobs must already have settled for Mesh to reach that milestone. The database settlement counts give Mesh credit immediately at the commit timestamp, ahead of client receipt.

| Rotation | Mesh effects | Mesh settled | Completed effects available to settle immediately | Additional Mesh effects needed even to reach 704 settlements |
|---|---:|---:|---:|---:|
| 1 | 574 | 555 | 19 | 130 |
| 2 | 454 | 447 | 7 | 250 |
| 3 | 545 | 533 | 12 | 159 |
| 4 | 521 | 511 | 10 | 183 |
| 5 | 537 | 518 | 19 | 167 |

At River’s 768th effect, its own database had recorded only 464–548 terminal settlements. Its separation of application completion from terminal persistence matters when comparing engine throughput. Mesh would need 149–257 more settlements at these deadlines. Settling all 7–19 already-completed effects at the deadline would still leave the deficits in the last column. To close the gap, cheaper settlement would have to free seats earlier **throughout** the middle window so that more replacements commit their application writes.

At Oban’s corresponding durations, Mesh had already reached 856–890 effects in four rotations. In rotation 2 it had reached 619, including 617 settled. This pattern survives alignment at the 256th write. The counts describe the recorded schedules and give no causal estimate of the feedback from a faster settlement or scheduler.

The [middle-window counts and raw hashes](middle-refill-capacity.json) include both peers and grant timestamp bounds. Reproduce them with `python3 -B middle_refill_capacity.py`.
