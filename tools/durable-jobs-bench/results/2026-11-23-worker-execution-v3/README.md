# Trusted persistent-worker native comparison

One matched rotation on November 23, 2026 used the same release artifact
from `67ec9bb9ba405f5f2cb830a338f77a8eed76d897` for both Mesh arms.
The persistent arm used the supervised reuse contract. The strict arm
used fresh process isolation per attempt. PostgreSQL and workers shared
CPUs 0 and 1. The benchmark used the existing stock peer executables,
two independently committed application writes, and the native
terminal commit timestamp. The ledger reconciled both frozen profiles.
[Exact config and pins](series.json), [per-arm scores and hashes](summary-partial.json)

| Profile | Metric | Strict | Trusted persistent | Oban | River | Apalis |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Dense no-op | Submit jobs/s | 170.35 | 239.00 | 655.88 | 849.15 | 1,137.33 |
| Dense no-op | Commit jobs/s | 189.39 | 277.82 | 726.54 | 904.05 | 1,185.86 |
| Dense no-op | Runner CPU ms/job | 5.81 | 2.89 | 9.81 | 0.77 | 0.58 |
| Dense no-op | Peak runner RSS MiB | 138.00 | 475.88 | 761.68 | 123.10 | 66.63 |
| Short | Submit jobs/s | 190.73 | 245.78 | 298.10 | 717.56 | 752.07 |
| Short | Commit jobs/s | 221.19 | 298.42 | 339.23 | 779.80 | 802.44 |
| Short | Runner CPU ms/job | 4.78 | 2.69 | 11.47 | 1.02 | 0.72 |
| Short | Peak runner RSS MiB | 21.75 | 110.63 | 99.52 | 16.76 | 8.63 |

The first SETUP exited during PostgreSQL startup. The generated Unix
socket path exceeded the Linux path limit. A build-free retry shortened
that path and reran the same compiled binaries. It passed the hard and
confirmed lease deadlines with the supervisor suspended, stale generation
messages on the same PID, pre-entry expiry, process lifecycle, and
post-commit receipt recovery. Both retry smokes reconciled 1,024 jobs
and 2,048 independent application commits. The strict smoke used 1,024
distinct PIDs. The persistent smoke used 64.

The results cover one rotation. Increased throughput and lower runner
CPU came with higher peak RSS than strict Mesh. The remaining native
slots went to the other active comparisons.
