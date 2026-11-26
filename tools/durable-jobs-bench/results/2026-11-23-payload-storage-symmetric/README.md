# EXTERNAL payload storage on all four engines

Five crossover blocks ran on November 23, 2026. Each arm ran the same
64 × 512 KiB workload on CPUs 0 and 1 shared by the runner and PostgreSQL
17.11. Each lane had a separate warmup. All lanes reconciled 64 terminal jobs
and 128 independently committed application writes, with EXTERNAL storage
verified before submission and byte-exact uncompressed payloads verified
after terminal settlement. Mesh kept fresh process workers and fenced effects.

| Engine and configuration | Jobs/s from submit, median [range] | Jobs/s from first commit | Submit-to-settle p99 (ms) | Runner CPU (ms/job) | Pinned CPU (ms/job) | Observed arm (s) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mesh, stock broker | 33.03 [32.78–36.31] | 73.13 | 1,938 | 24.18 | 50.47 | 10.06 |
| Mesh, direct-file broker | 37.34 [33.84–40.12] | 75.35 | 1,714 | 24.65 | 44.69 | 10.08 |
| Oban | 36.88 [34.69–37.15] | 116.50 | 1,735 | 146.47 | 40.94 | 15.95 |
| River | 27.11 [24.93–27.52] | 69.31 | 2,361 | 14.89 | 55.00 | 9.04 |
| Apalis | 73.68 [2.39–76.13] | 333.52 | 869 | 5.81 | 21.09 | 8.58 |

Direct-file Mesh finished ahead of River in five blocks and Oban in four.
Apalis finished ahead of Mesh in four blocks. One Apalis block settled a job
late and took nearly 27 seconds from submit to the last settlement. The
ledger reconciles and the block remains in the range and median.

The CPU and wall-time scopes match the companion series. The per-engine
DDL ran before first submit, with a median of 19.71 ms for direct-file Mesh,
18.94 ms for stock-broker Mesh, 19.07 ms for Oban, 0.16 ms for River, and
18.99 ms for Apalis. River applies the setting inside its migration.
Observed arm time includes initialization and teardown. [Mesh-only EXTERNAL timing and replay commands](../2026-11-23-payload-storage-mesh/README.md)

The setup changed the storage strategy of the payload column on an empty
database. All engines used the stock peer executables from the following
pins. The full release and wrapper file hashes are in the frozen series.

| Executable | SHA-256 |
| --- | --- |
| Mesh worker ELF | `3dc20c730bd269da86c9c98f225d5f85bdc0dd3d3a31908e0eac90aa50d8dfd2` |
| Mesh direct-file broker ELF | `e4bab43e5e22662d6ecd4a4d32aaf554978779c1d1f0c88f72284bc99f06e419` |
| Oban BEAM VM | `921aeaddb61ce696db24621eaece52220b16993d3fe591920a271bf3dd720206` |
| River benchmark ELF | `5db25a75ccfbf992cb72479a86470baa3a1ac054b5d780e0854e41caaaadf4d0` |
| Apalis benchmark ELF | `b3c2fb2b472d7c8b2706e13c38281a46958b669f9d4cf22ca252636d083e7e66` |

[Frozen pins](series.json) · [Complete comparison](comparison.csv) · [Raw archive pins and selected blocks](summary.json) · [Storage replay](storage-replay.json)
