# Short jobs with stock and prewarmed River

Six matched Linode rotations measured 128 jobs per arm. Each job committed two independent application writes.
All durations below are milliseconds. Negative differences mean the first lane finished sooner.

| First lane minus second | Submit to settle | Drain from commit | Enqueue | Commit to 16th write pair | Remaining 112 write pairs | Last write to settlement | Wins submit / drain |
|---|---:|---:|---:|---:|---:|---:|---:|
| river-warm minus river-cold | -59.65 | -60.58 | +0.20 | -56.37 | +12.20 | -26.90 | 5 / 5 |
| river-cold minus river | +0.48 | +1.03 | -0.60 | -0.20 | -1.78 | -1.63 | 2 / 3 |
| r2-d2 minus river | -32.21 | -63.75 | +26.44 | -58.31 | +69.58 | -68.14 | 4 / 5 |
| r2-d2 minus river-warm | +60.35 | +39.96 | +26.37 | +1.74 | +66.54 | -24.36 | 1 / 2 |
| r2-d2 minus oban | -230.03 | -220.54 | -9.89 | -188.28 | -37.55 | +0.86 | 6 / 6 |
| r2-d2 minus apalis | +61.90 | +31.53 | +31.57 | -4.66 | +51.63 | -2.20 | 1 / 1 |
| r2-d2 minus r2-off | -13.72 | -17.12 | +3.40 | -1.93 | -9.33 | +0.23 | 4 / 4 |

Each column is the paired median. The component intervals sum to the elapsed difference within each rotation. Their medians can come from different rotations.
Write cohorts use second application commit order. River warm and cold use one executable. The separate river lane uses the original stock executable.
R2 D2 and R2 off use the exit-confirmed, writer-off Mesh runner with hot and checked application pool checkout respectively.

[Per-rotation data and executable evidence](evidence/summary.json)
