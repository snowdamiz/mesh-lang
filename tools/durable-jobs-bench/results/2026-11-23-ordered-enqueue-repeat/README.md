# Native ordered-enqueue repeat

**Hold `77c447f7`.** Ordered enqueue sped up the dense producer commit in
all five direct pairs. Dense submit-to-settlement throughput was nearly
flat. On short, submit-to-settlement throughput fell in four of five
pairs and runner CPU rose in all five. The product patch stays isolated.

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

## Alternating Mesh pairs

Five trace-off pairs per profile passed with identical stock and candidate
release artifacts throughout. The 20 measured arms reconciled 11,520 jobs
and 23,040 ordered application commits. The ten dense warmups also passed.
Positive changes in throughput mean more jobs per second. Positive changes
in latency or CPU mean more time or CPU used. [Raw pair directories](focused/pairs) ·
[Per-pair values and ranges](paired-comparison.json)

| Metric | Dense median paired change [min, max] | Short median paired change [min, max] |
| --- | ---: | ---: |
| Submit to durable producer commit | −15.57% [−18.14%, −2.96%] | +15.69% [−16.01%, +22.72%] |
| Submit to producer acknowledgement | −13.23% [−20.21%, −4.76%] | +16.37% [−14.04%, +22.80%] |
| Submit-to-settlement throughput | −0.22% [−1.45%, +6.08%] | −9.85% [−18.35%, +3.21%] |
| Commit-to-settlement throughput | −1.01% [−3.17%, +4.61%] | −8.66% [−21.50%, +1.17%] |
| Submit-to-pickup p99 | +0.24% [−5.87%, +1.33%] | +10.01% [−3.16%, +21.09%] |
| Commit-to-pickup p99 | +1.35% [−4.54%, +3.14%] | +8.43% [−1.18%, +25.91%] |
| Runner CPU per job | +0.19% [−3.65%, +2.94%] | +2.83% [+0.19%, +17.12%] |

Dense median durable producer commit fell from 567.08 ms to 482.65 ms.
The separate producer-only diagnostics show where the time was saved.
The candidate cut dense client TLS records from 1,053 to 237 while keeping
the same SQL call count and reconciling all durable receipts and XIDs.
That isolated win was small relative to more than five seconds of dense
postcommit drain. [Producer diagnostics](../2026-11-23-ordered-enqueue-qualification/NATIVE.md)

Short median submit throughput fell from 198.27 to 184.74 jobs/s. Short
pair 3 had 0.46% candidate steal and higher I/O wait. Pairs 1 and 5 each
had zero steal and stock/candidate I/O wait within 0.04 percentage points.
They still lost 9.85% and 10.76% submit throughput respectively. These
measurements establish the regression for this candidate and workload.
They leave its cause unresolved. [Host counters and both clocks for each pair](paired-comparison.json)

The native truncated-response unit-test attempts ended during compilation.
The isolated Darwin test and the separate native executable's SQL-error,
disconnect and rollback probes passed. A further wire-proof-only run
would leave the measured short regression in place, so this candidate
received no additional reservation. [Qualification and wire receipts](../2026-11-23-ordered-enqueue-qualification/NATIVE.md)

```sh
bench=tools/durable-jobs-bench
series=2026-11-23-ordered-enqueue-repeat
python3 -B "$bench/run.py" summarize --series "$series"
python3 -B "$bench/results/$series/repeat_evidence.py" "$series"
python3 -B "$bench/results/$series/focused_pairs.py" summarize "$series"
```
