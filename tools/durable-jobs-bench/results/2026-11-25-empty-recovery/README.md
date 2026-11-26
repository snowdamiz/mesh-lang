# Empty due-job recovery UPDATE on PostgreSQL 17.11

Four Linode traces identified work in the scheduled-job promotion UPDATE inside `mesh_jobs.recover`. Each run completed 1,024 dense jobs and 2,048 independently committed application writes. The promotion UPDATE changed zero rows throughout. Before submit it recorded two shared-buffer hits per call. From the pre-submit snapshot through the first post-settlement snapshot the same statement recorded the following totals. [Source archives and counter deltas](native-evidence.json)

| Runners | Group | Calls | Shared-buffer hits | Summed executor time (ms) |
|---:|---:|---:|---:|---:|
| 2 | 1 | 14 | 13,231 | 10.35 |
| 2 | 2 | 12 | 11,405 | 10.55 |
| 8 | 1 | 73 | 73,060 | 64.55 |
| 8 | 2 | 90 | 93,576 | 94.35 |

All four read zero blocks into shared buffers for this statement during the observation window. Hits count repeated accesses to cached pages. Executor elapsed time can overlap across runners and includes waiting. At two runners the measured statement accounted for about 10 ms in total. A material improvement in full drain remains an open question, especially with a larger retained job history.

[Candidate SQL](candidate.sql) adds `AND EXISTS (SELECT 1 FROM due)` to the UPDATE. The existence test references the same ordered, limited, row-locking CTE as the join. PostgreSQL 17.11 placed a one-time filter above the target scan in an untimed generic plan. Native execution against populated tables is needed to measure the visits avoided and the cost when jobs are due. Only the experiment installs this function.

The [correctness proof](correctness.json) compares the full frozen function against stock, including the true-batch layout guard, two sessions contending for the first due job, ordered promotion of the next eligible row, queue and namespace filtering, rollback, expired terminal attempts and lease cleanup. An UPDATE statement trigger sees an empty transition table on an empty promotion and one row on a successful promotion. The first plan verifier included the ModifyTable root among scan nodes. The corrected verifier checks the actual target scans.

Run from the checkout root. The first command reads archived native data. The second uses a disposable, socket-only PostgreSQL 17.11 cluster and requires its bin directory. Both commands write to stdout.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-empty-recovery
python3 -B "$d/audit.py" | cmp - "$d/native-evidence.json"
python3 -B "$d/prove.py" /opt/homebrew/opt/postgresql@17/bin | cmp - "$d/correctness.json"
```
