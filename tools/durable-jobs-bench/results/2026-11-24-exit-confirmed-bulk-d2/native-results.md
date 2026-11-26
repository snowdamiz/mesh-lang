# Exit-confirmed R2 short comparison on Linode

Five matched 128-job blocks reconciled on November 24, 2026 UTC. PostgreSQL
17.11 and workers shared CPUs 0 and 1. The new SETUP used the retained D2
ELF `972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7`
and the original writer-off control. It qualified four D2 fault cases,
three writer-off control cases, and the two diagnostic workload arms.
[Recovered qualification](recovery/evidence/qualification.json) ·
[Original failed harness attempt](evidence/exit-bulk-d2-c7c50e726ca0/)

| Lane | Median submit jobs/s | Median drain jobs/s | Median submit p99 (ms) |
| --- | ---: | ---: | ---: |
| R2 D2 | 626.59 | 762.06 | 202.29 |
| R2 off | 600.51 | 720.80 | 212.57 |
| Protected Mesh | 193.59 | 221.68 | 654.17 |
| Oban | 285.49 | 321.83 | 448.32 |
| River | 566.19 | 612.64 | 226.07 |
| Apalis | 808.73 | 874.10 | 157.95 |

The D2 arms issued two checkout probes per block. The control issued 256.
Against R2 off, D2 won three of five blocks on each clock. Median paired
throughput changes were +3.58% from submit and +1.14% from enqueue commit.
The ranges were −16.04% to +7.15% and −13.09% to +8.74%, respectively.
Those ranges leave the throughput effect inconclusive. The exact same
preserved blocks show three wins against River, five against Oban, and
zero against Apalis on each clock. [Reconciled raw blocks and distributions](evidence/native-timing/2026-11-24-exit-confirmed-bulk-d2/summary.json)
