# Native Jobs reversed controls

The two initial controls ran the same four release artifacts, frozen inputs, and PostgreSQL 17.11 settings on CPUs 0 and 1 of the dedicated x86-64 Linode. Control 1 ran Mesh, Oban, River, then Apalis. Control 2 ran the reverse. The controller also rotated the profile order and retained 24 full unmeasured warmup arms for no-op, slow, and large.

All 40 control arms passed. They reconciled 14,016 completed jobs and 28,032 ordered application commits. The largest CPU steal observed in the submit windows was 0.00%. The largest I/O wait was 1.54%. The raw archives retain the CPU and disk samples that bracket each window. [Validated block index](index.json), [release and wrapper identities](inventory.json)

Cells show submit-to-durable-settlement jobs/s for control 1 and control 2. The Rust short-profile spread warrants checking all five baseline rotations before interpreting close comparisons.

| Profile | Mesh | Oban | River | Apalis |
| --- | ---: | ---: | ---: | ---: |
| No-op | 174.09 / 172.50 | 591.64 / 618.38 | 820.79 / 859.11 | 1,119.56 / 1,142.27 |
| Slow | 165.91 / 160.71 | 245.73 / 234.92 | 288.30 / 252.40 | 345.39 / 341.79 |
| Sparse | 2.65 / 2.66 | 2.03 / 2.03 | 2.08 / 2.08 | 2.50 / 2.50 |
| Short | 198.71 / 202.17 | 311.35 / 307.51 | 732.34 / 708.74 | 648.10 / 892.20 |
| Large | 22.94 / 23.96 | 23.59 / 24.26 | 19.75 / 19.84 | 39.22 / 39.53 |

```sh
python3 -B tools/durable-jobs-bench/run.py summarize --series 2026-11-23-controls
```
