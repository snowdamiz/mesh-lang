# Alternating dense Jobs repeat

The frozen baseline and due-query candidate ran five alternating pairs on CPUs 0 and 1 of the same Linode. Each pair used fresh on-disk PostgreSQL 17.11 databases over verified TLS and one full unmeasured warmup per build. The 10 measured arms reconciled 10,240 jobs and 20,480 ordered application commits.

| Metric | Baseline median [range] | Candidate median [range] | Median paired change [range] |
| --- | ---: | ---: | ---: |
| Submit to settlement, jobs/s | 172.58 [166.21–174.01] | 175.40 [173.51–177.35] | +1.92% [+0.53–+4.99%] |
| Commit to settlement, jobs/s | 190.69 [182.98–192.23] | 193.87 [191.87–197.21] | +2.59% [+0.07–+5.67%] |
| Commit-to-pickup p99, ms | 5,300.70 [5,261.11–5,541.79] | 5,210.09 [5,115.76–5,249.66] | −2.76% [−5.54–−0.29%] |
| Runner CPU, ms/job | 5.81 [5.77–5.98] | 5.75 [5.66–5.78] | −2.48% [−4.11–+0.14%] |

The candidate was faster in both throughput windows in every pair. The absolute throughput ranges overlap, and the observed gain leaves the dense gap against Oban and River open. The four-engine repeat did not establish a broad speedup across the other profiles. [Per-pair values](trial-1795457586447042000/summary.json), [four-engine repeat](../2026-11-23-due-range-repeat/README.md)

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-due-range-focused/run-focus.py
```
