# Concurrent application transactions witnessed by the durable ledgers

Prewarmed River had at least **14–39 application transactions with an assigned XID still awaiting their commit timestamp** at one instant in each of six dense drains. These lower bounds come from the 2,048 independent application writes, rather than the count of open sockets. They show that River used substantial database concurrency in all six runs. They do not establish that matching its peak will reproduce its throughput.

| Rotation | Mesh peak lower bound | Same-image cold River | Prewarmed River | Warm River lower bound within middle 512 second writes | Warm River drain time with at least nine proven concurrent |
|---|---:|---:|---:|---:|---:|
| 1 | 2 | 8 | 21 | 21 | 8.2 ms |
| 2 | 3 | 15 | 23 | 4 | 9.9 ms |
| 3 | 2 | 5 | 39 | 20 | 39.7 ms |
| 4 | 1 | 13 | 14 | 4 | 3.6 ms |
| 5 | 1 | 29 | 14 | 10 | 6.7 ms |
| 6 | 2 | 12 | 14 | 14 | 2.8 ms |

In the separate released individual-settlement pilot, actor-owned PostgreSQL readiness raised the witnessed peak to eight from two in both synchronous controls. In its qualified rotation it still drained **31.6 ms slower** than the faster control. Its first rotation failed qualification. The current fixed8 direct-pool and scheduler comparison must therefore advance actual durable writes and settlement to justify the extra concurrency. [Individual pilot](../2026-11-24-dense-hot-readiness/evidence/pilot/README.md)

## What the replay proves

A higher ordinary XID must have been assigned by its own commit timestamp. Each lower XID had already been assigned by then. If a lower-XID write's own commit timestamp comes later, the intervening interval is a witnessed part of its transaction lifetime. For each lower XID, the replay takes the earliest commit among it and all higher application XIDs in the same fresh database. It sums the resulting half-open intervals, treating simultaneous endpoints together. Each job's two application commits have distinct increasing XIDs, and every arm stays within one nonwrapping XID range. The commit witness is the PostgreSQL record timestamp. Client return and confirmed WAL flush can come later.

In-order commits can overlap with no inversion, so a small floor alone cannot establish a small actual peak. Insetting each end of every inferred interval by 2 ms as a sensitivity check gives warm-River full-drain peak floors of 7, 13, 35, 8, 13 and 1. The longer intervals strongly corroborate some rotations. The measurements do not isolate disk flush time or CPU time.

From this directory run `python3 -B inflight_xids.py | cmp - inflight-xids.json`. The replay verifies all 21 measured archive hashes and their original SQL/XID ledgers, including 18 from the matched River-readiness series.
