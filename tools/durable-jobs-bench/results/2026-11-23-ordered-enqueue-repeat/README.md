# Native ordered-enqueue repeat

This series compares the sealed ordered-enqueue candidate `77c447f7` with
the original Mesh baseline `3f69b150` on the same Linode boot, disk,
PostgreSQL 17.11 and two pinned CPUs. Oban, River and Apalis reuse the
exact baseline releases and wrappers. The controlled baseline is
`069c9c68` with 25 validated blocks. The scheduled-work index experiment
uses a separate artifact and results. [Baseline](../2026-11-23-baseline/README.md) ·
[Ordered-enqueue qualification](../2026-11-23-ordered-enqueue-qualification/README.md) ·
[Scheduled-work repeat](../2026-11-23-due-range-focused/README.md)

The repeated profiles are dense no-op and short (16 slots). Their 1 KiB
payloads exercise batching. Native large and sparse producer diagnostics
and all five semantic smokes cover the scalar paths. The five rotated
four-engine groups use the original plan and verified reversed controls. Five
alternating stock/candidate Mesh pairs per profile supply the direct
comparison. Both use the existing matrix scorer and the original frozen
workload. Dense has one warmup per arm. Short follows the original plan
without a warmup.

## Four-engine repeat

All ten blocks passed, with 40 measured arms and 20 dense warmups. The
measured arms reconciled 23,040 jobs and 46,080 ordered application commits.
Observed CPU steal was zero in all measured arms. The highest I/O wait
was 2.47%. The local checkout and the sealed Linode artifact have separate
source identities. [Block index](index.json) · [All engine metrics](comparison.csv)

| Mesh metric | Dense no-op, stock → ordered | Short, stock → ordered |
| --- | ---: | ---: |
| Median durable producer commit, ms | 573.05 → 490.01 | 68.67 → 63.27 |
| Median submit-to-settlement, jobs/s | 170.90 → 174.07 | 198.92 → 206.70 |
| Median commit-to-settlement, jobs/s | 190.34 → 191.32 | 224.91 → 229.90 |
| Median commit-to-pickup p99, ms | 5,310.72 → 5,303.97 | 538.43 → 541.92 |
| Median runner CPU, ms/job | 5.81 → 5.73 | 4.69 → 4.75 |

The two matrices ran at different times. Median matched-rotation
submit throughput changed by +0.81% dense and +0.22% short, with mixed
directions. Commit throughput changed by −0.87% dense and −0.05% short.
The unchanged peers also varied between the two series, especially on
short. [Per-group ratios, both timing windows and peer controls](group-comparison.json)
