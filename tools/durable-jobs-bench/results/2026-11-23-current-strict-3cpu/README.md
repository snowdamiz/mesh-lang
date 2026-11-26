# Strict native three-CPU sensitivity

On November 23, 2026, the first rotated dense and short blocks repeated
the strict current and historical Mesh comparison on CPUs 0, 1 and 2.
The adapters and PostgreSQL 17.11 shared that mask. Engine executables,
runner settings, and the frozen workloads match the two-CPU reference.
The native Mesh and Oban launchers explicitly validated the three-CPU
mask. Both blocks reconciled payloads, committed application effects,
and durable settlement. [Exact three-CPU pins](series.json) · [Two-CPU reference](../2026-11-23-current-strict-2cpu/README.md)

| Profile and engine | Submit jobs/s, 2 CPU | Submit jobs/s, 3 CPU | Drain jobs/s, 2 CPU | Drain jobs/s, 3 CPU |
| --- | ---: | ---: | ---: | ---: |
| Dense, current Mesh | 172.47 | 211.98 | 189.92 | 244.45 |
| Dense, historical Mesh | 172.00 | 213.75 | 189.73 | 245.93 |
| Dense, Oban | 583.55 | 749.83 | 650.27 | 845.58 |
| Dense, River | 954.04 | 1,019.81 | 1,023.06 | 1,116.07 |
| Short, current Mesh | 207.44 | 232.84 | 232.68 | 268.70 |
| Short, historical Mesh | 207.18 | 235.29 | 235.43 | 271.87 |
| Short, Oban | 277.16 | 324.78 | 312.75 | 374.17 |
| Short, River | 565.26 | 474.61 | 610.17 | 504.05 |

The caps were measured in separate rotations on the same boot, with
one valid block per profile per cap. Current Mesh used 11.17 → 12.77
pinned CPU-ms/job on dense and 8.20 → 9.45 on short. The CPU numerator
includes PostgreSQL. Dense current Mesh took 12.97 → 11.74 seconds across
the complete observed arm, and short took 6.19 → 6.38 seconds. Measured
CPU steal was zero. The published workload comparisons use two CPUs.
[Raw replay index](summary-partial.json) · [Resource metrics](comparison-partial.csv)
