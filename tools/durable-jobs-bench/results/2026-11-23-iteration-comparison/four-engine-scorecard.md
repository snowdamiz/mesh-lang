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

## Short jobs with set-based claims and grouped successes

5/5 matched rotations. The candidate uses set-based admission and can settle up to eight successful jobs in one transaction. The ordinary control uses stock admission and one terminal transaction per job.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh candidate | 502.74 | 691.00 | — | — |
| Trusted Mesh control | 437.54 | 585.93 | 3/5 | 3/5 |
| Protected Mesh | 199.68 | 223.55 | 5/5 | 5/5 |
| Oban | 298.04 | 335.35 | 5/5 | 5/5 |
| River | 715.12 | 772.55 | 0/5 | 2/5 |
| Apalis | 925.66 | 999.87 | 0/5 | 0/5 |

## Slow jobs with set-based enqueue

5/5 matched rotations. The first two lanes use the same executable with set-based enqueue enabled or disabled. The original V1 trusted runner and its protected control use their standard producer.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh · batch enqueue | 261.68 | 356.65 | — | — |
| Same binary · scalar enqueue | 250.39 | 366.52 | 5/5 | 1/5 |
| Original trusted Mesh | 251.24 | 365.48 | 5/5 | 0/5 |
| Protected Mesh | 164.77 | 209.05 | 5/5 | 5/5 |
| Oban | 247.27 | 331.48 | 4/5 | 5/5 |
| River | 288.66 | 387.57 | 1/5 | 1/5 |
| Apalis | 355.45 | 457.84 | 0/5 | 1/5 |

## Short jobs with both producer and runner changes

5/5 matched rotations. Both Mesh lanes use the V2 grouped-success policy. Only the producer's set-based enqueue switch changes.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh · batch enqueue | 596.54 | 714.01 | — | — |
| Same binary · scalar enqueue | 498.16 | 700.18 | 5/5 | 4/5 |
| Oban | 308.13 | 353.14 | 5/5 | 5/5 |
| River | 697.01 | 754.41 | 2/5 | 2/5 |
| Apalis | 742.12 | 790.22 | 0/5 | 1/5 |

Dense, short, slow and sparse use each engine's standard producer. Large uses the direct-file Mesh producer with both Mesh modes and PostgreSQL EXTERNAL argument storage on every engine. The sparse workload includes the prescribed gaps between arrivals. Every completed job has two independently committed application writes.

Protected Mesh executes a fresh process and fences its two application writes. Trusted Mesh runs the handler in the application's process with its database credentials. The five-workload comparison uses the V1 binaries. The short grouped-success comparison uses its own frozen V2 inventory. [Per-rotation chart](chart.html) · [Data, binary identities and raw digests](scores.json) · [Slow-job slot use](slow-slots.md) · [How each engine groups durable completions](../2026-11-23-native-completion-batches/README.md) · [Original four-engine baseline](../2026-11-23-baseline/README.md)
