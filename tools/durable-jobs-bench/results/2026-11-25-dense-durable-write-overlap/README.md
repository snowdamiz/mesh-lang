# How closely dense application commits can overlap

In two reversed Linode rotations, the timestamps of Mesh's 2,048 application writes required at least **623 and 619 distinct durability events** with its two synchronous writers per runner. Even the unrestricted timing bounds were 506 and 503. River recorded 459 and 513 PostgreSQL WAL syncs for its entire workload in those same rotations. Both engines ran 1,024 jobs with two independent application commits each, on two CPUs shared with PostgreSQL 17.11. The comparison makes write overlap worth investigating alongside the service cost of a sync. It does not attribute a particular sync to an application write. [Native measurements and raw digests](native.json) · [WAL experiment](../2026-11-24-dense-wal-cost/NATIVE-RESULTS.md)

| Engine | Minimum events from commit times | Minimum with observed synchronous writer limit | PostgreSQL WAL syncs, whole workload | Distinct terminal XIDs |
|---|---:|---:|---:|---:|
| Mesh hot R2 | 506 / 503 | 623 / 619 | 1,694 / 1,650 | 875 / 858 |
| River | 94 / 99 | | 459 / 513 | 13 / 12 |
| Oban | 161 / 165 | | 932 / 1,013 | 1,024 / 1,024 |
| Apalis | 108 / 100 | | 514 / 507 | 54 / 56 |

The entries give the two rotations in order. In the middle of the drain, between the 103rd and 922nd second-write commit records, Mesh had 1,625 and 1,633 write intervals entirely within the window. These alone require 493 and 487 events with the synchronous writer limit. The middle lasted 891 and 888 ms for Mesh, compared with 356 and 340 ms for River. Jobs crossing a window edge are excluded from the interval counts. Each engine has its own window.

## Deriving a bound from ordinary receipts

PostgreSQL records a local write transaction's commit timestamp before waiting for its synchronous WAL flush. For the first application write, the handler waits for success before starting the second write. For the second, terminal settlement requires handler success. These facts give two intervals per job in PostgreSQL's clock, from the first commit record to the second, and from the second to the terminal record. By the end of each interval its write must be durable. The replay checks the two distinct application XIDs for every job and their separation from all terminal XIDs. It verifies the raw hashes, settings, system identifier and statistics reset identity for each WAL comparison. [PostgreSQL 17.11 commit implementation](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/access/transam/xact.c#L1338-L1492) · [Synchronous commit](https://www.postgresql.org/docs/17/wal-async-commit.html)

Writes whose intervals have no common time cannot have become durable in the same event. We allow closed endpoints and arbitrarily placed events, giving grouping every opportunity consistent with the receipts. At each earliest remaining deadline we take eligible intervals in deadline order. In the Mesh calculation the launcher pins two synchronous scheduler workers per runner, so each runner can contribute at most two in-flight application writes. The reduction verifies both runner PIDs against the ledger. It applies no per-runner cap to peers.

Moving the earliest event to the first remaining deadline preserves all assignments to it. At that deadline an eligible earlier-expiring write can replace a later-expiring write on the same runner. The latter remains eligible for the former's subsequent assignment. Filling all available places this way yields the minimum in the model. The tests compare the reducer with exhaustive feasible partitions on small inputs.

## What changing the bounds tells us

Six separate instrumented Linode arms captured the compiled handler returning from each INSERT. Joining the two returns by job, runner and order tightens the upper endpoints and raises the per-runner minima from 615–714 to **816–981**. These were checked-pool direct-handler arms with a scalar producer. Observation added 391 ms of drain in one arm. Their values describe those measured timelines and cannot be transferred to hot R2. [Handler-return records and observer effects](../2026-11-25-dense-handler-commit-return/README.md)

The cluster counters cover admission, application writes, settlement and other WAL activity. A minimum event count gives neither a per-role physical flush count nor a wall-time saving. Changing the runner can change the transaction timeline itself. The separate native logger-off readiness pilot failed its advancement gate. To isolate service cost, a comparison of PostgreSQL's durable sync methods can keep the application, CPU budget and durability flags fixed. With a synchronous-open method the cost can move into WAL writes, so both write and sync time and both durable throughput windows matter. [Readiness pilot](../2026-11-24-dense-hot-readiness/evidence/pilot/README.md)

## Distinguishing application overlap from delayed settlement

The first-write interval ends at the second application commit and can be computed without a terminal timestamp. Mesh still required at least 494 and 486 events for the 1,024 first writes alone. River required at least 93 and 99. Mesh moved each job from its first to its second commit in a shorter median time, 1.32 and 1.25 ms versus River's 4.73 and 4.24 ms. River moved more jobs through that stage concurrently. [Stage replay with raw hashes](stages.json)

| Mean jobs in stage during the middle 80%, rotations 1 / 2 | Entry to first app commit | First to second app commit | Second app commit to durable terminal |
|---|---:|---:|---:|
| Mesh hot R2 | 9.2 / 8.1 | 1.5 / 1.5 | 5.5 / 5.5 |
| River | 2.9 / 2.4 | 10.9 / 10.2 | 169.4 / 179.8 |
| Oban | 5.7 / 6.2 | 11.2 / 9.5 | 8.6 / 8.4 |
| Apalis | 4.5 / 4.7 | 7.4 / 8.4 | 23.5 / 23.2 |

For each stage the replay clips each job's interval to the middle window and divides their summed duration by elapsed wall time. The last column counts completed second writes awaiting a durable terminal record. River spent 96.8% and 96.9% of that window with at least 64 such jobs, while Mesh peaked at 17 and 16. River can accumulate acknowledgments after a handler releases an execution slot. A 64-slot configuration therefore describes different numbers of unsettled jobs across engines. Mesh's observed claim-to-terminal occupancy peaked at 60 and 59 during these windows. The ledger does not reveal local slot-release times or rule out contention on an individual runner. [Benchmark occupancy contract](../../README.md)

The separate qualified readiness rotation tested whether more write overlap helped Mesh. The mean number of jobs between first and second commits rose from 1.45 and 1.39 in the two synchronous controls to **4.31** with actor readiness. The corresponding first-write event floor fell from 506 and 519 to **142**. Yet the candidate's middle second writes took 904.8 ms against 904.6 and 914.6 ms, and its full drain took 31.6 ms longer than the faster control. Mean jobs between the second commit and settlement rose from 5.7 and 7.7 to 14.8. This is one valid rotation on an individual-settlement ELF. It shows a concrete change in overlap without a durable throughput gain. [Native pilot and excluded rotation](../2026-11-24-dense-hot-readiness/evidence/pilot/README.md)

## Offline replay

From the repository root, use the archived Linode receipts.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-durable-write-overlap
python3 -B "$d/test_bounds.py"
python3 -B "$d/replay.py" | cmp - "$d/native.json"
python3 -B "$d/stages.py" | cmp - "$d/stages.json"
```
