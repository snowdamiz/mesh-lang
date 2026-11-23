# Protected Mesh with EXTERNAL payload storage

Five crossover blocks ran on November 23, 2026. Each lane ran a separate
warmup and measured arm with 64 jobs of 512 KiB each. Workers and PostgreSQL
17.11 shared CPUs 0 and 1 on the same AMD64 Linode. Each measured arm reconciled
64 terminal jobs and 128 independently committed application writes. Mesh
kept its fresh-worker and fenced-effect execution contract. All peers in
this series used their stock storage policies.

| Engine and configuration | Jobs/s from submit, median [range] | Jobs/s from first commit | Submit-to-settle p99 (ms) | Runner CPU (ms/job) | Pinned CPU (ms/job) | Observed arm (s) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mesh, stock broker and storage | 23.83 [23.33–24.64] | 77.08 | 2,686 | 25.80 | 64.38 | 10.68 |
| Mesh, stock broker and EXTERNAL | 33.73 [33.01–35.04] | 77.19 | 1,898 | 23.92 | 49.69 | 10.05 |
| Mesh, direct-file broker and EXTERNAL | 40.13 [39.33–42.10] | 77.44 | 1,595 | 23.67 | 42.97 | 9.88 |
| Oban | 24.99 [23.30–25.46] | 119.10 | 2,561 | 148.83 | 57.81 | 16.62 |
| River | 20.49 [19.70–20.87] | 70.74 | 3,123 | 15.06 | 69.22 | 9.53 |
| Apalis | 39.17 [37.46–39.95] | 345.97 | 1,634 | 5.96 | 38.59 | 9.04 |

All columns report five-block medians. Pinned CPU includes kernel and user
time on both CPUs in the samples bracketing submit and final settlement,
including PostgreSQL. Runner CPU covers the runner process tree over its
lifetime. Observed arm wall time includes initialization and checks, including
the EXTERNAL DDL before submit. The median DDL call took 18.81 ms with the
stock broker and 19.89 ms with the direct-file broker.

The direct-file EXTERNAL lane improved median submitted throughput by 68%
over stock Mesh. Median time from first submit to first committed enqueue
fell from 1,863 ms to 770 ms. Drain rates after the first commit stayed within
1% across the Mesh lanes. The storage replay verified 64 matching hashes,
zero compressed datums, and 33,559,808 stored datum bytes in each of the 20
tuned arms. The comparison with the same storage policy on every engine and
the compressible-input tradeoff are in separate records. [Symmetric timing](../2026-11-23-payload-storage-symmetric/README.md) · [Compressible payload control](../2026-11-23-large-storage-component/README.md)

The exact execution pins are in `series.json`. All three Mesh lanes ran ELF
`3dc20c730bd269da86c9c98f225d5f85bdc0dd3d3a31908e0eac90aa50d8dfd2`.
The direct-file broker ELF was
`e4bab43e5e22662d6ecd4a4d32aaf554978779c1d1f0c88f72284bc99f06e419`.
The peer executables are the same ones listed in the symmetric series. [Frozen pins](series.json)

Replay from the checkout root with the saved raw archives.

```sh
suite=tools/durable-jobs-bench
series=2026-11-23-payload-storage-mesh
python3 -B "$suite/interleave.py" summarize "$series"
python3 -B "$suite/native/payload-storage/verify-series.py" "$series"
(cd "$suite/results/$series" && shasum -a 256 -c SHA256SUMS)
```

[Complete comparison](comparison.csv) · [Raw archive pins and selected blocks](summary.json) · [Storage replay](storage-replay.json)
