# When durable completion happens in the four native queues

The five short rotations of ordinary Mesh V2 used **111–123 terminal transactions for 128 jobs** with the grouped-success policy enabled. The largest group was three to five jobs. Mesh beat Oban in all five submit and drain comparisons. River won all five from submission, and Mesh won two from enqueue commit. Set-based admission also changed in the candidate, so the elapsed comparison prices both changes together. [Matched four-engine scorecard](../2026-11-23-iteration-comparison/four-engine-scorecard.md)

| Short workload, five V2 blocks | Terminal transactions per 128 jobs | Largest terminal transaction, jobs | Peak jobs with both effects committed and terminal commit pending |
|---|---:|---:|---:|
| Mesh trusted, stock control | 128 | 1 | 6–11 |
| Mesh trusted, grouped successes | 111–123 | 3–5 | 5–11 |
| Oban | 128 | 1 | 14–16 |
| River | 1–2 | 122–128 | 128 |
| Apalis | 15–18 | 15–16 | 16 |

River used 16 handler slots throughout short. Every job had already committed both writes before the first terminal commit in each of these five runs. This lets River accumulate a completion batch larger than its handler count. Mesh V2 keeps its application and database slot charged through settlement. Apalis grouped most short successes while its peak outstanding two-write count fit in 16 slots. These counts follow the terminal rows' transaction IDs and PostgreSQL commit timestamps. The ledger does not record the instant a handler returned to its runtime.

The first native dense V1 block showed a different scale. Mesh issued 1,024 terminal transactions and had up to 13 jobs with both writes committed and terminal commit pending. Oban issued 1,024 and peaked at 30. River issued 30 terminal transactions and peaked at 429. Apalis issued 186 and peaked at 56. The older protected baseline has five dense rotations and independently reports its terminal transaction counts. [Original stage analysis](../2026-11-23-baseline-stage-analysis/report.md)

## A bounded batch experiment

For each trusted Mesh runner PID, the replay takes its actual second application commit timestamps as a hypothetical stream of eligible successes. The oldest pending item starts a timer. A group flushes at eight items or at the deadline, whichever comes first. Items exactly on the deadline can enter that group. Streams are independent and keep their original arrival times.

On the five V2 short candidate traces, a 1 ms deadline produces **68–86 predicted groups** and 2 ms produces **42–60**. On the one eight-runner V1 dense trace, the corresponding counts are **901 and 788** for 1,024 jobs. An actual writer must wait for handler return, honor cancellation and lease expiry, send the transaction, and receive its durable acknowledgement. Changing when it frees a slot can move subsequent arrivals. This replay prices potential batch size only. The compiled experiment measures both timing windows and checks that the original job-slot limit still holds through confirmed settlement.

## Replay

The CSV covers 110 measured arms, 26,560 completed jobs, and 53,120 independently committed application writes. It preserves per-arm results and the SHA-256 of each source archive. The replay compares each archive to its matrix receipt, verifies distinct application transaction IDs, and checks that all rows in a terminal transaction have the same commit timestamp. Intervals start on handler entry or second write and end on terminal commit. Simultaneous starts and ends use half-open intervals. [Per-arm CSV](batches.csv) · [Replay code](replay.py)

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-native-completion-batches/replay.py --check
```
