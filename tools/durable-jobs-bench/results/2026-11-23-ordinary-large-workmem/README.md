# Ordinary V1 large work_mem freeze

This packet preserves the build-free qualification request and the exact
PostgreSQL configuration overlay. In the planned matched series, ordinary
Mesh, Oban, River, and Apalis all use EXTERNAL payload storage and 16 MiB
`work_mem`. Two ordinary Mesh controls use 4 MiB. Every lane uses the same
64 × 512 KiB workload and shares CPUs 0 and 1 with PostgreSQL 17.11. The
runner and broker binaries match the completed ordinary large series.
[Executable provenance](../2026-11-23-ordinary-trusted-v1-large/README.md)

Qualification checks the effective PostgreSQL settings, input payloads,
ordered independent app commits, terminal rows, TLS, and live connection
caps. The receipt records claim spill blocks, temp-file activity by role,
and sampled PostgreSQL RSS and PSS. Native timing is conditional on the
sealed six-arm qualification and observed removal of the 4 MiB claim
spill at 16 MiB. [Exact request](request.json) · [Freeze manifest](freeze-02/freeze.json)

## Native results

Six diagnostic arms passed before timing. Both 4 MiB ordinary controls
spilled in the claim statement, with 6,156 and 8,208 temp blocks read and
written in aggregate. The 16 MiB ordinary arm recorded zero. The five
matched blocks then reconciled all 60 warmup and measured arms on
November 23, 2026. [Qualification](evidence/ordinary-large-workmem-f61ea70d9c52/setup-status.json) · [Measured raw hashes](evidence/ordinary-large-workmem-matched/summary.json)

| Lane | Jobs/s from submit, median [range] | Jobs/s from first commit | Submit-to-settle p99 (ms) | Pinned CPU (ms/job) | Sampled PostgreSQL PSS (MiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Ordinary, 16 MiB | 47.54 [45.96–48.42] | 105.85 | 1,346 | 34.22 | 207.04 |
| Ordinary control A, 4 MiB | 47.26 [44.28–48.03] | 101.40 | 1,354 | 37.81 | 211.32 |
| Ordinary control B, 4 MiB | 43.67 [41.06–46.10] | 101.65 | 1,466 | 38.12 | 206.04 |
| Oban, 16 MiB | 36.89 [35.43–37.56] | 116.70 | 1,735 | 46.09 | 377.93 |
| River, 16 MiB | 27.33 [25.61–28.01] | 72.61 | 2,342 | 55.62 | 278.16 |
| Apalis, 16 MiB | 75.52 [72.32–77.87] | 321.68 | 847 | 23.12 | 283.73 |

The 16 MiB setting eliminated claim temp reads and writes in every
measured ordinary arm. The 4 MiB controls recorded 2,052–4,104 blocks in
each direction. Pinned CPU savings relative to the mean duplicate controls
were a paired median of 3.28 ms/job [−0.39–4.22]. Median paired submit
throughput gained 2.86 jobs/s relative to the duplicate mean. The largest
absolute duplicate spread was 6.63 jobs/s. The runner and PostgreSQL share
the pinned CPU window. PSS is the median of sampled per-arm peaks inside
that window, with shared pages apportioned by process. [PostgreSQL counters by arm](evidence/ordinary-large-workmem-matched/summary.json)

To replay from a restored checkout, place the archived work_mem packet at
`target/ordinary-large-workmem` and the [ordinary large packet](../2026-11-23-ordinary-trusted-v1-large/README.md)
at `target/ordinary-trusted-large-assembly` if their original pinned paths
are absent. The freeze verifies both packets' original driver hashes.

```sh
lane=target/ordinary-large-workmem
python3 -B "$lane/series.py" summarize "$lane/freeze-02" \
  19a93a1cb12f5404d1abe1f7d94dfb50e84e8c9764cb6fefa8d84353436b595e
```
