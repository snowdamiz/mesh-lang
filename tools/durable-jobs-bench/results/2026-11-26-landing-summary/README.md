# Durable jobs: landing page figures

The landing page shows these medians. `build.py` reads each workload's recorded series and writes [comparison.csv](comparison.csv).

Each figure is the median time from the first enqueue submission to the last durable settlement. Every arm ran on the same Linode with CPUs 0 and 1 pinned and a fresh PostgreSQL 17.11 cluster that had verified TLS, fsync, full-page writes and synchronous commit enabled. Each job commits its attempt and effect writes independently. Lower is better.

| Workload | Shape | Mesh | Oban | River | Apalis | Rotations | Series |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| Short | 128 jobs, 16 slots, 1 ms handlers | 217 ms | 461 ms | 270 ms | 155 ms | 6 | [short16 River readiness](../2026-11-24-short16-river-readiness/README.md) |
| Slow | 512 jobs × 16 KiB, 2 × 32 slots, 100 ms handlers | 1,803 ms | 1,914 ms | 2,003 ms | 1,449 ms | 8 | [Slow at cap 71](../2026-11-24-ordinary-slow-equal-topology-cap71/README.md) |
| Large | 64 jobs × 512 KiB, 8 × 8 slots | 1,363 ms | 1,663 ms | 2,322 ms | 866 ms | 5 | [Large with EXTERNAL storage](../2026-11-25-public-large-file-external/README.md) |
| Dense | 1,024 jobs, 2 × 32 slots, no-op handlers | 1,003 ms | 1,322 ms | 1,021 ms | 740 ms | 5 | [Dense peer blocks](../2026-11-25-dense-fixed8-logical-factorial/PEER-RESULTS.md) |

Mesh finished ahead of Oban and stock River in every workload. Apalis finished first in all four. River was measured with its stock lazy connection pool. With its pool filled before the run, River finished Short in 145 ms and Dense in 779 ms in separate readiness series, ahead of Mesh.
