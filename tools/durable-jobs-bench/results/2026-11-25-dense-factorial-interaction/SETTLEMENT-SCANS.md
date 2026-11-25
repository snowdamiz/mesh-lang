# Grouped settlement on dense jobs

In the two released Linode diagnostic arms, the first lock lookup in `complete_batch_v1` examined 2,259 executing rows to settle 167 jobs. Each of the 62 sampled statements had both its start and end inside the drain. In every plan, PostgreSQL read the namespace from the `mesh_jobs_expiring` index, then matched individual jobs and lease tokens against the supplied batch. The subsequent UPDATE already used the primary key and returned one target row per settled job.

| Claim configuration during diagnostic | Sampled settlements | Jobs settled | Rows returned by the lock-side index scan | Shared hits at that scan | Rows returned by UPDATE-side lookup |
|---|---:|---:|---:|---:|---:|
| Stock | 32 | 81 | 980 | 531 | 81 |
| Logical key | 30 | 86 | 1,279 | 1,083 | 86 |

These plans mostly cover widths two and three. The six later fixed8 runs used larger batches. Their ledgers reconcile 159–226 grouped settlements and zero to seven scalar settlements per 1,024 jobs. PostgreSQL recorded 268–327 ms of cumulative statement execution for grouped settlement, including locks and updates across two sessions. Isolating the lock-side lookup requires a matched experiment on fixed8. The saved plans establish neither its cost on that workload nor a critical-path saving.

Across the six paired fixed8 blocks, tying prewarmed River on drain through settlement alone would require saving 0.64–2.81 ms per grouped call if every saved microsecond advanced final settlement and the call counts held. In the fifth block River led by 636 ms, almost twice the entire recorded 327 ms for those SQL calls. The distribution of batch widths and original paired clocks are in `settlement-scans.json`.

From this directory, run `python3 -B settlement_scans.py | diff settlement-scans.json -` to verify the two plan captures and twelve measured workload archives against the released digests and reproduce the counts. All timing data comes from existing Linode runs.
