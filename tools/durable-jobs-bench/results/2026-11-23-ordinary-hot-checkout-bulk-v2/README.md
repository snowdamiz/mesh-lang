# Native bulk V2 with a hot-checkout runner

On November 23–24, 2026 UTC, the retained bulk V2 producer and migrator
ran with the D2 runner in five matched short rotations. The new runner
ELF SHA-256 is `c6960a0d63af7e94a9b436a40233b7178a4e6c2cf5af95095fc52625ed039840`.
The bulk producer ELF is
`55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a`.
River used `5db25a75ccfbf992cb72479a86470baa3a1ac054b5d780e0854e41caaaadf4d0`.
PostgreSQL 17.11 and all workloads shared CPUs 0 and 1. The two Mesh
lanes retained grouped success, 256 independent application writes,
one bulk producer call and zero scalar calls per 128-job arm.
[Summary](native-summary.json) · [Controller evidence](controller/)

| Lane | Submit jobs/s | Drain jobs/s | Submit-to-settlement p99 (ms) | Pinned CPU (ms/job) | Full arm wall (s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Bulk + D2 | 650.47 | 788.43 | 196.23 | 3.125 | 6.106 |
| Bulk | 552.38 | 646.56 | 228.29 | 3.750 | 5.829 |
| River | 712.77 | 769.02 | 179.58 | 3.516 | 5.337 |

Bulk + D2 beat bulk in four of five groups on both clocks. The median
paired throughput gains against bulk were 18.33% from submit and 23.39%
from first enqueue commit. It beat River in two submit windows and
four drain windows. D2 sent two checkout probes against 256 in bulk
in every group. The full-arm time includes the observed setup and
teardown outside the workload window. [Measured distributions](native-summary.json)

The SETUP wrapper trimmed only whitespace around the PostgreSQL device
field before checking the sealed host ID. It retained the original source
transfer, existing peer pins and both native smokes. The standalone D2
comparison is available separately. [D2-only comparison](../2026-11-23-ordinary-hot-checkout/README.md)

Run the offline verification from the repository. It restores the frozen
inputs into temporary checkout-relative paths and replays the raw native
arms with the original scorer.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-ordinary-hot-checkout-bulk-v2/replay/verify.py
```
