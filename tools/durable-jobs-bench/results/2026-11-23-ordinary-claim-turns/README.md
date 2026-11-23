# Dense claim batches and unused slots on the Linode

Eight ordinary trusted Mesh V1 runners handled 1,024 dense jobs. They recorded **361 contention hints** and PostgreSQL counted **576 claim calls** from just before submission until just after the final terminal commit. At least 353 hints fall inside that counter window. Each hinted call returned zero jobs despite an optimistic snapshot showing eligible work and spare queue capacity. The snapshots cannot identify which lock blocked an individual call. [Frozen runner and claim SQL](../2026-11-23-ordinary-trusted-v1/README.md) · [Arm inputs and reductions](result.json)

Between the enqueue commit and the last recorded claim stamp, **13.79–14.25 jobs** on average had a claim stamp and still awaited terminal commit, out of 64 configured slots. On average **5.19–5.24 of the eight runners** had zero jobs in this interval-based count. The peak was 29–30 jobs. The busiest runner completed 273 jobs and the least busy completed 19. This was one measured native rotation. These observations motivate a same-binary test of runner count, keeping total slots and warmed application connections fixed. [Native four-engine rates](../2026-11-23-iteration-comparison/four-engine-scorecard.md)

| Measured arms | Claim statements per arm | Contention hints | Bounds on mean jobs per granting statement |
|---|---:|---:|---:|
| Dense, 1 group | 576 | 361 | 4.59–5.57 |
| Short, 5 groups | 22–28 | 0 | 4.57–7.53 across groups |
| Slow, 5 groups | 58–69 | 0 | 7.42–12.49 across groups |

## Derivation

A granting transaction cannot commit before it stamps every row it grants or after the first handler in that grant enters. Each job therefore gives a closed interval between its database claim stamp and its recorded handler entry. The claim stamp is floored to milliseconds. The replay groups intervals by the actual trusted runner PID, verifies each PID against the CPU affinity capture, and checks its job count against the runner exit reports. The maximum batch width is the smaller of 16 and the local slot limit.

For each PID, the replay places a hypothetical grant at the earliest handler entry among remaining jobs. It fills that grant with eligible intervals in earliest-deadline order up to the width. This gives the minimum number of grants compatible with these intervals. In any optimal assignment, grants earlier than that first deadline can move up to it. A later-finishing eligible job in the first grant can trade places with an earlier-finishing eligible job assigned later. The displaced job already started and can tolerate that later instant. Repeating this exchange produces the greedy first grant without increasing the number of grants. Reapplying it to the remaining jobs gives the bound. Actual database and runner constraints can require more grants. [Replay](replay.py) · [Independent exhaustive checks](check_bounds.py)

The dense lower bound is **184 granting statements**. For the upper bound, the opening PostgreSQL counter snapshot precedes the single enqueue. A contention hint needs a visible eligible row. At the closing snapshot, each runner can still have one claim query that saw such a row before the final settlement and finishes afterward. Subtracting the 361 lifetime hints from 576 measured calls and adding back eight possible boundary cases gives **at most 223 granting statements**. This upper bound also counts any ordinary empty calls. The timing of unsuccessful calls and their individual CPU costs are unavailable. Freshly scheduling the workload could change the entire sequence.

For the residence calculation, the fixed window lasts **2.225080 seconds**, from enqueue commit to the last floored claim stamp. Each job begins contributing at its own claim stamp and stops at its terminal commit. The replay first uses the floored stamp, then the latest possible microsecond in that millisecond capped at handler entry. The two assignments enclose all possible true stamps. This measures the job timeline. A runner can also have reservations awaiting a claim reply, or terminal acknowledgements in flight. PostgreSQL and the client ran on the same host. The calculation assumes a common realtime clock without a backward jump during the arm. [Native collection](../../native/ledger.py)

The replay verifies **4,224 jobs**, one enqueue transaction per arm, a distinct terminal XID per job, and two independent application XIDs per job across these eleven workload arms. It checks the database settings, successful exits, runner totals and raw archive hashes against the saved reduction.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-ordinary-claim-turns/replay.py --check
python3 -B tools/durable-jobs-bench/results/2026-11-23-ordinary-claim-turns/check_bounds.py
```
