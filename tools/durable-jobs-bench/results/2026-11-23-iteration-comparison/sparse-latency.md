# Response time for sparse durable jobs

5 matched rotations of 24 singleton submissions per engine. Each submission followed the preceding settlement and a 250 ms pause. Each job committed its two application writes independently. The pause falls outside each job’s response time. Times are milliseconds.

| Engine | Submit to settle p50 | Submit to settle p99 | Commit to pickup p99 | Commit to settle p99 | Worst submit to settle |
|---|---:|---:|---:|---:|---:|
| Trusted Mesh V1 | 12.75 | 23.36 | 9.77 | 16.17 | 50.70 |
| Protected Mesh | 21.82 | 32.22 | 16.11 | 27.50 | 33.67 |
| Oban | 172.65 | 232.74 | 225.70 | 229.16 | 236.02 |
| River | 194.37 | 242.60 | 2.26 | 241.20 | 248.69 |
| Apalis | 9.31 | 16.09 | 5.62 | 12.75 | 18.69 |

Each percentile column shows the median of the per-rotation percentiles. With 24 jobs, the matrix’s nearest-rank p99 is the slowest job in that arm. The final column is the maximum over all verified rotations.

| Rotation | Trusted Mesh submit to settle p99 | Protected Mesh | Oban | River | Apalis |
|---|---:|---:|---:|---:|---:|
| 1 | 18.95 | 28.89 | 226.02 | 242.60 | 18.69 |
| 2 | 23.36 | 32.97 | 232.74 | 246.45 | 16.46 |
| 3 | 25.10 | 31.48 | 231.88 | 242.17 | 14.50 |
| 4 | 19.53 | 32.22 | 233.00 | 248.69 | 16.09 |
| 5 | 50.70 | 33.67 | 236.02 | 239.91 | 13.86 |

[Time spent between submissions](sparse-windows.md) · [Arm identities, orders, and source digests](scores.json) · [All workload scores](four-engine-scorecard.md)
