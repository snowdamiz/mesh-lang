# Slot use during the five ordinary Mesh slow-job runs

Each native run used the original V1 trusted runner, the set-based batch producer, 64 slots, and 512 jobs with a 100 ms hold. Time runs from the PostgreSQL enqueue commit. Unoccupied time sums the free slots at each instant and divides by 64. A fully empty runner for 1 ms therefore contributes 1 ms. All time columns are milliseconds.

| Rotation | Drain | Last claim stamp | Unoccupied through last claim | Unoccupied through drain | Claim to entry | Entry to first write | Between writes | Second write to terminal |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1455.6 | 1329.4 | 344.0–352.0 | 402.0–410.0 | 87.9 | 23.0 | 838.5 | 104.1 |
| 2 | 1423.5 | 1293.9 | 348.1–356.0 | 383.0–391.0 | 75.8 | 22.8 | 832.7 | 109.2 |
| 3 | 1435.6 | 1297.1 | 348.6–356.6 | 406.0–414.0 | 73.7 | 20.9 | 832.6 | 102.3 |
| 4 | 1444.9 | 1326.6 | 350.2–358.1 | 399.2–407.2 | 80.1 | 25.7 | 832.0 | 107.9 |
| 5 | 1430.9 | 1295.3 | 332.5–340.5 | 390.2–398.2 | 94.4 | 24.7 | 827.5 | 94.1 |

The two ends of each unoccupied-time range use the recorded claim stamps and stamps shifted up to 1 ms later, capped at actual handler entry. The last-claim cutoff stays fixed at its recorded value in both calculations. The four residence columns use the recorded stamps and are normalized by the same 64 slots. These ledger intervals include waiting and execution. They do not assign CPU costs to the dispatcher, the database or the handlers.

## Last application write against each peer’s final settlement

A positive number means Mesh was still doing required application work after the peer had durably finished, measured from each arm’s own enqueue commit. With handler timings fixed, removing the final settlement delay could not close that drain gap.

| Rotation | Oban | River | Apalis |
|---|---:|---:|---:|
| 1 | -62.2 | +107.2 | -21.5 |
| 2 | -208.2 | +84.8 | +360.9 |
| 3 | -187.4 | +116.3 | +338.5 |
| 4 | -111.0 | -123.7 | +267.7 |
| 5 | -116.9 | +101.0 | +303.8 |

Earlier admission or earlier confirmed settlement can advance later handlers, which changes their observed write timestamps. The build-free V2 comparison tests that effect with the same application calls and the same slot and connection caps. [Native scores](four-engine-scorecard.md) · [Run ledgers](../2026-11-23-set-based-enqueue/evidence/bulk-enqueue-v1-slow) · [Replay](slow-slots.py)
