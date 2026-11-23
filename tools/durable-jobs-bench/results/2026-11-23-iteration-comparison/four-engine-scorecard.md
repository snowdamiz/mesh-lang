# Four native queue engines on the same two CPUs

The five workloads below use the original trusted Mesh V1 runner and its protected Mesh control. Each completed rotation contains both Mesh modes and all three stock peers on the same Linode. Rates are median jobs per second. The last column counts the rotations in which trusted Mesh was faster than each peer.

## Submission through last settlement

| Workload | Rotations | Protected Mesh | Trusted Mesh | Oban | River | Apalis | Trusted wins vs Oban / River / Apalis |
|---|---:|---:|---:|---:|---:|---:|---|
| Dense no-op | 1/5 | 174.69 | 358.24 | 641.43 | 912.49 | 1,139.36 | 0 / 0 / 0 |
| Short | 5/5 | 200.58 | 442.10 | 280.80 | 719.93 | 843.41 | 4 / 1 / 0 |
| Slow | 5/5 | 163.20 | 243.52 | 245.29 | 286.93 | 344.19 | 1 / 0 / 0 |
| Sparse | 0/5 | — | — | — | — | — | — |
| Large · EXTERNAL | 5/5 | 41.04 | 48.63 | 37.19 | 27.54 | 73.98 | 5 / 5 / 0 |

## First enqueue commit through last settlement

| Workload | Rotations | Protected Mesh | Trusted Mesh | Oban | River | Apalis | Trusted wins vs Oban / River / Apalis |
|---|---:|---:|---:|---:|---:|---:|---|
| Dense no-op | 1/5 | 193.54 | 456.12 | 706.75 | 978.31 | 1,190.70 | 0 / 0 / 0 |
| Short | 5/5 | 224.74 | 593.21 | 319.70 | 782.92 | 918.83 | 5 / 2 / 0 |
| Slow | 5/5 | 206.13 | 364.41 | 321.21 | 391.57 | 445.15 | 5 / 1 / 0 |
| Sparse | 0/5 | — | — | — | — | — | — |
| Large · EXTERNAL | 5/5 | 76.80 | 106.61 | 116.07 | 70.62 | 331.07 | 1 / 5 / 0 |

Dense, short, slow and sparse use each engine's standard producer. Large uses the direct-file Mesh producer with both Mesh modes and PostgreSQL EXTERNAL argument storage on every engine. The sparse workload includes the prescribed gaps between arrivals. Every completed job has two independently committed application writes.

Protected Mesh executes a fresh process and fences its two application writes. Trusted Mesh runs the handler in the application's process with its database credentials. These scores belong to the captured V1 binaries. [Per-rotation chart](chart.html) · [Data, binary identities and raw digests](scores.json) · [Original four-engine baseline](../2026-11-23-baseline/README.md)
