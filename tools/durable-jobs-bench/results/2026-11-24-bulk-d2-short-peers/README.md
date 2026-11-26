# Native short bulk+D2 versus stock peers

Five matched 128-job blocks ran on the original AMD64 Linode on November 24,
2026 UTC. Workers and PostgreSQL 17.11 shared CPUs 0 and 1. Each arm used a
fresh database. The frozen schedule contains three distinct launch orders
across the five blocks. [Inventory](short.json) · [Controller request](request.json) ·
[Release and hashes](evidence/2026-11-24-bulk-d2-short-peers/SHA256SUMS)

| Lane | Median submit jobs/s | Median drain jobs/s | Submit-to-settlement p99 (ms) | Peak connections | Bulk+D2 submit / drain wins |
| --- | ---: | ---: | ---: | ---: | ---: |
| Bulk+D2 | 634.90 | 763.67 | 201.61 | 13 | — |
| Bulk V2 | 600.04 | 716.20 | 212.78 | 13 | 3/5 · 4/5 |
| Protected Mesh | 208.39 | 234.67 | 609.97 | 13 | 5/5 · 5/5 |
| Oban | 309.54 | 352.13 | 413.50 | 19 | 5/5 · 5/5 |
| River | 473.74 | 504.50 | 270.19 | 18 | 5/5 · 5/5 |
| Apalis | 894.70 | 976.44 | 143.06 | 19 | 0/5 · 0/5 |

The two bulk lanes each did one set-based enqueue and 256 independent
application writes per arm under the grouped-success V2 contract. Checkout
probes fell from 256 to 2 in every pair. The median paired changes versus
bulk were +5.81% submit throughput, +6.63% drain throughput, −5.25% submit
p99, and −7.87% runner CPU per job. Median full-arm wall times were 5.98 s
for bulk+D2 and 5.89 s for bulk. Terminal XID medians were 120 and 117.
River used the same pinned inventory as the earlier three-lane series,
where its median was 712.77 submit and 769.02 drain jobs/s. The win counts
above use this series' matched controls. [Raw replay and distributions](evidence/2026-11-24-bulk-d2-short-peers/summary.json) ·
[Earlier three-lane series](../2026-11-23-ordinary-hot-checkout-bulk-v2/README.md)

| Deployment | Executable SHA-256 |
| --- | --- |
| Bulk+D2 runner | `c6960a0d63af7e94a9b436a40233b7178a4e6c2cf5af95095fc52625ed039840` |
| Bulk V2 runner and shared producer/migrator | `55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a` |
| Protected Mesh | `3dc20c730bd269da86c9c98f225d5f85bdc0dd3d3a31908e0eac90aa50d8dfd2` |
| Oban release executable | `3c6e0761a7980beb1b4eb8b7739afd34959e9ba3ee964b61a05fb6ac59b20c8e` |
| River | `5db25a75ccfbf992cb72479a86470baa3a1ac054b5d780e0854e41caaaadf4d0` |
| Apalis | `b3c2fb2b472d7c8b2706e13c38281a46958b669f9d4cf22ca252636d083e7e66` |

Replay the archived arms with the original scorer from the checkout root.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-24-bulk-d2-short-peers/verify.py
```
