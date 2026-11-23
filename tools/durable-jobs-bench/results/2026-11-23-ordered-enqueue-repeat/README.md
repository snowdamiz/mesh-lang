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
four-engine groups reuse the baseline’s paired control orders. Five
alternating stock/candidate Mesh pairs per profile supply the direct
comparison. Both use the existing matrix scorer and the original frozen
workload. Dense has one warmup per arm. Short follows the original plan
without a warmup.
