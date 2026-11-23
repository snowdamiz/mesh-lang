# Native grouped-success and bulk enqueue on slow jobs

On November 23, 2026, the previously qualified V2 short image ran the
frozen slow workload without a build or storage changes. Each arm handled
512 jobs with 16 KiB payloads, a 100 ms handler hold, and two separately
committed application writes. PostgreSQL 17.11 and the queue shared CPUs
0 and 1. The physical limit was 68 connections and the slot limit was 64.
[Exact inventory](replay/inventory.json) · [SETUP release](controller/bulk-enqueue-v2-slow-qualification.released.json)

Both V2 modes used the same Linux ELF, SHA-256
`55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a`.
They used `success_cohort` terminal settlement. The V1 control lanes
kept one terminal transaction per job. SETUP verified 512 completed jobs,
64 occupied slots and a peak of 13 connections on both V2 lanes. The
batch writer submitted eight bulk statements and eight scalar remainders.
The scalar control used 512 scalar inserts. [Native proof](evidence/native-proof-01/qualified.json)

| Lane | Submit-to-settlement jobs/s | First commit-to-settlement jobs/s | Submit-to-commit (ms) |
| --- | ---: | ---: | ---: |
| V2 bulk | 260.90 | 363.53 | 553.83 |
| V2 scalar | 254.10 | 383.25 | 678.84 |
| V1 bulk | 263.05 | 360.88 | 527.72 |
| V1 scalar | 243.40 | 351.32 | 645.52 |
| Oban | 236.63 | 310.37 | 515.38 |
| River | 273.80 | 368.18 | 475.24 |
| Apalis | 342.64 | 450.95 | 359.51 |

The table reports medians across two opposite orders. V2 bulk saved
4.24 ms and then lost 35.84 ms versus V1 bulk from first submit to last
settlement. It finished 57.82 ms ahead of River once and 221.24 ms behind
once. The pilot ended at two groups under the extension rule. V2 bulk
recorded 476 and 469 terminal XIDs, with maximum cohorts of seven and
eight. Host CPU steal was zero across the measured groups. [Pilot decision](evidence/bulk-enqueue-v2-slow/pilot-decision.json) · [Native summary](evidence/bulk-enqueue-v2-slow/summary-partial.json)

## Replay

Use a fresh directory three levels below the Git root. Copy the frozen
driver and its evidence there. The summarize exit status is 1 for this
intentional two-group pilot. Compare its output with `summary-partial.json`.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-bulk-enqueue-v2-slow
parent=$(mktemp -d "$PWD/target/v2-slow-replay.XXXXXX")
mkdir "$parent/packet"
cp "$packet/replay/series.py" "$parent/packet/"
cp -R "$packet/evidence" "$parent/packet/"
python3 -B "$parent/packet/series.py" summarize bulk-enqueue-v2-slow
```
