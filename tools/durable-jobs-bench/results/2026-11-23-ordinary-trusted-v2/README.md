# Ordinary trusted V2 short replay

This packet archives the frozen November 23, 2026 experiment and its
native qualification. Both ordinary lanes used the same AMD64 runner ELF
`cbd23c92eba11f9237c11ae6a5fc60128e1852956f4be21913f8e7f716b25a5f`.
The application still commits two independent writes per job. The success
cohort lane uses set-based admission and can group up to eight successful
terminal rows in one transaction inside the trusted shared process contract.
The protected Mesh control still runs fresh workers with fenced effects and
one terminal transaction per job. [Exact build and peer pins](evidence/ordinary-trusted-v2-short/series.json)

Five matched two-CPU blocks completed, with 128 jobs per arm, PostgreSQL
17.11, and verified TLS. The packet includes all 30 measured arms.

| Lane | Jobs/s from submit, median [range] | Jobs/s from first commit | Runner CPU (ms/job) |
| --- | ---: | ---: | ---: |
| Ordinary V2, success cohort | 502.74 [402.47–527.39] | 691.00 | 1.32 |
| Ordinary V2, stock queue policy | 437.54 [419.96–476.75] | 585.93 | 1.27 |
| Protected Mesh | 199.68 [192.55–212.15] | 223.55 | 4.83 |
| Oban | 298.04 [268.32–337.23] | 335.35 | 11.23 |
| River | 715.12 [475.17–724.10] | 772.55 | 0.92 |
| Apalis | 925.66 [885.73–944.09] | 999.87 | 0.61 |

The success cohort finished ahead of same-image stock in three blocks
and Oban in five. River and Apalis finished ahead of the cohort in all
five blocks. Every arm kept two independent application commits per job.
The ordinary cohort may share terminal XIDs across eligible successes.
[Five-block raw hashes and results](evidence/ordinary-trusted-v2-short/summary.json) · [Original first-block checkpoint](evidence/ordinary-trusted-v2-short/summary-partial.json)

Run the driver from the checkout root with `PYTHONPATH` pointing at the
benchmark suite. It replays the five selected blocks offline.

```sh
suite=tools/durable-jobs-bench
archive="$suite/results/2026-11-23-ordinary-trusted-v2"
PYTHONPATH="$suite" python3 -B "$archive/replay/interleave.py" summarize ordinary-trusted-v2-short
(cd "$archive" && shasum -a 256 -c SHA256SUMS)
```
