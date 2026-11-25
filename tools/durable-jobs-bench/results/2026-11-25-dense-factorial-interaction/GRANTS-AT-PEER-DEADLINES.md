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
