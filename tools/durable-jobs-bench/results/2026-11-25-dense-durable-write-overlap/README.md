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

## Offline replay

From the repository root, use the archived Linode receipts.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-durable-write-overlap
python3 -B "$d/test_bounds.py"
python3 -B "$d/replay.py" | cmp - "$d/native.json"
```
