# Which index did the dense grant UPDATE read?

In the two native eight-slot fixtures, PostgreSQL chose a bitmap scan of `mesh_jobs_unique_candidates` even in automatic plan mode. For each of the eight locked jobs it built another bitmap with over a thousand index entries. At sixteen slots the automatic plan used one primary-key lookup per job.

| Seeded stock case on PostgreSQL 17.11 | Grant rows | UPDATE-side target index | Rows per index scan | Scan loops |
|---|---:|---|---:|---:|
| 1,024 ready, 16 slots, auto | 16 | jobs_pkey | 1 | 16 |
| 512 ready, 16 slots, auto | 16 | jobs_pkey | 1 | 16 |
| 512 ready, 8 slots, auto | 8 | mesh_jobs_unique_candidates | 1,048 | 8 |
| 8 ready, 8 slots, auto | 8 | mesh_jobs_unique_candidates | 1,046 | 8 |
| Empty, 8 slots, auto | 0 | mesh_jobs_unique_candidates | 0 | 0 |

The eight-slot bitmap nodes produced approximately 8,384 and 8,368 index entries across their loops. PostgreSQL reports a rounded per-loop average. The bitmap heap scan can stop early once its parent has enough rows. Index entries and rows fetched from the heap are different counters. All values come from the UPDATE subtree of the stock grant. The ordered selection is a separate subtree. The automatic-plan fixture uses five savepoint rollbacks before the captured execution. Both forced modes also chose the broad index for the sixteen-slot inputs. [Native fixture and plans](../2026-11-24-public-dense-bounded-grant/evidence/native/README.md)

In the six real grouped-hot drains, 72–127 of the 1,024 jobs per arm shared their runner and grant timestamp with at most seven other jobs. The six individual-terminal drains had 43–152 such jobs per arm. A timestamp can combine multiple claims on one runner. These are lower bounds on jobs served by small grants. The ledgers omit the requested width and the plan selected for each grant.

The same-statement tuple lookup failed its concurrent-update proof. Stock granted jobs 1 and 2 in order. The physical lookup granted job 2 alone because its scan could not see the newly locked tuple version in the statement snapshot. The candidate is closed. An earlier UUID-array predicate scanned 1,024 rows in the empty case. Any future target lookup needs to preserve the full ordered prefix in that race, the zero-work empty case, and the efficient sixteen-slot plan. Live plan frequency remains open. [Concurrent-update witness](../2026-11-25-public-dense-tid-grant/README.md) · [UUID-array decision](../2026-11-24-public-dense-bounded-grant/evidence/native/README.md)

Run `python3 -B replay.py | cmp - native.json` here to verify the native plan archive and twelve measured workload archives. The resulting [node counts and grant timestamps](native.json) cover all five fixture shapes and three planner settings.
