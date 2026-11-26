# Durable effects waiting for settlement on canonical SLOW

Across the six measured arms of the width-16 logical-key pilot, faster claim SQL increased the number of jobs that had committed their second application write and still awaited a scalar terminal commit. The window runs from the 64th to the 448th durable second write in each 512-job run. All arms had one runner and 64 slots.

| Rotation | Lane | Time-average jobs awaiting settlement | Time with at least eight pending | Longest continuous span with at least eight |
| --- | --- | ---: | ---: | ---: |
| Forward | Stock A | 2.44 | 11.29% | 34.3 ms |
| Forward | Key | 5.37 | 25.95% | 51.3 ms |
| Forward | Stock B | 2.57 | 12.13% | 37.7 ms |
| Reverse | Stock A | 2.35 | 11.55% | 40.0 ms |
| Reverse | Key | 6.03 | 26.62% | 49.8 ms |
| Reverse | Stock B | 1.92 | 6.05% | 13.9 ms |

The replay increments the pending count at the durable effect and decrements it at the terminal commit. Simultaneous events share one timestamp. The whole-drain integral must equal the sum of the 512 individual effect-to-terminal durations exactly. The counts bound how many jobs could be ready to settle together. The ledgers do not record when each actor exited or its terminal acknowledgement arrived. An uninterrupted span can include turnover in which jobs are pending.

Both key rotations reduced PostgreSQL CPU and missed the SLOW wall-time gate. The separate grouped-success review closed before native booking. In the reverse rotation the last durable effect would still need to advance about 125 ms to clear the 97 ms submit target even with immediate final settlement. This replay leaves the benefit of grouped success on SLOW unmeasured. The review and its price replay are recorded in local commit `5f88ac339`.

Replay from a repository containing the source owner's archive commit `7df5b961c`.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-slow-claim-width-price
python3 -B "$d/settlement_backlog.py" | cmp - "$d/settlement-backlog.json"
```
