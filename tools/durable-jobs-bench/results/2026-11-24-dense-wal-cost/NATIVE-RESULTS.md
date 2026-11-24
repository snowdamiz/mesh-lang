# PostgreSQL WAL on the two-CPU Linode

The middle 820 application writes took Mesh 535 and 547 ms longer than River across two opposite-order runs. Each engine ran two 32-slot runners with PostgreSQL 17.11 on CPUs 0 and 1. All 16 warmup and measured arms reconciled.

| Engine | WAL syncs, orders 1 / 2 | Cumulative sync ms, orders 1 / 2 | Middle write span ms, orders 1 / 2 |
|---|---:|---:|---:|
| Mesh hot R2 | 1,694 / 1,650 | 821 / 806 | 891 / 888 |
| River | 459 / 513 | 488 / 450 | 356 / 340 |
| Oban | 932 / 1,013 | 793 / 781 | 874 / 863 |
| Apalis | 514 / 507 | 367 / 375 | 516 / 496 |

The PostgreSQL counters cover the whole workload. They aggregate time across processes. The device counters separately bracket the drain and cover every user of the filesystem. Neither measurement assigns a flush to a particular job or gives the time saved by removing one. Mesh’s cumulative WAL write and sync totals of 845 and 830 ms leave open whether flushes limit late application writes. The separate completion experiment tests whether grouping additional ready outcomes actually brings those writes forward.

From this directory, `python3 -B controller.py replay` reconstructs the archived [full result](evidence/native/final-replay.json) from the raw captures. [Controller release](evidence/native/controller-release.json)
