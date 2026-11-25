# Public LARGE progress with symmetric EXTERNAL storage

The public Mesh FILE producer settled 64 × 512 KiB jobs sooner than River and Oban in all five matched Linode rotations. All lanes used PostgreSQL 17.11, EXTERNAL payload storage and two CPUs shared by PostgreSQL and the workload. The stock public inline comparator, same-broker inline duplicates, and three stock peers ran alongside FILE. The [controller replay](../2026-11-25-public-large-file-external/grant-parser-retry/README.md) covers all 70 warmup and measured arms.

| Peer | Median FILE / peer submit-to-settlement throughput, paired | FILE wins | Rotations with ≥20% FILE throughput advantage |
|---|---:|---:|---:|
| River | 1.752× | 5/5 | 5/5 |
| Oban | 1.203× | 5/5 | 3/5 |
| Apalis | 0.623× | 0/5 | 0/5 |

Median full-submit throughputs were 46.96 jobs/s for Mesh, 27.56 for River, 38.48 for Oban and 73.89 for Apalis. Ratios in the table use matched rotations. The 20% column is descriptive.

## Where the differences accrued

Elapsed times below are paired median Mesh minus peer values in milliseconds. Negative values mean Mesh took less time. Each arm starts at its own submission, and the application milestones count durable second writes in completion order. Intervals partition each individual run exactly. Medians across runs need not add to the median total.

| Peer | Submit to durable enqueue | Commit to eighth second write | Eighth through 56th second write | 56th second write to last settlement |
|---|---:|---:|---:|---:|
| River | −751 | −189 | +163 | −159 |
| Oban | −440 | +37 | +101 | +33 |
| Apalis | +53 | +34 | +403 | +42 |

Mesh's enqueue advantage carried all five full-submit wins over Oban. Oban completed its post-commit drain sooner in all five, by a paired median 163 ms. The middle 48 writes took Mesh 326–573 ms longer than Apalis across the five pairs. That interval includes both admission and handler execution. The ledger alone cannot allocate the difference between them.

Against the faster same-broker inline control, FILE saved a median 293 ms to enqueue in five wins and 320 ms through settlement in four wins. The remaining full-submit pair lost 13 ms. The file-path transport was exercised by the benchmark broker; these measurements apply to that producer and deployment.

Run `python3 -B replay.py | cmp - native.json` here to verify the 20 measured FILE and peer archive hashes and reproduce [all intervals](native.json). For the controls and storage checks, run `python3 -B ../2026-11-25-public-large-file-external/grant-parser-retry/retry.py replay`.
